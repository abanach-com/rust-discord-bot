use std::{
    sync::{
        Arc, RwLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use chrono::Utc;
use dashmap::DashMap;
use serenity::all::{
    Channel, ChannelId, ChannelType, CreateAllowedMentions, CreateMessage, GuildId, Http,
    MessageId, Permissions,
};
use tokio::sync::{Notify, mpsc};
use uuid::Uuid;

use crate::storage::Storage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StickyCacheEntry {
    MissingUntil(std::time::Instant),
    InvalidatedUntil(std::time::Instant),
}

#[derive(Clone, Debug)]
pub enum WorkerSignal {
    Trigger(MessageId),
    ConfigChanged,
    Deleted,
}

#[doc(hidden)]
#[derive(Clone, Default)]
pub struct StickyTestPause {
    pub worker_reached_ok_none: Arc<Notify>,
    pub allow_worker_to_proceed: Arc<Notify>,
}

#[derive(Clone, Debug)]
pub struct StickyRefresh {
    pub guild_id: GuildId,
    pub channel_id: ChannelId,
    pub trigger_message_id: MessageId,
}

#[derive(Clone)]
struct WorkerHandle {
    owner_id: Uuid,
    tx: mpsc::Sender<WorkerSignal>,
}

#[derive(Clone)]
struct WorkerEnv {
    storage: Storage,
    channel_senders: Arc<DashMap<(GuildId, ChannelId), WorkerHandle>>,
    negative_cache: Arc<DashMap<(GuildId, ChannelId), StickyCacheEntry>>,
    test_pause: Arc<RwLock<Option<StickyTestPause>>>,
    storage_fetch_count: Arc<AtomicUsize>,
}

/// Coordinates sticky message delivery across channels within a single bot process.
///
/// # Concurrency & Multi-Process Model
/// Negative caching uses a two-state token ([`StickyCacheEntry::MissingUntil`] vs
/// [`StickyCacheEntry::InvalidatedUntil`]) to guarantee that concurrent local web mutations
/// (`set_sticky`) are never overwritten by stale worker reads (`Ok(None)`).
/// In a single-process deployment (or per-shard bot process), invalidations take effect immediately.
/// For clustered / multi-process bot deployments sharing a PostgreSQL database, cache invalidation
/// across independent process boundaries relies on the maximum 60-second TTL bound, or can be
/// augmented with PostgreSQL LISTEN/NOTIFY.
pub struct StickyCoordinator {
    storage: Storage,
    channel_senders: Arc<DashMap<(GuildId, ChannelId), WorkerHandle>>,
    negative_cache: Arc<DashMap<(GuildId, ChannelId), StickyCacheEntry>>,
    #[doc(hidden)]
    test_pause: Arc<RwLock<Option<StickyTestPause>>>,
    #[doc(hidden)]
    storage_fetch_count: Arc<AtomicUsize>,
}

impl StickyCoordinator {
    pub fn new(storage: Storage) -> Self {
        Self {
            storage,
            channel_senders: Arc::new(DashMap::new()),
            negative_cache: Arc::new(DashMap::new()),
            test_pause: Arc::new(RwLock::new(None)),
            storage_fetch_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn active_worker_count(&self) -> usize {
        self.channel_senders.len()
    }

    #[doc(hidden)]
    pub fn storage_fetch_count(&self) -> usize {
        self.storage_fetch_count.load(Ordering::SeqCst)
    }

    pub fn negative_cache_len(&self) -> usize {
        let now = std::time::Instant::now();
        self.negative_cache
            .iter()
            .filter(|entry| match entry.value() {
                StickyCacheEntry::MissingUntil(expiry) => *expiry > now,
                StickyCacheEntry::InvalidatedUntil(_) => false,
            })
            .count()
    }

    pub fn invalidate_channel(&self, guild_id: &str, channel_id: &str) {
        if let (Ok(gid), Ok(cid)) = (guild_id.parse::<u64>(), channel_id.parse::<u64>()) {
            let key = (GuildId::new(gid), ChannelId::new(cid));
            let now = std::time::Instant::now();
            self.negative_cache.insert(
                key,
                StickyCacheEntry::InvalidatedUntil(now + Duration::from_secs(60)),
            );
            if let Some(handle) = self.channel_senders.get(&key) {
                let _ = handle.tx.try_send(WorkerSignal::ConfigChanged);
            }
        }
    }

    #[doc(hidden)]
    pub fn set_test_pause(&self, pause: StickyTestPause) {
        *self.test_pause.write().unwrap() = Some(pause);
    }

    #[doc(hidden)]
    pub fn clear_test_pause(&self) {
        *self.test_pause.write().unwrap() = None;
    }

    pub fn enqueue_refresh(
        &self,
        http: Arc<Http>,
        guild_id: GuildId,
        channel_id: ChannelId,
        trigger_message_id: MessageId,
    ) {
        let key = (guild_id, channel_id);

        // Fast-path: Check negative cache (channels known not to have sticky messages)
        let now = std::time::Instant::now();
        if let Some(entry) = self.negative_cache.get(&key) {
            match *entry {
                StickyCacheEntry::MissingUntil(expiry) => {
                    if expiry > now {
                        return;
                    }
                    drop(entry);
                    self.negative_cache.remove(&key);
                }
                StickyCacheEntry::InvalidatedUntil(expiry) => {
                    if expiry <= now {
                        drop(entry);
                        self.negative_cache.remove(&key);
                    }
                    // InvalidatedUntil forces normal worker execution; do not return early
                }
            }
        }

        // Opportunistic cleanup of negative cache if it grows large
        if self.negative_cache.len() >= 4096 {
            self.negative_cache.retain(|_, entry| match entry {
                StickyCacheEntry::MissingUntil(expiry) => *expiry > now,
                StickyCacheEntry::InvalidatedUntil(expiry) => *expiry > now,
            });
        }

        use dashmap::mapref::entry::Entry;

        let spawn_info = match self.channel_senders.entry(key) {
            Entry::Occupied(entry) if !entry.get().tx.is_closed() => {
                let _ = entry
                    .get()
                    .tx
                    .try_send(WorkerSignal::Trigger(trigger_message_id));
                None
            }
            Entry::Occupied(mut entry) => {
                let (tx, rx) = mpsc::channel(64);
                let _ = tx.try_send(WorkerSignal::Trigger(trigger_message_id));
                let owner_id = Uuid::new_v4();
                entry.insert(WorkerHandle { owner_id, tx });
                Some((rx, owner_id))
            }
            Entry::Vacant(entry) => {
                let (tx, rx) = mpsc::channel(64);
                let _ = tx.try_send(WorkerSignal::Trigger(trigger_message_id));
                let owner_id = Uuid::new_v4();
                entry.insert(WorkerHandle { owner_id, tx });
                Some((rx, owner_id))
            }
        };

        if let Some((rx, owner_id)) = spawn_info {
            let env = WorkerEnv {
                storage: self.storage.clone(),
                channel_senders: self.channel_senders.clone(),
                negative_cache: self.negative_cache.clone(),
                test_pause: self.test_pause.clone(),
                storage_fetch_count: self.storage_fetch_count.clone(),
            };
            tokio::spawn(async move {
                run_channel_worker(key, owner_id, rx, http, env).await;
            });
        }
    }

    pub fn remove_channel(&self, guild_id: &str, channel_id: &str) {
        if let (Ok(gid), Ok(cid)) = (guild_id.parse::<u64>(), channel_id.parse::<u64>()) {
            let key = (GuildId::new(gid), ChannelId::new(cid));
            if let Some((_, handle)) = self.channel_senders.remove(&key) {
                let _ = handle.tx.try_send(WorkerSignal::Deleted);
            }
            self.negative_cache.remove(&key);
        }
    }
}

pub async fn check_bot_channel_permission(
    http: &Http,
    guild_id: GuildId,
    channel_id: ChannelId,
) -> Result<()> {
    let channel = channel_id
        .to_channel(http)
        .await
        .with_context(|| format!("cannot fetch channel {channel_id}"))?;

    let Channel::Guild(guild_channel) = channel else {
        anyhow::bail!("destination channel must be a guild channel");
    };

    if guild_channel.guild_id != guild_id {
        anyhow::bail!(
            "target channel {channel_id} belongs to guild {}, expected {guild_id}",
            guild_channel.guild_id
        );
    }

    if guild_channel.kind == ChannelType::Category {
        anyhow::bail!("category channel cannot be the destination for sticky messages");
    }

    let is_thread = matches!(
        guild_channel.kind,
        ChannelType::PublicThread | ChannelType::PrivateThread | ChannelType::NewsThread
    );

    let parent_channel = if is_thread {
        if let Some(parent_id) = guild_channel.parent_id {
            match parent_id.to_channel(http).await {
                Ok(Channel::Guild(p)) => Some(p),
                _ => None,
            }
        } else {
            None
        }
    } else {
        None
    };

    let overwrite_channel = parent_channel.as_ref().unwrap_or(&guild_channel);

    let bot_user = http
        .get_current_user()
        .await
        .context("cannot fetch bot user")?;

    let bot_member = guild_id
        .member(http, bot_user.id)
        .await
        .with_context(|| format!("cannot fetch bot member in guild {guild_id}"))?;

    let guild = guild_id
        .to_partial_guild(http)
        .await
        .with_context(|| format!("cannot fetch guild data for {guild_id}"))?;

    let bot_perms = crate::discord::scheduler::calculate_effective_channel_permissions(
        &guild,
        &bot_member.roles,
        bot_user.id,
        overwrite_channel,
    );

    if !bot_perms.contains(Permissions::VIEW_CHANNEL) {
        anyhow::bail!("bot lacks VIEW_CHANNEL permission in channel {channel_id}");
    }

    if is_thread {
        if !bot_perms.contains(Permissions::SEND_MESSAGES_IN_THREADS) {
            anyhow::bail!("bot lacks SEND_MESSAGES_IN_THREADS in thread {channel_id}");
        }
    } else if !bot_perms.contains(Permissions::SEND_MESSAGES) {
        anyhow::bail!("bot lacks SEND_MESSAGES permission in channel {channel_id}");
    }

    Ok(())
}

async fn run_channel_worker(
    key: (GuildId, ChannelId),
    owner_id: Uuid,
    mut rx: mpsc::Receiver<WorkerSignal>,
    http: Arc<Http>,
    env: WorkerEnv,
) {
    let gid_str = key.0.get().to_string();
    let cid_str = key.1.get().to_string();

    while let Ok(maybe_signal) = tokio::time::timeout(Duration::from_secs(300), rx.recv()).await {
        let Some(signal) = maybe_signal else {
            break;
        };

        let mut trigger_id = match signal {
            WorkerSignal::Trigger(id) => id,
            WorkerSignal::ConfigChanged => {
                let mut found_trigger = None;
                while let Ok(next) = rx.try_recv() {
                    match next {
                        WorkerSignal::Trigger(id) => found_trigger = Some(id),
                        WorkerSignal::ConfigChanged => {}
                        WorkerSignal::Deleted => {
                            env.channel_senders
                                .remove_if(&key, |_, h| h.owner_id == owner_id);
                            return;
                        }
                    }
                }
                match found_trigger {
                    Some(id) => id,
                    None => continue,
                }
            }
            WorkerSignal::Deleted => {
                env.channel_senders
                    .remove_if(&key, |_, h| h.owner_id == owner_id);
                return;
            }
        };

        // Drain pending burst triggers to coalesce into the latest
        while let Ok(next) = rx.try_recv() {
            match next {
                WorkerSignal::Trigger(id) => trigger_id = id,
                WorkerSignal::ConfigChanged => {}
                WorkerSignal::Deleted => {
                    env.channel_senders
                        .remove_if(&key, |_, h| h.owner_id == owner_id);
                    return;
                }
            }
        }

        let mut delivery_attempts = 0;
        while delivery_attempts < 3 {
            // 1. Fetch current sticky configuration
            env.storage_fetch_count.fetch_add(1, Ordering::SeqCst);
            let sticky = match env.storage.get_sticky_message(&gid_str, &cid_str).await {
                Ok(Some(s)) => {
                    // Successful read of configured sticky message: clear any lingering InvalidatedUntil token
                    if let Some(entry) = env.negative_cache.get(&key)
                        && matches!(*entry, StickyCacheEntry::InvalidatedUntil(_))
                    {
                        drop(entry);
                        env.negative_cache.remove(&key);
                    }
                    s
                }
                Ok(None) => {
                    // Test hook: allow test to freeze worker right after receiving Ok(None)
                    let pause_hook = env.test_pause.read().unwrap().clone();
                    if let Some(pause) = pause_hook {
                        pause.worker_reached_ok_none.notify_one();
                        pause.allow_worker_to_proceed.notified().await;
                    }

                    // No sticky configured for this channel: atomically insert MissingUntil only if not InvalidatedUntil
                    use dashmap::mapref::entry::Entry;
                    let now = std::time::Instant::now();
                    let mut recheck_after_invalidation = false;
                    match env.negative_cache.entry(key) {
                        Entry::Occupied(mut occ) => match *occ.get() {
                            StickyCacheEntry::InvalidatedUntil(expiry) if expiry > now => {
                                while let Ok(next) = rx.try_recv() {
                                    match next {
                                        WorkerSignal::Trigger(next_id) => {
                                            trigger_id = next_id;
                                            recheck_after_invalidation = true;
                                        }
                                        WorkerSignal::ConfigChanged => {
                                            recheck_after_invalidation = true;
                                        }
                                        WorkerSignal::Deleted => {
                                            env.channel_senders
                                                .remove_if(&key, |_, h| h.owner_id == owner_id);
                                            return;
                                        }
                                    }
                                }
                            }
                            _ => {
                                occ.insert(StickyCacheEntry::MissingUntil(
                                    now + Duration::from_secs(60),
                                ));
                            }
                        },
                        Entry::Vacant(vac) => {
                            vac.insert(StickyCacheEntry::MissingUntil(
                                now + Duration::from_secs(60),
                            ));
                        }
                    }

                    if recheck_after_invalidation {
                        continue;
                    }

                    env.channel_senders
                        .remove_if(&key, |_, h| h.owner_id == owner_id);
                    return;
                }
                Err(err) => {
                    tracing::error!(
                        "Database error fetching sticky message for {gid_str}/{cid_str}: {err}"
                    );
                    env.channel_senders
                        .remove_if(&key, |_, h| h.owner_id == owner_id);
                    return;
                }
            };

            // 2. Cooldown check: if inside cooldown, sleep until fixed deadline while coalescing triggers WITHOUT querying storage
            if let Some(last_bumped) = sticky.last_bumped_at {
                let elapsed = Utc::now().signed_duration_since(last_bumped);
                let cooldown_dur = chrono::Duration::seconds(i64::from(sticky.cooldown_seconds));
                if elapsed < cooldown_dur {
                    let remaining = (cooldown_dur - elapsed).to_std().unwrap_or_default();
                    if !remaining.is_zero() {
                        let deadline = tokio::time::Instant::now() + remaining;

                        loop {
                            tokio::select! {
                                _ = tokio::time::sleep_until(deadline) => {
                                    // Cooldown deadline expired; break sleep loop and proceed to storage query & delivery
                                    break;
                                }
                                maybe_signal = rx.recv() => {
                                    match maybe_signal {
                                        Some(WorkerSignal::Trigger(id)) => {
                                            trigger_id = id;
                                            let mut config_changed = false;
                                            while let Ok(next) = rx.try_recv() {
                                                match next {
                                                    WorkerSignal::Trigger(id) => trigger_id = id,
                                                    WorkerSignal::ConfigChanged => {
                                                        config_changed = true;
                                                        break;
                                                    }
                                                    WorkerSignal::Deleted => {
                                                        env.channel_senders
                                                            .remove_if(&key, |_, h| h.owner_id == owner_id);
                                                        return;
                                                    }
                                                }
                                            }
                                            if config_changed {
                                                break;
                                            }
                                            // Coalesce trigger; continue sleeping until the fixed deadline without querying storage!
                                        }
                                        Some(WorkerSignal::ConfigChanged) => {
                                            // Config changed via web; break sleep loop immediately to re-read config
                                            break;
                                        }
                                        Some(WorkerSignal::Deleted) | None => {
                                            env.channel_senders
                                                .remove_if(&key, |_, h| h.owner_id == owner_id);
                                            return;
                                        }
                                    }
                                }
                            }
                        }

                        // Re-query storage and evaluate CAS/delivery after deadline or config change
                        continue;
                    }
                }
            }

            delivery_attempts += 1;

            // 3. Channel existence, guild membership, type, and bot permission check
            if let Err(err) = check_bot_channel_permission(&http, key.0, key.1).await {
                tracing::warn!(
                    "Sticky message delivery guard rejected channel {}: {}",
                    key.1,
                    err
                );
                break;
            }

            // 4. Send the new sticky message at the bottom of the channel
            let create_msg = CreateMessage::new()
                .content(&sticky.content)
                .allowed_mentions(CreateAllowedMentions::new());

            let new_msg = match key.1.send_message(&http, create_msg).await {
                Ok(msg) => msg,
                Err(e) => {
                    tracing::error!("Failed to post sticky message in channel {}: {}", key.1, e);
                    break;
                }
            };

            // 5. Commit delivery state to storage with optimistic concurrency control (CAS)
            let now = Utc::now();
            let commit_res = env
                .storage
                .update_sticky_posted_message(
                    &gid_str,
                    &cid_str,
                    &new_msg.id.to_string(),
                    now,
                    sticky.generation,
                    sticky.config_revision,
                )
                .await;

            match commit_res {
                Ok(Some(_updated)) => {
                    // 6. Delete previously posted sticky message (if any)
                    if let Some(ref old_id_str) = sticky.last_posted_message_id
                        && old_id_str != &new_msg.id.to_string()
                        && old_id_str != &trigger_id.to_string()
                        && let Ok(old_id_num) = old_id_str.parse::<u64>()
                    {
                        let old_mid = MessageId::new(old_id_num);
                        let _ = key.1.delete_message(&http, old_mid).await;
                    }
                    break;
                }
                Ok(None) => {
                    // Stale configuration: config changed during flight.
                    // Compensate by deleting the newly posted obsolete message.
                    tracing::warn!(
                        "Sticky message in channel {} was superseded during delivery (expected gen={}, rev={}); deleting stale message {}",
                        key.1,
                        sticky.generation,
                        sticky.config_revision,
                        new_msg.id
                    );
                    let _ = key.1.delete_message(&http, new_msg.id).await;
                    // Drain any newer triggers that arrived while we were sending
                    while let Ok(next) = rx.try_recv() {
                        match next {
                            WorkerSignal::Trigger(id) => trigger_id = id,
                            WorkerSignal::ConfigChanged => {}
                            WorkerSignal::Deleted => {
                                env.channel_senders
                                    .remove_if(&key, |_, h| h.owner_id == owner_id);
                                return;
                            }
                        }
                    }
                    // Loop again: fetch updated config and deliver immediately without waiting for another user message
                    continue;
                }
                Err(err) => {
                    tracing::error!(
                        "Failed to commit sticky message delivery to storage in channel {}: {}",
                        key.1,
                        err
                    );
                    let _ = key.1.delete_message(&http, new_msg.id).await;
                    break;
                }
            }
        }
    }

    // Worker timed out or channel closed; clean up only if still registered owner
    env.channel_senders
        .remove_if(&key, |_, h| h.owner_id == owner_id);
}
