mod memory;

mod postgres;

mod progression;

use chrono::{DateTime, Utc};

use serde::{Deserialize, Serialize};

use serde_json::Value;

use uuid::Uuid;

pub use memory::MemoryStore;

pub use postgres::{PostgresStore, run_safe_migrations};

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

    pub created_at: DateTime<Utc>,
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

        appeal_id: Uuid,

        status: &str,

        staff_notes: Option<&str>,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => Ok(store.update_appeal_status(appeal_id, status, staff_notes)),

            Self::Postgres(store) => {
                store
                    .update_appeal_status(appeal_id, status, staff_notes)
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
    ) -> anyhow::Result<(StoreItem, i64)> {
        match self {
            Self::Memory(store) => store.purchase_store_item(guild_id, user_id, item_id),

            Self::Postgres(store) => store.purchase_store_item(guild_id, user_id, item_id).await,
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
}
