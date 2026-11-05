use std::sync::atomic::{AtomicBool, Ordering};

use std::time::{Duration, Instant};

use dashmap::DashMap;

use serde_json::{Map, Value, json};

use serenity::{
    all::{
        ChannelId, CommandDataOption, CommandDataOptionValue, CommandInteraction, GuildChannel,
        GuildId, Interaction, Member, Message, MessageId, MessageUpdateEvent, PartialGuildChannel,
        Permissions, Ready, User, UserId, VoiceState,
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

        // Reconcile voice companion channel leases after startup/restart (#161)
        let companion_state = self.state.clone();
        let companion_ctx = ctx.clone();
        let guild_ids: Vec<GuildId> = ready.guilds.iter().map(|g| g.id).collect();
        tokio::spawn(async move {
            for gid in guild_ids {
                if let Err(e) = crate::discord::companion::reconcile_guild_companion_channels(
                    &companion_ctx,
                    &companion_state,
                    gid,
                )
                .await
                {
                    tracing::warn!(
                        guild_id = %gid,
                        ?e,
                        "Failed companion channel reconciliation during startup"
                    );
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

        let guild_id_str = guild_id.get().to_string();
        let channel_id_str = message.channel_id.get().to_string();
        let author_id_str = message.author.id.get().to_string();
        let member_roles: Vec<String> = message
            .member
            .as_ref()
            .map(|m| m.roles.iter().map(|r| r.to_string()).collect())
            .unwrap_or_default();

        let storage = self.state.storage.clone();
        tokio::spawn(async move {
            if let Ok(Some(session)) = storage
                .get_help_forum_session(&guild_id_str, &channel_id_str)
                .await
                && session.status == "open"
            {
                let is_helper = if let Ok(Some(cfg)) = storage
                    .get_help_forum_config(&guild_id_str, &session.forum_channel_id)
                    .await
                {
                    cfg.helper_role_ids
                        .0
                        .iter()
                        .any(|r| member_roles.iter().any(|mr| mr == r))
                } else {
                    false
                };
                let _ = storage
                    .record_help_forum_activity(
                        &guild_id_str,
                        &channel_id_str,
                        &author_id_str,
                        is_helper,
                    )
                    .await;
            }
        });
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

        let roles: Vec<String> = new_member
            .roles
            .iter()
            .map(|r| r.get().to_string())
            .collect();
        let joined_at = new_member
            .joined_at
            .and_then(|ts| chrono::DateTime::from_timestamp(ts.unix_timestamp(), 0));
        let g_id_str = guild_id.get().to_string();
        let u_id_str = new_member.user.id.get().to_string();

        let _ = self
            .state
            .storage
            .upsert_member_snapshot(crate::storage::UpsertMemberSnapshotParams {
                guild_id: &g_id_str,
                user_id: &u_id_str,
                username: &new_member.user.name,
                global_name: new_member.user.global_name.as_deref(),
                nickname: new_member.nick.as_deref(),
                role_ids: &roles,
                joined_at,
            })
            .await;

        let _ = self
            .state
            .storage
            .record_member_identity_change(
                &g_id_str,
                &u_id_str,
                "username",
                None,
                Some(&new_member.user.name),
                90,
            )
            .await;

        if let Some(ref nick) = new_member.nick {
            let _ = self
                .state
                .storage
                .record_member_identity_change(
                    &g_id_str,
                    &u_id_str,
                    "nickname",
                    None,
                    Some(nick),
                    90,
                )
                .await;
        }

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

    async fn guild_member_update(
        &self,
        _ctx: Context,
        old_if_available: Option<Member>,
        _new: Option<Member>,
        event: serenity::all::GuildMemberUpdateEvent,
    ) {
        self.state
            .metrics
            .discord_events_total
            .fetch_add(1, Ordering::Relaxed);

        let g_id_str = event.guild_id.get().to_string();
        let u_id_str = event.user.id.get().to_string();
        let roles: Vec<String> = event.roles.iter().map(|r| r.get().to_string()).collect();
        let joined_at = chrono::DateTime::from_timestamp(event.joined_at.unix_timestamp(), 0);

        let old_nick = old_if_available.as_ref().and_then(|m| m.nick.as_deref());
        let new_nick = event.nick.as_deref();
        if old_nick != new_nick {
            let _ = self
                .state
                .storage
                .record_member_identity_change(
                    &g_id_str, &u_id_str, "nickname", old_nick, new_nick, 90,
                )
                .await;
        }

        let old_user = old_if_available.as_ref().map(|m| &m.user);
        let old_name = old_user.map(|u| u.name.as_str());
        let new_name = event.user.name.as_str();
        if old_name.is_some_and(|old| old != new_name) {
            let _ = self
                .state
                .storage
                .record_member_identity_change(
                    &g_id_str,
                    &u_id_str,
                    "username",
                    old_name,
                    Some(new_name),
                    90,
                )
                .await;
        }

        let old_global = old_user.and_then(|u| u.global_name.as_deref());
        let new_global = event.user.global_name.as_deref();
        if old_global != new_global {
            let _ = self
                .state
                .storage
                .record_member_identity_change(
                    &g_id_str,
                    &u_id_str,
                    "global_name",
                    old_global,
                    new_global,
                    90,
                )
                .await;
        }

        let _ = self
            .state
            .storage
            .upsert_member_snapshot(crate::storage::UpsertMemberSnapshotParams {
                guild_id: &g_id_str,
                user_id: &u_id_str,
                username: &event.user.name,
                global_name: event.user.global_name.as_deref(),
                nickname: event.nick.as_deref(),
                role_ids: &roles,
                joined_at,
            })
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

        let guild_str = guild_id.get().to_string();
        let user_str = user.id.get().to_string();

        if let Err(error) = self
            .state
            .storage
            .remove_member_snapshot(&guild_str, &user_str)
            .await
        {
            tracing::error!(
                ?error,
                %guild_id,
                user_id = %user.id,
                "failed to clear member snapshot after departure"
            );
        }

        let _ = self
            .state
            .storage
            .mark_help_forum_owner_left(&guild_str, &user_str)
            .await;

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

    async fn voice_state_update(&self, ctx: Context, old: Option<VoiceState>, new: VoiceState) {
        if let Some(guild_id) = new
            .guild_id
            .or_else(|| old.as_ref().and_then(|o| o.guild_id))
        {
            let bot_id = ctx.cache.current_user().id;

            if new.user_id == bot_id
                && new.channel_id.is_none()
                && let Some(manager) = songbird::get(&ctx).await
            {
                let _ = manager.remove(guild_id).await;
            }

            if new.user_id != bot_id {
                let old_voice = old.and_then(|o| o.channel_id);
                let new_voice = new.channel_id;
                if let Err(err) = crate::discord::companion::reconcile_companion_channel_access(
                    &ctx,
                    &self.state,
                    guild_id,
                    new.user_id,
                    old_voice,
                    new_voice,
                )
                .await
                {
                    tracing::warn!(
                        guild_id = %guild_id,
                        user_id = %new.user_id,
                        ?err,
                        "Failed to reconcile companion channel access on voice state update"
                    );
                }
            }
        }
    }

    async fn thread_create(&self, ctx: Context, thread: GuildChannel) {
        let Some(parent_id) = thread.parent_id else {
            return;
        };
        let guild_id_str = thread.guild_id.get().to_string();
        let forum_id_str = parent_id.get().to_string();

        let storage = self.state.storage.clone();
        let http = ctx.http.clone();
        tokio::spawn(async move {
            if let Ok(Some(cfg)) = storage
                .get_help_forum_config(&guild_id_str, &forum_id_str)
                .await
                && cfg.enabled
            {
                let owner_id_str = thread
                    .owner_id
                    .map(|o| o.get().to_string())
                    .unwrap_or_default();
                let thread_id_str = thread.id.get().to_string();

                let session = crate::help_forum::HelpForumSession {
                    guild_id: guild_id_str,
                    forum_channel_id: forum_id_str,
                    thread_id: thread_id_str,
                    owner_user_id: owner_id_str,
                    status: "open".to_string(),
                    opened_at: chrono::Utc::now(),
                    last_activity_at: chrono::Utc::now(),
                    first_helper_response_at: None,
                    closed_at: None,
                    closing_reason: None,
                    generation: 1,
                };
                let _ = storage.create_help_forum_session(session).await;

                if let Some(welcome) = &cfg.welcome_message {
                    let _ = thread.id.say(&http, welcome).await;
                }
            }
        });
    }

    async fn thread_delete(
        &self,
        _ctx: Context,
        thread: PartialGuildChannel,
        _full: Option<GuildChannel>,
    ) {
        let guild_id_str = thread.guild_id.get().to_string();
        let thread_id_str = thread.id.get().to_string();
        let storage = self.state.storage.clone();
        tokio::spawn(async move {
            let _ = storage
                .close_help_forum_session(
                    &guild_id_str,
                    &thread_id_str,
                    "deleted",
                    Some("thread_deleted_by_discord"),
                    None,
                )
                .await;
        });
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

const fn is_thread_channel_type(kind: serenity::all::ChannelType) -> bool {
    matches!(
        kind,
        serenity::all::ChannelType::NewsThread
            | serenity::all::ChannelType::PublicThread
            | serenity::all::ChannelType::PrivateThread
    )
}

async fn resolve_thread_parent_id(
    ctx: &serenity::client::Context,
    guild_id: serenity::all::GuildId,
    channel_id: serenity::all::ChannelId,
) -> Option<String> {
    if let Some(guild) = ctx.cache.guild(guild_id) {
        if let Some(ch) = guild.channels.get(&channel_id) {
            if is_thread_channel_type(ch.kind) {
                return ch.parent_id.map(|p| p.get().to_string());
            }
            return None;
        }
        if let Some(th) = guild.threads.iter().find(|t| t.id == channel_id) {
            if is_thread_channel_type(th.kind) {
                return th.parent_id.map(|p| p.get().to_string());
            }
            return None;
        }
    }
    if let Ok(ch) = ctx.http.get_channel(channel_id).await
        && let Some(gc) = ch.guild()
        && gc.guild_id == guild_id
        && is_thread_channel_type(gc.kind)
    {
        return gc.parent_id.map(|p| p.get().to_string());
    }
    None
}

async fn execute_command_inner(
    ctx: &Context,

    state: &AppState,

    command: &CommandInteraction,

    deferred: &std::sync::Arc<std::sync::atomic::AtomicBool>,

    responded: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> anyhow::Result<()> {
    if command.data.name == "policy" {
        return handle_policy_builtin_command(ctx, state, command, deferred, responded).await;
    }

    if command.data.name == "duty" {
        return handle_duty_builtin_command(ctx, state, command, deferred, responded).await;
    }

    if command.data.name == "help" {
        return handle_help_builtin_command(ctx, state, command, deferred, responded).await;
    }

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

    if let Some(guild_id) = command.guild_id {
        let guild_id_str = guild_id.get().to_string();

        let mut roles_with_pos: Vec<(String, i64)> = Vec::new();
        let mut is_guild_owner = false;
        let mut voice_channel_id = None;

        if let Some(guild) = ctx.cache.guild(guild_id) {
            is_guild_owner = command.user.id == guild.owner_id;
            if let Some(member) = command.member.as_ref() {
                for role_id in &member.roles {
                    let pos = guild
                        .roles
                        .get(role_id)
                        .map(|r| r.position as i64)
                        .unwrap_or(0);
                    roles_with_pos.push((role_id.get().to_string(), pos));
                }
            }
            if let Some(voice_state) = guild.voice_states.get(&command.user.id) {
                voice_channel_id = voice_state.channel_id.map(|c| c.get().to_string());
            }
        } else if let Some(member) = command.member.as_ref() {
            for role_id in &member.roles {
                roles_with_pos.push((role_id.get().to_string(), 0));
            }
        }

        let is_administrator = command
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.contains(serenity::all::Permissions::ADMINISTRATOR))
            .unwrap_or(false);

        let thread_parent_id = resolve_thread_parent_id(ctx, guild_id, command.channel_id).await;

        let policy_ctx = crate::policy::PolicyContext {
            guild_id: guild_id_str.clone(),
            user_id: user_id.clone(),
            roles: roles_with_pos,
            channel_id: command.channel_id.get().to_string(),
            thread_parent_id,
            voice_channel_id,
            is_guild_owner,
            is_administrator,
        };

        let db_revision = match state.storage.get_policy_revision(&guild_id_str).await {
            Ok(rev) => rev,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    guild_id = %guild_id_str,
                    "Failed to fetch policy revision from storage; policy fail-closed"
                );
                respond_or_edit(
                    ctx,
                    command,
                    deferred,
                    responded,
                    "Access policy service is temporarily unavailable. Please try again later.",
                )
                .await?;
                return Ok(());
            }
        };

        let rules = match state.policy_engine.get_cached_rules(&guild_id_str) {
            Some((rev, cached)) if rev >= db_revision => cached,
            _ => {
                let fetched = match state.storage.list_policy_rules(&guild_id_str).await {
                    Ok(rules) => rules,
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            guild_id = %guild_id_str,
                            "Failed to list policy rules from storage; policy fail-closed"
                        );
                        respond_or_edit(
                            ctx,
                            command,
                            deferred,
                            responded,
                            "Access policy service is temporarily unavailable. Please try again later.",
                        )
                        .await?;
                        return Ok(());
                    }
                };
                state
                    .policy_engine
                    .set_cached_rules(&guild_id_str, db_revision, fetched.clone());
                fetched
            }
        };

        let resource = crate::policy::PolicyResource::Command {
            module_id: module_id.clone(),
            command_name: command.data.name.clone(),
        };

        let outcome = state
            .policy_engine
            .evaluate(&policy_ctx, &resource, &rules, enabled);

        if !outcome.allowed {
            respond_or_edit(ctx, command, deferred, responded, &outcome.reason).await?;
            return Ok(());
        }
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

    let actions = match state
        .scripts
        .execute_command(&module_id, &command.data.name, execution_context)
        .await
    {
        Ok(actions) => actions,
        Err(err) if err.is_busy() => {
            state
                .metrics
                .lua_commands_overloaded_total
                .fetch_add(1, Ordering::Relaxed);
            let msg = crate::translations::t(&command.locale, "system.bot_busy");
            respond_or_edit(ctx, command, deferred, responded, &msg).await?;
            return Ok(());
        }
        Err(err) => return Err(err.into()),
    };

    let is_deferred = deferred.load(Ordering::SeqCst);

    responded.store(true, Ordering::SeqCst);

    executor::execute_command_actions(ctx, state, command, &module_id, &actions, is_deferred).await
}

async fn handle_policy_builtin_command(
    ctx: &Context,
    state: &AppState,
    command: &CommandInteraction,
    deferred: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    responded: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> anyhow::Result<()> {
    let Some(guild_id) = command.guild_id else {
        respond_or_edit(
            ctx,
            command,
            deferred,
            responded,
            "Policy commands can only be used in a server.",
        )
        .await?;
        return Ok(());
    };

    let guild_id_str = guild_id.get().to_string();
    let is_owner = ctx
        .cache
        .guild(guild_id)
        .map(|g| g.owner_id == command.user.id)
        .unwrap_or(false);

    let is_admin = command
        .member
        .as_ref()
        .and_then(|m| m.permissions)
        .map(|p| p.contains(serenity::all::Permissions::ADMINISTRATOR))
        .unwrap_or(false);

    if !is_owner && !is_admin {
        respond_or_edit(
            ctx,
            command,
            deferred,
            responded,
            "🔒 Only server administrators and the server owner can use policy diagnostic/recovery tools.",
        )
        .await?;
        return Ok(());
    }

    let options = options_to_json(&command.data.options);
    if let Some(reset_opts) = options.get("reset") {
        let confirmed = reset_opts
            .get("confirm")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !confirmed {
            respond_or_edit(
                ctx,
                command,
                deferred,
                responded,
                "⚠️ Resetting all command policies will remove custom rules. Pass `confirm: true` to confirm.",
            )
            .await?;
            return Ok(());
        }

        let count = state.storage.reset_policy_rules(&guild_id_str).await?;
        state.policy_engine.invalidate(&guild_id_str);
        let msg = format!("✅ Command policies reset to default ({count} custom rules removed).");
        respond_or_edit(ctx, command, deferred, responded, &msg).await?;
        return Ok(());
    }

    let explain_opts = options.get("explain").or_else(|| options.get("canrun"));
    if let Some(opts) = explain_opts {
        let target_command = opts.get("command").and_then(|v| v.as_str()).unwrap_or("");

        if target_command.is_empty() {
            respond_or_edit(
                ctx,
                command,
                deferred,
                responded,
                "Please specify the command name to inspect.",
            )
            .await?;
            return Ok(());
        }

        let module_id = state
            .scripts
            .module_for_command(target_command)
            .await
            .unwrap_or_else(|| "unknown".to_string());

        let target_user_id = opts
            .get("user")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| command.user.id.get().to_string());

        let mut roles_with_pos = Vec::new();
        if let (Some(guild), Some(member)) = (ctx.cache.guild(guild_id), command.member.as_ref()) {
            for role_id in &member.roles {
                let pos = guild
                    .roles
                    .get(role_id)
                    .map(|r| r.position as i64)
                    .unwrap_or(0);
                roles_with_pos.push((role_id.get().to_string(), pos));
            }
        }

        let thread_parent_id = resolve_thread_parent_id(ctx, guild_id, command.channel_id).await;

        let policy_ctx = crate::policy::PolicyContext {
            guild_id: guild_id_str.clone(),
            user_id: target_user_id,
            roles: roles_with_pos,
            channel_id: command.channel_id.get().to_string(),
            thread_parent_id,
            voice_channel_id: None,
            is_guild_owner: is_owner,
            is_administrator: is_admin,
        };

        let rules = match state.storage.list_policy_rules(&guild_id_str).await {
            Ok(rules) => rules,
            Err(e) => {
                let msg = format!("⚠️ Failed to fetch policy rules: {e}");
                respond_or_edit(ctx, command, deferred, responded, &msg).await?;
                return Ok(());
            }
        };
        let resource = crate::policy::PolicyResource::Command {
            module_id,
            command_name: target_command.to_string(),
        };

        let outcome = state
            .policy_engine
            .evaluate(&policy_ctx, &resource, &rules, true);
        let mut text = format!(
            "📋 **Policy Evaluation for `/{}`**:\nResult: **{}**\nReason: {}\n\n**Trace:**\n",
            target_command,
            if outcome.allowed { "ALLOWED" } else { "DENIED" },
            outcome.reason
        );
        for step in outcome.trace {
            text.push_str(&format!("- [{}] {}\n", step.step, step.description));
        }

        respond_or_edit(ctx, command, deferred, responded, &text).await?;
        return Ok(());
    }

    respond_or_edit(
        ctx,
        command,
        deferred,
        responded,
        "Usage: `/policy explain command:<name>` or `/policy reset confirm:true`",
    )
    .await?;
    Ok(())
}

async fn handle_duty_builtin_command(
    ctx: &Context,
    state: &AppState,
    command: &CommandInteraction,
    deferred: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    responded: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> anyhow::Result<()> {
    let Some(guild_id) = command.guild_id else {
        respond_or_edit(
            ctx,
            command,
            deferred,
            responded,
            "Duty commands can only be used in a server.",
        )
        .await?;
        return Ok(());
    };

    let guild_id_str = guild_id.get().to_string();
    let user_id_str = command.user.id.get().to_string();

    let settings = state
        .storage
        .get_duty_settings(&guild_id_str)
        .await?
        .unwrap_or_else(|| crate::duty::ModeratorDutySettings {
            guild_id: guild_id_str.clone(),
            staff_role_id: None,
            on_call_role_id: None,
            max_override_duration_mins: 1440,
            fallback_channel_id: None,
            updated_at: chrono::Utc::now(),
        });

    let member_roles: Vec<String> = command
        .member
        .as_ref()
        .map(|m| m.roles.iter().map(|r| r.get().to_string()).collect())
        .unwrap_or_default();

    if let Some(ref staff_role) = settings.staff_role_id
        && !member_roles.contains(staff_role)
    {
        respond_or_edit(
            ctx,
            command,
            deferred,
            responded,
            "🔒 Only staff members with the designated staff role can manage duty availability.",
        )
        .await?;
        return Ok(());
    }

    let mut profile = state
        .storage
        .get_duty_profile(&guild_id_str, &user_id_str)
        .await?
        .unwrap_or_else(|| {
            crate::duty::ModeratorDutyProfile::new(
                guild_id_str.clone(),
                user_id_str.clone(),
                "UTC".to_string(),
                crate::duty::RecurringSchedule::default(),
            )
            .unwrap()
        });

    let options = options_to_json(&command.data.options);

    if options.get("on").is_some() {
        profile.manual_override = None;
        profile.override_expires_at = None;
        let saved = state.storage.upsert_duty_profile(profile).await?;

        let outcome = crate::duty::reconcile_member_duty_state(crate::duty::ReconcileDutyParams {
            storage: &state.storage,
            guild_id: &guild_id_str,
            user_id: &user_id_str,
            settings: &settings,
            profile: &saved,
            member_roles: &member_roles,
            bot_highest_role_pos: None,
            on_call_role_pos: None,
            is_live_discord: !state.config.discord_token.starts_with("dummy")
                && state.config.discord_token != "mock"
                && state.config.discord_token != "offline",
            http: Some(&ctx.http),
        })
        .await?;

        let status_msg = if outcome.expected_on_call {
            "🟢 You are now **ON DUTY** (manual override cleared). On-call role is active."
        } else {
            "⚪ Manual override cleared. You are currently outside your scheduled duty hours."
        };
        respond_or_edit(ctx, command, deferred, responded, status_msg).await?;
        return Ok(());
    }

    if let Some(off_opts) = options.get("off") {
        let duration_str = off_opts
            .get("duration")
            .and_then(|v| v.as_str())
            .unwrap_or("2h");

        let duration_mins = parse_simple_duration_minutes(duration_str)
            .min(settings.max_override_duration_mins as u64)
            .max(1);

        let expires_at = chrono::Utc::now() + chrono::Duration::minutes(duration_mins as i64);
        profile.manual_override = Some("force_off".to_string());
        profile.override_expires_at = Some(expires_at);
        let saved = state.storage.upsert_duty_profile(profile).await?;

        let _ = crate::duty::reconcile_member_duty_state(crate::duty::ReconcileDutyParams {
            storage: &state.storage,
            guild_id: &guild_id_str,
            user_id: &user_id_str,
            settings: &settings,
            profile: &saved,
            member_roles: &member_roles,
            bot_highest_role_pos: None,
            on_call_role_pos: None,
            is_live_discord: !state.config.discord_token.starts_with("dummy")
                && state.config.discord_token != "mock"
                && state.config.discord_token != "offline",
            http: Some(&ctx.http),
        })
        .await?;

        let msg = format!(
            "🔴 You are now **OFF DUTY** (pings-off) for {duration_mins} minutes (until <t:{}:t>).",
            expires_at.timestamp()
        );
        respond_or_edit(ctx, command, deferred, responded, &msg).await?;
        return Ok(());
    }

    if options.get("status").is_some() {
        let is_staff = match settings.staff_role_id {
            Some(ref sr) => member_roles.contains(sr),
            None => true,
        };
        let is_on_call = crate::duty::compute_is_on_call(&profile, chrono::Utc::now(), is_staff);

        let override_info = match (&profile.manual_override, profile.override_expires_at) {
            (Some(o), Some(exp)) => format!("{o} (until <t:{}:R>)", exp.timestamp()),
            (Some(o), None) => o.clone(),
            _ => "None (following schedule)".to_string(),
        };

        let msg = format!(
            "👮 **Moderator Duty Status for <@{user_id_str}>**:\n\
             - **On-Call Status**: {}\n\
             - **Active Override**: {}\n\
             - **Timezone**: `{}`\n\
             - **Staff Role**: {}\n\
             - **On-Call Role**: {}",
            if is_on_call {
                "🟢 ACTIVE (Pingable)"
            } else {
                "⚪ INACTIVE"
            },
            override_info,
            profile.timezone,
            settings
                .staff_role_id
                .as_deref()
                .unwrap_or("Not configured"),
            settings
                .on_call_role_id
                .as_deref()
                .unwrap_or("Not configured")
        );
        respond_or_edit(ctx, command, deferred, responded, &msg).await?;
        return Ok(());
    }

    if let Some(sched_opts) = options.get("schedule")
        && sched_opts.get("clear").is_some()
    {
        profile.recurring_schedule = sqlx::types::Json(crate::duty::RecurringSchedule::default());
        let saved = state.storage.upsert_duty_profile(profile).await?;
        let _ = crate::duty::reconcile_member_duty_state(crate::duty::ReconcileDutyParams {
            storage: &state.storage,
            guild_id: &guild_id_str,
            user_id: &user_id_str,
            settings: &settings,
            profile: &saved,
            member_roles: &member_roles,
            bot_highest_role_pos: None,
            on_call_role_pos: None,
            is_live_discord: !state.config.discord_token.starts_with("dummy")
                && state.config.discord_token != "mock"
                && state.config.discord_token != "offline",
            http: Some(&ctx.http),
        })
        .await?;
        respond_or_edit(
            ctx,
            command,
            deferred,
            responded,
            "🗑️ Your recurring duty schedule has been cleared.",
        )
        .await?;
        return Ok(());
    }

    respond_or_edit(
        ctx,
        command,
        deferred,
        responded,
        "Usage: `/duty on`, `/duty off duration:2h`, `/duty status`, or `/duty schedule clear`",
    )
    .await?;
    Ok(())
}

fn parse_simple_duration_minutes(val: &str) -> u64 {
    let val = val.trim().to_lowercase();
    if let Some(h) = val.strip_suffix('h') {
        h.parse::<u64>().map(|v| v * 60).unwrap_or(120)
    } else if let Some(m) = val.strip_suffix('m') {
        m.parse::<u64>().unwrap_or(60)
    } else if let Some(d) = val.strip_suffix('d') {
        d.parse::<u64>().map(|v| v * 1440).unwrap_or(1440)
    } else {
        val.parse::<u64>().unwrap_or(120)
    }
}

async fn handle_help_builtin_command(
    ctx: &Context,
    state: &AppState,
    command: &CommandInteraction,
    deferred: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    responded: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> anyhow::Result<()> {
    let Some(guild_id) = command.guild_id else {
        respond_or_edit(
            ctx,
            command,
            deferred,
            responded,
            "⚠️ The `/help` command can only be used within a server.",
        )
        .await?;
        return Ok(());
    };

    let guild_id_str = guild_id.to_string();
    let thread_id_str = command.channel_id.to_string();

    let session = state
        .storage
        .get_help_forum_session(&guild_id_str, &thread_id_str)
        .await?;

    let Some(session) = session else {
        respond_or_edit(
            ctx,
            command,
            deferred,
            responded,
            "⚠️ This command can only be used inside an active Help Forum post.",
        )
        .await?;
        return Ok(());
    };

    if session.status != "open" {
        respond_or_edit(
            ctx,
            command,
            deferred,
            responded,
            &format!("ℹ️ This help post is already closed ({})", session.status),
        )
        .await?;
        return Ok(());
    }

    let config = state
        .storage
        .get_help_forum_config(&guild_id_str, &session.forum_channel_id)
        .await?
        .unwrap_or_default();

    let user_id_str = command.user.id.to_string();
    let member_roles: Vec<String> = command
        .member
        .as_ref()
        .map(|m| m.roles.iter().map(|r| r.to_string()).collect())
        .unwrap_or_default();

    let is_admin = command
        .member
        .as_ref()
        .and_then(|m| m.permissions)
        .map(|p| p.contains(Permissions::ADMINISTRATOR))
        .unwrap_or(false);

    if !crate::help_forum::can_close_session(
        &session,
        &config,
        &user_id_str,
        &member_roles,
        is_admin,
    ) {
        respond_or_edit(
            ctx,
            command,
            deferred,
            responded,
            "⛔ You do not have permission to close this help forum post. Only the post author or helpers may mark it solved.",
        )
        .await?;
        return Ok(());
    }

    let closed = state
        .storage
        .close_help_forum_session(
            &guild_id_str,
            &thread_id_str,
            "solved",
            Some("marked_solved_by_user"),
            None,
        )
        .await?;

    if closed {
        if !state.config.discord_token.starts_with("dummy")
            && state.config.discord_token != "mock"
            && state.config.discord_token != "offline"
        {
            let mut map = serde_json::Map::new();
            if config.archive_on_close {
                map.insert("archived".to_string(), serde_json::Value::Bool(true));
            }
            if config.lock_on_close {
                map.insert("locked".to_string(), serde_json::Value::Bool(true));
            }
            let _ = ctx
                .http
                .edit_channel(
                    command.channel_id,
                    &serde_json::Value::Object(map),
                    Some("Help forum marked as solved"),
                )
                .await;
        }

        respond_or_edit(
            ctx,
            command,
            deferred,
            responded,
            "✅ This help forum post has been marked as solved! Thank you.",
        )
        .await?;
    } else {
        respond_or_edit(
            ctx,
            command,
            deferred,
            responded,
            "⚠️ Unable to close the help post; it may have already been closed.",
        )
        .await?;
    }

    Ok(())
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

            Err(error) if error.is_busy() => {
                state
                    .metrics
                    .lua_events_overloaded_total
                    .fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    module_id,
                    event_name,
                    guild_id = ?guild_id.map(|g| g.get()),
                    "Lua event shed due to concurrency/capacity saturation"
                );
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
