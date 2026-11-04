use std::sync::atomic::{AtomicBool, Ordering};

use std::time::{Duration, Instant};

use dashmap::DashMap;

use serde_json::{Map, Value, json};

use serenity::{
    all::{
        ChannelId, CommandDataOption, CommandDataOptionValue, CommandInteraction, GuildId,
        Interaction, Member, Message, MessageId, MessageUpdateEvent, Ready, User, UserId,
        VoiceState,
    },
    async_trait,
    builder::{CreateAllowedMentions, CreateInteractionResponse, CreateInteractionResponseMessage},
    client::{Context, EventHandler},
};

use crate::{
    AppState,
    lua::{LuaEventContext, LuaExecutionContext},
};

use super::{commands, executor};

pub struct Handler {
    state: AppState,

    commands_registered: AtomicBool,

    recent_joins: DashMap<GuildId, Vec<Instant>>,
}

impl Handler {
    pub fn new(state: AppState) -> Self {
        Self {
            state,

            commands_registered: AtomicBool::new(false),

            recent_joins: DashMap::new(),
        }
    }
}

#[async_trait]

impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        self.state.is_gateway_ready.store(true, Ordering::SeqCst);

        self.state
            .metrics
            .discord_events_total
            .fetch_add(1, Ordering::Relaxed);

        tracing::info!(

            user = %ready.user.name,

            guilds = ready.guilds.len(),

            "Discord gateway is ready"

        );

        if !self.commands_registered.swap(true, Ordering::AcqRel)
            && let Err(error) = commands::register(&ctx, &self.state).await
        {
            self.commands_registered.store(false, Ordering::Release);

            tracing::error!(?error, "failed to register application commands");
        }

        // Reconcile pending stuck softbans (#50)

        let state = self.state.clone();

        let http = ctx.http.clone();

        tokio::spawn(async move {
            let pending_banned = state
                .storage
                .list_pending_moderation_intents("softban", "banned")
                .await
                .unwrap_or_default();

            let pending_failed = state
                .storage
                .list_pending_moderation_intents("softban", "unban_failed")
                .await
                .unwrap_or_default();

            for intent in pending_banned.into_iter().chain(pending_failed) {
                if let (Ok(guild_num), Ok(user_num)) = (
                    intent.guild_id.parse::<u64>(),
                    intent.user_id.parse::<u64>(),
                ) {
                    let g_id = GuildId::new(guild_num);

                    let u_id = UserId::new(user_num);

                    match g_id.unban(&http, u_id).await {
                        Ok(()) => {
                            let _ = state
                                .storage
                                .update_moderation_intent_step(intent.id, "completed", None)
                                .await;

                            tracing::info!(

                                intent_id = %intent.id,

                                guild_id = %intent.guild_id,

                                user_id = %intent.user_id,

                                "Stuck softban reconciled: user unbanned successfully"

                            );
                        }

                        Err(err) => {
                            let _ = state
                                .storage
                                .update_moderation_intent_step(
                                    intent.id,
                                    "unban_failed",
                                    Some(&err.to_string()),
                                )
                                .await;

                            tracing::warn!(

                                intent_id = %intent.id,

                                guild_id = %intent.guild_id,

                                user_id = %intent.user_id,

                                ?err,

                                "Failed to reconcile stuck softban during startup"

                            );
                        }
                    }
                }
            }
        });
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        self.state
            .metrics
            .discord_events_total
            .fetch_add(1, Ordering::Relaxed);

        let Interaction::Command(command) = interaction else {
            return;
        };

        if let Err(error) = handle_command(&ctx, &self.state, &command).await {
            self.state
                .metrics
                .lua_command_errors_total
                .fetch_add(1, Ordering::Relaxed);

            tracing::error!(

                command = %command.data.name,

                user_id = %command.user.id,

                ?error,

                "failed to handle application command"

            );

            let _ = respond_ephemeral(
                &ctx,
                &command,
                &crate::translations::t(&command.locale, "system.internal_error"),
            )
            .await;
        }
    }

    async fn message(&self, ctx: Context, message: Message) {
        self.state
            .metrics
            .discord_events_total
            .fetch_add(1, Ordering::Relaxed);

        if message.author.bot {
            return;
        }

        let Some(guild_id) = message.guild_id else {
            return;
        };

        let event = LuaEventContext {
            name: "message_create".to_owned(),

            guild_id: Some(guild_id.get().to_string()),

            channel_id: Some(message.channel_id.get().to_string()),

            actor_id: Some(message.author.id.get().to_string()),

            data: json!({

                "message_id": message.id,

                "content": message.content,

                "author": {

                    "id": message.author.id,

                    "name": message.author.name,

                    "global_name": message.author.global_name,

                },

                "mentions": message.mentions.iter().map(|user| user.id).collect::<Vec<_>>(),

                "mention_roles": message.mention_roles.iter().map(|role| role.get().to_string()).collect::<Vec<_>>(),

                "mention_everyone": message.mention_everyone,

                "attachments": message.attachments.iter().map(|attachment| {

                    json!({

                        "id": attachment.id,

                        "filename": attachment.filename,

                        "size": attachment.size,

                        "content_type": attachment.content_type,

                        "url": attachment.url,

                    })

                }).collect::<Vec<_>>(),

            }),

            config: Value::Null,
        };

        run_event(
            &ctx,
            &self.state,
            "message_create",
            event,
            Some(guild_id),
            Some(message.channel_id),
            Some(message.author.id),
        )
        .await;
    }

    async fn message_update(
        &self,

        ctx: Context,

        _old_if_available: Option<Message>,

        new: Option<Message>,

        event: MessageUpdateEvent,
    ) {
        self.state
            .metrics
            .discord_events_total
            .fetch_add(1, Ordering::Relaxed);

        let author = event
            .author
            .as_ref()
            .or_else(|| new.as_ref().map(|m| &m.author));

        let Some(author) = author else {
            self.state
                .metrics
                .message_updates_unknown_author_total
                .fetch_add(1, Ordering::Relaxed);
            tracing::debug!(
                message_id = %event.id,
                "skipping message_update with unknown author to prevent unverified moderation actions"
            );
            return;
        };

        if author.bot {
            return;
        }

        let guild_id = event
            .guild_id
            .or_else(|| new.as_ref().and_then(|m| m.guild_id));

        let Some(guild_id) = guild_id else {
            return;
        };

        let content = event
            .content
            .as_ref()
            .or_else(|| new.as_ref().map(|m| &m.content));

        let Some(content) = content else {
            return;
        };

        let author_id = author.id;
        let author_name = author.name.clone();
        let author_global_name = author.global_name.clone();

        let mentions: Vec<_> = event
            .mentions
            .as_ref()
            .map(|m| m.iter().map(|u| u.id).collect())
            .or_else(|| {
                new.as_ref()
                    .map(|m| m.mentions.iter().map(|u| u.id).collect())
            })
            .unwrap_or_default();

        let mention_roles: Vec<_> = event
            .mention_roles
            .as_ref()
            .map(|r| r.iter().map(|role| role.get().to_string()).collect())
            .or_else(|| {
                new.as_ref().map(|m| {
                    m.mention_roles
                        .iter()
                        .map(|role| role.get().to_string())
                        .collect()
                })
            })
            .unwrap_or_default();

        let mention_everyone = event
            .mention_everyone
            .or_else(|| new.as_ref().map(|m| m.mention_everyone))
            .unwrap_or(false);

        let attachments = event
            .attachments
            .as_ref()
            .or_else(|| new.as_ref().map(|m| &m.attachments));

        let attachments_json: Vec<_> = attachments
            .map(|list| {
                list.iter()
                    .map(|attachment| {
                        json!({

                            "id": attachment.id,

                            "filename": attachment.filename,

                            "size": attachment.size,

                            "content_type": attachment.content_type,

                            "url": attachment.url,

                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let event_context = LuaEventContext {
            name: "message_update".to_owned(),

            guild_id: Some(guild_id.get().to_string()),

            channel_id: Some(event.channel_id.get().to_string()),

            actor_id: Some(author_id.get().to_string()),

            data: json!({

                "message_id": event.id,

                "content": content,

                "author": {

                    "id": author_id,

                    "name": author_name,

                    "global_name": author_global_name,

                },

                "mentions": mentions,

                "mention_roles": mention_roles,

                "mention_everyone": mention_everyone,

                "attachments": attachments_json,

                "is_edit": true,

            }),

            config: Value::Null,
        };

        run_event(
            &ctx,
            &self.state,
            "message_update",
            event_context,
            Some(guild_id),
            Some(event.channel_id),
            Some(author_id),
        )
        .await;
    }

    async fn guild_member_addition(&self, ctx: Context, new_member: Member) {
        self.state
            .metrics
            .discord_events_total
            .fetch_add(1, Ordering::Relaxed);

        let guild_id = new_member.guild_id;

        let now = Instant::now();

        let window = Duration::from_secs(60);

        let mut entry = self.recent_joins.entry(guild_id).or_default();

        entry.retain(|&ts| now.saturating_duration_since(ts) < window);

        entry.push(now);

        let recent_joins_count = entry.len();

        drop(entry);

        let guild_name = ctx.cache.guild(guild_id).map(|g| g.name.clone());

        let member_count = ctx.cache.guild(guild_id).map(|g| g.member_count);

        let event = LuaEventContext {
            name: "guild_member_add".to_owned(),

            guild_id: Some(guild_id.get().to_string()),

            channel_id: None,

            actor_id: Some(new_member.user.id.get().to_string()),

            data: json!({

                "user": {

                    "id": new_member.user.id,

                    "name": new_member.user.name,

                    "global_name": new_member.user.global_name,

                    "bot": new_member.user.bot,

                    "avatar": new_member.user.avatar.map(|hash| hash.to_string()),

                    "has_avatar": new_member.user.avatar.is_some(),

                },

                "roles": new_member.roles,

                "joined_at": new_member.joined_at,

                "recent_joins_count": recent_joins_count,

                "recent_joins_window_seconds": 60,

                "guild_name": guild_name,

                "member_count": member_count,

            }),

            config: Value::Null,
        };

        run_event(
            &ctx,
            &self.state,
            "guild_member_add",
            event,
            Some(guild_id),
            None,
            Some(new_member.user.id),
        )
        .await;
    }

    async fn guild_member_removal(
        &self,

        ctx: Context,

        guild_id: GuildId,

        user: User,

        member_data_if_available: Option<Member>,
    ) {
        self.state
            .metrics
            .discord_events_total
            .fetch_add(1, Ordering::Relaxed);

        let guild_name = ctx.cache.guild(guild_id).map(|g| g.name.clone());

        let member_count = ctx.cache.guild(guild_id).map(|g| g.member_count);

        let event = LuaEventContext {
            name: "guild_member_remove".to_owned(),

            guild_id: Some(guild_id.get().to_string()),

            channel_id: None,

            actor_id: Some(user.id.get().to_string()),

            data: json!({

                "user": {

                    "id": user.id,

                    "name": user.name,

                    "global_name": user.global_name,

                    "bot": user.bot,

                },

                "roles": member_data_if_available.as_ref().map(|m| &m.roles),

                "joined_at": member_data_if_available.as_ref().and_then(|m| m.joined_at),

                "guild_name": guild_name,

                "member_count": member_count,

            }),

            config: Value::Null,
        };

        run_event(
            &ctx,
            &self.state,
            "guild_member_remove",
            event,
            Some(guild_id),
            None,
            Some(user.id),
        )
        .await;
    }

    async fn message_delete(
        &self,

        ctx: Context,

        channel_id: ChannelId,

        deleted_message_id: MessageId,

        guild_id: Option<GuildId>,
    ) {
        self.state
            .metrics
            .discord_events_total
            .fetch_add(1, Ordering::Relaxed);

        let Some(guild_id) = guild_id else {
            return;
        };

        let event = LuaEventContext {
            name: "message_delete".to_owned(),

            guild_id: Some(guild_id.get().to_string()),

            channel_id: Some(channel_id.get().to_string()),

            actor_id: None,

            data: json!({

                "message_id": deleted_message_id.get().to_string(),

                "channel_id": channel_id.get().to_string(),

            }),

            config: Value::Null,
        };

        run_event(
            &ctx,
            &self.state,
            "message_delete",
            event,
            Some(guild_id),
            Some(channel_id),
            None,
        )
        .await;
    }

    async fn voice_state_update(&self, ctx: Context, _old: Option<VoiceState>, new: VoiceState) {
        if let Some(guild_id) = new.guild_id {
            let bot_id = ctx.cache.current_user().id;

            if new.user_id == bot_id
                && new.channel_id.is_none()
                && let Some(manager) = songbird::get(&ctx).await
            {
                let _ = manager.remove(guild_id).await;
            }
        }
    }
}

async fn handle_command(
    ctx: &Context,

    state: &AppState,

    command: &CommandInteraction,
) -> anyhow::Result<()> {
    let deferred = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    let responded = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    let defer_http = ctx.http.clone();

    let defer_command = command.clone();

    let defer_flag = deferred.clone();

    let responded_flag = responded.clone();

    // Issue #111: Default defer visibility to ephemeral to guarantee private/confidential responses

    let is_ephemeral_by_default = true;

    // Issue #121: Set defer_flag ONLY after ACK HTTP response successfully completes to prevent races

    let defer_handle = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

        if !responded_flag.load(Ordering::SeqCst) {
            let res = defer_command
                .create_response(
                    &defer_http,
                    serenity::builder::CreateInteractionResponse::Defer(
                        serenity::builder::CreateInteractionResponseMessage::new()
                            .ephemeral(is_ephemeral_by_default),
                    ),
                )
                .await;

            if res.is_ok() {
                defer_flag.store(true, Ordering::SeqCst);
            }
        }
    });

    let result = execute_command_inner(ctx, state, command, &deferred, &responded).await;

    defer_handle.abort();

    if let Err(err) = result {
        tracing::error!(command = %command.data.name, error = %err, "Error executing command");

        let err_text = crate::translations::t_args(
            &command.locale,
            "system.command_error",
            &[("error", &err.to_string())],
        );

        if deferred.load(Ordering::SeqCst) {
            let _ = command
                .edit_response(
                    &ctx.http,
                    serenity::builder::EditInteractionResponse::new().content(&err_text),
                )
                .await;
        } else if !responded.swap(true, Ordering::SeqCst) {
            let _ = respond_ephemeral(ctx, command, &err_text).await;
        }
    }

    Ok(())
}

async fn respond_or_edit(
    ctx: &Context,

    command: &CommandInteraction,

    deferred: &std::sync::Arc<std::sync::atomic::AtomicBool>,

    responded: &std::sync::Arc<std::sync::atomic::AtomicBool>,

    content: &str,
) -> anyhow::Result<()> {
    if deferred.load(Ordering::SeqCst) {
        let _ = command
            .edit_response(
                &ctx.http,
                serenity::builder::EditInteractionResponse::new()
                    .content(content)
                    .allowed_mentions(serenity::builder::CreateAllowedMentions::new()),
            )
            .await;
    } else if !responded.swap(true, Ordering::SeqCst) {
        respond_ephemeral(ctx, command, content).await?;
    }

    Ok(())
}

async fn execute_command_inner(
    ctx: &Context,

    state: &AppState,

    command: &CommandInteraction,

    deferred: &std::sync::Arc<std::sync::atomic::AtomicBool>,

    responded: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> anyhow::Result<()> {
    let Some(module_id) = state.scripts.module_for_command(&command.data.name).await else {
        respond_or_edit(
            ctx,
            command,
            deferred,
            responded,
            &crate::translations::t(&command.locale, "system.command_unavailable"),
        )
        .await?;

        return Ok(());
    };

    let Some(manifest) = state.scripts.manifest(&module_id).await else {
        respond_or_edit(
            ctx,
            command,
            deferred,
            responded,
            &crate::translations::t(&command.locale, "system.module_not_loaded"),
        )
        .await?;

        return Ok(());
    };

    let (enabled, config) = match command.guild_id {
        Some(guild_id) => match state
            .storage
            .get_module_settings(&guild_id.get().to_string(), &module_id)
            .await?
        {
            Some(settings) => (settings.enabled, settings.config),

            None => (
                manifest.default_enabled,
                state.scripts.default_config(&module_id).await,
            ),
        },

        None => (
            manifest.default_enabled,
            state.scripts.default_config(&module_id).await,
        ),
    };

    if !enabled {
        respond_or_edit(
            ctx,
            command,
            deferred,
            responded,
            &crate::translations::t(&command.locale, "system.module_disabled"),
        )
        .await?;

        return Ok(());
    }

    let user_id = command.user.id.get().to_string();

    let guild_id_str = command.guild_id.map(|id| id.get().to_string());

    let rate_decision =
        state
            .rate_limiter
            .check_command(&command.data.name, &user_id, guild_id_str.as_deref());

    if !rate_decision.allowed {
        state
            .metrics
            .rate_limit_throttled_total
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let retry_secs = rate_decision.reset_after.as_secs().max(1);

        let msg = crate::translations::t_args(
            &command.locale,
            "system.rate_limited",
            &[
                ("command", &command.data.name),
                ("seconds", &retry_secs.to_string()),
            ],
        );

        respond_or_edit(ctx, command, deferred, responded, &msg).await?;

        return Ok(());
    }

    let member_roles = command
        .member
        .as_ref()
        .map(|member| {
            member
                .roles
                .iter()
                .map(|role_id| role_id.get().to_string())
                .collect()
        })
        .unwrap_or_default();

    let member_permissions = command
        .member
        .as_ref()
        .and_then(|member| member.permissions)
        .map_or_else(
            || "0".to_owned(),
            |permissions| permissions.bits().to_string(),
        );

    let execution_context = LuaExecutionContext {
        guild_id: command.guild_id.map(|id| id.get().to_string()),

        channel_id: command.channel_id.get().to_string(),

        user_id: command.user.id.get().to_string(),

        user_name: command
            .user
            .global_name
            .clone()
            .unwrap_or_else(|| command.user.name.clone()),

        member_roles,

        member_permissions,

        locale: command.locale.clone(),

        options: options_to_json(&command.data.options),

        config,
    };

    state
        .metrics
        .lua_commands_total
        .fetch_add(1, Ordering::Relaxed);

    let actions = state
        .scripts
        .execute_command(&module_id, &command.data.name, execution_context)
        .await?;

    let is_deferred = deferred.load(Ordering::SeqCst);

    responded.store(true, Ordering::SeqCst);

    executor::execute_command_actions(ctx, state, command, &module_id, &actions, is_deferred).await
}

async fn run_event(
    ctx: &Context,

    state: &AppState,

    event_name: &str,

    event: LuaEventContext,

    guild_id: Option<serenity::all::GuildId>,

    channel_id: Option<serenity::all::ChannelId>,

    actor_id: Option<serenity::all::UserId>,
) {
    for module_id in state.scripts.modules_for_event(event_name).await {
        let Some(manifest) = state.scripts.manifest(&module_id).await else {
            continue;
        };

        let mut module_event = event.clone();

        if let Some(guild_id) = guild_id {
            match state
                .storage
                .get_module_settings(&guild_id.get().to_string(), &module_id)
                .await
            {
                Ok(Some(settings)) if settings.enabled => {
                    module_event.config = settings.config;
                }

                Ok(Some(_)) => continue,

                Ok(None) if manifest.default_enabled => {
                    module_event.config = state.scripts.default_config(&module_id).await;
                }

                Ok(None) => continue,

                Err(error) => {
                    tracing::error!(module_id, event_name, ?error, "cannot load module settings");

                    continue;
                }
            }
        }

        match state
            .scripts
            .execute_event(&module_id, event_name, module_event)
            .await
        {
            Ok(actions) => {
                if let Err(error) = executor::execute_event_actions(
                    ctx, state, &module_id, guild_id, channel_id, actor_id, &actions,
                )
                .await
                {
                    tracing::warn!(module_id, event_name, ?error, "event action failed");
                }
            }

            Err(error) => {
                tracing::warn!(module_id, event_name, ?error, "Lua event failed");
            }
        }
    }
}

fn options_to_json(options: &[CommandDataOption]) -> Value {
    let mut object = Map::new();

    for option in options {
        object.insert(option.name.clone(), option_value_to_json(&option.value));
    }

    Value::Object(object)
}

fn option_value_to_json(value: &CommandDataOptionValue) -> Value {
    match value {
        CommandDataOptionValue::Autocomplete { value, .. }
        | CommandDataOptionValue::String(value) => Value::String(value.clone()),

        CommandDataOptionValue::Boolean(value) => Value::Bool(*value),

        CommandDataOptionValue::Integer(value) => json!(value),

        CommandDataOptionValue::Number(value) => json!(value),

        CommandDataOptionValue::SubCommand(options)
        | CommandDataOptionValue::SubCommandGroup(options) => options_to_json(options),

        CommandDataOptionValue::Attachment(value) => Value::String(value.get().to_string()),

        CommandDataOptionValue::Channel(value) => Value::String(value.get().to_string()),

        CommandDataOptionValue::Mentionable(value) => Value::String(value.get().to_string()),

        CommandDataOptionValue::Role(value) => Value::String(value.get().to_string()),

        CommandDataOptionValue::User(value) => Value::String(value.get().to_string()),

        CommandDataOptionValue::Unknown(value) => json!(value),

        _ => Value::Null,
    }
}

async fn respond_ephemeral(
    ctx: &Context,

    command: &CommandInteraction,

    content: &str,
) -> anyhow::Result<()> {
    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content(content)
                    .ephemeral(true)
                    .allowed_mentions(CreateAllowedMentions::new()),
            ),
        )
        .await?;

    Ok(())
}
