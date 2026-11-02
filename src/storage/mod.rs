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
            Self::Memory(store) => Ok(store.get_member_progress(guild_id, user_id)),
            Self::Postgres(store) => store.get_member_progress(guild_id, user_id).await,
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

    pub async fn delete_server_snapshot(&self, snapshot_id: Uuid) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => Ok(store.delete_server_snapshot(snapshot_id)),
            Self::Postgres(store) => store.delete_server_snapshot(snapshot_id).await,
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

    pub async fn delete_custom_automation(&self, automation_id: Uuid) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => Ok(store.delete_custom_automation(automation_id)),
            Self::Postgres(store) => store.delete_custom_automation(automation_id).await,
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
        poll_id: Uuid,
        option_idx: usize,
    ) -> anyhow::Result<Option<CommunityPoll>> {
        match self {
            Self::Memory(store) => Ok(store.vote_community_poll(poll_id, option_idx)),
            Self::Postgres(store) => store.vote_community_poll(poll_id, option_idx).await,
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
        max_main: i32,
    ) -> anyhow::Result<EventRoster> {
        match self {
            Self::Memory(store) => Ok(store.join_event_roster(event_id, user_id, max_main)),
            Self::Postgres(store) => store.join_event_roster(event_id, user_id, max_main).await,
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
        tournament_id: Uuid,
        match_idx: usize,
        winner: &str,
    ) -> anyhow::Result<Tournament> {
        match self {
            Self::Memory(store) => store.advance_tournament_match(tournament_id, match_idx, winner),
            Self::Postgres(store) => {
                store
                    .advance_tournament_match(tournament_id, match_idx, winner)
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
        channel_id: &str,
        is_empty: bool,
    ) -> anyhow::Result<()> {
        match self {
            Self::Memory(store) => {
                store.update_dynamic_voice_empty_state(channel_id, is_empty);
                Ok(())
            }
            Self::Postgres(store) => {
                store
                    .update_dynamic_voice_empty_state(channel_id, is_empty)
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
    ) -> anyhow::Result<TupperProfile> {
        match self {
            Self::Memory(store) => Ok(store.create_tupper_profile(
                guild_id,
                user_id,
                name,
                avatar_url,
                prefix_or_brackets,
            )),
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

    pub async fn delete_faq_entry(&self, faq_id: Uuid) -> anyhow::Result<bool> {
        match self {
            Self::Memory(store) => Ok(store.delete_faq_entry(faq_id)),
            Self::Postgres(store) => store.delete_faq_entry(faq_id).await,
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
}
