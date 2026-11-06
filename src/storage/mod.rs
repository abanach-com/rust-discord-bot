mod memory;

mod postgres;

mod progression;

use chrono::{DateTime, Utc};

use serde::{Deserialize, Serialize};

use serde_json::Value;

use uuid::Uuid;

pub use memory::MemoryStore;

pub use postgres::{PostgresStore, run_safe_migrations};

pub use crate::policy::CommandPolicyRule;

pub use crate::duty::{
    DutyReconcileOutcome, DutyTimeRange, ModeratorDutyProfile, ModeratorDutyRoleGrant,
    ModeratorDutySettings, ModeratorDutyState, RecurringSchedule, StaffAudience,
};

pub use crate::recruitment::{
    CandidateApplicationView, RecruitmentApplication, RecruitmentQuestion, RecruitmentReview,
    RecruitmentRoleFulfillment, RecruitmentTrack, RecruitmentTrackConfig, StaffApplicationView,
    SubmittedAnswer, TransitionApplicationParams,
};

pub use crate::help_forum::{HelpForumConfig, HelpForumSession, HelpForumStats};

pub use crate::asset_analytics::{
    AssetInventoryItem, AssetKind, AssetUsageDaily, AssetUsageStat, AssetUsageSummary,
};

pub use progression::{
    CoinTransfer, CoinTransferOutcome, DailyClaimOutcome, MemberProgress, ProgressMetric,
    ReputationGrantOutcome, XpAward, level_for_xp, total_xp_for_level,
};

#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
pub enum Storage {
    Memory(MemoryStore),

    Postgres(PostgresStore),
}

#[derive(Clone, Debug)]
pub struct UpdateModuleSettingsParams<'a> {
    pub guild_id: &'a str,

    pub module_id: &'a str,

    pub enabled: bool,

    pub config: Value,

    pub actor_id: Option<&'a str>,

    pub audit_event: &'a str,

    pub audit_data: Value,
}

#[derive(Clone, Debug)]
pub struct AdjustMemberProgressParams<'a> {
    pub guild_id: &'a str,

    pub user_id: &'a str,

    pub xp_delta: i64,

    pub coins_delta: i64,

    pub reputation_delta: i64,

    pub actor_id: Option<&'a str>,

    pub reason: Option<&'a str>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserErasureReport {
    pub user_id: String,
    pub guild_id: String,
    pub progression_erased: bool,
    pub related_records_erased: usize,
    pub retained_records: Vec<RetainedRecordNotice>,
    pub erased_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetainedRecordNotice {
    pub entity: String,
    pub reason: String,
    pub retention_policy: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct GuildModuleSettings {
    pub guild_id: String,

    pub module_id: String,

    pub enabled: bool,

    pub config: Value,

    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct AuditLogEntry {
    pub id: i64,

    pub guild_id: Option<String>,

    pub actor_id: Option<String>,

    pub module_id: String,

    pub event: String,

    pub data: Value,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ModerationExecutionIntent {
    pub id: Uuid,

    pub interaction_id: Option<String>,

    pub guild_id: String,

    pub user_id: String,

    pub kind: String,

    pub step: String,

    pub reason: Option<String>,

    pub error: Option<String>,

    pub created_at: DateTime<Utc>,

    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ModuleDraftConfig {
    pub guild_id: String,

    pub module_id: String,

    pub config_json: Value,

    pub updated_by: String,

    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ModerationAppeal {
    pub id: Uuid,

    pub guild_id: String,

    pub case_number: i64,

    pub user_id: String,

    pub status: String,

    pub reason: String,

    pub evidence_url: Option<String>,

    pub staff_notes: Option<String>,

    pub created_at: DateTime<Utc>,

    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct TimedRoleGrant {
    pub id: Uuid,

    pub guild_id: String,

    pub user_id: String,

    pub role_id: String,

    pub expires_at: DateTime<Utc>,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ServerSnapshot {
    pub id: Uuid,

    pub guild_id: String,

    pub name: String,

    pub snapshot_json: Value,

    pub created_by: String,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct CustomAutomation {
    pub id: Uuid,

    pub guild_id: String,

    pub name: String,

    pub trigger_type: String,

    pub conditions_json: Value,

    pub actions_json: Value,

    pub enabled: bool,

    pub created_at: DateTime<Utc>,

    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct StickyMessage {
    pub guild_id: String,

    pub channel_id: String,

    pub message_id: String,

    pub content: String,

    pub cooldown_seconds: i32,

    pub last_updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct CommunityPoll {
    pub id: Uuid,

    pub guild_id: String,

    pub channel_id: String,

    pub question: String,

    pub options_json: Value,

    pub tallies_json: Value,

    pub is_anonymous: bool,

    pub expires_at: DateTime<Utc>,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Giveaway {
    pub id: Uuid,

    pub guild_id: String,

    pub channel_id: String,

    pub prize: String,

    pub winners_count: i32,

    pub participants_json: Value,

    pub winners_json: Value,

    pub ended: bool,

    pub expires_at: DateTime<Utc>,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct StarboardEntry {
    pub guild_id: String,

    pub source_channel_id: String,

    pub source_message_id: String,

    pub starboard_message_id: String,

    pub reaction_count: i32,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct StoreItem {
    pub id: Uuid,

    pub guild_id: String,

    pub name: String,

    pub description: String,

    pub price: i64,

    pub role_id: Option<String>,

    pub stock: Option<i32>,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct StorePurchase {
    pub id: Uuid,

    pub guild_id: String,

    pub user_id: String,

    pub item_id: Uuid,

    pub price_paid: i64,

    pub role_id: Option<String>,

    pub fulfillment_status: String,

    pub failure_reason: Option<String>,

    pub fulfilled_at: Option<DateTime<Utc>>,

    pub created_at: DateTime<Utc>,
}

pub fn validate_snowflake(label: &str, id: &str) -> anyhow::Result<u64> {
    anyhow::ensure!(!id.is_empty(), "{label} cannot be empty");
    anyhow::ensure!(
        id.chars().all(|c| c.is_ascii_digit()),
        "{label} must contain only decimal digits"
    );
    let parsed = id
        .parse::<u64>()
        .map_err(|_| anyhow::anyhow!("{label} must be a valid decimal Discord snowflake"))?;
    anyhow::ensure!(parsed != 0, "{label} must be nonzero");
    Ok(parsed)
}

pub fn snowflake_created_at(id: u64) -> DateTime<Utc> {
    let millis = (id >> 22) + 1_420_070_400_000;
    DateTime::from_timestamp_millis(millis as i64).unwrap_or_else(Utc::now)
}

/// Background janitor periodically pruning expired identity history records.
pub async fn run_identity_history_janitor(
    storage: Storage,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
    interval: std::time::Duration,
) {
    let mut timer = tokio::time::interval(interval);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    tracing::info!(
        interval_secs = interval.as_secs(),
        "started member identity history janitor"
    );

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                tracing::info!("member identity history janitor received shutdown signal");
                break;
            }
            _ = timer.tick() => {
                match storage.cleanup_expired_identity_history().await {
                    Ok(count) => {
                        if count > 0 {
                            tracing::info!(count, "expired member identity history records pruned");
                        } else {
                            tracing::debug!("expired member identity history check completed (0 pruned)");
                        }
                    }
                    Err(error) => {
                        tracing::warn!(?error, "failed to cleanup expired member identity history records");
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct MemberIdentityRecord {
    pub id: Uuid,
    pub guild_id: String,
    pub user_id: String,
    pub kind: String,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub observed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct MemberSnapshot {
    pub guild_id: String,
    pub user_id: String,
    pub username: String,
    pub global_name: Option<String>,
    pub nickname: Option<String>,
    pub role_ids: serde_json::Value,
    pub joined_at: Option<DateTime<Utc>>,
    pub account_created_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct UpsertMemberSnapshotParams<'a> {
    pub guild_id: &'a str,
    pub user_id: &'a str,
    pub username: &'a str,
    pub global_name: Option<&'a str>,
    pub nickname: Option<&'a str>,
    pub role_ids: &'a [String],
    pub joined_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MemberSearchFilter {
    #[serde(default)]
    pub role_ids: Vec<String>,
    #[serde(default)]
    pub exclude_role_ids: Vec<String>,
    pub joined_after: Option<DateTime<Utc>>,
    pub joined_before: Option<DateTime<Utc>>,
    pub account_created_after: Option<DateTime<Utc>>,
    pub account_created_before: Option<DateTime<Utc>>,
    pub min_case_count: Option<i64>,
    pub identity_changed_after: Option<DateTime<Utc>>,
    pub limit: Option<u16>,
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemberSearchRow {
    pub user_id: String,
    pub username: String,
    pub global_name: Option<String>,
    pub nickname: Option<String>,
    pub role_ids: Vec<String>,
    pub joined_at: Option<DateTime<Utc>>,
    pub account_created_at: Option<DateTime<Utc>>,
    pub moderation_case_count: i64,
    pub xp: Option<i64>,
    pub level: Option<i32>,
    pub message_count: Option<i64>,
    pub recent_name_changes: i64,
}

/// Encodes a pagination cursor as "{timestamp_millis}_{user_id}" or "null_{user_id}".
pub fn encode_member_search_cursor(joined_at: Option<DateTime<Utc>>, user_id: &str) -> String {
    match joined_at {
        Some(dt) => format!("{}_{}", dt.timestamp_millis(), user_id),
        None => format!("null_{}", user_id),
    }
}

/// Parses a pagination cursor into (joined_at, user_id).
/// Supports both composite keyset ("{millis}_{user_id}" or "null_{user_id}") and legacy plain user_id.
pub fn parse_member_search_cursor(cursor: &str) -> Option<(Option<DateTime<Utc>>, String)> {
    let trimmed = cursor.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some((left, right)) = trimmed.split_once('_') {
        let user_id = right.trim().to_string();
        if user_id.is_empty() || !user_id.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        if left.eq_ignore_ascii_case("null") || left.eq_ignore_ascii_case("none") {
            return Some((None, user_id));
        }
        if let Ok(ms) = left.parse::<i64>() {
            return DateTime::from_timestamp_millis(ms).map(|dt| (Some(dt), user_id));
        }
    }

    if trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Some((None, trimmed.to_string()));
    }

    None
}

/// Returns true if (row_joined_at, row_user_id) comes strictly after (cursor_joined_at, cursor_user_id)
/// according to ORDER BY joined_at DESC NULLS LAST, user_id ASC.
pub fn is_member_after_cursor(
    row_joined_at: Option<DateTime<Utc>>,
    row_user_id: &str,
    cursor_joined_at: Option<DateTime<Utc>>,
    cursor_user_id: &str,
) -> bool {
    match cursor_joined_at {
        Some(cur_dt) => match row_joined_at {
            Some(row_dt) => row_dt < cur_dt || (row_dt == cur_dt && row_user_id > cursor_user_id),
            None => true,
        },
        None => row_joined_at.is_none() && row_user_id > cursor_user_id,
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CleanupFilter {
    pub author_id: Option<String>,
    #[serde(default)]
    pub bots_only: bool,
    #[serde(default)]
    pub humans_only: bool,
    pub contains: Option<String>,
    pub has_link: Option<bool>,
    pub has_attachment: Option<bool>,
    #[serde(default)]
    pub duplicate_only: bool,
    #[serde(default)]
    pub include_pinned: bool,
    pub before_message_id: Option<String>,
    pub after_message_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct CleanupPlan {
    pub id: Uuid,
    pub guild_id: String,
    pub channel_id: String,
    pub actor_user_id: String,
    pub message_ids: Value,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
}

impl CleanupPlan {
    pub fn message_ids_vec(&self) -> Vec<String> {
        serde_json::from_value(self.message_ids.clone()).unwrap_or_default()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct CompanionChannelMapping {
    pub id: Uuid,
    pub guild_id: String,
    pub voice_channel_id: String,
    pub text_channel_id: String,
    pub allow_send_messages: bool,
    pub allow_read_history: bool,
    pub created_at: DateTime<Utc>,
    #[serde(default = "default_true_field")]
    pub enabled: bool,
}

pub const MAX_COMPANION_MAPPINGS_PER_GUILD: usize = 50;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompanionMappingItem {
    pub voice_channel_id: String,
    pub text_channel_id: String,
    #[serde(default = "default_true_field")]
    pub allow_send_messages: bool,
    #[serde(default = "default_true_field")]
    pub allow_read_history: bool,
}

fn default_true_field() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct CompanionChannelLease {
    pub guild_id: String,
    pub voice_channel_id: String,
    pub text_channel_id: String,
    pub user_id: String,
    pub granted_at: DateTime<Utc>,
    pub generation: Uuid,
    #[serde(default = "default_lease_status")]
    pub status: String,
    #[serde(default)]
    pub revoke_attempts: i32,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub revoked_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub allow_bits: i64,
    #[serde(default)]
    pub deny_bits: i64,
}

fn default_lease_status() -> String {
    "active".to_string()
}

#[derive(Clone, Copy, Debug)]
pub struct RecordCompanionLeaseParams<'a> {
    pub guild_id: &'a str,
    pub voice_channel_id: &'a str,
    pub text_channel_id: &'a str,
    pub user_id: &'a str,
    pub generation: Uuid,
    pub allow_bits: i64,
    pub deny_bits: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ProgressionSeason {
    pub id: Uuid,

    pub guild_id: String,

    pub season_number: i32,

    pub name: String,

    pub standings_json: Value,

    pub archived_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ScheduledEvent {
    pub id: Uuid,

    pub guild_id: String,

    pub title: String,

    pub description: String,

    pub start_time: DateTime<Utc>,

    pub end_time: Option<DateTime<Utc>>,

    pub timezone: String,

    pub max_participants: Option<i32>,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct EventRoster {
    pub event_id: Uuid,

    pub user_id: String,

    pub role_type: String,

    pub position: i32,

    pub joined_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Tournament {
    pub id: Uuid,

    pub guild_id: String,

    pub name: String,

    pub bracket_json: Value,

    pub status: String,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct InviteTracking {
    pub guild_id: String,

    pub code: String,

    pub inviter_id: String,

    pub uses: i32,

    pub campaign_tag: Option<String>,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct DynamicVoiceRoom {
    pub channel_id: String,

    pub guild_id: String,

    pub owner_id: String,

    pub empty_since: Option<DateTime<Utc>>,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct FeedSubscription {
    pub id: Uuid,

    pub guild_id: String,

    pub channel_id: String,

    pub feed_type: String,

    pub url_or_target: String,

    pub watermark: Option<String>,

    pub last_polled_at: Option<DateTime<Utc>>,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct GameAccountLink {
    pub id: Uuid,

    pub guild_id: String,

    pub user_id: String,

    pub platform: String,

    pub platform_account_id: String,

    pub verification_code: Option<String>,

    pub verified: bool,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct CountingGame {
    pub guild_id: String,

    pub channel_id: String,

    pub current_count: i64,

    pub last_user_id: Option<String>,

    pub highest_count: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct CardCollection {
    pub id: Uuid,

    pub guild_id: String,

    pub user_id: String,

    pub card_id: String,

    pub card_name: String,

    pub rarity: String,

    pub acquired_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct CardTrade {
    pub id: Uuid,

    pub guild_id: String,

    pub initiator_user_id: String,

    pub initiator_card_id: Uuid,

    pub target_user_id: String,

    pub target_card_id: Uuid,

    pub status: String,

    pub created_at: DateTime<Utc>,

    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct TupperProfile {
    pub id: Uuid,

    pub guild_id: String,

    pub user_id: String,

    pub name: String,

    pub avatar_url: Option<String>,

    pub prefix_or_brackets: String,

    pub created_at: DateTime<Utc>,
}

#[derive(thiserror::Error, Debug)]
pub enum TupperError {
    #[error("Tupper prefix already exists for this user")]
    PrefixConflict,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct FaqEntry {
    pub id: Uuid,

    pub guild_id: String,

    pub question: String,

    pub answer: String,

    pub aliases_json: Value,

    pub category: String,

    pub created_at: DateTime<Utc>,

    pub updated_at: DateTime<Utc>,
}

// ==========================================

// Phase 4: Integrations & Extensibility (#92-#107)

// ==========================================

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct IntegrationConnection {
    pub id: Uuid,

    pub guild_id: String,

    pub provider: String,

    pub provider_account_id: Option<String>,

    pub access_token: Option<String>,

    pub refresh_token: Option<String>,

    pub token_expires_at: Option<DateTime<Utc>>,

    pub scopes: Value,

    pub metadata: Value,

    pub enabled: bool,

    pub created_at: DateTime<Utc>,

    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct IntegrationWebhookSubscription {
    pub id: Uuid,

    pub guild_id: String,

    pub provider: String,

    pub external_id: String,

    pub secret_token: Option<String>,

    pub target_channel_id: String,

    pub config: Value,

    pub active: bool,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct IntegrationDeliveryLog {
    pub id: Uuid,

    pub guild_id: String,

    pub provider: String,

    pub event_type: String,

    pub status: String,

    pub error_message: Option<String>,

    pub payload_summary: Value,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct LinkedAccount {
    pub id: Uuid,

    pub guild_id: String,

    pub discord_user_id: String,

    pub provider: String,

    pub provider_subject: String,

    pub attributes: Value,

    pub verified_at: DateTime<Utc>,

    pub synced_role_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct SteamLfgPost {
    pub id: Uuid,

    pub guild_id: String,

    pub author_id: String,

    pub game_id: String,

    pub game_name: String,

    pub description: String,

    pub max_players: i32,

    pub current_players: Value,

    pub expires_at: DateTime<Utc>,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct GitHubDevopsMapping {
    pub id: Uuid,

    pub guild_id: String,

    pub repo_full_name: String,

    pub thread_channel_id: String,

    pub issue_or_pr_number: i32,

    pub discord_thread_id: String,

    pub last_status: String,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct TwitchStreamSubscription {
    pub id: Uuid,

    pub guild_id: String,

    pub streamer_login: String,

    pub streamer_id: String,

    pub discord_channel_id: String,

    pub live_role_id: Option<String>,

    pub last_message_id: Option<String>,

    pub is_live: bool,

    pub last_title: Option<String>,

    pub last_game: Option<String>,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct KickStreamSubscription {
    pub id: Uuid,

    pub guild_id: String,

    pub slug: String,

    pub channel_id: String,

    pub discord_channel_id: String,

    pub live_role_id: Option<String>,

    pub last_message_id: Option<String>,

    pub is_live: bool,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct YouTubeChannelSubscription {
    pub id: Uuid,

    pub guild_id: String,

    pub channel_id: String,

    pub channel_title: String,

    pub discord_channel_id: String,

    pub last_video_id: Option<String>,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct TelegramBridge {
    pub id: Uuid,

    pub guild_id: String,

    pub telegram_chat_id: String,

    pub discord_channel_id: String,

    pub secret_token: String,

    pub paused: bool,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct TelegramMessageMapping {
    pub id: Uuid,

    pub bridge_id: Uuid,

    pub telegram_message_id: String,

    pub discord_message_id: String,

    pub direction: String,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct WorkflowDefinition {
    pub id: Uuid,

    pub guild_id: String,

    pub name: String,

    pub dag_nodes: Value,

    pub dag_edges: Value,

    pub is_active: bool,

    pub version: i32,

    pub created_at: DateTime<Utc>,

    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct WorkflowRunExecution {
    pub id: Uuid,

    pub workflow_id: Uuid,

    pub guild_id: String,

    pub trigger_event: String,

    pub status: String,

    pub logs: Value,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct GitLabSubscription {
    pub id: Uuid,

    pub guild_id: String,

    pub host: String,

    pub project_id: String,

    pub project_path: String,

    pub secret_token: String,

    pub discord_channel_id: String,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct VkSubscription {
    pub id: Uuid,

    pub guild_id: String,

    pub group_id: String,

    pub confirmation_code: String,

    pub secret_key: String,

    pub discord_channel_id: String,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct RedditSubscription {
    pub id: Uuid,

    pub guild_id: String,

    pub subreddit: String,

    pub discord_channel_id: String,

    pub filter_flair: Option<String>,

    pub min_score: i32,

    pub last_seen_fullname: Option<String>,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct GamingHubGameStatus {
    pub id: Uuid,

    pub game_id: String,

    pub region: String,

    pub server_name: String,

    pub status: String,

    pub player_count: i32,

    pub last_checked_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct SocialPublicationDraft {
    pub id: Uuid,

    pub guild_id: String,

    pub author_id: String,

    pub content: String,

    pub media_urls: Value,

    pub targets: Value,

    pub scheduled_at: Option<DateTime<Utc>>,

    pub approved_by: Option<String>,

    pub status: String,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct AiGuildBudget {
    pub guild_id: String,

    pub monthly_token_limit: i64,

    pub used_tokens: i64,

    pub monthly_usd_limit: f64,

    pub used_usd: f64,

    pub is_halted: bool,

    pub reset_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct AiKnowledgeDocument {
    pub id: Uuid,

    pub guild_id: String,

    pub channel_acl: Value,

    pub title: String,

    pub content: String,

    pub source_type: String,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct McpToolAuthorization {
    pub id: Uuid,

    pub guild_id: String,

    pub tool_name: String,

    pub allowed_roles: Value,

    pub requires_approval: bool,

    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct PluginMarketplacePackage {
    pub id: Uuid,

    pub plugin_id: String,

    pub name: String,

    pub version: String,

    pub author: String,

    pub sha256_hash: String,

    pub manifest: Value,

    pub is_verified: bool,

    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct PluginGuildInstallation {
    pub id: Uuid,

    pub guild_id: String,

    pub plugin_id: String,

    pub version: String,

    pub enabled: bool,

    pub config: Value,

    pub installed_at: DateTime<Utc>,
}

impl Storage {
    pub async fn connect(database_url: Option<&str>) -> anyhow::Result<Self> {
        match database_url {
            Some(url) => Ok(Self::Postgres(PostgresStore::connect(url).await?)),

            None => {
                tracing::warn!(
                    "DATABASE_URL is not set; using volatile in-memory configuration storage"
                );

                Ok(Self::Memory(MemoryStore::default()))
            }
        }
    }

    #[must_use]
    pub fn postgres(pool: sqlx::PgPool) -> Self {
        Self::Postgres(PostgresStore::new(pool))
    }

    #[must_use]
    pub fn memory() -> Self {
        Self::Memory(MemoryStore::default())
    }

    pub async fn get_module_settings(
        &self,

        guild_id: &str,

        module_id: &str,
    ) -> anyhow::Result<Option<GuildModuleSettings>> {
        match self {
            Self::Memory(store) => Ok(store.get_module_settings(guild_id, module_id)),

            Self::Postgres(store) => store.get_module_settings(guild_id, module_id).await,
        }
    }

    pub async fn ping(&self) -> anyhow::Result<()> {
        match self {
            Self::Memory(_) => Ok(()),

            Self::Postgres(store) => store.ping().await,
        }
    }

    pub async fn set_module_settings(
        &self,

        guild_id: &str,

        module_id: &str,

        enabled: bool,

        config: Value,
    ) -> anyhow::Result<GuildModuleSettings> {
        match self {
            Self::Memory(store) => {
                Ok(store.set_module_settings(guild_id, module_id, enabled, config))
            }

            Self::Postgres(store) => {
                store
                    .set_module_settings(guild_id, module_id, enabled, config)
                    .await
            }
        }
    }

    pub async fn update_module_settings_with_audit(
        &self,

        params: UpdateModuleSettingsParams<'_>,
    ) -> anyhow::Result<GuildModuleSettings> {
        match self {
            Self::Memory(store) => store.update_module_settings_with_audit(params),

            Self::Postgres(store) => store.update_module_settings_with_audit(params).await,
        }
    }

    pub fn inject_audit_failure(&self, fail: bool) {
        if let Self::Memory(store) = self {
            store.inject_audit_failure(fail);
        }
    }

    pub async fn record_audit(
        &self,

        guild_id: Option<&str>,

        actor_id: Option<&str>,

        module_id: &str,

        event: &str,

        data: Value,
    ) -> anyhow::Result<()> {
        match self {
            Self::Memory(store) => store.record_audit(guild_id, actor_id, module_id, event, data),

            Self::Postgres(store) => {
                store
                    .record_audit(guild_id, actor_id, module_id, event, data)
                    .await
            }
        }
    }

    pub async fn list_audit_events(
        &self,

        guild_id: &str,

        limit: i64,
    ) -> anyhow::Result<Vec<AuditLogEntry>> {
        match self {
            Self::Memory(store) => Ok(store.list_audit_events(guild_id, limit)),

            Self::Postgres(store) => store.list_audit_events(guild_id, limit).await,
        }
    }

    pub async fn get_member_progress(
        &self,

        guild_id: &str,

        user_id: &str,
    ) -> anyhow::Result<MemberProgress> {
        match self {
            Self::Memory(store) => {
                if store.is_progression_failure_injected() {
                    anyhow::bail!("simulated progression failure");
                }
                Ok(store.get_member_progress(guild_id, user_id))
            }

            Self::Postgres(store) => store.get_member_progress(guild_id, user_id).await,
        }
    }

    pub async fn has_member_progress(&self, guild_id: &str, user_id: &str) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => store.has_member_progress(guild_id, user_id),
            Self::Postgres(store) => store.has_member_progress(guild_id, user_id).await,
        }
    }

    pub fn inject_progression_failure(&self, fail: bool) {
        if let Self::Memory(store) = self {
            store.inject_progression_failure(fail);
        }
    }

    pub async fn erase_guild_user_data(
        &self,
        guild_id: &str,
        user_id: &str,
    ) -> anyhow::Result<UserErasureReport> {
        match self {
            Self::Memory(store) => Ok(store.erase_guild_user_data(guild_id, user_id)),
            Self::Postgres(store) => store.erase_guild_user_data(guild_id, user_id).await,
        }
    }

    pub async fn award_message_xp(
        &self,

        guild_id: &str,

        user_id: &str,

        amount: i64,

        cooldown_seconds: u32,
    ) -> anyhow::Result<XpAward> {
        match self {
            Self::Memory(store) => {
                Ok(store.award_message_xp(guild_id, user_id, amount, cooldown_seconds))
            }

            Self::Postgres(store) => {
                store
                    .award_message_xp(guild_id, user_id, amount, cooldown_seconds)
                    .await
            }
        }
    }

    pub async fn claim_daily(
        &self,

        guild_id: &str,

        user_id: &str,

        base_reward: i64,

        streak_bonus: i64,

        max_streak_bonus: i32,
    ) -> anyhow::Result<DailyClaimOutcome> {
        match self {
            Self::Memory(store) => Ok(store.claim_daily(
                guild_id,
                user_id,
                base_reward,
                streak_bonus,
                max_streak_bonus,
            )),

            Self::Postgres(store) => {
                store
                    .claim_daily(
                        guild_id,
                        user_id,
                        base_reward,
                        streak_bonus,
                        max_streak_bonus,
                    )
                    .await
            }
        }
    }

    pub async fn transfer_coins(
        &self,

        guild_id: &str,

        sender_user_id: &str,

        recipient_user_id: &str,

        amount: i64,
    ) -> anyhow::Result<CoinTransferOutcome> {
        match self {
            Self::Memory(store) => {
                store.transfer_coins(guild_id, sender_user_id, recipient_user_id, amount)
            }

            Self::Postgres(store) => {
                store
                    .transfer_coins(guild_id, sender_user_id, recipient_user_id, amount)
                    .await
            }
        }
    }

    pub async fn give_reputation(
        &self,

        guild_id: &str,

        giver_user_id: &str,

        target_user_id: &str,

        amount: i64,

        cooldown_seconds: u32,
    ) -> anyhow::Result<ReputationGrantOutcome> {
        match self {
            Self::Memory(store) => Ok(store.give_reputation(
                guild_id,
                giver_user_id,
                target_user_id,
                amount,
                cooldown_seconds,
            )),

            Self::Postgres(store) => {
                store
                    .give_reputation(
                        guild_id,
                        giver_user_id,
                        target_user_id,
                        amount,
                        cooldown_seconds,
                    )
                    .await
            }
        }
    }

    pub async fn adjust_member_progress(
        &self,

        guild_id: &str,

        user_id: &str,

        xp_delta: i64,

        coins_delta: i64,

        reputation_delta: i64,
    ) -> anyhow::Result<MemberProgress> {
        match self {
            Self::Memory(store) => Ok(store.adjust_member_progress(
                guild_id,
                user_id,
                xp_delta,
                coins_delta,
                reputation_delta,
            )),

            Self::Postgres(store) => {
                store
                    .adjust_member_progress(
                        guild_id,
                        user_id,
                        xp_delta,
                        coins_delta,
                        reputation_delta,
                    )
                    .await
            }
        }
    }

    pub async fn adjust_member_progress_with_audit(
        &self,

        params: AdjustMemberProgressParams<'_>,
    ) -> anyhow::Result<MemberProgress> {
        match self {
            Self::Memory(store) => store.adjust_member_progress_with_audit(params),

            Self::Postgres(store) => store.adjust_member_progress_with_audit(params).await,
        }
    }

    pub async fn progression_leaderboard(
        &self,

        guild_id: &str,

        metric: ProgressMetric,

        limit: u8,
    ) -> anyhow::Result<Vec<MemberProgress>> {
        match self {
            Self::Memory(store) => Ok(store.progression_leaderboard(guild_id, metric, limit)),

            Self::Postgres(store) => store.progression_leaderboard(guild_id, metric, limit).await,
        }
    }

    pub async fn is_role_reward_granted(
        &self,

        guild_id: &str,

        user_id: &str,

        role_id: &str,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => Ok(store.is_role_reward_granted(guild_id, user_id, role_id)),

            Self::Postgres(store) => {
                store
                    .is_role_reward_granted(guild_id, user_id, role_id)
                    .await
            }
        }
    }

    pub async fn record_role_reward_granted(
        &self,

        guild_id: &str,

        user_id: &str,

        role_id: &str,

        level: i64,
    ) -> anyhow::Result<()> {
        match self {
            Self::Memory(store) => {
                store.record_role_reward_granted(guild_id, user_id, role_id, level);

                Ok(())
            }

            Self::Postgres(store) => {
                store
                    .record_role_reward_granted(guild_id, user_id, role_id, level)
                    .await
            }
        }
    }

    pub async fn record_moderation_intent(
        &self,

        intent: &ModerationExecutionIntent,
    ) -> anyhow::Result<()> {
        match self {
            Self::Memory(store) => {
                store.record_moderation_intent(intent.clone());

                Ok(())
            }

            Self::Postgres(store) => store.record_moderation_intent(intent).await,
        }
    }

    pub async fn update_moderation_intent_step(
        &self,

        id: Uuid,

        step: &str,

        error: Option<&str>,
    ) -> anyhow::Result<()> {
        match self {
            Self::Memory(store) => {
                store.update_moderation_intent_step(id, step, error);

                Ok(())
            }

            Self::Postgres(store) => store.update_moderation_intent_step(id, step, error).await,
        }
    }

    pub async fn list_pending_moderation_intents(
        &self,

        kind: &str,

        step: &str,
    ) -> anyhow::Result<Vec<ModerationExecutionIntent>> {
        match self {
            Self::Memory(store) => Ok(store.list_pending_moderation_intents(kind, step)),

            Self::Postgres(store) => store.list_pending_moderation_intents(kind, step).await,
        }
    }

    pub async fn get_module_draft(
        &self,

        guild_id: &str,

        module_id: &str,
    ) -> anyhow::Result<Option<ModuleDraftConfig>> {
        match self {
            Self::Memory(store) => Ok(store.get_module_draft(guild_id, module_id)),

            Self::Postgres(store) => store.get_module_draft(guild_id, module_id).await,
        }
    }

    pub async fn set_module_draft(
        &self,

        guild_id: &str,

        module_id: &str,

        config: Value,

        updated_by: &str,
    ) -> anyhow::Result<ModuleDraftConfig> {
        match self {
            Self::Memory(store) => {
                Ok(store.set_module_draft(guild_id, module_id, config, updated_by))
            }

            Self::Postgres(store) => {
                store
                    .set_module_draft(guild_id, module_id, config, updated_by)
                    .await
            }
        }
    }

    pub async fn delete_module_draft(
        &self,

        guild_id: &str,

        module_id: &str,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => Ok(store.delete_module_draft(guild_id, module_id)),

            Self::Postgres(store) => store.delete_module_draft(guild_id, module_id).await,
        }
    }

    pub async fn create_appeal(&self, appeal: &ModerationAppeal) -> anyhow::Result<()> {
        match self {
            Self::Memory(store) => {
                store.create_appeal(appeal.clone());

                Ok(())
            }

            Self::Postgres(store) => store.create_appeal(appeal).await,
        }
    }

    pub async fn list_appeals(
        &self,

        guild_id: &str,

        status: Option<&str>,
    ) -> anyhow::Result<Vec<ModerationAppeal>> {
        match self {
            Self::Memory(store) => Ok(store.list_appeals(guild_id, status)),

            Self::Postgres(store) => store.list_appeals(guild_id, status).await,
        }
    }

    pub async fn get_appeal(&self, appeal_id: Uuid) -> anyhow::Result<Option<ModerationAppeal>> {
        match self {
            Self::Memory(store) => Ok(store.get_appeal(appeal_id)),

            Self::Postgres(store) => store.get_appeal(appeal_id).await,
        }
    }

    pub async fn update_appeal_status(
        &self,
        guild_id: &str,
        appeal_id: Uuid,
        status: &str,
        staff_notes: Option<&str>,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => {
                Ok(store.update_appeal_status(guild_id, appeal_id, status, staff_notes))
            }

            Self::Postgres(store) => {
                store
                    .update_appeal_status(guild_id, appeal_id, status, staff_notes)
                    .await
            }
        }
    }

    pub async fn grant_timed_role(
        &self,

        guild_id: &str,

        user_id: &str,

        role_id: &str,

        expires_at: DateTime<Utc>,
    ) -> anyhow::Result<Uuid> {
        match self {
            Self::Memory(store) => {
                Ok(store.grant_timed_role(guild_id, user_id, role_id, expires_at))
            }

            Self::Postgres(store) => {
                store
                    .grant_timed_role(guild_id, user_id, role_id, expires_at)
                    .await
            }
        }
    }

    pub async fn get_active_timed_roles(
        &self,

        guild_id: &str,

        user_id: &str,
    ) -> anyhow::Result<Vec<TimedRoleGrant>> {
        match self {
            Self::Memory(store) => Ok(store.get_active_timed_roles(guild_id, user_id)),

            Self::Postgres(store) => store.get_active_timed_roles(guild_id, user_id).await,
        }
    }

    pub async fn save_persistent_roles(
        &self,

        guild_id: &str,

        user_id: &str,

        role_ids: &[String],
    ) -> anyhow::Result<()> {
        match self {
            Self::Memory(store) => {
                store.save_persistent_roles(guild_id, user_id, role_ids);

                Ok(())
            }

            Self::Postgres(store) => {
                store
                    .save_persistent_roles(guild_id, user_id, role_ids)
                    .await
            }
        }
    }

    pub async fn get_persistent_roles(
        &self,

        guild_id: &str,

        user_id: &str,
    ) -> anyhow::Result<Vec<String>> {
        match self {
            Self::Memory(store) => Ok(store.get_persistent_roles(guild_id, user_id)),

            Self::Postgres(store) => store.get_persistent_roles(guild_id, user_id).await,
        }
    }

    // Phase 2 & 3 storage methods

    pub async fn create_server_snapshot(
        &self,

        guild_id: &str,

        name: &str,

        snapshot_json: Value,

        created_by: &str,
    ) -> anyhow::Result<ServerSnapshot> {
        match self {
            Self::Memory(store) => {
                Ok(store.create_server_snapshot(guild_id, name, snapshot_json, created_by))
            }

            Self::Postgres(store) => {
                store
                    .create_server_snapshot(guild_id, name, snapshot_json, created_by)
                    .await
            }
        }
    }

    pub async fn list_server_snapshots(
        &self,

        guild_id: &str,
    ) -> anyhow::Result<Vec<ServerSnapshot>> {
        match self {
            Self::Memory(store) => Ok(store.list_server_snapshots(guild_id)),

            Self::Postgres(store) => store.list_server_snapshots(guild_id).await,
        }
    }

    pub async fn get_server_snapshot(
        &self,

        snapshot_id: Uuid,
    ) -> anyhow::Result<Option<ServerSnapshot>> {
        match self {
            Self::Memory(store) => Ok(store.get_server_snapshot(snapshot_id)),

            Self::Postgres(store) => store.get_server_snapshot(snapshot_id).await,
        }
    }

    pub async fn delete_server_snapshot(
        &self,
        guild_id: &str,
        snapshot_id: Uuid,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => Ok(store.delete_server_snapshot(guild_id, snapshot_id)),

            Self::Postgres(store) => store.delete_server_snapshot(guild_id, snapshot_id).await,
        }
    }

    pub async fn create_custom_automation(
        &self,

        guild_id: &str,

        name: &str,

        trigger_type: &str,

        conditions_json: Value,

        actions_json: Value,
    ) -> anyhow::Result<CustomAutomation> {
        match self {
            Self::Memory(store) => Ok(store.create_custom_automation(
                guild_id,
                name,
                trigger_type,
                conditions_json,
                actions_json,
            )),

            Self::Postgres(store) => {
                store
                    .create_custom_automation(
                        guild_id,
                        name,
                        trigger_type,
                        conditions_json,
                        actions_json,
                    )
                    .await
            }
        }
    }

    pub async fn list_custom_automations(
        &self,

        guild_id: &str,
    ) -> anyhow::Result<Vec<CustomAutomation>> {
        match self {
            Self::Memory(store) => Ok(store.list_custom_automations(guild_id)),

            Self::Postgres(store) => store.list_custom_automations(guild_id).await,
        }
    }

    pub async fn delete_custom_automation(
        &self,
        guild_id: &str,
        automation_id: Uuid,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => Ok(store.delete_custom_automation(guild_id, automation_id)),

            Self::Postgres(store) => {
                store
                    .delete_custom_automation(guild_id, automation_id)
                    .await
            }
        }
    }

    pub async fn get_sticky_message(
        &self,

        guild_id: &str,

        channel_id: &str,
    ) -> anyhow::Result<Option<StickyMessage>> {
        match self {
            Self::Memory(store) => Ok(store.get_sticky_message(guild_id, channel_id)),

            Self::Postgres(store) => store.get_sticky_message(guild_id, channel_id).await,
        }
    }

    pub async fn set_sticky_message(
        &self,

        guild_id: &str,

        channel_id: &str,

        message_id: &str,

        content: &str,

        cooldown_seconds: i32,
    ) -> anyhow::Result<StickyMessage> {
        match self {
            Self::Memory(store) => Ok(store.set_sticky_message(
                guild_id,
                channel_id,
                message_id,
                content,
                cooldown_seconds,
            )),

            Self::Postgres(store) => {
                store
                    .set_sticky_message(guild_id, channel_id, message_id, content, cooldown_seconds)
                    .await
            }
        }
    }

    pub async fn delete_sticky_message(
        &self,

        guild_id: &str,

        channel_id: &str,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => Ok(store.delete_sticky_message(guild_id, channel_id)),

            Self::Postgres(store) => store.delete_sticky_message(guild_id, channel_id).await,
        }
    }

    pub async fn create_community_poll(
        &self,

        guild_id: &str,

        channel_id: &str,

        question: &str,

        options: &[String],

        is_anonymous: bool,

        expires_at: DateTime<Utc>,
    ) -> anyhow::Result<CommunityPoll> {
        match self {
            Self::Memory(store) => Ok(store.create_community_poll(
                guild_id,
                channel_id,
                question,
                options,
                is_anonymous,
                expires_at,
            )),

            Self::Postgres(store) => {
                store
                    .create_community_poll(
                        guild_id,
                        channel_id,
                        question,
                        options,
                        is_anonymous,
                        expires_at,
                    )
                    .await
            }
        }
    }

    pub async fn get_community_poll(&self, poll_id: Uuid) -> anyhow::Result<Option<CommunityPoll>> {
        match self {
            Self::Memory(store) => Ok(store.get_community_poll(poll_id)),

            Self::Postgres(store) => store.get_community_poll(poll_id).await,
        }
    }

    pub async fn vote_community_poll(
        &self,

        guild_id: &str,

        poll_id: Uuid,

        user_id: &str,

        option_idx: usize,
    ) -> anyhow::Result<Option<CommunityPoll>> {
        match self {
            Self::Memory(store) => {
                store.vote_community_poll(guild_id, poll_id, user_id, option_idx)
            }

            Self::Postgres(store) => {
                store
                    .vote_community_poll(guild_id, poll_id, user_id, option_idx)
                    .await
            }
        }
    }

    pub async fn create_giveaway(
        &self,

        guild_id: &str,

        channel_id: &str,

        prize: &str,

        winners_count: i32,

        expires_at: DateTime<Utc>,
    ) -> anyhow::Result<Giveaway> {
        match self {
            Self::Memory(store) => {
                Ok(store.create_giveaway(guild_id, channel_id, prize, winners_count, expires_at))
            }

            Self::Postgres(store) => {
                store
                    .create_giveaway(guild_id, channel_id, prize, winners_count, expires_at)
                    .await
            }
        }
    }

    pub async fn get_giveaway(&self, giveaway_id: Uuid) -> anyhow::Result<Option<Giveaway>> {
        match self {
            Self::Memory(store) => Ok(store.get_giveaway(giveaway_id)),

            Self::Postgres(store) => store.get_giveaway(giveaway_id).await,
        }
    }

    pub async fn join_giveaway(&self, giveaway_id: Uuid, user_id: &str) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => Ok(store.join_giveaway(giveaway_id, user_id)),

            Self::Postgres(store) => store.join_giveaway(giveaway_id, user_id).await,
        }
    }

    pub async fn reroll_giveaway(&self, giveaway_id: Uuid) -> anyhow::Result<Vec<String>> {
        match self {
            Self::Memory(store) => Ok(store.reroll_giveaway(giveaway_id)),

            Self::Postgres(store) => store.reroll_giveaway(giveaway_id).await,
        }
    }

    pub async fn record_starboard_entry(
        &self,

        guild_id: &str,

        source_channel_id: &str,

        source_message_id: &str,

        starboard_message_id: &str,

        reaction_count: i32,
    ) -> anyhow::Result<StarboardEntry> {
        match self {
            Self::Memory(store) => Ok(store.record_starboard_entry(
                guild_id,
                source_channel_id,
                source_message_id,
                starboard_message_id,
                reaction_count,
            )),

            Self::Postgres(store) => {
                store
                    .record_starboard_entry(
                        guild_id,
                        source_channel_id,
                        source_message_id,
                        starboard_message_id,
                        reaction_count,
                    )
                    .await
            }
        }
    }

    pub async fn get_starboard_entry(
        &self,

        guild_id: &str,

        source_message_id: &str,
    ) -> anyhow::Result<Option<StarboardEntry>> {
        match self {
            Self::Memory(store) => Ok(store.get_starboard_entry(guild_id, source_message_id)),

            Self::Postgres(store) => store.get_starboard_entry(guild_id, source_message_id).await,
        }
    }

    pub async fn list_store_items(&self, guild_id: &str) -> anyhow::Result<Vec<StoreItem>> {
        match self {
            Self::Memory(store) => Ok(store.list_store_items(guild_id)),

            Self::Postgres(store) => store.list_store_items(guild_id).await,
        }
    }

    pub async fn create_store_item(
        &self,

        guild_id: &str,

        name: &str,

        description: &str,

        price: i64,

        role_id: Option<&str>,

        stock: Option<i32>,
    ) -> anyhow::Result<StoreItem> {
        anyhow::ensure!(price >= 0, "price must be non-negative");
        anyhow::ensure!(stock.is_none_or(|n| n >= 0), "stock must be non-negative");
        if let Some(r_id) = role_id {
            validate_snowflake("role_id", r_id)?;
        }

        match self {
            Self::Memory(store) => {
                Ok(store.create_store_item(guild_id, name, description, price, role_id, stock))
            }

            Self::Postgres(store) => {
                store
                    .create_store_item(guild_id, name, description, price, role_id, stock)
                    .await
            }
        }
    }

    pub async fn purchase_store_item(
        &self,

        guild_id: &str,

        user_id: &str,

        item_id: Uuid,
    ) -> anyhow::Result<(StoreItem, StorePurchase, i64)> {
        match self {
            Self::Memory(store) => store.purchase_store_item(guild_id, user_id, item_id),

            Self::Postgres(store) => store.purchase_store_item(guild_id, user_id, item_id).await,
        }
    }

    pub async fn get_store_purchase(
        &self,
        purchase_id: Uuid,
    ) -> anyhow::Result<Option<StorePurchase>> {
        match self {
            Self::Memory(store) => Ok(store.get_store_purchase(purchase_id)),

            Self::Postgres(store) => store.get_store_purchase(purchase_id).await,
        }
    }

    pub async fn list_store_purchases(
        &self,
        guild_id: &str,
        user_id: Option<&str>,
    ) -> anyhow::Result<Vec<StorePurchase>> {
        match self {
            Self::Memory(store) => Ok(store.list_store_purchases(guild_id, user_id)),

            Self::Postgres(store) => store.list_store_purchases(guild_id, user_id).await,
        }
    }

    pub async fn update_store_purchase_fulfillment(
        &self,
        purchase_id: Uuid,
        status: &str,
        failure_reason: Option<&str>,
        fulfilled_at: Option<DateTime<Utc>>,
    ) -> anyhow::Result<()> {
        match self {
            Self::Memory(store) => store.update_store_purchase_fulfillment(
                purchase_id,
                status,
                failure_reason,
                fulfilled_at,
            ),

            Self::Postgres(store) => {
                store
                    .update_store_purchase_fulfillment(
                        purchase_id,
                        status,
                        failure_reason,
                        fulfilled_at,
                    )
                    .await
            }
        }
    }

    pub async fn record_member_identity_change(
        &self,
        guild_id: &str,
        user_id: &str,
        kind: &str,
        old_value: Option<&str>,
        new_value: Option<&str>,
        retention_days: i64,
    ) -> anyhow::Result<Option<MemberIdentityRecord>> {
        match self {
            Self::Memory(store) => store.record_member_identity_change(
                guild_id,
                user_id,
                kind,
                old_value,
                new_value,
                retention_days,
            ),
            Self::Postgres(store) => {
                store
                    .record_member_identity_change(
                        guild_id,
                        user_id,
                        kind,
                        old_value,
                        new_value,
                        retention_days,
                    )
                    .await
            }
        }
    }

    pub async fn get_member_identity_history(
        &self,
        guild_id: &str,
        user_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<MemberIdentityRecord>> {
        match self {
            Self::Memory(store) => store.get_member_identity_history(guild_id, user_id, limit),
            Self::Postgres(store) => {
                store
                    .get_member_identity_history(guild_id, user_id, limit)
                    .await
            }
        }
    }

    pub async fn cleanup_expired_identity_history(&self) -> anyhow::Result<u64> {
        match self {
            Self::Memory(store) => Ok(store.cleanup_expired_identity_history()),
            Self::Postgres(store) => store.cleanup_expired_identity_history().await,
        }
    }

    pub fn insert_raw_identity_record(&self, record: MemberIdentityRecord) {
        if let Self::Memory(store) = self {
            store.insert_raw_identity_record(record);
        }
    }

    pub fn raw_identity_history_count(&self) -> Option<usize> {
        match self {
            Self::Memory(store) => Some(store.raw_identity_history_count()),
            Self::Postgres(_) => None,
        }
    }

    pub async fn list_policy_rules(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<CommandPolicyRule>> {
        match self {
            Self::Memory(store) => store.list_policy_rules(guild_id),
            Self::Postgres(store) => store.list_policy_rules(guild_id).await,
        }
    }

    pub async fn upsert_policy_rule(
        &self,
        rule: CommandPolicyRule,
    ) -> anyhow::Result<CommandPolicyRule> {
        match self {
            Self::Memory(store) => store.upsert_policy_rule(rule),
            Self::Postgres(store) => store.upsert_policy_rule(rule).await,
        }
    }

    pub async fn delete_policy_rule(&self, guild_id: &str, rule_id: Uuid) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => store.delete_policy_rule(guild_id, rule_id),
            Self::Postgres(store) => store.delete_policy_rule(guild_id, rule_id).await,
        }
    }

    pub async fn reset_policy_rules(&self, guild_id: &str) -> anyhow::Result<usize> {
        match self {
            Self::Memory(store) => store.reset_policy_rules(guild_id),
            Self::Postgres(store) => store.reset_policy_rules(guild_id).await,
        }
    }

    pub async fn replace_policy_rules(
        &self,
        guild_id: &str,
        expected_revision: Option<i64>,
        rules: Vec<CommandPolicyRule>,
    ) -> anyhow::Result<Vec<CommandPolicyRule>> {
        match self {
            Self::Memory(store) => store.replace_policy_rules(guild_id, expected_revision, rules),
            Self::Postgres(store) => {
                store
                    .replace_policy_rules(guild_id, expected_revision, rules)
                    .await
            }
        }
    }

    pub async fn get_policy_revision(&self, guild_id: &str) -> anyhow::Result<i64> {
        match self {
            Self::Memory(store) => store.get_policy_revision(guild_id),
            Self::Postgres(store) => store.get_policy_revision(guild_id).await,
        }
    }

    pub async fn bump_policy_revision(&self, guild_id: &str) -> anyhow::Result<i64> {
        match self {
            Self::Memory(store) => store.bump_policy_revision(guild_id),
            Self::Postgres(store) => store.bump_policy_revision(guild_id).await,
        }
    }

    pub async fn get_duty_settings(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Option<ModeratorDutySettings>> {
        match self {
            Self::Memory(store) => store.get_duty_settings(guild_id),
            Self::Postgres(store) => store.get_duty_settings(guild_id).await,
        }
    }

    pub async fn upsert_duty_settings(
        &self,
        settings: ModeratorDutySettings,
    ) -> anyhow::Result<ModeratorDutySettings> {
        match self {
            Self::Memory(store) => store.upsert_duty_settings(settings),
            Self::Postgres(store) => store.upsert_duty_settings(settings).await,
        }
    }

    pub async fn get_duty_profile(
        &self,
        guild_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Option<ModeratorDutyProfile>> {
        match self {
            Self::Memory(store) => store.get_duty_profile(guild_id, user_id),
            Self::Postgres(store) => store.get_duty_profile(guild_id, user_id).await,
        }
    }

    pub async fn list_duty_profiles(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<ModeratorDutyProfile>> {
        match self {
            Self::Memory(store) => store.list_duty_profiles(guild_id),
            Self::Postgres(store) => store.list_duty_profiles(guild_id).await,
        }
    }

    pub async fn list_guilds_with_duty_profiles(&self) -> anyhow::Result<Vec<String>> {
        match self {
            Self::Memory(store) => store.list_guilds_with_duty_profiles(),
            Self::Postgres(store) => store.list_guilds_with_duty_profiles().await,
        }
    }

    pub async fn upsert_duty_profile(
        &self,
        profile: ModeratorDutyProfile,
    ) -> anyhow::Result<ModeratorDutyProfile> {
        match self {
            Self::Memory(store) => store.upsert_duty_profile(profile),
            Self::Postgres(store) => store.upsert_duty_profile(profile).await,
        }
    }

    pub async fn delete_duty_profile(&self, guild_id: &str, user_id: &str) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => store.delete_duty_profile(guild_id, user_id),
            Self::Postgres(store) => store.delete_duty_profile(guild_id, user_id).await,
        }
    }

    pub async fn get_duty_state(
        &self,
        guild_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Option<ModeratorDutyState>> {
        match self {
            Self::Memory(store) => store.get_duty_state(guild_id, user_id),
            Self::Postgres(store) => store.get_duty_state(guild_id, user_id).await,
        }
    }

    pub async fn list_duty_states(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<ModeratorDutyState>> {
        match self {
            Self::Memory(store) => store.list_duty_states(guild_id),
            Self::Postgres(store) => store.list_duty_states(guild_id).await,
        }
    }

    pub async fn upsert_duty_state(
        &self,
        state: ModeratorDutyState,
    ) -> anyhow::Result<ModeratorDutyState> {
        match self {
            Self::Memory(store) => store.upsert_duty_state(state),
            Self::Postgres(store) => store.upsert_duty_state(state).await,
        }
    }

    pub async fn get_duty_role_grant(
        &self,
        guild_id: &str,
        user_id: &str,
        role_id: &str,
    ) -> anyhow::Result<Option<ModeratorDutyRoleGrant>> {
        match self {
            Self::Memory(store) => store.get_duty_role_grant(guild_id, user_id, role_id),
            Self::Postgres(store) => store.get_duty_role_grant(guild_id, user_id, role_id).await,
        }
    }

    pub async fn upsert_duty_role_grant(
        &self,
        grant: ModeratorDutyRoleGrant,
    ) -> anyhow::Result<ModeratorDutyRoleGrant> {
        match self {
            Self::Memory(store) => store.upsert_duty_role_grant(grant),
            Self::Postgres(store) => store.upsert_duty_role_grant(grant).await,
        }
    }

    pub async fn record_duty_role_grant(
        &self,
        guild_id: &str,
        user_id: &str,
        role_id: &str,
        status: &str,
    ) -> anyhow::Result<()> {
        match self {
            Self::Memory(store) => store.record_duty_role_grant(guild_id, user_id, role_id, status),
            Self::Postgres(store) => {
                store
                    .record_duty_role_grant(guild_id, user_id, role_id, status)
                    .await
            }
        }
    }

    pub async fn mark_duty_role_grants_for_revocation(
        &self,
        guild_id: &str,
        user_id: Option<&str>,
        role_id: Option<&str>,
    ) -> anyhow::Result<Vec<ModeratorDutyRoleGrant>> {
        match self {
            Self::Memory(store) => {
                store.mark_duty_role_grants_for_revocation(guild_id, user_id, role_id)
            }
            Self::Postgres(store) => {
                store
                    .mark_duty_role_grants_for_revocation(guild_id, user_id, role_id)
                    .await
            }
        }
    }

    pub async fn list_pending_duty_role_revocations(
        &self,
        guild_id: Option<&str>,
    ) -> anyhow::Result<Vec<ModeratorDutyRoleGrant>> {
        match self {
            Self::Memory(store) => store.list_pending_duty_role_revocations(guild_id),
            Self::Postgres(store) => store.list_pending_duty_role_revocations(guild_id).await,
        }
    }

    pub async fn get_recruitment_track(
        &self,
        guild_id: &str,
        track_id: Uuid,
    ) -> anyhow::Result<Option<RecruitmentTrack>> {
        match self {
            Self::Memory(store) => store.get_recruitment_track(guild_id, track_id),
            Self::Postgres(store) => store.get_recruitment_track(guild_id, track_id).await,
        }
    }

    pub async fn get_recruitment_track_by_slug(
        &self,
        guild_id: &str,
        slug: &str,
    ) -> anyhow::Result<Option<RecruitmentTrack>> {
        match self {
            Self::Memory(store) => store.get_recruitment_track_by_slug(guild_id, slug),
            Self::Postgres(store) => store.get_recruitment_track_by_slug(guild_id, slug).await,
        }
    }

    pub async fn list_recruitment_tracks(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<RecruitmentTrack>> {
        match self {
            Self::Memory(store) => store.list_recruitment_tracks(guild_id),
            Self::Postgres(store) => store.list_recruitment_tracks(guild_id).await,
        }
    }

    pub async fn upsert_recruitment_track(
        &self,
        track: RecruitmentTrack,
    ) -> anyhow::Result<RecruitmentTrack> {
        match self {
            Self::Memory(store) => store.upsert_recruitment_track(track),
            Self::Postgres(store) => store.upsert_recruitment_track(track).await,
        }
    }

    pub async fn delete_recruitment_track(
        &self,
        guild_id: &str,
        track_id: Uuid,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => store.delete_recruitment_track(guild_id, track_id),
            Self::Postgres(store) => store.delete_recruitment_track(guild_id, track_id).await,
        }
    }

    pub async fn get_recruitment_application(
        &self,
        guild_id: &str,
        id: Uuid,
    ) -> anyhow::Result<Option<RecruitmentApplication>> {
        match self {
            Self::Memory(store) => store.get_recruitment_application(guild_id, id),
            Self::Postgres(store) => store.get_recruitment_application(guild_id, id).await,
        }
    }

    pub async fn list_recruitment_applications(
        &self,
        guild_id: &str,
        track_id: Option<Uuid>,
        status: Option<&str>,
        applicant_user_id: Option<&str>,
    ) -> anyhow::Result<Vec<RecruitmentApplication>> {
        match self {
            Self::Memory(store) => {
                store.list_recruitment_applications(guild_id, track_id, status, applicant_user_id)
            }
            Self::Postgres(store) => {
                store
                    .list_recruitment_applications(guild_id, track_id, status, applicant_user_id)
                    .await
            }
        }
    }

    pub async fn create_recruitment_application(
        &self,
        app: RecruitmentApplication,
    ) -> anyhow::Result<RecruitmentApplication> {
        match self {
            Self::Memory(store) => store.create_recruitment_application(app),
            Self::Postgres(store) => store.create_recruitment_application(app).await,
        }
    }

    pub async fn create_recruitment_application_checked(
        &self,
        app: RecruitmentApplication,
        max_active: u32,
    ) -> anyhow::Result<RecruitmentApplication> {
        match self {
            Self::Memory(store) => store.create_recruitment_application_checked(app, max_active),
            Self::Postgres(store) => {
                store
                    .create_recruitment_application_checked(app, max_active)
                    .await
            }
        }
    }

    pub async fn update_recruitment_application_transition(
        &self,
        params: TransitionApplicationParams<'_>,
    ) -> anyhow::Result<RecruitmentApplication> {
        match self {
            Self::Memory(store) => store.update_recruitment_application_transition(params),
            Self::Postgres(store) => {
                store
                    .update_recruitment_application_transition(params)
                    .await
            }
        }
    }

    pub async fn accept_recruitment_application_with_role_enqueue(
        &self,
        params: TransitionApplicationParams<'_>,
        target_role_id: Option<&str>,
    ) -> anyhow::Result<(RecruitmentApplication, Option<RecruitmentRoleFulfillment>)> {
        match self {
            Self::Memory(store) => {
                store.accept_recruitment_application_with_role_enqueue(params, target_role_id)
            }
            Self::Postgres(store) => {
                store
                    .accept_recruitment_application_with_role_enqueue(params, target_role_id)
                    .await
            }
        }
    }

    pub async fn cancel_pending_role_fulfillments_for_application(
        &self,
        application_id: Uuid,
    ) -> anyhow::Result<usize> {
        match self {
            Self::Memory(store) => {
                store.cancel_pending_role_fulfillments_for_application(application_id)
            }
            Self::Postgres(store) => {
                store
                    .cancel_pending_role_fulfillments_for_application(application_id)
                    .await
            }
        }
    }

    pub async fn count_active_user_applications(
        &self,
        guild_id: &str,
        user_id: &str,
        track_id: Uuid,
    ) -> anyhow::Result<u32> {
        match self {
            Self::Memory(store) => {
                store.count_active_user_applications(guild_id, user_id, track_id)
            }
            Self::Postgres(store) => {
                store
                    .count_active_user_applications(guild_id, user_id, track_id)
                    .await
            }
        }
    }

    pub async fn list_recruitment_reviews(
        &self,
        application_id: Uuid,
    ) -> anyhow::Result<Vec<RecruitmentReview>> {
        match self {
            Self::Memory(store) => store.list_recruitment_reviews(application_id),
            Self::Postgres(store) => store.list_recruitment_reviews(application_id).await,
        }
    }

    pub async fn upsert_recruitment_review(
        &self,
        review: RecruitmentReview,
    ) -> anyhow::Result<RecruitmentReview> {
        match self {
            Self::Memory(store) => store.upsert_recruitment_review(review),
            Self::Postgres(store) => store.upsert_recruitment_review(review).await,
        }
    }

    pub async fn get_role_fulfillment(
        &self,
        application_id: Uuid,
    ) -> anyhow::Result<Option<RecruitmentRoleFulfillment>> {
        match self {
            Self::Memory(store) => store.get_role_fulfillment(application_id),
            Self::Postgres(store) => store.get_role_fulfillment(application_id).await,
        }
    }

    pub async fn upsert_role_fulfillment(
        &self,
        fulfillment: RecruitmentRoleFulfillment,
    ) -> anyhow::Result<RecruitmentRoleFulfillment> {
        match self {
            Self::Memory(store) => store.upsert_role_fulfillment(fulfillment),
            Self::Postgres(store) => store.upsert_role_fulfillment(fulfillment).await,
        }
    }

    pub async fn list_pending_role_fulfillments(
        &self,
        guild_id: Option<&str>,
    ) -> anyhow::Result<Vec<RecruitmentRoleFulfillment>> {
        match self {
            Self::Memory(store) => store.list_pending_role_fulfillments(guild_id),
            Self::Postgres(store) => store.list_pending_role_fulfillments(guild_id).await,
        }
    }

    pub async fn get_help_forum_config(
        &self,
        guild_id: &str,
        forum_channel_id: &str,
    ) -> anyhow::Result<Option<HelpForumConfig>> {
        match self {
            Self::Memory(store) => store.get_help_forum_config(guild_id, forum_channel_id),
            Self::Postgres(store) => {
                store
                    .get_help_forum_config(guild_id, forum_channel_id)
                    .await
            }
        }
    }

    pub async fn list_help_forum_configs(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<HelpForumConfig>> {
        match self {
            Self::Memory(store) => store.list_help_forum_configs(guild_id),
            Self::Postgres(store) => store.list_help_forum_configs(guild_id).await,
        }
    }

    pub async fn list_enabled_help_forum_guilds(&self) -> anyhow::Result<Vec<String>> {
        match self {
            Self::Memory(store) => store.list_enabled_help_forum_guilds(),
            Self::Postgres(store) => store.list_enabled_help_forum_guilds().await,
        }
    }

    pub async fn upsert_help_forum_config(
        &self,
        config: HelpForumConfig,
    ) -> anyhow::Result<HelpForumConfig> {
        match self {
            Self::Memory(store) => store.upsert_help_forum_config(config),
            Self::Postgres(store) => store.upsert_help_forum_config(config).await,
        }
    }

    pub async fn delete_help_forum_config(
        &self,
        guild_id: &str,
        forum_channel_id: &str,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => store.delete_help_forum_config(guild_id, forum_channel_id),
            Self::Postgres(store) => {
                store
                    .delete_help_forum_config(guild_id, forum_channel_id)
                    .await
            }
        }
    }

    pub async fn get_help_forum_session(
        &self,
        guild_id: &str,
        thread_id: &str,
    ) -> anyhow::Result<Option<HelpForumSession>> {
        match self {
            Self::Memory(store) => store.get_help_forum_session(guild_id, thread_id),
            Self::Postgres(store) => store.get_help_forum_session(guild_id, thread_id).await,
        }
    }

    pub async fn list_help_forum_sessions(
        &self,
        guild_id: &str,
        forum_channel_id: Option<&str>,
        status: Option<&str>,
    ) -> anyhow::Result<Vec<HelpForumSession>> {
        match self {
            Self::Memory(store) => {
                store.list_help_forum_sessions(guild_id, forum_channel_id, status)
            }
            Self::Postgres(store) => {
                store
                    .list_help_forum_sessions(guild_id, forum_channel_id, status)
                    .await
            }
        }
    }

    pub async fn create_help_forum_session(
        &self,
        session: HelpForumSession,
    ) -> anyhow::Result<HelpForumSession> {
        match self {
            Self::Memory(store) => store.create_help_forum_session(session),
            Self::Postgres(store) => store.create_help_forum_session(session).await,
        }
    }

    pub async fn record_help_forum_activity(
        &self,
        guild_id: &str,
        thread_id: &str,
        author_user_id: &str,
        is_helper: bool,
    ) -> anyhow::Result<Option<HelpForumSession>> {
        match self {
            Self::Memory(store) => {
                store.record_help_forum_activity(guild_id, thread_id, author_user_id, is_helper)
            }
            Self::Postgres(store) => {
                store
                    .record_help_forum_activity(guild_id, thread_id, author_user_id, is_helper)
                    .await
            }
        }
    }

    pub async fn close_help_forum_session(
        &self,
        guild_id: &str,
        thread_id: &str,
        new_status: &str,
        reason: Option<&str>,
        expected_generation: Option<i64>,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => store.close_help_forum_session(
                guild_id,
                thread_id,
                new_status,
                reason,
                expected_generation,
            ),
            Self::Postgres(store) => {
                store
                    .close_help_forum_session(
                        guild_id,
                        thread_id,
                        new_status,
                        reason,
                        expected_generation,
                    )
                    .await
            }
        }
    }

    pub async fn mark_help_forum_owner_left(
        &self,
        guild_id: &str,
        owner_user_id: &str,
    ) -> anyhow::Result<usize> {
        match self {
            Self::Memory(store) => store.mark_help_forum_owner_left(guild_id, owner_user_id),
            Self::Postgres(store) => {
                store
                    .mark_help_forum_owner_left(guild_id, owner_user_id)
                    .await
            }
        }
    }

    pub async fn get_help_forum_stats(
        &self,
        guild_id: &str,
        forum_channel_id: &str,
    ) -> anyhow::Result<HelpForumStats> {
        match self {
            Self::Memory(store) => store.get_help_forum_stats(guild_id, forum_channel_id),
            Self::Postgres(store) => store.get_help_forum_stats(guild_id, forum_channel_id).await,
        }
    }

    pub async fn upsert_member_snapshot(
        &self,
        params: UpsertMemberSnapshotParams<'_>,
    ) -> anyhow::Result<MemberSnapshot> {
        match self {
            Self::Memory(store) => store.upsert_member_snapshot(params),
            Self::Postgres(store) => store.upsert_member_snapshot(params).await,
        }
    }

    pub async fn remove_member_snapshot(
        &self,
        guild_id: &str,
        user_id: &str,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => store.remove_member_snapshot(guild_id, user_id),
            Self::Postgres(store) => store.remove_member_snapshot(guild_id, user_id).await,
        }
    }

    pub async fn search_members(
        &self,
        guild_id: &str,
        filter: &MemberSearchFilter,
    ) -> anyhow::Result<Vec<MemberSearchRow>> {
        match self {
            Self::Memory(store) => store.search_members(guild_id, filter),
            Self::Postgres(store) => store.search_members(guild_id, filter).await,
        }
    }

    pub async fn create_cleanup_plan(
        &self,
        guild_id: &str,
        channel_id: &str,
        actor_user_id: &str,
        message_ids: &[String],
        ttl_seconds: i64,
    ) -> anyhow::Result<CleanupPlan> {
        match self {
            Self::Memory(store) => store.create_cleanup_plan(
                guild_id,
                channel_id,
                actor_user_id,
                message_ids,
                ttl_seconds,
            ),
            Self::Postgres(store) => {
                store
                    .create_cleanup_plan(
                        guild_id,
                        channel_id,
                        actor_user_id,
                        message_ids,
                        ttl_seconds,
                    )
                    .await
            }
        }
    }

    pub async fn get_cleanup_plan(
        &self,
        guild_id: &str,
        plan_id: Uuid,
    ) -> anyhow::Result<Option<CleanupPlan>> {
        match self {
            Self::Memory(store) => store.get_cleanup_plan(guild_id, plan_id),
            Self::Postgres(store) => store.get_cleanup_plan(guild_id, plan_id).await,
        }
    }

    pub async fn consume_cleanup_plan(
        &self,
        guild_id: &str,
        plan_id: Uuid,
        actor_user_id: &str,
    ) -> anyhow::Result<Option<CleanupPlan>> {
        match self {
            Self::Memory(store) => store.consume_cleanup_plan(guild_id, plan_id, actor_user_id),
            Self::Postgres(store) => {
                store
                    .consume_cleanup_plan(guild_id, plan_id, actor_user_id)
                    .await
            }
        }
    }

    pub async fn list_companion_mappings(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<CompanionChannelMapping>> {
        match self {
            Self::Memory(store) => store.list_companion_mappings(guild_id),
            Self::Postgres(store) => store.list_companion_mappings(guild_id).await,
        }
    }

    pub async fn upsert_companion_mapping(
        &self,
        guild_id: &str,
        voice_channel_id: &str,
        text_channel_id: &str,
        allow_send_messages: bool,
        allow_read_history: bool,
    ) -> anyhow::Result<CompanionChannelMapping> {
        match self {
            Self::Memory(store) => store.upsert_companion_mapping(
                guild_id,
                voice_channel_id,
                text_channel_id,
                allow_send_messages,
                allow_read_history,
            ),
            Self::Postgres(store) => {
                store
                    .upsert_companion_mapping(
                        guild_id,
                        voice_channel_id,
                        text_channel_id,
                        allow_send_messages,
                        allow_read_history,
                    )
                    .await
            }
        }
    }

    pub async fn batch_upsert_companion_mappings(
        &self,
        guild_id: &str,
        items: &[CompanionMappingItem],
    ) -> anyhow::Result<Vec<CompanionChannelMapping>> {
        match self {
            Self::Memory(store) => store.batch_upsert_companion_mappings(guild_id, items),
            Self::Postgres(store) => store.batch_upsert_companion_mappings(guild_id, items).await,
        }
    }

    pub async fn delete_companion_mapping(
        &self,
        guild_id: &str,
        mapping_id: Uuid,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => store.delete_companion_mapping(guild_id, mapping_id),
            Self::Postgres(store) => store.delete_companion_mapping(guild_id, mapping_id).await,
        }
    }

    pub async fn get_companion_mapping_by_voice(
        &self,
        guild_id: &str,
        voice_channel_id: &str,
    ) -> anyhow::Result<Option<CompanionChannelMapping>> {
        match self {
            Self::Memory(store) => store.get_companion_mapping_by_voice(guild_id, voice_channel_id),
            Self::Postgres(store) => {
                store
                    .get_companion_mapping_by_voice(guild_id, voice_channel_id)
                    .await
            }
        }
    }

    pub async fn record_companion_lease(
        &self,
        params: RecordCompanionLeaseParams<'_>,
    ) -> anyhow::Result<CompanionChannelLease> {
        match self {
            Self::Memory(store) => store.record_companion_lease(params),
            Self::Postgres(store) => store.record_companion_lease(params).await,
        }
    }

    pub async fn mark_companion_lease_revoke_pending(
        &self,
        guild_id: &str,
        text_channel_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Option<CompanionChannelLease>> {
        match self {
            Self::Memory(store) => {
                store.mark_companion_lease_revoke_pending(guild_id, text_channel_id, user_id)
            }
            Self::Postgres(store) => {
                store
                    .mark_companion_lease_revoke_pending(guild_id, text_channel_id, user_id)
                    .await
            }
        }
    }

    pub async fn record_companion_lease_failure(
        &self,
        guild_id: &str,
        text_channel_id: &str,
        user_id: &str,
        error: &str,
    ) -> anyhow::Result<()> {
        match self {
            Self::Memory(store) => {
                store.record_companion_lease_failure(guild_id, text_channel_id, user_id, error)
            }
            Self::Postgres(store) => {
                store
                    .record_companion_lease_failure(guild_id, text_channel_id, user_id, error)
                    .await
            }
        }
    }

    pub async fn get_companion_lease(
        &self,
        guild_id: &str,
        text_channel_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Option<CompanionChannelLease>> {
        match self {
            Self::Memory(store) => store.get_companion_lease(guild_id, text_channel_id, user_id),
            Self::Postgres(store) => {
                store
                    .get_companion_lease(guild_id, text_channel_id, user_id)
                    .await
            }
        }
    }

    pub async fn delete_companion_lease(
        &self,
        guild_id: &str,
        text_channel_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Option<CompanionChannelLease>> {
        match self {
            Self::Memory(store) => store.delete_companion_lease(guild_id, text_channel_id, user_id),
            Self::Postgres(store) => {
                store
                    .delete_companion_lease(guild_id, text_channel_id, user_id)
                    .await
            }
        }
    }

    pub async fn list_active_companion_leases(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<CompanionChannelLease>> {
        match self {
            Self::Memory(store) => store.list_active_companion_leases(guild_id),
            Self::Postgres(store) => store.list_active_companion_leases(guild_id).await,
        }
    }

    pub async fn list_pending_revoke_companion_leases(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<CompanionChannelLease>> {
        match self {
            Self::Memory(store) => store.list_pending_revoke_companion_leases(guild_id),
            Self::Postgres(store) => store.list_pending_revoke_companion_leases(guild_id).await,
        }
    }

    pub async fn archive_progression_season(
        &self,

        guild_id: &str,

        name: &str,
    ) -> anyhow::Result<ProgressionSeason> {
        match self {
            Self::Memory(store) => Ok(store.archive_progression_season(guild_id, name)),

            Self::Postgres(store) => store.archive_progression_season(guild_id, name).await,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_scheduled_event(
        &self,

        guild_id: &str,

        title: &str,

        description: &str,

        start_time: DateTime<Utc>,

        end_time: Option<DateTime<Utc>>,

        timezone: &str,

        max_participants: Option<i32>,
    ) -> anyhow::Result<ScheduledEvent> {
        match self {
            Self::Memory(store) => Ok(store.create_scheduled_event(
                guild_id,
                title,
                description,
                start_time,
                end_time,
                timezone,
                max_participants,
            )),

            Self::Postgres(store) => {
                store
                    .create_scheduled_event(
                        guild_id,
                        title,
                        description,
                        start_time,
                        end_time,
                        timezone,
                        max_participants,
                    )
                    .await
            }
        }
    }

    pub async fn list_scheduled_events(
        &self,

        guild_id: &str,
    ) -> anyhow::Result<Vec<ScheduledEvent>> {
        match self {
            Self::Memory(store) => Ok(store.list_scheduled_events(guild_id)),

            Self::Postgres(store) => store.list_scheduled_events(guild_id).await,
        }
    }

    pub async fn get_scheduled_event(
        &self,

        event_id: Uuid,
    ) -> anyhow::Result<Option<ScheduledEvent>> {
        match self {
            Self::Memory(store) => Ok(store.get_scheduled_event(event_id)),

            Self::Postgres(store) => store.get_scheduled_event(event_id).await,
        }
    }

    pub async fn join_event_roster(
        &self,

        event_id: Uuid,

        user_id: &str,
    ) -> anyhow::Result<EventRoster> {
        match self {
            Self::Memory(store) => store.join_event_roster(event_id, user_id),

            Self::Postgres(store) => store.join_event_roster(event_id, user_id).await,
        }
    }

    pub async fn leave_event_roster(
        &self,

        event_id: Uuid,

        user_id: &str,
    ) -> anyhow::Result<Option<String>> {
        match self {
            Self::Memory(store) => Ok(store.leave_event_roster(event_id, user_id)),

            Self::Postgres(store) => store.leave_event_roster(event_id, user_id).await,
        }
    }

    pub async fn get_event_rosters(&self, event_id: Uuid) -> anyhow::Result<Vec<EventRoster>> {
        match self {
            Self::Memory(store) => Ok(store.get_event_rosters(event_id)),

            Self::Postgres(store) => store.get_event_rosters(event_id).await,
        }
    }

    pub async fn create_tournament(
        &self,

        guild_id: &str,

        name: &str,

        teams: &[String],
    ) -> anyhow::Result<Tournament> {
        match self {
            Self::Memory(store) => Ok(store.create_tournament(guild_id, name, teams)),

            Self::Postgres(store) => store.create_tournament(guild_id, name, teams).await,
        }
    }

    pub async fn advance_tournament_match(
        &self,
        guild_id: &str,
        tournament_id: Uuid,
        match_idx: usize,
        winner: &str,
    ) -> anyhow::Result<Tournament> {
        match self {
            Self::Memory(store) => {
                store.advance_tournament_match(guild_id, tournament_id, match_idx, winner)
            }
            Self::Postgres(store) => {
                store
                    .advance_tournament_match(guild_id, tournament_id, match_idx, winner)
                    .await
            }
        }
    }

    pub async fn record_invite(
        &self,

        guild_id: &str,

        code: &str,

        inviter_id: &str,

        campaign_tag: Option<&str>,
    ) -> anyhow::Result<InviteTracking> {
        match self {
            Self::Memory(store) => {
                Ok(store.record_invite(guild_id, code, inviter_id, campaign_tag))
            }

            Self::Postgres(store) => {
                store
                    .record_invite(guild_id, code, inviter_id, campaign_tag)
                    .await
            }
        }
    }

    pub async fn list_invites(&self, guild_id: &str) -> anyhow::Result<Vec<InviteTracking>> {
        match self {
            Self::Memory(store) => Ok(store.list_invites(guild_id)),

            Self::Postgres(store) => store.list_invites(guild_id).await,
        }
    }

    pub async fn register_dynamic_voice_room(
        &self,

        channel_id: &str,

        guild_id: &str,

        owner_id: &str,
    ) -> anyhow::Result<DynamicVoiceRoom> {
        match self {
            Self::Memory(store) => {
                Ok(store.register_dynamic_voice_room(channel_id, guild_id, owner_id))
            }

            Self::Postgres(store) => {
                store
                    .register_dynamic_voice_room(channel_id, guild_id, owner_id)
                    .await
            }
        }
    }

    pub async fn update_dynamic_voice_empty_state(
        &self,
        guild_id: &str,
        channel_id: &str,
        is_empty: bool,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => {
                Ok(store.update_dynamic_voice_empty_state(guild_id, channel_id, is_empty))
            }
            Self::Postgres(store) => {
                store
                    .update_dynamic_voice_empty_state(guild_id, channel_id, is_empty)
                    .await
            }
        }
    }

    pub async fn get_stale_dynamic_voice_rooms(
        &self,

        threshold_seconds: i64,
    ) -> anyhow::Result<Vec<DynamicVoiceRoom>> {
        match self {
            Self::Memory(store) => Ok(store.get_stale_dynamic_voice_rooms(threshold_seconds)),

            Self::Postgres(store) => store.get_stale_dynamic_voice_rooms(threshold_seconds).await,
        }
    }

    pub async fn remove_dynamic_voice_room(&self, channel_id: &str) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => Ok(store.remove_dynamic_voice_room(channel_id)),

            Self::Postgres(store) => store.remove_dynamic_voice_room(channel_id).await,
        }
    }

    pub async fn create_feed_subscription(
        &self,

        guild_id: &str,

        channel_id: &str,

        feed_type: &str,

        url_or_target: &str,
    ) -> anyhow::Result<FeedSubscription> {
        match self {
            Self::Memory(store) => {
                Ok(store.create_feed_subscription(guild_id, channel_id, feed_type, url_or_target))
            }

            Self::Postgres(store) => {
                store
                    .create_feed_subscription(guild_id, channel_id, feed_type, url_or_target)
                    .await
            }
        }
    }

    pub async fn list_feed_subscriptions(
        &self,

        guild_id: &str,
    ) -> anyhow::Result<Vec<FeedSubscription>> {
        match self {
            Self::Memory(store) => Ok(store.list_feed_subscriptions(guild_id)),

            Self::Postgres(store) => store.list_feed_subscriptions(guild_id).await,
        }
    }

    pub async fn update_feed_watermark(
        &self,

        subscription_id: Uuid,

        watermark: &str,
    ) -> anyhow::Result<()> {
        match self {
            Self::Memory(store) => {
                store.update_feed_watermark(subscription_id, watermark);

                Ok(())
            }

            Self::Postgres(store) => {
                store
                    .update_feed_watermark(subscription_id, watermark)
                    .await
            }
        }
    }

    pub async fn create_game_link(
        &self,

        guild_id: &str,

        user_id: &str,

        platform: &str,

        platform_account_id: &str,

        code: Option<&str>,
    ) -> anyhow::Result<GameAccountLink> {
        match self {
            Self::Memory(store) => {
                Ok(store.create_game_link(guild_id, user_id, platform, platform_account_id, code))
            }

            Self::Postgres(store) => {
                store
                    .create_game_link(guild_id, user_id, platform, platform_account_id, code)
                    .await
            }
        }
    }

    pub async fn verify_game_link(
        &self,

        guild_id: &str,

        user_id: &str,

        platform: &str,

        code: &str,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => Ok(store.verify_game_link(guild_id, user_id, platform, code)),

            Self::Postgres(store) => {
                store
                    .verify_game_link(guild_id, user_id, platform, code)
                    .await
            }
        }
    }

    pub async fn get_game_links(
        &self,

        guild_id: &str,

        user_id: &str,
    ) -> anyhow::Result<Vec<GameAccountLink>> {
        match self {
            Self::Memory(store) => Ok(store.get_game_links(guild_id, user_id)),

            Self::Postgres(store) => store.get_game_links(guild_id, user_id).await,
        }
    }

    pub async fn get_or_create_counting_game(
        &self,

        guild_id: &str,

        channel_id: &str,
    ) -> anyhow::Result<CountingGame> {
        match self {
            Self::Memory(store) => Ok(store.get_or_create_counting_game(guild_id, channel_id)),

            Self::Postgres(store) => {
                store
                    .get_or_create_counting_game(guild_id, channel_id)
                    .await
            }
        }
    }

    pub async fn submit_count(
        &self,

        guild_id: &str,

        channel_id: &str,

        user_id: &str,

        count: i64,
    ) -> anyhow::Result<(bool, CountingGame)> {
        match self {
            Self::Memory(store) => Ok(store.submit_count(guild_id, channel_id, user_id, count)),

            Self::Postgres(store) => {
                store
                    .submit_count(guild_id, channel_id, user_id, count)
                    .await
            }
        }
    }

    pub async fn list_user_cards(
        &self,

        guild_id: &str,

        user_id: &str,
    ) -> anyhow::Result<Vec<CardCollection>> {
        match self {
            Self::Memory(store) => Ok(store.list_user_cards(guild_id, user_id)),

            Self::Postgres(store) => store.list_user_cards(guild_id, user_id).await,
        }
    }

    pub async fn grant_user_card(
        &self,

        guild_id: &str,

        user_id: &str,

        card_id: &str,

        card_name: &str,

        rarity: &str,
    ) -> anyhow::Result<CardCollection> {
        match self {
            Self::Memory(store) => {
                Ok(store.grant_user_card(guild_id, user_id, card_id, card_name, rarity))
            }

            Self::Postgres(store) => {
                store
                    .grant_user_card(guild_id, user_id, card_id, card_name, rarity)
                    .await
            }
        }
    }

    pub async fn propose_card_trade(
        &self,

        guild_id: &str,

        initiator_id: &str,

        initiator_card_id: Uuid,

        target_id: &str,

        target_card_id: Uuid,
    ) -> anyhow::Result<CardTrade> {
        anyhow::ensure!(
            initiator_card_id != target_card_id,
            "cannot trade a card for itself"
        );

        match self {
            Self::Memory(store) => store.propose_card_trade(
                guild_id,
                initiator_id,
                initiator_card_id,
                target_id,
                target_card_id,
            ),

            Self::Postgres(store) => {
                store
                    .propose_card_trade(
                        guild_id,
                        initiator_id,
                        initiator_card_id,
                        target_id,
                        target_card_id,
                    )
                    .await
            }
        }
    }

    pub async fn accept_card_trade(
        &self,

        guild_id: &str,

        trade_id: Uuid,

        target_id: &str,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => store.accept_card_trade(guild_id, trade_id, target_id),

            Self::Postgres(store) => store.accept_card_trade(guild_id, trade_id, target_id).await,
        }
    }

    pub async fn create_tupper_profile(
        &self,

        guild_id: &str,

        user_id: &str,

        name: &str,

        avatar_url: Option<&str>,

        prefix_or_brackets: &str,
    ) -> Result<TupperProfile, TupperError> {
        match self {
            Self::Memory(store) => {
                store.create_tupper_profile(guild_id, user_id, name, avatar_url, prefix_or_brackets)
            }

            Self::Postgres(store) => {
                store
                    .create_tupper_profile(guild_id, user_id, name, avatar_url, prefix_or_brackets)
                    .await
            }
        }
    }

    pub async fn list_tupper_profiles(
        &self,

        guild_id: &str,

        user_id: &str,
    ) -> anyhow::Result<Vec<TupperProfile>> {
        match self {
            Self::Memory(store) => Ok(store.list_tupper_profiles(guild_id, user_id)),

            Self::Postgres(store) => store.list_tupper_profiles(guild_id, user_id).await,
        }
    }

    pub async fn create_faq_entry(
        &self,

        guild_id: &str,

        question: &str,

        answer: &str,

        aliases: &[String],

        category: &str,
    ) -> anyhow::Result<FaqEntry> {
        match self {
            Self::Memory(store) => {
                Ok(store.create_faq_entry(guild_id, question, answer, aliases, category))
            }

            Self::Postgres(store) => {
                store
                    .create_faq_entry(guild_id, question, answer, aliases, category)
                    .await
            }
        }
    }

    pub async fn list_faq_entries(
        &self,

        guild_id: &str,

        query: Option<&str>,
    ) -> anyhow::Result<Vec<FaqEntry>> {
        match self {
            Self::Memory(store) => Ok(store.list_faq_entries(guild_id, query)),

            Self::Postgres(store) => store.list_faq_entries(guild_id, query).await,
        }
    }

    pub async fn delete_faq_entry(&self, guild_id: &str, faq_id: Uuid) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => Ok(store.delete_faq_entry(guild_id, faq_id)),

            Self::Postgres(store) => store.delete_faq_entry(guild_id, faq_id).await,
        }
    }

    // ==========================================

    // Phase 4: Integrations & Extensibility (#92-#107)

    // ==========================================

    pub async fn upsert_integration_connection(
        &self,

        conn: IntegrationConnection,
    ) -> anyhow::Result<IntegrationConnection> {
        match self {
            Self::Memory(store) => Ok(store.upsert_integration_connection(conn)),

            Self::Postgres(store) => store.upsert_integration_connection(conn).await,
        }
    }

    pub async fn get_integration_connection(
        &self,

        guild_id: &str,

        provider: &str,
    ) -> anyhow::Result<Option<IntegrationConnection>> {
        match self {
            Self::Memory(store) => Ok(store.get_integration_connection(guild_id, provider)),

            Self::Postgres(store) => store.get_integration_connection(guild_id, provider).await,
        }
    }

    pub async fn list_integration_connections(
        &self,

        guild_id: &str,
    ) -> anyhow::Result<Vec<IntegrationConnection>> {
        match self {
            Self::Memory(store) => Ok(store.list_integration_connections(guild_id)),

            Self::Postgres(store) => store.list_integration_connections(guild_id).await,
        }
    }

    pub async fn delete_integration_connection(
        &self,

        guild_id: &str,

        provider: &str,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => Ok(store.delete_integration_connection(guild_id, provider)),

            Self::Postgres(store) => {
                store
                    .delete_integration_connection(guild_id, provider)
                    .await
            }
        }
    }

    pub async fn create_webhook_subscription(
        &self,

        sub: IntegrationWebhookSubscription,
    ) -> anyhow::Result<IntegrationWebhookSubscription> {
        match self {
            Self::Memory(store) => Ok(store.create_webhook_subscription(sub)),

            Self::Postgres(store) => store.create_webhook_subscription(sub).await,
        }
    }

    pub async fn list_webhook_subscriptions(
        &self,

        guild_id: &str,

        provider: Option<&str>,
    ) -> anyhow::Result<Vec<IntegrationWebhookSubscription>> {
        match self {
            Self::Memory(store) => Ok(store.list_webhook_subscriptions(guild_id, provider)),

            Self::Postgres(store) => store.list_webhook_subscriptions(guild_id, provider).await,
        }
    }

    pub async fn delete_webhook_subscription(&self, id: Uuid) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => Ok(store.delete_webhook_subscription(id)),

            Self::Postgres(store) => store.delete_webhook_subscription(id).await,
        }
    }

    pub async fn check_and_record_event_dedupe(
        &self,

        provider: &str,

        event_id: &str,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => Ok(store.check_and_record_event_dedupe(provider, event_id)),

            Self::Postgres(store) => {
                store
                    .check_and_record_event_dedupe(provider, event_id)
                    .await
            }
        }
    }

    pub async fn record_delivery_log(
        &self,

        log: IntegrationDeliveryLog,
    ) -> anyhow::Result<IntegrationDeliveryLog> {
        match self {
            Self::Memory(store) => Ok(store.record_delivery_log(log)),

            Self::Postgres(store) => store.record_delivery_log(log).await,
        }
    }

    pub async fn list_delivery_logs(
        &self,

        guild_id: &str,

        provider: Option<&str>,
    ) -> anyhow::Result<Vec<IntegrationDeliveryLog>> {
        match self {
            Self::Memory(store) => Ok(store.list_delivery_logs(guild_id, provider)),

            Self::Postgres(store) => store.list_delivery_logs(guild_id, provider).await,
        }
    }

    // Agent 04 (#93): Unified Identity

    pub async fn link_account(&self, link: LinkedAccount) -> anyhow::Result<LinkedAccount> {
        match self {
            Self::Memory(store) => store.link_account(link),

            Self::Postgres(store) => store.link_account(link).await,
        }
    }

    pub async fn get_linked_account(
        &self,

        guild_id: &str,

        user_id: &str,

        provider: &str,
    ) -> anyhow::Result<Option<LinkedAccount>> {
        match self {
            Self::Memory(store) => Ok(store.get_linked_account(guild_id, user_id, provider)),

            Self::Postgres(store) => store.get_linked_account(guild_id, user_id, provider).await,
        }
    }

    pub async fn list_linked_accounts(
        &self,

        guild_id: &str,

        user_id: &str,
    ) -> anyhow::Result<Vec<LinkedAccount>> {
        match self {
            Self::Memory(store) => Ok(store.list_linked_accounts(guild_id, user_id)),

            Self::Postgres(store) => store.list_linked_accounts(guild_id, user_id).await,
        }
    }

    pub async fn unlink_account(
        &self,

        guild_id: &str,

        user_id: &str,

        provider: &str,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => Ok(store.unlink_account(guild_id, user_id, provider)),

            Self::Postgres(store) => store.unlink_account(guild_id, user_id, provider).await,
        }
    }

    // Agent 05 (#94) & Agent 15 (#104): Steam & Gaming Hub LFG

    pub async fn create_steam_lfg_post(&self, post: SteamLfgPost) -> anyhow::Result<SteamLfgPost> {
        match self {
            Self::Memory(store) => Ok(store.create_steam_lfg_post(post)),

            Self::Postgres(store) => store.create_steam_lfg_post(post).await,
        }
    }

    pub async fn list_steam_lfg_posts(
        &self,

        guild_id: &str,

        game_id: Option<&str>,
    ) -> anyhow::Result<Vec<SteamLfgPost>> {
        match self {
            Self::Memory(store) => Ok(store.list_steam_lfg_posts(guild_id, game_id)),

            Self::Postgres(store) => store.list_steam_lfg_posts(guild_id, game_id).await,
        }
    }

    pub async fn join_steam_lfg_post(
        &self,

        guild_id: &str,

        id: Uuid,

        user_id: &str,
    ) -> anyhow::Result<SteamLfgPost> {
        match self {
            Self::Memory(store) => store.join_steam_lfg_post(guild_id, id, user_id),

            Self::Postgres(store) => store.join_steam_lfg_post(guild_id, id, user_id).await,
        }
    }

    // Agent 06 (#95): GitHub DevOps

    pub async fn upsert_github_devops_mapping(
        &self,

        mapping: GitHubDevopsMapping,
    ) -> anyhow::Result<GitHubDevopsMapping> {
        match self {
            Self::Memory(store) => Ok(store.upsert_github_devops_mapping(mapping)),

            Self::Postgres(store) => store.upsert_github_devops_mapping(mapping).await,
        }
    }

    pub async fn get_github_devops_mapping(
        &self,

        guild_id: &str,

        repo: &str,

        number: i32,
    ) -> anyhow::Result<Option<GitHubDevopsMapping>> {
        match self {
            Self::Memory(store) => Ok(store.get_github_devops_mapping(guild_id, repo, number)),

            Self::Postgres(store) => {
                store
                    .get_github_devops_mapping(guild_id, repo, number)
                    .await
            }
        }
    }

    // Agent 07 (#96): Twitch Extended

    pub async fn upsert_twitch_subscription(
        &self,

        sub: TwitchStreamSubscription,
    ) -> anyhow::Result<TwitchStreamSubscription> {
        match self {
            Self::Memory(store) => Ok(store.upsert_twitch_subscription(sub)),

            Self::Postgres(store) => store.upsert_twitch_subscription(sub).await,
        }
    }

    pub async fn get_twitch_subscription(
        &self,

        guild_id: &str,

        streamer: &str,
    ) -> anyhow::Result<Option<TwitchStreamSubscription>> {
        match self {
            Self::Memory(store) => Ok(store.get_twitch_subscription(guild_id, streamer)),

            Self::Postgres(store) => store.get_twitch_subscription(guild_id, streamer).await,
        }
    }

    // Agent 08 (#97): Kick Integration

    pub async fn upsert_kick_subscription(
        &self,

        sub: KickStreamSubscription,
    ) -> anyhow::Result<KickStreamSubscription> {
        match self {
            Self::Memory(store) => Ok(store.upsert_kick_subscription(sub)),

            Self::Postgres(store) => store.upsert_kick_subscription(sub).await,
        }
    }

    pub async fn get_kick_subscription(
        &self,

        guild_id: &str,

        slug: &str,
    ) -> anyhow::Result<Option<KickStreamSubscription>> {
        match self {
            Self::Memory(store) => Ok(store.get_kick_subscription(guild_id, slug)),

            Self::Postgres(store) => store.get_kick_subscription(guild_id, slug).await,
        }
    }

    // Agent 09 (#98): YouTube Extended

    pub async fn upsert_youtube_subscription(
        &self,

        sub: YouTubeChannelSubscription,
    ) -> anyhow::Result<YouTubeChannelSubscription> {
        match self {
            Self::Memory(store) => Ok(store.upsert_youtube_subscription(sub)),

            Self::Postgres(store) => store.upsert_youtube_subscription(sub).await,
        }
    }

    pub async fn get_youtube_subscription(
        &self,

        guild_id: &str,

        channel_id: &str,
    ) -> anyhow::Result<Option<YouTubeChannelSubscription>> {
        match self {
            Self::Memory(store) => Ok(store.get_youtube_subscription(guild_id, channel_id)),

            Self::Postgres(store) => store.get_youtube_subscription(guild_id, channel_id).await,
        }
    }

    // Agent 10 (#99): Telegram Bridge

    pub async fn create_telegram_bridge(
        &self,

        bridge: TelegramBridge,
    ) -> anyhow::Result<TelegramBridge> {
        match self {
            Self::Memory(store) => Ok(store.create_telegram_bridge(bridge)),

            Self::Postgres(store) => store.create_telegram_bridge(bridge).await,
        }
    }

    pub async fn get_telegram_bridge(
        &self,

        guild_id: &str,

        channel_id: &str,
    ) -> anyhow::Result<Option<TelegramBridge>> {
        match self {
            Self::Memory(store) => Ok(store.get_telegram_bridge(guild_id, channel_id)),

            Self::Postgres(store) => store.get_telegram_bridge(guild_id, channel_id).await,
        }
    }

    pub async fn list_telegram_bridges(
        &self,

        guild_id: &str,
    ) -> anyhow::Result<Vec<TelegramBridge>> {
        match self {
            Self::Memory(store) => Ok(store.list_telegram_bridges(guild_id)),

            Self::Postgres(store) => store.list_telegram_bridges(guild_id).await,
        }
    }

    pub async fn record_telegram_message_mapping(
        &self,

        mapping: TelegramMessageMapping,
    ) -> anyhow::Result<TelegramMessageMapping> {
        match self {
            Self::Memory(store) => Ok(store.record_telegram_message_mapping(mapping)),

            Self::Postgres(store) => store.record_telegram_message_mapping(mapping).await,
        }
    }

    pub async fn get_telegram_message_mapping(
        &self,

        bridge_id: Uuid,

        msg_id: &str,

        direction: &str,
    ) -> anyhow::Result<Option<TelegramMessageMapping>> {
        match self {
            Self::Memory(store) => {
                Ok(store.get_telegram_message_mapping(bridge_id, msg_id, direction))
            }

            Self::Postgres(store) => {
                store
                    .get_telegram_message_mapping(bridge_id, msg_id, direction)
                    .await
            }
        }
    }

    pub async fn delete_telegram_bridge(&self, bridge_id: Uuid) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => Ok(store.delete_telegram_bridge(bridge_id)),

            Self::Postgres(store) => store.delete_telegram_bridge(bridge_id).await,
        }
    }

    // Agent 11 (#100): Workflow Builder

    pub async fn create_workflow_definition(
        &self,

        wf: WorkflowDefinition,
    ) -> anyhow::Result<WorkflowDefinition> {
        match self {
            Self::Memory(store) => Ok(store.create_workflow_definition(wf)),

            Self::Postgres(store) => store.create_workflow_definition(wf).await,
        }
    }

    pub async fn get_workflow_definition(
        &self,

        id: Uuid,
    ) -> anyhow::Result<Option<WorkflowDefinition>> {
        match self {
            Self::Memory(store) => Ok(store.get_workflow_definition(id)),

            Self::Postgres(store) => store.get_workflow_definition(id).await,
        }
    }

    pub async fn list_workflow_definitions(
        &self,

        guild_id: &str,
    ) -> anyhow::Result<Vec<WorkflowDefinition>> {
        match self {
            Self::Memory(store) => Ok(store.list_workflow_definitions(guild_id)),

            Self::Postgres(store) => store.list_workflow_definitions(guild_id).await,
        }
    }

    pub async fn record_workflow_run(
        &self,

        run: WorkflowRunExecution,
    ) -> anyhow::Result<WorkflowRunExecution> {
        match self {
            Self::Memory(store) => Ok(store.record_workflow_run(run)),

            Self::Postgres(store) => store.record_workflow_run(run).await,
        }
    }

    // Agent 12 (#101): GitLab

    pub async fn upsert_gitlab_subscription(
        &self,

        sub: GitLabSubscription,
    ) -> anyhow::Result<GitLabSubscription> {
        match self {
            Self::Memory(store) => Ok(store.upsert_gitlab_subscription(sub)),

            Self::Postgres(store) => store.upsert_gitlab_subscription(sub).await,
        }
    }

    pub async fn get_gitlab_subscription(
        &self,

        guild_id: &str,

        host: &str,

        project_id: &str,
    ) -> anyhow::Result<Option<GitLabSubscription>> {
        match self {
            Self::Memory(store) => Ok(store.get_gitlab_subscription(guild_id, host, project_id)),

            Self::Postgres(store) => {
                store
                    .get_gitlab_subscription(guild_id, host, project_id)
                    .await
            }
        }
    }

    // Agent 13 (#102): VK Integration

    pub async fn upsert_vk_subscription(
        &self,

        sub: VkSubscription,
    ) -> anyhow::Result<VkSubscription> {
        match self {
            Self::Memory(store) => Ok(store.upsert_vk_subscription(sub)),

            Self::Postgres(store) => store.upsert_vk_subscription(sub).await,
        }
    }

    pub async fn get_vk_subscription(
        &self,

        guild_id: &str,

        group_id: &str,
    ) -> anyhow::Result<Option<VkSubscription>> {
        match self {
            Self::Memory(store) => Ok(store.get_vk_subscription(guild_id, group_id)),

            Self::Postgres(store) => store.get_vk_subscription(guild_id, group_id).await,
        }
    }

    // Agent 14 (#103): Reddit

    pub async fn upsert_reddit_subscription(
        &self,

        sub: RedditSubscription,
    ) -> anyhow::Result<RedditSubscription> {
        match self {
            Self::Memory(store) => Ok(store.upsert_reddit_subscription(sub)),

            Self::Postgres(store) => store.upsert_reddit_subscription(sub).await,
        }
    }

    pub async fn get_reddit_subscription(
        &self,

        guild_id: &str,

        subreddit: &str,
    ) -> anyhow::Result<Option<RedditSubscription>> {
        match self {
            Self::Memory(store) => Ok(store.get_reddit_subscription(guild_id, subreddit)),

            Self::Postgres(store) => store.get_reddit_subscription(guild_id, subreddit).await,
        }
    }

    // Agent 15 (#104): Gaming Hub

    pub async fn update_gaming_hub_game_status(
        &self,

        status: GamingHubGameStatus,
    ) -> anyhow::Result<GamingHubGameStatus> {
        match self {
            Self::Memory(store) => Ok(store.update_gaming_hub_game_status(status)),

            Self::Postgres(store) => store.update_gaming_hub_game_status(status).await,
        }
    }

    pub async fn list_gaming_hub_game_statuses(
        &self,

        game_id: Option<&str>,
    ) -> anyhow::Result<Vec<GamingHubGameStatus>> {
        match self {
            Self::Memory(store) => Ok(store.list_gaming_hub_game_statuses(game_id)),

            Self::Postgres(store) => store.list_gaming_hub_game_statuses(game_id).await,
        }
    }

    // Agent 16 (#105): Social Publisher

    pub async fn create_social_draft(
        &self,

        draft: SocialPublicationDraft,
    ) -> anyhow::Result<SocialPublicationDraft> {
        match self {
            Self::Memory(store) => Ok(store.create_social_draft(draft)),

            Self::Postgres(store) => store.create_social_draft(draft).await,
        }
    }

    pub async fn list_social_drafts(
        &self,

        guild_id: &str,
    ) -> anyhow::Result<Vec<SocialPublicationDraft>> {
        match self {
            Self::Memory(store) => Ok(store.list_social_drafts(guild_id)),

            Self::Postgres(store) => store.list_social_drafts(guild_id).await,
        }
    }

    pub async fn approve_social_draft(
        &self,

        guild_id: &str,

        id: Uuid,

        approver_id: &str,
    ) -> anyhow::Result<Option<SocialPublicationDraft>> {
        match self {
            Self::Memory(store) => Ok(store.approve_social_draft(guild_id, id, approver_id)),

            Self::Postgres(store) => store.approve_social_draft(guild_id, id, approver_id).await,
        }
    }

    // Agent 17 & 18 (#106): AI + MCP

    pub async fn get_or_create_ai_budget(&self, guild_id: &str) -> anyhow::Result<AiGuildBudget> {
        match self {
            Self::Memory(store) => Ok(store.get_or_create_ai_budget(guild_id)),

            Self::Postgres(store) => store.get_or_create_ai_budget(guild_id).await,
        }
    }

    pub async fn record_ai_usage(
        &self,

        guild_id: &str,

        tokens: i64,

        usd: f64,
    ) -> anyhow::Result<AiGuildBudget> {
        match self {
            Self::Memory(store) => store.record_ai_usage(guild_id, tokens, usd),

            Self::Postgres(store) => store.record_ai_usage(guild_id, tokens, usd).await,
        }
    }

    pub async fn create_ai_document(
        &self,

        doc: AiKnowledgeDocument,
    ) -> anyhow::Result<AiKnowledgeDocument> {
        match self {
            Self::Memory(store) => Ok(store.create_ai_document(doc)),

            Self::Postgres(store) => store.create_ai_document(doc).await,
        }
    }

    pub async fn search_ai_documents(
        &self,

        guild_id: &str,

        user_channels: &[String],

        query: &str,
    ) -> anyhow::Result<Vec<AiKnowledgeDocument>> {
        match self {
            Self::Memory(store) => Ok(store.search_ai_documents(guild_id, user_channels, query)),

            Self::Postgres(store) => {
                store
                    .search_ai_documents(guild_id, user_channels, query)
                    .await
            }
        }
    }

    pub async fn upsert_mcp_tool_authorization(
        &self,

        auth: McpToolAuthorization,
    ) -> anyhow::Result<McpToolAuthorization> {
        match self {
            Self::Memory(store) => Ok(store.upsert_mcp_tool_authorization(auth)),

            Self::Postgres(store) => store.upsert_mcp_tool_authorization(auth).await,
        }
    }

    pub async fn get_mcp_tool_authorizations(
        &self,

        guild_id: &str,
    ) -> anyhow::Result<Vec<McpToolAuthorization>> {
        match self {
            Self::Memory(store) => Ok(store.get_mcp_tool_authorizations(guild_id)),

            Self::Postgres(store) => store.get_mcp_tool_authorizations(guild_id).await,
        }
    }

    // Agent 19 (#107): Plugin Marketplace

    pub async fn publish_marketplace_package(
        &self,

        pkg: PluginMarketplacePackage,
    ) -> anyhow::Result<PluginMarketplacePackage> {
        match self {
            Self::Memory(store) => Ok(store.publish_marketplace_package(pkg)),

            Self::Postgres(store) => store.publish_marketplace_package(pkg).await,
        }
    }

    pub async fn list_marketplace_packages(&self) -> anyhow::Result<Vec<PluginMarketplacePackage>> {
        match self {
            Self::Memory(store) => Ok(store.list_marketplace_packages()),

            Self::Postgres(store) => store.list_marketplace_packages().await,
        }
    }

    pub async fn install_plugin_package(
        &self,

        install: PluginGuildInstallation,
    ) -> anyhow::Result<PluginGuildInstallation> {
        match self {
            Self::Memory(store) => Ok(store.install_plugin_package(install)),

            Self::Postgres(store) => store.install_plugin_package(install).await,
        }
    }

    pub async fn list_installed_plugins(
        &self,

        guild_id: &str,
    ) -> anyhow::Result<Vec<PluginGuildInstallation>> {
        match self {
            Self::Memory(store) => Ok(store.list_installed_plugins(guild_id)),

            Self::Postgres(store) => store.list_installed_plugins(guild_id).await,
        }
    }

    pub async fn uninstall_plugin_package(
        &self,

        guild_id: &str,

        plugin_id: &str,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => Ok(store.uninstall_plugin_package(guild_id, plugin_id)),

            Self::Postgres(store) => store.uninstall_plugin_package(guild_id, plugin_id).await,
        }
    }

    pub async fn record_asset_usage_batch(
        &self,
        daily_items: &[crate::asset_analytics::AssetUsageDaily],
        inventory_items: &[crate::asset_analytics::AssetInventoryItem],
    ) -> anyhow::Result<usize> {
        match self {
            Self::Memory(store) => store.record_asset_usage_batch(daily_items, inventory_items),
            Self::Postgres(store) => {
                store
                    .record_asset_usage_batch(daily_items, inventory_items)
                    .await
            }
        }
    }

    pub async fn get_asset_usage_summary(
        &self,
        guild_id: &str,
        kind: Option<crate::asset_analytics::AssetKind>,
        period: &str,
    ) -> anyhow::Result<crate::asset_analytics::AssetUsageSummary> {
        match self {
            Self::Memory(store) => store.get_asset_usage_summary(guild_id, kind, period),
            Self::Postgres(store) => store.get_asset_usage_summary(guild_id, kind, period).await,
        }
    }

    pub async fn list_asset_inventory(
        &self,
        guild_id: &str,
        kind: Option<crate::asset_analytics::AssetKind>,
    ) -> anyhow::Result<Vec<crate::asset_analytics::AssetUsageStat>> {
        match self {
            Self::Memory(store) => store.list_asset_inventory(guild_id, kind),
            Self::Postgres(store) => store.list_asset_inventory(guild_id, kind).await,
        }
    }

    pub async fn sync_guild_inventory_active_state(
        &self,
        guild_id: &str,
        kind: crate::asset_analytics::AssetKind,
        active_ids: &[String],
    ) -> anyhow::Result<usize> {
        match self {
            Self::Memory(store) => {
                store.sync_guild_inventory_active_state(guild_id, kind, active_ids)
            }
            Self::Postgres(store) => {
                store
                    .sync_guild_inventory_active_state(guild_id, kind, active_ids)
                    .await
            }
        }
    }

    pub async fn reconcile_guild_inventory(
        &self,
        guild_id: &str,
        kind: crate::asset_analytics::AssetKind,
        active_items: &[crate::asset_analytics::AssetInventoryItem],
    ) -> anyhow::Result<usize> {
        match self {
            Self::Memory(store) => store.reconcile_guild_inventory(guild_id, kind, active_items),
            Self::Postgres(store) => {
                store
                    .reconcile_guild_inventory(guild_id, kind, active_items)
                    .await
            }
        }
    }

    pub async fn prune_asset_analytics(
        &self,
        guild_id: Option<&str>,
        older_than_days: u32,
    ) -> anyhow::Result<usize> {
        match self {
            Self::Memory(store) => store.prune_asset_analytics(guild_id, older_than_days),
            Self::Postgres(store) => store.prune_asset_analytics(guild_id, older_than_days).await,
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[tokio::test]

    async fn test_update_module_settings_with_audit_atomic_success_and_rollback() {
        let store = Storage::Memory(MemoryStore::default());

        let guild_id = "100000000000000001";

        let module_id = "moderation";

        // 1. Successful atomic update

        let initial_config = serde_json::json!({"action": "log"});

        let updated = store
            .update_module_settings_with_audit(UpdateModuleSettingsParams {
                guild_id,

                module_id,

                enabled: true,

                config: initial_config.clone(),

                actor_id: Some("admin_1"),

                audit_event: "module_settings_updated",

                audit_data: serde_json::json!({"module_id": module_id, "enabled": true}),
            })
            .await
            .expect("update should succeed");

        assert!(updated.enabled);

        assert_eq!(updated.config, initial_config);

        let fetched = store
            .get_module_settings(guild_id, module_id)
            .await
            .unwrap()
            .unwrap();

        assert!(fetched.enabled);

        // 2. Inject audit failure

        store.inject_audit_failure(true);

        let new_config = serde_json::json!({"action": "ban"});

        let failed_result = store
            .update_module_settings_with_audit(UpdateModuleSettingsParams {
                guild_id,

                module_id,

                enabled: false,

                config: new_config,

                actor_id: Some("admin_1"),

                audit_event: "module_settings_updated",

                audit_data: serde_json::json!({"module_id": module_id, "enabled": false}),
            })
            .await;

        assert!(failed_result.is_err());

        // 3. Verify rollback: settings must remain exactly as before

        let after_rollback = store
            .get_module_settings(guild_id, module_id)
            .await
            .unwrap()
            .unwrap();

        assert!(
            after_rollback.enabled,
            "module must still be enabled due to atomic rollback"
        );

        assert_eq!(
            after_rollback.config, initial_config,
            "config must remain unmutated"
        );

        // 4. Test unconfigured module fresh rollback

        let unconfigured_mod = "automod";

        let failed_fresh = store
            .update_module_settings_with_audit(UpdateModuleSettingsParams {
                guild_id,

                module_id: unconfigured_mod,

                enabled: true,

                config: serde_json::json!({}),

                actor_id: Some("admin_1"),

                audit_event: "module_settings_updated",

                audit_data: serde_json::json!({}),
            })
            .await;

        assert!(failed_fresh.is_err());

        let fetched_fresh = store
            .get_module_settings(guild_id, unconfigured_mod)
            .await
            .unwrap();

        assert!(
            fetched_fresh.is_none(),
            "unconfigured module settings must not exist after rollback"
        );
    }

    #[tokio::test]
    async fn test_store_item_negative_price_and_stock_rejected() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_id = "guild_123";

        // Negative price rejected
        let res_neg_price = store
            .create_store_item(guild_id, "Bad Item", "Desc", -10, None, None)
            .await;
        assert!(res_neg_price.is_err());
        assert!(
            res_neg_price
                .unwrap_err()
                .to_string()
                .contains("price must be non-negative")
        );

        // Negative stock rejected
        let res_neg_stock = store
            .create_store_item(guild_id, "Bad Item", "Desc", 10, None, Some(-1))
            .await;
        assert!(res_neg_stock.is_err());
        assert!(
            res_neg_stock
                .unwrap_err()
                .to_string()
                .contains("stock must be non-negative")
        );

        // Zero price (free) and positive price allowed
        let free_item = store
            .create_store_item(guild_id, "Free Item", "Desc", 0, None, Some(5))
            .await
            .expect("price 0 should be allowed");
        assert_eq!(free_item.price, 0);

        let paid_item = store
            .create_store_item(guild_id, "Paid Item", "Desc", 100, None, Some(5))
            .await
            .expect("positive price should be allowed");
        assert_eq!(paid_item.price, 100);
    }

    #[tokio::test]
    async fn test_purchase_store_item_rejects_negative_price_without_side_effects() {
        let mem = MemoryStore::default();
        let corrupt_id = Uuid::new_v4();
        let guild_id = "guild_123";
        let user_id = "user_456";

        // Manually insert corrupted store item with negative price
        mem.store_items.insert(
            corrupt_id,
            StoreItem {
                id: corrupt_id,
                guild_id: guild_id.to_owned(),
                name: "Corrupt Item".to_owned(),
                description: "Test".to_owned(),
                price: -100,
                role_id: None,
                stock: Some(5),
                created_at: Utc::now(),
            },
        );

        let store = Storage::Memory(mem);
        let purchase_res = store
            .purchase_store_item(guild_id, user_id, corrupt_id)
            .await;
        assert!(purchase_res.is_err());
        assert!(
            purchase_res
                .unwrap_err()
                .to_string()
                .contains("stored store item has invalid negative price")
        );

        // Verify balance was NOT modified (user still has 0, did NOT gain coins)
        let progress = store.get_member_progress(guild_id, user_id).await.unwrap();
        assert_eq!(progress.coins, 0, "buyer balance must not increase");

        // Verify stock was NOT decremented
        let items = store.list_store_items(guild_id).await.unwrap();
        let corrupt_item = items.iter().find(|i| i.id == corrupt_id).unwrap();
        assert_eq!(corrupt_item.stock, Some(5), "stock must not decrement");
    }

    #[tokio::test]
    async fn test_purchase_store_item_role_snapshot_and_fulfillment_tracking() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_id = "guild_role_test";
        let user_id = "user_buyer";

        store
            .adjust_member_progress(guild_id, user_id, 0, 500, 0)
            .await
            .unwrap();

        let item = store
            .create_store_item(
                guild_id,
                "VIP Role",
                "Grants VIP status",
                200,
                Some("123456789012345678"),
                Some(10),
            )
            .await
            .unwrap();

        let (purchased_item, purchase, remaining) = store
            .purchase_store_item(guild_id, user_id, item.id)
            .await
            .unwrap();

        assert_eq!(purchased_item.id, item.id);
        assert_eq!(remaining, 300);
        assert_eq!(purchase.item_id, item.id);
        assert_eq!(purchase.price_paid, 200);
        assert_eq!(purchase.role_id, Some("123456789012345678".to_string()));
        assert_eq!(purchase.fulfillment_status, "pending");
        assert!(purchase.fulfilled_at.is_none());

        let now = Utc::now();
        store
            .update_store_purchase_fulfillment(purchase.id, "completed", None, Some(now))
            .await
            .unwrap();

        let updated_purchase = store
            .get_store_purchase(purchase.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated_purchase.fulfillment_status, "completed");
        assert_eq!(updated_purchase.fulfilled_at, Some(now));

        let list = store
            .list_store_purchases(guild_id, Some(user_id))
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, purchase.id);
        assert_eq!(list[0].role_id, Some("123456789012345678".to_string()));
    }

    #[test]
    fn test_validate_snowflake_comprehensive_rules() {
        // Valid snowflakes
        assert_eq!(
            validate_snowflake("role_id", "100000000000000001").unwrap(),
            100000000000000001
        );
        assert_eq!(validate_snowflake("role_id", "42").unwrap(), 42);

        // Empty and whitespace
        assert!(validate_snowflake("role_id", "").is_err());
        assert!(validate_snowflake("role_id", "   ").is_err());
        assert!(validate_snowflake("role_id", " 42 ").is_err());
        assert!(validate_snowflake("role_id", "42 ").is_err());
        assert!(validate_snowflake("role_id", " 42").is_err());
        assert!(validate_snowflake("role_id", "\t42").is_err());
        assert!(validate_snowflake("role_id", "42\n").is_err());

        // Zero
        assert!(validate_snowflake("role_id", "0").is_err());
        assert!(validate_snowflake("role_id", "0000").is_err());

        // Non-digits, traversal, and invalid characters
        assert!(validate_snowflake("role_id", "role_color_vip").is_err());
        assert!(
            validate_snowflake("role_id", "../../../../../guilds/333/members/444/roles/555")
                .is_err()
        );
        assert!(validate_snowflake("role_id", "123/456").is_err());
        assert!(validate_snowflake("role_id", "123?456").is_err());
        assert!(validate_snowflake("role_id", "123#456").is_err());
        assert!(validate_snowflake("role_id", "-100").is_err());
        assert!(validate_snowflake("role_id", "12.34").is_err());

        // u64 overflow (2^64)
        assert!(validate_snowflake("role_id", "18446744073709551616").is_err());
    }

    #[tokio::test]
    async fn test_create_store_item_rejects_traversal_role_id() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_id = "guild_123";

        let bad_role = store
            .create_store_item(
                guild_id,
                "Malicious Item",
                "desc",
                100,
                Some("../../../../../guilds/333/members/444/roles/555"),
                Some(5),
            )
            .await;
        assert!(bad_role.is_err());
        assert!(
            bad_role
                .unwrap_err()
                .to_string()
                .contains("must contain only decimal digits")
        );

        let whitespace_role = store
            .create_store_item(
                guild_id,
                "Whitespace Item",
                "desc",
                100,
                Some(" 42 "),
                Some(5),
            )
            .await;
        assert!(whitespace_role.is_err());
        assert!(
            whitespace_role
                .unwrap_err()
                .to_string()
                .contains("must contain only decimal digits")
        );
    }

    #[tokio::test]
    async fn test_purchase_store_item_rejects_corrupted_traversal_role_without_debit() {
        let mem = MemoryStore::default();
        let bad_item_id = Uuid::new_v4();
        let guild_id = "guild_456";
        let user_id = "user_789";

        // Give user 1000 coins
        mem.adjust_member_progress(guild_id, user_id, 0, 1000, 0);

        // Directly insert a corrupted store item that somehow bypassed validation (e.g. legacy DB row)
        mem.store_items.insert(
            bad_item_id,
            StoreItem {
                id: bad_item_id,
                guild_id: guild_id.to_owned(),
                name: "Corrupted Item".to_owned(),
                description: "Test".to_owned(),
                price: 250,
                role_id: Some("../../../../../guilds/333/members/444/roles/555".to_owned()),
                stock: Some(5),
                created_at: Utc::now(),
            },
        );

        let store = Storage::Memory(mem);
        let res = store
            .purchase_store_item(guild_id, user_id, bad_item_id)
            .await;
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("must contain only decimal digits")
        );

        // Verify balance was NOT modified (user still has 1000)
        let progress = store.get_member_progress(guild_id, user_id).await.unwrap();
        assert_eq!(
            progress.coins, 1000,
            "coins must NOT be deducted for invalid role item"
        );

        // Verify stock was NOT decremented
        let items = store.list_store_items(guild_id).await.unwrap();
        let item = items.iter().find(|i| i.id == bad_item_id).unwrap();
        assert_eq!(item.stock, Some(5), "stock must NOT be decremented");
    }

    #[test]
    fn test_url_standard_normalization_behavior() {
        let base = "https://discord.com/api/v10";
        let role_traversal = "../../../../../guilds/333/members/444/roles/555";
        let full = format!("{base}/guilds/111/members/222/roles/{role_traversal}");
        let parsed = reqwest::Url::parse(&full).unwrap();
        // Demonstrates that WHATWG URL Standard normalizes ../ away into /api/v10/guilds/333/members/444/roles/555
        assert_eq!(parsed.path(), "/api/v10/guilds/333/members/444/roles/555");
        // And confirms our guard detects the manipulation:
        let expected_suffix = "/guilds/111/members/222/roles/".to_owned() + role_traversal;
        assert!(!parsed.path().ends_with(&expected_suffix));
    }

    #[tokio::test]
    async fn test_member_identity_history_dedup_retention_and_isolation() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_1 = "100000000000000001";
        let guild_2 = "100000000000000002";
        let user_id = "200000000000000001";

        let change1 = store
            .record_member_identity_change(
                guild_1,
                user_id,
                "nickname",
                Some("Alpha"),
                Some("Beta"),
                90,
            )
            .await
            .unwrap();
        assert!(change1.is_some());

        let dup = store
            .record_member_identity_change(
                guild_1,
                user_id,
                "nickname",
                Some("Beta"),
                Some(" Beta "),
                90,
            )
            .await
            .unwrap();
        assert!(
            dup.is_none(),
            "Identical change must not produce a history record"
        );

        let change2 = store
            .record_member_identity_change(
                guild_1,
                user_id,
                "nickname",
                Some("Beta"),
                Some("Charlie"),
                90,
            )
            .await
            .unwrap();
        assert!(change2.is_some());

        let history = store
            .get_member_identity_history(guild_1, user_id, 10)
            .await
            .unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].new_value, Some("Charlie".to_string()));
        assert_eq!(history[1].new_value, Some("Beta".to_string()));

        let g2_history = store
            .get_member_identity_history(guild_2, user_id, 10)
            .await
            .unwrap();
        assert_eq!(
            g2_history.len(),
            0,
            "Identity history must be strictly per-guild isolated"
        );
    }

    #[tokio::test]
    async fn test_member_identity_history_janitor_and_lifecycle() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_id = "100000000000000001";
        let user_id = "200000000000000001";
        let now = Utc::now();

        // 1. Insert a non-expired record and an expired record
        let active_rec = MemberIdentityRecord {
            id: Uuid::new_v4(),
            guild_id: guild_id.to_string(),
            user_id: user_id.to_string(),
            kind: "nickname".to_string(),
            old_value: Some("OldNick".to_string()),
            new_value: Some("ActiveNick".to_string()),
            observed_at: now - chrono::Duration::days(1),
            expires_at: now + chrono::Duration::days(30),
        };
        let expired_rec = MemberIdentityRecord {
            id: Uuid::new_v4(),
            guild_id: guild_id.to_string(),
            user_id: user_id.to_string(),
            kind: "nickname".to_string(),
            old_value: Some("ActiveNick".to_string()),
            new_value: Some("ExpiredNick".to_string()),
            observed_at: now - chrono::Duration::days(95),
            expires_at: now - chrono::Duration::days(5),
        };

        store.insert_raw_identity_record(active_rec);
        store.insert_raw_identity_record(expired_rec);

        // Both are physically present in the store
        assert_eq!(store.raw_identity_history_count(), Some(2));

        // Logical read filters out expired record
        let logical_history = store
            .get_member_identity_history(guild_id, user_id, 10)
            .await
            .unwrap();
        assert_eq!(logical_history.len(), 1);
        assert_eq!(logical_history[0].new_value, Some("ActiveNick".to_string()));

        // Run cleanup - should prune exactly 1 record
        let pruned = store.cleanup_expired_identity_history().await.unwrap();
        assert_eq!(pruned, 1);
        assert_eq!(store.raw_identity_history_count(), Some(1));

        // Second run is idempotent
        let pruned_again = store.cleanup_expired_identity_history().await.unwrap();
        assert_eq!(pruned_again, 0);
        assert_eq!(store.raw_identity_history_count(), Some(1));

        // 2. Test janitor loop execution and graceful shutdown
        let janitor_store = store.clone();
        let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);

        // Insert another expired record
        let expired_rec_2 = MemberIdentityRecord {
            id: Uuid::new_v4(),
            guild_id: guild_id.to_string(),
            user_id: user_id.to_string(),
            kind: "global_name".to_string(),
            old_value: None,
            new_value: Some("OldGlobal".to_string()),
            observed_at: now - chrono::Duration::days(100),
            expires_at: now - chrono::Duration::days(10),
        };
        store.insert_raw_identity_record(expired_rec_2);
        assert_eq!(store.raw_identity_history_count(), Some(2));

        let janitor_task = tokio::spawn(run_identity_history_janitor(
            janitor_store,
            shutdown_rx,
            std::time::Duration::from_millis(20),
        ));

        // Wait for janitor to tick
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        assert_eq!(store.raw_identity_history_count(), Some(1));

        // Trigger shutdown and wait for janitor to stop cleanly
        let _ = shutdown_tx.send(());
        let res = tokio::time::timeout(std::time::Duration::from_millis(500), janitor_task).await;
        assert!(
            res.is_ok(),
            "janitor must terminate cleanly upon shutdown signal"
        );
    }

    #[tokio::test]
    async fn test_member_search_multicriteria_and_pagination() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_1 = "100000000000000001";
        let guild_2 = "100000000000000002";
        let user_a = "200000000000000001";
        let user_b = "200000000000000002";
        let user_c = "200000000000000003";

        let now = Utc::now();
        let one_hour_ago = now - chrono::Duration::hours(1);
        let two_days_ago = now - chrono::Duration::days(2);

        let role_a = ["9991".to_string()];
        let role_b = ["9992".to_string()];

        store
            .upsert_member_snapshot(UpsertMemberSnapshotParams {
                guild_id: guild_1,
                user_id: user_a,
                username: "alice",
                global_name: Some("AliceInWonderland"),
                nickname: Some("Ali"),
                role_ids: &role_a,
                joined_at: Some(one_hour_ago),
            })
            .await
            .unwrap();

        store
            .upsert_member_snapshot(UpsertMemberSnapshotParams {
                guild_id: guild_1,
                user_id: user_b,
                username: "bob",
                global_name: Some("BobTheBuilder"),
                nickname: None,
                role_ids: &role_b,
                joined_at: Some(two_days_ago),
            })
            .await
            .unwrap();

        store
            .upsert_member_snapshot(UpsertMemberSnapshotParams {
                guild_id: guild_2,
                user_id: user_c,
                username: "charlie",
                global_name: None,
                nickname: None,
                role_ids: &role_a,
                joined_at: Some(one_hour_ago),
            })
            .await
            .unwrap();

        if let Storage::Memory(ref mem) = store {
            mem.set_member_case_count(guild_1, user_a, 2);
        }

        let res_all = store
            .search_members(guild_1, &MemberSearchFilter::default())
            .await
            .unwrap();
        assert_eq!(res_all.len(), 2);
        assert_eq!(res_all[0].user_id, user_a);
        assert_eq!(res_all[1].user_id, user_b);

        let res_role = store
            .search_members(
                guild_1,
                &MemberSearchFilter {
                    role_ids: vec!["9991".to_string()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(res_role.len(), 1);
        assert_eq!(res_role[0].user_id, user_a);

        let res_exclude = store
            .search_members(
                guild_1,
                &MemberSearchFilter {
                    exclude_role_ids: vec!["9991".to_string()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(res_exclude.len(), 1);
        assert_eq!(res_exclude[0].user_id, user_b);

        let res_cases = store
            .search_members(
                guild_1,
                &MemberSearchFilter {
                    min_case_count: Some(2),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(res_cases.len(), 1);
        assert_eq!(res_cases[0].user_id, user_a);

        let res_joined = store
            .search_members(
                guild_1,
                &MemberSearchFilter {
                    joined_after: Some(now - chrono::Duration::days(1)),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(res_joined.len(), 1);
        assert_eq!(res_joined[0].user_id, user_a);

        assert!(res_all.iter().all(|r| r.user_id != user_c));
    }

    #[tokio::test]
    async fn test_member_search_keyset_pagination_and_performance() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_id = "111000000000000001";
        let now = Utc::now();

        // 1. Insert 5 users with varied joined_at (including duplicate timestamps, null, and large snowflakes > 2^53)
        // Snowflake > 2^53: 18_446_744_073_709_551_615 is max u64, e.g. "900719925474099991" > 2^53 (9007199254740992)
        let t1 = now - chrono::Duration::hours(1);
        let t2 = now - chrono::Duration::hours(2);

        let users = vec![
            ("100", "user_100", Some(t1)),
            ("200", "user_200", Some(t1)),
            ("900719925474099991", "user_large", Some(t2)),
            ("150", "user_null_1", None),
            ("250", "user_null_2", None),
        ];

        for (uid, uname, joined) in &users {
            store
                .upsert_member_snapshot(UpsertMemberSnapshotParams {
                    guild_id,
                    user_id: uid,
                    username: uname,
                    global_name: None,
                    nickname: None,
                    role_ids: &[],
                    joined_at: *joined,
                })
                .await
                .unwrap();
        }

        // Expected total order:
        // 1. "100" (t1)
        // 2. "200" (t1)
        // 3. "900719925474099991" (t2)
        // 4. "150" (None)
        // 5. "250" (None)

        // Page 1: limit 2
        let page1 = store
            .search_members(
                guild_id,
                &MemberSearchFilter {
                    limit: Some(2),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(page1.len(), 2);
        assert_eq!(page1[0].user_id, "100");
        assert_eq!(page1[1].user_id, "200");

        let cur1 = encode_member_search_cursor(page1[1].joined_at, &page1[1].user_id);

        // Page 2: limit 2 with cursor from page 1
        let page2 = store
            .search_members(
                guild_id,
                &MemberSearchFilter {
                    limit: Some(2),
                    cursor: Some(cur1),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(page2.len(), 2);
        assert_eq!(page2[0].user_id, "900719925474099991");
        assert_eq!(page2[1].user_id, "150");

        // Test cursor deletion resilience:
        // Delete user "900719925474099991" from the database
        let cur2 = encode_member_search_cursor(page2[0].joined_at, &page2[0].user_id);
        let removed = store
            .remove_member_snapshot(guild_id, "900719925474099991")
            .await
            .unwrap();
        assert!(removed);

        // Page with deleted cursor still seeks correctly without restarting from page 1
        let page_after_del = store
            .search_members(
                guild_id,
                &MemberSearchFilter {
                    limit: Some(2),
                    cursor: Some(cur2),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(page_after_del.len(), 2);
        assert_eq!(page_after_del[0].user_id, "150");
        assert_eq!(page_after_del[1].user_id, "250");

        // Page with cursor from NULL joined_at ("150")
        let cur_null =
            encode_member_search_cursor(page_after_del[0].joined_at, &page_after_del[0].user_id);
        assert!(cur_null.starts_with("null_"));
        let page_last = store
            .search_members(
                guild_id,
                &MemberSearchFilter {
                    limit: Some(2),
                    cursor: Some(cur_null),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(page_last.len(), 1);
        assert_eq!(page_last[0].user_id, "250");
    }

    #[tokio::test]
    async fn test_card_trade_validation_and_guild_isolation() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_1 = "guild_1";
        let guild_2 = "guild_2";
        let user_a = "user_a";
        let user_b = "user_b";

        let card_a = store
            .grant_user_card(guild_1, user_a, "card_1", "Card 1", "Common")
            .await
            .unwrap();
        let card_b = store
            .grant_user_card(guild_1, user_b, "card_2", "Card 2", "Rare")
            .await
            .unwrap();

        // 1. Cannot trade card with itself
        let self_trade = store
            .propose_card_trade(guild_1, user_a, card_a.id, user_b, card_a.id)
            .await;
        assert!(self_trade.is_err());

        // 2. Cross-guild trade rejection
        let normal_trade = store
            .propose_card_trade(guild_1, user_a, card_a.id, user_b, card_b.id)
            .await
            .unwrap();
        let wrong_guild = store
            .accept_card_trade(guild_2, normal_trade.id, user_b)
            .await;
        assert!(wrong_guild.is_err());

        // 3. Normal trade succeeds
        let success = store
            .accept_card_trade(guild_1, normal_trade.id, user_b)
            .await
            .unwrap();
        assert!(success);

        // 4. Cannot re-accept already completed trade
        let reaccept = store
            .accept_card_trade(guild_1, normal_trade.id, user_b)
            .await;
        assert!(reaccept.is_err());
    }

    #[tokio::test]
    async fn test_ai_guild_budget_lifecycle_and_idempotent_reads() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_1 = "guild_ai_1";
        let guild_2 = "guild_ai_2";

        // 1. Initial read creates default budget
        let budget_1 = store
            .get_or_create_ai_budget(guild_1)
            .await
            .expect("initial get_or_create_ai_budget");
        assert_eq!(budget_1.guild_id, guild_1);
        assert_eq!(budget_1.monthly_token_limit, 1_000_000);
        assert_eq!(budget_1.used_tokens, 0);
        assert_eq!(budget_1.monthly_usd_limit, 10.0);
        assert_eq!(budget_1.used_usd, 0.0);
        assert!(!budget_1.is_halted);

        // 2. Repeated read returns identical values without mutation
        for _ in 0..10 {
            let repeated = store
                .get_or_create_ai_budget(guild_1)
                .await
                .expect("repeated get_or_create_ai_budget");
            assert_eq!(repeated.used_tokens, 0);
            assert_eq!(repeated.used_usd, 0.0);
            assert_eq!(repeated.reset_at, budget_1.reset_at);
        }

        // 3. Record AI usage
        let recorded = store
            .record_ai_usage(guild_1, 2500, 0.08)
            .await
            .expect("record_ai_usage");
        assert_eq!(recorded.used_tokens, 2500);
        assert_eq!(recorded.used_usd, 0.08);

        // 4. Repeated read after usage preserves recorded usage without resetting
        let after_usage = store
            .get_or_create_ai_budget(guild_1)
            .await
            .expect("get_or_create_ai_budget after usage");
        assert_eq!(after_usage.used_tokens, 2500);
        assert_eq!(after_usage.used_usd, 0.08);
        assert_eq!(after_usage.reset_at, budget_1.reset_at);

        // 5. Guild isolation
        let budget_2 = store
            .get_or_create_ai_budget(guild_2)
            .await
            .expect("guild 2 budget");
        assert_eq!(budget_2.guild_id, guild_2);
        assert_eq!(budget_2.used_tokens, 0);
        assert_eq!(budget_2.used_usd, 0.0);

        // 6. Negative values are rejected
        assert!(store.record_ai_usage(guild_1, -10, 0.0).await.is_err());
        assert!(store.record_ai_usage(guild_1, 10, -0.5).await.is_err());
    }

    #[tokio::test]
    async fn test_tupper_profile_prefix_uniqueness_and_concurrency() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_1 = "tupper_guild_1";
        let guild_2 = "tupper_guild_2";
        let user_a = "user_a";
        let user_b = "user_b";

        // 1. Initial creation succeeds
        let t1 = store
            .create_tupper_profile(guild_1, user_a, "Hero 1", None, "h:")
            .await
            .expect("initial tupper creation");
        assert_eq!(t1.name, "Hero 1");
        assert_eq!(t1.prefix_or_brackets, "h:");

        // 2. Duplicate prefix for same user and guild returns PrefixConflict
        let dup = store
            .create_tupper_profile(guild_1, user_a, "Hero 2", None, "h:")
            .await;
        assert!(matches!(dup, Err(TupperError::PrefixConflict)));

        // 3. Same prefix for distinct user in same guild succeeds
        let t_user_b = store
            .create_tupper_profile(guild_1, user_b, "Hero B", None, "h:")
            .await
            .expect("same prefix for distinct user must succeed");
        assert_eq!(t_user_b.user_id, user_b);

        // 4. Same prefix for same user in distinct guild succeeds
        let t_guild_2 = store
            .create_tupper_profile(guild_2, user_a, "Hero Guild 2", None, "h:")
            .await
            .expect("same prefix in distinct guild must succeed");
        assert_eq!(t_guild_2.guild_id, guild_2);

        // 5. Concurrent creation: 20 parallel tasks attempting to register the same prefix
        let concurrent_user = "user_concurrent";
        let mut handles = Vec::new();
        for i in 0..20 {
            let store_clone = store.clone();
            handles.push(tokio::spawn(async move {
                store_clone
                    .create_tupper_profile(
                        guild_1,
                        concurrent_user,
                        &format!("Concurrent Hero {i}"),
                        None,
                        "c:",
                    )
                    .await
            }));
        }

        let mut successes = 0;
        let mut conflicts = 0;
        for handle in handles {
            match handle.await.unwrap() {
                Ok(_) => successes += 1,
                Err(TupperError::PrefixConflict) => conflicts += 1,
                Err(e) => panic!("Unexpected error: {e:?}"),
            }
        }
        assert_eq!(successes, 1, "Exactly one concurrent attempt must succeed");
        assert_eq!(
            conflicts, 19,
            "Exactly 19 concurrent attempts must fail with PrefixConflict"
        );

        let list = store
            .list_tupper_profiles(guild_1, concurrent_user)
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn test_event_roster_monotonic_positions_and_fifo_promotion() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_id = "event_guild_1";

        let event = store
            .create_scheduled_event(
                guild_id,
                "Raid Night",
                "Description",
                Utc::now(),
                None,
                "UTC",
                Some(1), // capacity = 1
            )
            .await
            .expect("create scheduled event");

        // 1. A joins -> main, pos 1
        let a = store.join_event_roster(event.id, "user_a").await.unwrap();
        assert_eq!(a.role_type, "main");
        assert_eq!(a.position, 1);

        // 2. B joins -> reserve, pos 2
        let b = store.join_event_roster(event.id, "user_b").await.unwrap();
        assert_eq!(b.role_type, "reserve");
        assert_eq!(b.position, 2);

        // 3. C joins -> reserve, pos 3
        let c = store.join_event_roster(event.id, "user_c").await.unwrap();
        assert_eq!(c.role_type, "reserve");
        assert_eq!(c.position, 3);

        // 4. B leaves -> pos 2 deleted, remaining: A[1], C[3]
        let left_b = store.leave_event_roster(event.id, "user_b").await.unwrap();
        assert_eq!(left_b, None); // no promotion because B was reserve

        // 5. D joins -> MUST receive pos 4, NOT pos 3 (Issue #155)
        let d = store.join_event_roster(event.id, "user_d").await.unwrap();
        assert_eq!(d.role_type, "reserve");
        assert_eq!(d.position, 4, "New position must be MAX(position) + 1 = 4");

        // Verify active positions: 1, 3, 4 without duplicates
        let active = store.get_event_rosters(event.id).await.unwrap();
        let positions: Vec<i32> = active.iter().map(|r| r.position).collect();
        assert_eq!(positions, vec![1, 3, 4]);

        // 6. A leaves (main) -> C (reserve pos 3) MUST be promoted over D (reserve pos 4)
        let promoted = store.leave_event_roster(event.id, "user_a").await.unwrap();
        assert_eq!(
            promoted,
            Some("user_c".to_string()),
            "Oldest reserve C must be promoted"
        );

        let after_leave = store.get_event_rosters(event.id).await.unwrap();
        assert_eq!(after_leave.len(), 2);
        let c_active = after_leave.iter().find(|r| r.user_id == "user_c").unwrap();
        assert_eq!(c_active.role_type, "main");
        assert_eq!(c_active.position, 3);

        let d_active = after_leave.iter().find(|r| r.user_id == "user_d").unwrap();
        assert_eq!(d_active.role_type, "reserve");
        assert_eq!(d_active.position, 4);
    }

    #[tokio::test]
    async fn test_event_roster_stress_join_leave_no_duplicate_positions() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_id = "event_stress_guild";

        let event = store
            .create_scheduled_event(
                guild_id,
                "Stress Event",
                "Description",
                Utc::now(),
                None,
                "UTC",
                Some(5),
            )
            .await
            .unwrap();

        // Perform 100 alternating join and leave operations
        for i in 0..100 {
            let u_id = format!("user_{i}");
            store.join_event_roster(event.id, &u_id).await.unwrap();

            // Periodically drop a previous participant to create gaps in position sequences
            if i % 3 == 0 && i >= 3 {
                let drop_user = format!("user_{}", i - 2);
                let _ = store.leave_event_roster(event.id, &drop_user).await;
            }
        }

        let roster = store.get_event_rosters(event.id).await.unwrap();
        assert!(!roster.is_empty());

        // Verify that in all remaining participants, every position is unique!
        let mut seen_positions = std::collections::HashSet::new();
        for r in &roster {
            assert!(
                seen_positions.insert(r.position),
                "Duplicate position {} found in event roster",
                r.position
            );
        }
    }

    #[tokio::test]
    async fn test_cleanup_plan_lifecycle_isolation_and_atomic_consumption() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_1 = "100000000000000001";
        let guild_2 = "100000000000000002";
        let channel_id = "200000000000000001";
        let actor_1 = "300000000000000001";
        let actor_2 = "300000000000000002";
        let msgs = vec![
            "400000000000000001".to_string(),
            "400000000000000002".to_string(),
        ];

        // 1. Create plan with 60s TTL
        let plan = store
            .create_cleanup_plan(guild_1, channel_id, actor_1, &msgs, 60)
            .await
            .expect("create cleanup plan");
        assert_eq!(plan.guild_id, guild_1);
        assert_eq!(plan.channel_id, channel_id);
        assert_eq!(plan.actor_user_id, actor_1);
        assert_eq!(plan.message_ids_vec(), msgs);
        assert!(plan.consumed_at.is_none());

        // 2. Cross-guild get returns None
        let g2_get = store.get_cleanup_plan(guild_2, plan.id).await.unwrap();
        assert!(g2_get.is_none());

        // 3. Confirm by another actor in same guild fails
        let wrong_actor = store
            .consume_cleanup_plan(guild_1, plan.id, actor_2)
            .await
            .unwrap();
        assert!(
            wrong_actor.is_none(),
            "Actor mismatch must fail consumption"
        );

        // 4. Confirm in another guild fails
        let wrong_guild = store
            .consume_cleanup_plan(guild_2, plan.id, actor_1)
            .await
            .unwrap();
        assert!(
            wrong_guild.is_none(),
            "Guild mismatch must fail consumption"
        );

        // 5. Correct actor and guild succeeds
        let consumed = store
            .consume_cleanup_plan(guild_1, plan.id, actor_1)
            .await
            .unwrap()
            .expect("consumption should succeed");
        assert!(consumed.consumed_at.is_some());

        // 6. Second confirm fails (idempotent atomic consumption)
        let second_consume = store
            .consume_cleanup_plan(guild_1, plan.id, actor_1)
            .await
            .unwrap();
        assert!(
            second_consume.is_none(),
            "Already consumed plan must not be consumed again"
        );

        // 7. Expired plan test
        let expired_plan = store
            .create_cleanup_plan(guild_1, channel_id, actor_1, &msgs, -10)
            .await
            .unwrap();
        let consume_expired = store
            .consume_cleanup_plan(guild_1, expired_plan.id, actor_1)
            .await
            .unwrap();
        assert!(
            consume_expired.is_none(),
            "Expired plan must not be consumed"
        );
    }

    #[tokio::test]
    async fn test_companion_channel_mapping_and_lease_lifecycle() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_id = "111000000000000001";
        let voice_id = "222000000000000001";
        let text_id = "333000000000000001";
        let user_id = "444000000000000001";
        let generation = Uuid::new_v4();

        // 1. Initial listing is empty
        let initial_mappings = store.list_companion_mappings(guild_id).await.unwrap();
        assert!(initial_mappings.is_empty());

        // 2. Create mapping
        let created = store
            .upsert_companion_mapping(guild_id, voice_id, text_id, true, false)
            .await
            .unwrap();
        assert_eq!(created.guild_id, guild_id);
        assert_eq!(created.voice_channel_id, voice_id);
        assert_eq!(created.text_channel_id, text_id);
        assert!(created.allow_send_messages);
        assert!(!created.allow_read_history);

        // 3. Get mapping by voice
        let by_voice = store
            .get_companion_mapping_by_voice(guild_id, voice_id)
            .await
            .unwrap()
            .expect("mapping should exist");
        assert_eq!(by_voice.id, created.id);

        // 4. Overwrite mapping for same voice channel
        let updated_text = "333000000000000002";
        let updated = store
            .upsert_companion_mapping(guild_id, voice_id, updated_text, true, true)
            .await
            .unwrap();
        assert_eq!(updated.text_channel_id, updated_text);
        assert!(updated.allow_read_history);

        let list_after_update = store.list_companion_mappings(guild_id).await.unwrap();
        assert_eq!(list_after_update.len(), 1);
        assert_eq!(list_after_update[0].text_channel_id, updated_text);

        // 5. Leases lifecycle
        let no_lease = store
            .get_companion_lease(guild_id, updated_text, user_id)
            .await
            .unwrap();
        assert!(no_lease.is_none());

        let lease = store
            .record_companion_lease(RecordCompanionLeaseParams {
                guild_id,
                voice_channel_id: voice_id,
                text_channel_id: updated_text,
                user_id,
                generation,
                allow_bits: 1024,
                deny_bits: 0,
            })
            .await
            .unwrap();
        assert_eq!(lease.guild_id, guild_id);
        assert_eq!(lease.generation, generation);
        assert_eq!(lease.status, "active");
        assert_eq!(lease.allow_bits, 1024);

        // Test outbox lifecycle: mark revoke_pending
        let pending = store
            .mark_companion_lease_revoke_pending(guild_id, updated_text, user_id)
            .await
            .unwrap()
            .expect("lease should exist");
        assert_eq!(pending.status, "revoke_pending");

        // Failure records retry
        store
            .record_companion_lease_failure(
                guild_id,
                updated_text,
                user_id,
                "Discord 500 Internal Error",
            )
            .await
            .unwrap();

        let failed_lease = store
            .get_companion_lease(guild_id, updated_text, user_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(failed_lease.revoke_attempts, 1);
        assert!(failed_lease.last_error.unwrap().contains("Discord 500"));
        assert!(failed_lease.next_attempt_at.is_some());

        let fetched_lease = store
            .get_companion_lease(guild_id, updated_text, user_id)
            .await
            .unwrap()
            .expect("lease must exist");
        assert_eq!(fetched_lease.generation, generation);

        let active_leases = store.list_active_companion_leases(guild_id).await.unwrap();
        assert_eq!(active_leases.len(), 0);

        let pending_leases = store
            .list_pending_revoke_companion_leases(guild_id)
            .await
            .unwrap();
        assert_eq!(pending_leases.len(), 1);

        // 6. Delete lease
        let deleted_lease = store
            .delete_companion_lease(guild_id, updated_text, user_id)
            .await
            .unwrap()
            .expect("deleted lease returned");
        assert_eq!(deleted_lease.user_id, user_id);

        let active_after_del = store.list_active_companion_leases(guild_id).await.unwrap();
        assert!(active_after_del.is_empty());

        // 6b. Test Issue #175: Updating mapping target invalidates active leases for that voice channel
        let new_gen = Uuid::new_v4();
        store
            .record_companion_lease(RecordCompanionLeaseParams {
                guild_id,
                voice_channel_id: voice_id,
                text_channel_id: updated_text,
                user_id,
                generation: new_gen,
                allow_bits: 1024,
                deny_bits: 0,
            })
            .await
            .unwrap();

        let third_text = "333000000000000003";
        let third = store
            .upsert_companion_mapping(guild_id, voice_id, third_text, true, true)
            .await
            .unwrap();
        assert_eq!(
            third.id, created.id,
            "mapping ID must remain stable across updates"
        );
        assert_eq!(third.text_channel_id, third_text);
        assert!(third.enabled);

        let invalidated = store
            .get_companion_lease(guild_id, updated_text, user_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            invalidated.status, "revoke_pending",
            "updating mapping target must mark old lease revoke_pending (#175)"
        );

        let _ = store
            .delete_companion_lease(guild_id, updated_text, user_id)
            .await
            .unwrap();

        // 7. Delete mapping
        let deleted_mapping = store
            .delete_companion_mapping(guild_id, updated.id)
            .await
            .unwrap();
        assert!(deleted_mapping);
        assert!(
            store
                .list_companion_mappings(guild_id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn test_companion_channel_multi_guild_isolation_and_erasure() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_1 = "111000000000000001";
        let guild_2 = "111000000000000002";
        let voice_1 = "222000000000000001";
        let text_1 = "333000000000000001";
        let user_1 = "444000000000000001";

        let m1 = store
            .upsert_companion_mapping(guild_1, voice_1, text_1, true, true)
            .await
            .unwrap();
        let _l1 = store
            .record_companion_lease(RecordCompanionLeaseParams {
                guild_id: guild_1,
                voice_channel_id: voice_1,
                text_channel_id: text_1,
                user_id: user_1,
                generation: Uuid::new_v4(),
                allow_bits: 1024,
                deny_bits: 0,
            })
            .await
            .unwrap();

        // Guild 2 has empty mappings and leases
        assert!(
            store
                .list_companion_mappings(guild_2)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .list_active_companion_leases(guild_2)
                .await
                .unwrap()
                .is_empty()
        );

        // Cross-guild deletion is rejected
        let del_cross = store
            .delete_companion_mapping(guild_2, m1.id)
            .await
            .unwrap();
        assert!(!del_cross);
        assert_eq!(
            store.list_companion_mappings(guild_1).await.unwrap().len(),
            1
        );

        // Erasure of user in guild 1 removes their lease
        let erasure = store.erase_guild_user_data(guild_1, user_1).await.unwrap();
        assert!(erasure.related_records_erased > 0);
        assert!(
            store
                .list_active_companion_leases(guild_1)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn test_remove_member_snapshot_departure_and_isolation() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_1 = "111000000000000001";
        let guild_2 = "111000000000000002";
        let user_id = "444000000000000001";

        // Add member snapshot in guild 1 and guild 2
        store
            .upsert_member_snapshot(UpsertMemberSnapshotParams {
                guild_id: guild_1,
                user_id,
                username: "user444",
                global_name: Some("User Four"),
                nickname: Some("Four"),
                role_ids: &[],
                joined_at: Some(Utc::now()),
            })
            .await
            .unwrap();

        store
            .upsert_member_snapshot(UpsertMemberSnapshotParams {
                guild_id: guild_2,
                user_id,
                username: "user444",
                global_name: Some("User Four"),
                nickname: None,
                role_ids: &[],
                joined_at: Some(Utc::now()),
            })
            .await
            .unwrap();

        // Search in guild 1 finds the user
        let results = store
            .search_members(guild_1, &MemberSearchFilter::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].user_id, user_id);

        // Remove snapshot from guild 1 upon departure
        let removed = store
            .remove_member_snapshot(guild_1, user_id)
            .await
            .unwrap();
        assert!(removed);

        // Second removal is idempotent
        let removed_again = store
            .remove_member_snapshot(guild_1, user_id)
            .await
            .unwrap();
        assert!(!removed_again);

        // Search in guild 1 no longer returns the user
        let results_after = store
            .search_members(guild_1, &MemberSearchFilter::default())
            .await
            .unwrap();
        assert!(results_after.is_empty());

        // Snapshot in guild 2 remains intact
        let results_g2 = store
            .search_members(guild_2, &MemberSearchFilter::default())
            .await
            .unwrap();
        assert_eq!(results_g2.len(), 1);
    }

    #[tokio::test]
    async fn test_command_policy_storage_and_revision_lifecycle() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_1 = "100000000000000001";
        let guild_2 = "200000000000000002";
        let user_id = "300000000000000003";

        // Initial revision is 1
        assert_eq!(store.get_policy_revision(guild_1).await.unwrap(), 1);
        assert_eq!(store.get_policy_revision(guild_2).await.unwrap(), 1);

        let rule1 = CommandPolicyRule::new_user_rule(
            guild_1.to_string(),
            user_id.to_string(),
            crate::policy::PolicyResource::Command {
                module_id: "music".to_string(),
                command_name: "play".to_string(),
            },
            crate::policy::PolicyEffect::Deny,
            10,
            "admin".to_string(),
        )
        .unwrap();

        let rule2 = CommandPolicyRule::new_guild_rule(
            guild_1.to_string(),
            crate::policy::PolicyResource::Module {
                module_id: "music".to_string(),
            },
            crate::policy::PolicyEffect::Allow,
            0,
            "admin".to_string(),
        )
        .unwrap();

        // Upsert rule 1
        let saved1 = store.upsert_policy_rule(rule1.clone()).await.unwrap();
        assert_eq!(saved1.id, rule1.id);
        let rev = store.get_policy_revision(guild_1).await.unwrap();
        assert!(rev >= 2);

        // Upsert rule 2
        store.upsert_policy_rule(rule2.clone()).await.unwrap();

        // List rules: ordered by priority DESC (rule1 with priority 10 before rule2 with priority 0)
        let rules_g1 = store.list_policy_rules(guild_1).await.unwrap();
        assert_eq!(rules_g1.len(), 2);
        assert_eq!(rules_g1[0].id, rule1.id);
        assert_eq!(rules_g1[1].id, rule2.id);

        // Guild 2 has 0 rules (isolation)
        let rules_g2 = store.list_policy_rules(guild_2).await.unwrap();
        assert!(rules_g2.is_empty());

        // Delete rule 1
        let deleted = store.delete_policy_rule(guild_1, rule1.id).await.unwrap();
        assert!(deleted);
        let rules_after_del = store.list_policy_rules(guild_1).await.unwrap();
        assert_eq!(rules_after_del.len(), 1);
        assert_eq!(rules_after_del[0].id, rule2.id);

        // Reset policies
        let reset_count = store.reset_policy_rules(guild_1).await.unwrap();
        assert_eq!(reset_count, 1);
        let rules_after_reset = store.list_policy_rules(guild_1).await.unwrap();
        assert!(rules_after_reset.is_empty());
    }

    #[tokio::test]
    async fn test_command_policy_cross_guild_upsert_isolation() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_a = "100000000000000001";
        let guild_b = "200000000000000002";
        let user_id = "300000000000000003";

        let rule_a = CommandPolicyRule::new_user_rule(
            guild_a.to_string(),
            user_id.to_string(),
            crate::policy::PolicyResource::Command {
                module_id: "music".to_string(),
                command_name: "play".to_string(),
            },
            crate::policy::PolicyEffect::Deny,
            10,
            "admin_a".to_string(),
        )
        .unwrap();

        let saved_a = store.upsert_policy_rule(rule_a.clone()).await.unwrap();
        let rev_a_before = store.get_policy_revision(guild_a).await.unwrap();
        let rev_b_before = store.get_policy_revision(guild_b).await.unwrap();

        // Admin B attempts to upsert a rule with UUID belonging to guild A
        let mut malicious_b = rule_a.clone();
        malicious_b.guild_id = guild_b.to_string();
        malicious_b.created_by = "admin_b".to_string();
        malicious_b.effect = "allow".to_string();

        let res = store.upsert_policy_rule(malicious_b).await;
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("belongs to another guild")
        );

        // Guild A's rule remains intact
        let rules_a = store.list_policy_rules(guild_a).await.unwrap();
        assert_eq!(rules_a.len(), 1);
        assert_eq!(rules_a[0].id, saved_a.id);
        assert_eq!(rules_a[0].effect, "deny");
        assert_eq!(rules_a[0].guild_id, guild_a);

        // Neither revision bumped
        assert_eq!(
            store.get_policy_revision(guild_a).await.unwrap(),
            rev_a_before
        );
        assert_eq!(
            store.get_policy_revision(guild_b).await.unwrap(),
            rev_b_before
        );
    }

    #[tokio::test]
    async fn test_command_policy_replace_atomic_and_validation() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_1 = "100000000000000001";
        let user_id = "300000000000000003";

        let initial_rule = CommandPolicyRule::new_user_rule(
            guild_1.to_string(),
            user_id.to_string(),
            crate::policy::PolicyResource::Command {
                module_id: "music".to_string(),
                command_name: "stop".to_string(),
            },
            crate::policy::PolicyEffect::Deny,
            5,
            "admin".to_string(),
        )
        .unwrap();

        store
            .upsert_policy_rule(initial_rule.clone())
            .await
            .unwrap();
        let rev_before = store.get_policy_revision(guild_1).await.unwrap();

        // 1. Invalid rule in batch fails validation; existing rules preserved
        let mut invalid_rule = initial_rule.clone();
        invalid_rule.id = uuid::Uuid::new_v4();
        invalid_rule.effect = "invalid_effect".to_string();

        let res = store
            .replace_policy_rules(guild_1, Some(rev_before), vec![invalid_rule])
            .await;
        assert!(res.is_err());
        let rules = store.list_policy_rules(guild_1).await.unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, initial_rule.id);

        // 2. Stale expected_revision fails with conflict; existing rules preserved
        let valid_new_rule = CommandPolicyRule::new_guild_rule(
            guild_1.to_string(),
            crate::policy::PolicyResource::Module {
                module_id: "moderation".to_string(),
            },
            crate::policy::PolicyEffect::Allow,
            0,
            "admin".to_string(),
        )
        .unwrap();

        let res_stale = store
            .replace_policy_rules(guild_1, Some(rev_before + 99), vec![valid_new_rule.clone()])
            .await;
        assert!(res_stale.is_err());
        assert!(
            res_stale
                .unwrap_err()
                .to_string()
                .contains("Revision conflict")
        );

        let rules_after_conflict = store.list_policy_rules(guild_1).await.unwrap();
        assert_eq!(rules_after_conflict.len(), 1);
        assert_eq!(rules_after_conflict[0].id, initial_rule.id);

        // 3. Successful atomic replace with correct expected_revision
        let replaced = store
            .replace_policy_rules(guild_1, Some(rev_before), vec![valid_new_rule.clone()])
            .await
            .unwrap();
        assert_eq!(replaced.len(), 1);
        assert_eq!(replaced[0].id, valid_new_rule.id);

        let rules_final = store.list_policy_rules(guild_1).await.unwrap();
        assert_eq!(rules_final.len(), 1);
        assert_eq!(rules_final[0].id, valid_new_rule.id);
        assert_eq!(
            store.get_policy_revision(guild_1).await.unwrap(),
            rev_before + 1
        );
    }

    #[tokio::test]
    async fn test_moderator_duty_storage_lifecycle_and_isolation() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_1 = "100000000000000001";
        let guild_2 = "200000000000000002";
        let user_id = "300000000000000003";

        // Initial settings is None
        assert!(store.get_duty_settings(guild_1).await.unwrap().is_none());

        // Upsert settings
        let settings = ModeratorDutySettings {
            guild_id: guild_1.to_string(),
            staff_role_id: Some("400000000000000001".to_string()),
            on_call_role_id: Some("500000000000000001".to_string()),
            max_override_duration_mins: 720,
            fallback_channel_id: Some("600000000000000001".to_string()),
            updated_at: Utc::now(),
        };
        let saved_settings = store.upsert_duty_settings(settings).await.unwrap();
        assert_eq!(saved_settings.max_override_duration_mins, 720);

        // Settings in G2 remains None (isolation)
        assert!(store.get_duty_settings(guild_2).await.unwrap().is_none());

        // Upsert profile
        let profile = ModeratorDutyProfile::new(
            guild_1.to_string(),
            user_id.to_string(),
            "Europe/Berlin".to_string(),
            RecurringSchedule::default(),
        )
        .unwrap();
        let saved_profile = store.upsert_duty_profile(profile).await.unwrap();
        assert_eq!(saved_profile.version, 1);

        // Fetch profile
        let fetched = store
            .get_duty_profile(guild_1, user_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.timezone, "Europe/Berlin");

        // G2 has 0 profiles
        assert!(store.list_duty_profiles(guild_2).await.unwrap().is_empty());

        // Upsert state
        let state = ModeratorDutyState {
            guild_id: guild_1.to_string(),
            user_id: user_id.to_string(),
            expected_on_call: true,
            state_generation: 0,
            last_reconciled_at: Some(Utc::now()),
            last_error: None,
        };
        let saved_state = store.upsert_duty_state(state).await.unwrap();
        assert_eq!(saved_state.state_generation, 1);
        assert!(saved_state.expected_on_call);

        // Delete profile
        let deleted = store.delete_duty_profile(guild_1, user_id).await.unwrap();
        assert!(deleted);
        assert!(
            store
                .get_duty_profile(guild_1, user_id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .get_duty_state(guild_1, user_id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_memorystore_upsert_parity_github_twitch_kick_youtube() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_id = "100000000000000001";

        // 1. GitHub DevOps mapping UPSERT parity
        let id1 = Uuid::new_v4();
        let created_at1 = Utc::now() - chrono::Duration::hours(2);
        let mapping1 = GitHubDevopsMapping {
            id: id1,
            guild_id: guild_id.to_string(),
            repo_full_name: "MyOrg/MyRepo".to_string(),
            thread_channel_id: "chan_1".to_string(),
            issue_or_pr_number: 42,
            discord_thread_id: "thread_A".to_string(),
            last_status: "opened".to_string(),
            created_at: created_at1,
        };
        let res1 = store.upsert_github_devops_mapping(mapping1).await.unwrap();
        assert_eq!(res1.id, id1);
        assert_eq!(res1.discord_thread_id, "thread_A");

        let id2 = Uuid::new_v4();
        let mapping2 = GitHubDevopsMapping {
            id: id2,
            guild_id: guild_id.to_string(),
            repo_full_name: "myorg/myrepo".to_string(), // case insensitive match
            thread_channel_id: "chan_2".to_string(),
            issue_or_pr_number: 42,
            discord_thread_id: "thread_B".to_string(),
            last_status: "closed".to_string(),
            created_at: Utc::now(),
        };
        let res2 = store.upsert_github_devops_mapping(mapping2).await.unwrap();
        // ID and created_at must be preserved from first insert
        assert_eq!(res2.id, id1);
        assert_eq!(res2.created_at, created_at1);
        assert_eq!(res2.discord_thread_id, "thread_B");
        assert_eq!(res2.last_status, "closed");

        let fetched_github = store
            .get_github_devops_mapping(guild_id, "MYORG/MYREPO", 42)
            .await
            .unwrap()
            .expect("should find updated mapping");
        assert_eq!(fetched_github.id, id1);
        assert_eq!(fetched_github.discord_thread_id, "thread_B");

        // 2. Twitch Subscription UPSERT parity
        let twitch_id1 = Uuid::new_v4();
        let twitch_created1 = Utc::now() - chrono::Duration::hours(1);
        let twitch1 = TwitchStreamSubscription {
            id: twitch_id1,
            guild_id: guild_id.to_string(),
            streamer_login: "GamerPro".to_string(),
            streamer_id: "twitch_101".to_string(),
            discord_channel_id: "chan_twitch".to_string(),
            live_role_id: None,
            last_message_id: None,
            is_live: false,
            last_title: None,
            last_game: None,
            created_at: twitch_created1,
        };
        let t_res1 = store.upsert_twitch_subscription(twitch1).await.unwrap();
        assert_eq!(t_res1.id, twitch_id1);

        let twitch2 = TwitchStreamSubscription {
            id: Uuid::new_v4(),
            guild_id: guild_id.to_string(),
            streamer_login: "gamerpro".to_string(),
            streamer_id: "twitch_101".to_string(),
            discord_channel_id: "chan_twitch_2".to_string(),
            live_role_id: Some("role_live".to_string()),
            last_message_id: Some("msg_999".to_string()),
            is_live: true,
            last_title: Some("Streaming Rust Bot".to_string()),
            last_game: Some("Rust".to_string()),
            created_at: Utc::now(),
        };
        let t_res2 = store.upsert_twitch_subscription(twitch2).await.unwrap();
        assert_eq!(t_res2.id, twitch_id1);
        assert_eq!(t_res2.created_at, twitch_created1);
        assert!(t_res2.is_live);
        assert_eq!(t_res2.last_title.as_deref(), Some("Streaming Rust Bot"));

        let fetched_twitch = store
            .get_twitch_subscription(guild_id, "GAMERPRO")
            .await
            .unwrap()
            .expect("should find twitch sub");
        assert_eq!(fetched_twitch.id, twitch_id1);
        assert!(fetched_twitch.is_live);

        // 3. Kick Subscription UPSERT parity
        let kick_id1 = Uuid::new_v4();
        let kick_created1 = Utc::now() - chrono::Duration::hours(1);
        let kick1 = KickStreamSubscription {
            id: kick_id1,
            guild_id: guild_id.to_string(),
            slug: "FastPlayer".to_string(),
            channel_id: "kick_chan_1".to_string(),
            discord_channel_id: "disc_chan_1".to_string(),
            live_role_id: None,
            last_message_id: None,
            is_live: false,
            created_at: kick_created1,
        };
        let k_res1 = store.upsert_kick_subscription(kick1).await.unwrap();
        assert_eq!(k_res1.id, kick_id1);

        let kick2 = KickStreamSubscription {
            id: Uuid::new_v4(),
            guild_id: guild_id.to_string(),
            slug: "fastplayer".to_string(),
            channel_id: "kick_chan_1".to_string(),
            discord_channel_id: "disc_chan_2".to_string(),
            live_role_id: Some("role_kick".to_string()),
            last_message_id: Some("msg_kick_1".to_string()),
            is_live: true,
            created_at: Utc::now(),
        };
        let k_res2 = store.upsert_kick_subscription(kick2).await.unwrap();
        assert_eq!(k_res2.id, kick_id1);
        assert_eq!(k_res2.created_at, kick_created1);
        assert!(k_res2.is_live);

        let fetched_kick = store
            .get_kick_subscription(guild_id, "FASTPLAYER")
            .await
            .unwrap()
            .expect("should find kick sub");
        assert_eq!(fetched_kick.id, kick_id1);
        assert!(fetched_kick.is_live);

        // 4. YouTube Subscription UPSERT parity
        let yt_id1 = Uuid::new_v4();
        let yt_created1 = Utc::now() - chrono::Duration::hours(1);
        let yt1 = YouTubeChannelSubscription {
            id: yt_id1,
            guild_id: guild_id.to_string(),
            channel_id: "UC_CHANNEL_123".to_string(),
            channel_title: "Tech Channel".to_string(),
            discord_channel_id: "disc_yt_1".to_string(),
            last_video_id: None,
            created_at: yt_created1,
        };
        let yt_res1 = store.upsert_youtube_subscription(yt1).await.unwrap();
        assert_eq!(yt_res1.id, yt_id1);

        let yt2 = YouTubeChannelSubscription {
            id: Uuid::new_v4(),
            guild_id: guild_id.to_string(),
            channel_id: "UC_CHANNEL_123".to_string(),
            channel_title: "Tech Channel Updated".to_string(),
            discord_channel_id: "disc_yt_2".to_string(),
            last_video_id: Some("vid_abc".to_string()),
            created_at: Utc::now(),
        };
        let yt_res2 = store.upsert_youtube_subscription(yt2).await.unwrap();
        assert_eq!(yt_res2.id, yt_id1);
        assert_eq!(yt_res2.created_at, yt_created1);
        assert_eq!(yt_res2.last_video_id.as_deref(), Some("vid_abc"));

        let fetched_yt = store
            .get_youtube_subscription(guild_id, "UC_CHANNEL_123")
            .await
            .unwrap()
            .expect("should find youtube sub");
        assert_eq!(fetched_yt.id, yt_id1);
        assert_eq!(fetched_yt.channel_title, "Tech Channel Updated");
    }

    #[tokio::test]
    async fn test_batch_upsert_companion_mappings_quota_and_atomicity() {
        let store = Storage::Memory(MemoryStore::default());
        let guild_id = "100000000000000001";

        // 1. Batch of 51 mappings must fail and insert NOTHING (atomicity + quota)
        let mut over_limit_items = Vec::new();
        for i in 1..=51 {
            over_limit_items.push(CompanionMappingItem {
                voice_channel_id: format!("1000000000000000{:02}", i),
                text_channel_id: format!("2000000000000000{:02}", i),
                allow_send_messages: true,
                allow_read_history: true,
            });
        }
        let res_over = store
            .batch_upsert_companion_mappings(guild_id, &over_limit_items)
            .await;
        assert!(res_over.is_err());
        assert!(res_over.unwrap_err().to_string().contains("limit reached"));
        assert_eq!(
            store.list_companion_mappings(guild_id).await.unwrap().len(),
            0
        );

        // 2. Batch with 1 valid and 1 invalid item (voice == text) must fail with 0 inserts
        let mixed_items = vec![
            CompanionMappingItem {
                voice_channel_id: "100000000000000099".to_string(),
                text_channel_id: "200000000000000099".to_string(),
                allow_send_messages: true,
                allow_read_history: true,
            },
            CompanionMappingItem {
                voice_channel_id: "100000000000000088".to_string(),
                text_channel_id: "100000000000000088".to_string(), // invalid: same
                allow_send_messages: true,
                allow_read_history: true,
            },
        ];
        let res_mixed = store
            .batch_upsert_companion_mappings(guild_id, &mixed_items)
            .await;
        assert!(res_mixed.is_err());
        assert_eq!(
            store.list_companion_mappings(guild_id).await.unwrap().len(),
            0
        );

        // 3. Batch of exactly 50 mappings must succeed
        let exact_50_items: Vec<CompanionMappingItem> = over_limit_items[..50].to_vec();
        let res_50 = store
            .batch_upsert_companion_mappings(guild_id, &exact_50_items)
            .await
            .expect("50 mappings should succeed");
        assert_eq!(res_50.len(), 50);
        assert_eq!(
            store.list_companion_mappings(guild_id).await.unwrap().len(),
            50
        );

        // 4. Updating an existing voice channel within 50 mappings succeeds
        let update_item = vec![CompanionMappingItem {
            voice_channel_id: "100000000000000001".to_string(),
            text_channel_id: "200000000000000099".to_string(),
            allow_send_messages: false,
            allow_read_history: true,
        }];
        let res_update = store
            .batch_upsert_companion_mappings(guild_id, &update_item)
            .await
            .expect("updating existing mapping should succeed");
        assert_eq!(res_update.len(), 1);
        assert_eq!(
            store.list_companion_mappings(guild_id).await.unwrap().len(),
            50
        );

        // 5. Adding 1 new mapping when 50 exist must fail
        let new_item = vec![CompanionMappingItem {
            voice_channel_id: "100000000000000051".to_string(),
            text_channel_id: "200000000000000051".to_string(),
            allow_send_messages: true,
            allow_read_history: true,
        }];
        let res_51st = store
            .batch_upsert_companion_mappings(guild_id, &new_item)
            .await;
        assert!(res_51st.is_err());
        assert_eq!(
            store.list_companion_mappings(guild_id).await.unwrap().len(),
            50
        );

        // 6. Batch with duplicate voice_channel_id must be rejected (Issue #215)
        let dup_items = vec![
            CompanionMappingItem {
                voice_channel_id: "100000000000000077".to_string(),
                text_channel_id: "200000000000000077".to_string(),
                allow_send_messages: true,
                allow_read_history: true,
            },
            CompanionMappingItem {
                voice_channel_id: "100000000000000077".to_string(),
                text_channel_id: "200000000000000078".to_string(),
                allow_send_messages: true,
                allow_read_history: true,
            },
        ];
        let res_dup = store
            .batch_upsert_companion_mappings(guild_id, &dup_items)
            .await;
        assert!(res_dup.is_err());
        assert!(
            res_dup
                .unwrap_err()
                .to_string()
                .contains("Duplicate voice_channel_id")
        );

        // 7. Batch with >50 items all having same voice_channel_id must fail on length quota
        let mut dup_51_items = Vec::new();
        for i in 0..51 {
            dup_51_items.push(CompanionMappingItem {
                voice_channel_id: "100000000000000088".to_string(),
                text_channel_id: format!("20000000000000{:04}", i),
                allow_send_messages: true,
                allow_read_history: true,
            });
        }
        let res_dup_51 = store
            .batch_upsert_companion_mappings(guild_id, &dup_51_items)
            .await;
        assert!(res_dup_51.is_err());
        assert_eq!(
            store.list_companion_mappings(guild_id).await.unwrap().len(),
            50
        );
    }

    #[tokio::test]
    async fn test_check_and_record_event_dedupe_atomic_single_winner_and_case_insensitive() {
        let store = Storage::Memory(MemoryStore::default());

        // 1. Case-insensitivity check
        let first = store
            .check_and_record_event_dedupe("GitHub", "evt-12345")
            .await
            .expect("dedupe check");
        assert!(first);

        let second = store
            .check_and_record_event_dedupe("github", "evt-12345")
            .await
            .expect("dedupe check");
        assert!(
            !second,
            "Different case of provider must be recognized as duplicate"
        );

        let third = store
            .check_and_record_event_dedupe("GITHUB", "evt-12345")
            .await
            .expect("dedupe check");
        assert!(!third, "Uppercase provider must be recognized as duplicate");

        // 2. High-concurrency race condition test
        for round in 0..50 {
            let event_id = format!("round-event-{round}");
            let gate = std::sync::Arc::new(tokio::sync::Barrier::new(16));
            let mut handles = Vec::new();

            for thread_idx in 0..16 {
                let store = store.clone();
                let gate = gate.clone();
                let event_id = event_id.clone();
                let provider = if thread_idx % 2 == 0 {
                    "GitHub"
                } else {
                    "github"
                };

                handles.push(tokio::spawn(async move {
                    gate.wait().await;
                    store
                        .check_and_record_event_dedupe(provider, &event_id)
                        .await
                        .expect("check dedupe")
                }));
            }

            let mut true_count = 0;
            for h in handles {
                if h.await.unwrap() {
                    true_count += 1;
                }
            }

            assert_eq!(
                true_count, 1,
                "Exactly one worker must win deduplication for event {event_id}"
            );
        }
    }
}
