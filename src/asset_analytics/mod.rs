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
    pub is_guild_owned: bool,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

/// Aggregated usage statistics and metadata for a single asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetUsageStat {
    pub asset_id: String,
    pub asset_kind: AssetKind,
    pub name: Option<String>,
    pub animated: bool,
    pub is_guild_owned: bool,
    pub usage_today: i64,
    pub usage_7d: i64,
    pub usage_30d: i64,
    pub usage_total: i64,
    pub period_usage: i64,
    pub first_seen_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub unused_days: Option<i64>,
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
    is_guild_owned: bool,
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
}

impl AssetAnalyticsBatcher {
    pub fn new(storage: crate::storage::Storage) -> Self {
        Self {
            storage,
            counts: DashMap::new(),
            last_seen: DashMap::new(),
            inventory: DashMap::new(),
            dedupe: Mutex::new(BoundedMessageDedupe::new(10_000)),
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
            let is_guild_owned = match guild_emojis {
                Some(owned_list) => owned_list.iter().any(|id| id == &emoji.id),
                None => true,
            };

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
                    inv.is_guild_owned = is_guild_owned;
                })
                .or_insert_with(|| InMemInventory {
                    name: Some(emoji.name),
                    animated: emoji.animated,
                    is_guild_owned,
                    first_seen_at: now,
                    last_seen_at: now,
                });

            total_recorded += 1;
        }

        // 3. Record stickers
        for sticker in stickers {
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
                })
                .or_insert_with(|| InMemInventory {
                    name: Some(sticker.name.clone()),
                    animated: sticker.animated,
                    is_guild_owned: true,
                    first_seen_at: now,
                    last_seen_at: now,
                });

            total_recorded += 1;
        }

        total_recorded
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
        is_guild_owned: bool,
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
                inv.is_guild_owned = is_guild_owned;
            })
            .or_insert_with(|| InMemInventory {
                name: name.map(|s| s.to_string()),
                animated,
                is_guild_owned,
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

/// Helper to aggregate raw daily rows and inventory into a full `AssetUsageSummary`.
pub fn calculate_asset_summary(
    guild_id: &str,
    target_kind: Option<AssetKind>,
    period: &str,
    daily_rows: &[AssetUsageDaily],
    inventory_rows: &[AssetInventoryItem],
) -> AssetUsageSummary {
    let now = Utc::now();
    let today = now.date_naive();
    let seven_days_ago = today - chrono::Duration::days(7);
    let thirty_days_ago = today - chrono::Duration::days(30);

    let period_cutoff: Option<NaiveDate> = match period {
        "today" => Some(today),
        "7d" => Some(seven_days_ago),
        "30d" => Some(thirty_days_ago),
        _ => None,
    };

    // Index inventory by (kind, id)
    let mut inv_map = std::collections::HashMap::new();
    for inv in inventory_rows {
        if inv.guild_id == guild_id
            && let Ok(kind) = inv.asset_kind.parse::<AssetKind>()
            && (target_kind.is_none() || target_kind == Some(kind))
        {
            inv_map.insert((kind, inv.asset_id.clone()), inv.clone());
        }
    }

    // Aggregate daily rows by (kind, id)
    struct Agg {
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
            .or_insert_with(|| Agg {
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

    // Combine all known assets from both daily rows and inventory
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

        let usage_today = agg.map(|a| a.today).unwrap_or(0);
        let usage_7d = agg.map(|a| a.seven_days).unwrap_or(0);
        let usage_30d = agg.map(|a| a.thirty_days).unwrap_or(0);
        let usage_total = agg.map(|a| a.total).unwrap_or(0);
        let period_usage = agg.map(|a| a.period).unwrap_or(0);
        let last_used_at = agg.map(|a| a.last_used_at);

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
            is_guild_owned: inv.map(|i| i.is_guild_owned).unwrap_or(true),
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

    // Sort items:
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
            true,
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
            true,
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
            true,
            10,
            now,
        );
        batcher.record_asset_usage(
            guild_id,
            AssetKind::Sticker,
            shared_id,
            Some("fire_sticker"),
            true,
            true,
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
            false, // external
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
            true, // owned
            15,
            now,
        );

        batcher.flush().await.unwrap();

        let g1_summary = storage
            .get_asset_usage_summary(guild_1, Some(AssetKind::Emoji), "all")
            .await
            .unwrap();
        assert_eq!(g1_summary.total_usages, 7);
        assert!(!g1_summary.items[0].is_guild_owned);

        let g2_summary = storage
            .get_asset_usage_summary(guild_2, Some(AssetKind::Emoji), "all")
            .await
            .unwrap();
        assert_eq!(g2_summary.total_usages, 15);
        assert!(g2_summary.items[0].is_guild_owned);
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
}
