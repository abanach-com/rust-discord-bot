use std::collections::HashMap;
use std::sync::{Arc, OnceLock, Weak};

use anyhow::{Result, bail};
use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use serde::{Deserialize, Serialize};
use serenity::all::{
    Channel, ChannelId, ChannelType, Context, GuildChannel, GuildId, PermissionOverwrite,
    PermissionOverwriteType, Permissions, UserId, VoiceState,
};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::state::AppState;
use crate::storage::CompanionChannelMapping;

type UserLockMap = DashMap<(String, String), Weak<Mutex<()>>>;

static USER_LOCKS: OnceLock<UserLockMap> = OnceLock::new();

pub fn get_user_lock(guild_id: &str, user_id: &str) -> Arc<Mutex<()>> {
    let locks = USER_LOCKS.get_or_init(DashMap::new);
    match locks.entry((guild_id.to_owned(), user_id.to_owned())) {
        Entry::Occupied(mut entry) => {
            if let Some(lock) = entry.get().upgrade() {
                lock
            } else {
                let lock = Arc::new(Mutex::new(()));
                entry.insert(Arc::downgrade(&lock));
                lock
            }
        }
        Entry::Vacant(entry) => {
            let lock = Arc::new(Mutex::new(()));
            entry.insert(Arc::downgrade(&lock));
            lock
        }
    }
}

pub fn prune_unused_user_locks() -> usize {
    if let Some(locks) = USER_LOCKS.get() {
        let before = locks.len();
        locks.retain(|_, weak| weak.strong_count() != 0);
        before.saturating_sub(locks.len())
    } else {
        0
    }
}

pub fn user_locks_len() -> usize {
    USER_LOCKS.get().map(|m| m.len()).unwrap_or(0)
}

/// Janitor loop that periodically purges unreferenced voice companion locks from RAM.
pub async fn run_companion_lock_janitor(
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
    interval: std::time::Duration,
) {
    let mut timer = tokio::time::interval(interval);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                break;
            }
            _ = timer.tick() => {
                let pruned = prune_unused_user_locks();
                if pruned > 0 {
                    tracing::debug!(pruned, "pruned unreferenced companion user locks from memory");
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconciliationSummary {
    pub guild_id: String,
    pub revoked: usize,
    pub granted: usize,
    pub active_leases: usize,
    pub pending_retries: usize,
}

/// Helper checking if a Discord API error represents an already absent channel or overwrite (404/NotFound).
pub fn is_unknown_channel_or_overwrite(err: &serenity::all::Error) -> bool {
    if let serenity::all::Error::Http(serenity::all::HttpError::UnsuccessfulRequest(resp)) = err
        && (resp.status_code == 404
            || resp.error.code == 10003
            || resp.error.code == 10009
            || resp.error.code == 10007)
    {
        return true;
    }
    let err_str = err.to_string();
    err_str.contains("Unknown Channel")
        || err_str.contains("Unknown Permissions Overwrite")
        || err_str.contains("Unknown Member")
        || err_str.contains("404 Not Found")
}

/// Validates that both voice and text channels belong to the specified guild and match policy.
pub async fn resolve_companion_mapping_channels(
    ctx: &Context,
    guild_id: GuildId,
    voice_id: ChannelId,
    text_id: ChannelId,
) -> Result<(GuildChannel, GuildChannel)> {
    let voice = voice_id.to_channel(ctx).await?;
    let text = text_id.to_channel(ctx).await?;

    let Channel::Guild(voice_chan) = voice else {
        bail!("Voice channel is not a guild channel");
    };
    let Channel::Guild(text_chan) = text else {
        bail!("Text channel is not a guild channel");
    };

    if voice_chan.guild_id != guild_id {
        bail!(
            "Tenant isolation violation: voice channel {} belongs to guild {}, expected {}",
            voice_id,
            voice_chan.guild_id,
            guild_id
        );
    }
    if text_chan.guild_id != guild_id {
        bail!(
            "Tenant isolation violation: text channel {} belongs to guild {}, expected {}",
            text_id,
            text_chan.guild_id,
            guild_id
        );
    }

    if !matches!(voice_chan.kind, ChannelType::Voice | ChannelType::Stage) {
        bail!("Voice source channel must be of type Voice or Stage");
    }
    if !matches!(text_chan.kind, ChannelType::Text | ChannelType::News) {
        bail!("Companion target channel must be a supported text channel");
    }

    Ok((voice_chan, text_chan))
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

    // Retrieve active and pending revoke leases for this specific member via targeted query (#261)
    let user_leases = state
        .storage
        .list_member_companion_leases(&guild_str, &user_str)
        .await?;

    if let Some(target_voice_id) = new_voice {
        let voice_str = target_voice_id.get().to_string();
        let target_mapping = state
            .storage
            .get_companion_mapping_by_voice(&guild_str, &voice_str)
            .await?
            .filter(|m| m.enabled);

        // 1. Revoke any leases that belong to channels other than the new mapped companion channel
        for lease in &user_leases {
            let keep = match &target_mapping {
                Some(m) => m.text_channel_id == lease.text_channel_id,
                None => false,
            };
            if !keep {
                let _ = revoke_companion_lease(
                    ctx,
                    state,
                    &guild_str,
                    &lease.text_channel_id,
                    &user_str,
                )
                .await;
            }
        }

        // 2. Grant access to the target companion channel if mapped
        if let Some(mapping) = target_mapping {
            let already_active = user_leases
                .iter()
                .any(|l| l.text_channel_id == mapping.text_channel_id && l.status == "active");

            if !already_active {
                grant_companion_access(ctx, state, &mapping, user_id).await?;
            }
        }
    } else {
        // User left all voice channels: revoke all companion leases in this guild
        for lease in user_leases {
            let _ =
                revoke_companion_lease(ctx, state, &guild_str, &lease.text_channel_id, &user_str)
                    .await;
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
    if !mapping.enabled {
        tracing::warn!(
            guild_id = %mapping.guild_id,
            voice_channel_id = %mapping.voice_channel_id,
            text_channel_id = %mapping.text_channel_id,
            user_id = %user_id,
            "Companion mapping is disabled, refusing to grant access"
        );
        return Ok(());
    }

    let target_channel_num = mapping.text_channel_id.parse::<u64>()?;
    let target_channel_id = ChannelId::new(target_channel_num);
    let user_str = user_id.get().to_string();
    let expected_guild_num = mapping.guild_id.parse::<u64>()?;
    let expected_guild_id = GuildId::new(expected_guild_num);

    // Resolve target channel and perform runtime tenant validation (#172)
    let target_channel = if let Some(channel) = ctx
        .cache
        .guild(expected_guild_id)
        .and_then(|g| g.channels.get(&target_channel_id).cloned())
    {
        Some(channel)
    } else if let Ok(Channel::Guild(channel)) = ctx.http.get_channel(target_channel_id).await {
        Some(channel)
    } else {
        None
    };

    if let Some(ref chan) = target_channel {
        if chan.guild_id != expected_guild_id {
            let err_msg = format!(
                "Tenant isolation violation: target channel {} belongs to guild {}, not origin guild {}",
                target_channel_id, chan.guild_id, expected_guild_id
            );
            tracing::error!(%err_msg);
            let _ = state
                .storage
                .record_audit(
                    Some(&mapping.guild_id),
                    Some(&user_str),
                    "companion_channels",
                    "cross_guild_violation_blocked",
                    serde_json::json!({
                        "target_channel_id": mapping.text_channel_id,
                        "detected_guild_id": chan.guild_id.get().to_string(),
                    }),
                )
                .await;
            bail!(err_msg);
        }
        if !matches!(chan.kind, ChannelType::Text | ChannelType::News) {
            bail!(
                "Target channel {} is not a supported text channel (kind: {:?})",
                target_channel_id,
                chan.kind
            );
        }
    }

    // Check if user already has a recorded lease
    let existing_lease = state
        .storage
        .get_companion_lease(&mapping.guild_id, &mapping.text_channel_id, &user_str)
        .await?;

    // Check if there is an existing overwrite on Discord
    let has_discord_overwrite = target_channel
        .as_ref()
        .map(|c| {
            c.permission_overwrites
                .iter()
                .any(|ow| ow.kind == PermissionOverwriteType::Member(user_id))
        })
        .unwrap_or(false);

    // Manual overwrite protection:
    // If overwrite exists on Discord, but we have NO lease recorded, it was manually created by staff!
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

    let allow_bits = allow.bits() as i64;
    let deny_bits = 0i64;

    let overwrite = PermissionOverwrite {
        allow,
        deny: Permissions::empty(),
        kind: PermissionOverwriteType::Member(user_id),
    };

    // Attempt to create/update permission on Discord.
    // If the bot lacks permissions, this will fail and we do NOT record a false lease.
    target_channel_id
        .create_permission(&ctx.http, overwrite)
        .await?;

    let generation = Uuid::new_v4();
    state
        .storage
        .record_companion_lease(crate::storage::RecordCompanionLeaseParams {
            guild_id: &mapping.guild_id,
            voice_channel_id: &mapping.voice_channel_id,
            text_channel_id: &mapping.text_channel_id,
            user_id: &user_str,
            generation,
            allow_bits,
            deny_bits,
        })
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

/// Revokes permission overwrite from Discord with durable outbox lifecycle (#173).
/// Does NOT drop ownership metadata before external Discord confirmation.
pub async fn revoke_companion_lease(
    ctx: &Context,
    state: &AppState,
    guild_id: &str,
    text_channel_id_str: &str,
    user_id_str: &str,
) -> Result<bool> {
    // 1. Mark lease as revoke_pending in DB so ownership is preserved if external call fails
    let pending_lease = state
        .storage
        .mark_companion_lease_revoke_pending(guild_id, text_channel_id_str, user_id_str)
        .await?;

    let Some(lease) = pending_lease else {
        // No lease found to revoke
        return Ok(false);
    };

    let target_channel_num = match text_channel_id_str.parse::<u64>() {
        Ok(n) => n,
        Err(_) => {
            // Corrupted channel ID: delete lease to unblock
            let _ = state
                .storage
                .delete_companion_lease(guild_id, text_channel_id_str, user_id_str)
                .await;
            return Ok(true);
        }
    };
    let user_num = match user_id_str.parse::<u64>() {
        Ok(n) => n,
        Err(_) => {
            let _ = state
                .storage
                .delete_companion_lease(guild_id, text_channel_id_str, user_id_str)
                .await;
            return Ok(true);
        }
    };

    let target_channel_id = ChannelId::new(target_channel_num);
    let user_id = UserId::new(user_num);

    // 2. Call Discord API to remove overwrite
    let del_res = target_channel_id
        .delete_permission(&ctx.http, PermissionOverwriteType::Member(user_id))
        .await;

    // 3. Evaluate external outcome
    match del_res {
        Ok(()) => {
            // Success: external state reached, safely delete lease
            state
                .storage
                .delete_companion_lease(guild_id, text_channel_id_str, user_id_str)
                .await?;
            tracing::info!(
                guild_id,
                text_channel_id = text_channel_id_str,
                user_id = user_id_str,
                generation = %lease.generation,
                "Revoked companion channel lease successfully"
            );
            Ok(true)
        }
        Err(ref err) if is_unknown_channel_or_overwrite(err) => {
            // 404 / already absent: external state reached, safely delete lease
            state
                .storage
                .delete_companion_lease(guild_id, text_channel_id_str, user_id_str)
                .await?;
            tracing::info!(
                guild_id,
                text_channel_id = text_channel_id_str,
                user_id = user_id_str,
                "External overwrite was already absent; deleted companion lease"
            );
            Ok(true)
        }
        Err(err) => {
            // External failure: record failure and keep lease in revoke_pending so ownership is not lost
            let err_str = err.to_string();
            tracing::error!(
                guild_id,
                text_channel_id = text_channel_id_str,
                user_id = user_id_str,
                ?err,
                "Discord permission delete failed; lease retained as revoke_pending"
            );
            state
                .storage
                .record_companion_lease_failure(
                    guild_id,
                    text_channel_id_str,
                    user_id_str,
                    &err_str,
                )
                .await?;
            Err(err.into())
        }
    }
}

/// Reconciles all active and pending companion leases for a guild against current voice states in cache.
/// Fails fast if guild cache is unavailable, deferring reconciliation to avoid unjustified revocations (#263).
pub async fn reconcile_guild_companion_channels(
    ctx: &Context,
    state: &AppState,
    guild_id: GuildId,
) -> Result<ReconciliationSummary> {
    // Ensure guild is available in cache before attempting reconciliation (#263).
    // An unavailable guild must NEVER be treated as having zero voice states,
    // which would cause mass unjustified revocation of active companion leases during startup.
    let current_voice_states = match ctx.cache.guild(guild_id) {
        Some(guild_ref) => guild_ref.voice_states.clone(),
        None => {
            tracing::info!(
                guild_id = %guild_id,
                "Guild cache is unavailable; deferring companion channel reconciliation"
            );
            anyhow::bail!(
                "Guild {guild_id} cache is unavailable; deferring companion reconciliation"
            );
        }
    };

    reconcile_guild_companion_channels_with_voice_states(ctx, state, guild_id, current_voice_states)
        .await
}

/// Reconciles companion leases against an explicitly provided, reliable snapshot of guild voice states.
pub async fn reconcile_guild_companion_channels_with_voice_states(
    ctx: &Context,
    state: &AppState,
    guild_id: GuildId,
    current_voice_states: HashMap<UserId, VoiceState>,
) -> Result<ReconciliationSummary> {
    let guild_str = guild_id.get().to_string();
    let mappings = state.storage.list_companion_mappings(&guild_str).await?;
    let active_leases = state
        .storage
        .list_active_companion_leases(&guild_str)
        .await?;
    let pending_leases = state
        .storage
        .list_pending_revoke_companion_leases(&guild_str)
        .await?;

    let mut revoked_count = 0;
    let mut granted_count = 0;
    let mut pending_retries = 0;

    // 1. Retry pending revoke leases (#173)
    for lease in pending_leases {
        let user_lock = get_user_lock(&guild_str, &lease.user_id);
        let _guard = user_lock.lock().await;
        match revoke_companion_lease(
            ctx,
            state,
            &guild_str,
            &lease.text_channel_id,
            &lease.user_id,
        )
        .await
        {
            Ok(true) => revoked_count += 1,
            Ok(false) => {}
            Err(_) => pending_retries += 1,
        }
    }

    // 2. Check active leases: if user is not in the mapped voice channel or mapping changed/disabled, revoke!
    for lease in active_leases {
        let Ok(uid) = lease.user_id.parse::<u64>() else {
            continue;
        };
        let user_id = UserId::new(uid);
        let user_lock = get_user_lock(&guild_str, &lease.user_id);
        let _guard = user_lock.lock().await;

        // Resolve authoritative current voice channel under user lock (Issue #268).
        // If the live guild cache is available, it is authoritative; do not fallback to
        // a potentially stale snapshot, which would cause races with recent disconnects/moves.
        let current_voice_channel = if let Some(guild) = ctx.cache.guild(guild_id) {
            guild
                .voice_states
                .get(&user_id)
                .and_then(|vs| vs.channel_id)
        } else {
            current_voice_states
                .get(&user_id)
                .and_then(|vs| vs.channel_id)
        }
        .map(|c| c.get().to_string());

        let current_voice_matches_lease = match current_voice_channel {
            Some(ref v) => v == &lease.voice_channel_id,
            None => false,
        };

        let still_mapped_to_this_text = mappings.iter().any(|m| {
            m.voice_channel_id == lease.voice_channel_id
                && m.text_channel_id == lease.text_channel_id
                && m.enabled
        });

        let is_valid = current_voice_matches_lease && still_mapped_to_this_text;

        if !is_valid {
            match revoke_companion_lease(
                ctx,
                state,
                &guild_str,
                &lease.text_channel_id,
                &lease.user_id,
            )
            .await
            {
                Ok(true) => revoked_count += 1,
                Ok(false) => {}
                Err(_) => pending_retries += 1,
            }
        }
    }

    // 3. For all users currently in voice, ensure active lease if mapped and enabled.
    // When live guild cache is available, it is the authoritative source for voice presence.
    // Stale snapshots must never re-grant access to users who have already disconnected (Issue #268).
    let candidate_users: std::collections::HashSet<UserId> =
        if let Some(guild) = ctx.cache.guild(guild_id) {
            guild.voice_states.keys().copied().collect()
        } else {
            current_voice_states.keys().copied().collect()
        };

    for user_id in candidate_users {
        let user_str = user_id.get().to_string();
        let user_lock = get_user_lock(&guild_str, &user_str);
        let _guard = user_lock.lock().await;

        // Determine authoritative voice channel under user lock (Issue #268)
        let live_voice_channel = if let Some(guild) = ctx.cache.guild(guild_id) {
            guild
                .voice_states
                .get(&user_id)
                .and_then(|vs| vs.channel_id)
        } else {
            current_voice_states
                .get(&user_id)
                .and_then(|vs| vs.channel_id)
        };

        if let Some(voice_channel_id) = live_voice_channel {
            let voice_str = voice_channel_id.get().to_string();
            if let Some(mapping) = mappings
                .iter()
                .find(|m| m.enabled && m.voice_channel_id == voice_str)
            {
                let existing = state
                    .storage
                    .get_companion_lease(&guild_str, &mapping.text_channel_id, &user_str)
                    .await?;
                let needs_grant = existing.is_none()
                    || existing.as_ref().map(|l| l.status.as_str()) != Some("active");
                if needs_grant
                    && grant_companion_access(ctx, state, mapping, user_id)
                        .await
                        .is_ok()
                {
                    granted_count += 1;
                }
            }
        }
    }

    let remaining_leases = state
        .storage
        .list_active_companion_leases(&guild_str)
        .await?
        .len();

    prune_unused_user_locks();

    Ok(ReconciliationSummary {
        guild_id: guild_str,
        revoked: revoked_count,
        granted: granted_count,
        active_leases: remaining_leases,
        pending_retries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_user_locks_share_mutex_while_active() {
        let lock1 = get_user_lock("guild_test_1", "user_test_1");
        let lock2 = get_user_lock("guild_test_1", "user_test_1");
        assert!(
            Arc::ptr_eq(&lock1, &lock2),
            "concurrent callers for the same (guild, user) must share the identical Mutex"
        );

        let lock_diff_user = get_user_lock("guild_test_1", "user_test_2");
        let lock_diff_guild = get_user_lock("guild_test_2", "user_test_1");
        assert!(!Arc::ptr_eq(&lock1, &lock_diff_user));
        assert!(!Arc::ptr_eq(&lock1, &lock_diff_guild));
    }

    #[tokio::test]
    async fn test_user_locks_pruning_cleans_dead_weak_refs_without_affecting_active_locks() {
        // Clear any remnants
        prune_unused_user_locks();

        // 1. Create 50 ephemeral locks that are dropped immediately
        for i in 0..50 {
            let _ = get_user_lock("guild_ephemeral", &format!("user_{i}"));
        }

        // 2. Create 5 active locks that we retain strong Arc references to
        let mut held_locks = Vec::new();
        for i in 0..5 {
            held_locks.push(get_user_lock("guild_active", &format!("user_{i}")));
        }

        // Before prune, all 55 entries are in DashMap
        assert!(user_locks_len() >= 55);

        // Run pruning: only active locks (strong_count > 0) are preserved
        let pruned = prune_unused_user_locks();
        assert!(pruned >= 50);

        // Active locks must still be valid and match held pointers
        for (i, held) in held_locks.iter().enumerate() {
            let reacquired = get_user_lock("guild_active", &format!("user_{i}"));
            assert!(
                Arc::ptr_eq(held, &reacquired),
                "held lock for active user must remain identical"
            );
        }

        // Drop held locks
        drop(held_locks);

        // Pruning now cleans up the remaining 5
        let pruned_remaining = prune_unused_user_locks();
        assert!(pruned_remaining >= 5);
    }

    #[tokio::test]
    async fn test_user_lock_serializes_critical_section() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let counter = Arc::new(AtomicU32::new(0));
        let mut handles = Vec::new();

        for _ in 0..10 {
            let counter = counter.clone();
            handles.push(tokio::spawn(async move {
                let lock = get_user_lock("guild_concurrent", "user_concurrent");
                let _guard = lock.lock().await;

                let val = counter.load(Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                counter.store(val + 1, Ordering::SeqCst);
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }

        assert_eq!(counter.load(Ordering::SeqCst), 10);
    }
}
