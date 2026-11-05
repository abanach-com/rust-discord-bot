use std::sync::Arc;

use anyhow::Result;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serenity::all::{
    Channel, ChannelId, Context, GuildId, PermissionOverwrite, PermissionOverwriteType,
    Permissions, UserId,
};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::state::AppState;
use crate::storage::CompanionChannelMapping;

type UserLockMap = DashMap<(String, String), Arc<Mutex<()>>>;

static USER_LOCKS: std::sync::OnceLock<UserLockMap> = std::sync::OnceLock::new();

pub fn get_user_lock(guild_id: &str, user_id: &str) -> Arc<Mutex<()>> {
    let locks = USER_LOCKS.get_or_init(DashMap::new);
    locks
        .entry((guild_id.to_string(), user_id.to_string()))
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconciliationSummary {
    pub guild_id: String,
    pub revoked: usize,
    pub granted: usize,
    pub active_leases: usize,
}

/// Reconciles voice companion channel access for a user when their voice state updates.
pub async fn reconcile_companion_channel_access(
    ctx: &Context,
    state: &AppState,
    guild_id: GuildId,
    user_id: UserId,
    _old_voice: Option<ChannelId>,
    new_voice: Option<ChannelId>,
) -> Result<()> {
    let guild_str = guild_id.get().to_string();
    let user_str = user_id.get().to_string();

    // Serialize access per (guild, user) to prevent race conditions during rapid voice transitions.
    let user_lock = get_user_lock(&guild_str, &user_str);
    let _guard = user_lock.lock().await;

    // Check all currently active leases for this user in this guild
    let all_leases = state
        .storage
        .list_active_companion_leases(&guild_str)
        .await?;
    let user_leases: Vec<_> = all_leases
        .into_iter()
        .filter(|l| l.user_id == user_str)
        .collect();

    if let Some(target_voice_id) = new_voice {
        let voice_str = target_voice_id.get().to_string();
        let target_mapping = state
            .storage
            .get_companion_mapping_by_voice(&guild_str, &voice_str)
            .await?;

        // 1. Revoke any leases that belong to channels other than the new mapped companion channel
        for lease in &user_leases {
            let keep = match &target_mapping {
                Some(m) => m.text_channel_id == lease.text_channel_id,
                None => false,
            };
            if !keep {
                revoke_companion_lease(ctx, state, &guild_str, &lease.text_channel_id, &user_str)
                    .await?;
            }
        }

        // 2. Grant access to the target companion channel if mapped
        if let Some(mapping) = target_mapping {
            let already_granted = user_leases
                .iter()
                .any(|l| l.text_channel_id == mapping.text_channel_id);

            if !already_granted {
                grant_companion_access(ctx, state, &mapping, user_id).await?;
            }
        }
    } else {
        // User left all voice channels: revoke all active companion leases in this guild
        for lease in user_leases {
            revoke_companion_lease(ctx, state, &guild_str, &lease.text_channel_id, &user_str)
                .await?;
        }
    }

    Ok(())
}

/// Grants permission to the companion text channel and records a persistent lease.
pub async fn grant_companion_access(
    ctx: &Context,
    state: &AppState,
    mapping: &CompanionChannelMapping,
    user_id: UserId,
) -> Result<()> {
    let target_channel_id = ChannelId::new(mapping.text_channel_id.parse::<u64>()?);
    let user_str = user_id.get().to_string();

    // Check if user already has a recorded lease
    let existing_lease = state
        .storage
        .get_companion_lease(&mapping.guild_id, &mapping.text_channel_id, &user_str)
        .await?;

    let guild_id = GuildId::new(mapping.guild_id.parse::<u64>()?);

    // Check if there is an existing overwrite on Discord
    let has_discord_overwrite = if let Some(channel) = ctx
        .cache
        .guild(guild_id)
        .and_then(|g| g.channels.get(&target_channel_id).cloned())
    {
        channel
            .permission_overwrites
            .iter()
            .any(|ow| ow.kind == PermissionOverwriteType::Member(user_id))
    } else if let Ok(Channel::Guild(channel)) = ctx.http.get_channel(target_channel_id).await {
        channel
            .permission_overwrites
            .iter()
            .any(|ow| ow.kind == PermissionOverwriteType::Member(user_id))
    } else {
        false
    };

    // If an overwrite exists on Discord but we have NO lease recorded, it was manually created by staff!
    // As per specification: do not overwrite manual decisions, report conflict.
    if has_discord_overwrite && existing_lease.is_none() {
        tracing::warn!(
            guild_id = %mapping.guild_id,
            text_channel_id = %mapping.text_channel_id,
            user_id = %user_str,
            "Refusing to grant companion channel overwrite: manual overwrite already exists for member"
        );
        let _ = state
            .storage
            .record_audit(
                Some(&mapping.guild_id),
                Some(&user_str),
                "companion_channels",
                "permission_conflict",
                serde_json::json!({
                    "voice_channel_id": mapping.voice_channel_id,
                    "text_channel_id": mapping.text_channel_id,
                    "conflict_reason": "existing_manual_overwrite"
                }),
            )
            .await;
        return Ok(());
    }

    let mut allow = Permissions::VIEW_CHANNEL;
    if mapping.allow_read_history {
        allow |= Permissions::READ_MESSAGE_HISTORY;
    }
    if mapping.allow_send_messages {
        allow |= Permissions::SEND_MESSAGES | Permissions::SEND_MESSAGES_IN_THREADS;
    }

    let overwrite = PermissionOverwrite {
        allow,
        deny: Permissions::empty(),
        kind: PermissionOverwriteType::Member(user_id),
    };

    // Attempt to create/update the permission overwrite on Discord.
    // If the bot lacks permissions, this will fail and we do NOT record a false lease.
    target_channel_id
        .create_permission(&ctx.http, overwrite)
        .await?;

    let generation = Uuid::new_v4();
    state
        .storage
        .record_companion_lease(
            &mapping.guild_id,
            &mapping.voice_channel_id,
            &mapping.text_channel_id,
            &user_str,
            generation,
        )
        .await?;

    tracing::info!(
        guild_id = %mapping.guild_id,
        voice_channel_id = %mapping.voice_channel_id,
        text_channel_id = %mapping.text_channel_id,
        user_id = %user_str,
        generation = %generation,
        "Granted companion channel lease"
    );

    Ok(())
}

/// Revokes permission overwrite from Discord if a ZuckerBot lease was recorded, and removes the lease.
pub async fn revoke_companion_lease(
    ctx: &Context,
    state: &AppState,
    guild_id: &str,
    text_channel_id_str: &str,
    user_id_str: &str,
) -> Result<()> {
    // Only delete Discord permission if we actually hold an active lease!
    let deleted_lease = state
        .storage
        .delete_companion_lease(guild_id, text_channel_id_str, user_id_str)
        .await?;

    if deleted_lease.is_some() {
        if let (Ok(channel_num), Ok(user_num)) = (
            text_channel_id_str.parse::<u64>(),
            user_id_str.parse::<u64>(),
        ) {
            let target_channel_id = ChannelId::new(channel_num);
            let user_id = UserId::new(user_num);
            let _ = target_channel_id
                .delete_permission(&ctx.http, PermissionOverwriteType::Member(user_id))
                .await;
        }

        tracing::info!(
            guild_id,
            text_channel_id = text_channel_id_str,
            user_id = user_id_str,
            "Revoked companion channel lease"
        );
    }

    Ok(())
}

/// Reconciles all active companion leases for a guild against current voice states.
/// Idempotent and safe to run on startup / reconnection or on admin request.
pub async fn reconcile_guild_companion_channels(
    ctx: &Context,
    state: &AppState,
    guild_id: GuildId,
) -> Result<ReconciliationSummary> {
    let guild_str = guild_id.get().to_string();
    let mappings = state.storage.list_companion_mappings(&guild_str).await?;
    let leases = state
        .storage
        .list_active_companion_leases(&guild_str)
        .await?;

    let mut revoked_count = 0;
    let mut granted_count = 0;

    // Current voice states from guild cache
    let current_voice_states = ctx
        .cache
        .guild(guild_id)
        .map(|g| g.voice_states.clone())
        .unwrap_or_default();

    // 1. Check existing leases: if user is not in the mapped voice channel, revoke!
    for lease in leases {
        let is_valid = match lease.user_id.parse::<u64>() {
            Ok(uid) => {
                let current_voice_channel = current_voice_states
                    .get(&UserId::new(uid))
                    .and_then(|vs| vs.channel_id)
                    .map(|c| c.get().to_string());

                match current_voice_channel {
                    Some(ref v) => v == &lease.voice_channel_id,
                    None => false,
                }
            }
            Err(_) => false,
        };

        if !is_valid {
            let user_lock = get_user_lock(&guild_str, &lease.user_id);
            let _guard = user_lock.lock().await;
            revoke_companion_lease(
                ctx,
                state,
                &guild_str,
                &lease.text_channel_id,
                &lease.user_id,
            )
            .await?;
            revoked_count += 1;
        }
    }

    // 2. For all users currently in voice, ensure active lease if mapped
    for (user_id, voice_state) in current_voice_states {
        if let Some(voice_channel_id) = voice_state.channel_id {
            let voice_str = voice_channel_id.get().to_string();
            if let Some(mapping) = mappings.iter().find(|m| m.voice_channel_id == voice_str) {
                let user_str = user_id.get().to_string();
                let existing = state
                    .storage
                    .get_companion_lease(&guild_str, &mapping.text_channel_id, &user_str)
                    .await?;
                if existing.is_none() {
                    let user_lock = get_user_lock(&guild_str, &user_str);
                    let _guard = user_lock.lock().await;
                    if let Ok(()) = grant_companion_access(ctx, state, mapping, user_id).await {
                        granted_count += 1;
                    }
                }
            }
        }
    }

    let remaining_leases = state
        .storage
        .list_active_companion_leases(&guild_str)
        .await?
        .len();

    Ok(ReconciliationSummary {
        guild_id: guild_str,
        revoked: revoked_count,
        granted: granted_count,
        active_leases: remaining_leases,
    })
}
