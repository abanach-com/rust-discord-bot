use std::collections::HashMap;

use anyhow::{Context as _, Result, anyhow};

use chrono::{Duration, Utc};

use uuid::Uuid;

use serenity::{
    all::{
        ChannelId, ChannelType, CommandInteraction, GuildId, Member, MessageId, Permissions, Role,
        RoleId, UserId,
    },
    builder::{
        CreateAllowedMentions, CreateInteractionResponse, CreateInteractionResponseFollowup,
        CreateInteractionResponseMessage, CreateMessage, EditMember, GetMessages,
    },
    client::Context,
};

use crate::{
    AppState,
    lua::{LuaAction, TagOperation},
};

use super::{
    economy, giveaway, moderation_cases, music, progression, reaction_roles, scheduler, tags,
    tickets,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]

pub enum ChannelPurpose {
    SendMessage,

    ManageMessages,
}

#[derive(Clone, Debug)]

pub struct ActionOrigin {
    pub guild_id: Option<GuildId>,

    pub channel_id: Option<ChannelId>,

    pub actor_id: Option<UserId>,

    pub actor_permissions: Permissions,

    pub actor_roles: Vec<RoleId>,

    pub app_permissions: Permissions,

    pub enforce_actor_permissions: bool,

    pub interaction_id: Option<String>,
}

pub fn chunk_discord_text(text: &str, limit: usize) -> Vec<String> {
    if text.len() <= limit {
        return vec![text.to_owned()];
    }

    let mut result = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        if remaining.len() <= limit {
            result.push(remaining.to_owned());
            break;
        }

        let max_end = remaining
            .char_indices()
            .take_while(|(start, ch)| start + ch.len_utf8() <= limit)
            .map(|(start, ch)| start + ch.len_utf8())
            .last()
            .unwrap_or_else(|| {
                remaining
                    .chars()
                    .next()
                    .map(|c| c.len_utf8())
                    .unwrap_or(remaining.len())
            });

        // Prefer splitting after the last newline within the limit to preserve natural line breaks
        let split_at = if let Some(nl_pos) = remaining[..max_end].rfind('\n') {
            nl_pos + 1
        } else {
            max_end
        };

        result.push(remaining[..split_at].to_owned());
        remaining = &remaining[split_at..];
    }

    result
}

pub async fn execute_command_actions(
    ctx: &Context,

    state: &AppState,

    command: &CommandInteraction,

    module_id: &str,

    actions: &[LuaAction],

    is_deferred: bool,
) -> Result<()> {
    let first_reply = actions.iter().position(|action| {
        matches!(
            action,
            LuaAction::Reply {
                content: _,

                ephemeral: _
            }
        )
    });

    if is_deferred {
        if let Some(index) = first_reply {
            if let LuaAction::Reply { content, ephemeral } = &actions[index] {
                let chunks = chunk_discord_text(content, 1950);

                if let Some(first) = chunks.first() {
                    command
                        .edit_response(
                            &ctx.http,
                            serenity::builder::EditInteractionResponse::new()
                                .content(first)
                                .allowed_mentions(CreateAllowedMentions::new()),
                        )
                        .await?;
                }

                for chunk in chunks.into_iter().skip(1) {
                    command
                        .create_followup(
                            &ctx.http,
                            CreateInteractionResponseFollowup::new()
                                .content(chunk)
                                .ephemeral(*ephemeral)
                                .allowed_mentions(CreateAllowedMentions::new()),
                        )
                        .await?;
                }
            }
        } else {
            command
                .edit_response(
                    &ctx.http,
                    serenity::builder::EditInteractionResponse::new()
                        .content(crate::translations::t(
                            &command.locale,
                            "system.command_executed",
                        ))
                        .allowed_mentions(CreateAllowedMentions::new()),
                )
                .await?;
        }
    } else if let Some(index) = first_reply {
        if let LuaAction::Reply { content, ephemeral } = &actions[index] {
            let chunks = chunk_discord_text(content, 1950);

            let first_chunk = chunks.first().cloned().unwrap_or_default();

            command
                .create_response(
                    &ctx.http,
                    CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .content(first_chunk)
                            .ephemeral(*ephemeral)
                            .allowed_mentions(CreateAllowedMentions::new()),
                    ),
                )
                .await?;

            for chunk in chunks.into_iter().skip(1) {
                command
                    .create_followup(
                        &ctx.http,
                        CreateInteractionResponseFollowup::new()
                            .content(chunk)
                            .ephemeral(*ephemeral)
                            .allowed_mentions(CreateAllowedMentions::new()),
                    )
                    .await?;
            }
        }
    } else {
        command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content(crate::translations::t(
                            &command.locale,
                            "system.command_accepted",
                        ))
                        .ephemeral(true)
                        .allowed_mentions(CreateAllowedMentions::new()),
                ),
            )
            .await?;
    }

    let actor_roles = command
        .member
        .as_ref()
        .map(|m| m.roles.clone())
        .unwrap_or_default();

    let origin = ActionOrigin {
        guild_id: command.guild_id,

        channel_id: Some(command.channel_id),

        actor_id: Some(command.user.id),

        actor_permissions: command
            .member
            .as_ref()
            .and_then(|member| member.permissions)
            .unwrap_or_else(Permissions::empty),

        actor_roles,

        app_permissions: command.app_permissions.unwrap_or_else(Permissions::empty),

        enforce_actor_permissions: true,

        interaction_id: Some(command.id.get().to_string()),
    };

    for (index, action) in actions.iter().enumerate() {
        if Some(index) == first_reply {
            continue;
        }

        if let LuaAction::Reply { content, ephemeral } = action {
            for chunk in chunk_discord_text(content, 1950) {
                command
                    .create_followup(
                        &ctx.http,
                        CreateInteractionResponseFollowup::new()
                            .content(chunk)
                            .ephemeral(*ephemeral)
                            .allowed_mentions(CreateAllowedMentions::new()),
                    )
                    .await?;
            }

            continue;
        }

        match execute_action(ctx, state, module_id, &origin, action).await {
            Ok(Some(status)) => {
                for chunk in chunk_discord_text(&status, 1950) {
                    command
                        .create_followup(
                            &ctx.http,
                            CreateInteractionResponseFollowup::new()
                                .content(chunk)
                                .ephemeral(true)
                                .allowed_mentions(CreateAllowedMentions::new()),
                        )
                        .await?;
                }
            }

            Ok(None) => {}

            Err(error) => {
                tracing::warn!(

                    module_id,

                    command = %command.data.name,

                    user_id = %command.user.id,

                    ?error,

                    "Lua action failed"

                );

                for chunk in
                    chunk_discord_text(&format!("Nie udało się wykonać akcji: {error}"), 1950)
                {
                    command
                        .create_followup(
                            &ctx.http,
                            CreateInteractionResponseFollowup::new()
                                .content(chunk)
                                .ephemeral(true)
                                .allowed_mentions(CreateAllowedMentions::new()),
                        )
                        .await?;
                }

                break;
            }
        }
    }

    Ok(())
}

pub async fn execute_event_actions(
    ctx: &Context,

    state: &AppState,

    module_id: &str,

    guild_id: Option<GuildId>,

    channel_id: Option<ChannelId>,

    actor_id: Option<UserId>,

    actions: &[LuaAction],
) -> Result<()> {
    let origin = ActionOrigin {
        guild_id,

        channel_id,

        actor_id,

        actor_permissions: Permissions::empty(),

        actor_roles: Vec::new(),

        app_permissions: Permissions::all(),

        enforce_actor_permissions: false,

        interaction_id: None,
    };

    let mut last_action_succeeded = true;

    let mut message_delete_failed = false;

    for action in actions {
        if let LuaAction::Audit { event, .. } = action
            && !last_action_succeeded
            && (event == "welcome_sent" || event == "goodbye_sent" || event.ends_with("_sent"))
        {
            tracing::info!(
                event,
                "Skipping confirmation audit event because previous action failed"
            );

            continue;
        }

        let action = if message_delete_failed {
            if let LuaAction::SendMessage {
                channel_id,

                content,
            } = action
            {
                let adjusted_content = if content.contains("została usunięta przez AutoMod") {
                    content.replace(
                        "została usunięta przez AutoMod",
                        "narusza zasady AutoMod (usunięcie zablokowane)",
                    )
                } else if content.contains("was removed by AutoMod") {
                    content.replace(
                        "was removed by AutoMod",
                        "violates AutoMod rules (deletion blocked)",
                    )
                } else {
                    content.clone()
                };

                LuaAction::SendMessage {
                    channel_id: channel_id.clone(),

                    content: adjusted_content,
                }
            } else {
                action.clone()
            }
        } else {
            action.clone()
        };

        match &action {
            LuaAction::Reply { content, .. } => {
                let target_channel = match resolve_and_validate_channel(
                    ctx,
                    &origin,
                    None,
                    ChannelPurpose::SendMessage,
                )
                .await
                {
                    Ok(cid) => cid,

                    Err(err) => {
                        last_action_succeeded = false;

                        tracing::warn!(error = %err, module_id, "Brak lub błąd kanału dla event reply");

                        let _ = audit_action(
                            state,
                            &origin,
                            module_id,
                            "action_denied",
                            serde_json::json!({

                                "action": "reply",

                                "error": err.to_string(),

                            }),
                        )
                        .await;

                        continue;
                    }
                };

                if let Err(err) = send_message(ctx, target_channel, content).await {
                    last_action_succeeded = false;

                    tracing::warn!(error = %err, module_id, "Błąd wysyłania odpowiedzi na zdarzenie");

                    let _ = audit_action(
                        state,
                        &origin,
                        module_id,
                        "action_failed",
                        serde_json::json!({

                            "action": "reply",

                            "error": err.to_string(),

                        }),
                    )
                    .await;
                } else {
                    last_action_succeeded = true;
                }
            }

            LuaAction::DeleteMessage { .. } => {
                if let Err(err) = execute_action(ctx, state, module_id, &origin, &action).await {
                    last_action_succeeded = false;

                    message_delete_failed = true;

                    tracing::warn!(

                        error = %err,

                        module_id,

                        ?action,

                        "Błąd lub odmowa wykonania akcji zdarzenia"

                    );

                    let _ = audit_action(
                        state,
                        &origin,
                        module_id,
                        "action_denied",
                        serde_json::json!({

                            "module_id": module_id,

                            "action": format!("{action:?}"),

                            "error": err.to_string(),

                        }),
                    )
                    .await;
                } else {
                    last_action_succeeded = true;
                }
            }

            _ => {
                if let Err(err) = execute_action(ctx, state, module_id, &origin, &action).await {
                    last_action_succeeded = false;

                    tracing::warn!(

                        error = %err,

                        module_id,

                        ?action,

                        "Błąd lub odmowa wykonania akcji zdarzenia"

                    );

                    let _ = audit_action(
                        state,
                        &origin,
                        module_id,
                        "action_denied",
                        serde_json::json!({

                            "module_id": module_id,

                            "action": format!("{action:?}"),

                            "error": err.to_string(),

                        }),
                    )
                    .await;
                } else {
                    last_action_succeeded = true;
                }
            }
        }
    }

    Ok(())
}

fn action_rate_limit_scope(action: &LuaAction) -> Option<&'static str> {
    match action {
        LuaAction::Ticket {
            operation: crate::lua::TicketOperation::Open { .. },
        } => Some("ticket_create"),

        LuaAction::Scheduler {
            operation: crate::lua::SchedulerOperation::Create { .. },
        } => Some("scheduler_create"),

        LuaAction::Music {
            operation: crate::lua::MusicOperation::Play,
            ..
        } => Some("music_play"),

        LuaAction::Giveaway {
            operation: crate::lua::GiveawayOperation::Start { .. },
        } => Some("giveaway_start"),

        LuaAction::Tag {
            operation: crate::lua::TagOperation::Create { .. },
        } => Some("tag_create"),

        LuaAction::ReactionRole { .. } => Some("reaction_role"),
        LuaAction::Economy { .. } => Some("economy"),

        LuaAction::BanMember { .. }
        | LuaAction::KickMember { .. }
        | LuaAction::TimeoutMember { .. }
        | LuaAction::DeleteMessage { .. }
        | LuaAction::Purge { .. }
        | LuaAction::ModerationCase { .. } => Some("destructive_admin"),

        _ => None,
    }
}

async fn execute_action(
    ctx: &Context,

    state: &AppState,

    module_id: &str,

    origin: &ActionOrigin,

    action: &LuaAction,
) -> Result<Option<String>> {
    // Action-level rate limits apply to interactive commands, exempting automated system/event actions

    if origin.enforce_actor_permissions
        && let Some(scope) = action_rate_limit_scope(action)
        && let Some(actor_id) = origin.actor_id
    {
        let guild_id_str = origin.guild_id.map(|g| g.get().to_string());

        let decision = state.rate_limiter.check_action(
            scope,
            &actor_id.get().to_string(),
            guild_id_str.as_deref(),
        );

        if !decision.allowed {
            state
                .metrics
                .rate_limit_throttled_total
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

            return Err(anyhow!(
                "Przekroczono limit dla operacji `{scope}` (odczekaj {} s)",
                decision.reset_after.as_secs().max(1)
            ));
        }
    }

    if let Some(required) = required_permission(action) {
        if origin.enforce_actor_permissions && !has_permission(origin.actor_permissions, required) {
            return Err(anyhow!(
                "wywołujący nie ma wymaganych uprawnień: {required:?}"
            ));
        }

        if origin.enforce_actor_permissions && !has_permission(origin.app_permissions, required) {
            return Err(anyhow!("bot nie ma wymaganych uprawnień: {required:?}"));
        }
    }

    if let Some(guild_id) = origin.guild_id {
        let action_key = action.policy_action_key();
        let guild_id_str = guild_id.get().to_string();

        let db_revision = state
            .storage
            .get_policy_revision(&guild_id_str)
            .await
            .map_err(|e| anyhow!("Nie udało się pobrać wersji polityk: {e}"))?;

        let rules = match state.policy_engine.get_cached_rules(&guild_id_str) {
            Some((rev, cached)) if rev >= db_revision => cached,
            _ => {
                let fetched = state
                    .storage
                    .list_policy_rules(&guild_id_str)
                    .await
                    .map_err(|e| anyhow!("Nie udało się wczytać reguł polityk: {e}"))?;
                state
                    .policy_engine
                    .set_cached_rules(&guild_id_str, db_revision, fetched.clone());
                fetched
            }
        };

        let user_id_str = origin
            .actor_id
            .map(|u| u.get().to_string())
            .unwrap_or_default();
        let channel_id_str = origin
            .channel_id
            .map(|c| c.get().to_string())
            .unwrap_or_default();
        let is_owner = if let Some(actor_id) = origin.actor_id {
            ctx.cache
                .guild(guild_id)
                .is_some_and(|g| g.owner_id == actor_id)
        } else {
            false
        };
        let is_admin = origin
            .actor_permissions
            .contains(serenity::all::Permissions::ADMINISTRATOR);

        let roles_with_pos = if let Some(guild) = ctx.cache.guild(guild_id) {
            origin
                .actor_roles
                .iter()
                .map(|rid| {
                    let pos = guild.roles.get(rid).map(|r| r.position as i64).unwrap_or(0);
                    (rid.get().to_string(), pos)
                })
                .collect()
        } else {
            Vec::new()
        };

        let policy_ctx = crate::policy::PolicyContext {
            guild_id: guild_id_str,
            user_id: user_id_str,
            roles: roles_with_pos,
            channel_id: channel_id_str,
            thread_parent_id: None,
            voice_channel_id: None,
            is_guild_owner: is_owner,
            is_administrator: is_admin,
        };

        let resource = crate::policy::PolicyResource::Action {
            action_key: action_key.to_string(),
        };

        let outcome = state
            .policy_engine
            .evaluate(&policy_ctx, &resource, &rules, true);
        if !outcome.allowed {
            return Err(anyhow!(
                "Akcja `{action_key}` została zablokowana przez politykę serwera: {}",
                outcome.reason
            ));
        }
    }

    match action {
        LuaAction::Reply { .. } => Ok(None),

        LuaAction::SendMessage {
            channel_id,

            content,
        } => {
            let channel_id = resolve_and_validate_channel(
                ctx,
                origin,
                channel_id.as_deref(),
                ChannelPurpose::SendMessage,
            )
            .await?;

            send_message(ctx, channel_id, content).await?;

            Ok(None)
        }

        LuaAction::SendDirectMessage { user_id, content } => {
            let target_user = UserId::new(parse_snowflake(user_id)?);

            match target_user.create_dm_channel(&ctx.http).await {
                Ok(dm_channel) => {
                    if let Err(err) = dm_channel
                        .send_message(
                            &ctx.http,
                            CreateMessage::new()
                                .content(content)
                                .allowed_mentions(CreateAllowedMentions::new()),
                        )
                        .await
                    {
                        tracing::warn!(

                            user_id = %user_id,

                            error = %err,

                            "Failed to deliver direct message"

                        );

                        anyhow::bail!(
                            "nie udało się dostarczyć wiadomości prywatnej do {user_id}: {err}"
                        );
                    } else {
                        audit_action(
                            state,
                            origin,
                            module_id,
                            "send_direct_message",
                            serde_json::json!({

                                "target_user_id": user_id,

                            }),
                        )
                        .await?;
                    }
                }

                Err(err) => {
                    tracing::warn!(

                        user_id = %user_id,

                        error = %err,

                        "Failed to create DM channel"

                    );

                    anyhow::bail!("nie udało się otworzyć kanału prywatnego do {user_id}: {err}");
                }
            }

            Ok(None)
        }

        LuaAction::DeleteMessage {
            channel_id,

            message_id,
        } => {
            let channel_id = resolve_and_validate_channel(
                ctx,
                origin,
                Some(channel_id.as_str()),
                ChannelPurpose::ManageMessages,
            )
            .await?;

            let guild_id = origin.guild_id.unwrap_or(GuildId::new(1));
            let message_id = MessageId::new(parse_snowflake(message_id)?);

            state
                .delete_coordinator
                .delete_message(ctx.http.clone(), guild_id, channel_id, message_id)
                .await?;

            Ok(None)
        }

        LuaAction::TimeoutMember {
            user_id,

            seconds,

            reason,
        } => {
            let guild_id = require_guild(origin)?;

            let target_id = UserId::new(parse_snowflake(user_id)?);

            let authorizing_actor = if origin.enforce_actor_permissions {
                origin.actor_id
            } else {
                None
            };

            ensure_member_hierarchy(ctx, guild_id, authorizing_actor, target_id).await?;

            let until = (Utc::now() + Duration::seconds(i64::try_from(*seconds)?)).to_rfc3339();

            guild_id
                .edit_member(
                    &ctx.http,
                    target_id,
                    EditMember::new().disable_communication_until(until),
                )
                .await?;

            audit_action(
                state,
                origin,
                module_id,
                "timeout_member",
                serde_json::json!({

                    "target_user_id": user_id,

                    "seconds": seconds,

                    "reason": reason,

                }),
            )
            .await?;

            Ok(None)
        }

        LuaAction::KickMember { user_id, reason } => {
            let guild_id = require_guild(origin)?;

            let target_id = UserId::new(parse_snowflake(user_id)?);

            let authorizing_actor = if origin.enforce_actor_permissions {
                origin.actor_id
            } else {
                None
            };

            ensure_member_hierarchy(ctx, guild_id, authorizing_actor, target_id).await?;

            guild_id
                .kick_with_reason(
                    &ctx.http,
                    target_id,
                    reason.as_deref().unwrap_or("Rusty Discord Bot Lua module"),
                )
                .await?;

            audit_action(
                state,
                origin,
                module_id,
                "kick_member",
                serde_json::json!({

                    "target_user_id": user_id,

                    "reason": reason,

                }),
            )
            .await?;

            Ok(None)
        }

        LuaAction::BanMember {
            user_id,

            delete_message_days,

            reason,
        } => {
            let guild_id = require_guild(origin)?;

            let target_id = UserId::new(parse_snowflake(user_id)?);

            let authorizing_actor = if origin.enforce_actor_permissions {
                origin.actor_id
            } else {
                None
            };

            ensure_member_hierarchy(ctx, guild_id, authorizing_actor, target_id).await?;

            guild_id
                .ban_with_reason(
                    &ctx.http,
                    target_id,
                    *delete_message_days,
                    reason.as_deref().unwrap_or("Rusty Discord Bot Lua module"),
                )
                .await?;

            audit_action(
                state,
                origin,
                module_id,
                "ban_member",
                serde_json::json!({

                    "target_user_id": user_id,

                    "delete_message_days": delete_message_days,

                    "reason": reason,

                }),
            )
            .await?;

            Ok(None)
        }

        LuaAction::AddRole {
            user_id,

            role_id,

            reason,
        } => {
            let guild_id = require_guild(origin)?;

            let user_id = UserId::new(parse_snowflake(user_id)?);

            let role_id = RoleId::new(parse_snowflake(role_id)?);

            if origin.enforce_actor_permissions {
                ensure_member_hierarchy(ctx, guild_id, origin.actor_id, user_id).await?;

                ensure_role_hierarchy(ctx, guild_id, origin.actor_id, role_id).await?;
            } else {
                let role = ensure_role_hierarchy_with_role(ctx, guild_id, None, role_id).await?;
                if crate::discord::reaction_roles::is_privileged_role_permissions(role.permissions)
                {
                    return Err(anyhow!(
                        "event-based automation cannot grant privileged role (permissions contain administrative rights)"
                    ));
                }
            }

            let member = guild_id.member(&ctx.http, user_id).await?;

            member.add_role(&ctx.http, role_id).await?;

            audit_action(
                state,
                origin,
                module_id,
                "add_role",
                serde_json::json!({

                    "target_user_id": user_id,

                    "role_id": role_id,

                    "reason": reason,

                }),
            )
            .await?;

            Ok(None)
        }

        LuaAction::RemoveRole {
            user_id,

            role_id,

            reason,
        } => {
            let guild_id = require_guild(origin)?;

            let user_id = UserId::new(parse_snowflake(user_id)?);

            let role_id = RoleId::new(parse_snowflake(role_id)?);

            if origin.enforce_actor_permissions {
                ensure_member_hierarchy(ctx, guild_id, origin.actor_id, user_id).await?;

                ensure_role_hierarchy(ctx, guild_id, origin.actor_id, role_id).await?;
            } else {
                ensure_role_hierarchy(ctx, guild_id, None, role_id).await?;
            }

            let member = guild_id.member(&ctx.http, user_id).await?;

            member.remove_role(&ctx.http, role_id).await?;

            audit_action(
                state,
                origin,
                module_id,
                "remove_role",
                serde_json::json!({

                    "target_user_id": user_id,

                    "role_id": role_id,

                    "reason": reason,

                }),
            )
            .await?;

            Ok(None)
        }

        LuaAction::Purge { channel_id, amount } => {
            let channel_id = resolve_and_validate_channel(
                ctx,
                origin,
                channel_id.as_deref(),
                ChannelPurpose::ManageMessages,
            )
            .await?;

            let messages = channel_id
                .messages(&ctx.http, GetMessages::new().limit(*amount))
                .await?;

            let message_ids = messages
                .iter()
                .map(|message| message.id)
                .collect::<Vec<_>>();

            let guild_id = origin.guild_id.unwrap_or(GuildId::new(1));
            let outcome = state
                .delete_coordinator
                .delete_messages(ctx.http.clone(), guild_id, channel_id, message_ids)
                .await?;

            audit_action(
                state,
                origin,
                module_id,
                "purge",
                serde_json::json!({

                    "channel_id": channel_id,

                    "requested_amount": amount,

                    "deleted_amount": outcome.deleted,

                }),
            )
            .await?;

            Ok(None)
        }

        LuaAction::Cleanup {
            channel_id,
            scan_limit,
            delete_limit,
            filter,
            dry_run,
        } => {
            let guild_id = require_guild(origin)?;
            let actor_id = origin
                .actor_id
                .ok_or_else(|| anyhow!("cleanup actions require a user context"))?;
            let channel_id = resolve_and_validate_channel(
                ctx,
                origin,
                channel_id.as_deref(),
                ChannelPurpose::ManageMessages,
            )
            .await?;

            let fetch_limit = (*scan_limit).clamp(1, 100);
            let mut get_messages = GetMessages::new().limit(fetch_limit as u8);
            if let Some(ref bid) = filter.before_message_id {
                let parsed = bid
                    .parse::<u64>()
                    .map_err(|e| anyhow!("invalid before_message_id: {e}"))?;
                get_messages = get_messages.before(MessageId::new(parsed));
            }
            if let Some(ref aid) = filter.after_message_id {
                let parsed = aid
                    .parse::<u64>()
                    .map_err(|e| anyhow!("invalid after_message_id: {e}"))?;
                get_messages = get_messages.after(MessageId::new(parsed));
            }

            let messages = channel_id.messages(&ctx.http, get_messages).await?;
            let two_weeks_ago = Utc::now() - chrono::Duration::days(14);

            let mut scanned = 0usize;
            let mut matched_messages = Vec::new();
            let mut older_than_bulk_window = 0usize;
            let mut bulk_deletable = 0usize;
            let mut pinned_skipped = 0usize;
            let mut seen_duplicates = std::collections::HashSet::new();

            for msg in &messages {
                scanned += 1;

                if !filter.include_pinned && msg.pinned {
                    pinned_skipped += 1;
                    continue;
                }

                if let Some(ref auth) = filter.author_id
                    && msg.author.id.to_string() != *auth
                {
                    continue;
                }

                if filter.bots_only && !msg.author.bot {
                    continue;
                }

                if filter.humans_only && msg.author.bot {
                    continue;
                }

                if let Some(ref sub) = filter.contains
                    && !msg.content.to_lowercase().contains(&sub.to_lowercase())
                {
                    continue;
                }

                let contains_link = msg.content.contains("http://")
                    || msg.content.contains("https://")
                    || msg.content.contains("discord.gg/");
                if let Some(hl) = filter.has_link
                    && hl != contains_link
                {
                    continue;
                }

                if let Some(ha) = filter.has_attachment
                    && ha != (!msg.attachments.is_empty())
                {
                    continue;
                }

                if filter.duplicate_only {
                    let normalized = msg.content.trim().to_lowercase();
                    if !seen_duplicates.insert(normalized) {
                        // Seen duplicate
                    } else {
                        // First time seen, keep original
                        continue;
                    }
                }

                let msg_created = crate::storage::snowflake_created_at(msg.id.get());
                if msg_created < two_weeks_ago {
                    older_than_bulk_window += 1;
                } else {
                    bulk_deletable += 1;
                }

                matched_messages.push(msg);
                if matched_messages.len() >= *delete_limit as usize {
                    break;
                }
            }

            let matched_ids = matched_messages
                .iter()
                .map(|m| m.id.to_string())
                .collect::<Vec<_>>();

            if *dry_run {
                let plan = state
                    .storage
                    .create_cleanup_plan(
                        &guild_id.to_string(),
                        &channel_id.to_string(),
                        &actor_id.to_string(),
                        &matched_ids,
                        60,
                    )
                    .await?;

                audit_action(
                    state,
                    origin,
                    module_id,
                    "cleanup_preview",
                    serde_json::json!({
                        "channel_id": channel_id.to_string(),
                        "scanned": scanned,
                        "matched": matched_ids.len(),
                        "bulk_deletable": bulk_deletable,
                        "older_than_bulk_window": older_than_bulk_window,
                        "pinned_skipped": pinned_skipped,
                        "plan_id": plan.id.to_string(),
                        "expires_in_seconds": 60,
                    }),
                )
                .await?;

                let reply_text = format!(
                    "**Cleanup Preview**\n• Scanned: {scanned}\n• Matched: {}\n• Bulk deletable (<14d): {bulk_deletable}\n• Older than 14d: {older_than_bulk_window}\n• Pinned skipped: {pinned_skipped}\n• Plan ID: `{}`\n*To execute, run confirm within 60s.*",
                    matched_ids.len(),
                    plan.id
                );

                send_message(ctx, channel_id, &reply_text).await?;
                return Ok(None);
            }

            // Direct execution
            let ids_to_delete = matched_messages.iter().map(|m| m.id).collect::<Vec<_>>();
            let (deleted_count, failed_count) =
                delete_partitioned_messages(ctx, channel_id, &ids_to_delete, two_weeks_ago).await?;

            audit_action(
                state,
                origin,
                module_id,
                "cleanup_executed",
                serde_json::json!({
                    "channel_id": channel_id.to_string(),
                    "scanned": scanned,
                    "matched": matched_ids.len(),
                    "deleted_count": deleted_count,
                    "failed_count": failed_count,
                }),
            )
            .await?;

            let reply_text = format!(
                "**Cleanup Complete**\n• Successfully deleted: {deleted_count}\n• Failed: {failed_count}"
            );
            send_message(ctx, channel_id, &reply_text).await?;

            Ok(None)
        }

        LuaAction::ConfirmCleanup { plan_id } => {
            let guild_id = require_guild(origin)?;
            let actor_id = origin
                .actor_id
                .ok_or_else(|| anyhow!("cleanup confirmation requires a user context"))?;
            let plan_uuid =
                Uuid::parse_str(plan_id).map_err(|e| anyhow!("invalid plan_id: {e}"))?;

            let plan = state
                .storage
                .consume_cleanup_plan(&guild_id.to_string(), plan_uuid, &actor_id.to_string())
                .await?
                .ok_or_else(|| anyhow!("cleanup plan not found, expired, or already consumed"))?;

            let channel_id_u64 = plan
                .channel_id
                .parse::<u64>()
                .map_err(|e| anyhow!("invalid channel_id in plan: {e}"))?;
            let channel_id = ChannelId::new(channel_id_u64);

            let msg_ids = plan
                .message_ids_vec()
                .into_iter()
                .filter_map(|s| s.parse::<u64>().ok().map(MessageId::new))
                .collect::<Vec<_>>();

            let two_weeks_ago = Utc::now() - chrono::Duration::days(14);
            let (deleted_count, failed_count) =
                delete_partitioned_messages(ctx, channel_id, &msg_ids, two_weeks_ago).await?;

            audit_action(
                state,
                origin,
                module_id,
                "cleanup_confirmed",
                serde_json::json!({
                    "channel_id": plan.channel_id,
                    "plan_id": plan.id.to_string(),
                    "total_requested": msg_ids.len(),
                    "deleted_count": deleted_count,
                    "failed_count": failed_count,
                }),
            )
            .await?;

            let reply_text = format!(
                "**Cleanup Complete**\n• Successfully deleted: {deleted_count}\n• Failed: {failed_count}"
            );
            send_message(ctx, channel_id, &reply_text).await?;

            Ok(None)
        }

        LuaAction::Music { operation, query } => {
            let guild_id = require_guild(origin)?;

            let actor_id = origin
                .actor_id
                .ok_or_else(|| anyhow!("music actions require a user context"))?;

            let status =
                music::execute(ctx, state, guild_id, actor_id, *operation, query.as_deref())
                    .await?;

            audit_action(
                state,
                origin,
                module_id,
                "music",
                serde_json::json!({

                    "operation": operation,

                    "query": query,

                }),
            )
            .await?;

            Ok(Some(status))
        }

        LuaAction::Progression { operation } => {
            progression::execute(
                ctx,
                state,
                module_id,
                progression::ProgressionExecutionContext::new(
                    origin.guild_id,
                    origin.channel_id,
                    origin.actor_id,
                    origin.actor_permissions,
                    origin.enforce_actor_permissions,
                ),
                operation,
            )
            .await
        }

        LuaAction::Scheduler { operation } => {
            scheduler::execute(
                ctx,
                state,
                module_id,
                scheduler::SchedulerExecutionContext::new(
                    origin.guild_id,
                    origin.channel_id,
                    origin.actor_id,
                    origin.actor_permissions,
                    origin.enforce_actor_permissions,
                ),
                operation,
            )
            .await
        }

        LuaAction::Ticket { operation } => {
            tickets::execute(
                ctx,
                state,
                module_id,
                tickets::TicketExecutionContext::new(
                    origin.guild_id,
                    origin.channel_id,
                    origin.actor_id,
                    origin.actor_permissions,
                    origin.app_permissions,
                    origin.enforce_actor_permissions,
                ),
                operation,
            )
            .await
        }

        LuaAction::ModerationCase { operation } => {
            moderation_cases::execute(
                ctx,
                state,
                module_id,
                moderation_cases::ModerationExecutionContext::new(
                    origin.guild_id,
                    origin.channel_id,
                    origin.actor_id,
                    origin.actor_permissions,
                    origin.app_permissions,
                    origin.enforce_actor_permissions,
                ),
                operation,
            )
            .await
        }

        LuaAction::Giveaway { operation } => {
            giveaway::execute(
                ctx,
                state,
                module_id,
                giveaway::GiveawayExecutionContext::new(
                    origin.guild_id,
                    origin.channel_id,
                    origin.actor_id,
                    origin.actor_permissions,
                    origin.app_permissions,
                    origin.enforce_actor_permissions,
                ),
                operation,
            )
            .await
        }

        LuaAction::Tag { operation } => {
            tags::execute_tag_action(
                ctx,
                state,
                module_id,
                tags::TagExecutionContext::new(
                    origin.guild_id,
                    origin.channel_id,
                    origin.actor_id,
                    origin.actor_permissions,
                    origin.app_permissions,
                    origin.enforce_actor_permissions,
                ),
                operation,
            )
            .await
        }

        LuaAction::ReactionRole { operation } => {
            reaction_roles::execute_reaction_role_action(
                ctx,
                state,
                module_id,
                reaction_roles::ReactionRoleExecutionContext::new(
                    origin.guild_id,
                    origin.channel_id,
                    origin.actor_id,
                    origin.actor_permissions,
                    origin.app_permissions,
                    origin.enforce_actor_permissions,
                ),
                operation,
            )
            .await
        }

        LuaAction::Economy { operation } => {
            economy::execute(
                ctx,
                state,
                module_id,
                economy::EconomyExecutionContext {
                    guild_id: origin.guild_id,
                    channel_id: origin.channel_id,
                    actor_id: origin.actor_id,
                    actor_permissions: origin.actor_permissions,
                    enforce_actor_permissions: origin.enforce_actor_permissions,
                    interaction_id: origin.interaction_id.clone(),
                },
                operation,
            )
            .await
        }

        LuaAction::PublishSubmission {
            channel_id,
            content,
            success_reply,
            audit_event,
            audit_data,
        } => {
            let target_channel = resolve_and_validate_channel(
                ctx,
                origin,
                Some(channel_id),
                ChannelPurpose::SendMessage,
            )
            .await?;

            let delivered = target_channel
                .send_message(
                    &ctx.http,
                    CreateMessage::new()
                        .content(content)
                        .allowed_mentions(CreateAllowedMentions::new()),
                )
                .await?;

            let mut audit_map = audit_data.clone();
            audit_map.insert(
                "message_id".to_string(),
                serde_json::Value::String(delivered.id.get().to_string()),
            );
            audit_map.insert(
                "channel_id".to_string(),
                serde_json::Value::String(delivered.channel_id.get().to_string()),
            );

            let _ = state
                .storage
                .record_audit(
                    origin.guild_id.map(|id| id.get().to_string()).as_deref(),
                    origin.actor_id.map(|id| id.get().to_string()).as_deref(),
                    module_id,
                    audit_event,
                    serde_json::Value::Object(audit_map.into_iter().collect()),
                )
                .await;

            Ok(Some(success_reply.clone()))
        }

        LuaAction::Audit { event, data } => {
            state
                .storage
                .record_audit(
                    origin.guild_id.map(|id| id.get().to_string()).as_deref(),
                    origin.actor_id.map(|id| id.get().to_string()).as_deref(),
                    module_id,
                    event,
                    serde_json::to_value(data)?,
                )
                .await?;

            if let Some(gid) = origin.guild_id {
                let _ = state.guild_events.send(crate::state::GuildRealtimeEvent {
                    guild_id: gid.get().to_string(),

                    event_type: "audit_entry".to_string(),

                    payload: serde_json::json!({

                        "module_id": module_id,

                        "event": event,

                    }),
                });
            }

            Ok(None)
        }
    }
}

async fn send_message(ctx: &Context, channel_id: ChannelId, content: &str) -> Result<()> {
    channel_id
        .send_message(
            &ctx.http,
            CreateMessage::new()
                .content(content)
                .allowed_mentions(CreateAllowedMentions::new()),
        )
        .await?;

    Ok(())
}

fn required_permission(action: &LuaAction) -> Option<Permissions> {
    match action {
        LuaAction::DeleteMessage { .. } | LuaAction::Purge { .. } => {
            Some(Permissions::MANAGE_MESSAGES)
        }

        LuaAction::Tag {
            operation: TagOperation::Create { .. } | TagOperation::Delete { .. },
        } => Some(Permissions::MANAGE_MESSAGES),

        LuaAction::TimeoutMember { .. } => Some(Permissions::MODERATE_MEMBERS),

        LuaAction::KickMember { .. } => Some(Permissions::KICK_MEMBERS),

        LuaAction::BanMember { .. } => Some(Permissions::BAN_MEMBERS),

        LuaAction::AddRole { .. }
        | LuaAction::RemoveRole { .. }
        | LuaAction::ReactionRole { .. } => Some(Permissions::MANAGE_ROLES),

        _ => None,
    }
}

fn has_permission(actual: Permissions, required: Permissions) -> bool {
    actual.contains(Permissions::ADMINISTRATOR) || actual.contains(required)
}

fn require_guild(origin: &ActionOrigin) -> Result<GuildId> {
    origin
        .guild_id
        .ok_or_else(|| anyhow!("this action can only be performed on a server"))
}

pub async fn resolve_and_validate_channel(
    ctx: &Context,

    origin: &ActionOrigin,

    channel_value: Option<&str>,

    purpose: ChannelPurpose,
) -> Result<ChannelId> {
    let channel_id = match channel_value {
        Some(value) => ChannelId::new(parse_snowflake(value)?),

        None => origin
            .channel_id
            .ok_or_else(|| anyhow!("missing channel for this action"))?,
    };

    let channel = channel_id
        .to_channel(ctx)
        .await
        .with_context(|| format!("cannot read data for channel {channel_id}"))?;

    match channel {
        serenity::model::channel::Channel::Guild(guild_channel) => {
            if let Some(expected_guild_id) = origin.guild_id
                && guild_channel.guild_id != expected_guild_id
            {
                return Err(anyhow!(
                    "target channel {channel_id} belongs to a different server {}",
                    guild_channel.guild_id
                ));
            }

            match purpose {
                ChannelPurpose::SendMessage => {
                    if guild_channel.kind == ChannelType::Category {
                        return Err(anyhow!(
                            "channel categories cannot be the target of messages"
                        ));
                    }
                }

                ChannelPurpose::ManageMessages => {
                    if guild_channel.kind == ChannelType::Category {
                        return Err(anyhow!(
                            "channel categories cannot be the target of message deletions"
                        ));
                    }
                }
            }

            if origin.enforce_actor_permissions {
                let mut perms = origin.actor_permissions;

                if !perms.contains(Permissions::ADMINISTRATOR)
                    && let Some(actor_id) = origin.actor_id
                {
                    let mut role_deny = Permissions::empty();

                    let mut role_allow = Permissions::empty();

                    let mut member_deny = Permissions::empty();

                    let mut member_allow = Permissions::empty();

                    for overwrite in &guild_channel.permission_overwrites {
                        match overwrite.kind {
                            serenity::all::PermissionOverwriteType::Role(role_id) => {
                                if role_id.get() == guild_channel.guild_id.get() {
                                    perms.remove(overwrite.deny);

                                    perms.insert(overwrite.allow);
                                } else if origin.actor_roles.contains(&role_id) {
                                    role_deny.insert(overwrite.deny);

                                    role_allow.insert(overwrite.allow);
                                }
                            }

                            serenity::all::PermissionOverwriteType::Member(user_id)
                                if user_id.get() == actor_id.get() =>
                            {
                                member_deny.insert(overwrite.deny);

                                member_allow.insert(overwrite.allow);
                            }

                            _ => {}
                        }
                    }

                    perms.remove(role_deny);

                    perms.insert(role_allow);

                    perms.remove(member_deny);

                    perms.insert(member_allow);
                }

                match purpose {
                    ChannelPurpose::SendMessage => {
                        if !perms.contains(Permissions::SEND_MESSAGES)
                            || (origin.actor_permissions.contains(Permissions::VIEW_CHANNEL)
                                && !perms.contains(Permissions::VIEW_CHANNEL))
                        {
                            return Err(anyhow!(
                                "missing user permissions to send messages in channel {channel_id}"
                            ));
                        }
                    }

                    ChannelPurpose::ManageMessages => {
                        if !perms.contains(Permissions::MANAGE_MESSAGES)
                            || (origin.actor_permissions.contains(Permissions::VIEW_CHANNEL)
                                && !perms.contains(Permissions::VIEW_CHANNEL))
                        {
                            return Err(anyhow!(
                                "missing user permissions to manage messages in channel {channel_id}"
                            ));
                        }
                    }
                }
            }

            Ok(guild_channel.id)
        }

        serenity::model::channel::Channel::Private(private_channel) => {
            if origin.guild_id.is_some() {
                return Err(anyhow!(
                    "private channel {channel_id} does not belong to this server"
                ));
            }

            Ok(private_channel.id)
        }

        _ => Err(anyhow!("unsupported channel type {channel_id}")),
    }
}

pub(crate) fn parse_snowflake(value: &str) -> Result<u64> {
    let parsed = value
        .parse::<u64>()
        .with_context(|| format!("{value:?} is not a valid Discord identifier"))?;

    if parsed == 0 {
        return Err(anyhow!("Discord identifier cannot be zero"));
    }

    Ok(parsed)
}

pub async fn ensure_member_hierarchy(
    ctx: &Context,

    guild_id: GuildId,

    actor_id: Option<UserId>,

    target_id: UserId,
) -> Result<()> {
    let guild = guild_id.to_partial_guild(&ctx.http).await?;

    if target_id == guild.owner_id {
        return Err(anyhow!("the server owner cannot be moderated"));
    }

    let target = guild_id.member(&ctx.http, target_id).await?;

    let bot_id = {
        let id = ctx.cache.current_user().id;

        if id.get() == 0 {
            ctx.http
                .get_current_user()
                .await
                .map(|u| u.id)
                .unwrap_or(id)
        } else {
            id
        }
    };

    let bot = guild_id.member(&ctx.http, bot_id).await?;

    let target_rank = member_rank(&target, &guild.roles);

    let bot_rank = member_rank(&bot, &guild.roles);

    if !rank_is_higher(bot_rank, target_rank) {
        return Err(anyhow!(
            "the bot's highest role must be higher than the target member's role"
        ));
    }

    if let Some(actor_id) = actor_id {
        if actor_id == target_id {
            return Err(anyhow!("you cannot perform this action on yourself"));
        }

        if actor_id != guild.owner_id {
            let actor = guild_id.member(&ctx.http, actor_id).await?;

            if !rank_is_higher(member_rank(&actor, &guild.roles), target_rank) {
                return Err(anyhow!(
                    "your highest role must be higher than the target member's role"
                ));
            }
        }
    }

    Ok(())
}

pub async fn ensure_role_hierarchy_with_role(
    ctx: &Context,

    guild_id: GuildId,

    actor_id: Option<UserId>,

    role_id: RoleId,
) -> Result<Role> {
    let guild = guild_id.to_partial_guild(&ctx.http).await?;

    let role = guild
        .roles
        .get(&role_id)
        .ok_or_else(|| anyhow!("role does not exist on this server"))?;

    if role.managed {
        return Err(anyhow!("managed roles cannot be manually modified"));
    }

    let target_rank = role_rank(role);

    let bot_id = {
        let id = ctx.cache.current_user().id;

        if id.get() == 0 {
            ctx.http
                .get_current_user()
                .await
                .map(|u| u.id)
                .unwrap_or(id)
        } else {
            id
        }
    };

    let bot = guild_id.member(&ctx.http, bot_id).await?;

    if !rank_is_higher(member_rank(&bot, &guild.roles), target_rank) {
        return Err(anyhow!(
            "the bot's highest role must be above the managed role"
        ));
    }

    if let Some(actor_id) = actor_id
        && actor_id != guild.owner_id
    {
        let actor = guild_id.member(&ctx.http, actor_id).await?;

        if !rank_is_higher(member_rank(&actor, &guild.roles), target_rank) {
            return Err(anyhow!(
                "twoja najwyższa rola musi znajdować się nad zarządzaną rolą"
            ));
        }
    }

    Ok(role.clone())
}

pub async fn ensure_role_hierarchy(
    ctx: &Context,

    guild_id: GuildId,

    actor_id: Option<UserId>,

    role_id: RoleId,
) -> Result<()> {
    ensure_role_hierarchy_with_role(ctx, guild_id, actor_id, role_id)
        .await
        .map(|_| ())
}

fn member_rank(member: &Member, roles: &HashMap<RoleId, Role>) -> (u16, u64) {
    member
        .roles
        .iter()
        .filter_map(|role_id| roles.get(role_id))
        .map(role_rank)
        .max_by(|left, right| compare_rank(*left, *right))
        .unwrap_or((0, member.guild_id.get()))
}

fn role_rank(role: &Role) -> (u16, u64) {
    (role.position, role.id.get())
}

fn rank_is_higher(left: (u16, u64), right: (u16, u64)) -> bool {
    compare_rank(left, right).is_gt()
}

fn compare_rank(left: (u16, u64), right: (u16, u64)) -> std::cmp::Ordering {
    left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1))
}

async fn audit_action(
    state: &AppState,

    origin: &ActionOrigin,

    module_id: &str,

    event: &str,

    data: serde_json::Value,
) -> Result<()> {
    let guild_id = origin.guild_id.map(|id| id.get().to_string());

    let actor_id = origin.actor_id.map(|id| id.get().to_string());

    state
        .storage
        .record_audit(
            guild_id.as_deref(),
            actor_id.as_deref(),
            module_id,
            event,
            data,
        )
        .await?;

    if let Some(ref gid) = guild_id {
        let _ = state.guild_events.send(crate::state::GuildRealtimeEvent {
            guild_id: gid.clone(),

            event_type: "audit_entry".to_string(),

            payload: serde_json::json!({

                "module_id": module_id,

                "event": event,

            }),
        });
    }

    Ok(())
}

async fn delete_partitioned_messages(
    ctx: &Context,
    channel_id: ChannelId,
    message_ids: &[MessageId],
    two_weeks_ago: chrono::DateTime<Utc>,
) -> Result<(usize, usize)> {
    let mut bulk_ids = Vec::new();
    let mut old_ids = Vec::new();

    for &mid in message_ids {
        let created = crate::storage::snowflake_created_at(mid.get());
        if created >= two_weeks_ago {
            bulk_ids.push(mid);
        } else {
            old_ids.push(mid);
        }
    }

    let mut deleted_count = 0usize;
    let mut failed_count = 0usize;

    for chunk in bulk_ids.chunks(100) {
        if chunk.len() == 1 {
            match channel_id.delete_message(&ctx.http, chunk[0]).await {
                Ok(()) => deleted_count += 1,
                Err(err) => {
                    tracing::warn!(error = %err, message_id = %chunk[0], "Failed to delete single message in bulk window");
                    failed_count += 1;
                }
            }
        } else if chunk.len() > 1 {
            match channel_id.delete_messages(&ctx.http, chunk).await {
                Ok(()) => deleted_count += chunk.len(),
                Err(err) => {
                    tracing::warn!(error = %err, count = chunk.len(), "Failed to bulk delete messages");
                    for &mid in chunk {
                        match channel_id.delete_message(&ctx.http, mid).await {
                            Ok(()) => deleted_count += 1,
                            Err(_) => failed_count += 1,
                        }
                    }
                }
            }
        }
    }

    for &mid in &old_ids {
        match channel_id.delete_message(&ctx.http, mid).await {
            Ok(()) => deleted_count += 1,
            Err(err) => {
                tracing::warn!(error = %err, message_id = %mid, "Failed to delete older message individually");
                failed_count += 1;
            }
        }
    }

    Ok((deleted_count, failed_count))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_discord_text_cases_from_issue_146() {
        // Case 1: line with exact length of limit followed by newline and next line
        // Must NOT emit an empty first chunk, and must not exceed limit
        let res1 = chunk_discord_text("abcde\nx", 5);
        assert_eq!(res1, vec!["abcde", "\nx"]);
        assert_eq!(res1.concat(), "abcde\nx");
        assert!(res1.iter().all(|c| !c.is_empty() && c.len() <= 5));

        // Case 2: line exceeding limit followed by newline
        // Must preserve the newline separator
        let res2 = chunk_discord_text("abcdef\nz", 5);
        assert_eq!(res2, vec!["abcde", "f\nz"]);
        assert_eq!(res2.concat(), "abcdef\nz");
        assert!(res2.iter().all(|c| !c.is_empty() && c.len() <= 5));
    }

    #[test]
    fn test_chunk_discord_text_preserves_utf8_and_line_breaks_at_boundaries() {
        for text in [
            format!("{}\nkoniec", "a".repeat(1950)),
            format!("{}\nkoniec", "a".repeat(1951)),
            format!("{}\nkoniec", "ą".repeat(1000)),
            format!("{}\n", "a".repeat(1950)),
            format!("🦀{}", "x".repeat(1950)),
            "line1\nline2\nline3\nline4".to_string(),
            "".to_string(),
        ] {
            let chunks = chunk_discord_text(&text, 1950);
            if text.is_empty() {
                assert_eq!(chunks, vec![""]);
            } else {
                assert!(!chunks.is_empty());
                assert!(
                    chunks
                        .iter()
                        .all(|part| !part.is_empty() && part.len() <= 1950)
                );
            }
            assert_eq!(chunks.concat(), text);
        }
    }

    #[test]
    fn test_chunk_discord_text_multi_byte_never_slices_char() {
        let text = "🚀🦀⭐".repeat(100);
        let chunks = chunk_discord_text(&text, 10);
        for chunk in &chunks {
            assert!(chunk.len() <= 10);
            assert!(!chunk.is_empty());
        }
        assert_eq!(chunks.concat(), text);
    }
}
