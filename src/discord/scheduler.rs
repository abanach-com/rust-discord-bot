use anyhow::{Context as _, Result, anyhow};

use serenity::{
    all::{
        Channel, ChannelId, ChannelType, GuildId, Member, PartialGuild, PermissionOverwriteType,
        Permissions, RoleId, UserId,
    },
    client::Context,
    http::Http,
};

use crate::{
    AppState,
    lua::{SchedulerOperation, SchedulerScope},
    scheduler::{
        CreateJobOutcome, JobMutation, JobMutationOutcome, NewScheduledJob, ScheduledJob,
        time::{parse_repeat_interval, parse_schedule_time},
    },
};

#[derive(Clone, Copy)]

pub struct SchedulerExecutionContext {
    pub guild_id: Option<GuildId>,

    pub channel_id: Option<ChannelId>,

    pub actor_id: Option<UserId>,

    pub actor_permissions: Permissions,

    pub command_context: bool,
}

impl SchedulerExecutionContext {
    pub const fn new(
        guild_id: Option<GuildId>,

        channel_id: Option<ChannelId>,

        actor_id: Option<UserId>,

        actor_permissions: Permissions,

        command_context: bool,
    ) -> Self {
        Self {
            guild_id,

            channel_id,

            actor_id,

            actor_permissions,

            command_context,
        }
    }
}

pub async fn execute(
    ctx: &Context,

    state: &AppState,

    module_id: &str,

    execution: SchedulerExecutionContext,

    operation: &SchedulerOperation,
) -> Result<Option<String>> {
    if module_id != "scheduler" {
        return Err(anyhow!(
            "scheduler operations are reserved for the trusted scheduler module"
        ));
    }

    if !execution.command_context {
        return Err(anyhow!(
            "scheduler tasks can only be created and modified through user commands"
        ));
    }

    let guild_id = execution
        .guild_id
        .ok_or_else(|| anyhow!("scheduler operates exclusively on a server"))?;

    let actor_id = execution
        .actor_id
        .ok_or_else(|| anyhow!("scheduler operation requires a user"))?;

    let can_manage_guild = has_manage_guild(execution.actor_permissions);

    if operation.requires_manage_guild() && !can_manage_guild {
        return Err(anyhow!(
            "this scheduler operation requires Manage Server permission"
        ));
    }

    let guild_key = guild_id.get().to_string();

    let actor_key = actor_id.get().to_string();

    match operation {
        SchedulerOperation::Create {
            when,

            content,

            channel_id,

            repeat,

            repeat_count,

            mention_creator,

            max_jobs,

            minimum_delay_seconds,

            maximum_delay_seconds,
        } => {
            let now = chrono::Utc::now();

            let run_at =
                parse_schedule_time(when, now, *minimum_delay_seconds, *maximum_delay_seconds)
                    .map_err(|error| anyhow!("invalid schedule deadline: {error}"))?;

            let target_channel = match channel_id {
                Some(channel_id) => {
                    let channel_id = ChannelId::new(parse_snowflake(channel_id)?);

                    ensure_scheduled_delivery_access(&ctx.http, guild_id, actor_id, channel_id)
                        .await?;

                    channel_id
                }

                None => {
                    let channel_id = execution
                        .channel_id
                        .ok_or_else(|| anyhow!("missing destination channel"))?;

                    ensure_scheduled_delivery_access(&ctx.http, guild_id, actor_id, channel_id)
                        .await?;

                    channel_id
                }
            };

            let repeat_every_seconds = repeat
                .as_deref()
                .map(parse_repeat_interval)
                .transpose()
                .map_err(|error| anyhow!("invalid repeat interval: {error}"))?;

            let remaining_runs = if repeat_every_seconds.is_some() {
                repeat_count.map(i64::from)
            } else {
                Some(1)
            };

            let new_job = NewScheduledJob {
                guild_id: guild_key.clone(),

                module_id: module_id.to_owned(),

                channel_id: target_channel.get().to_string(),

                creator_user_id: actor_key.clone(),

                content: content.trim().to_owned(),

                mention_creator: *mention_creator,

                run_at,

                repeat_every_seconds,

                remaining_runs,

                max_attempts: 5,
            };

            match state
                .scheduler
                .create_job_with_audit(&state.storage, new_job, *max_jobs)
                .await?
            {
                CreateJobOutcome::Created(job) => Ok(Some(format!(
                    "✅ Created task `{}`.\nChannel: <#{}>\nExecution: <t:{}:F> (<t:{}:R>)\nMode: {}",
                    job.id,
                    job.channel_id,
                    job.run_at.timestamp(),
                    job.run_at.timestamp(),
                    recurrence_description(&job),
                ))),

                CreateJobOutcome::LimitReached { limit } => Ok(Some(format!(
                    "Reached limit of **{limit}** active tasks for this user."
                ))),
            }
        }

        SchedulerOperation::List { scope, limit } => {
            let include_all = matches!(scope, SchedulerScope::All);

            let jobs = state
                .scheduler
                .list_jobs(&guild_key, &actor_key, include_all, *limit)
                .await?;

            Ok(Some(format_job_list(&jobs, include_all)))
        }

        SchedulerOperation::Cancel { job_id } => {
            mutate_job(
                state,
                module_id,
                &guild_key,
                &actor_key,
                can_manage_guild,
                job_id,
                JobMutation::Cancel,
            )
            .await
        }

        SchedulerOperation::Pause { job_id } => {
            mutate_job(
                state,
                module_id,
                &guild_key,
                &actor_key,
                can_manage_guild,
                job_id,
                JobMutation::Pause,
            )
            .await
        }

        SchedulerOperation::Resume { job_id } => {
            mutate_job(
                state,
                module_id,
                &guild_key,
                &actor_key,
                can_manage_guild,
                job_id,
                JobMutation::Resume,
            )
            .await
        }
    }
}

async fn mutate_job(
    state: &AppState,

    _module_id: &str,

    guild_id: &str,

    actor_user_id: &str,

    allow_any: bool,

    job_id: &str,

    mutation: JobMutation,
) -> Result<Option<String>> {
    let outcome = state
        .scheduler
        .mutate_job_with_audit(
            &state.storage,
            guild_id,
            job_id,
            actor_user_id,
            allow_any,
            mutation,
        )
        .await?;

    match outcome {
        JobMutationOutcome::Updated(job) => {
            let msg = if mutation == JobMutation::Cancel
                && job.cancellation_requested_at.is_some()
                && job.status == crate::scheduler::STATUS_PROCESSING
            {
                format!(
                    "⏳ Zadanie `{}` jest w trakcie przetwarzania — zlecono żądanie anulowania.",
                    job.id
                )
            } else {
                format!(
                    "✅ Zadanie `{}` ma teraz status **{}**.",
                    job.id,
                    status_label(&job.status),
                )
            };

            Ok(Some(msg))
        }

        JobMutationOutcome::NotFound => Ok(Some(
            "Nie znaleziono zadania o podanym identyfikatorze na tym serwerze.".to_owned(),
        )),

        JobMutationOutcome::Forbidden => Ok(Some(
            "You can only manage your own tasks. A moderator with Manage Server can manage all."
                .to_owned(),
        )),

        JobMutationOutcome::InvalidState { current_status } => Ok(Some(format!(
            "Ta zmiana nie jest dozwolona dla zadania w stanie **{}**.",
            status_label(&current_status),
        ))),
    }
}

pub fn calculate_effective_channel_permissions(
    guild: &serenity::model::guild::PartialGuild,
    member_roles: &[RoleId],
    user_id: UserId,
    channel: &serenity::model::channel::GuildChannel,
) -> Permissions {
    if guild.owner_id == user_id {
        return Permissions::all();
    }

    let mut base_permissions = Permissions::empty();
    let everyone_role_id = RoleId::new(guild.id.get());
    if let Some(everyone_role) = guild.roles.get(&everyone_role_id) {
        base_permissions |= everyone_role.permissions;
    }
    for role_id in member_roles {
        if let Some(role) = guild.roles.get(role_id) {
            base_permissions |= role.permissions;
        }
    }

    if base_permissions.contains(Permissions::ADMINISTRATOR) {
        return Permissions::all();
    }

    let mut perms = base_permissions;

    for overwrite in &channel.permission_overwrites {
        if let PermissionOverwriteType::Role(r) = overwrite.kind
            && r == everyone_role_id
        {
            perms.remove(overwrite.deny);
            perms.insert(overwrite.allow);
        }
    }

    let mut role_deny = Permissions::empty();
    let mut role_allow = Permissions::empty();
    for overwrite in &channel.permission_overwrites {
        if let PermissionOverwriteType::Role(r) = overwrite.kind
            && r != everyone_role_id
            && member_roles.contains(&r)
        {
            role_deny.insert(overwrite.deny);
            role_allow.insert(overwrite.allow);
        }
    }
    perms.remove(role_deny);
    perms.insert(role_allow);

    for overwrite in &channel.permission_overwrites {
        if let PermissionOverwriteType::Member(u) = overwrite.kind
            && u == user_id
        {
            perms.remove(overwrite.deny);
            perms.insert(overwrite.allow);
        }
    }

    perms
}

pub struct DeliveryAccessContext {
    pub guild_id: GuildId,
    pub actor_id: UserId,
    pub guild: PartialGuild,
    pub actor_member: Member,
    pub bot_member: Member,
    pub bot_user_id: UserId,
}

impl DeliveryAccessContext {
    pub async fn fetch(http: &Http, guild_id: GuildId, actor_id: UserId) -> Result<Self> {
        let guild = guild_id
            .to_partial_guild(http)
            .await
            .with_context(|| format!("cannot fetch guild data for {guild_id}"))?;

        let actor_member = guild_id
            .member(http, actor_id)
            .await
            .with_context(|| format!("cannot fetch member {actor_id} in guild {guild_id}"))?;

        let bot_user = http
            .get_current_user()
            .await
            .context("cannot fetch current bot user")?;

        let bot_member = guild_id
            .member(http, bot_user.id)
            .await
            .with_context(|| format!("cannot fetch bot member in guild {guild_id}"))?;

        Ok(Self {
            guild_id,
            actor_id,
            guild,
            actor_member,
            bot_member,
            bot_user_id: bot_user.id,
        })
    }
}

pub async fn ensure_channel_access_with_context(
    http: &Http,
    ctx: &DeliveryAccessContext,
    target_channel_id: ChannelId,
    require_embed: bool,
) -> Result<()> {
    let channel = target_channel_id
        .to_channel(http)
        .await
        .with_context(|| format!("cannot read data for channel {target_channel_id}"))?;

    let Channel::Guild(guild_channel) = channel else {
        anyhow::bail!("destination channel must be a guild channel");
    };

    if guild_channel.guild_id != ctx.guild_id {
        anyhow::bail!(
            "target channel {target_channel_id} belongs to a different server {}",
            guild_channel.guild_id
        );
    }

    super::ensure_send_message_channel_type(guild_channel.kind)?;

    let is_thread = matches!(
        guild_channel.kind,
        ChannelType::PublicThread | ChannelType::PrivateThread | ChannelType::NewsThread
    );

    let parent_channel = if is_thread {
        if let Some(parent_id) = guild_channel.parent_id {
            let parent = parent_id.to_channel(http).await.with_context(|| {
                format!("cannot read parent channel {parent_id} for thread {target_channel_id}")
            })?;
            match parent {
                Channel::Guild(p) => Some(p),
                _ => None,
            }
        } else {
            None
        }
    } else {
        None
    };

    let overwrite_channel = parent_channel.as_ref().unwrap_or(&guild_channel);

    // 1. Verify creator permissions in destination channel
    let creator_perms = calculate_effective_channel_permissions(
        &ctx.guild,
        &ctx.actor_member.roles,
        ctx.actor_id,
        overwrite_channel,
    );

    if !creator_perms.contains(Permissions::VIEW_CHANNEL) {
        anyhow::bail!("creator lacks VIEW_CHANNEL permission in channel {target_channel_id}");
    }

    if is_thread {
        if !creator_perms.contains(Permissions::SEND_MESSAGES_IN_THREADS) {
            anyhow::bail!(
                "creator lacks SEND_MESSAGES_IN_THREADS permission in thread {target_channel_id}"
            );
        }
    } else if !creator_perms.contains(Permissions::SEND_MESSAGES) {
        anyhow::bail!("creator lacks SEND_MESSAGES permission in channel {target_channel_id}");
    }

    if require_embed && !creator_perms.contains(Permissions::EMBED_LINKS) {
        anyhow::bail!("creator lacks EMBED_LINKS permission in channel {target_channel_id}");
    }

    if guild_channel.kind == ChannelType::PrivateThread
        && !creator_perms.contains(Permissions::ADMINISTRATOR)
        && !creator_perms.contains(Permissions::MANAGE_THREADS)
    {
        let thread_members = http
            .get_channel_thread_members(target_channel_id)
            .await
            .with_context(|| {
                format!("cannot fetch thread members for private thread {target_channel_id}")
            })?;
        if !thread_members.iter().any(|tm| tm.user_id == ctx.actor_id) {
            anyhow::bail!("creator is not a member of the private thread {target_channel_id}");
        }
    }

    // 2. Verify bot permissions in destination channel
    let bot_perms = calculate_effective_channel_permissions(
        &ctx.guild,
        &ctx.bot_member.roles,
        ctx.bot_user_id,
        overwrite_channel,
    );

    if !bot_perms.contains(Permissions::VIEW_CHANNEL) {
        anyhow::bail!("bot lacks VIEW_CHANNEL permission in channel {target_channel_id}");
    }

    if is_thread {
        if !bot_perms.contains(Permissions::SEND_MESSAGES_IN_THREADS) {
            anyhow::bail!(
                "bot lacks SEND_MESSAGES_IN_THREADS permission in thread {target_channel_id}"
            );
        }
    } else if !bot_perms.contains(Permissions::SEND_MESSAGES) {
        anyhow::bail!("bot lacks SEND_MESSAGES permission in channel {target_channel_id}");
    }

    if require_embed && !bot_perms.contains(Permissions::EMBED_LINKS) {
        anyhow::bail!("bot lacks EMBED_LINKS permission in channel {target_channel_id}");
    }

    if guild_channel.kind == ChannelType::PrivateThread
        && !bot_perms.contains(Permissions::ADMINISTRATOR)
        && !bot_perms.contains(Permissions::MANAGE_THREADS)
    {
        let thread_members = http
            .get_channel_thread_members(target_channel_id)
            .await
            .with_context(|| {
                format!("cannot fetch thread members for private thread {target_channel_id}")
            })?;
        if !thread_members
            .iter()
            .any(|tm| tm.user_id == ctx.bot_user_id)
        {
            anyhow::bail!("bot is not a member of the private thread {target_channel_id}");
        }
    }

    Ok(())
}

pub async fn ensure_channel_delivery_access(
    http: &Http,
    guild_id: GuildId,
    actor_id: UserId,
    target_channel_id: ChannelId,
    require_embed: bool,
) -> Result<()> {
    let ctx = DeliveryAccessContext::fetch(http, guild_id, actor_id).await?;
    ensure_channel_access_with_context(http, &ctx, target_channel_id, require_embed).await
}

pub async fn ensure_scheduled_delivery_access(
    http: &Http,
    guild_id: GuildId,
    actor_id: UserId,
    target_channel_id: ChannelId,
) -> Result<()> {
    ensure_channel_delivery_access(http, guild_id, actor_id, target_channel_id, false).await
}

fn format_job_list(jobs: &[ScheduledJob], include_owner: bool) -> String {
    if jobs.is_empty() {
        return "No active, paused, or running tasks.".to_owned();
    }

    let mut lines = Vec::with_capacity(jobs.len() + 1);

    lines.push(format!("🗓️ **Zaplanowane zadania ({})**", jobs.len()));

    for job in jobs {
        let mut preview = job.content.replace(['\n', '\r'], " ");

        if preview.chars().count() > 56 {
            preview = preview.chars().take(53).collect::<String>() + "...";
        }

        let owner = if include_owner {
            format!(" · <@{}>", job.creator_user_id)
        } else {
            String::new()
        };

        lines.push(format!(
            "• `{}` · **{}** · <t:{}:R> · <#{}>{owner}\n  `{}` · {}",
            job.short_id(),
            status_label(&job.status),
            job.run_at.timestamp(),
            job.channel_id,
            job.id,
            preview,
        ));
    }

    lines.join("\n")
}

fn recurrence_description(job: &ScheduledJob) -> String {
    let Some(seconds) = job.repeat_every_seconds else {
        return "jednorazowo".to_owned();
    };

    match job.remaining_runs {
        Some(runs) => format!("every {seconds}s, {runs} runs total"),

        None => format!("every {seconds}s, unlimited runs"),
    }
}

fn status_label(status: &str) -> &str {
    match status {
        "active" => "aktywne",

        "paused" => "wstrzymane",

        "processing" => "wykonywane",

        "completed" => "completed",

        "cancelled" => "anulowane",

        "failed" => "nieudane",

        _ => status,
    }
}

fn has_manage_guild(permissions: Permissions) -> bool {
    permissions.contains(Permissions::ADMINISTRATOR)
        || permissions.contains(Permissions::MANAGE_GUILD)
}

fn parse_snowflake(value: &str) -> Result<u64> {
    let value = value
        .parse::<u64>()
        .with_context(|| format!("{value:?} nie jest identyfikatorem Discorda"))?;

    if value == 0 {
        return Err(anyhow!("Discord ID cannot be zero"));
    }

    Ok(value)
}
