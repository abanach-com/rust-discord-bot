use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;

use chrono::{DateTime, NaiveDate, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use crate::storage::validate_snowflake;

/// Kind of Discord asset being tracked for analytics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    Emoji,
    Sticker,
}

impl AssetKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Emoji => "emoji",
            Self::Sticker => "sticker",
        }
    }
}

impl std::fmt::Display for AssetKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for AssetKind {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "emoji" => Ok(Self::Emoji),
            "sticker" => Ok(Self::Sticker),
            other => anyhow::bail!("Invalid asset kind: '{other}'. Expected 'emoji' or 'sticker'"),
        }
    }
}

/// Parsed custom emoji from message content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCustomEmoji {
    pub name: String,
    pub id: String,
    pub animated: bool,
}

/// Parsed sticker from message payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSticker {
    pub id: String,
    pub name: String,
    pub animated: bool,
}

/// Daily aggregated usage record for an asset.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AssetUsageDaily {
    pub guild_id: String,
    pub usage_date: NaiveDate,
    pub asset_kind: String,
    pub asset_id: String,
    pub usage_count: i64,
    pub last_used_at: DateTime<Utc>,
}

/// Inventory record for a known emoji or sticker in a guild.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AssetInventoryItem {
    pub guild_id: String,
    pub asset_kind: String,
    pub asset_id: String,
    pub name: Option<String>,
    pub animated: bool,
    pub is_guild_owned: Option<bool>,
    #[serde(default = "default_true")]
    pub is_active: bool,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

fn default_true() -> bool {
    true
}

/// Aggregated usage statistics and metadata for a single asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetUsageStat {
    pub asset_id: String,
    pub asset_kind: AssetKind,
    pub name: Option<String>,
    pub animated: bool,
    pub is_guild_owned: Option<bool>,
    #[serde(default = "default_true")]
    pub is_active: bool,
    pub usage_today: i64,
    pub usage_7d: i64,
    pub usage_30d: i64,
    pub usage_total: i64,
    pub period_usage: i64,
    pub first_seen_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub unused_days: Option<i64>,
}

/// Aggregated usage query result mapped from SQL group-by query.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AssetUsageAggregate {
    pub asset_kind: String,
    pub asset_id: String,
    pub usage_total: i64,
    pub usage_7d: i64,
    pub usage_30d: i64,
    pub usage_today: i64,
    pub period_usage: i64,
    pub last_used_at: Option<DateTime<Utc>>,
}

/// Summary report returned to the dashboard and API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetUsageSummary {
    pub guild_id: String,
    pub asset_kind: Option<AssetKind>,
    pub period: String,
    pub total_usages: i64,
    pub unique_assets_used: usize,
    pub top: Vec<AssetUsageStat>,
    pub bottom: Vec<AssetUsageStat>,
    pub unused: Vec<AssetUsageStat>,
    pub items: Vec<AssetUsageStat>,
}

/// Key identifying an aggregated daily usage bucket.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct AssetUsageKey {
    pub guild_id: String,
    pub day: NaiveDate,
    pub kind: AssetKind,
    pub asset_id: String,
}

/// Metadata stored in the in-memory inventory buffer before flush.
#[derive(Debug, Clone)]
struct InMemInventory {
    name: Option<String>,
    animated: bool,
    is_guild_owned: Option<bool>,
    is_active: bool,
    first_seen_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
}

/// Bounded FIFO deduplication cache for message IDs.
///
/// Dedupe Policy:
/// Approximate analytics with bounded message-id LRU/FIFO queue.
/// Replay events from gateway reconnects within recent window are ignored.
#[derive(Debug)]
pub struct BoundedMessageDedupe {
    max_entries: usize,
    seen: HashSet<String>,
    order: VecDeque<String>,
}

impl BoundedMessageDedupe {
    pub fn new(max_entries: usize) -> Self {
        Self {
            max_entries: max_entries.max(1),
            seen: HashSet::new(),
            order: VecDeque::new(),
        }
    }

    /// Returns `true` if `message_id` was already seen recently (duplicate event).
    /// If not seen, records it and returns `false`.
    pub fn is_duplicate_or_record(&mut self, message_id: &str) -> bool {
        if self.seen.contains(message_id) {
            return true;
        }

        if self.order.len() >= self.max_entries
            && let Some(oldest) = self.order.pop_front()
        {
            self.seen.remove(&oldest);
        }

        self.seen.insert(message_id.to_string());
        self.order.push_back(message_id.to_string());
        false
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

/// Parses custom Discord emojis from text content:
/// - Static: `<:name:id>`
/// - Animated: `<a:name:id>`
///
/// Strict rules:
/// - Name: 2..=32 chars, ASCII alphanumeric or underscore `[a-zA-Z0-9_]`.
/// - ID: 17..=20 chars, ASCII digits `[0-9]`.
/// - Repeated emojis are all counted (e.g. 3x in message -> 3 occurrences).
pub fn parse_custom_emojis(content: &str) -> Vec<ParsedCustomEmoji> {
    let mut emojis = Vec::new();
    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'<' {
            let (animated, start_name) = if i + 3 < len && &bytes[i..i + 3] == b"<a:" {
                (true, i + 3)
            } else if i + 2 < len && &bytes[i..i + 2] == b"<:" {
                (false, i + 2)
            } else {
                i += 1;
                continue;
            };

            // Find next ':'
            let mut colon_pos = None;
            let mut j = start_name;
            while j < len && j - start_name <= 33 {
                if bytes[j] == b':' {
                    colon_pos = Some(j);
                    break;
                }
                j += 1;
            }

            if let Some(colon_idx) = colon_pos {
                let name_bytes = &bytes[start_name..colon_idx];
                if name_bytes.len() >= 2
                    && name_bytes.len() <= 32
                    && name_bytes
                        .iter()
                        .all(|&b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    // Find closing '>'
                    let start_id = colon_idx + 1;
                    let mut gt_pos = None;
                    let mut k = start_id;
                    while k < len && k - start_id <= 21 {
                        if bytes[k] == b'>' {
                            gt_pos = Some(k);
                            break;
                        }
                        k += 1;
                    }

                    if let Some(gt_idx) = gt_pos {
                        let id_bytes = &bytes[start_id..gt_idx];
                        if id_bytes.len() >= 17
                            && id_bytes.len() <= 20
                            && id_bytes.iter().all(|&b| b.is_ascii_digit())
                        {
                            let name = String::from_utf8_lossy(name_bytes).to_string();
                            let id = String::from_utf8_lossy(id_bytes).to_string();
                            emojis.push(ParsedCustomEmoji { name, id, animated });
                            i = gt_idx + 1;
                            continue;
                        }
                    }
                }
            }
        }
        i += 1;
    }

    emojis
}

/// In-memory batch collector for Discord emoji and sticker analytics.
///
/// Features:
/// - Fast concurrent DashMap updates on message events without database locks.
/// - Privacy by design: NEVER stores user_id or message.content.
/// - Bounded message deduplication LRU preventing double-counting on gateway reconnects.
/// - Atomic flush with failure recovery: if PostgreSQL/storage fails during flush,
///   the snapshot batch is merged back into the in-memory state without data loss.
pub struct AssetAnalyticsBatcher {
    storage: crate::storage::Storage,
    counts: DashMap<AssetUsageKey, u64>,
    last_seen: DashMap<AssetUsageKey, DateTime<Utc>>,
    inventory: DashMap<(String, AssetKind, String), InMemInventory>,
    dedupe: Mutex<BoundedMessageDedupe>,
    flush_lock: tokio::sync::Mutex<()>,
}

impl AssetAnalyticsBatcher {
    pub fn new(storage: crate::storage::Storage) -> Self {
        Self {
            storage,
            counts: DashMap::new(),
            last_seen: DashMap::new(),
            inventory: DashMap::new(),
            dedupe: Mutex::new(BoundedMessageDedupe::new(10_000)),
            flush_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Records emoji and sticker assets from an incoming message.
    /// Returns number of assets registered, or 0 if duplicate message.
    pub fn record_message(
        &self,
        guild_id: &str,
        message_id: &str,
        content: &str,
        stickers: &[ParsedSticker],
        guild_emojis: Option<&[String]>,
        guild_stickers: Option<&[String]>,
    ) -> usize {
        if validate_snowflake("guild_id", guild_id).is_err() {
            return 0;
        }

        // 1. Message deduplication check
        {
            let mut dedupe = match self.dedupe.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            if dedupe.is_duplicate_or_record(message_id) {
                return 0;
            }
        }

        let now = Utc::now();
        let today = now.date_naive();
        let mut total_recorded = 0;

        // 2. Parse and record custom emojis
        let emojis = parse_custom_emojis(content);
        for emoji in emojis {
            // Issue #199: Some(true) = owned, Some(false) = external, None = unconfirmed cache miss
            let is_guild_owned =
                guild_emojis.map(|owned_list| owned_list.iter().any(|id| id == &emoji.id));

            let key = AssetUsageKey {
                guild_id: guild_id.to_string(),
                day: today,
                kind: AssetKind::Emoji,
                asset_id: emoji.id.clone(),
            };

            *self.counts.entry(key.clone()).or_insert(0) += 1;
            self.last_seen.insert(key, now);

            self.inventory
                .entry((guild_id.to_string(), AssetKind::Emoji, emoji.id))
                .and_modify(|inv| {
                    inv.last_seen_at = now;
                    inv.animated = emoji.animated;
                    inv.name = Some(emoji.name.clone());
                    if is_guild_owned.is_some() {
                        inv.is_guild_owned = is_guild_owned;
                    }
                    inv.is_active = true;
                })
                .or_insert_with(|| InMemInventory {
                    name: Some(emoji.name),
                    animated: emoji.animated,
                    is_guild_owned,
                    is_active: true,
                    first_seen_at: now,
                    last_seen_at: now,
                });

            total_recorded += 1;
        }

        // 3. Record stickers
        for sticker in stickers {
            // Issue #199: Some(true) = owned, Some(false) = external, None = unconfirmed cache miss
            let is_guild_owned =
                guild_stickers.map(|owned_list| owned_list.iter().any(|id| id == &sticker.id));

            let key = AssetUsageKey {
                guild_id: guild_id.to_string(),
                day: today,
                kind: AssetKind::Sticker,
                asset_id: sticker.id.clone(),
            };

            *self.counts.entry(key.clone()).or_insert(0) += 1;
            self.last_seen.insert(key, now);

            self.inventory
                .entry((guild_id.to_string(), AssetKind::Sticker, sticker.id.clone()))
                .and_modify(|inv| {
                    inv.last_seen_at = now;
                    inv.animated = sticker.animated;
                    inv.name = Some(sticker.name.clone());
                    if is_guild_owned.is_some() {
                        inv.is_guild_owned = is_guild_owned;
                    }
                    inv.is_active = true;
                })
                .or_insert_with(|| InMemInventory {
                    name: Some(sticker.name.clone()),
                    animated: sticker.animated,
                    is_guild_owned,
                    is_active: true,
                    first_seen_at: now,
                    last_seen_at: now,
                });

            total_recorded += 1;
        }

        total_recorded
    }

    /// Records snapshot of currently configured guild emojis and stickers (Issue #198).
    pub fn record_inventory_snapshot(
        &self,
        guild_id: &str,
        emojis: &[ParsedCustomEmoji],
        stickers: &[ParsedSticker],
    ) {
        if validate_snowflake("guild_id", guild_id).is_err() {
            return;
        }
        let now = Utc::now();

        for emoji in emojis {
            if validate_snowflake("asset_id", &emoji.id).is_err() {
                continue;
            }
            self.inventory
                .entry((guild_id.to_string(), AssetKind::Emoji, emoji.id.clone()))
                .and_modify(|inv| {
                    inv.last_seen_at = now;
                    inv.name = Some(emoji.name.clone());
                    inv.animated = emoji.animated;
                    inv.is_guild_owned = Some(true);
                    inv.is_active = true;
                })
                .or_insert_with(|| InMemInventory {
                    name: Some(emoji.name.clone()),
                    animated: emoji.animated,
                    is_guild_owned: Some(true),
                    is_active: true,
                    first_seen_at: now,
                    last_seen_at: now,
                });
        }

        for sticker in stickers {
            if validate_snowflake("asset_id", &sticker.id).is_err() {
                continue;
            }
            self.inventory
                .entry((guild_id.to_string(), AssetKind::Sticker, sticker.id.clone()))
                .and_modify(|inv| {
                    inv.last_seen_at = now;
                    inv.name = Some(sticker.name.clone());
                    inv.animated = sticker.animated;
                    inv.is_guild_owned = Some(true);
                    inv.is_active = true;
                })
                .or_insert_with(|| InMemInventory {
                    name: Some(sticker.name.clone()),
                    animated: sticker.animated,
                    is_guild_owned: Some(true),
                    is_active: true,
                    first_seen_at: now,
                    last_seen_at: now,
                });
        }
    }

    /// Directly record raw asset usage (e.g. for testing or explicit recording).
    #[allow(clippy::too_many_arguments)]
    pub fn record_asset_usage(
        &self,
        guild_id: &str,
        kind: AssetKind,
        asset_id: &str,
        name: Option<&str>,
        animated: bool,
        is_guild_owned: Option<bool>,
        count: u64,
        timestamp: DateTime<Utc>,
    ) {
        if validate_snowflake("guild_id", guild_id).is_err()
            || validate_snowflake("asset_id", asset_id).is_err()
            || count == 0
        {
            return;
        }

        let day = timestamp.date_naive();
        let key = AssetUsageKey {
            guild_id: guild_id.to_string(),
            day,
            kind,
            asset_id: asset_id.to_string(),
        };

        *self.counts.entry(key.clone()).or_insert(0) += count;
        self.last_seen
            .entry(key)
            .and_modify(|ts| {
                if timestamp > *ts {
                    *ts = timestamp;
                }
            })
            .or_insert(timestamp);

        self.inventory
            .entry((guild_id.to_string(), kind, asset_id.to_string()))
            .and_modify(|inv| {
                if timestamp > inv.last_seen_at {
                    inv.last_seen_at = timestamp;
                }
                if let Some(n) = name {
                    inv.name = Some(n.to_string());
                }
                inv.animated = animated;
                if is_guild_owned.is_some() {
                    inv.is_guild_owned = is_guild_owned;
                }
                inv.is_active = true;
            })
            .or_insert_with(|| InMemInventory {
                name: name.map(|s| s.to_string()),
                animated,
                is_guild_owned,
                is_active: true,
                first_seen_at: timestamp,
                last_seen_at: timestamp,
            });
    }

    /// Flushes buffered usage counts and inventory items to persistent storage.
    ///
    /// Recovery:
    /// In case of storage failure, the snapshot items are merged back into
    /// the in-memory maps so no analytics counts are lost.
    pub async fn flush(&self) -> anyhow::Result<usize> {
        let _guard = self.flush_lock.lock().await;

        // 1. Drain counts snapshot
        let count_keys: Vec<AssetUsageKey> =
            self.counts.iter().map(|kv| kv.key().clone()).collect();
        if count_keys.is_empty() && self.inventory.is_empty() {
            return Ok(0);
        }

        let mut snapshot_counts = Vec::with_capacity(count_keys.len());
        for key in count_keys {
            if let Some((_, count)) = self.counts.remove(&key) {
                let last_used = self
                    .last_seen
                    .remove(&key)
                    .map(|(_, ts)| ts)
                    .unwrap_or_else(Utc::now);
                snapshot_counts.push((key, count, last_used));
            }
        }

        // 2. Drain inventory snapshot
        let inv_keys: Vec<(String, AssetKind, String)> =
            self.inventory.iter().map(|kv| kv.key().clone()).collect();
        let mut snapshot_inventory = Vec::with_capacity(inv_keys.len());
        for key in inv_keys {
            if let Some((_, inv)) = self.inventory.remove(&key) {
                snapshot_inventory.push((key, inv));
            }
        }

        // 3. Prepare storage models
        let daily_items: Vec<AssetUsageDaily> = snapshot_counts
            .iter()
            .map(|(key, count, last_used)| AssetUsageDaily {
                guild_id: key.guild_id.clone(),
                usage_date: key.day,
                asset_kind: key.kind.as_str().to_string(),
                asset_id: key.asset_id.clone(),
                usage_count: *count as i64,
                last_used_at: *last_used,
            })
            .collect();

        let inventory_items: Vec<AssetInventoryItem> = snapshot_inventory
            .iter()
            .map(|((guild_id, kind, asset_id), inv)| AssetInventoryItem {
                guild_id: guild_id.clone(),
                asset_kind: kind.as_str().to_string(),
                asset_id: asset_id.clone(),
                name: inv.name.clone(),
                animated: inv.animated,
                is_guild_owned: inv.is_guild_owned,
                is_active: inv.is_active,
                first_seen_at: inv.first_seen_at,
                last_seen_at: inv.last_seen_at,
            })
            .collect();

        // 4. Execute atomic batch write
        let write_result = self
            .storage
            .record_asset_usage_batch(&daily_items, &inventory_items)
            .await;

        match write_result {
            Ok(rows) => Ok(rows),
            Err(err) => {
                // RECOVERY: Merge back snapshot into in-memory state
                for (key, count, last_used) in snapshot_counts {
                    *self.counts.entry(key.clone()).or_insert(0) += count;
                    self.last_seen
                        .entry(key)
                        .and_modify(|ts| {
                            if last_used > *ts {
                                *ts = last_used;
                            }
                        })
                        .or_insert(last_used);
                }

                for ((guild_id, kind, asset_id), inv) in snapshot_inventory {
                    self.inventory
                        .entry((guild_id, kind, asset_id))
                        .and_modify(|existing| {
                            if inv.last_seen_at > existing.last_seen_at {
                                existing.last_seen_at = inv.last_seen_at;
                            }
                            if inv.first_seen_at < existing.first_seen_at {
                                existing.first_seen_at = inv.first_seen_at;
                            }
                            if inv.is_guild_owned.is_some() {
                                existing.is_guild_owned = inv.is_guild_owned;
                            }
                            existing.is_active = inv.is_active;
                        })
                        .or_insert(inv);
                }

                Err(err)
            }
        }
    }

    pub fn pending_counts_len(&self) -> usize {
        self.counts.len()
    }

    pub fn pending_inventory_len(&self) -> usize {
        self.inventory.len()
    }
}

/// Periodic flush worker running with interval and handling graceful shutdown (Issue #197).
pub async fn run_asset_analytics_flush_loop(
    batcher: std::sync::Arc<AssetAnalyticsBatcher>,
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
    every: std::time::Duration,
) {
    let mut interval = tokio::time::interval(every);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval.tick().await;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Err(error) = batcher.flush().await {
                    tracing::warn!(?error, "asset analytics periodic flush failed");
                }
            }
            _ = shutdown.recv() => {
                tracing::info!("asset analytics flush loop received shutdown signal, performing final flush");
                if let Err(error) = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    batcher.flush(),
                ).await {
                    tracing::warn!(?error, "asset analytics final flush timed out or failed");
                }
                break;
            }
        }
    }
}

/// Helper to assemble a full `AssetUsageSummary` from asset usage aggregates and inventory items.
pub fn build_asset_summary_from_aggregates(
    guild_id: &str,
    target_kind: Option<AssetKind>,
    period: &str,
    aggregates: &[AssetUsageAggregate],
    inventory_rows: &[AssetInventoryItem],
    now: DateTime<Utc>,
) -> AssetUsageSummary {
    let mut inv_map = std::collections::HashMap::new();
    for inv in inventory_rows {
        if inv.guild_id == guild_id
            && let Ok(kind) = inv.asset_kind.parse::<AssetKind>()
            && (target_kind.is_none() || target_kind == Some(kind))
        {
            inv_map.insert((kind, inv.asset_id.clone()), inv.clone());
        }
    }

    let mut agg_map = std::collections::HashMap::new();
    for agg in aggregates {
        if let Ok(kind) = agg.asset_kind.parse::<AssetKind>()
            && (target_kind.is_none() || target_kind == Some(kind))
        {
            agg_map.insert((kind, agg.asset_id.clone()), agg.clone());
        }
    }

    let mut all_keys: HashSet<(AssetKind, String)> = HashSet::new();
    for k in inv_map.keys() {
        all_keys.insert(k.clone());
    }
    for k in agg_map.keys() {
        all_keys.insert(k.clone());
    }

    let mut items = Vec::new();
    let mut total_usages = 0;
    let mut unique_assets_used = 0;

    for (kind, id) in all_keys {
        let inv = inv_map.get(&(kind, id.clone()));
        let agg = agg_map.get(&(kind, id.clone()));

        let usage_today = agg.map(|a| a.usage_today).unwrap_or(0);
        let usage_7d = agg.map(|a| a.usage_7d).unwrap_or(0);
        let usage_30d = agg.map(|a| a.usage_30d).unwrap_or(0);
        let usage_total = agg.map(|a| a.usage_total).unwrap_or(0);
        let period_usage = agg.map(|a| a.period_usage).unwrap_or(0);
        let last_used_at = agg.and_then(|a| a.last_used_at);

        total_usages += period_usage;
        if period_usage > 0 {
            unique_assets_used += 1;
        }

        let unused_days = last_used_at.map(|lu| (now - lu).num_days().max(0));

        let stat = AssetUsageStat {
            asset_id: id,
            asset_kind: kind,
            name: inv.and_then(|i| i.name.clone()),
            animated: inv.map(|i| i.animated).unwrap_or(false),
            is_guild_owned: inv.and_then(|i| i.is_guild_owned),
            is_active: inv.map(|i| i.is_active).unwrap_or(true),
            usage_today,
            usage_7d,
            usage_30d,
            usage_total,
            period_usage,
            first_seen_at: inv.map(|i| i.first_seen_at),
            last_used_at,
            unused_days,
        };
        items.push(stat);
    }

    // Stable sort for items: highest total usage first, then asset_kind, then asset_id (Issue #200)
    items.sort_by(|a, b| {
        b.usage_total
            .cmp(&a.usage_total)
            .then_with(|| a.asset_kind.as_str().cmp(b.asset_kind.as_str()))
            .then_with(|| a.asset_id.cmp(&b.asset_id))
    });

    // Top: sorted by period_usage desc
    let mut top = items.clone();
    top.retain(|i| i.period_usage > 0);
    top.sort_by_key(|a| std::cmp::Reverse(a.period_usage));

    // Bottom: used at least once in period, sorted by period_usage asc
    let mut bottom = top.clone();
    bottom.sort_by_key(|a| a.period_usage);

    // Unused: period_usage == 0
    let mut unused = items.clone();
    unused.retain(|i| i.period_usage == 0);
    unused.sort_by(|a, b| match (a.unused_days, b.unused_days) {
        (Some(da), Some(db)) => db.cmp(&da),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.asset_id.cmp(&b.asset_id),
    });

    AssetUsageSummary {
        guild_id: guild_id.to_string(),
        asset_kind: target_kind,
        period: period.to_string(),
        total_usages,
        unique_assets_used,
        top,
        bottom,
        unused,
        items,
    }
}

/// Aggregates raw daily rows and inventory into a full `AssetUsageSummary` at a given reference time.
///
/// Date Boundaries (Issue #196):
/// - "today": UTC today.
/// - "7d": UTC today and 6 previous days (exactly 7 calendar days inclusive).
/// - "30d": UTC today and 29 previous days (exactly 30 calendar days inclusive).
pub fn calculate_asset_summary_at(
    guild_id: &str,
    target_kind: Option<AssetKind>,
    period: &str,
    daily_rows: &[AssetUsageDaily],
    inventory_rows: &[AssetInventoryItem],
    now: DateTime<Utc>,
) -> AssetUsageSummary {
    let today = now.date_naive();
    let seven_days_ago = today - chrono::Duration::days(6);
    let thirty_days_ago = today - chrono::Duration::days(29);

    let period_cutoff: Option<NaiveDate> = match period {
        "today" => Some(today),
        "7d" => Some(seven_days_ago),
        "30d" => Some(thirty_days_ago),
        _ => None,
    };

    struct InMemAgg {
        today: i64,
        seven_days: i64,
        thirty_days: i64,
        total: i64,
        period: i64,
        last_used_at: DateTime<Utc>,
    }

    let mut agg_map = std::collections::HashMap::new();
    for row in daily_rows {
        if row.guild_id != guild_id {
            continue;
        }
        let Ok(kind) = row.asset_kind.parse::<AssetKind>() else {
            continue;
        };
        if let Some(target) = target_kind
            && target != kind
        {
            continue;
        }

        let entry = agg_map
            .entry((kind, row.asset_id.clone()))
            .or_insert_with(|| InMemAgg {
                today: 0,
                seven_days: 0,
                thirty_days: 0,
                total: 0,
                period: 0,
                last_used_at: row.last_used_at,
            });

        if row.last_used_at > entry.last_used_at {
            entry.last_used_at = row.last_used_at;
        }

        entry.total += row.usage_count;

        if row.usage_date == today {
            entry.today += row.usage_count;
        }
        if row.usage_date >= seven_days_ago {
            entry.seven_days += row.usage_count;
        }
        if row.usage_date >= thirty_days_ago {
            entry.thirty_days += row.usage_count;
        }

        let in_period = match period_cutoff {
            Some(cutoff) => row.usage_date >= cutoff,
            None => true,
        };
        if in_period {
            entry.period += row.usage_count;
        }
    }

    let aggregates: Vec<AssetUsageAggregate> = agg_map
        .into_iter()
        .map(|((kind, id), agg)| AssetUsageAggregate {
            asset_kind: kind.as_str().to_string(),
            asset_id: id,
            usage_total: agg.total,
            usage_7d: agg.seven_days,
            usage_30d: agg.thirty_days,
            usage_today: agg.today,
            period_usage: agg.period,
            last_used_at: Some(agg.last_used_at),
        })
        .collect();

    build_asset_summary_from_aggregates(
        guild_id,
        target_kind,
        period,
        &aggregates,
        inventory_rows,
        now,
    )
}

/// Helper to aggregate raw daily rows and inventory into a full `AssetUsageSummary` at `Utc::now()`.
pub fn calculate_asset_summary(
    guild_id: &str,
    target_kind: Option<AssetKind>,
    period: &str,
    daily_rows: &[AssetUsageDaily],
    inventory_rows: &[AssetInventoryItem],
) -> AssetUsageSummary {
    calculate_asset_summary_at(
        guild_id,
        target_kind,
        period,
        daily_rows,
        inventory_rows,
        Utc::now(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_custom_emojis() {
        let msg =
            "Hello <:pepe:123456789012345678> and animated <a:dance_party:987654321098765432>!";
        let parsed = parse_custom_emojis(msg);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "pepe");
        assert_eq!(parsed[0].id, "123456789012345678");
        assert!(!parsed[0].animated);

        assert_eq!(parsed[1].name, "dance_party");
        assert_eq!(parsed[1].id, "987654321098765432");
        assert!(parsed[1].animated);
    }

    #[test]
    fn test_parse_custom_emojis_repeated_and_distinct() {
        // Message with same emoji 3x
        let msg =
            "<:pepe:123456789012345678> text <:pepe:123456789012345678> <:pepe:123456789012345678>";
        let parsed = parse_custom_emojis(msg);
        assert_eq!(parsed.len(), 3);
        assert!(parsed.iter().all(|e| e.id == "123456789012345678"));

        // Two emojis with same name but different IDs
        let diff = "<:pepe:111111111111111111> <:pepe:222222222222222222>";
        let parsed_diff = parse_custom_emojis(diff);
        assert_eq!(parsed_diff.len(), 2);
        assert_eq!(parsed_diff[0].name, "pepe");
        assert_eq!(parsed_diff[1].name, "pepe");
        assert_ne!(parsed_diff[0].id, parsed_diff[1].id);
    }

    #[test]
    fn test_parse_custom_emojis_ignores_invalid_tags() {
        let msg = "<:a:123> <:invalid name:123456789012345678> <::123456789012345678> <:valid_emoji:123456789012345678>";
        let parsed = parse_custom_emojis(msg);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "valid_emoji");
        assert_eq!(parsed[0].id, "123456789012345678");
    }

    #[test]
    fn test_bounded_message_dedupe() {
        let mut dedupe = BoundedMessageDedupe::new(3);
        assert!(!dedupe.is_duplicate_or_record("msg1"));
        assert!(!dedupe.is_duplicate_or_record("msg2"));
        assert!(!dedupe.is_duplicate_or_record("msg3"));

        // Duplicate within capacity
        assert!(dedupe.is_duplicate_or_record("msg2"));

        // Add 4th item -> msg1 is evicted
        assert!(!dedupe.is_duplicate_or_record("msg4"));
        assert_eq!(dedupe.len(), 3);

        // msg1 should now be considered new again because it fell out of the LRU window
        assert!(!dedupe.is_duplicate_or_record("msg1"));
    }

    #[tokio::test]
    async fn test_batcher_atomic_sum_and_failure_recovery() {
        let store = crate::storage::MemoryStore::default();
        let storage = crate::storage::Storage::Memory(store.clone());
        let batcher = AssetAnalyticsBatcher::new(storage.clone());

        let guild_id = "100000000000000001";
        let emoji_id = "200000000000000002";
        let now = Utc::now();

        // 1. Record 5 usages
        batcher.record_asset_usage(
            guild_id,
            AssetKind::Emoji,
            emoji_id,
            Some("pepe"),
            false,
            Some(true),
            5,
            now,
        );
        assert_eq!(batcher.pending_counts_len(), 1);

        // 2. Inject failure during flush
        store.inject_asset_analytics_failure(true);
        let res = batcher.flush().await;
        assert!(
            res.is_err(),
            "Flush must fail when database error is injected"
        );

        // Verify data was NOT lost from batcher!
        assert_eq!(
            batcher.pending_counts_len(),
            1,
            "Pending batch must be recovered after failure"
        );

        // 3. Clear failure and flush successfully
        store.inject_asset_analytics_failure(false);
        let res = batcher.flush().await;
        assert!(res.is_ok());
        assert_eq!(batcher.pending_counts_len(), 0);

        // 4. Record 3 more usages and flush again
        batcher.record_asset_usage(
            guild_id,
            AssetKind::Emoji,
            emoji_id,
            Some("pepe"),
            false,
            Some(true),
            3,
            now,
        );
        let res = batcher.flush().await;
        assert!(res.is_ok());

        // 5. Verify atomic sum in storage is 8
        let summary = storage
            .get_asset_usage_summary(guild_id, Some(AssetKind::Emoji), "all")
            .await
            .unwrap();
        assert_eq!(summary.total_usages, 8);
        assert_eq!(summary.items.len(), 1);
        assert_eq!(summary.items[0].usage_total, 8);
        assert_eq!(summary.items[0].name.as_deref(), Some("pepe"));
    }

    #[tokio::test]
    async fn test_emoji_and_sticker_same_id_no_collision() {
        let store = crate::storage::MemoryStore::default();
        let storage = crate::storage::Storage::Memory(store);
        let batcher = AssetAnalyticsBatcher::new(storage.clone());

        let guild_id = "100000000000000001";
        let shared_id = "999888777666555444";
        let now = Utc::now();

        batcher.record_asset_usage(
            guild_id,
            AssetKind::Emoji,
            shared_id,
            Some("fire_emoji"),
            false,
            Some(true),
            10,
            now,
        );
        batcher.record_asset_usage(
            guild_id,
            AssetKind::Sticker,
            shared_id,
            Some("fire_sticker"),
            true,
            Some(true),
            4,
            now,
        );

        batcher.flush().await.unwrap();

        let emoji_summary = storage
            .get_asset_usage_summary(guild_id, Some(AssetKind::Emoji), "all")
            .await
            .unwrap();
        assert_eq!(emoji_summary.total_usages, 10);
        assert_eq!(emoji_summary.items.len(), 1);
        assert_eq!(emoji_summary.items[0].asset_kind, AssetKind::Emoji);
        assert_eq!(emoji_summary.items[0].name.as_deref(), Some("fire_emoji"));

        let sticker_summary = storage
            .get_asset_usage_summary(guild_id, Some(AssetKind::Sticker), "all")
            .await
            .unwrap();
        assert_eq!(sticker_summary.total_usages, 4);
        assert_eq!(sticker_summary.items.len(), 1);
        assert_eq!(sticker_summary.items[0].asset_kind, AssetKind::Sticker);
        assert_eq!(
            sticker_summary.items[0].name.as_deref(),
            Some("fire_sticker")
        );
    }

    #[tokio::test]
    async fn test_guild_isolation_and_external_emoji() {
        let store = crate::storage::MemoryStore::default();
        let storage = crate::storage::Storage::Memory(store);
        let batcher = AssetAnalyticsBatcher::new(storage.clone());

        let guild_1 = "100000000000000001";
        let guild_2 = "200000000000000002";
        let emoji_id = "300000000000000003";
        let now = Utc::now();

        // Guild 1: external emoji used
        batcher.record_asset_usage(
            guild_1,
            AssetKind::Emoji,
            emoji_id,
            Some("nitro_cat"),
            false,
            Some(false), // external
            7,
            now,
        );

        // Guild 2: guild-owned emoji used
        batcher.record_asset_usage(
            guild_2,
            AssetKind::Emoji,
            emoji_id,
            Some("nitro_cat"),
            false,
            Some(true), // owned
            15,
            now,
        );

        batcher.flush().await.unwrap();

        let g1_summary = storage
            .get_asset_usage_summary(guild_1, Some(AssetKind::Emoji), "all")
            .await
            .unwrap();
        assert_eq!(g1_summary.total_usages, 7);
        assert_eq!(g1_summary.items[0].is_guild_owned, Some(false));

        let g2_summary = storage
            .get_asset_usage_summary(guild_2, Some(AssetKind::Emoji), "all")
            .await
            .unwrap();
        assert_eq!(g2_summary.total_usages, 15);
        assert_eq!(g2_summary.items[0].is_guild_owned, Some(true));
    }

    #[tokio::test]
    async fn test_retention_prune_respects_guild_and_age() {
        let store = crate::storage::MemoryStore::default();
        let storage = crate::storage::Storage::Memory(store);

        let guild_1 = "100000000000000001";
        let guild_2 = "200000000000000002";
        let asset_id = "400000000000000004";

        let old_date = Utc::now().date_naive() - chrono::Duration::days(100);
        let recent_date = Utc::now().date_naive() - chrono::Duration::days(10);
        let now = Utc::now();

        let daily_items = vec![
            AssetUsageDaily {
                guild_id: guild_1.to_string(),
                usage_date: old_date,
                asset_kind: "emoji".to_string(),
                asset_id: asset_id.to_string(),
                usage_count: 5,
                last_used_at: now,
            },
            AssetUsageDaily {
                guild_id: guild_1.to_string(),
                usage_date: recent_date,
                asset_kind: "emoji".to_string(),
                asset_id: asset_id.to_string(),
                usage_count: 3,
                last_used_at: now,
            },
            AssetUsageDaily {
                guild_id: guild_2.to_string(),
                usage_date: old_date,
                asset_kind: "emoji".to_string(),
                asset_id: asset_id.to_string(),
                usage_count: 12,
                last_used_at: now,
            },
        ];

        storage
            .record_asset_usage_batch(&daily_items, &[])
            .await
            .unwrap();

        // Prune older than 30 days for guild 1 only
        let pruned = storage
            .prune_asset_analytics(Some(guild_1), 30)
            .await
            .unwrap();
        assert_eq!(pruned, 1);

        // Guild 1 should only have recent row (usage = 3)
        let g1 = storage
            .get_asset_usage_summary(guild_1, Some(AssetKind::Emoji), "all")
            .await
            .unwrap();
        assert_eq!(g1.total_usages, 3);

        // Guild 2 old row should NOT have been pruned (usage = 12)
        let g2 = storage
            .get_asset_usage_summary(guild_2, Some(AssetKind::Emoji), "all")
            .await
            .unwrap();
        assert_eq!(g2.total_usages, 12);
    }

    #[test]
    fn test_date_boundaries_exact_7d_and_30d() {
        let fixed_now: DateTime<Utc> = "2026-11-05T12:00:00Z".parse().unwrap();
        let guild_id = "100000000000000001";
        let asset_id = "200000000000000002";

        let d_today = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
        let d_6_ago = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap(); // today - 6 (inclusive for 7d)
        let d_7_ago = NaiveDate::from_ymd_opt(2026, 10, 29).unwrap(); // today - 7 (MUST be excluded from 7d)
        let d_29_ago = NaiveDate::from_ymd_opt(2026, 10, 7).unwrap(); // today - 29 (inclusive for 30d)
        let d_30_ago = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap(); // today - 30 (MUST be excluded from 30d)

        let daily_rows = vec![
            AssetUsageDaily {
                guild_id: guild_id.to_string(),
                usage_date: d_today,
                asset_kind: "emoji".to_string(),
                asset_id: asset_id.to_string(),
                usage_count: 1,
                last_used_at: fixed_now,
            },
            AssetUsageDaily {
                guild_id: guild_id.to_string(),
                usage_date: d_6_ago,
                asset_kind: "emoji".to_string(),
                asset_id: asset_id.to_string(),
                usage_count: 3,
                last_used_at: fixed_now,
            },
            AssetUsageDaily {
                guild_id: guild_id.to_string(),
                usage_date: d_7_ago,
                asset_kind: "emoji".to_string(),
                asset_id: asset_id.to_string(),
                usage_count: 10,
                last_used_at: fixed_now,
            },
            AssetUsageDaily {
                guild_id: guild_id.to_string(),
                usage_date: d_29_ago,
                asset_kind: "emoji".to_string(),
                asset_id: asset_id.to_string(),
                usage_count: 100,
                last_used_at: fixed_now,
            },
            AssetUsageDaily {
                guild_id: guild_id.to_string(),
                usage_date: d_30_ago,
                asset_kind: "emoji".to_string(),
                asset_id: asset_id.to_string(),
                usage_count: 1000,
                last_used_at: fixed_now,
            },
        ];

        let inv_rows = vec![AssetInventoryItem {
            guild_id: guild_id.to_string(),
            asset_kind: "emoji".to_string(),
            asset_id: asset_id.to_string(),
            name: Some("test_emoji".to_string()),
            animated: false,
            is_guild_owned: Some(true),
            is_active: true,
            first_seen_at: fixed_now,
            last_seen_at: fixed_now,
        }];

        // 1. Period = today: only d_today (1 usage)
        let s_today = calculate_asset_summary_at(
            guild_id,
            Some(AssetKind::Emoji),
            "today",
            &daily_rows,
            &inv_rows,
            fixed_now,
        );
        assert_eq!(s_today.total_usages, 1);
        assert_eq!(s_today.items[0].usage_today, 1);

        // 2. Period = 7d: exactly 7 days (today + 6 prior): 1 + 3 = 4 usages!
        // 10 usages from 7 days ago (2026-10-29) MUST be excluded!
        let s_7d = calculate_asset_summary_at(
            guild_id,
            Some(AssetKind::Emoji),
            "7d",
            &daily_rows,
            &inv_rows,
            fixed_now,
        );
        assert_eq!(s_7d.total_usages, 4);
        assert_eq!(s_7d.items[0].usage_7d, 4);
        assert_eq!(s_7d.items[0].period_usage, 4);

        // 3. Period = 30d: exactly 30 days (today + 29 prior): 1 + 3 + 10 + 100 = 114 usages!
        // 1000 usages from 30 days ago (2026-10-06) MUST be excluded!
        let s_30d = calculate_asset_summary_at(
            guild_id,
            Some(AssetKind::Emoji),
            "30d",
            &daily_rows,
            &inv_rows,
            fixed_now,
        );
        assert_eq!(s_30d.total_usages, 114);
        assert_eq!(s_30d.items[0].usage_30d, 114);
        assert_eq!(s_30d.items[0].period_usage, 114);

        // 4. Period = all: 1 + 3 + 10 + 100 + 1000 = 1114 usages
        let s_all = calculate_asset_summary_at(
            guild_id,
            Some(AssetKind::Emoji),
            "all",
            &daily_rows,
            &inv_rows,
            fixed_now,
        );
        assert_eq!(s_all.total_usages, 1114);
        assert_eq!(s_all.items[0].usage_total, 1114);
    }

    #[tokio::test]
    async fn test_external_stickers_and_cache_miss_ownership() {
        let store = crate::storage::MemoryStore::default();
        let storage = crate::storage::Storage::Memory(store);
        let batcher = AssetAnalyticsBatcher::new(storage.clone());

        let guild_id = "100000000000000001";
        let owned_sticker = ParsedSticker {
            id: "200000000000000001".to_string(),
            name: "local_sticker".to_string(),
            animated: false,
        };
        let external_sticker = ParsedSticker {
            id: "200000000000000002".to_string(),
            name: "nitro_sticker".to_string(),
            animated: true,
        };

        // 1. Message with known guild stickers list:
        // owned_sticker is in cache -> Some(true), external_sticker not in cache -> Some(false)
        let known_stickers = vec!["200000000000000001".to_string()];
        batcher.record_message(
            guild_id,
            "300000000000000001",
            "hello",
            &[owned_sticker, external_sticker],
            None,
            Some(&known_stickers),
        );

        batcher.flush().await.unwrap();

        let summary = storage
            .get_asset_usage_summary(guild_id, Some(AssetKind::Sticker), "all")
            .await
            .unwrap();

        let s_owned = summary
            .items
            .iter()
            .find(|i| i.asset_id == "200000000000000001")
            .unwrap();
        assert_eq!(s_owned.is_guild_owned, Some(true));

        let s_ext = summary
            .items
            .iter()
            .find(|i| i.asset_id == "200000000000000002")
            .unwrap();
        assert_eq!(s_ext.is_guild_owned, Some(false));

        // 2. Cache miss scenario: both guild_emojis and guild_stickers are None
        // Ownership must be None (unknown), NOT true!
        let unknown_emoji_content = "<:mystery:400000000000000001>";
        batcher.record_message(
            guild_id,
            "300000000000000002",
            unknown_emoji_content,
            &[],
            None,
            None,
        );

        batcher.flush().await.unwrap();

        let emoji_summary = storage
            .get_asset_usage_summary(guild_id, Some(AssetKind::Emoji), "all")
            .await
            .unwrap();

        let s_unknown = emoji_summary
            .items
            .iter()
            .find(|i| i.asset_id == "400000000000000001")
            .unwrap();
        assert_eq!(
            s_unknown.is_guild_owned, None,
            "Cache miss emoji must have is_guild_owned = None, not false or true"
        );
    }

    #[tokio::test]
    async fn test_unused_emojis_filter_and_guild_sync() {
        let store = crate::storage::MemoryStore::default();
        let storage = crate::storage::Storage::Memory(store);
        let batcher = AssetAnalyticsBatcher::new(storage.clone());

        let guild_id = "100000000000000001";
        let now = Utc::now();
        let past_45d = now - chrono::Duration::days(45);

        // 1. Sync inventory snapshot from guild (3 emojis)
        let emojis = vec![
            ParsedCustomEmoji {
                name: "active_used_recently".to_string(),
                id: "100000000000000001".to_string(),
                animated: false,
            },
            ParsedCustomEmoji {
                name: "active_used_long_ago".to_string(),
                id: "100000000000000002".to_string(),
                animated: false,
            },
            ParsedCustomEmoji {
                name: "never_ever_used".to_string(),
                id: "100000000000000003".to_string(),
                animated: false,
            },
        ];

        batcher.record_inventory_snapshot(guild_id, &emojis, &[]);

        // 2. Record usage for #1 today, and #2 45 days ago; #3 has 0 usages
        batcher.record_asset_usage(
            guild_id,
            AssetKind::Emoji,
            "100000000000000001",
            Some("active_used_recently"),
            false,
            Some(true),
            5,
            now,
        );
        batcher.record_asset_usage(
            guild_id,
            AssetKind::Emoji,
            "100000000000000002",
            Some("active_used_long_ago"),
            false,
            Some(true),
            2,
            past_45d,
        );

        batcher.flush().await.unwrap();

        let summary = storage
            .get_asset_usage_summary(guild_id, Some(AssetKind::Emoji), "all")
            .await
            .unwrap();

        // Filter inactive for 30+ days
        let mut unused: Vec<_> = summary
            .items
            .into_iter()
            .filter(|item| {
                item.is_guild_owned.unwrap_or(false)
                    && item.is_active
                    && item.unused_days.map(|d| d >= 30).unwrap_or(true)
            })
            .collect();

        unused.sort_by(|a, b| match (a.unused_days, b.unused_days) {
            (None, None) => a.asset_id.cmp(&b.asset_id),
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (Some(da), Some(db)) => db.cmp(&da),
        });

        assert_eq!(unused.len(), 2);
        // Never used emoji comes first
        assert_eq!(unused[0].asset_id, "100000000000000003");
        assert_eq!(unused[0].unused_days, None);
        // 45 days inactive emoji comes second
        assert_eq!(unused[1].asset_id, "100000000000000002");
        assert!(unused[1].unused_days.unwrap() >= 45);
    }

    #[tokio::test]
    async fn test_flush_worker_and_graceful_shutdown() {
        let store = crate::storage::MemoryStore::default();
        let storage = crate::storage::Storage::Memory(store);
        let batcher = std::sync::Arc::new(AssetAnalyticsBatcher::new(storage.clone()));

        let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
        let batcher_clone = batcher.clone();

        // Run worker loop with 1 hour tick (so it won't tick during test, only final flush on shutdown)
        let worker_handle = tokio::spawn(run_asset_analytics_flush_loop(
            batcher_clone,
            shutdown_rx,
            std::time::Duration::from_secs(3600),
        ));

        let guild_id = "100000000000000001";
        let asset_id = "500000000000000005";
        batcher.record_asset_usage(
            guild_id,
            AssetKind::Emoji,
            asset_id,
            Some("shutdown_emoji"),
            false,
            Some(true),
            99,
            Utc::now(),
        );
        assert_eq!(batcher.pending_counts_len(), 1);

        // Send shutdown
        let _ = shutdown_tx.send(());
        let join_res = tokio::time::timeout(std::time::Duration::from_secs(2), worker_handle).await;
        assert!(join_res.is_ok(), "Worker must shut down cleanly");

        // Verify final flush succeeded: batcher is drained and storage has data
        assert_eq!(batcher.pending_counts_len(), 0);
        let summary = storage
            .get_asset_usage_summary(guild_id, Some(AssetKind::Emoji), "all")
            .await
            .unwrap();
        assert_eq!(summary.total_usages, 99);
    }
}
