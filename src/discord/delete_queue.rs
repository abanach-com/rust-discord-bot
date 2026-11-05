use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, anyhow};
use chrono::Utc;
use dashmap::DashMap;
use serenity::all::{ChannelId, GuildId, Http, MessageId};
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeleteOutcome {
    pub requested: usize,
    pub unique: usize,
    pub deleted: usize,
    pub already_gone: usize,
    pub failed: usize,
}

pub struct DeleteRequest {
    pub guild_id: GuildId,
    pub channel_id: ChannelId,
    pub message_ids: Vec<MessageId>,
    pub reply: Option<oneshot::Sender<DeleteOutcome>>,
}

#[derive(Clone)]
pub struct DeleteCoordinator {
    channels: Arc<DashMap<(GuildId, ChannelId), mpsc::Sender<DeleteRequest>>>,
    max_channels_per_guild: usize,
    channel_queue_capacity: usize,
    batch_window_ms: u64,
}

impl DeleteCoordinator {
    pub fn new(max_channels_per_guild: usize, channel_queue_capacity: usize) -> Self {
        Self {
            channels: Arc::new(DashMap::new()),
            max_channels_per_guild,
            channel_queue_capacity,
            batch_window_ms: 100,
        }
    }

    pub fn with_batch_window(mut self, window_ms: u64) -> Self {
        self.batch_window_ms = window_ms;
        self
    }

    pub fn active_channel_count(&self) -> usize {
        self.channels.len()
    }

    fn get_or_spawn_channel_worker(
        &self,
        http: Arc<Http>,
        guild_id: GuildId,
        channel_id: ChannelId,
    ) -> Result<mpsc::Sender<DeleteRequest>> {
        let key = (guild_id, channel_id);
        if let Some(sender) = self.channels.get(&key) {
            return Ok(sender.value().clone());
        }

        // Count channels in this guild to prevent channel queue exhaustion
        let guild_channel_count = self
            .channels
            .iter()
            .filter(|entry| entry.key().0 == guild_id)
            .count();

        if guild_channel_count >= self.max_channels_per_guild {
            return Err(anyhow!(
                "Too many active moderation delete queues for guild {guild_id} (limit: {})",
                self.max_channels_per_guild
            ));
        }

        let (tx, rx) = mpsc::channel(self.channel_queue_capacity);
        let channels_map = self.channels.clone();
        let batch_window = Duration::from_millis(self.batch_window_ms);

        tokio::spawn(async move {
            run_channel_worker(channels_map, key, rx, http, batch_window).await;
        });

        self.channels.insert(key, tx.clone());
        Ok(tx)
    }

    pub async fn enqueue(
        &self,
        http: Arc<Http>,
        guild_id: GuildId,
        channel_id: ChannelId,
        message_ids: Vec<MessageId>,
        reply: Option<oneshot::Sender<DeleteOutcome>>,
    ) -> Result<()> {
        if message_ids.is_empty() {
            if let Some(r) = reply {
                let _ = r.send(DeleteOutcome::default());
            }
            return Ok(());
        }

        let sender = self.get_or_spawn_channel_worker(http, guild_id, channel_id)?;
        let req = DeleteRequest {
            guild_id,
            channel_id,
            message_ids,
            reply,
        };

        sender.try_send(req).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => {
                anyhow!("Moderation delete queue full for channel {channel_id} (backpressure)")
            }
            mpsc::error::TrySendError::Closed(_) => {
                anyhow!("Moderation delete queue closed for channel {channel_id}")
            }
        })?;

        Ok(())
    }

    pub async fn delete_message(
        &self,
        http: Arc<Http>,
        guild_id: GuildId,
        channel_id: ChannelId,
        message_id: MessageId,
    ) -> Result<DeleteOutcome> {
        let (tx, rx) = oneshot::channel();
        self.enqueue(http, guild_id, channel_id, vec![message_id], Some(tx))
            .await?;
        rx.await
            .map_err(|_| anyhow!("Delete worker dropped response for message {message_id}"))
    }

    pub async fn delete_messages(
        &self,
        http: Arc<Http>,
        guild_id: GuildId,
        channel_id: ChannelId,
        message_ids: Vec<MessageId>,
    ) -> Result<DeleteOutcome> {
        let (tx, rx) = oneshot::channel();
        self.enqueue(http, guild_id, channel_id, message_ids, Some(tx))
            .await?;
        rx.await.map_err(|_| {
            anyhow!("Delete worker dropped response for batch deletion in channel {channel_id}")
        })
    }
}

async fn run_channel_worker(
    channels_map: Arc<DashMap<(GuildId, ChannelId), mpsc::Sender<DeleteRequest>>>,
    key: (GuildId, ChannelId),
    mut rx: mpsc::Receiver<DeleteRequest>,
    http: Arc<Http>,
    batch_window: Duration,
) {
    let (_guild_id, channel_id) = key;
    let idle_timeout = Duration::from_secs(30);

    loop {
        let first_req = match tokio::time::timeout(idle_timeout, rx.recv()).await {
            Ok(Some(req)) => req,
            Ok(None) | Err(_) => {
                channels_map.remove(&key);
                break;
            }
        };

        let mut batch = vec![first_req];
        let deadline = tokio::time::Instant::now() + batch_window;

        while let Ok(remaining) = tokio::time::timeout_at(deadline, rx.recv()).await {
            if let Some(req) = remaining {
                batch.push(req);
                if batch.len() >= 100 {
                    break;
                }
            } else {
                break;
            }
        }

        let mut message_to_replies: HashMap<MessageId, Vec<usize>> = HashMap::new();
        let mut reply_senders: Vec<Option<oneshot::Sender<DeleteOutcome>>> = Vec::new();
        let mut outcomes_per_request: Vec<DeleteOutcome> = Vec::new();

        for (idx, req) in batch.into_iter().enumerate() {
            let requested_count = req.message_ids.len();
            reply_senders.push(req.reply);
            outcomes_per_request.push(DeleteOutcome {
                requested: requested_count,
                unique: 0,
                deleted: 0,
                already_gone: 0,
                failed: 0,
            });

            let mut req_unique_seen = HashSet::new();
            for mid in req.message_ids {
                if req_unique_seen.insert(mid) {
                    message_to_replies.entry(mid).or_default().push(idx);
                }
            }
            outcomes_per_request[idx].unique = req_unique_seen.len();
        }

        let unique_message_ids: Vec<MessageId> = message_to_replies.keys().copied().collect();
        if unique_message_ids.is_empty() {
            for (idx, sender) in reply_senders.into_iter().enumerate() {
                if let Some(s) = sender {
                    let _ = s.send(outcomes_per_request[idx].clone());
                }
            }
            continue;
        }

        let two_weeks_ago_unix = (Utc::now() - chrono::Duration::days(14)).timestamp();
        let mut bulk_ids = Vec::new();
        let mut old_ids = Vec::new();

        for mid in unique_message_ids {
            let created_unix = mid.created_at().unix_timestamp();
            if created_unix >= two_weeks_ago_unix {
                bulk_ids.push(mid);
            } else {
                old_ids.push(mid);
            }
        }

        let mut message_status: HashMap<MessageId, (usize, usize, usize)> = HashMap::new();

        // 1. Bulk deletable messages in chunks of 2..=100
        for chunk in bulk_ids.chunks(100) {
            if chunk.len() == 1 {
                let mid = chunk[0];
                match channel_id.delete_message(&http, mid).await {
                    Ok(()) => {
                        message_status.insert(mid, (1, 0, 0));
                    }
                    Err(e) => {
                        if is_already_gone_error(&e) {
                            message_status.insert(mid, (0, 1, 0));
                        } else {
                            tracing::warn!(error = %e, message_id = %mid, "Failed to delete single message via queue");
                            message_status.insert(mid, (0, 0, 1));
                        }
                    }
                }
            } else {
                match channel_id.delete_messages(&http, chunk).await {
                    Ok(()) => {
                        for &mid in chunk {
                            message_status.insert(mid, (1, 0, 0));
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, count = chunk.len(), "Failed to bulk delete chunk via queue; falling back to individual deletes");
                        for &mid in chunk {
                            match channel_id.delete_message(&http, mid).await {
                                Ok(()) => {
                                    message_status.insert(mid, (1, 0, 0));
                                }
                                Err(err) => {
                                    if is_already_gone_error(&err) {
                                        message_status.insert(mid, (0, 1, 0));
                                    } else {
                                        message_status.insert(mid, (0, 0, 1));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // 2. Older messages individually
        for mid in old_ids {
            match channel_id.delete_message(&http, mid).await {
                Ok(()) => {
                    message_status.insert(mid, (1, 0, 0));
                }
                Err(e) => {
                    if is_already_gone_error(&e) {
                        message_status.insert(mid, (0, 1, 0));
                    } else {
                        tracing::warn!(error = %e, message_id = %mid, "Failed to delete older message via queue");
                        message_status.insert(mid, (0, 0, 1));
                    }
                }
            }
        }

        for (mid, (del, gone, fail)) in message_status {
            if let Some(request_indices) = message_to_replies.get(&mid) {
                for &req_idx in request_indices {
                    outcomes_per_request[req_idx].deleted += del;
                    outcomes_per_request[req_idx].already_gone += gone;
                    outcomes_per_request[req_idx].failed += fail;
                }
            }
        }

        for (idx, sender) in reply_senders.into_iter().enumerate() {
            if let Some(s) = sender {
                let _ = s.send(outcomes_per_request[idx].clone());
            }
        }
    }
}

fn is_already_gone_error(err: &serenity::Error) -> bool {
    let err_str = err.to_string();
    err_str.contains("Unknown Message")
        || err_str.contains("Unknown Channel")
        || err_str.contains("10008")
        || err_str.contains("10003")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_delete_queue_backpressure_and_channel_limits() {
        let coordinator = DeleteCoordinator::new(2, 2);
        let http = Arc::new(Http::new("dummy-token"));
        let guild_id = GuildId::new(100);

        // Fill channel 1 queue (capacity = 2)
        coordinator
            .enqueue(
                http.clone(),
                guild_id,
                ChannelId::new(1),
                vec![MessageId::new(10)],
                None,
            )
            .await
            .expect("enqueue 1");

        coordinator
            .enqueue(
                http.clone(),
                guild_id,
                ChannelId::new(1),
                vec![MessageId::new(11)],
                None,
            )
            .await
            .expect("enqueue 2");

        // Third enqueue must trigger backpressure (queue full)
        let backpressure_err = coordinator
            .enqueue(
                http.clone(),
                guild_id,
                ChannelId::new(1),
                vec![MessageId::new(12)],
                None,
            )
            .await;
        assert!(backpressure_err.is_err());
        assert!(
            backpressure_err
                .unwrap_err()
                .to_string()
                .contains("backpressure")
        );

        // Channel 2 in same guild succeeds
        coordinator
            .enqueue(
                http.clone(),
                guild_id,
                ChannelId::new(2),
                vec![MessageId::new(20)],
                None,
            )
            .await
            .expect("enqueue channel 2");

        // Channel 3 in same guild exceeds max_channels_per_guild (limit = 2)
        let limit_err = coordinator
            .enqueue(
                http.clone(),
                guild_id,
                ChannelId::new(3),
                vec![MessageId::new(30)],
                None,
            )
            .await;
        assert!(limit_err.is_err());
        assert!(
            limit_err
                .unwrap_err()
                .to_string()
                .contains("Too many active moderation delete queues")
        );

        // Another guild can still create queues
        let guild_b = GuildId::new(200);
        coordinator
            .enqueue(
                http.clone(),
                guild_b,
                ChannelId::new(3),
                vec![MessageId::new(30)],
                None,
            )
            .await
            .expect("enqueue other guild");
    }

    #[test]
    fn test_is_already_gone_error_detection() {
        let io_err = serenity::Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Unknown Message (error 10008)",
        ));
        assert!(is_already_gone_error(&io_err));

        let perm_err = serenity::Error::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Missing Permissions (error 50013)",
        ));
        assert!(!is_already_gone_error(&perm_err));
    }
}
