use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use chrono::Utc;
use dashmap::DashMap;
use serenity::all::{
    Channel, ChannelId, ChannelType, CreateAllowedMentions, CreateMessage, GuildId, Http,
    MessageId, Permissions,
};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::storage::Storage;

#[derive(Clone, Debug)]
pub struct StickyRefresh {
    pub guild_id: GuildId,
    pub channel_id: ChannelId,
    pub trigger_message_id: MessageId,
}

#[derive(Clone)]
struct WorkerHandle {
    owner_id: Uuid,
    tx: mpsc::Sender<MessageId>,
}

pub struct StickyCoordinator {
    storage: Storage,
    channel_senders: Arc<DashMap<(GuildId, ChannelId), WorkerHandle>>,
}

impl StickyCoordinator {
    pub fn new(storage: Storage) -> Self {
        Self {
            storage,
            channel_senders: Arc::new(DashMap::new()),
        }
    }

    pub fn active_worker_count(&self) -> usize {
        self.channel_senders.len()
    }

    pub fn enqueue_refresh(
        &self,
        http: Arc<Http>,
        guild_id: GuildId,
        channel_id: ChannelId,
        trigger_message_id: MessageId,
    ) {
        let key = (guild_id, channel_id);
        use dashmap::mapref::entry::Entry;

        let spawn_info = match self.channel_senders.entry(key) {
            Entry::Occupied(entry) if !entry.get().tx.is_closed() => {
                let _ = entry.get().tx.try_send(trigger_message_id);
                None
            }
            Entry::Occupied(mut entry) => {
                let (tx, rx) = mpsc::channel(32);
                let _ = tx.try_send(trigger_message_id);
                let owner_id = Uuid::new_v4();
                entry.insert(WorkerHandle { owner_id, tx });
                Some((rx, owner_id))
            }
            Entry::Vacant(entry) => {
                let (tx, rx) = mpsc::channel(32);
                let _ = tx.try_send(trigger_message_id);
                let owner_id = Uuid::new_v4();
                entry.insert(WorkerHandle { owner_id, tx });
                Some((rx, owner_id))
            }
        };

        if let Some((rx, owner_id)) = spawn_info {
            let storage = self.storage.clone();
            let channel_senders = self.channel_senders.clone();
            tokio::spawn(async move {
                run_channel_worker(key, owner_id, rx, http, storage, channel_senders).await;
            });
        }
    }

    pub fn remove_channel(&self, guild_id: &str, channel_id: &str) {
        if let (Ok(gid), Ok(cid)) = (guild_id.parse::<u64>(), channel_id.parse::<u64>()) {
            let key = (GuildId::new(gid), ChannelId::new(cid));
            self.channel_senders.remove(&key);
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
    mut rx: mpsc::Receiver<MessageId>,
    http: Arc<Http>,
    storage: Storage,
    channel_senders: Arc<DashMap<(GuildId, ChannelId), WorkerHandle>>,
) {
    let gid_str = key.0.get().to_string();
    let cid_str = key.1.get().to_string();

    while let Ok(maybe_trigger) = tokio::time::timeout(Duration::from_secs(300), rx.recv()).await {
        let Some(mut trigger_id) = maybe_trigger else {
            break;
        };

        // Drain pending burst triggers to coalesce into the latest
        while let Ok(next) = rx.try_recv() {
            trigger_id = next;
        }

        // 1. Fetch current sticky configuration
        let sticky = match storage.get_sticky_message(&gid_str, &cid_str).await {
            Ok(Some(s)) => s,
            _ => {
                // If deleted or errored, remove worker if we are still registered owner and exit
                channel_senders.remove_if(&key, |_, h| h.owner_id == owner_id);
                return;
            }
        };

        // 2. Cooldown check
        if let Some(last_bumped) = sticky.last_bumped_at {
            let elapsed = Utc::now().signed_duration_since(last_bumped).num_seconds();
            if elapsed < sticky.cooldown_seconds as i64 {
                // Still inside cooldown window; drop this burst
                continue;
            }
        }

        // 3. Channel existence, guild membership, type, and bot permission check
        if let Err(err) = check_bot_channel_permission(&http, key.0, key.1).await {
            tracing::warn!(
                "Sticky message delivery guard rejected channel {}: {}",
                key.1,
                err
            );
            continue;
        }

        // 4. Send the new sticky message at the bottom of the channel
        let create_msg = CreateMessage::new()
            .content(&sticky.content)
            .allowed_mentions(CreateAllowedMentions::new());

        let new_msg = match key.1.send_message(&http, create_msg).await {
            Ok(msg) => msg,
            Err(e) => {
                tracing::error!("Failed to post sticky message in channel {}: {}", key.1, e);
                continue;
            }
        };

        // 5. Commit delivery state to storage
        let now = Utc::now();
        let _ = storage
            .update_sticky_posted_message(&gid_str, &cid_str, &new_msg.id.to_string(), now)
            .await;

        // 6. Delete previously posted sticky message (if any)
        if let Some(ref old_id_str) = sticky.last_posted_message_id
            && old_id_str != &new_msg.id.to_string()
            && old_id_str != &trigger_id.to_string()
            && let Ok(old_id_num) = old_id_str.parse::<u64>()
        {
            let old_mid = MessageId::new(old_id_num);
            let _ = key.1.delete_message(&http, old_mid).await;
        }
    }

    // Worker timed out or channel closed; clean up only if still registered owner
    channel_senders.remove_if(&key, |_, h| h.owner_id == owner_id);
}
