use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};

use serde_json::Value;

use sqlx::{PgPool, Postgres, Transaction, postgres::PgPoolOptions};

use uuid::Uuid;

use super::{
    AdjustMemberProgressParams, AiGuildBudget, AiKnowledgeDocument, AuditLogEntry, CardCollection,
    CardTrade, CleanupPlan, CoinTransfer, CoinTransferOutcome, CommandPolicyRule, CommunityPoll,
    CompanionChannelLease, CompanionChannelMapping, CompanionMappingItem, CountingGame,
    CrimeClaimOutcome, CustomAutomation, DailyClaimOutcome, DynamicVoiceRoom, EventRoster,
    FaqEntry, FeedSubscription, GameAccountLink, GamingHubGameStatus, GitHubDevopsMapping,
    GitLabSubscription, Giveaway, GuildModuleSettings, HelpForumConfig, HelpForumSession,
    HelpForumStats, IntegrationConnection, IntegrationDeliveryLog, IntegrationWebhookSubscription,
    InviteTracking, KickStreamSubscription, LinkedAccount, MAX_COMPANION_MAPPINGS_PER_GUILD,
    McpToolAuthorization, MemberIdentityRecord, MemberProgress, MemberSearchFilter,
    MemberSearchRow, MemberSnapshot, ModerationAppeal, ModerationExecutionIntent,
    ModeratorDutyProfile, ModeratorDutySettings, ModeratorDutyState, ModuleDraftConfig,
    PluginGuildInstallation, PluginMarketplacePackage, ProgressMetric, ProgressionSeason,
    ReactionRoleBinding, RecordCompanionLeaseParams, RecruitmentApplication, RecruitmentReview,
    RecruitmentRoleFulfillment, RecruitmentTrack, RedditSubscription, ReputationGrantOutcome,
    RetainedRecordNotice, ScheduledEvent, ServerSnapshot, SocialPublicationDraft, StarboardEntry,
    SteamLfgPost, StickyMessage, StoreItem, StorePurchase, TagRecord, TelegramBridge,
    TelegramMessageMapping, TimedRoleGrant, Tournament, TransitionApplicationParams, TupperError,
    TupperProfile, TwitchStreamSubscription, UpsertMemberSnapshotParams, UserErasureReport,
    VkSubscription, WORK_JOBS, WorkClaimOutcome, WorkflowDefinition, WorkflowRunExecution, XpAward,
    YouTubeChannelSubscription, level_for_xp, parse_member_search_cursor, snowflake_created_at,
    validate_snowflake,
};

pub async fn run_safe_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    // 1. Pre-migration check: If legacy moderation_cases (from 0001) exists with target_user_id column,

    //    rename it to legacy_0001_moderation_cases so 0005 DROP TABLE won't delete user data (#51).

    let _ = sqlx::query(
        r#"

        DO $$

        BEGIN

            IF EXISTS (

                SELECT 1 FROM information_schema.columns 

                WHERE table_name = 'moderation_cases' AND column_name = 'target_user_id'

            ) THEN

                ALTER TABLE moderation_cases RENAME TO legacy_0001_moderation_cases;

            END IF;

        END $$;

        "#,
    )
    .execute(pool)
    .await;

    // 2. Pre-migration checksum synchronization for applied migrations:

    //    If _sqlx_migrations exists, synchronize stored checksums with current migrator checksums

    //    to prevent VersionMismatch panics on existing databases (#59).

    let migrator = sqlx::migrate!("./migrations");

    let table_exists: bool = sqlx::query_scalar(

        "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = '_sqlx_migrations')",

    )

    .fetch_one(pool)

    .await

    .unwrap_or(false);

    if table_exists {
        for migration in migrator.iter() {
            let _ = sqlx::query("UPDATE _sqlx_migrations SET checksum = $1 WHERE version = $2")
                .bind(&*migration.checksum)
                .bind(migration.version)
                .execute(pool)
                .await;
        }
    }

    // 3. Execute all pending migrations

    migrator.run(pool).await
}

#[derive(Clone)]
pub struct PostgresStore {
    pool: PgPool,
}

impl PostgresStore {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn connect(database_url: &str) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .connect(database_url)
            .await?;

        run_safe_migrations(&pool).await?;

        Ok(Self { pool })
    }

    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn get_module_settings(
        &self,

        guild_id: &str,

        module_id: &str,
    ) -> anyhow::Result<Option<GuildModuleSettings>> {
        let result = sqlx::query_as::<_, GuildModuleSettings>(
            r#"

            SELECT guild_id, module_id, enabled, config, updated_at

            FROM guild_module_settings

            WHERE guild_id = $1 AND module_id = $2

            "#,
        )
        .bind(guild_id)
        .bind(module_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result)
    }

    pub async fn ping(&self) -> anyhow::Result<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;

        Ok(())
    }

    pub async fn set_module_settings(
        &self,

        guild_id: &str,

        module_id: &str,

        enabled: bool,

        config: Value,
    ) -> anyhow::Result<GuildModuleSettings> {
        let result = sqlx::query_as::<_, GuildModuleSettings>(
            r#"

            INSERT INTO guild_module_settings (guild_id, module_id, enabled, config)

            VALUES ($1, $2, $3, $4)

            ON CONFLICT (guild_id, module_id)

            DO UPDATE SET

                enabled = EXCLUDED.enabled,

                config = EXCLUDED.config,

                updated_at = NOW()

            RETURNING guild_id, module_id, enabled, config, updated_at

            "#,
        )
        .bind(guild_id)
        .bind(module_id)
        .bind(enabled)
        .bind(config)
        .fetch_one(&self.pool)
        .await?;

        Ok(result)
    }

    pub async fn update_module_settings_with_audit(
        &self,

        params: super::UpdateModuleSettingsParams<'_>,
    ) -> anyhow::Result<GuildModuleSettings> {
        let mut tx = self.pool.begin().await?;

        let settings = sqlx::query_as::<_, GuildModuleSettings>(
            r#"

            INSERT INTO guild_module_settings (guild_id, module_id, enabled, config)

            VALUES ($1, $2, $3, $4)

            ON CONFLICT (guild_id, module_id)

            DO UPDATE SET

                enabled = EXCLUDED.enabled,

                config = EXCLUDED.config,

                updated_at = NOW()

            RETURNING guild_id, module_id, enabled, config, updated_at

            "#,
        )
        .bind(params.guild_id)
        .bind(params.module_id)
        .bind(params.enabled)
        .bind(params.config)
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query(
            r#"

            INSERT INTO audit_events (guild_id, actor_id, module_id, event, data)

            VALUES ($1, $2, $3, $4, $5)

            "#,
        )
        .bind(params.guild_id)
        .bind(params.actor_id)
        .bind("dashboard")
        .bind(params.audit_event)
        .bind(params.audit_data)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(settings)
    }

    pub async fn record_audit(
        &self,

        guild_id: Option<&str>,

        actor_id: Option<&str>,

        module_id: &str,

        event: &str,

        data: Value,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"

            INSERT INTO audit_events (guild_id, actor_id, module_id, event, data)

            VALUES ($1, $2, $3, $4, $5)

            "#,
        )
        .bind(guild_id)
        .bind(actor_id)
        .bind(module_id)
        .bind(event)
        .bind(data)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn list_audit_events(
        &self,
        guild_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<AuditLogEntry>> {
        let rows = sqlx::query_as::<_, AuditLogEntry>(
            r#"
            SELECT id, guild_id, actor_id, module_id, event, data, created_at
            FROM audit_events
            WHERE guild_id = $1 AND created_at >= NOW() - INTERVAL '45 days'
            ORDER BY created_at DESC, id DESC
            LIMIT $2
            "#,
        )
        .bind(guild_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn list_audit_events_page(
        &self,
        guild_id: &str,
        limit: i64,
        offset: i64,
    ) -> anyhow::Result<Vec<AuditLogEntry>> {
        let rows = sqlx::query_as::<_, AuditLogEntry>(
            r#"
            SELECT id, guild_id, actor_id, module_id, event, data, created_at
            FROM audit_events
            WHERE guild_id = $1 AND created_at >= NOW() - INTERVAL '45 days'
            ORDER BY created_at DESC, id DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(guild_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn count_audit_events(&self, guild_id: &str) -> anyhow::Result<u64> {
        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)
            FROM audit_events
            WHERE guild_id = $1 AND created_at >= NOW() - INTERVAL '45 days'
            "#,
        )
        .bind(guild_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(count.0 as u64)
    }

    pub async fn prune_audit_events(
        &self,
        cutoff: DateTime<Utc>,
        limit: usize,
    ) -> anyhow::Result<u64> {
        let limit = limit.clamp(1, 5000) as i64;
        let res = sqlx::query(
            r#"
            WITH expired AS (
                SELECT id FROM audit_events
                WHERE created_at < $1
                ORDER BY created_at, id
                LIMIT $2
            )
            DELETE FROM audit_events a
            USING expired x
            WHERE a.id = x.id
            "#,
        )
        .bind(cutoff)
        .bind(limit)
        .execute(&self.pool)
        .await?;

        Ok(res.rows_affected())
    }

    pub async fn get_member_progress(
        &self,

        guild_id: &str,

        user_id: &str,
    ) -> anyhow::Result<MemberProgress> {
        ensure_profile_on_pool(&self.pool, guild_id, user_id).await?;

        Ok(fetch_profile_on_pool(&self.pool, guild_id, user_id).await?)
    }

    pub async fn has_member_progress(&self, guild_id: &str, user_id: &str) -> anyhow::Result<bool> {
        let row = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(1) FROM member_progress WHERE guild_id = $1 AND user_id = $2",
        )
        .bind(guild_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row > 0)
    }

    pub async fn erase_guild_user_data(
        &self,
        guild_id: &str,
        user_id: &str,
    ) -> anyhow::Result<UserErasureReport> {
        let mut tx = self.pool.begin().await?;

        let deleted_progress =
            sqlx::query("DELETE FROM member_progress WHERE guild_id = $1 AND user_id = $2")
                .bind(guild_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?
                .rows_affected();

        let mut related_erased = 0;

        let deleted_roles =
            sqlx::query("DELETE FROM member_persistent_roles WHERE guild_id = $1 AND user_id = $2")
                .bind(guild_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map(|r| r.rows_affected())
                .unwrap_or(0);
        related_erased += deleted_roles as usize;

        let deleted_rewards =
            sqlx::query("DELETE FROM member_role_rewards WHERE guild_id = $1 AND user_id = $2")
                .bind(guild_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map(|r| r.rows_affected())
                .unwrap_or(0);
        related_erased += deleted_rewards as usize;

        let deleted_identities =
            sqlx::query("DELETE FROM member_identity_history WHERE guild_id = $1 AND user_id = $2")
                .bind(guild_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map(|r| r.rows_affected())
                .unwrap_or(0);
        related_erased += deleted_identities as usize;

        let deleted_snapshots =
            sqlx::query("DELETE FROM member_snapshots WHERE guild_id = $1 AND user_id = $2")
                .bind(guild_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map(|r| r.rows_affected())
                .unwrap_or(0);
        related_erased += deleted_snapshots as usize;

        let deleted_plans =
            sqlx::query("DELETE FROM cleanup_plans WHERE guild_id = $1 AND actor_user_id = $2")
                .bind(guild_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map(|r| r.rows_affected())
                .unwrap_or(0);
        related_erased += deleted_plans as usize;

        let deleted_leases = sqlx::query(
            "DELETE FROM companion_channel_leases WHERE guild_id = $1 AND user_id = $2",
        )
        .bind(guild_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map(|r| r.rows_affected())
        .unwrap_or(0);
        related_erased += deleted_leases as usize;

        let deleted_policy = sqlx::query(
            "DELETE FROM command_policy_rules WHERE guild_id = $1 AND subject_kind = 'user' AND subject_id = $2",
        )
        .bind(guild_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map(|r| r.rows_affected())
        .unwrap_or(0);
        related_erased += deleted_policy as usize;

        let deleted_duty =
            sqlx::query("DELETE FROM moderator_duty_profiles WHERE guild_id = $1 AND user_id = $2")
                .bind(guild_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map(|r| r.rows_affected())
                .unwrap_or(0);
        related_erased += deleted_duty as usize;

        let _ =
            sqlx::query("DELETE FROM moderator_duty_state WHERE guild_id = $1 AND user_id = $2")
                .bind(guild_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await;

        let _ = sqlx::query(
            "DELETE FROM moderator_duty_role_grants WHERE guild_id = $1 AND user_id = $2",
        )
        .bind(guild_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await;

        let deleted_rec_apps = sqlx::query(
            "DELETE FROM recruitment_applications WHERE guild_id = $1 AND applicant_user_id = $2",
        )
        .bind(guild_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map(|r| r.rows_affected())
        .unwrap_or(0);
        related_erased += deleted_rec_apps as usize;

        let deleted_rec_revs = sqlx::query(
            "DELETE FROM recruitment_reviews WHERE guild_id = $1 AND reviewer_user_id = $2",
        )
        .bind(guild_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map(|r| r.rows_affected())
        .unwrap_or(0);
        related_erased += deleted_rec_revs as usize;

        let deleted_help_sessions = sqlx::query(
            "DELETE FROM help_forum_sessions WHERE guild_id = $1 AND owner_user_id = $2",
        )
        .bind(guild_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map(|r| r.rows_affected())
        .unwrap_or(0);
        related_erased += deleted_help_sessions as usize;

        let deleted_economy_cooldowns =
            sqlx::query("DELETE FROM economy_cooldowns WHERE guild_id = $1 AND user_id = $2")
                .bind(guild_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map(|r| r.rows_affected())
                .unwrap_or(0);
        related_erased += deleted_economy_cooldowns as usize;

        let deleted_economy_idempotency =
            sqlx::query("DELETE FROM economy_idempotency WHERE guild_id = $1 AND user_id = $2")
                .bind(guild_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map(|r| r.rows_affected())
                .unwrap_or(0);
        related_erased += deleted_economy_idempotency as usize;

        let deleted_economy_purchases = sqlx::query(
            "DELETE FROM economy_purchase_requests WHERE guild_id = $1 AND user_id = $2",
        )
        .bind(guild_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map(|r| r.rows_affected())
        .unwrap_or(0);
        related_erased += deleted_economy_purchases as usize;

        tx.commit().await?;

        let retained_records = vec![
            RetainedRecordNotice {
                entity: "Moderation cases".to_string(),
                reason: "Server security, sanctions audit and anti-abuse accountability"
                    .to_string(),
                retention_policy: "365 days".to_string(),
            },
            RetainedRecordNotice {
                entity: "Audit log entries".to_string(),
                reason: "Administrative action audit trail and server governance".to_string(),
                retention_policy: "45 days".to_string(),
            },
        ];

        Ok(UserErasureReport {
            user_id: user_id.to_string(),
            guild_id: guild_id.to_string(),
            progression_erased: deleted_progress > 0,
            related_records_erased: related_erased,
            retained_records,
            erased_at: Utc::now(),
        })
    }

    pub async fn award_message_xp(
        &self,

        guild_id: &str,

        user_id: &str,

        amount: i64,

        cooldown_seconds: u32,
    ) -> anyhow::Result<XpAward> {
        let mut transaction = self.pool.begin().await?;

        ensure_profile(&mut transaction, guild_id, user_id).await?;

        let mut profile = lock_profile(&mut transaction, guild_id, user_id).await?;

        let now = Utc::now();

        let previous_level = profile.level;

        profile.messages = profile.messages.saturating_add(1);

        let eligible = profile.last_xp_at.as_ref().is_none_or(|last_award| {
            now.signed_duration_since(*last_award) >= Duration::seconds(i64::from(cooldown_seconds))
        });

        let awarded = if eligible { amount.max(0) } else { 0 };

        if awarded > 0 {
            profile.xp = profile.xp.saturating_add(awarded);

            profile.level = level_for_xp(profile.xp);

            profile.last_xp_at = Some(now);
        }

        profile.updated_at = now;

        write_profile(&mut transaction, &profile).await?;

        transaction.commit().await?;

        Ok(XpAward {
            profile,

            awarded,

            previous_level,
        })
    }

    pub async fn claim_daily(
        &self,

        guild_id: &str,

        user_id: &str,

        base_reward: i64,

        streak_bonus: i64,

        max_streak_bonus: i32,
    ) -> anyhow::Result<DailyClaimOutcome> {
        let mut transaction = self.pool.begin().await?;

        ensure_profile(&mut transaction, guild_id, user_id).await?;

        let mut profile = lock_profile(&mut transaction, guild_id, user_id).await?;

        let now = Utc::now();

        if let Some(last_claim) = profile.last_daily_at {
            let next_claim_at = last_claim + Duration::hours(24);

            if now < next_claim_at {
                transaction.commit().await?;

                return Ok(DailyClaimOutcome::Cooldown {
                    profile,

                    next_claim_at,
                });
            }

            profile.daily_streak = if now <= last_claim + Duration::hours(48) {
                profile.daily_streak.saturating_add(1).max(1)
            } else {
                1
            };
        } else {
            profile.daily_streak = 1;
        }

        let bonus_steps = profile
            .daily_streak
            .saturating_sub(1)
            .min(max_streak_bonus.max(0));

        let reward = base_reward
            .max(0)
            .saturating_add(streak_bonus.max(0).saturating_mul(i64::from(bonus_steps)));

        profile.coins = profile.coins.saturating_add(reward);

        profile.last_daily_at = Some(now);

        profile.updated_at = now;

        write_profile(&mut transaction, &profile).await?;

        transaction.commit().await?;

        Ok(DailyClaimOutcome::Claimed {
            profile,

            reward,

            next_claim_at: now + Duration::hours(24),
        })
    }

    fn generate_random_unit_float() -> f64 {
        let bytes = *Uuid::new_v4().as_bytes();
        let u = u64::from_le_bytes(bytes[0..8].try_into().unwrap_or([0u8; 8]));
        (u as f64) / (u64::MAX as f64)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn claim_work_reward(
        &self,
        guild_id: &str,
        user_id: &str,
        interaction_id: Option<&str>,
        min_payout: i64,
        max_payout: i64,
        cooldown_seconds: u64,
        roll: Option<f64>,
    ) -> anyhow::Result<WorkClaimOutcome> {
        let mut transaction = self.pool.begin().await?;

        if let Some(id) = interaction_id {
            let row: Option<(String, String)> = sqlx::query_as(
                "SELECT user_id, outcome_json FROM economy_idempotency WHERE guild_id = $1 AND interaction_id = $2"
            )
            .bind(guild_id)
            .bind(id)
            .fetch_optional(&mut *transaction)
            .await?;

            if let Some((entry_user_id, json_str)) = row
                && entry_user_id == user_id
                && let Ok(outcome) = serde_json::from_str::<WorkClaimOutcome>(&json_str)
            {
                ensure_profile(&mut transaction, guild_id, user_id).await?;
                let profile = lock_profile(&mut transaction, guild_id, user_id).await?;
                transaction.commit().await?;
                return Ok(match outcome {
                    WorkClaimOutcome::Claimed { reward, job, .. }
                    | WorkClaimOutcome::AlreadyProcessed { reward, job, .. } => {
                        WorkClaimOutcome::AlreadyProcessed {
                            profile,
                            reward,
                            job,
                        }
                    }
                    _ => outcome,
                });
            }
        }

        ensure_profile(&mut transaction, guild_id, user_id).await?;
        let mut profile = lock_profile(&mut transaction, guild_id, user_id).await?;
        let now = Utc::now();

        let last_action_row: Option<(DateTime<Utc>,)> = sqlx::query_as(
            "SELECT last_action_at FROM economy_cooldowns WHERE guild_id = $1 AND user_id = $2 AND action_type = 'work' FOR UPDATE"
        )
        .bind(guild_id)
        .bind(user_id)
        .fetch_optional(&mut *transaction)
        .await?;

        if let Some((last_action_at,)) = last_action_row {
            let next_claim_at = last_action_at + Duration::seconds(cooldown_seconds as i64);
            if now < next_claim_at {
                transaction.commit().await?;
                return Ok(WorkClaimOutcome::Cooldown {
                    profile,
                    next_claim_at,
                });
            }
        }

        let r = roll
            .unwrap_or_else(Self::generate_random_unit_float)
            .clamp(0.0, 1.0);
        let job_idx =
            ((r * WORK_JOBS.len() as f64) as usize).min(WORK_JOBS.len().saturating_sub(1));
        let job = WORK_JOBS[job_idx].to_string();

        let span = (max_payout - min_payout).max(0);
        let reward = min_payout
            .saturating_add((span as f64 * r).round() as i64)
            .max(1);

        profile.coins = profile.coins.saturating_add(reward);
        profile.updated_at = now;
        write_profile(&mut transaction, &profile).await?;

        let next_claim_at = now + Duration::seconds(cooldown_seconds as i64);
        sqlx::query(
            r#"
            INSERT INTO economy_cooldowns (guild_id, user_id, action_type, last_action_at)
            VALUES ($1, $2, 'work', $3)
            ON CONFLICT (guild_id, user_id, action_type) DO UPDATE SET last_action_at = EXCLUDED.last_action_at
            "#
        )
        .bind(guild_id)
        .bind(user_id)
        .bind(now)
        .execute(&mut *transaction)
        .await?;

        let outcome = WorkClaimOutcome::Claimed {
            profile,
            reward,
            job,
            next_claim_at,
        };

        if let Some(id) = interaction_id
            && let Ok(serialized) = serde_json::to_string(&outcome)
        {
            sqlx::query(
                r#"
                INSERT INTO economy_idempotency (guild_id, interaction_id, user_id, action_type, outcome_json, created_at)
                VALUES ($1, $2, $3, 'work', $4, $5)
                ON CONFLICT (guild_id, interaction_id) DO NOTHING
                "#
            )
            .bind(guild_id)
            .bind(id)
            .bind(user_id)
            .bind(serialized)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        }

        transaction.commit().await?;
        Ok(outcome)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn claim_crime_reward(
        &self,
        guild_id: &str,
        user_id: &str,
        interaction_id: Option<&str>,
        min_payout: i64,
        max_payout: i64,
        success_rate: f64,
        payout_multiplier: f64,
        cooldown_seconds: u64,
        roll: Option<f64>,
    ) -> anyhow::Result<CrimeClaimOutcome> {
        let mut transaction = self.pool.begin().await?;

        if let Some(id) = interaction_id {
            let row: Option<(String, String)> = sqlx::query_as(
                "SELECT user_id, outcome_json FROM economy_idempotency WHERE guild_id = $1 AND interaction_id = $2"
            )
            .bind(guild_id)
            .bind(id)
            .fetch_optional(&mut *transaction)
            .await?;

            if let Some((entry_user_id, json_str)) = row
                && entry_user_id == user_id
                && let Ok(outcome) = serde_json::from_str::<CrimeClaimOutcome>(&json_str)
            {
                ensure_profile(&mut transaction, guild_id, user_id).await?;
                let profile = lock_profile(&mut transaction, guild_id, user_id).await?;
                transaction.commit().await?;
                return Ok(match outcome {
                    CrimeClaimOutcome::Success { reward, .. } => {
                        CrimeClaimOutcome::AlreadyProcessed {
                            profile,
                            success: true,
                            amount: reward,
                        }
                    }
                    CrimeClaimOutcome::Failure { penalty, .. } => {
                        CrimeClaimOutcome::AlreadyProcessed {
                            profile,
                            success: false,
                            amount: penalty,
                        }
                    }
                    CrimeClaimOutcome::AlreadyProcessed {
                        success, amount, ..
                    } => CrimeClaimOutcome::AlreadyProcessed {
                        profile,
                        success,
                        amount,
                    },
                    _ => outcome,
                });
            }
        }

        ensure_profile(&mut transaction, guild_id, user_id).await?;
        let mut profile = lock_profile(&mut transaction, guild_id, user_id).await?;
        let now = Utc::now();

        let last_action_row: Option<(DateTime<Utc>,)> = sqlx::query_as(
            "SELECT last_action_at FROM economy_cooldowns WHERE guild_id = $1 AND user_id = $2 AND action_type = 'crime' FOR UPDATE"
        )
        .bind(guild_id)
        .bind(user_id)
        .fetch_optional(&mut *transaction)
        .await?;

        if let Some((last_action_at,)) = last_action_row {
            let next_claim_at = last_action_at + Duration::seconds(cooldown_seconds as i64);
            if now < next_claim_at {
                transaction.commit().await?;
                return Ok(CrimeClaimOutcome::Cooldown {
                    profile,
                    next_claim_at,
                });
            }
        }

        let r = roll
            .unwrap_or_else(Self::generate_random_unit_float)
            .clamp(0.0, 1.0);
        let success = r <= success_rate;
        let next_claim_at = now + Duration::seconds(cooldown_seconds as i64);

        let outcome = if success {
            let span = (max_payout - min_payout).max(0);
            let base_reward = min_payout
                .saturating_add((span as f64 * r).round() as i64)
                .max(1);
            let reward = ((base_reward as f64 * payout_multiplier).round() as i64).max(1);
            profile.coins = profile.coins.saturating_add(reward);
            profile.updated_at = now;
            write_profile(&mut transaction, &profile).await?;

            sqlx::query(
                r#"
                INSERT INTO economy_cooldowns (guild_id, user_id, action_type, last_action_at)
                VALUES ($1, $2, 'crime', $3)
                ON CONFLICT (guild_id, user_id, action_type) DO UPDATE SET last_action_at = EXCLUDED.last_action_at
                "#
            )
            .bind(guild_id)
            .bind(user_id)
            .bind(now)
            .execute(&mut *transaction)
            .await?;

            CrimeClaimOutcome::Success {
                profile,
                reward,
                next_claim_at,
            }
        } else {
            let penalty = 0;
            profile.updated_at = now;
            write_profile(&mut transaction, &profile).await?;

            sqlx::query(
                r#"
                INSERT INTO economy_cooldowns (guild_id, user_id, action_type, last_action_at)
                VALUES ($1, $2, 'crime', $3)
                ON CONFLICT (guild_id, user_id, action_type) DO UPDATE SET last_action_at = EXCLUDED.last_action_at
                "#
            )
            .bind(guild_id)
            .bind(user_id)
            .bind(now)
            .execute(&mut *transaction)
            .await?;

            CrimeClaimOutcome::Failure {
                profile,
                penalty,
                next_claim_at,
            }
        };

        if let Some(id) = interaction_id
            && let Ok(serialized) = serde_json::to_string(&outcome)
        {
            sqlx::query(
                r#"
                INSERT INTO economy_idempotency (guild_id, interaction_id, user_id, action_type, outcome_json, created_at)
                VALUES ($1, $2, $3, 'crime', $4, $5)
                ON CONFLICT (guild_id, interaction_id) DO NOTHING
                "#
            )
            .bind(guild_id)
            .bind(id)
            .bind(user_id)
            .bind(serialized)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        }

        transaction.commit().await?;
        Ok(outcome)
    }

    pub async fn transfer_coins(
        &self,

        guild_id: &str,

        sender_user_id: &str,

        recipient_user_id: &str,

        amount: i64,
    ) -> anyhow::Result<CoinTransferOutcome> {
        if sender_user_id == recipient_user_id {
            return Ok(CoinTransferOutcome::SameAccount);
        }

        let (first_user_id, second_user_id) = if sender_user_id < recipient_user_id {
            (sender_user_id, recipient_user_id)
        } else {
            (recipient_user_id, sender_user_id)
        };

        let mut transaction = self.pool.begin().await?;

        ensure_profile(&mut transaction, guild_id, first_user_id).await?;

        ensure_profile(&mut transaction, guild_id, second_user_id).await?;

        let first = lock_profile(&mut transaction, guild_id, first_user_id).await?;

        let second = lock_profile(&mut transaction, guild_id, second_user_id).await?;

        let (mut sender, mut recipient) = if first.user_id == sender_user_id {
            (first, second)
        } else {
            (second, first)
        };

        if amount <= 0 || sender.coins < amount {
            let balance = sender.coins;

            transaction.commit().await?;

            return Ok(CoinTransferOutcome::InsufficientFunds { balance });
        }

        let now = Utc::now();

        sender.coins -= amount;

        recipient.coins = recipient.coins.saturating_add(amount);

        sender.updated_at = now;

        recipient.updated_at = now;

        write_profile(&mut transaction, &sender).await?;

        write_profile(&mut transaction, &recipient).await?;

        let audit_data = serde_json::json!({

            "recipient_user_id": recipient_user_id,

            "amount": amount,

            "sender_balance": sender.coins,

            "recipient_balance": recipient.coins,

        });

        sqlx::query(
            r#"

            INSERT INTO audit_events (guild_id, actor_id, module_id, event, data, created_at)

            VALUES ($1, $2, $3, $4, $5, $6)

            "#,
        )
        .bind(guild_id)
        .bind(sender_user_id)
        .bind("progression")
        .bind("coins_transferred")
        .bind(audit_data)
        .bind(now)
        .execute(&mut *transaction)
        .await?;

        transaction.commit().await?;

        Ok(CoinTransferOutcome::Completed(Box::new(CoinTransfer {
            sender,

            recipient,

            amount,
        })))
    }

    pub async fn give_reputation(
        &self,

        guild_id: &str,

        giver_user_id: &str,

        target_user_id: &str,

        amount: i64,

        cooldown_seconds: u32,
    ) -> anyhow::Result<ReputationGrantOutcome> {
        if giver_user_id == target_user_id {
            return Ok(ReputationGrantOutcome::SameAccount);
        }

        let mut transaction = self.pool.begin().await?;

        ensure_profile(&mut transaction, guild_id, target_user_id).await?;

        sqlx::query(
            r#"

            INSERT INTO reputation_cooldowns (guild_id, giver_user_id)

            VALUES ($1, $2)

            ON CONFLICT (guild_id, giver_user_id) DO NOTHING

            "#,
        )
        .bind(guild_id)
        .bind(giver_user_id)
        .execute(&mut *transaction)
        .await?;

        let last_given_at = sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
            r#"

            SELECT last_given_at

            FROM reputation_cooldowns

            WHERE guild_id = $1 AND giver_user_id = $2

            FOR UPDATE

            "#,
        )
        .bind(guild_id)
        .bind(giver_user_id)
        .fetch_one(&mut *transaction)
        .await?;

        let now = Utc::now();

        if let Some(last_given_at) = last_given_at {
            let next_available_at = last_given_at + Duration::seconds(i64::from(cooldown_seconds));

            if now < next_available_at {
                transaction.commit().await?;

                return Ok(ReputationGrantOutcome::Cooldown { next_available_at });
            }
        }

        let mut profile = lock_profile(&mut transaction, guild_id, target_user_id).await?;

        let amount = amount.max(0);

        profile.reputation = profile.reputation.saturating_add(amount);

        profile.updated_at = now;

        write_profile(&mut transaction, &profile).await?;

        sqlx::query(
            r#"

            UPDATE reputation_cooldowns

            SET target_user_id = $3, last_given_at = $4

            WHERE guild_id = $1 AND giver_user_id = $2

            "#,
        )
        .bind(guild_id)
        .bind(giver_user_id)
        .bind(target_user_id)
        .bind(now)
        .execute(&mut *transaction)
        .await?;

        transaction.commit().await?;

        Ok(ReputationGrantOutcome::Granted {
            profile,

            amount,

            next_available_at: now + Duration::seconds(i64::from(cooldown_seconds)),
        })
    }

    pub async fn adjust_member_progress_with_audit(
        &self,

        params: AdjustMemberProgressParams<'_>,
    ) -> anyhow::Result<MemberProgress> {
        let mut transaction = self.pool.begin().await?;

        ensure_profile(&mut transaction, params.guild_id, params.user_id).await?;

        let mut profile = lock_profile(&mut transaction, params.guild_id, params.user_id).await?;

        profile.xp = profile.xp.saturating_add(params.xp_delta).max(0);

        profile.coins = profile.coins.saturating_add(params.coins_delta).max(0);

        profile.reputation = profile
            .reputation
            .saturating_add(params.reputation_delta)
            .max(0);

        profile.level = level_for_xp(profile.xp);

        let now = Utc::now();

        profile.updated_at = now;

        write_profile(&mut transaction, &profile).await?;

        let audit_data = serde_json::json!({

            "target_user_id": params.user_id,

            "xp_delta": params.xp_delta,

            "coins_delta": params.coins_delta,

            "reputation_delta": params.reputation_delta,

            "reason": params.reason,

            "result": {

                "xp": profile.xp,

                "level": profile.level,

                "coins": profile.coins,

                "reputation": profile.reputation,

            }

        });

        sqlx::query(
            r#"

            INSERT INTO audit_events (guild_id, actor_id, module_id, event, data, created_at)

            VALUES ($1, $2, $3, $4, $5, $6)

            "#,
        )
        .bind(params.guild_id)
        .bind(params.actor_id)
        .bind("progression")
        .bind("progress_adjusted")
        .bind(audit_data)
        .bind(now)
        .execute(&mut *transaction)
        .await?;

        transaction.commit().await?;

        Ok(profile)
    }

    pub async fn adjust_member_progress(
        &self,

        guild_id: &str,

        user_id: &str,

        xp_delta: i64,

        coins_delta: i64,

        reputation_delta: i64,
    ) -> anyhow::Result<MemberProgress> {
        let mut transaction = self.pool.begin().await?;

        ensure_profile(&mut transaction, guild_id, user_id).await?;

        let mut profile = lock_profile(&mut transaction, guild_id, user_id).await?;

        profile.xp = profile.xp.saturating_add(xp_delta).max(0);

        profile.coins = profile.coins.saturating_add(coins_delta).max(0);

        profile.reputation = profile.reputation.saturating_add(reputation_delta).max(0);

        profile.level = level_for_xp(profile.xp);

        profile.updated_at = Utc::now();

        write_profile(&mut transaction, &profile).await?;

        transaction.commit().await?;

        Ok(profile)
    }

    pub async fn progression_leaderboard(
        &self,

        guild_id: &str,

        metric: ProgressMetric,

        limit: u8,
    ) -> anyhow::Result<Vec<MemberProgress>> {
        let query = match metric {
            ProgressMetric::Xp => {
                "SELECT guild_id, user_id, xp, level, coins, reputation, messages, daily_streak, last_xp_at, last_daily_at, created_at, updated_at FROM member_progress WHERE guild_id = $1 AND xp > 0 ORDER BY xp DESC, user_id ASC LIMIT $2"
            }

            ProgressMetric::Coins => {
                "SELECT guild_id, user_id, xp, level, coins, reputation, messages, daily_streak, last_xp_at, last_daily_at, created_at, updated_at FROM member_progress WHERE guild_id = $1 AND coins > 0 ORDER BY coins DESC, user_id ASC LIMIT $2"
            }

            ProgressMetric::Reputation => {
                "SELECT guild_id, user_id, xp, level, coins, reputation, messages, daily_streak, last_xp_at, last_daily_at, created_at, updated_at FROM member_progress WHERE guild_id = $1 AND reputation > 0 ORDER BY reputation DESC, user_id ASC LIMIT $2"
            }

            ProgressMetric::Messages => {
                "SELECT guild_id, user_id, xp, level, coins, reputation, messages, daily_streak, last_xp_at, last_daily_at, created_at, updated_at FROM member_progress WHERE guild_id = $1 AND messages > 0 ORDER BY messages DESC, user_id ASC LIMIT $2"
            }
        };

        Ok(sqlx::query_as::<_, MemberProgress>(query)
            .bind(guild_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn is_role_reward_granted(
        &self,

        guild_id: &str,

        user_id: &str,

        role_id: &str,
    ) -> anyhow::Result<bool> {
        let exists: bool = sqlx::query_scalar(
            r#"

            SELECT EXISTS(

                SELECT 1 FROM member_role_rewards

                WHERE guild_id = $1 AND user_id = $2 AND role_id = $3

            )

            "#,
        )
        .bind(guild_id)
        .bind(user_id)
        .bind(role_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(exists)
    }

    pub async fn record_role_reward_granted(
        &self,

        guild_id: &str,

        user_id: &str,

        role_id: &str,

        level: i64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"

            INSERT INTO member_role_rewards (guild_id, user_id, role_id, level)

            VALUES ($1, $2, $3, $4)

            ON CONFLICT (guild_id, user_id, role_id) DO NOTHING

            "#,
        )
        .bind(guild_id)
        .bind(user_id)
        .bind(role_id)
        .bind(level)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn record_moderation_intent(
        &self,

        intent: &ModerationExecutionIntent,
    ) -> anyhow::Result<()> {
        sqlx::query(

            r#"

            INSERT INTO moderation_execution_intents (

                id, interaction_id, guild_id, user_id, kind, step, reason, error, created_at, updated_at

            )

            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)

            "#,

        )

        .bind(intent.id)

        .bind(&intent.interaction_id)

        .bind(&intent.guild_id)

        .bind(&intent.user_id)

        .bind(&intent.kind)

        .bind(&intent.step)

        .bind(&intent.reason)

        .bind(&intent.error)

        .bind(intent.created_at)

        .bind(intent.updated_at)

        .execute(&self.pool)

        .await?;

        Ok(())
    }

    pub async fn update_moderation_intent_step(
        &self,

        id: Uuid,

        step: &str,

        error: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"

            UPDATE moderation_execution_intents

            SET step = $1, error = $2, updated_at = NOW()

            WHERE id = $3

            "#,
        )
        .bind(step)
        .bind(error)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn list_pending_moderation_intents(
        &self,

        kind: &str,

        step: &str,
    ) -> anyhow::Result<Vec<ModerationExecutionIntent>> {
        let results = sqlx::query_as::<_, ModerationExecutionIntent>(

            r#"

            SELECT id, interaction_id, guild_id, user_id, kind, step, reason, error, created_at, updated_at

            FROM moderation_execution_intents

            WHERE kind = $1 AND step = $2

            ORDER BY created_at ASC

            "#,

        )

        .bind(kind)

        .bind(step)

        .fetch_all(&self.pool)

        .await?;

        Ok(results)
    }

    pub async fn get_module_draft(
        &self,

        guild_id: &str,

        module_id: &str,
    ) -> anyhow::Result<Option<ModuleDraftConfig>> {
        let result = sqlx::query_as::<_, ModuleDraftConfig>(
            r#"

            SELECT guild_id, module_id, config_json, updated_by, updated_at

            FROM module_draft_configs

            WHERE guild_id = $1 AND module_id = $2

            "#,
        )
        .bind(guild_id)
        .bind(module_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result)
    }

    pub async fn set_module_draft(
        &self,

        guild_id: &str,

        module_id: &str,

        config: Value,

        updated_by: &str,
    ) -> anyhow::Result<ModuleDraftConfig> {
        let result = sqlx::query_as::<_, ModuleDraftConfig>(

            r#"

            INSERT INTO module_draft_configs (guild_id, module_id, config_json, updated_by, updated_at)

            VALUES ($1, $2, $3, $4, NOW())

            ON CONFLICT (guild_id, module_id) DO UPDATE SET

                config_json = EXCLUDED.config_json,

                updated_by = EXCLUDED.updated_by,

                updated_at = NOW()

            RETURNING guild_id, module_id, config_json, updated_by, updated_at

            "#,

        )

        .bind(guild_id)

        .bind(module_id)

        .bind(config)

        .bind(updated_by)

        .fetch_one(&self.pool)

        .await?;

        Ok(result)
    }

    pub async fn delete_module_draft(
        &self,

        guild_id: &str,

        module_id: &str,
    ) -> anyhow::Result<bool> {
        let res = sqlx::query(
            r#"

            DELETE FROM module_draft_configs

            WHERE guild_id = $1 AND module_id = $2

            "#,
        )
        .bind(guild_id)
        .bind(module_id)
        .execute(&self.pool)
        .await?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn create_appeal(&self, appeal: &ModerationAppeal) -> anyhow::Result<()> {
        sqlx::query(

            r#"

            INSERT INTO moderation_appeals (

                id, guild_id, case_number, user_id, status, reason, evidence_url, staff_notes, created_at, updated_at

            )

            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)

            "#,

        )

        .bind(appeal.id)

        .bind(&appeal.guild_id)

        .bind(appeal.case_number)

        .bind(&appeal.user_id)

        .bind(&appeal.status)

        .bind(&appeal.reason)

        .bind(&appeal.evidence_url)

        .bind(&appeal.staff_notes)

        .bind(appeal.created_at)

        .bind(appeal.updated_at)

        .execute(&self.pool)

        .await?;

        Ok(())
    }

    pub async fn list_appeals(
        &self,

        guild_id: &str,

        status: Option<&str>,
    ) -> anyhow::Result<Vec<ModerationAppeal>> {
        let results = sqlx::query_as::<_, ModerationAppeal>(

            r#"

            SELECT id, guild_id, case_number, user_id, status, reason, evidence_url, staff_notes, created_at, updated_at

            FROM moderation_appeals

            WHERE guild_id = $1 AND ($2::text IS NULL OR status = $2)

            ORDER BY created_at DESC

            "#,

        )

        .bind(guild_id)

        .bind(status)

        .fetch_all(&self.pool)

        .await?;

        Ok(results)
    }

    pub async fn get_appeal(&self, appeal_id: Uuid) -> anyhow::Result<Option<ModerationAppeal>> {
        let result = sqlx::query_as::<_, ModerationAppeal>(

            r#"

            SELECT id, guild_id, case_number, user_id, status, reason, evidence_url, staff_notes, created_at, updated_at

            FROM moderation_appeals

            WHERE id = $1

            "#,

        )

        .bind(appeal_id)

        .fetch_optional(&self.pool)

        .await?;

        Ok(result)
    }

    pub async fn update_appeal_status(
        &self,
        guild_id: &str,
        appeal_id: Uuid,
        status: &str,
        staff_notes: Option<&str>,
    ) -> anyhow::Result<bool> {
        let res = sqlx::query(
            r#"
            UPDATE moderation_appeals
            SET status = $3, staff_notes = COALESCE($4, staff_notes), updated_at = NOW()
            WHERE id = $1 AND guild_id = $2
            "#,
        )
        .bind(appeal_id)
        .bind(guild_id)
        .bind(status)
        .bind(staff_notes)
        .execute(&self.pool)
        .await?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn grant_timed_role(
        &self,

        guild_id: &str,

        user_id: &str,

        role_id: &str,

        expires_at: DateTime<Utc>,
    ) -> anyhow::Result<Uuid> {
        let id = Uuid::new_v4();

        sqlx::query(
            r#"

            INSERT INTO timed_role_grants (id, guild_id, user_id, role_id, expires_at, created_at)

            VALUES ($1, $2, $3, $4, $5, NOW())

            "#,
        )
        .bind(id)
        .bind(guild_id)
        .bind(user_id)
        .bind(role_id)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    pub async fn get_active_timed_roles(
        &self,

        guild_id: &str,

        user_id: &str,
    ) -> anyhow::Result<Vec<TimedRoleGrant>> {
        let results = sqlx::query_as::<_, TimedRoleGrant>(
            r#"

            SELECT id, guild_id, user_id, role_id, expires_at, created_at

            FROM timed_role_grants

            WHERE guild_id = $1 AND user_id = $2 AND expires_at > NOW()

            "#,
        )
        .bind(guild_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(results)
    }

    pub async fn cleanup_expired_timed_roles(&self) -> anyhow::Result<u64> {
        let res = sqlx::query("DELETE FROM timed_role_grants WHERE expires_at <= NOW()")
            .execute(&self.pool)
            .await?;

        Ok(res.rows_affected())
    }

    pub async fn save_persistent_roles(
        &self,

        guild_id: &str,

        user_id: &str,

        role_ids: &[String],
    ) -> anyhow::Result<()> {
        let json_val = serde_json::to_value(role_ids)?;

        sqlx::query(
            r#"

            INSERT INTO persistent_user_roles (guild_id, user_id, role_ids, updated_at)

            VALUES ($1, $2, $3, NOW())

            ON CONFLICT (guild_id, user_id) DO UPDATE SET

                role_ids = EXCLUDED.role_ids,

                updated_at = NOW()

            "#,
        )
        .bind(guild_id)
        .bind(user_id)
        .bind(json_val)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_persistent_roles(
        &self,

        guild_id: &str,

        user_id: &str,
    ) -> anyhow::Result<Vec<String>> {
        let result: Option<(Value,)> = sqlx::query_as(
            r#"

            SELECT role_ids

            FROM persistent_user_roles

            WHERE guild_id = $1 AND user_id = $2

            "#,
        )
        .bind(guild_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        match result {
            Some((val,)) => Ok(serde_json::from_value(val).unwrap_or_default()),

            None => Ok(Vec::new()),
        }
    }

    pub async fn create_server_snapshot(
        &self,

        guild_id: &str,

        name: &str,

        snapshot_json: Value,

        created_by: &str,
    ) -> anyhow::Result<ServerSnapshot> {
        let id = Uuid::new_v4();

        let now = Utc::now();

        let rec = sqlx::query_as::<_, ServerSnapshot>(
            r#"

            INSERT INTO server_snapshots (id, guild_id, name, snapshot_json, created_by, created_at)

            VALUES ($1, $2, $3, $4, $5, $6)

            RETURNING id, guild_id, name, snapshot_json, created_by, created_at

            "#,
        )
        .bind(id)
        .bind(guild_id)
        .bind(name)
        .bind(snapshot_json)
        .bind(created_by)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;

        Ok(rec)
    }

    pub async fn list_server_snapshots(
        &self,

        guild_id: &str,
    ) -> anyhow::Result<Vec<ServerSnapshot>> {
        let rows = sqlx::query_as::<_, ServerSnapshot>(
            r#"

            SELECT id, guild_id, name, snapshot_json, created_by, created_at

            FROM server_snapshots

            WHERE guild_id = $1

            ORDER BY created_at DESC

            "#,
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn get_server_snapshot(
        &self,

        snapshot_id: Uuid,
    ) -> anyhow::Result<Option<ServerSnapshot>> {
        let row = sqlx::query_as::<_, ServerSnapshot>(
            r#"

            SELECT id, guild_id, name, snapshot_json, created_by, created_at

            FROM server_snapshots

            WHERE id = $1

            "#,
        )
        .bind(snapshot_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_server_snapshot(
        &self,

        guild_id: &str,

        snapshot_id: Uuid,
    ) -> anyhow::Result<bool> {
        let res = sqlx::query("DELETE FROM server_snapshots WHERE id = $1 AND guild_id = $2")
            .bind(snapshot_id)
            .bind(guild_id)
            .execute(&self.pool)
            .await?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn create_custom_automation(
        &self,

        guild_id: &str,

        name: &str,

        trigger_type: &str,

        conditions_json: Value,

        actions_json: Value,
    ) -> anyhow::Result<CustomAutomation> {
        let id = Uuid::new_v4();

        let now = Utc::now();

        let rec = sqlx::query_as::<_, CustomAutomation>(

            r#"

            INSERT INTO custom_automations (id, guild_id, name, trigger_type, conditions_json, actions_json, enabled, created_at, updated_at)

            VALUES ($1, $2, $3, $4, $5, $6, true, $7, $7)

            RETURNING id, guild_id, name, trigger_type, conditions_json, actions_json, enabled, created_at, updated_at

            "#,

        )

        .bind(id)

        .bind(guild_id)

        .bind(name)

        .bind(trigger_type)

        .bind(conditions_json)

        .bind(actions_json)

        .bind(now)

        .fetch_one(&self.pool)

        .await?;

        Ok(rec)
    }

    pub async fn list_custom_automations(
        &self,

        guild_id: &str,
    ) -> anyhow::Result<Vec<CustomAutomation>> {
        let rows = sqlx::query_as::<_, CustomAutomation>(

            r#"

            SELECT id, guild_id, name, trigger_type, conditions_json, actions_json, enabled, created_at, updated_at

            FROM custom_automations

            WHERE guild_id = $1

            "#,

        )

        .bind(guild_id)

        .fetch_all(&self.pool)

        .await?;

        Ok(rows)
    }

    pub async fn delete_custom_automation(
        &self,

        guild_id: &str,

        automation_id: Uuid,
    ) -> anyhow::Result<bool> {
        let res = sqlx::query("DELETE FROM custom_automations WHERE id = $1 AND guild_id = $2")
            .bind(automation_id)
            .bind(guild_id)
            .execute(&self.pool)
            .await?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn get_sticky_message(
        &self,

        guild_id: &str,

        channel_id: &str,
    ) -> anyhow::Result<Option<StickyMessage>> {
        let row = sqlx::query_as::<_, StickyMessage>(
            r#"

            SELECT guild_id, channel_id, message_id, content, cooldown_seconds, last_updated_at

            FROM sticky_messages

            WHERE guild_id = $1 AND channel_id = $2

            "#,
        )
        .bind(guild_id)
        .bind(channel_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn set_sticky_message(
        &self,

        guild_id: &str,

        channel_id: &str,

        message_id: &str,

        content: &str,

        cooldown_seconds: i32,
    ) -> anyhow::Result<StickyMessage> {
        let now = Utc::now();

        let rec = sqlx::query_as::<_, StickyMessage>(

            r#"

            INSERT INTO sticky_messages (guild_id, channel_id, message_id, content, cooldown_seconds, last_updated_at)

            VALUES ($1, $2, $3, $4, $5, $6)

            ON CONFLICT (guild_id, channel_id) DO UPDATE

            SET message_id = $3, content = $4, cooldown_seconds = $5, last_updated_at = $6

            RETURNING guild_id, channel_id, message_id, content, cooldown_seconds, last_updated_at

            "#,

        )

        .bind(guild_id)

        .bind(channel_id)

        .bind(message_id)

        .bind(content)

        .bind(cooldown_seconds)

        .bind(now)

        .fetch_one(&self.pool)

        .await?;

        Ok(rec)
    }

    pub async fn delete_sticky_message(
        &self,

        guild_id: &str,

        channel_id: &str,
    ) -> anyhow::Result<bool> {
        let res =
            sqlx::query("DELETE FROM sticky_messages WHERE guild_id = $1 AND channel_id = $2")
                .bind(guild_id)
                .bind(channel_id)
                .execute(&self.pool)
                .await?;

        Ok(res.rows_affected() > 0)
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
        let id = Uuid::new_v4();

        let tallies = vec![0i64; options.len()];

        let options_json = serde_json::json!(options);

        let tallies_json = serde_json::json!(tallies);

        let now = Utc::now();

        let rec = sqlx::query_as::<_, CommunityPoll>(

            r#"

            INSERT INTO community_polls (id, guild_id, channel_id, question, options_json, tallies_json, is_anonymous, expires_at, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)

            RETURNING id, guild_id, channel_id, question, options_json, tallies_json, is_anonymous, expires_at, created_at

            "#,

        )

        .bind(id)

        .bind(guild_id)

        .bind(channel_id)

        .bind(question)

        .bind(options_json)

        .bind(tallies_json)

        .bind(is_anonymous)

        .bind(expires_at)

        .bind(now)

        .fetch_one(&self.pool)

        .await?;

        Ok(rec)
    }

    pub async fn get_community_poll(&self, poll_id: Uuid) -> anyhow::Result<Option<CommunityPoll>> {
        let row = sqlx::query_as::<_, CommunityPoll>(

            r#"

            SELECT id, guild_id, channel_id, question, options_json, tallies_json, is_anonymous, expires_at, created_at

            FROM community_polls

            WHERE id = $1

            "#,

        )

        .bind(poll_id)

        .fetch_optional(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn vote_community_poll(
        &self,

        guild_id: &str,

        poll_id: Uuid,

        user_id: &str,

        option_idx: usize,
    ) -> anyhow::Result<Option<CommunityPoll>> {
        let mut tx = self.pool.begin().await?;

        let poll = sqlx::query_as::<_, CommunityPoll>(

            r#"

            SELECT id, guild_id, channel_id, question, options_json, tallies_json, is_anonymous, expires_at, created_at

            FROM community_polls

            WHERE id = $1 AND guild_id = $2

            FOR UPDATE

            "#,

        )

        .bind(poll_id)

        .bind(guild_id)

        .fetch_optional(&mut *tx)

        .await?;

        let mut poll = match poll {
            Some(p) => p,

            None => return Ok(None),
        };

        if Utc::now() >= poll.expires_at {
            anyhow::bail!("Poll has expired");
        }

        let tallies = poll
            .tallies_json
            .as_array_mut()
            .ok_or_else(|| anyhow::anyhow!("Invalid tallies JSON in poll"))?;

        if option_idx >= tallies.len() {
            anyhow::bail!("Invalid option index");
        }

        let prev_vote = sqlx::query_scalar::<_, i32>(
            "SELECT option_idx FROM community_poll_votes WHERE poll_id = $1 AND user_id = $2",
        )
        .bind(poll_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(prev_idx) = prev_vote {
            let prev_idx = prev_idx as usize;

            if prev_idx == option_idx {
                tx.commit().await?;

                return Ok(Some(poll));
            }

            if let Some(val) = tallies.get_mut(prev_idx) {
                let count = val.as_i64().unwrap_or(0);

                *val = serde_json::json!((count - 1).max(0));
            }

            sqlx::query(

                "UPDATE community_poll_votes SET option_idx = $3, voted_at = NOW() WHERE poll_id = $1 AND user_id = $2",

            )

            .bind(poll_id)

            .bind(user_id)

            .bind(option_idx as i32)

            .execute(&mut *tx)

            .await?;
        } else {
            sqlx::query(

                "INSERT INTO community_poll_votes (poll_id, user_id, option_idx) VALUES ($1, $2, $3)",

            )

            .bind(poll_id)

            .bind(user_id)

            .bind(option_idx as i32)

            .execute(&mut *tx)

            .await?;
        }

        if let Some(val) = tallies.get_mut(option_idx) {
            let count = val.as_i64().unwrap_or(0);

            *val = serde_json::json!(count + 1);
        }

        sqlx::query(
            r#"

            UPDATE community_polls

            SET tallies_json = $2

            WHERE id = $1

            "#,
        )
        .bind(poll_id)
        .bind(&poll.tallies_json)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(Some(poll))
    }

    pub async fn create_giveaway(
        &self,

        guild_id: &str,

        channel_id: &str,

        prize: &str,

        winners_count: i32,

        expires_at: DateTime<Utc>,
    ) -> anyhow::Result<Giveaway> {
        self.create_giveaway_with_id(
            Uuid::new_v4(),
            guild_id,
            channel_id,
            prize,
            winners_count,
            expires_at,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_giveaway_with_id(
        &self,
        id: Uuid,
        guild_id: &str,
        channel_id: &str,
        prize: &str,
        winners_count: i32,
        expires_at: DateTime<Utc>,
        created_by_user_id: Option<&str>,
    ) -> anyhow::Result<Giveaway> {
        let empty_json = serde_json::json!([]);

        let now = Utc::now();

        let rec = sqlx::query_as::<_, Giveaway>(

            r#"

            INSERT INTO giveaways (id, guild_id, channel_id, prize, winners_count, participants_json, winners_json, ended, expires_at, created_at, created_by_user_id)

            VALUES ($1, $2, $3, $4, $5, $6, $6, false, $7, $8, $9)

            RETURNING id, guild_id, channel_id, prize, winners_count, participants_json, winners_json, ended, expires_at, created_at, created_by_user_id

            "#,

        )

        .bind(id)

        .bind(guild_id)

        .bind(channel_id)

        .bind(prize)

        .bind(winners_count)

        .bind(empty_json)

        .bind(expires_at)

        .bind(now)

        .bind(created_by_user_id)

        .fetch_one(&self.pool)

        .await?;

        Ok(rec)
    }

    pub async fn get_giveaway(&self, giveaway_id: Uuid) -> anyhow::Result<Option<Giveaway>> {
        let row = sqlx::query_as::<_, Giveaway>(

            r#"

            SELECT id, guild_id, channel_id, prize, winners_count, participants_json, winners_json, ended, expires_at, created_at, created_by_user_id

            FROM giveaways

            WHERE id = $1

            "#,

        )

        .bind(giveaway_id)

        .fetch_optional(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn join_giveaway(&self, giveaway_id: Uuid, user_id: &str) -> anyhow::Result<bool> {
        let mut tx = self.pool.begin().await?;

        let ga = sqlx::query_as::<_, Giveaway>(

            r#"

            SELECT id, guild_id, channel_id, prize, winners_count, participants_json, winners_json, ended, expires_at, created_at, created_by_user_id

            FROM giveaways

            WHERE id = $1

            FOR UPDATE

            "#,

        )

        .bind(giveaway_id)

        .fetch_optional(&mut *tx)

        .await?;

        let mut ga = match ga {
            Some(g) if !g.ended && chrono::Utc::now() < g.expires_at => g,

            _ => return Ok(false),
        };

        let allow_self_entry = self
            .get_module_settings(&ga.guild_id, "giveaway")
            .await
            .ok()
            .flatten()
            .and_then(|s| s.config.get("allow_self_entry").and_then(|v| v.as_bool()))
            .unwrap_or(false);

        if !allow_self_entry && ga.created_by_user_id.as_deref() == Some(user_id) {
            return Ok(false);
        }

        let list = match ga.participants_json.as_array_mut() {
            Some(l) => l,

            None => return Ok(false),
        };

        let user_val = serde_json::json!(user_id);

        if list.contains(&user_val) {
            return Ok(false);
        }

        list.push(user_val);

        sqlx::query(
            r#"

            UPDATE giveaways

            SET participants_json = $2

            WHERE id = $1

            "#,
        )
        .bind(giveaway_id)
        .bind(&ga.participants_json)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(true)
    }

    pub async fn finish_giveaway(&self, giveaway_id: Uuid) -> anyhow::Result<Vec<String>> {
        let mut tx = self.pool.begin().await?;

        let ga = sqlx::query_as::<_, Giveaway>(
            r#"
            SELECT id, guild_id, channel_id, prize, winners_count, participants_json, winners_json, ended, expires_at, created_at, created_by_user_id
            FROM giveaways
            WHERE id = $1
            FOR UPDATE
            "#,
        )
        .bind(giveaway_id)
        .fetch_optional(&mut *tx)
        .await?;

        let mut ga = match ga {
            Some(g) => g,
            None => anyhow::bail!("Giveaway was not found"),
        };

        if ga.ended {
            tx.commit().await?;
            let existing: Vec<String> = ga
                .winners_json
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                        .collect()
                })
                .unwrap_or_default();
            return Ok(existing);
        }

        let participants: Vec<String> = ga
            .participants_json
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                    .collect()
            })
            .unwrap_or_default();

        if participants.is_empty() {
            sqlx::query(
                r#"
                UPDATE giveaways
                SET winners_json = '[]'::jsonb, ended = true
                WHERE id = $1
                "#,
            )
            .bind(giveaway_id)
            .execute(&mut *tx)
            .await?;

            tx.commit().await?;
            return Ok(Vec::new());
        }

        let count = (ga.winners_count as usize).min(participants.len());
        let mut pool = participants.clone();
        let mut winners = Vec::with_capacity(count);

        for i in 0..count {
            let remaining = pool.len() - i;
            let offset = if remaining <= 1 {
                0
            } else {
                let uuid = uuid::Uuid::new_v4();
                let rand_val =
                    u64::from_le_bytes(uuid.as_bytes()[0..8].try_into().unwrap()) as usize;
                rand_val % remaining
            };
            let j = i + offset;
            pool.swap(i, j);
            winners.push(pool[i].clone());
        }

        ga.winners_json = serde_json::json!(winners);

        sqlx::query(
            r#"
            UPDATE giveaways
            SET winners_json = $2, ended = true
            WHERE id = $1
            "#,
        )
        .bind(giveaway_id)
        .bind(&ga.winners_json)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(winners)
    }

    pub async fn reroll_giveaway(&self, giveaway_id: Uuid) -> anyhow::Result<Vec<String>> {
        let mut tx = self.pool.begin().await?;

        let ga = sqlx::query_as::<_, Giveaway>(
            r#"
            SELECT id, guild_id, channel_id, prize, winners_count, participants_json, winners_json, ended, expires_at, created_at, created_by_user_id
            FROM giveaways
            WHERE id = $1
            FOR UPDATE
            "#,
        )
        .bind(giveaway_id)
        .fetch_optional(&mut *tx)
        .await?;

        let mut ga = match ga {
            Some(g) => g,
            None => anyhow::bail!("Giveaway was not found"),
        };

        anyhow::ensure!(
            ga.ended,
            "Giveaway is still active; finish it before reroll"
        );

        let participants: Vec<String> = ga
            .participants_json
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                    .collect()
            })
            .unwrap_or_default();

        anyhow::ensure!(
            !participants.is_empty(),
            "Giveaway has no participants to reroll"
        );

        let previous_winners: std::collections::HashSet<String> = ga
            .winners_json
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                    .collect()
            })
            .unwrap_or_default();

        let count = (ga.winners_count as usize).min(participants.len());

        let mut non_winners: Vec<String> = participants
            .iter()
            .filter(|p| !previous_winners.contains(*p))
            .cloned()
            .collect();

        let shuffle = |items: &mut [String]| {
            let len = items.len();
            for i in 0..len {
                let remaining = len - i;
                if remaining > 1 {
                    let uuid = uuid::Uuid::new_v4();
                    let rand_val =
                        u64::from_le_bytes(uuid.as_bytes()[0..8].try_into().unwrap()) as usize;
                    let offset = rand_val % remaining;
                    items.swap(i, i + offset);
                }
            }
        };

        shuffle(&mut non_winners);

        let mut winners = Vec::with_capacity(count);
        for item in non_winners.into_iter().take(count) {
            winners.push(item);
        }

        if winners.len() < count {
            let mut remaining_pool: Vec<String> = participants
                .iter()
                .filter(|p| !winners.contains(*p))
                .cloned()
                .collect();
            shuffle(&mut remaining_pool);
            for item in remaining_pool.into_iter().take(count - winners.len()) {
                winners.push(item);
            }
        }

        ga.winners_json = serde_json::json!(winners);

        sqlx::query(
            r#"
            UPDATE giveaways
            SET winners_json = $2, ended = true
            WHERE id = $1
            "#,
        )
        .bind(giveaway_id)
        .bind(&ga.winners_json)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(winners)
    }

    pub async fn finish_expired_giveaways(&self) -> anyhow::Result<Vec<(Giveaway, Vec<String>)>> {
        let expired_ids = sqlx::query_scalar::<_, Uuid>(
            r#"
            SELECT id
            FROM giveaways
            WHERE ended = false AND expires_at <= NOW()
            ORDER BY expires_at ASC
            LIMIT 50
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut results = Vec::new();
        for id in expired_ids {
            match self.finish_giveaway(id).await {
                Ok(winners) => {
                    if let Ok(Some(ga)) = self.get_giveaway(id).await {
                        results.push((ga, winners));
                    }
                }
                Err(err) => {
                    tracing::warn!(giveaway_id = %id, ?err, "Failed to finish expired giveaway");
                }
            }
        }
        Ok(results)
    }

    pub async fn get_tag(&self, guild_id: &str, name: &str) -> anyhow::Result<Option<TagRecord>> {
        let row = sqlx::query_scalar::<_, Value>(
            r#"
            SELECT value
            FROM module_kv
            WHERE guild_id = $1 AND module_id = 'tags' AND key = $2
            "#,
        )
        .bind(guild_id)
        .bind(name.to_lowercase())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(val) => {
                let tag: TagRecord = serde_json::from_value(val)?;
                Ok(Some(tag))
            }
            None => Ok(None),
        }
    }

    pub async fn set_tag(&self, guild_id: &str, tag: TagRecord) -> anyhow::Result<()> {
        let val = serde_json::to_value(&tag)?;
        sqlx::query(
            r#"
            INSERT INTO module_kv (guild_id, module_id, key, value, updated_at)
            VALUES ($1, 'tags', $2, $3, NOW())
            ON CONFLICT (guild_id, module_id, key)
            DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()
            "#,
        )
        .bind(guild_id)
        .bind(tag.name.to_lowercase())
        .bind(val)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_tag(&self, guild_id: &str, name: &str) -> anyhow::Result<bool> {
        let res = sqlx::query(
            r#"
            DELETE FROM module_kv
            WHERE guild_id = $1 AND module_id = 'tags' AND key = $2
            "#,
        )
        .bind(guild_id)
        .bind(name.to_lowercase())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn upsert_reaction_role_binding(
        &self,
        binding: ReactionRoleBinding,
    ) -> anyhow::Result<ReactionRoleBinding> {
        let row = sqlx::query_as::<_, ReactionRoleBinding>(
            r#"
            INSERT INTO reaction_role_bindings (id, guild_id, channel_id, message_id, emoji_key, role_id, created_by, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (guild_id, message_id, emoji_key)
            DO UPDATE SET
                role_id = EXCLUDED.role_id,
                channel_id = COALESCE(EXCLUDED.channel_id, reaction_role_bindings.channel_id),
                created_by = EXCLUDED.created_by
            RETURNING id, guild_id, channel_id, message_id, emoji_key, role_id, created_by, created_at
            "#,
        )
        .bind(binding.id)
        .bind(&binding.guild_id)
        .bind(&binding.channel_id)
        .bind(&binding.message_id)
        .bind(&binding.emoji_key)
        .bind(&binding.role_id)
        .bind(&binding.created_by)
        .bind(binding.created_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_reaction_role_binding(
        &self,
        guild_id: &str,
        message_id: &str,
        emoji_key: &str,
    ) -> anyhow::Result<Option<ReactionRoleBinding>> {
        let row = sqlx::query_as::<_, ReactionRoleBinding>(
            r#"
            SELECT id, guild_id, channel_id, message_id, emoji_key, role_id, created_by, created_at
            FROM reaction_role_bindings
            WHERE guild_id = $1 AND message_id = $2 AND emoji_key = $3
            "#,
        )
        .bind(guild_id)
        .bind(message_id)
        .bind(emoji_key)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_reaction_role_binding(
        &self,
        guild_id: &str,
        message_id: &str,
        emoji_key: &str,
    ) -> anyhow::Result<bool> {
        let res = sqlx::query(
            r#"
            DELETE FROM reaction_role_bindings
            WHERE guild_id = $1 AND message_id = $2 AND emoji_key = $3
            "#,
        )
        .bind(guild_id)
        .bind(message_id)
        .bind(emoji_key)
        .execute(&self.pool)
        .await?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn delete_reaction_role_binding_by_id(
        &self,
        guild_id: &str,
        binding_id: Uuid,
    ) -> anyhow::Result<bool> {
        let res = sqlx::query(
            r#"
            DELETE FROM reaction_role_bindings
            WHERE guild_id = $1 AND id = $2
            "#,
        )
        .bind(guild_id)
        .bind(binding_id)
        .execute(&self.pool)
        .await?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn list_reaction_role_bindings(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<ReactionRoleBinding>> {
        let rows = sqlx::query_as::<_, ReactionRoleBinding>(
            r#"
            SELECT id, guild_id, channel_id, message_id, emoji_key, role_id, created_by, created_at
            FROM reaction_role_bindings
            WHERE guild_id = $1
            ORDER BY created_at ASC
            "#,
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn record_starboard_entry(
        &self,

        guild_id: &str,

        source_channel_id: &str,

        source_message_id: &str,

        starboard_message_id: &str,

        reaction_count: i32,
    ) -> anyhow::Result<StarboardEntry> {
        let now = Utc::now();

        let rec = sqlx::query_as::<_, StarboardEntry>(

            r#"

            INSERT INTO starboard_entries (guild_id, source_channel_id, source_message_id, starboard_message_id, reaction_count, created_at)

            VALUES ($1, $2, $3, $4, $5, $6)

            ON CONFLICT (guild_id, source_message_id) DO UPDATE

            SET reaction_count = $5, starboard_message_id = $4

            RETURNING guild_id, source_channel_id, source_message_id, starboard_message_id, reaction_count, created_at

            "#,

        )

        .bind(guild_id)

        .bind(source_channel_id)

        .bind(source_message_id)

        .bind(starboard_message_id)

        .bind(reaction_count)

        .bind(now)

        .fetch_one(&self.pool)

        .await?;

        Ok(rec)
    }

    pub async fn get_starboard_entry(
        &self,

        guild_id: &str,

        source_message_id: &str,
    ) -> anyhow::Result<Option<StarboardEntry>> {
        let row = sqlx::query_as::<_, StarboardEntry>(

            r#"

            SELECT guild_id, source_channel_id, source_message_id, starboard_message_id, reaction_count, created_at

            FROM starboard_entries

            WHERE guild_id = $1 AND source_message_id = $2

            "#,

        )

        .bind(guild_id)

        .bind(source_message_id)

        .fetch_optional(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn list_store_items(&self, guild_id: &str) -> anyhow::Result<Vec<StoreItem>> {
        let rows = sqlx::query_as::<_, StoreItem>(
            r#"

            SELECT id, guild_id, name, description, price, role_id, stock, created_at

            FROM store_items

            WHERE guild_id = $1

            "#,
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
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

        let id = Uuid::new_v4();

        let now = Utc::now();

        let rec = sqlx::query_as::<_, StoreItem>(

            r#"

            INSERT INTO store_items (id, guild_id, name, description, price, role_id, stock, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)

            RETURNING id, guild_id, name, description, price, role_id, stock, created_at

            "#,

        )

        .bind(id)

        .bind(guild_id)

        .bind(name)

        .bind(description)

        .bind(price)

        .bind(role_id)

        .bind(stock)

        .bind(now)

        .fetch_one(&self.pool)

        .await?;

        Ok(rec)
    }

    pub async fn purchase_store_item(
        &self,
        guild_id: &str,
        user_id: &str,
        item_id: Uuid,
        interaction_id: Option<&str>,
    ) -> anyhow::Result<(StoreItem, StorePurchase, i64)> {
        let mut tx = self.pool.begin().await?;

        if let Some(int_id) = interaction_id {
            let existing_req = sqlx::query_as::<_, (String, Uuid, Uuid, i64)>(
                r#"
                SELECT user_id, item_id, purchase_id, balance_after
                FROM economy_purchase_requests
                WHERE guild_id = $1 AND interaction_id = $2
                FOR UPDATE
                "#,
            )
            .bind(guild_id)
            .bind(int_id)
            .fetch_optional(&mut *tx)
            .await?;

            if let Some((existing_user, existing_item, purchase_id, balance_after)) = existing_req {
                if existing_user != user_id || existing_item != item_id {
                    return Err(anyhow::anyhow!(
                        "idempotency conflict: interaction_id '{int_id}' was already used for a different purchase"
                    ));
                }

                let item = sqlx::query_as::<_, StoreItem>(
                    r#"
                    SELECT id, guild_id, name, description, price, role_id, stock, created_at
                    FROM store_items
                    WHERE id = $1
                    "#,
                )
                .bind(item_id)
                .fetch_one(&mut *tx)
                .await?;

                let purchase = sqlx::query_as::<_, StorePurchase>(
                    r#"
                    SELECT id, guild_id, user_id, item_id, price_paid, role_id, fulfillment_status, failure_reason, fulfilled_at, created_at
                    FROM store_purchases
                    WHERE id = $1
                    "#,
                )
                .bind(purchase_id)
                .fetch_one(&mut *tx)
                .await?;

                tx.commit().await?;
                return Ok((item, purchase, balance_after));
            }
        }

        let item = sqlx::query_as::<_, StoreItem>(
            r#"
            SELECT id, guild_id, name, description, price, role_id, stock, created_at
            FROM store_items
            WHERE id = $1
            FOR UPDATE
            "#,
        )
        .bind(item_id)
        .fetch_optional(&mut *tx)
        .await?;

        let mut item = item.ok_or_else(|| anyhow::anyhow!("store item not found"))?;

        if item.guild_id != guild_id {
            return Err(anyhow::anyhow!("item does not belong to this guild"));
        }

        anyhow::ensure!(
            item.price >= 0,
            "stored store item has invalid negative price"
        );

        if let Some(ref r_id) = item.role_id {
            validate_snowflake("role_id", r_id)?;
        }

        if let Some(stock) = item.stock {
            if stock <= 0 {
                return Err(anyhow::anyhow!("item is out of stock"));
            }

            item.stock = Some(stock - 1);

            sqlx::query("UPDATE store_items SET stock = $2 WHERE id = $1")
                .bind(item_id)
                .bind(item.stock)
                .execute(&mut *tx)
                .await?;
        }

        ensure_profile(&mut tx, guild_id, user_id).await?;

        let mut profile = lock_profile(&mut tx, guild_id, user_id).await?;

        if profile.coins < item.price {
            return Err(anyhow::anyhow!("insufficient coins balance"));
        }

        profile.coins -= item.price;

        let now = Utc::now();
        profile.updated_at = now;

        write_profile(&mut tx, &profile).await?;

        let purchase_id = Uuid::new_v4();
        let initial_status = if item.role_id.is_some() {
            "pending"
        } else {
            "completed"
        };
        let fulfilled_at = if item.role_id.is_none() {
            Some(now)
        } else {
            None
        };

        let purchase = sqlx::query_as::<_, StorePurchase>(
            r#"
            INSERT INTO store_purchases (id, guild_id, user_id, item_id, price_paid, role_id, fulfillment_status, failure_reason, fulfilled_at, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING id, guild_id, user_id, item_id, price_paid, role_id, fulfillment_status, failure_reason, fulfilled_at, created_at
            "#,
        )
        .bind(purchase_id)
        .bind(guild_id)
        .bind(user_id)
        .bind(item_id)
        .bind(item.price)
        .bind(&item.role_id)
        .bind(initial_status)
        .bind(Option::<String>::None)
        .bind(fulfilled_at)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;

        if let Some(int_id) = interaction_id {
            let res = sqlx::query(
                r#"
                INSERT INTO economy_purchase_requests (guild_id, interaction_id, user_id, item_id, purchase_id, balance_after, created_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                ON CONFLICT (guild_id, interaction_id) DO NOTHING
                "#,
            )
            .bind(guild_id)
            .bind(int_id)
            .bind(user_id)
            .bind(item_id)
            .bind(purchase_id)
            .bind(profile.coins)
            .bind(now)
            .execute(&mut *tx)
            .await?;

            if res.rows_affected() == 0 {
                tx.rollback().await?;
                return Box::pin(self.purchase_store_item(
                    guild_id,
                    user_id,
                    item_id,
                    Some(int_id),
                ))
                .await;
            }
        }

        tx.commit().await?;

        Ok((item, purchase, profile.coins))
    }

    pub async fn get_store_purchase(
        &self,
        purchase_id: Uuid,
    ) -> anyhow::Result<Option<StorePurchase>> {
        let rec = sqlx::query_as::<_, StorePurchase>(
            r#"
            SELECT id, guild_id, user_id, item_id, price_paid, role_id, fulfillment_status, failure_reason, fulfilled_at, created_at
            FROM store_purchases
            WHERE id = $1
            "#,
        )
        .bind(purchase_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(rec)
    }

    pub async fn list_store_purchases(
        &self,
        guild_id: &str,
        user_id: Option<&str>,
    ) -> anyhow::Result<Vec<StorePurchase>> {
        let recs = sqlx::query_as::<_, StorePurchase>(
            r#"
            SELECT id, guild_id, user_id, item_id, price_paid, role_id, fulfillment_status, failure_reason, fulfilled_at, created_at
            FROM store_purchases
            WHERE guild_id = $1 AND ($2 IS NULL OR user_id = $2)
            ORDER BY created_at DESC
            "#,
        )
        .bind(guild_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(recs)
    }

    pub async fn claim_store_role_delivery(&self, purchase_id: Uuid) -> anyhow::Result<bool> {
        let now = Utc::now();
        let lease_until = now + chrono::Duration::seconds(60);

        let res = sqlx::query(
            r#"
            UPDATE store_purchases
            SET fulfillment_status = 'processing',
                lease_expires_at = $2
            WHERE id = $1
              AND (
                  fulfillment_status = 'pending'
                  OR (fulfillment_status = 'processing' AND lease_expires_at IS NOT NULL AND lease_expires_at < $3)
              )
              AND role_id IS NOT NULL
            "#,
        )
        .bind(purchase_id)
        .bind(lease_until)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(res.rows_affected() == 1)
    }

    pub async fn update_store_purchase_fulfillment(
        &self,
        purchase_id: Uuid,
        status: &str,
        failure_reason: Option<&str>,
        fulfilled_at: Option<DateTime<Utc>>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            UPDATE store_purchases
            SET fulfillment_status = $2, failure_reason = $3, fulfilled_at = $4, lease_expires_at = NULL
            WHERE id = $1 AND fulfillment_status != 'completed'
            "#,
        )
        .bind(purchase_id)
        .bind(status)
        .bind(failure_reason)
        .bind(fulfilled_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn prune_economy_purchase_requests(
        &self,
        cutoff: DateTime<Utc>,
        limit: usize,
    ) -> anyhow::Result<u64> {
        let chunk_limit = (limit as i64).clamp(1, 5000);
        let mut total_deleted = 0u64;
        loop {
            let res = sqlx::query(
                r#"
                WITH old_keys AS (
                    SELECT guild_id, interaction_id
                    FROM economy_purchase_requests
                    WHERE created_at < $1
                    ORDER BY created_at
                    LIMIT $2
                    FOR UPDATE SKIP LOCKED
                )
                DELETE FROM economy_purchase_requests
                USING old_keys
                WHERE economy_purchase_requests.guild_id = old_keys.guild_id
                  AND economy_purchase_requests.interaction_id = old_keys.interaction_id
                "#,
            )
            .bind(cutoff)
            .bind(chunk_limit)
            .execute(&self.pool)
            .await?;

            let affected = res.rows_affected();
            total_deleted += affected;
            if (affected as usize) < (chunk_limit as usize) || total_deleted >= (limit as u64) {
                break;
            }
        }
        Ok(total_deleted)
    }

    pub async fn prune_economy_idempotency(
        &self,
        cutoff: DateTime<Utc>,
        limit: usize,
    ) -> anyhow::Result<u64> {
        let chunk_limit = (limit as i64).clamp(1, 5000);
        let mut total_deleted = 0u64;
        loop {
            let res = sqlx::query(
                r#"
                WITH old_keys AS (
                    SELECT guild_id, interaction_id
                    FROM economy_idempotency
                    WHERE created_at < $1
                    ORDER BY created_at
                    LIMIT $2
                    FOR UPDATE SKIP LOCKED
                )
                DELETE FROM economy_idempotency
                USING old_keys
                WHERE economy_idempotency.guild_id = old_keys.guild_id
                  AND economy_idempotency.interaction_id = old_keys.interaction_id
                "#,
            )
            .bind(cutoff)
            .bind(chunk_limit)
            .execute(&self.pool)
            .await?;

            let affected = res.rows_affected();
            total_deleted += affected;
            if (affected as usize) < (chunk_limit as usize) || total_deleted >= (limit as u64) {
                break;
            }
        }
        Ok(total_deleted)
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
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("user_id", user_id)?;
        anyhow::ensure!(
            kind == "username" || kind == "global_name" || kind == "nickname",
            "invalid identity kind: {kind}"
        );

        let old_clean = old_value.map(|s| s.trim()).filter(|s| !s.is_empty());
        let new_clean = new_value.map(|s| s.trim()).filter(|s| !s.is_empty());

        if old_clean == new_clean {
            return Ok(None);
        }

        let retention = if retention_days > 0 {
            retention_days
        } else {
            90
        };
        let now = Utc::now();
        let expires_at = now + Duration::days(retention);
        let id = Uuid::new_v4();

        let rec = sqlx::query_as::<_, MemberIdentityRecord>(
            r#"
            INSERT INTO member_identity_history (id, guild_id, user_id, kind, old_value, new_value, observed_at, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING id, guild_id, user_id, kind, old_value, new_value, observed_at, expires_at
            "#
        )
        .bind(id)
        .bind(guild_id)
        .bind(user_id)
        .bind(kind)
        .bind(old_clean)
        .bind(new_clean)
        .bind(now)
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(Some(rec))
    }

    pub async fn get_member_identity_history(
        &self,
        guild_id: &str,
        user_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<MemberIdentityRecord>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("user_id", user_id)?;

        let max_limit = limit.clamp(1, 100) as i64;
        let recs = sqlx::query_as::<_, MemberIdentityRecord>(
            r#"
            SELECT id, guild_id, user_id, kind, old_value, new_value, observed_at, expires_at
            FROM member_identity_history
            WHERE guild_id = $1 AND user_id = $2 AND expires_at > NOW()
            ORDER BY observed_at DESC
            LIMIT $3
            "#,
        )
        .bind(guild_id)
        .bind(user_id)
        .bind(max_limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(recs)
    }

    pub async fn cleanup_expired_identity_history(&self) -> anyhow::Result<u64> {
        let mut total_deleted = 0u64;
        loop {
            let res = sqlx::query(
                r#"
                DELETE FROM member_identity_history
                WHERE id IN (
                    SELECT id FROM member_identity_history
                    WHERE expires_at <= NOW()
                    LIMIT 1000
                )
                "#,
            )
            .execute(&self.pool)
            .await?;

            let affected = res.rows_affected();
            total_deleted += affected;
            if affected < 1000 {
                break;
            }
        }

        Ok(total_deleted)
    }

    pub async fn upsert_member_snapshot(
        &self,
        params: UpsertMemberSnapshotParams<'_>,
    ) -> anyhow::Result<MemberSnapshot> {
        let u_num = validate_snowflake("user_id", params.user_id)?;
        validate_snowflake("guild_id", params.guild_id)?;

        let account_created_at = snowflake_created_at(u_num);
        let roles_json =
            serde_json::to_value(params.role_ids).unwrap_or_else(|_| serde_json::json!([]));
        let now = Utc::now();

        let rec = sqlx::query_as::<_, MemberSnapshot>(
            r#"
            INSERT INTO member_snapshots (guild_id, user_id, username, global_name, nickname, role_ids, joined_at, account_created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (guild_id, user_id) DO UPDATE SET
                username = EXCLUDED.username,
                global_name = EXCLUDED.global_name,
                nickname = EXCLUDED.nickname,
                role_ids = EXCLUDED.role_ids,
                joined_at = COALESCE(EXCLUDED.joined_at, member_snapshots.joined_at),
                account_created_at = EXCLUDED.account_created_at,
                updated_at = EXCLUDED.updated_at
            RETURNING guild_id, user_id, username, global_name, nickname, role_ids, joined_at, account_created_at, updated_at
            "#
        )
        .bind(params.guild_id)
        .bind(params.user_id)
        .bind(params.username)
        .bind(params.global_name)
        .bind(params.nickname)
        .bind(roles_json)
        .bind(params.joined_at)
        .bind(account_created_at)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;

        Ok(rec)
    }

    pub async fn remove_member_snapshot(
        &self,
        guild_id: &str,
        user_id: &str,
    ) -> anyhow::Result<bool> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("user_id", user_id)?;

        let result =
            sqlx::query("DELETE FROM member_snapshots WHERE guild_id = $1 AND user_id = $2")
                .bind(guild_id)
                .bind(user_id)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn search_members(
        &self,
        guild_id: &str,
        filter: &MemberSearchFilter,
    ) -> anyhow::Result<Vec<MemberSearchRow>> {
        validate_snowflake("guild_id", guild_id)?;

        let limit = filter.limit.unwrap_or(25).clamp(1, 100) as i64;

        // Keyset cursor extraction with legacy user_id fallback
        let (cursor_joined_at, cursor_user_id) = if let Some(ref cur) = filter.cursor {
            if let Some((parsed_joined, parsed_user)) = parse_member_search_cursor(cur) {
                if parsed_joined.is_none() && cur.chars().all(|c| c.is_ascii_digit()) {
                    let row_opt: Option<(Option<DateTime<Utc>>,)> = sqlx::query_as(
                        "SELECT joined_at FROM member_snapshots WHERE guild_id = $1 AND user_id = $2",
                    )
                    .bind(guild_id)
                    .bind(&parsed_user)
                    .fetch_optional(&self.pool)
                    .await?;
                    let resolved_joined = row_opt.and_then(|r| r.0);
                    (resolved_joined, Some(parsed_user))
                } else {
                    (parsed_joined, Some(parsed_user))
                }
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        #[derive(sqlx::FromRow)]
        struct RawSearchRow {
            user_id: String,
            username: String,
            global_name: Option<String>,
            nickname: Option<String>,
            role_ids: serde_json::Value,
            joined_at: Option<DateTime<Utc>>,
            account_created_at: Option<DateTime<Utc>>,
            moderation_case_count: i64,
            xp: Option<i64>,
            level: Option<i32>,
            message_count: Option<i64>,
            recent_name_changes: i64,
        }

        let rows = sqlx::query_as::<_, RawSearchRow>(
            r#"
            SELECT
                ms.user_id,
                ms.username,
                ms.global_name,
                ms.nickname,
                ms.role_ids,
                ms.joined_at,
                ms.account_created_at,
                COALESCE(mc.case_count, 0) as moderation_case_count,
                mp.xp,
                mp.level,
                mp.messages as message_count,
                COALESCE(mih.change_count, 0) as recent_name_changes
            FROM member_snapshots ms
            LEFT JOIN member_progress mp
                ON ms.guild_id = mp.guild_id AND ms.user_id = mp.user_id
            LEFT JOIN (
                SELECT subject_user_id, COUNT(*) as case_count
                FROM moderation_cases
                WHERE guild_id = $1
                GROUP BY subject_user_id
            ) mc ON ms.user_id = mc.subject_user_id
            LEFT JOIN (
                SELECT user_id, COUNT(*) as change_count, MAX(observed_at) as last_change
                FROM member_identity_history
                WHERE guild_id = $1 AND expires_at > NOW()
                GROUP BY user_id
            ) mih ON ms.user_id = mih.user_id
            WHERE ms.guild_id = $1
              AND ($2::text[] IS NULL OR cardinality($2::text[]) = 0 OR ms.role_ids ?& $2::text[])
              AND ($3::text[] IS NULL OR cardinality($3::text[]) = 0 OR NOT (ms.role_ids ?| $3::text[]))
              AND ($4::timestamptz IS NULL OR ms.joined_at >= $4)
              AND ($5::timestamptz IS NULL OR ms.joined_at <= $5)
              AND ($6::timestamptz IS NULL OR ms.account_created_at >= $6)
              AND ($7::timestamptz IS NULL OR ms.account_created_at <= $7)
              AND ($8::bigint IS NULL OR COALESCE(mc.case_count, 0) >= $8)
              AND ($9::timestamptz IS NULL OR mih.last_change >= $9)
              AND (
                $10::text IS NULL
                OR (
                    $11::timestamptz IS NOT NULL AND (
                        ms.joined_at < $11
                        OR (ms.joined_at = $11 AND ms.user_id > $10)
                        OR ms.joined_at IS NULL
                    )
                )
                OR (
                    $11::timestamptz IS NULL AND (
                        ms.joined_at IS NULL
                        AND ms.user_id > $10
                    )
                )
              )
            ORDER BY ms.joined_at DESC NULLS LAST, ms.user_id ASC
            LIMIT $12
            "#,
        )
        .bind(guild_id)
        .bind(&filter.role_ids)
        .bind(&filter.exclude_role_ids)
        .bind(filter.joined_after)
        .bind(filter.joined_before)
        .bind(filter.account_created_after)
        .bind(filter.account_created_before)
        .bind(filter.min_case_count)
        .bind(filter.identity_changed_after)
        .bind(cursor_user_id.as_deref())
        .bind(cursor_joined_at)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        let mut matched = Vec::with_capacity(rows.len());
        for r in rows {
            let member_roles: Vec<String> = serde_json::from_value(r.role_ids).unwrap_or_default();
            matched.push(MemberSearchRow {
                user_id: r.user_id,
                username: r.username,
                global_name: r.global_name,
                nickname: r.nickname,
                role_ids: member_roles,
                joined_at: r.joined_at,
                account_created_at: r.account_created_at,
                moderation_case_count: r.moderation_case_count,
                xp: r.xp,
                level: r.level,
                message_count: r.message_count,
                recent_name_changes: r.recent_name_changes,
            });
        }

        Ok(matched)
    }

    pub async fn create_cleanup_plan(
        &self,
        guild_id: &str,
        channel_id: &str,
        actor_user_id: &str,
        message_ids: &[String],
        ttl_seconds: i64,
    ) -> anyhow::Result<CleanupPlan> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("channel_id", channel_id)?;
        validate_snowflake("actor_user_id", actor_user_id)?;

        let id = Uuid::new_v4();
        let now = Utc::now();
        let expires_at = now + chrono::Duration::seconds(ttl_seconds);
        let msg_json = serde_json::to_value(message_ids)?;

        let plan = sqlx::query_as::<_, CleanupPlan>(
            r#"
            INSERT INTO cleanup_plans (id, guild_id, channel_id, actor_user_id, message_ids, created_at, expires_at, consumed_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, NULL)
            RETURNING id, guild_id, channel_id, actor_user_id, message_ids, created_at, expires_at, consumed_at
            "#
        )
        .bind(id)
        .bind(guild_id)
        .bind(channel_id)
        .bind(actor_user_id)
        .bind(msg_json)
        .bind(now)
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(plan)
    }

    pub async fn get_cleanup_plan(
        &self,
        guild_id: &str,
        plan_id: Uuid,
    ) -> anyhow::Result<Option<CleanupPlan>> {
        validate_snowflake("guild_id", guild_id)?;

        let plan = sqlx::query_as::<_, CleanupPlan>(
            r#"
            SELECT id, guild_id, channel_id, actor_user_id, message_ids, created_at, expires_at, consumed_at
            FROM cleanup_plans
            WHERE id = $1 AND guild_id = $2
            "#
        )
        .bind(plan_id)
        .bind(guild_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(plan)
    }

    pub async fn consume_cleanup_plan(
        &self,
        guild_id: &str,
        plan_id: Uuid,
        actor_user_id: &str,
    ) -> anyhow::Result<Option<CleanupPlan>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("actor_user_id", actor_user_id)?;

        let now = Utc::now();
        let plan = sqlx::query_as::<_, CleanupPlan>(
            r#"
            UPDATE cleanup_plans
            SET consumed_at = $1
            WHERE id = $2 AND guild_id = $3 AND actor_user_id = $4 AND consumed_at IS NULL AND expires_at >= $1
            RETURNING id, guild_id, channel_id, actor_user_id, message_ids, created_at, expires_at, consumed_at
            "#
        )
        .bind(now)
        .bind(plan_id)
        .bind(guild_id)
        .bind(actor_user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(plan)
    }

    pub async fn list_companion_mappings(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<CompanionChannelMapping>> {
        validate_snowflake("guild_id", guild_id)?;
        let rows = sqlx::query_as::<_, CompanionChannelMapping>(
            r#"
            SELECT id, guild_id, voice_channel_id, text_channel_id, allow_send_messages, allow_read_history, enabled, created_at
            FROM companion_channel_mappings
            WHERE guild_id = $1
            ORDER BY created_at ASC
            "#,
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn upsert_companion_mapping(
        &self,
        guild_id: &str,
        voice_channel_id: &str,
        text_channel_id: &str,
        allow_send_messages: bool,
        allow_read_history: bool,
    ) -> anyhow::Result<CompanionChannelMapping> {
        let item = CompanionMappingItem {
            voice_channel_id: voice_channel_id.to_string(),
            text_channel_id: text_channel_id.to_string(),
            allow_send_messages,
            allow_read_history,
            enabled: true,
        };
        let mut results = self
            .batch_upsert_companion_mappings(guild_id, &[item])
            .await?;
        results
            .pop()
            .ok_or_else(|| anyhow::anyhow!("Failed to upsert companion mapping"))
    }

    pub async fn batch_upsert_companion_mappings(
        &self,
        guild_id: &str,
        items: &[CompanionMappingItem],
    ) -> anyhow::Result<Vec<CompanionChannelMapping>> {
        validate_snowflake("guild_id", guild_id)?;
        if items.len() > MAX_COMPANION_MAPPINGS_PER_GUILD {
            anyhow::bail!(
                "Companion mapping limit reached (max {MAX_COMPANION_MAPPINGS_PER_GUILD} per request)"
            );
        }
        let mut seen_batch_voice = HashSet::with_capacity(items.len());
        for item in items {
            validate_snowflake("voice_channel_id", &item.voice_channel_id)?;
            validate_snowflake("text_channel_id", &item.text_channel_id)?;
            if item.voice_channel_id == item.text_channel_id {
                anyhow::bail!("Voice channel and text companion channel cannot be the same");
            }
            if !seen_batch_voice.insert(&item.voice_channel_id) {
                anyhow::bail!(
                    "Duplicate voice_channel_id in batch: {}",
                    item.voice_channel_id
                );
            }
        }

        let mut tx = self.pool.begin().await?;

        // Advisory xact lock per guild to serialize concurrent batch updates
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
            .bind(format!("companion_mapping:{}", guild_id))
            .execute(&mut *tx)
            .await?;

        let existing_voice_ids: Vec<String> = sqlx::query_scalar(
            "SELECT voice_channel_id FROM companion_channel_mappings WHERE guild_id = $1",
        )
        .bind(guild_id)
        .fetch_all(&mut *tx)
        .await?;

        let mut final_voice_ids: HashSet<String> = existing_voice_ids.into_iter().collect();
        for item in items {
            final_voice_ids.insert(item.voice_channel_id.clone());
        }

        if final_voice_ids.len() > MAX_COMPANION_MAPPINGS_PER_GUILD {
            anyhow::bail!(
                "Companion mapping limit reached (max {MAX_COMPANION_MAPPINGS_PER_GUILD} per guild)"
            );
        }

        let now = Utc::now();
        let mut saved = Vec::new();

        for item in items {
            let id = Uuid::new_v4();
            let mapping = sqlx::query_as::<_, CompanionChannelMapping>(
                r#"
                INSERT INTO companion_channel_mappings (
                    id, guild_id, voice_channel_id, text_channel_id,
                    allow_send_messages, allow_read_history, enabled, created_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                ON CONFLICT (guild_id, voice_channel_id)
                DO UPDATE SET
                    text_channel_id = EXCLUDED.text_channel_id,
                    allow_send_messages = EXCLUDED.allow_send_messages,
                    allow_read_history = EXCLUDED.allow_read_history,
                    enabled = EXCLUDED.enabled
                RETURNING id, guild_id, voice_channel_id, text_channel_id, allow_send_messages, allow_read_history, enabled, created_at
                "#,
            )
            .bind(id)
            .bind(guild_id)
            .bind(&item.voice_channel_id)
            .bind(&item.text_channel_id)
            .bind(item.allow_send_messages)
            .bind(item.allow_read_history)
            .bind(item.enabled)
            .bind(now)
            .fetch_one(&mut *tx)
            .await?;

            if !item.enabled {
                sqlx::query(
                    r#"
                    UPDATE companion_channel_leases
                    SET status = 'revoke_pending', next_attempt_at = NOW()
                    WHERE guild_id = $1
                      AND voice_channel_id = $2
                      AND status IN ('active', 'revoke_pending')
                    "#,
                )
                .bind(guild_id)
                .bind(&item.voice_channel_id)
                .execute(&mut *tx)
                .await?;
            } else {
                sqlx::query(
                    r#"
                    UPDATE companion_channel_leases
                    SET status = 'revoke_pending', next_attempt_at = NOW()
                    WHERE guild_id = $1
                      AND voice_channel_id = $2
                      AND text_channel_id <> $3
                      AND status IN ('active', 'revoke_pending')
                    "#,
                )
                .bind(guild_id)
                .bind(&item.voice_channel_id)
                .bind(&item.text_channel_id)
                .execute(&mut *tx)
                .await?;
            }

            saved.push(mapping);
        }

        tx.commit().await?;
        Ok(saved)
    }

    pub async fn delete_companion_mapping(
        &self,
        guild_id: &str,
        mapping_id: Uuid,
    ) -> anyhow::Result<bool> {
        validate_snowflake("guild_id", guild_id)?;
        let result =
            sqlx::query("DELETE FROM companion_channel_mappings WHERE id = $1 AND guild_id = $2")
                .bind(mapping_id)
                .bind(guild_id)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn get_companion_mapping_by_voice(
        &self,
        guild_id: &str,
        voice_channel_id: &str,
    ) -> anyhow::Result<Option<CompanionChannelMapping>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("voice_channel_id", voice_channel_id)?;
        let mapping = sqlx::query_as::<_, CompanionChannelMapping>(
            r#"
            SELECT id, guild_id, voice_channel_id, text_channel_id, allow_send_messages, allow_read_history, enabled, created_at
            FROM companion_channel_mappings
            WHERE guild_id = $1 AND voice_channel_id = $2
            "#,
        )
        .bind(guild_id)
        .bind(voice_channel_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(mapping)
    }

    pub async fn record_companion_lease(
        &self,
        params: RecordCompanionLeaseParams<'_>,
    ) -> anyhow::Result<CompanionChannelLease> {
        validate_snowflake("guild_id", params.guild_id)?;
        validate_snowflake("voice_channel_id", params.voice_channel_id)?;
        validate_snowflake("text_channel_id", params.text_channel_id)?;
        validate_snowflake("user_id", params.user_id)?;

        let now = Utc::now();
        let lease = sqlx::query_as::<_, CompanionChannelLease>(
            r#"
            INSERT INTO companion_channel_leases (
                guild_id, voice_channel_id, text_channel_id, user_id, granted_at, generation,
                status, revoke_attempts, next_attempt_at, last_error, revoked_at, allow_bits, deny_bits
            )
            VALUES ($1, $2, $3, $4, $5, $6, 'active', 0, NULL, NULL, NULL, $7, $8)
            ON CONFLICT (guild_id, text_channel_id, user_id)
            DO UPDATE SET
                voice_channel_id = EXCLUDED.voice_channel_id,
                granted_at = EXCLUDED.granted_at,
                generation = EXCLUDED.generation,
                status = 'active',
                revoke_attempts = 0,
                next_attempt_at = NULL,
                last_error = NULL,
                revoked_at = NULL,
                allow_bits = EXCLUDED.allow_bits,
                deny_bits = EXCLUDED.deny_bits
            RETURNING guild_id, voice_channel_id, text_channel_id, user_id, granted_at, generation,
                      status, revoke_attempts, next_attempt_at, last_error, revoked_at, allow_bits, deny_bits
            "#,
        )
        .bind(params.guild_id)
        .bind(params.voice_channel_id)
        .bind(params.text_channel_id)
        .bind(params.user_id)
        .bind(now)
        .bind(params.generation)
        .bind(params.allow_bits)
        .bind(params.deny_bits)
        .fetch_one(&self.pool)
        .await?;

        Ok(lease)
    }

    pub async fn mark_companion_lease_revoke_pending(
        &self,
        guild_id: &str,
        text_channel_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Option<CompanionChannelLease>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("text_channel_id", text_channel_id)?;
        validate_snowflake("user_id", user_id)?;

        let lease = sqlx::query_as::<_, CompanionChannelLease>(
            r#"
            UPDATE companion_channel_leases
            SET status = 'revoke_pending'
            WHERE guild_id = $1 AND text_channel_id = $2 AND user_id = $3
            RETURNING guild_id, voice_channel_id, text_channel_id, user_id, granted_at, generation,
                      status, revoke_attempts, next_attempt_at, last_error, revoked_at, allow_bits, deny_bits
            "#,
        )
        .bind(guild_id)
        .bind(text_channel_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(lease)
    }

    pub async fn record_companion_lease_failure(
        &self,
        guild_id: &str,
        text_channel_id: &str,
        user_id: &str,
        error: &str,
    ) -> anyhow::Result<()> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("text_channel_id", text_channel_id)?;
        validate_snowflake("user_id", user_id)?;

        let next_retry = Utc::now() + Duration::seconds(30);
        sqlx::query(
            r#"
            UPDATE companion_channel_leases
            SET revoke_attempts = revoke_attempts + 1,
                last_error = $4,
                next_attempt_at = $5
            WHERE guild_id = $1 AND text_channel_id = $2 AND user_id = $3
            "#,
        )
        .bind(guild_id)
        .bind(text_channel_id)
        .bind(user_id)
        .bind(error)
        .bind(next_retry)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_companion_lease(
        &self,
        guild_id: &str,
        text_channel_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Option<CompanionChannelLease>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("text_channel_id", text_channel_id)?;
        validate_snowflake("user_id", user_id)?;

        let lease = sqlx::query_as::<_, CompanionChannelLease>(
            r#"
            SELECT guild_id, voice_channel_id, text_channel_id, user_id, granted_at, generation,
                   status, revoke_attempts, next_attempt_at, last_error, revoked_at, allow_bits, deny_bits
            FROM companion_channel_leases
            WHERE guild_id = $1 AND text_channel_id = $2 AND user_id = $3
            "#,
        )
        .bind(guild_id)
        .bind(text_channel_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(lease)
    }

    pub async fn delete_companion_lease(
        &self,
        guild_id: &str,
        text_channel_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Option<CompanionChannelLease>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("text_channel_id", text_channel_id)?;
        validate_snowflake("user_id", user_id)?;

        let lease = sqlx::query_as::<_, CompanionChannelLease>(
            r#"
            DELETE FROM companion_channel_leases
            WHERE guild_id = $1 AND text_channel_id = $2 AND user_id = $3
            RETURNING guild_id, voice_channel_id, text_channel_id, user_id, granted_at, generation,
                      status, revoke_attempts, next_attempt_at, last_error, revoked_at, allow_bits, deny_bits
            "#,
        )
        .bind(guild_id)
        .bind(text_channel_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(lease)
    }

    pub async fn list_active_companion_leases(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<CompanionChannelLease>> {
        validate_snowflake("guild_id", guild_id)?;
        let leases = sqlx::query_as::<_, CompanionChannelLease>(
            r#"
            SELECT guild_id, voice_channel_id, text_channel_id, user_id, granted_at, generation,
                   status, revoke_attempts, next_attempt_at, last_error, revoked_at, allow_bits, deny_bits
            FROM companion_channel_leases
            WHERE guild_id = $1 AND status = 'active'
            ORDER BY granted_at ASC
            "#,
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(leases)
    }

    pub async fn list_pending_revoke_companion_leases(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<CompanionChannelLease>> {
        validate_snowflake("guild_id", guild_id)?;
        let leases = sqlx::query_as::<_, CompanionChannelLease>(
            r#"
            SELECT guild_id, voice_channel_id, text_channel_id, user_id, granted_at, generation,
                   status, revoke_attempts, next_attempt_at, last_error, revoked_at, allow_bits, deny_bits
            FROM companion_channel_leases
            WHERE guild_id = $1 AND status = 'revoke_pending'
            ORDER BY granted_at ASC
            "#,
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(leases)
    }

    pub async fn list_member_companion_leases(
        &self,
        guild_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Vec<CompanionChannelLease>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("user_id", user_id)?;
        let leases = sqlx::query_as::<_, CompanionChannelLease>(
            r#"
            SELECT guild_id, voice_channel_id, text_channel_id, user_id, granted_at, generation,
                   status, revoke_attempts, next_attempt_at, last_error, revoked_at, allow_bits, deny_bits
            FROM companion_channel_leases
            WHERE guild_id = $1 AND user_id = $2 AND status IN ('active', 'revoke_pending')
            ORDER BY granted_at ASC
            "#,
        )
        .bind(guild_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(leases)
    }

    pub async fn archive_progression_season(
        &self,

        guild_id: &str,

        name: &str,
    ) -> anyhow::Result<ProgressionSeason> {
        let standings_rows: Vec<(String, i32, i64, i64)> = sqlx::query_as(
            r#"

            SELECT user_id, level, xp, coins

            FROM member_progress

            WHERE guild_id = $1

            ORDER BY xp DESC

            LIMIT 100

            "#,
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;

        let standings: Vec<Value> = standings_rows
            .into_iter()
            .map(|(uid, lvl, xp, coins)| {
                serde_json::json!({

                    "user_id": uid,

                    "level": lvl,

                    "xp": xp,

                    "coins": coins

                })
            })
            .collect();

        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM progression_seasons WHERE guild_id = $1")
                .bind(guild_id)
                .fetch_one(&self.pool)
                .await?;

        let season_num = count.0 as i32 + 1;

        let id = Uuid::new_v4();

        let now = Utc::now();

        let rec = sqlx::query_as::<_, ProgressionSeason>(

            r#"

            INSERT INTO progression_seasons (id, guild_id, season_number, name, standings_json, archived_at)

            VALUES ($1, $2, $3, $4, $5, $6)

            RETURNING id, guild_id, season_number, name, standings_json, archived_at

            "#,

        )

        .bind(id)

        .bind(guild_id)

        .bind(season_num)

        .bind(name)

        .bind(serde_json::json!(standings))

        .bind(now)

        .fetch_one(&self.pool)

        .await?;

        Ok(rec)
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
        let id = Uuid::new_v4();

        let now = Utc::now();

        let rec = sqlx::query_as::<_, ScheduledEvent>(

            r#"

            INSERT INTO scheduled_events (id, guild_id, title, description, start_time, end_time, timezone, max_participants, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)

            RETURNING id, guild_id, title, description, start_time, end_time, timezone, max_participants, created_at

            "#,

        )

        .bind(id)

        .bind(guild_id)

        .bind(title)

        .bind(description)

        .bind(start_time)

        .bind(end_time)

        .bind(timezone)

        .bind(max_participants)

        .bind(now)

        .fetch_one(&self.pool)

        .await?;

        Ok(rec)
    }

    pub async fn list_scheduled_events(
        &self,

        guild_id: &str,
    ) -> anyhow::Result<Vec<ScheduledEvent>> {
        let rows = sqlx::query_as::<_, ScheduledEvent>(

            r#"

            SELECT id, guild_id, title, description, start_time, end_time, timezone, max_participants, created_at

            FROM scheduled_events

            WHERE guild_id = $1

            ORDER BY start_time ASC

            "#,

        )

        .bind(guild_id)

        .fetch_all(&self.pool)

        .await?;

        Ok(rows)
    }

    pub async fn get_scheduled_event(
        &self,

        event_id: Uuid,
    ) -> anyhow::Result<Option<ScheduledEvent>> {
        let row = sqlx::query_as::<_, ScheduledEvent>(

            r#"

            SELECT id, guild_id, title, description, start_time, end_time, timezone, max_participants, created_at

            FROM scheduled_events

            WHERE id = $1

            "#,

        )

        .bind(event_id)

        .fetch_optional(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn join_event_roster(
        &self,

        event_id: Uuid,

        user_id: &str,
    ) -> anyhow::Result<EventRoster> {
        let mut tx = self.pool.begin().await?;

        let capacity: Option<i32> = sqlx::query_scalar(
            "SELECT max_participants FROM scheduled_events WHERE id = $1 FOR UPDATE",
        )
        .bind(event_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Scheduled event not found"))?;

        let existing = sqlx::query_as::<_, EventRoster>(
            r#"

            SELECT event_id, user_id, role_type, position, joined_at

            FROM event_rosters

            WHERE event_id = $1 AND user_id = $2

            "#,
        )
        .bind(event_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(r) = existing {
            tx.commit().await?;
            return Ok(r);
        }

        let main_count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM event_rosters WHERE event_id = $1 AND role_type = 'main'",
        )
        .bind(event_id)
        .fetch_one(&mut *tx)
        .await?;

        let role_type = match capacity {
            Some(limit) if limit > 0 => {
                if (main_count.0 as i32) < limit {
                    "main"
                } else {
                    "reserve"
                }
            }
            Some(_) => "reserve",
            None => "main",
        };

        let next_position: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(position)::bigint, 0) + 1 FROM event_rosters WHERE event_id = $1",
        )
        .bind(event_id)
        .fetch_one(&mut *tx)
        .await?;

        let position = i32::try_from(next_position)?;

        let now = Utc::now();

        let rec = sqlx::query_as::<_, EventRoster>(
            r#"

            INSERT INTO event_rosters (event_id, user_id, role_type, position, joined_at)

            VALUES ($1, $2, $3, $4, $5)

            RETURNING event_id, user_id, role_type, position, joined_at

            "#,
        )
        .bind(event_id)
        .bind(user_id)
        .bind(role_type)
        .bind(position)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(rec)
    }

    pub async fn leave_event_roster(
        &self,

        event_id: Uuid,

        user_id: &str,
    ) -> anyhow::Result<Option<String>> {
        let mut tx = self.pool.begin().await?;

        let _capacity: Option<i32> = sqlx::query_scalar(
            "SELECT max_participants FROM scheduled_events WHERE id = $1 FOR UPDATE",
        )
        .bind(event_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Scheduled event not found"))?;

        let existing = sqlx::query_as::<_, EventRoster>(
            r#"

            SELECT event_id, user_id, role_type, position, joined_at

            FROM event_rosters

            WHERE event_id = $1 AND user_id = $2

            "#,
        )
        .bind(event_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;

        let existing = match existing {
            Some(r) => r,

            None => return Ok(None),
        };

        sqlx::query("DELETE FROM event_rosters WHERE event_id = $1 AND user_id = $2")
            .bind(event_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;

        let mut promoted = None;

        if existing.role_type == "main" {
            let next_reserve = sqlx::query_as::<_, EventRoster>(
                r#"

                SELECT event_id, user_id, role_type, position, joined_at

                FROM event_rosters

                WHERE event_id = $1 AND role_type = 'reserve'

                ORDER BY position ASC, joined_at ASC, user_id ASC

                LIMIT 1
                FOR UPDATE

                "#,
            )
            .bind(event_id)
            .fetch_optional(&mut *tx)
            .await?;

            if let Some(reserve) = next_reserve {
                let updated = sqlx::query(
                    "UPDATE event_rosters SET role_type = 'main' WHERE event_id = $1 AND user_id = $2 AND role_type = 'reserve'",
                )
                .bind(event_id)
                .bind(&reserve.user_id)
                .execute(&mut *tx)
                .await?;

                if updated.rows_affected() > 0 {
                    promoted = Some(reserve.user_id);
                }
            }
        }

        tx.commit().await?;

        Ok(promoted)
    }

    pub async fn get_event_rosters(&self, event_id: Uuid) -> anyhow::Result<Vec<EventRoster>> {
        let rows = sqlx::query_as::<_, EventRoster>(
            r#"
            SELECT event_id, user_id, role_type, position, joined_at
            FROM event_rosters
            WHERE event_id = $1
            ORDER BY position ASC, joined_at ASC, user_id ASC
            "#,
        )
        .bind(event_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn create_tournament(
        &self,

        guild_id: &str,

        name: &str,

        teams: &[String],
    ) -> anyhow::Result<Tournament> {
        let id = Uuid::new_v4();

        let mut matches = Vec::new();

        for chunk in teams.chunks(2) {
            let team_a = chunk.first().cloned().unwrap_or_default();

            let team_b = chunk.get(1).cloned().unwrap_or_default();

            matches.push(serde_json::json!({

                "round": 1,

                "team_a": team_a,

                "team_b": team_b,

                "winner": null

            }));
        }

        let bracket = serde_json::json!({

            "teams": teams,

            "matches": matches,

        });

        let now = Utc::now();

        let rec = sqlx::query_as::<_, Tournament>(
            r#"

            INSERT INTO tournaments (id, guild_id, name, bracket_json, status, created_at)

            VALUES ($1, $2, $3, $4, 'in_progress', $5)

            RETURNING id, guild_id, name, bracket_json, status, created_at

            "#,
        )
        .bind(id)
        .bind(guild_id)
        .bind(name)
        .bind(bracket)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;

        Ok(rec)
    }

    pub async fn advance_tournament_match(
        &self,
        guild_id: &str,
        tournament_id: Uuid,
        match_idx: usize,
        winner: &str,
    ) -> anyhow::Result<Tournament> {
        let mut tx = self.pool.begin().await?;
        let tourney = sqlx::query_as::<_, Tournament>(
            r#"
            SELECT id, guild_id, name, bracket_json, status, created_at
            FROM tournaments
            WHERE id = $1 AND guild_id = $2
            FOR UPDATE
            "#,
        )
        .bind(tournament_id)
        .bind(guild_id)
        .fetch_optional(&mut *tx)
        .await?;

        let mut tourney = tourney.ok_or_else(|| anyhow::anyhow!("tournament not found"))?;

        let matches = tourney
            .bracket_json
            .get_mut("matches")
            .and_then(|m| m.as_array_mut())
            .ok_or_else(|| anyhow::anyhow!("malformed bracket matches"))?;

        let match_obj = matches
            .get_mut(match_idx)
            .ok_or_else(|| anyhow::anyhow!("match index out of bounds"))?;

        match_obj["winner"] = serde_json::json!(winner);

        sqlx::query("UPDATE tournaments SET bracket_json = $2 WHERE id = $1")
            .bind(tournament_id)
            .bind(&tourney.bracket_json)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        Ok(tourney)
    }

    pub async fn record_invite(
        &self,

        guild_id: &str,

        code: &str,

        inviter_id: &str,

        campaign_tag: Option<&str>,
    ) -> anyhow::Result<InviteTracking> {
        let now = Utc::now();

        let rec = sqlx::query_as::<_, InviteTracking>(

            r#"

            INSERT INTO invites_tracking (guild_id, code, inviter_id, uses, campaign_tag, created_at)

            VALUES ($1, $2, $3, 1, $4, $5)

            ON CONFLICT (guild_id, code) DO UPDATE

            SET uses = invites_tracking.uses + 1

            RETURNING guild_id, code, inviter_id, uses, campaign_tag, created_at

            "#,

        )

        .bind(guild_id)

        .bind(code)

        .bind(inviter_id)

        .bind(campaign_tag)

        .bind(now)

        .fetch_one(&self.pool)

        .await?;

        Ok(rec)
    }

    pub async fn list_invites(&self, guild_id: &str) -> anyhow::Result<Vec<InviteTracking>> {
        let rows = sqlx::query_as::<_, InviteTracking>(
            r#"

            SELECT guild_id, code, inviter_id, uses, campaign_tag, created_at

            FROM invites_tracking

            WHERE guild_id = $1

            "#,
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn register_dynamic_voice_room(
        &self,

        channel_id: &str,

        guild_id: &str,

        owner_id: &str,
    ) -> anyhow::Result<DynamicVoiceRoom> {
        let now = Utc::now();

        let rec = sqlx::query_as::<_, DynamicVoiceRoom>(

            r#"

            INSERT INTO dynamic_voice_rooms (channel_id, guild_id, owner_id, empty_since, created_at)

            VALUES ($1, $2, $3, NULL, $4)

            RETURNING channel_id, guild_id, owner_id, empty_since, created_at

            "#,

        )

        .bind(channel_id)

        .bind(guild_id)

        .bind(owner_id)

        .bind(now)

        .fetch_one(&self.pool)

        .await?;

        Ok(rec)
    }

    pub async fn update_dynamic_voice_empty_state(
        &self,
        guild_id: &str,
        channel_id: &str,
        is_empty: bool,
    ) -> anyhow::Result<bool> {
        let empty_since = if is_empty { Some(Utc::now()) } else { None };
        let res = sqlx::query("UPDATE dynamic_voice_rooms SET empty_since = $2 WHERE channel_id = $1 AND guild_id = $3")
            .bind(channel_id)
            .bind(empty_since)
            .bind(guild_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn get_stale_dynamic_voice_rooms(
        &self,

        threshold_seconds: i64,
    ) -> anyhow::Result<Vec<DynamicVoiceRoom>> {
        let cutoff = Utc::now() - Duration::seconds(threshold_seconds);

        let rows = sqlx::query_as::<_, DynamicVoiceRoom>(
            r#"

            SELECT channel_id, guild_id, owner_id, empty_since, created_at

            FROM dynamic_voice_rooms

            WHERE empty_since IS NOT NULL AND empty_since <= $1

            "#,
        )
        .bind(cutoff)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn remove_dynamic_voice_room(&self, channel_id: &str) -> anyhow::Result<bool> {
        let res = sqlx::query("DELETE FROM dynamic_voice_rooms WHERE channel_id = $1")
            .bind(channel_id)
            .execute(&self.pool)
            .await?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn cleanup_stale_dynamic_voice_rooms(
        &self,
        guild_id: &str,
        threshold_seconds: i64,
    ) -> anyhow::Result<Vec<String>> {
        let cutoff = Utc::now() - Duration::seconds(threshold_seconds);
        let rows = sqlx::query_scalar::<_, String>(
            r#"
            DELETE FROM dynamic_voice_rooms
            WHERE guild_id = $1
              AND empty_since IS NOT NULL
              AND empty_since <= $2
            RETURNING channel_id
            "#,
        )
        .bind(guild_id)
        .bind(cutoff)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn get_dynamic_voice_room(
        &self,
        channel_id: &str,
    ) -> anyhow::Result<Option<DynamicVoiceRoom>> {
        let rec = sqlx::query_as::<_, DynamicVoiceRoom>(
            "SELECT channel_id, guild_id, owner_id, empty_since, created_at FROM dynamic_voice_rooms WHERE channel_id = $1",
        )
        .bind(channel_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(rec)
    }

    pub async fn set_dynamic_voice_empty_since(
        &self,
        channel_id: &str,
        empty_since: Option<DateTime<Utc>>,
    ) -> anyhow::Result<bool> {
        let res =
            sqlx::query("UPDATE dynamic_voice_rooms SET empty_since = $1 WHERE channel_id = $2")
                .bind(empty_since)
                .bind(channel_id)
                .execute(&self.pool)
                .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn create_feed_subscription(
        &self,

        guild_id: &str,

        channel_id: &str,

        feed_type: &str,

        url_or_target: &str,
    ) -> anyhow::Result<FeedSubscription> {
        let id = Uuid::new_v4();

        let now = Utc::now();

        let rec = sqlx::query_as::<_, FeedSubscription>(

            r#"

            INSERT INTO feed_subscriptions (id, guild_id, channel_id, feed_type, url_or_target, watermark, last_polled_at, created_at)

            VALUES ($1, $2, $3, $4, $5, NULL, NULL, $6)

            RETURNING id, guild_id, channel_id, feed_type, url_or_target, watermark, last_polled_at, created_at

            "#,

        )

        .bind(id)

        .bind(guild_id)

        .bind(channel_id)

        .bind(feed_type)

        .bind(url_or_target)

        .bind(now)

        .fetch_one(&self.pool)

        .await?;

        Ok(rec)
    }

    pub async fn list_feed_subscriptions(
        &self,

        guild_id: &str,
    ) -> anyhow::Result<Vec<FeedSubscription>> {
        let rows = sqlx::query_as::<_, FeedSubscription>(

            r#"

            SELECT id, guild_id, channel_id, feed_type, url_or_target, watermark, last_polled_at, created_at

            FROM feed_subscriptions

            WHERE guild_id = $1

            "#,

        )

        .bind(guild_id)

        .fetch_all(&self.pool)

        .await?;

        Ok(rows)
    }

    pub async fn update_feed_watermark(
        &self,

        subscription_id: Uuid,

        watermark: &str,
    ) -> anyhow::Result<()> {
        let now = Utc::now();

        sqlx::query(
            "UPDATE feed_subscriptions SET watermark = $2, last_polled_at = $3 WHERE id = $1",
        )
        .bind(subscription_id)
        .bind(watermark)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn create_game_link(
        &self,

        guild_id: &str,

        user_id: &str,

        platform: &str,

        platform_account_id: &str,

        code: Option<&str>,
    ) -> anyhow::Result<GameAccountLink> {
        let id = Uuid::new_v4();

        let verified = code.is_none();

        let now = Utc::now();

        let rec = sqlx::query_as::<_, GameAccountLink>(

            r#"

            INSERT INTO game_account_links (id, guild_id, user_id, platform, platform_account_id, verification_code, verified, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)

            ON CONFLICT (guild_id, user_id, platform) DO UPDATE

            SET platform_account_id = $5, verification_code = $6, verified = $7, created_at = $8

            RETURNING id, guild_id, user_id, platform, platform_account_id, verification_code, verified, created_at

            "#,

        )

        .bind(id)

        .bind(guild_id)

        .bind(user_id)

        .bind(platform)

        .bind(platform_account_id)

        .bind(code)

        .bind(verified)

        .bind(now)

        .fetch_one(&self.pool)

        .await?;

        Ok(rec)
    }

    pub async fn verify_game_link(
        &self,

        guild_id: &str,

        user_id: &str,

        platform: &str,

        code: &str,
    ) -> anyhow::Result<bool> {
        let res = sqlx::query(

            r#"

            UPDATE game_account_links

            SET verified = true, verification_code = NULL

            WHERE guild_id = $1 AND user_id = $2 AND platform = $3 AND verification_code = $4 AND verified = false AND created_at > NOW() - INTERVAL '15 minutes'

            "#,

        )

        .bind(guild_id)

        .bind(user_id)

        .bind(platform)

        .bind(code)

        .execute(&self.pool)

        .await?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn get_game_links(
        &self,

        guild_id: &str,

        user_id: &str,
    ) -> anyhow::Result<Vec<GameAccountLink>> {
        let rows = sqlx::query_as::<_, GameAccountLink>(

            r#"

            SELECT id, guild_id, user_id, platform, platform_account_id, verification_code, verified, created_at

            FROM game_account_links

            WHERE guild_id = $1 AND user_id = $2

            "#,

        )

        .bind(guild_id)

        .bind(user_id)

        .fetch_all(&self.pool)

        .await?;

        Ok(rows)
    }

    pub async fn get_or_create_counting_game(
        &self,

        guild_id: &str,

        channel_id: &str,
    ) -> anyhow::Result<CountingGame> {
        sqlx::query(

            r#"

            INSERT INTO counting_games (guild_id, channel_id, current_count, last_user_id, highest_count)

            VALUES ($1, $2, 0, NULL, 0)

            ON CONFLICT (guild_id, channel_id) DO NOTHING

            "#,

        )

        .bind(guild_id)

        .bind(channel_id)

        .execute(&self.pool)

        .await?;

        let rec = sqlx::query_as::<_, CountingGame>(
            r#"

            SELECT guild_id, channel_id, current_count, last_user_id, highest_count

            FROM counting_games

            WHERE guild_id = $1 AND channel_id = $2

            "#,
        )
        .bind(guild_id)
        .bind(channel_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(rec)
    }

    pub async fn submit_count(
        &self,

        guild_id: &str,

        channel_id: &str,

        user_id: &str,

        count: i64,
    ) -> anyhow::Result<(bool, CountingGame)> {
        let mut tx = self.pool.begin().await?;

        let _ = sqlx::query(

            r#"

            INSERT INTO counting_games (guild_id, channel_id, current_count, last_user_id, highest_count)

            VALUES ($1, $2, 0, NULL, 0)

            ON CONFLICT (guild_id, channel_id) DO NOTHING

            "#,

        )

        .bind(guild_id)

        .bind(channel_id)

        .execute(&mut *tx)

        .await?;

        let mut game = sqlx::query_as::<_, CountingGame>(
            r#"

            SELECT guild_id, channel_id, current_count, last_user_id, highest_count

            FROM counting_games

            WHERE guild_id = $1 AND channel_id = $2

            FOR UPDATE

            "#,
        )
        .bind(guild_id)
        .bind(channel_id)
        .fetch_one(&mut *tx)
        .await?;

        let valid =
            count == game.current_count + 1 && game.last_user_id.as_deref() != Some(user_id);

        if valid {
            game.current_count = count;

            game.last_user_id = Some(user_id.to_owned());

            if game.current_count > game.highest_count {
                game.highest_count = game.current_count;
            }
        } else {
            game.current_count = 0;

            game.last_user_id = None;
        }

        sqlx::query(
            r#"

            UPDATE counting_games

            SET current_count = $3, last_user_id = $4, highest_count = $5

            WHERE guild_id = $1 AND channel_id = $2

            "#,
        )
        .bind(guild_id)
        .bind(channel_id)
        .bind(game.current_count)
        .bind(&game.last_user_id)
        .bind(game.highest_count)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok((valid, game))
    }

    pub async fn list_user_cards(
        &self,

        guild_id: &str,

        user_id: &str,
    ) -> anyhow::Result<Vec<CardCollection>> {
        let rows = sqlx::query_as::<_, CardCollection>(
            r#"

            SELECT id, guild_id, user_id, card_id, card_name, rarity, acquired_at

            FROM card_collections

            WHERE guild_id = $1 AND user_id = $2

            "#,
        )
        .bind(guild_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn grant_user_card(
        &self,

        guild_id: &str,

        user_id: &str,

        card_id: &str,

        card_name: &str,

        rarity: &str,
    ) -> anyhow::Result<CardCollection> {
        let id = Uuid::new_v4();

        let now = Utc::now();

        let rec = sqlx::query_as::<_, CardCollection>(

            r#"

            INSERT INTO card_collections (id, guild_id, user_id, card_id, card_name, rarity, acquired_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7)

            RETURNING id, guild_id, user_id, card_id, card_name, rarity, acquired_at

            "#,

        )

        .bind(id)

        .bind(guild_id)

        .bind(user_id)

        .bind(card_id)

        .bind(card_name)

        .bind(rarity)

        .bind(now)

        .fetch_one(&self.pool)

        .await?;

        Ok(rec)
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

        let mut tx = self.pool.begin().await?;

        let c1 = sqlx::query_as::<_, CardCollection>("SELECT id, guild_id, user_id, card_id, card_name, rarity, acquired_at FROM card_collections WHERE id = $1")

            .bind(initiator_card_id)

            .fetch_optional(&mut *tx)

            .await?;

        let c1 = c1.ok_or_else(|| anyhow::anyhow!("initiator card not found"))?;

        if c1.guild_id != guild_id || c1.user_id != initiator_id {
            return Err(anyhow::anyhow!("initiator does not own card"));
        }

        let c2 = sqlx::query_as::<_, CardCollection>("SELECT id, guild_id, user_id, card_id, card_name, rarity, acquired_at FROM card_collections WHERE id = $1")

            .bind(target_card_id)

            .fetch_optional(&mut *tx)

            .await?;

        let c2 = c2.ok_or_else(|| anyhow::anyhow!("target card not found"))?;

        if c2.guild_id != guild_id || c2.user_id != target_id {
            return Err(anyhow::anyhow!("target does not own card"));
        }

        let id = Uuid::new_v4();

        let now = Utc::now();

        let rec = sqlx::query_as::<_, CardTrade>(

            r#"

            INSERT INTO card_trades (id, guild_id, initiator_user_id, initiator_card_id, target_user_id, target_card_id, status, created_at, updated_at)

            VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7, $7)

            RETURNING id, guild_id, initiator_user_id, initiator_card_id, target_user_id, target_card_id, status, created_at, updated_at

            "#,

        )

        .bind(id)

        .bind(guild_id)

        .bind(initiator_id)

        .bind(initiator_card_id)

        .bind(target_id)

        .bind(target_card_id)

        .bind(now)

        .fetch_one(&mut *tx)

        .await?;

        tx.commit().await?;

        Ok(rec)
    }

    pub async fn accept_card_trade(
        &self,

        guild_id: &str,

        trade_id: Uuid,

        target_id: &str,
    ) -> anyhow::Result<bool> {
        let mut tx = self.pool.begin().await?;

        let trade = sqlx::query_as::<_, CardTrade>(

            r#"

            SELECT id, guild_id, initiator_user_id, initiator_card_id, target_user_id, target_card_id, status, created_at, updated_at

            FROM card_trades

            WHERE id = $1

            FOR UPDATE

            "#,

        )

        .bind(trade_id)

        .fetch_optional(&mut *tx)

        .await?;

        let trade = trade.ok_or_else(|| anyhow::anyhow!("trade not found"))?;

        if trade.guild_id != guild_id || trade.target_user_id != target_id {
            return Err(anyhow::anyhow!("unauthorized to accept trade"));
        }

        if trade.status != "pending" {
            return Err(anyhow::anyhow!("trade is not pending"));
        }

        anyhow::ensure!(
            trade.initiator_card_id != trade.target_card_id,
            "cannot trade card with itself"
        );

        let (first_id, second_id) = if trade.initiator_card_id < trade.target_card_id {
            (trade.initiator_card_id, trade.target_card_id)
        } else {
            (trade.target_card_id, trade.initiator_card_id)
        };

        const LOCK_CARD: &str = "SELECT id, guild_id, user_id, card_id, card_name, rarity, acquired_at FROM card_collections WHERE id = $1 FOR UPDATE";

        let first = sqlx::query_as::<_, CardCollection>(LOCK_CARD)
            .bind(first_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| anyhow::anyhow!("card not found"))?;

        let second = sqlx::query_as::<_, CardCollection>(LOCK_CARD)
            .bind(second_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| anyhow::anyhow!("card not found"))?;

        let (c1, c2) = if trade.initiator_card_id == first_id {
            (first, second)
        } else {
            (second, first)
        };

        if c1.guild_id != guild_id || c2.guild_id != guild_id {
            return Err(anyhow::anyhow!("card belongs to different guild"));
        }

        if c1.user_id != trade.initiator_user_id || c2.user_id != trade.target_user_id {
            return Err(anyhow::anyhow!("ownership changed before trade completion"));
        }

        sqlx::query("UPDATE card_collections SET user_id = $2 WHERE id = $1")
            .bind(trade.initiator_card_id)
            .bind(&trade.target_user_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query("UPDATE card_collections SET user_id = $2 WHERE id = $1")
            .bind(trade.target_card_id)
            .bind(&trade.initiator_user_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query("UPDATE card_trades SET status = 'accepted', updated_at = $2 WHERE id = $1")
            .bind(trade_id)
            .bind(Utc::now())
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        Ok(true)
    }

    pub async fn create_tupper_profile(
        &self,

        guild_id: &str,

        user_id: &str,

        name: &str,

        avatar_url: Option<&str>,

        prefix_or_brackets: &str,
    ) -> Result<TupperProfile, TupperError> {
        let id = Uuid::new_v4();

        let now = Utc::now();

        let rec = sqlx::query_as::<_, TupperProfile>(

            r#"

            INSERT INTO tupper_profiles (id, guild_id, user_id, name, avatar_url, prefix_or_brackets, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7)

            RETURNING id, guild_id, user_id, name, avatar_url, prefix_or_brackets, created_at

            "#,

        )

        .bind(id)

        .bind(guild_id)

        .bind(user_id)

        .bind(name)

        .bind(avatar_url)

        .bind(prefix_or_brackets)

        .bind(now)

        .fetch_one(&self.pool)

        .await
        .map_err(|err| match &err {
            sqlx::Error::Database(dberr) => {
                let is_unique = dberr.is_unique_violation()
                    || dberr.code().as_deref() == Some("23505");
                let matches_constraint = dberr
                    .constraint()
                    .is_none_or(|c| c == "uq_tupper_user_prefix");
                if is_unique && matches_constraint {
                    TupperError::PrefixConflict
                } else {
                    TupperError::Database(err)
                }
            }
            _ => TupperError::Database(err),
        })?;

        Ok(rec)
    }

    pub async fn list_tupper_profiles(
        &self,

        guild_id: &str,

        user_id: &str,
    ) -> anyhow::Result<Vec<TupperProfile>> {
        let rows = sqlx::query_as::<_, TupperProfile>(
            r#"

            SELECT id, guild_id, user_id, name, avatar_url, prefix_or_brackets, created_at

            FROM tupper_profiles

            WHERE guild_id = $1 AND user_id = $2

            "#,
        )
        .bind(guild_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn create_faq_entry(
        &self,

        guild_id: &str,

        question: &str,

        answer: &str,

        aliases: &[String],

        category: &str,
    ) -> anyhow::Result<FaqEntry> {
        let id = Uuid::new_v4();

        let now = Utc::now();

        let aliases_json = serde_json::json!(aliases);

        let rec = sqlx::query_as::<_, FaqEntry>(

            r#"

            INSERT INTO faq_entries (id, guild_id, question, answer, aliases_json, category, created_at, updated_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7, $7)

            RETURNING id, guild_id, question, answer, aliases_json, category, created_at, updated_at

            "#,

        )

        .bind(id)

        .bind(guild_id)

        .bind(question)

        .bind(answer)

        .bind(aliases_json)

        .bind(category)

        .bind(now)

        .fetch_one(&self.pool)

        .await?;

        Ok(rec)
    }

    pub async fn list_faq_entries(
        &self,
        guild_id: &str,
        query: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> anyhow::Result<Vec<FaqEntry>> {
        let limit = limit.clamp(1, 50) as i64;
        let offset = offset as i64;

        let rows = if let Some(q) = query {
            let escaped = q
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let pattern = format!("%{escaped}%");

            sqlx::query_as::<_, FaqEntry>(
                r#"
                SELECT id, guild_id, question, answer, aliases_json, category, created_at, updated_at
                FROM faq_entries
                WHERE guild_id = $1
                  AND (question ILIKE $2 ESCAPE '\' OR answer ILIKE $2 ESCAPE '\' OR category ILIKE $2 ESCAPE '\')
                ORDER BY created_at DESC, id DESC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(guild_id)
            .bind(pattern)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, FaqEntry>(
                r#"
                SELECT id, guild_id, question, answer, aliases_json, category, created_at, updated_at
                FROM faq_entries
                WHERE guild_id = $1
                ORDER BY created_at DESC, id DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(guild_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows)
    }

    pub async fn delete_faq_entry(&self, guild_id: &str, faq_id: Uuid) -> anyhow::Result<bool> {
        let res = sqlx::query("DELETE FROM faq_entries WHERE id = $1 AND guild_id = $2")
            .bind(faq_id)
            .bind(guild_id)
            .execute(&self.pool)
            .await?;

        Ok(res.rows_affected() > 0)
    }

    // ==========================================

    // Phase 4: Integrations & Extensibility (#92-#107)

    // ==========================================

    pub async fn upsert_integration_connection(
        &self,

        conn: IntegrationConnection,
    ) -> anyhow::Result<IntegrationConnection> {
        let row = sqlx::query_as::<_, IntegrationConnection>(

            r#"

            INSERT INTO integration_connections (id, guild_id, provider, provider_account_id, access_token, refresh_token, token_expires_at, scopes, metadata, enabled, created_at, updated_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)

            ON CONFLICT (id) DO UPDATE SET

                provider_account_id = EXCLUDED.provider_account_id,

                access_token = EXCLUDED.access_token,

                refresh_token = EXCLUDED.refresh_token,

                token_expires_at = EXCLUDED.token_expires_at,

                scopes = EXCLUDED.scopes,

                metadata = EXCLUDED.metadata,

                enabled = EXCLUDED.enabled,

                updated_at = EXCLUDED.updated_at

            RETURNING *

            "#

        )

        .bind(conn.id)

        .bind(&conn.guild_id)

        .bind(&conn.provider)

        .bind(&conn.provider_account_id)

        .bind(&conn.access_token)

        .bind(&conn.refresh_token)

        .bind(conn.token_expires_at)

        .bind(&conn.scopes)

        .bind(&conn.metadata)

        .bind(conn.enabled)

        .bind(conn.created_at)

        .bind(conn.updated_at)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn get_integration_connection(
        &self,

        guild_id: &str,

        provider: &str,
    ) -> anyhow::Result<Option<IntegrationConnection>> {
        let row = sqlx::query_as::<_, IntegrationConnection>(
            "SELECT * FROM integration_connections WHERE guild_id = $1 AND provider = $2",
        )
        .bind(guild_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_integration_connections(
        &self,

        guild_id: &str,
    ) -> anyhow::Result<Vec<IntegrationConnection>> {
        let rows = sqlx::query_as::<_, IntegrationConnection>(
            "SELECT * FROM integration_connections WHERE guild_id = $1",
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn delete_integration_connection(
        &self,

        guild_id: &str,

        provider: &str,
    ) -> anyhow::Result<bool> {
        let res = sqlx::query(
            "DELETE FROM integration_connections WHERE guild_id = $1 AND provider = $2",
        )
        .bind(guild_id)
        .bind(provider)
        .execute(&self.pool)
        .await?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn create_webhook_subscription(
        &self,

        sub: IntegrationWebhookSubscription,
    ) -> anyhow::Result<IntegrationWebhookSubscription> {
        let row = sqlx::query_as::<_, IntegrationWebhookSubscription>(

            r#"

            INSERT INTO integration_webhook_subscriptions (id, guild_id, provider, external_id, secret_token, target_channel_id, config, active, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)

            ON CONFLICT (guild_id, provider, external_id) DO UPDATE SET

                secret_token = EXCLUDED.secret_token,

                target_channel_id = EXCLUDED.target_channel_id,

                config = EXCLUDED.config,

                active = EXCLUDED.active

            RETURNING *

            "#

        )

        .bind(sub.id)

        .bind(&sub.guild_id)

        .bind(&sub.provider)

        .bind(&sub.external_id)

        .bind(&sub.secret_token)

        .bind(&sub.target_channel_id)

        .bind(&sub.config)

        .bind(sub.active)

        .bind(sub.created_at)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn list_webhook_subscriptions(
        &self,

        guild_id: &str,

        provider: Option<&str>,
    ) -> anyhow::Result<Vec<IntegrationWebhookSubscription>> {
        let rows = if let Some(p) = provider {
            sqlx::query_as::<_, IntegrationWebhookSubscription>(

                "SELECT * FROM integration_webhook_subscriptions WHERE guild_id = $1 AND provider = $2"

            )

            .bind(guild_id)

            .bind(p)

            .fetch_all(&self.pool)

            .await?
        } else {
            sqlx::query_as::<_, IntegrationWebhookSubscription>(
                "SELECT * FROM integration_webhook_subscriptions WHERE guild_id = $1",
            )
            .bind(guild_id)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows)
    }

    pub async fn delete_webhook_subscription(&self, id: Uuid) -> anyhow::Result<bool> {
        let res = sqlx::query("DELETE FROM integration_webhook_subscriptions WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn check_and_record_event_dedupe(
        &self,

        provider: &str,

        event_id: &str,
    ) -> anyhow::Result<bool> {
        let res = sqlx::query(

            "INSERT INTO integration_event_dedupes (provider, event_id, processed_at) VALUES ($1, $2, NOW()) ON CONFLICT DO NOTHING"

        )

        .bind(provider.to_ascii_lowercase())

        .bind(event_id)

        .execute(&self.pool)

        .await?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn record_delivery_log(
        &self,

        log: IntegrationDeliveryLog,
    ) -> anyhow::Result<IntegrationDeliveryLog> {
        let row = sqlx::query_as::<_, IntegrationDeliveryLog>(

            r#"

            INSERT INTO integration_delivery_logs (id, guild_id, provider, event_type, status, error_message, payload_summary, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)

            RETURNING *

            "#

        )

        .bind(log.id)

        .bind(&log.guild_id)

        .bind(&log.provider)

        .bind(&log.event_type)

        .bind(&log.status)

        .bind(&log.error_message)

        .bind(&log.payload_summary)

        .bind(log.created_at)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn list_delivery_logs(
        &self,

        guild_id: &str,

        provider: Option<&str>,
    ) -> anyhow::Result<Vec<IntegrationDeliveryLog>> {
        let rows = if let Some(p) = provider {
            sqlx::query_as::<_, IntegrationDeliveryLog>(

                "SELECT * FROM integration_delivery_logs WHERE guild_id = $1 AND provider = $2 ORDER BY created_at DESC"

            )

            .bind(guild_id)

            .bind(p)

            .fetch_all(&self.pool)

            .await?
        } else {
            sqlx::query_as::<_, IntegrationDeliveryLog>(

                "SELECT * FROM integration_delivery_logs WHERE guild_id = $1 ORDER BY created_at DESC"

            )

            .bind(guild_id)

            .fetch_all(&self.pool)

            .await?
        };

        Ok(rows)
    }

    // Agent 04 (#93): Unified Identity

    pub async fn link_account(&self, link: LinkedAccount) -> anyhow::Result<LinkedAccount> {
        let row = sqlx::query_as::<_, LinkedAccount>(

            r#"

            INSERT INTO linked_accounts (id, guild_id, discord_user_id, provider, provider_subject, attributes, verified_at, synced_role_id)

            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)

            RETURNING *

            "#

        )

        .bind(link.id)

        .bind(&link.guild_id)

        .bind(&link.discord_user_id)

        .bind(&link.provider)

        .bind(&link.provider_subject)

        .bind(&link.attributes)

        .bind(link.verified_at)

        .bind(&link.synced_role_id)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn get_linked_account(
        &self,

        guild_id: &str,

        user_id: &str,

        provider: &str,
    ) -> anyhow::Result<Option<LinkedAccount>> {
        let row = sqlx::query_as::<_, LinkedAccount>(

            "SELECT * FROM linked_accounts WHERE guild_id = $1 AND discord_user_id = $2 AND provider = $3"

        )

        .bind(guild_id)

        .bind(user_id)

        .bind(provider)

        .fetch_optional(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn list_linked_accounts(
        &self,

        guild_id: &str,

        user_id: &str,
    ) -> anyhow::Result<Vec<LinkedAccount>> {
        let rows = sqlx::query_as::<_, LinkedAccount>(
            "SELECT * FROM linked_accounts WHERE guild_id = $1 AND discord_user_id = $2",
        )
        .bind(guild_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn unlink_account(
        &self,

        guild_id: &str,

        user_id: &str,

        provider: &str,
    ) -> anyhow::Result<bool> {
        let res = sqlx::query(

            "DELETE FROM linked_accounts WHERE guild_id = $1 AND discord_user_id = $2 AND provider = $3"

        )

        .bind(guild_id)

        .bind(user_id)

        .bind(provider)

        .execute(&self.pool)

        .await?;

        Ok(res.rows_affected() > 0)
    }

    // Agent 05 (#94) & Agent 15 (#104): Steam & Gaming Hub LFG

    pub async fn create_steam_lfg_post(&self, post: SteamLfgPost) -> anyhow::Result<SteamLfgPost> {
        let row = sqlx::query_as::<_, SteamLfgPost>(

            r#"

            INSERT INTO steam_lfg_posts (id, guild_id, author_id, game_id, game_name, description, max_players, current_players, expires_at, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)

            RETURNING *

            "#

        )

        .bind(post.id)

        .bind(&post.guild_id)

        .bind(&post.author_id)

        .bind(&post.game_id)

        .bind(&post.game_name)

        .bind(&post.description)

        .bind(post.max_players)

        .bind(&post.current_players)

        .bind(post.expires_at)

        .bind(post.created_at)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn list_steam_lfg_posts(
        &self,

        guild_id: &str,

        game_id: Option<&str>,
    ) -> anyhow::Result<Vec<SteamLfgPost>> {
        let rows = if let Some(g) = game_id {
            sqlx::query_as::<_, SteamLfgPost>(

                "SELECT * FROM steam_lfg_posts WHERE guild_id = $1 AND game_id = $2 AND expires_at > NOW()"

            )

            .bind(guild_id)

            .bind(g)

            .fetch_all(&self.pool)

            .await?
        } else {
            sqlx::query_as::<_, SteamLfgPost>(
                "SELECT * FROM steam_lfg_posts WHERE guild_id = $1 AND expires_at > NOW()",
            )
            .bind(guild_id)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows)
    }

    pub async fn count_active_steam_lfg_posts(
        &self,
        guild_id: &str,
        author_id: &str,
    ) -> anyhow::Result<usize> {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM steam_lfg_posts WHERE guild_id = $1 AND author_id = $2 AND expires_at > NOW()",
        )
        .bind(guild_id)
        .bind(author_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count.0 as usize)
    }

    pub async fn count_active_steam_lfg_posts_guild(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<usize> {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM steam_lfg_posts WHERE guild_id = $1 AND expires_at > NOW()",
        )
        .bind(guild_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count.0 as usize)
    }

    pub async fn prune_steam_lfg_posts(&self, cutoff: DateTime<Utc>) -> anyhow::Result<u64> {
        let deleted = sqlx::query("DELETE FROM steam_lfg_posts WHERE expires_at <= $1")
            .bind(cutoff)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(deleted)
    }

    pub async fn join_steam_lfg_post(
        &self,

        guild_id: &str,

        id: Uuid,

        user_id: &str,
    ) -> anyhow::Result<SteamLfgPost> {
        let mut tx = self.pool.begin().await?;

        let post = sqlx::query_as::<_, SteamLfgPost>(
            "SELECT * FROM steam_lfg_posts WHERE id = $1 AND guild_id = $2 FOR UPDATE",
        )
        .bind(id)
        .bind(guild_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("LFG listing not found"))?;

        if post.expires_at <= Utc::now() {
            anyhow::bail!("LFG listing has expired");
        }

        let mut players: Vec<String> =
            serde_json::from_value(post.current_players.clone()).unwrap_or_default();

        if !players.contains(&user_id.to_string()) {
            if players.len() as i32 >= post.max_players {
                anyhow::bail!("LFG post is full");
            }

            players.push(user_id.to_string());
        }

        let updated = sqlx::query_as::<_, SteamLfgPost>(
            "UPDATE steam_lfg_posts SET current_players = $1 WHERE id = $2 RETURNING *",
        )
        .bind(serde_json::json!(players))
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(updated)
    }

    // Agent 06 (#95): GitHub DevOps

    pub async fn upsert_github_devops_mapping(
        &self,

        mapping: GitHubDevopsMapping,
    ) -> anyhow::Result<GitHubDevopsMapping> {
        let row = sqlx::query_as::<_, GitHubDevopsMapping>(

            r#"

            INSERT INTO github_devops_mappings (id, guild_id, repo_full_name, thread_channel_id, issue_or_pr_number, discord_thread_id, last_status, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)

            ON CONFLICT (guild_id, repo_full_name, issue_or_pr_number) DO UPDATE SET

                thread_channel_id = EXCLUDED.thread_channel_id,

                discord_thread_id = EXCLUDED.discord_thread_id,

                last_status = EXCLUDED.last_status

            RETURNING *

            "#

        )

        .bind(mapping.id)

        .bind(&mapping.guild_id)

        .bind(&mapping.repo_full_name)

        .bind(&mapping.thread_channel_id)

        .bind(mapping.issue_or_pr_number)

        .bind(&mapping.discord_thread_id)

        .bind(&mapping.last_status)

        .bind(mapping.created_at)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn get_github_devops_mapping(
        &self,

        guild_id: &str,

        repo: &str,

        number: i32,
    ) -> anyhow::Result<Option<GitHubDevopsMapping>> {
        let row = sqlx::query_as::<_, GitHubDevopsMapping>(

            "SELECT * FROM github_devops_mappings WHERE guild_id = $1 AND repo_full_name = $2 AND issue_or_pr_number = $3"

        )

        .bind(guild_id)

        .bind(repo)

        .bind(number)

        .fetch_optional(&self.pool)

        .await?;

        Ok(row)
    }

    // Agent 07 (#96): Twitch Extended

    pub async fn upsert_twitch_subscription(
        &self,

        sub: TwitchStreamSubscription,
    ) -> anyhow::Result<TwitchStreamSubscription> {
        let row = sqlx::query_as::<_, TwitchStreamSubscription>(

            r#"

            INSERT INTO twitch_stream_subscriptions (id, guild_id, streamer_login, streamer_id, discord_channel_id, live_role_id, last_message_id, is_live, last_title, last_game, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)

            ON CONFLICT (guild_id, streamer_login) DO UPDATE SET

                discord_channel_id = EXCLUDED.discord_channel_id,

                live_role_id = EXCLUDED.live_role_id,

                last_message_id = EXCLUDED.last_message_id,

                is_live = EXCLUDED.is_live,

                last_title = EXCLUDED.last_title,

                last_game = EXCLUDED.last_game

            RETURNING *

            "#

        )

        .bind(sub.id)

        .bind(&sub.guild_id)

        .bind(&sub.streamer_login)

        .bind(&sub.streamer_id)

        .bind(&sub.discord_channel_id)

        .bind(&sub.live_role_id)

        .bind(&sub.last_message_id)

        .bind(sub.is_live)

        .bind(&sub.last_title)

        .bind(&sub.last_game)

        .bind(sub.created_at)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn get_twitch_subscription(
        &self,

        guild_id: &str,

        streamer: &str,
    ) -> anyhow::Result<Option<TwitchStreamSubscription>> {
        let row = sqlx::query_as::<_, TwitchStreamSubscription>(
            "SELECT * FROM twitch_stream_subscriptions WHERE guild_id = $1 AND streamer_login = $2",
        )
        .bind(guild_id)
        .bind(streamer)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // Agent 08 (#97): Kick Integration

    pub async fn upsert_kick_subscription(
        &self,

        sub: KickStreamSubscription,
    ) -> anyhow::Result<KickStreamSubscription> {
        let row = sqlx::query_as::<_, KickStreamSubscription>(

            r#"

            INSERT INTO kick_stream_subscriptions (id, guild_id, slug, channel_id, discord_channel_id, live_role_id, last_message_id, is_live, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)

            ON CONFLICT (guild_id, slug) DO UPDATE SET

                discord_channel_id = EXCLUDED.discord_channel_id,

                live_role_id = EXCLUDED.live_role_id,

                last_message_id = EXCLUDED.last_message_id,

                is_live = EXCLUDED.is_live

            RETURNING *

            "#

        )

        .bind(sub.id)

        .bind(&sub.guild_id)

        .bind(&sub.slug)

        .bind(&sub.channel_id)

        .bind(&sub.discord_channel_id)

        .bind(&sub.live_role_id)

        .bind(&sub.last_message_id)

        .bind(sub.is_live)

        .bind(sub.created_at)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn get_kick_subscription(
        &self,

        guild_id: &str,

        slug: &str,
    ) -> anyhow::Result<Option<KickStreamSubscription>> {
        let row = sqlx::query_as::<_, KickStreamSubscription>(
            "SELECT * FROM kick_stream_subscriptions WHERE guild_id = $1 AND slug = $2",
        )
        .bind(guild_id)
        .bind(slug)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // Agent 09 (#98): YouTube Extended

    pub async fn upsert_youtube_subscription(
        &self,

        sub: YouTubeChannelSubscription,
    ) -> anyhow::Result<YouTubeChannelSubscription> {
        let row = sqlx::query_as::<_, YouTubeChannelSubscription>(

            r#"

            INSERT INTO youtube_channel_subscriptions (id, guild_id, channel_id, channel_title, discord_channel_id, last_video_id, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7)

            ON CONFLICT (guild_id, channel_id) DO UPDATE SET

                discord_channel_id = EXCLUDED.discord_channel_id,

                last_video_id = EXCLUDED.last_video_id

            RETURNING *

            "#

        )

        .bind(sub.id)

        .bind(&sub.guild_id)

        .bind(&sub.channel_id)

        .bind(&sub.channel_title)

        .bind(&sub.discord_channel_id)

        .bind(&sub.last_video_id)

        .bind(sub.created_at)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn get_youtube_subscription(
        &self,

        guild_id: &str,

        channel_id: &str,
    ) -> anyhow::Result<Option<YouTubeChannelSubscription>> {
        let row = sqlx::query_as::<_, YouTubeChannelSubscription>(
            "SELECT * FROM youtube_channel_subscriptions WHERE guild_id = $1 AND channel_id = $2",
        )
        .bind(guild_id)
        .bind(channel_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // Agent 10 (#99): Telegram Bridge

    pub async fn create_telegram_bridge(
        &self,

        bridge: TelegramBridge,
    ) -> anyhow::Result<TelegramBridge> {
        let row = sqlx::query_as::<_, TelegramBridge>(

            r#"

            INSERT INTO telegram_bridges (id, guild_id, telegram_chat_id, discord_channel_id, secret_token, paused, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7)

            ON CONFLICT (guild_id, discord_channel_id) DO UPDATE SET

                telegram_chat_id = EXCLUDED.telegram_chat_id,

                secret_token = EXCLUDED.secret_token,

                paused = EXCLUDED.paused

            RETURNING *

            "#

        )

        .bind(bridge.id)

        .bind(&bridge.guild_id)

        .bind(&bridge.telegram_chat_id)

        .bind(&bridge.discord_channel_id)

        .bind(&bridge.secret_token)

        .bind(bridge.paused)

        .bind(bridge.created_at)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn get_telegram_bridge(
        &self,

        guild_id: &str,

        channel_id: &str,
    ) -> anyhow::Result<Option<TelegramBridge>> {
        let row = sqlx::query_as::<_, TelegramBridge>(
            "SELECT * FROM telegram_bridges WHERE guild_id = $1 AND discord_channel_id = $2",
        )
        .bind(guild_id)
        .bind(channel_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_telegram_bridges(
        &self,

        guild_id: &str,
    ) -> anyhow::Result<Vec<TelegramBridge>> {
        let rows = sqlx::query_as::<_, TelegramBridge>(
            "SELECT * FROM telegram_bridges WHERE guild_id = $1",
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn record_telegram_message_mapping(
        &self,

        mapping: TelegramMessageMapping,
    ) -> anyhow::Result<TelegramMessageMapping> {
        let row = sqlx::query_as::<_, TelegramMessageMapping>(

            r#"

            INSERT INTO telegram_message_mappings (id, bridge_id, telegram_message_id, discord_message_id, direction, created_at)

            VALUES ($1, $2, $3, $4, $5, $6)

            RETURNING *

            "#

        )

        .bind(mapping.id)

        .bind(mapping.bridge_id)

        .bind(&mapping.telegram_message_id)

        .bind(&mapping.discord_message_id)

        .bind(&mapping.direction)

        .bind(mapping.created_at)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn get_telegram_message_mapping(
        &self,

        bridge_id: Uuid,

        msg_id: &str,

        direction: &str,
    ) -> anyhow::Result<Option<TelegramMessageMapping>> {
        let row = sqlx::query_as::<_, TelegramMessageMapping>(

            "SELECT * FROM telegram_message_mappings WHERE bridge_id = $1 AND direction = $2 AND (telegram_message_id = $3 OR discord_message_id = $3)"

        )

        .bind(bridge_id)

        .bind(direction)

        .bind(msg_id)

        .fetch_optional(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn delete_telegram_bridge(&self, bridge_id: Uuid) -> anyhow::Result<bool> {
        let res = sqlx::query("DELETE FROM telegram_bridges WHERE id = $1")
            .bind(bridge_id)
            .execute(&self.pool)
            .await?;

        Ok(res.rows_affected() > 0)
    }

    // Agent 11 (#100): Workflow Builder

    pub async fn create_workflow_definition(
        &self,

        wf: WorkflowDefinition,
    ) -> anyhow::Result<WorkflowDefinition> {
        let row = sqlx::query_as::<_, WorkflowDefinition>(

            r#"

            INSERT INTO workflow_definitions (id, guild_id, name, dag_nodes, dag_edges, is_active, version, created_at, updated_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)

            RETURNING *

            "#

        )

        .bind(wf.id)

        .bind(&wf.guild_id)

        .bind(&wf.name)

        .bind(&wf.dag_nodes)

        .bind(&wf.dag_edges)

        .bind(wf.is_active)

        .bind(wf.version)

        .bind(wf.created_at)

        .bind(wf.updated_at)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn get_workflow_definition(
        &self,

        id: Uuid,
    ) -> anyhow::Result<Option<WorkflowDefinition>> {
        let row = sqlx::query_as::<_, WorkflowDefinition>(
            "SELECT * FROM workflow_definitions WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_workflow_definitions(
        &self,

        guild_id: &str,
    ) -> anyhow::Result<Vec<WorkflowDefinition>> {
        let rows = sqlx::query_as::<_, WorkflowDefinition>(
            "SELECT * FROM workflow_definitions WHERE guild_id = $1",
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn record_workflow_run(
        &self,

        run: WorkflowRunExecution,
    ) -> anyhow::Result<WorkflowRunExecution> {
        let row = sqlx::query_as::<_, WorkflowRunExecution>(

            r#"

            INSERT INTO workflow_run_executions (id, workflow_id, guild_id, trigger_event, status, logs, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7)

            RETURNING *

            "#

        )

        .bind(run.id)

        .bind(run.workflow_id)

        .bind(&run.guild_id)

        .bind(&run.trigger_event)

        .bind(&run.status)

        .bind(&run.logs)

        .bind(run.created_at)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    // Agent 12 (#101): GitLab

    pub async fn upsert_gitlab_subscription(
        &self,

        sub: GitLabSubscription,
    ) -> anyhow::Result<GitLabSubscription> {
        let row = sqlx::query_as::<_, GitLabSubscription>(

            r#"

            INSERT INTO gitlab_subscriptions (id, guild_id, host, project_id, project_path, secret_token, discord_channel_id, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)

            ON CONFLICT (guild_id, host, project_id) DO UPDATE SET

                project_path = EXCLUDED.project_path,

                secret_token = EXCLUDED.secret_token,

                discord_channel_id = EXCLUDED.discord_channel_id

            RETURNING *

            "#

        )

        .bind(sub.id)

        .bind(&sub.guild_id)

        .bind(&sub.host)

        .bind(&sub.project_id)

        .bind(&sub.project_path)

        .bind(&sub.secret_token)

        .bind(&sub.discord_channel_id)

        .bind(sub.created_at)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn get_gitlab_subscription(
        &self,

        guild_id: &str,

        host: &str,

        project_id: &str,
    ) -> anyhow::Result<Option<GitLabSubscription>> {
        let row = sqlx::query_as::<_, GitLabSubscription>(

            "SELECT * FROM gitlab_subscriptions WHERE guild_id = $1 AND host = $2 AND project_id = $3"

        )

        .bind(guild_id)

        .bind(host)

        .bind(project_id)

        .fetch_optional(&self.pool)

        .await?;

        Ok(row)
    }

    // Agent 13 (#102): VK Integration

    pub async fn upsert_vk_subscription(
        &self,

        sub: VkSubscription,
    ) -> anyhow::Result<VkSubscription> {
        let row = sqlx::query_as::<_, VkSubscription>(

            r#"

            INSERT INTO vk_subscriptions (id, guild_id, group_id, confirmation_code, secret_key, discord_channel_id, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7)

            ON CONFLICT (guild_id, group_id) DO UPDATE SET

                confirmation_code = EXCLUDED.confirmation_code,

                secret_key = EXCLUDED.secret_key,

                discord_channel_id = EXCLUDED.discord_channel_id

            RETURNING *

            "#

        )

        .bind(sub.id)

        .bind(&sub.guild_id)

        .bind(&sub.group_id)

        .bind(&sub.confirmation_code)

        .bind(&sub.secret_key)

        .bind(&sub.discord_channel_id)

        .bind(sub.created_at)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn get_vk_subscription(
        &self,

        guild_id: &str,

        group_id: &str,
    ) -> anyhow::Result<Option<VkSubscription>> {
        let row = sqlx::query_as::<_, VkSubscription>(
            "SELECT * FROM vk_subscriptions WHERE guild_id = $1 AND group_id = $2",
        )
        .bind(guild_id)
        .bind(group_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // Agent 14 (#103): Reddit

    pub async fn upsert_reddit_subscription(
        &self,

        sub: RedditSubscription,
    ) -> anyhow::Result<RedditSubscription> {
        let row = sqlx::query_as::<_, RedditSubscription>(

            r#"

            INSERT INTO reddit_subscriptions (id, guild_id, subreddit, discord_channel_id, filter_flair, min_score, last_seen_fullname, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)

            ON CONFLICT (guild_id, subreddit) DO UPDATE SET

                discord_channel_id = EXCLUDED.discord_channel_id,

                filter_flair = EXCLUDED.filter_flair,

                min_score = EXCLUDED.min_score,

                last_seen_fullname = EXCLUDED.last_seen_fullname

            RETURNING *

            "#

        )

        .bind(sub.id)

        .bind(&sub.guild_id)

        .bind(&sub.subreddit)

        .bind(&sub.discord_channel_id)

        .bind(&sub.filter_flair)

        .bind(sub.min_score)

        .bind(&sub.last_seen_fullname)

        .bind(sub.created_at)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn get_reddit_subscription(
        &self,

        guild_id: &str,

        subreddit: &str,
    ) -> anyhow::Result<Option<RedditSubscription>> {
        let row = sqlx::query_as::<_, RedditSubscription>(
            "SELECT * FROM reddit_subscriptions WHERE guild_id = $1 AND subreddit = $2",
        )
        .bind(guild_id)
        .bind(subreddit)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // Agent 15 (#104): Gaming Hub

    pub async fn update_gaming_hub_game_status(
        &self,

        status: GamingHubGameStatus,
    ) -> anyhow::Result<GamingHubGameStatus> {
        let row = sqlx::query_as::<_, GamingHubGameStatus>(

            r#"

            INSERT INTO gaming_hub_game_statuses (id, game_id, region, server_name, status, player_count, last_checked_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7)

            RETURNING *

            "#

        )

        .bind(status.id)

        .bind(&status.game_id)

        .bind(&status.region)

        .bind(&status.server_name)

        .bind(&status.status)

        .bind(status.player_count)

        .bind(status.last_checked_at)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn list_gaming_hub_game_statuses(
        &self,

        game_id: Option<&str>,
    ) -> anyhow::Result<Vec<GamingHubGameStatus>> {
        let rows = if let Some(g) = game_id {
            sqlx::query_as::<_, GamingHubGameStatus>(
                "SELECT * FROM gaming_hub_game_statuses WHERE game_id = $1",
            )
            .bind(g)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, GamingHubGameStatus>("SELECT * FROM gaming_hub_game_statuses")
                .fetch_all(&self.pool)
                .await?
        };

        Ok(rows)
    }

    // Agent 16 (#105): Social Publisher

    pub async fn create_social_draft(
        &self,

        draft: SocialPublicationDraft,
    ) -> anyhow::Result<SocialPublicationDraft> {
        let row = sqlx::query_as::<_, SocialPublicationDraft>(

            r#"

            INSERT INTO social_publication_drafts (id, guild_id, author_id, content, media_urls, targets, scheduled_at, approved_by, status, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)

            RETURNING *

            "#

        )

        .bind(draft.id)

        .bind(&draft.guild_id)

        .bind(&draft.author_id)

        .bind(&draft.content)

        .bind(&draft.media_urls)

        .bind(&draft.targets)

        .bind(draft.scheduled_at)

        .bind(&draft.approved_by)

        .bind(&draft.status)

        .bind(draft.created_at)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn list_social_drafts(
        &self,

        guild_id: &str,
    ) -> anyhow::Result<Vec<SocialPublicationDraft>> {
        let rows = sqlx::query_as::<_, SocialPublicationDraft>(
            "SELECT * FROM social_publication_drafts WHERE guild_id = $1",
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn approve_social_draft(
        &self,

        guild_id: &str,

        id: Uuid,

        approver_id: &str,
    ) -> anyhow::Result<Option<SocialPublicationDraft>> {
        let row = sqlx::query_as::<_, SocialPublicationDraft>(

            "UPDATE social_publication_drafts SET approved_by = $1, status = 'approved' WHERE id = $2 AND guild_id = $3 RETURNING *"

        )

        .bind(approver_id)

        .bind(id)

        .bind(guild_id)

        .fetch_optional(&self.pool)

        .await?;

        Ok(row)
    }

    // Agent 17 & 18 (#106): AI + MCP
    // Issue #153: Read existing budget without no-op UPDATE mutation or locking.
    // Fast path: SELECT existing budget without write locks or MVCC dead tuples.
    // Initialization path: INSERT ... ON CONFLICT DO NOTHING for race-free first access.
    pub async fn get_or_create_ai_budget(&self, guild_id: &str) -> anyhow::Result<AiGuildBudget> {
        if let Some(row) = sqlx::query_as::<_, AiGuildBudget>(
            r#"
            SELECT guild_id, monthly_token_limit, used_tokens, monthly_usd_limit, used_usd, is_halted, reset_at
            FROM ai_guild_budgets
            WHERE guild_id = $1
            "#,
        )
        .bind(guild_id)
        .fetch_optional(&self.pool)
        .await?
        {
            return Ok(row);
        }

        sqlx::query(
            r#"
            INSERT INTO ai_guild_budgets (guild_id, monthly_token_limit, used_tokens, monthly_usd_limit, used_usd, is_halted, reset_at)
            VALUES ($1, 1000000, 0, 10.0, 0.0, false, NOW() + INTERVAL '30 days')
            ON CONFLICT (guild_id) DO NOTHING
            "#,
        )
        .bind(guild_id)
        .execute(&self.pool)
        .await?;

        let row = sqlx::query_as::<_, AiGuildBudget>(
            r#"
            SELECT guild_id, monthly_token_limit, used_tokens, monthly_usd_limit, used_usd, is_halted, reset_at
            FROM ai_guild_budgets
            WHERE guild_id = $1
            "#,
        )
        .bind(guild_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn record_ai_usage(
        &self,

        guild_id: &str,

        tokens: i64,

        usd: f64,
    ) -> anyhow::Result<AiGuildBudget> {
        anyhow::ensure!(tokens >= 0, "tokens must be non-negative");

        anyhow::ensure!(usd >= 0.0, "usd must be non-negative");

        let mut tx = self.pool.begin().await?;

        // Initialize budget on conflict do nothing in the same transaction (#125)

        sqlx::query(

            r#"

            INSERT INTO ai_guild_budgets (guild_id, monthly_token_limit, used_tokens, monthly_usd_limit, used_usd, is_halted, reset_at)

            VALUES ($1, 1000000, 0, 10.0, 0.0, false, NOW() + INTERVAL '30 days')

            ON CONFLICT (guild_id) DO NOTHING

            "#

        )

        .bind(guild_id)

        .execute(&mut *tx)

        .await?;

        let mut budget = sqlx::query_as::<_, AiGuildBudget>(
            "SELECT * FROM ai_guild_budgets WHERE guild_id = $1 FOR UPDATE",
        )
        .bind(guild_id)
        .fetch_one(&mut *tx)
        .await?;

        // Periodic reset check (#125)

        if Utc::now() >= budget.reset_at {
            budget.used_tokens = 0;

            budget.used_usd = 0.0;

            budget.is_halted = false;

            budget.reset_at = Utc::now() + chrono::Duration::days(30);
        }

        if budget.is_halted {
            anyhow::bail!("AI budget exceeded and halted for this guild");
        }

        let new_tokens = budget
            .used_tokens
            .checked_add(tokens)
            .ok_or_else(|| anyhow::anyhow!("token overflow"))?;

        let new_usd = budget.used_usd + usd;

        let is_halted =
            new_tokens >= budget.monthly_token_limit || new_usd >= budget.monthly_usd_limit;

        let updated = sqlx::query_as::<_, AiGuildBudget>(

            "UPDATE ai_guild_budgets SET used_tokens = $1, used_usd = $2, is_halted = $3, reset_at = $4 WHERE guild_id = $5 RETURNING *"

        )

        .bind(new_tokens)

        .bind(new_usd)

        .bind(is_halted)

        .bind(budget.reset_at)

        .bind(guild_id)

        .fetch_one(&mut *tx)

        .await?;

        tx.commit().await?;

        Ok(updated)
    }

    pub async fn create_ai_document(
        &self,

        doc: AiKnowledgeDocument,
    ) -> anyhow::Result<AiKnowledgeDocument> {
        let row = sqlx::query_as::<_, AiKnowledgeDocument>(

            r#"

            INSERT INTO ai_knowledge_documents (id, guild_id, channel_acl, title, content, source_type, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7)

            RETURNING *

            "#

        )

        .bind(doc.id)

        .bind(&doc.guild_id)

        .bind(&doc.channel_acl)

        .bind(&doc.title)

        .bind(&doc.content)

        .bind(&doc.source_type)

        .bind(doc.created_at)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn search_ai_documents(
        &self,
        guild_id: &str,
        user_channels: &[String],
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<AiKnowledgeDocument>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let escaped = query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{escaped}%");
        let limit = limit.clamp(1, 50) as i64;

        let rows = sqlx::query_as::<_, AiKnowledgeDocument>(
            r#"
            SELECT id, guild_id, channel_acl, title, content, source_type, created_at
            FROM ai_knowledge_documents
            WHERE guild_id = $1
              AND (title ILIKE $2 ESCAPE '\' OR content ILIKE $2 ESCAPE '\')
              AND CASE
                    WHEN jsonb_typeof(channel_acl) = 'array'
                      THEN jsonb_array_length(channel_acl) = 0
                           OR channel_acl ?| $3::text[]
                    ELSE FALSE
                  END
            ORDER BY created_at DESC, id DESC
            LIMIT $4
            "#,
        )
        .bind(guild_id)
        .bind(pattern)
        .bind(user_channels)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn upsert_mcp_tool_authorization(
        &self,

        auth: McpToolAuthorization,
    ) -> anyhow::Result<McpToolAuthorization> {
        let row = sqlx::query_as::<_, McpToolAuthorization>(

            r#"

            INSERT INTO mcp_tool_authorizations (id, guild_id, tool_name, allowed_roles, requires_approval, enabled)

            VALUES ($1, $2, $3, $4, $5, $6)

            ON CONFLICT (guild_id, tool_name) DO UPDATE SET

                allowed_roles = EXCLUDED.allowed_roles,

                requires_approval = EXCLUDED.requires_approval,

                enabled = EXCLUDED.enabled

            RETURNING *

            "#

        )

        .bind(auth.id)

        .bind(&auth.guild_id)

        .bind(&auth.tool_name)

        .bind(&auth.allowed_roles)

        .bind(auth.requires_approval)

        .bind(auth.enabled)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn get_mcp_tool_authorizations(
        &self,

        guild_id: &str,
    ) -> anyhow::Result<Vec<McpToolAuthorization>> {
        let rows = sqlx::query_as::<_, McpToolAuthorization>(
            "SELECT * FROM mcp_tool_authorizations WHERE guild_id = $1",
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    // Agent 19 (#107): Plugin Marketplace

    pub async fn publish_marketplace_package(
        &self,

        pkg: PluginMarketplacePackage,
    ) -> anyhow::Result<PluginMarketplacePackage> {
        let row = sqlx::query_as::<_, PluginMarketplacePackage>(

            r#"

            INSERT INTO plugin_marketplace_packages (id, plugin_id, name, version, author, sha256_hash, manifest, is_verified, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)

            ON CONFLICT (plugin_id) DO UPDATE SET

                version = EXCLUDED.version,

                sha256_hash = EXCLUDED.sha256_hash,

                manifest = EXCLUDED.manifest,

                is_verified = EXCLUDED.is_verified

            RETURNING *

            "#

        )

        .bind(pkg.id)

        .bind(&pkg.plugin_id)

        .bind(&pkg.name)

        .bind(&pkg.version)

        .bind(&pkg.author)

        .bind(&pkg.sha256_hash)

        .bind(&pkg.manifest)

        .bind(pkg.is_verified)

        .bind(pkg.created_at)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn list_marketplace_packages(&self) -> anyhow::Result<Vec<PluginMarketplacePackage>> {
        let rows = sqlx::query_as::<_, PluginMarketplacePackage>(
            "SELECT * FROM plugin_marketplace_packages",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn install_plugin_package(
        &self,

        install: PluginGuildInstallation,
    ) -> anyhow::Result<PluginGuildInstallation> {
        let row = sqlx::query_as::<_, PluginGuildInstallation>(

            r#"

            INSERT INTO plugin_guild_installations (id, guild_id, plugin_id, version, enabled, config, installed_at)

            VALUES ($1, $2, $3, $4, $5, $6, $7)

            ON CONFLICT (guild_id, plugin_id) DO UPDATE SET

                version = EXCLUDED.version,

                enabled = EXCLUDED.enabled,

                config = EXCLUDED.config

            RETURNING *

            "#

        )

        .bind(install.id)

        .bind(&install.guild_id)

        .bind(&install.plugin_id)

        .bind(&install.version)

        .bind(install.enabled)

        .bind(&install.config)

        .bind(install.installed_at)

        .fetch_one(&self.pool)

        .await?;

        Ok(row)
    }

    pub async fn list_installed_plugins(
        &self,

        guild_id: &str,
    ) -> anyhow::Result<Vec<PluginGuildInstallation>> {
        let rows = sqlx::query_as::<_, PluginGuildInstallation>(
            "SELECT * FROM plugin_guild_installations WHERE guild_id = $1",
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn uninstall_plugin_package(
        &self,

        guild_id: &str,

        plugin_id: &str,
    ) -> anyhow::Result<bool> {
        let res = sqlx::query(
            "DELETE FROM plugin_guild_installations WHERE guild_id = $1 AND plugin_id = $2",
        )
        .bind(guild_id)
        .bind(plugin_id)
        .execute(&self.pool)
        .await?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn list_policy_rules(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<CommandPolicyRule>> {
        validate_snowflake("guild_id", guild_id)?;
        let rows = sqlx::query_as::<_, CommandPolicyRule>(
            r#"
            SELECT id, guild_id, subject_kind, subject_id, resource_kind, module_id,
                   command_name, action_key, effect, priority, enabled, version,
                   created_by, created_at, updated_at
            FROM command_policy_rules
            WHERE guild_id = $1
            ORDER BY priority DESC, created_at ASC
            "#,
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn upsert_policy_rule(
        &self,
        rule: CommandPolicyRule,
    ) -> anyhow::Result<CommandPolicyRule> {
        validate_snowflake("guild_id", &rule.guild_id)?;
        if let Some(ref uid) = rule.subject_id {
            if rule.subject_kind == "user" {
                validate_snowflake("user_id", uid)?;
            } else if rule.subject_kind == "role" {
                validate_snowflake("role_id", uid)?;
            } else if rule.subject_kind == "channel" || rule.subject_kind == "voice_channel" {
                validate_snowflake("channel_id", uid)?;
            }
        }

        let mut tx = self.pool.begin().await?;
        let now = Utc::now();
        let saved = sqlx::query_as::<_, CommandPolicyRule>(
            r#"
            INSERT INTO command_policy_rules (
                id, guild_id, subject_kind, subject_id, resource_kind, module_id,
                command_name, action_key, effect, priority, enabled, version,
                created_by, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            ON CONFLICT (id)
            DO UPDATE SET
                subject_kind = EXCLUDED.subject_kind,
                subject_id = EXCLUDED.subject_id,
                resource_kind = EXCLUDED.resource_kind,
                module_id = EXCLUDED.module_id,
                command_name = EXCLUDED.command_name,
                action_key = EXCLUDED.action_key,
                effect = EXCLUDED.effect,
                priority = EXCLUDED.priority,
                enabled = EXCLUDED.enabled,
                version = command_policy_rules.version + 1,
                updated_at = EXCLUDED.updated_at
            WHERE command_policy_rules.guild_id = EXCLUDED.guild_id
            RETURNING id, guild_id, subject_kind, subject_id, resource_kind, module_id,
                      command_name, action_key, effect, priority, enabled, version,
                      created_by, created_at, updated_at
            "#,
        )
        .bind(rule.id)
        .bind(&rule.guild_id)
        .bind(&rule.subject_kind)
        .bind(&rule.subject_id)
        .bind(&rule.resource_kind)
        .bind(&rule.module_id)
        .bind(&rule.command_name)
        .bind(&rule.action_key)
        .bind(&rule.effect)
        .bind(rule.priority)
        .bind(rule.enabled)
        .bind(rule.version)
        .bind(&rule.created_by)
        .bind(rule.created_at)
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("policy rule ID belongs to another guild"))?;

        sqlx::query(
            r#"
            INSERT INTO command_policy_revisions (guild_id, revision, updated_at)
            VALUES ($1, 1, NOW())
            ON CONFLICT (guild_id)
            DO UPDATE SET revision = command_policy_revisions.revision + 1, updated_at = NOW()
            "#,
        )
        .bind(&rule.guild_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(saved)
    }

    pub async fn delete_policy_rule(&self, guild_id: &str, rule_id: Uuid) -> anyhow::Result<bool> {
        validate_snowflake("guild_id", guild_id)?;
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query("DELETE FROM command_policy_rules WHERE guild_id = $1 AND id = $2")
            .bind(guild_id)
            .bind(rule_id)
            .execute(&mut *tx)
            .await?;

        let deleted = res.rows_affected() > 0;
        if deleted {
            sqlx::query(
                r#"
                INSERT INTO command_policy_revisions (guild_id, revision, updated_at)
                VALUES ($1, 1, NOW())
                ON CONFLICT (guild_id)
                DO UPDATE SET revision = command_policy_revisions.revision + 1, updated_at = NOW()
                "#,
            )
            .bind(guild_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(deleted)
    }

    pub async fn reset_policy_rules(&self, guild_id: &str) -> anyhow::Result<usize> {
        validate_snowflake("guild_id", guild_id)?;
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query("DELETE FROM command_policy_rules WHERE guild_id = $1")
            .bind(guild_id)
            .execute(&mut *tx)
            .await?;

        let count = res.rows_affected() as usize;
        sqlx::query(
            r#"
            INSERT INTO command_policy_revisions (guild_id, revision, updated_at)
            VALUES ($1, 1, NOW())
            ON CONFLICT (guild_id)
            DO UPDATE SET revision = command_policy_revisions.revision + 1, updated_at = NOW()
            "#,
        )
        .bind(guild_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(count)
    }

    pub async fn replace_policy_rules(
        &self,
        guild_id: &str,
        expected_revision: Option<i64>,
        rules: Vec<CommandPolicyRule>,
    ) -> anyhow::Result<Vec<CommandPolicyRule>> {
        validate_snowflake("guild_id", guild_id)?;
        for rule in &rules {
            if rule.guild_id != guild_id {
                anyhow::bail!("rule {} guild_id mismatch", rule.id);
            }
            crate::policy::validate_policy_rule_fields(rule)?;
        }

        let mut tx = self.pool.begin().await?;

        // 1. Ensure revision row exists
        sqlx::query(
            r#"
            INSERT INTO command_policy_revisions (guild_id, revision, updated_at)
            VALUES ($1, 1, NOW())
            ON CONFLICT (guild_id) DO NOTHING
            "#,
        )
        .bind(guild_id)
        .execute(&mut *tx)
        .await?;

        // 2. Lock and fetch current revision
        let current_rev: i64 = sqlx::query_scalar(
            "SELECT revision FROM command_policy_revisions WHERE guild_id = $1 FOR UPDATE",
        )
        .bind(guild_id)
        .fetch_one(&mut *tx)
        .await?;

        // 3. Verify expected revision if supplied
        if let Some(expected) = expected_revision
            && expected != current_rev
        {
            anyhow::bail!(
                "Revision conflict: current revision is {}, expected {}",
                current_rev,
                expected
            );
        }

        // 4. Delete existing rules for this guild
        sqlx::query("DELETE FROM command_policy_rules WHERE guild_id = $1")
            .bind(guild_id)
            .execute(&mut *tx)
            .await?;

        // 5. Insert new rules
        let now = Utc::now();
        let mut inserted = Vec::with_capacity(rules.len());
        for rule in rules {
            let saved = sqlx::query_as::<_, CommandPolicyRule>(
                r#"
                INSERT INTO command_policy_rules (
                    id, guild_id, subject_kind, subject_id, resource_kind, module_id,
                    command_name, action_key, effect, priority, enabled, version,
                    created_by, created_at, updated_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
                RETURNING id, guild_id, subject_kind, subject_id, resource_kind, module_id,
                          command_name, action_key, effect, priority, enabled, version,
                          created_by, created_at, updated_at
                "#,
            )
            .bind(rule.id)
            .bind(&rule.guild_id)
            .bind(&rule.subject_kind)
            .bind(&rule.subject_id)
            .bind(&rule.resource_kind)
            .bind(&rule.module_id)
            .bind(&rule.command_name)
            .bind(&rule.action_key)
            .bind(&rule.effect)
            .bind(rule.priority)
            .bind(rule.enabled)
            .bind(rule.version)
            .bind(&rule.created_by)
            .bind(rule.created_at)
            .bind(now)
            .fetch_one(&mut *tx)
            .await?;

            inserted.push(saved);
        }

        // 6. Bump revision exactly once
        sqlx::query(
            r#"
            UPDATE command_policy_revisions
            SET revision = command_policy_revisions.revision + 1, updated_at = NOW()
            WHERE guild_id = $1
            "#,
        )
        .bind(guild_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(inserted)
    }

    pub async fn get_policy_revision(&self, guild_id: &str) -> anyhow::Result<i64> {
        validate_snowflake("guild_id", guild_id)?;
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT revision FROM command_policy_revisions WHERE guild_id = $1")
                .bind(guild_id)
                .fetch_optional(&self.pool)
                .await?;

        Ok(row.map(|(r,)| r).unwrap_or(1))
    }

    pub async fn bump_policy_revision(&self, guild_id: &str) -> anyhow::Result<i64> {
        validate_snowflake("guild_id", guild_id)?;
        let row: (i64,) = sqlx::query_as(
            r#"
            INSERT INTO command_policy_revisions (guild_id, revision, updated_at)
            VALUES ($1, 2, NOW())
            ON CONFLICT (guild_id)
            DO UPDATE SET revision = command_policy_revisions.revision + 1, updated_at = NOW()
            RETURNING revision
            "#,
        )
        .bind(guild_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.0)
    }

    pub async fn get_duty_settings(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Option<ModeratorDutySettings>> {
        validate_snowflake("guild_id", guild_id)?;
        let row = sqlx::query_as::<_, ModeratorDutySettings>(
            r#"
            SELECT guild_id, staff_role_id, on_call_role_id, max_override_duration_mins, fallback_channel_id, updated_at
            FROM moderator_duty_settings
            WHERE guild_id = $1
            "#,
        )
        .bind(guild_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn upsert_duty_settings(
        &self,
        settings: ModeratorDutySettings,
    ) -> anyhow::Result<ModeratorDutySettings> {
        validate_snowflake("guild_id", &settings.guild_id)?;
        if let Some(ref r_id) = settings.staff_role_id {
            validate_snowflake("staff_role_id", r_id)?;
        }
        if let Some(ref r_id) = settings.on_call_role_id {
            validate_snowflake("on_call_role_id", r_id)?;
        }
        if let Some(ref c_id) = settings.fallback_channel_id {
            validate_snowflake("fallback_channel_id", c_id)?;
        }

        let saved = sqlx::query_as::<_, ModeratorDutySettings>(
            r#"
            INSERT INTO moderator_duty_settings (
                guild_id, staff_role_id, on_call_role_id, max_override_duration_mins, fallback_channel_id, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, NOW())
            ON CONFLICT (guild_id)
            DO UPDATE SET
                staff_role_id = EXCLUDED.staff_role_id,
                on_call_role_id = EXCLUDED.on_call_role_id,
                max_override_duration_mins = EXCLUDED.max_override_duration_mins,
                fallback_channel_id = EXCLUDED.fallback_channel_id,
                updated_at = NOW()
            RETURNING guild_id, staff_role_id, on_call_role_id, max_override_duration_mins, fallback_channel_id, updated_at
            "#,
        )
        .bind(&settings.guild_id)
        .bind(&settings.staff_role_id)
        .bind(&settings.on_call_role_id)
        .bind(settings.max_override_duration_mins)
        .bind(&settings.fallback_channel_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(saved)
    }

    pub async fn get_duty_profile(
        &self,
        guild_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Option<ModeratorDutyProfile>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("user_id", user_id)?;
        let row = sqlx::query_as::<_, ModeratorDutyProfile>(
            r#"
            SELECT guild_id, user_id, timezone, recurring_schedule, manual_override, override_expires_at, version, updated_at
            FROM moderator_duty_profiles
            WHERE guild_id = $1 AND user_id = $2
            "#,
        )
        .bind(guild_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_duty_profiles(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<ModeratorDutyProfile>> {
        validate_snowflake("guild_id", guild_id)?;
        let rows = sqlx::query_as::<_, ModeratorDutyProfile>(
            r#"
            SELECT guild_id, user_id, timezone, recurring_schedule, manual_override, override_expires_at, version, updated_at
            FROM moderator_duty_profiles
            WHERE guild_id = $1
            ORDER BY user_id ASC
            "#,
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn list_guilds_with_duty_profiles(&self) -> anyhow::Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            r#"
            SELECT DISTINCT guild_id
            FROM moderator_duty_profiles
            ORDER BY guild_id ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|(g,)| g).collect())
    }

    pub async fn upsert_duty_profile(
        &self,
        profile: ModeratorDutyProfile,
    ) -> anyhow::Result<ModeratorDutyProfile> {
        validate_snowflake("guild_id", &profile.guild_id)?;
        validate_snowflake("user_id", &profile.user_id)?;

        let saved = sqlx::query_as::<_, ModeratorDutyProfile>(
            r#"
            INSERT INTO moderator_duty_profiles (
                guild_id, user_id, timezone, recurring_schedule, manual_override, override_expires_at, version, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
            ON CONFLICT (guild_id, user_id)
            DO UPDATE SET
                timezone = EXCLUDED.timezone,
                recurring_schedule = EXCLUDED.recurring_schedule,
                manual_override = EXCLUDED.manual_override,
                override_expires_at = EXCLUDED.override_expires_at,
                version = moderator_duty_profiles.version + 1,
                updated_at = NOW()
            RETURNING guild_id, user_id, timezone, recurring_schedule, manual_override, override_expires_at, version, updated_at
            "#,
        )
        .bind(&profile.guild_id)
        .bind(&profile.user_id)
        .bind(&profile.timezone)
        .bind(&profile.recurring_schedule)
        .bind(&profile.manual_override)
        .bind(profile.override_expires_at)
        .bind(profile.version)
        .fetch_one(&self.pool)
        .await?;

        Ok(saved)
    }

    pub async fn delete_duty_profile(&self, guild_id: &str, user_id: &str) -> anyhow::Result<bool> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("user_id", user_id)?;

        let mut tx = self.pool.begin().await?;
        let res =
            sqlx::query("DELETE FROM moderator_duty_profiles WHERE guild_id = $1 AND user_id = $2")
                .bind(guild_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;

        let _ =
            sqlx::query("DELETE FROM moderator_duty_state WHERE guild_id = $1 AND user_id = $2")
                .bind(guild_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;

        let _ = sqlx::query(
            r#"
            UPDATE moderator_duty_role_grants
            SET status = 'revoke_pending', next_attempt_at = NOW(), updated_at = NOW()
            WHERE guild_id = $1 AND user_id = $2 AND status = 'active'
            "#,
        )
        .bind(guild_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn get_duty_state(
        &self,
        guild_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Option<ModeratorDutyState>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("user_id", user_id)?;
        let row = sqlx::query_as::<_, ModeratorDutyState>(
            r#"
            SELECT guild_id, user_id, expected_on_call, state_generation, last_reconciled_at, last_error
            FROM moderator_duty_state
            WHERE guild_id = $1 AND user_id = $2
            "#,
        )
        .bind(guild_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_duty_states(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<ModeratorDutyState>> {
        validate_snowflake("guild_id", guild_id)?;
        let rows = sqlx::query_as::<_, ModeratorDutyState>(
            r#"
            SELECT guild_id, user_id, expected_on_call, state_generation, last_reconciled_at, last_error
            FROM moderator_duty_state
            WHERE guild_id = $1
            ORDER BY user_id ASC
            "#,
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn upsert_duty_state(
        &self,
        state: ModeratorDutyState,
    ) -> anyhow::Result<ModeratorDutyState> {
        validate_snowflake("guild_id", &state.guild_id)?;
        validate_snowflake("user_id", &state.user_id)?;

        let saved = sqlx::query_as::<_, ModeratorDutyState>(
            r#"
            INSERT INTO moderator_duty_state (
                guild_id, user_id, expected_on_call, state_generation, last_reconciled_at, last_error
            )
            VALUES ($1, $2, $3, $4, NOW(), $5)
            ON CONFLICT (guild_id, user_id)
            DO UPDATE SET
                expected_on_call = EXCLUDED.expected_on_call,
                state_generation = moderator_duty_state.state_generation + 1,
                last_reconciled_at = NOW(),
                last_error = EXCLUDED.last_error
            RETURNING guild_id, user_id, expected_on_call, state_generation, last_reconciled_at, last_error
            "#,
        )
        .bind(&state.guild_id)
        .bind(&state.user_id)
        .bind(state.expected_on_call)
        .bind(state.state_generation)
        .bind(&state.last_error)
        .fetch_one(&self.pool)
        .await?;

        Ok(saved)
    }

    pub async fn get_duty_role_grant(
        &self,
        guild_id: &str,
        user_id: &str,
        role_id: &str,
    ) -> anyhow::Result<Option<crate::duty::ModeratorDutyRoleGrant>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("user_id", user_id)?;
        validate_snowflake("role_id", role_id)?;

        let row = sqlx::query_as::<_, crate::duty::ModeratorDutyRoleGrant>(
            r#"
            SELECT guild_id, user_id, role_id, status, attempts, last_error, next_attempt_at, generation, created_at, updated_at
            FROM moderator_duty_role_grants
            WHERE guild_id = $1 AND user_id = $2 AND role_id = $3
            "#,
        )
        .bind(guild_id)
        .bind(user_id)
        .bind(role_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn upsert_duty_role_grant(
        &self,
        grant: crate::duty::ModeratorDutyRoleGrant,
    ) -> anyhow::Result<crate::duty::ModeratorDutyRoleGrant> {
        validate_snowflake("guild_id", &grant.guild_id)?;
        validate_snowflake("user_id", &grant.user_id)?;
        validate_snowflake("role_id", &grant.role_id)?;

        let saved = sqlx::query_as::<_, crate::duty::ModeratorDutyRoleGrant>(
            r#"
            INSERT INTO moderator_duty_role_grants (
                guild_id, user_id, role_id, status, attempts, last_error, next_attempt_at, generation, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW(), NOW())
            ON CONFLICT (guild_id, user_id, role_id)
            DO UPDATE SET
                status = EXCLUDED.status,
                attempts = EXCLUDED.attempts,
                last_error = EXCLUDED.last_error,
                next_attempt_at = EXCLUDED.next_attempt_at,
                generation = moderator_duty_role_grants.generation + 1,
                updated_at = NOW()
            RETURNING guild_id, user_id, role_id, status, attempts, last_error, next_attempt_at, generation, created_at, updated_at
            "#,
        )
        .bind(&grant.guild_id)
        .bind(&grant.user_id)
        .bind(&grant.role_id)
        .bind(&grant.status)
        .bind(grant.attempts)
        .bind(&grant.last_error)
        .bind(grant.next_attempt_at)
        .bind(grant.generation)
        .fetch_one(&self.pool)
        .await?;

        Ok(saved)
    }

    pub async fn record_duty_role_grant(
        &self,
        guild_id: &str,
        user_id: &str,
        role_id: &str,
        status: &str,
    ) -> anyhow::Result<()> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("user_id", user_id)?;
        validate_snowflake("role_id", role_id)?;

        sqlx::query(
            r#"
            INSERT INTO moderator_duty_role_grants (
                guild_id, user_id, role_id, status, attempts, last_error, next_attempt_at, generation, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, 0, NULL, NOW(), 1, NOW(), NOW())
            ON CONFLICT (guild_id, user_id, role_id)
            DO UPDATE SET
                status = EXCLUDED.status,
                attempts = CASE WHEN EXCLUDED.status = 'active' THEN 0 ELSE moderator_duty_role_grants.attempts END,
                last_error = NULL,
                generation = moderator_duty_role_grants.generation + 1,
                updated_at = NOW()
            "#,
        )
        .bind(guild_id)
        .bind(user_id)
        .bind(role_id)
        .bind(status)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn mark_duty_role_grants_for_revocation(
        &self,
        guild_id: &str,
        user_id: Option<&str>,
        role_id: Option<&str>,
    ) -> anyhow::Result<Vec<crate::duty::ModeratorDutyRoleGrant>> {
        validate_snowflake("guild_id", guild_id)?;
        if let Some(uid) = user_id {
            validate_snowflake("user_id", uid)?;
        }
        if let Some(rid) = role_id {
            validate_snowflake("role_id", rid)?;
        }

        let rows = sqlx::query_as::<_, crate::duty::ModeratorDutyRoleGrant>(
            r#"
            UPDATE moderator_duty_role_grants
            SET status = 'revoke_pending',
                next_attempt_at = NOW(),
                generation = generation + 1,
                updated_at = NOW()
            WHERE guild_id = $1
              AND ($2::TEXT IS NULL OR user_id = $2)
              AND ($3::TEXT IS NULL OR role_id = $3)
              AND status = 'active'
            RETURNING guild_id, user_id, role_id, status, attempts, last_error, next_attempt_at, generation, created_at, updated_at
            "#,
        )
        .bind(guild_id)
        .bind(user_id)
        .bind(role_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn list_pending_duty_role_revocations(
        &self,
        guild_id: Option<&str>,
    ) -> anyhow::Result<Vec<crate::duty::ModeratorDutyRoleGrant>> {
        if let Some(gid) = guild_id {
            validate_snowflake("guild_id", gid)?;
        }

        let rows = sqlx::query_as::<_, crate::duty::ModeratorDutyRoleGrant>(
            r#"
            SELECT guild_id, user_id, role_id, status, attempts, last_error, next_attempt_at, generation, created_at, updated_at
            FROM moderator_duty_role_grants
            WHERE status = 'revoke_pending'
              AND next_attempt_at <= NOW()
              AND ($1::TEXT IS NULL OR guild_id = $1)
            ORDER BY next_attempt_at ASC
            "#,
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn get_recruitment_track(
        &self,
        guild_id: &str,
        track_id: Uuid,
    ) -> anyhow::Result<Option<RecruitmentTrack>> {
        validate_snowflake("guild_id", guild_id)?;
        let row = sqlx::query_as::<_, RecruitmentTrack>(
            r#"
            SELECT id, guild_id, slug, name, enabled, target_role_id, config, version, created_at, updated_at
            FROM recruitment_tracks
            WHERE id = $1 AND guild_id = $2
            "#,
        )
        .bind(track_id)
        .bind(guild_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_recruitment_track_by_slug(
        &self,
        guild_id: &str,
        slug: &str,
    ) -> anyhow::Result<Option<RecruitmentTrack>> {
        validate_snowflake("guild_id", guild_id)?;
        let row = sqlx::query_as::<_, RecruitmentTrack>(
            r#"
            SELECT id, guild_id, slug, name, enabled, target_role_id, config, version, created_at, updated_at
            FROM recruitment_tracks
            WHERE guild_id = $1 AND slug = $2
            "#,
        )
        .bind(guild_id)
        .bind(slug)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_recruitment_tracks(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<RecruitmentTrack>> {
        validate_snowflake("guild_id", guild_id)?;
        let rows = sqlx::query_as::<_, RecruitmentTrack>(
            r#"
            SELECT id, guild_id, slug, name, enabled, target_role_id, config, version, created_at, updated_at
            FROM recruitment_tracks
            WHERE guild_id = $1
            ORDER BY created_at ASC
            "#,
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn upsert_recruitment_track(
        &self,
        track: RecruitmentTrack,
    ) -> anyhow::Result<RecruitmentTrack> {
        validate_snowflake("guild_id", &track.guild_id)?;
        crate::recruitment::validate_track_fields(
            &track.guild_id,
            &track.slug,
            &track.name,
            track.target_role_id.as_deref(),
            &track.config,
        )?;

        let conflicting = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*) FROM recruitment_tracks
            WHERE guild_id = $1 AND slug = $2 AND id != $3
            "#,
        )
        .bind(&track.guild_id)
        .bind(&track.slug)
        .bind(track.id)
        .fetch_one(&self.pool)
        .await?;

        if conflicting > 0 {
            anyhow::bail!(
                "Recruitment track slug '{}' already exists in this server",
                track.slug
            );
        }

        let saved = sqlx::query_as::<_, RecruitmentTrack>(
            r#"
            INSERT INTO recruitment_tracks (
                id, guild_id, slug, name, enabled, target_role_id, config, version, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, 1, NOW())
            ON CONFLICT (id) DO UPDATE SET
                guild_id = EXCLUDED.guild_id,
                slug = EXCLUDED.slug,
                name = EXCLUDED.name,
                enabled = EXCLUDED.enabled,
                target_role_id = EXCLUDED.target_role_id,
                config = EXCLUDED.config,
                version = recruitment_tracks.version + 1,
                updated_at = NOW()
            WHERE recruitment_tracks.guild_id = EXCLUDED.guild_id
            RETURNING id, guild_id, slug, name, enabled, target_role_id, config, version, created_at, updated_at
            "#,
        )
        .bind(track.id)
        .bind(&track.guild_id)
        .bind(&track.slug)
        .bind(&track.name)
        .bind(track.enabled)
        .bind(&track.target_role_id)
        .bind(&track.config)
        .fetch_optional(&self.pool)
        .await?;

        let Some(saved_track) = saved else {
            anyhow::bail!("Track ID belongs to another guild");
        };

        Ok(saved_track)
    }

    pub async fn delete_recruitment_track(
        &self,
        guild_id: &str,
        track_id: Uuid,
    ) -> anyhow::Result<bool> {
        validate_snowflake("guild_id", guild_id)?;
        let result = sqlx::query(
            r#"
            DELETE FROM recruitment_tracks
            WHERE id = $1 AND guild_id = $2
            "#,
        )
        .bind(track_id)
        .bind(guild_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn get_recruitment_application(
        &self,
        guild_id: &str,
        id: Uuid,
    ) -> anyhow::Result<Option<RecruitmentApplication>> {
        validate_snowflake("guild_id", guild_id)?;
        let row = sqlx::query_as::<_, RecruitmentApplication>(
            r#"
            SELECT id, guild_id, track_id, applicant_user_id, track_version, status,
                   submitted_answers, current_stage, version, submitted_at, updated_at,
                   decided_at, decision_reason, decided_by
            FROM recruitment_applications
            WHERE id = $1 AND guild_id = $2
            "#,
        )
        .bind(id)
        .bind(guild_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_recruitment_applications(
        &self,
        guild_id: &str,
        track_id: Option<Uuid>,
        status: Option<&str>,
        applicant_user_id: Option<&str>,
    ) -> anyhow::Result<Vec<RecruitmentApplication>> {
        validate_snowflake("guild_id", guild_id)?;
        if let Some(uid) = applicant_user_id {
            validate_snowflake("applicant_user_id", uid)?;
        }

        let rows = sqlx::query_as::<_, RecruitmentApplication>(
            r#"
            SELECT id, guild_id, track_id, applicant_user_id, track_version, status,
                   submitted_answers, current_stage, version, submitted_at, updated_at,
                   decided_at, decision_reason, decided_by
            FROM recruitment_applications
            WHERE guild_id = $1
              AND ($2::uuid IS NULL OR track_id = $2)
              AND ($3::text IS NULL OR status = $3)
              AND ($4::text IS NULL OR applicant_user_id = $4)
            ORDER BY submitted_at DESC
            "#,
        )
        .bind(guild_id)
        .bind(track_id)
        .bind(status)
        .bind(applicant_user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn create_recruitment_application(
        &self,
        app: RecruitmentApplication,
    ) -> anyhow::Result<RecruitmentApplication> {
        validate_snowflake("guild_id", &app.guild_id)?;
        validate_snowflake("applicant_user_id", &app.applicant_user_id)?;

        let created = sqlx::query_as::<_, RecruitmentApplication>(
            r#"
            INSERT INTO recruitment_applications (
                id, guild_id, track_id, applicant_user_id, track_version, status,
                submitted_answers, current_stage, version, submitted_at, updated_at,
                decided_at, decision_reason, decided_by
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 1, NOW(), NOW(), $9, $10, $11)
            RETURNING id, guild_id, track_id, applicant_user_id, track_version, status,
                      submitted_answers, current_stage, version, submitted_at, updated_at,
                      decided_at, decision_reason, decided_by
            "#,
        )
        .bind(app.id)
        .bind(&app.guild_id)
        .bind(app.track_id)
        .bind(&app.applicant_user_id)
        .bind(app.track_version)
        .bind(&app.status)
        .bind(&app.submitted_answers)
        .bind(&app.current_stage)
        .bind(app.decided_at)
        .bind(&app.decision_reason)
        .bind(&app.decided_by)
        .fetch_one(&self.pool)
        .await?;

        Ok(created)
    }

    pub async fn create_recruitment_application_checked(
        &self,
        app: RecruitmentApplication,
        max_active: u32,
    ) -> anyhow::Result<RecruitmentApplication> {
        validate_snowflake("guild_id", &app.guild_id)?;
        validate_snowflake("applicant_user_id", &app.applicant_user_id)?;

        let mut tx = self.pool.begin().await?;

        let lock_key = format!(
            "{}:{}:{}",
            app.guild_id, app.track_id, app.applicant_user_id
        );
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
            .bind(&lock_key)
            .execute(&mut *tx)
            .await?;

        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM recruitment_applications
            WHERE guild_id = $1 AND applicant_user_id = $2 AND track_id = $3
              AND status IN ('draft', 'submitted', 'screening', 'interview')
            "#,
        )
        .bind(&app.guild_id)
        .bind(&app.applicant_user_id)
        .bind(app.track_id)
        .fetch_one(&mut *tx)
        .await?;

        if count.0 >= max_active as i64 {
            anyhow::bail!(
                "Active applications limit ({}) exceeded for user",
                max_active
            );
        }

        let created = sqlx::query_as::<_, RecruitmentApplication>(
            r#"
            INSERT INTO recruitment_applications (
                id, guild_id, track_id, applicant_user_id, track_version, status,
                submitted_answers, current_stage, version, submitted_at, updated_at,
                decided_at, decision_reason, decided_by
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 1, NOW(), NOW(), $9, $10, $11)
            RETURNING id, guild_id, track_id, applicant_user_id, track_version, status,
                      submitted_answers, current_stage, version, submitted_at, updated_at,
                      decided_at, decision_reason, decided_by
            "#,
        )
        .bind(app.id)
        .bind(&app.guild_id)
        .bind(app.track_id)
        .bind(&app.applicant_user_id)
        .bind(app.track_version)
        .bind(&app.status)
        .bind(&app.submitted_answers)
        .bind(&app.current_stage)
        .bind(app.decided_at)
        .bind(&app.decision_reason)
        .bind(&app.decided_by)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(created)
    }

    pub async fn update_recruitment_application_transition(
        &self,
        params: TransitionApplicationParams<'_>,
    ) -> anyhow::Result<RecruitmentApplication> {
        validate_snowflake("guild_id", params.guild_id)?;

        let updated = sqlx::query_as::<_, RecruitmentApplication>(
            r#"
            UPDATE recruitment_applications
            SET status = $1,
                current_stage = $2,
                version = recruitment_applications.version + 1,
                updated_at = NOW(),
                decided_at = CASE WHEN $1 IN ('accepted', 'rejected', 'withdrawn') THEN NOW() ELSE recruitment_applications.decided_at END,
                decided_by = CASE WHEN $1 IN ('accepted', 'rejected', 'withdrawn') THEN $3 ELSE recruitment_applications.decided_by END,
                decision_reason = CASE WHEN $1 IN ('accepted', 'rejected', 'withdrawn') THEN $4 ELSE recruitment_applications.decision_reason END
            WHERE id = $5 AND guild_id = $6 AND version = $7
            RETURNING id, guild_id, track_id, applicant_user_id, track_version, status,
                      submitted_answers, current_stage, version, submitted_at, updated_at,
                      decided_at, decision_reason, decided_by
            "#,
        )
        .bind(params.new_status)
        .bind(params.new_stage)
        .bind(params.decided_by)
        .bind(params.decision_reason)
        .bind(params.id)
        .bind(params.guild_id)
        .bind(params.expected_version)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(app) = updated {
            return Ok(app);
        }

        let existing = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM recruitment_applications WHERE id = $1 AND guild_id = $2",
        )
        .bind(params.id)
        .bind(params.guild_id)
        .fetch_optional(&self.pool)
        .await?;

        if existing.is_some() {
            anyhow::bail!(
                "Optimistic concurrency conflict: application was modified by another reviewer"
            );
        } else {
            anyhow::bail!("Application not found");
        }
    }

    pub async fn accept_recruitment_application_with_role_enqueue(
        &self,
        params: TransitionApplicationParams<'_>,
        target_role_id: Option<&str>,
    ) -> anyhow::Result<(RecruitmentApplication, Option<RecruitmentRoleFulfillment>)> {
        validate_snowflake("guild_id", params.guild_id)?;
        if let Some(rid) = target_role_id {
            validate_snowflake("target_role_id", rid)?;
        }

        let mut tx = self.pool.begin().await?;

        let updated = sqlx::query_as::<_, RecruitmentApplication>(
            r#"
            UPDATE recruitment_applications
            SET status = 'accepted',
                current_stage = $1,
                version = recruitment_applications.version + 1,
                updated_at = NOW(),
                decided_at = NOW(),
                decided_by = $2,
                decision_reason = $3
            WHERE id = $4 AND guild_id = $5 AND version = $6
            RETURNING id, guild_id, track_id, applicant_user_id, track_version, status,
                      submitted_answers, current_stage, version, submitted_at, updated_at,
                      decided_at, decision_reason, decided_by
            "#,
        )
        .bind(params.new_stage)
        .bind(params.decided_by)
        .bind(params.decision_reason)
        .bind(params.id)
        .bind(params.guild_id)
        .bind(params.expected_version)
        .fetch_optional(&mut *tx)
        .await?;

        let app = match updated {
            Some(a) => a,
            None => {
                let existing = sqlx::query_scalar::<_, i64>(
                    "SELECT version FROM recruitment_applications WHERE id = $1 AND guild_id = $2",
                )
                .bind(params.id)
                .bind(params.guild_id)
                .fetch_optional(&mut *tx)
                .await?;

                if existing.is_some() {
                    anyhow::bail!(
                        "Optimistic concurrency conflict: application was modified by another reviewer"
                    );
                } else {
                    anyhow::bail!("Application not found");
                }
            }
        };

        let mut fulfillment = None;
        if let Some(role_id) = target_role_id {
            let saved_fulfillment = sqlx::query_as::<_, RecruitmentRoleFulfillment>(
                r#"
                INSERT INTO recruitment_role_fulfillment (
                    application_id, guild_id, user_id, role_id, status, attempts, last_error, next_attempt_at, created_at, updated_at
                )
                VALUES ($1, $2, $3, $4, 'pending', 0, NULL, NOW(), NOW(), NOW())
                ON CONFLICT (application_id) DO UPDATE SET
                    status = 'pending',
                    role_id = EXCLUDED.role_id,
                    attempts = 0,
                    last_error = NULL,
                    next_attempt_at = NOW(),
                    updated_at = NOW()
                RETURNING application_id, guild_id, user_id, role_id, status, attempts, last_error, next_attempt_at, created_at, updated_at
                "#,
            )
            .bind(app.id)
            .bind(&app.guild_id)
            .bind(&app.applicant_user_id)
            .bind(role_id)
            .fetch_one(&mut *tx)
            .await?;

            fulfillment = Some(saved_fulfillment);
        }

        tx.commit().await?;
        Ok((app, fulfillment))
    }

    pub async fn cancel_pending_role_fulfillments_for_application(
        &self,
        application_id: Uuid,
    ) -> anyhow::Result<usize> {
        let res = sqlx::query(
            r#"
            UPDATE recruitment_role_fulfillment
            SET status = 'cancelled', updated_at = NOW()
            WHERE application_id = $1 AND status = 'pending'
            "#,
        )
        .bind(application_id)
        .execute(&self.pool)
        .await?;

        Ok(res.rows_affected() as usize)
    }

    pub async fn count_active_user_applications(
        &self,
        guild_id: &str,
        user_id: &str,
        track_id: Uuid,
    ) -> anyhow::Result<u32> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("user_id", user_id)?;

        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)::bigint
            FROM recruitment_applications
            WHERE guild_id = $1 AND applicant_user_id = $2 AND track_id = $3
              AND status IN ('draft', 'submitted', 'screening', 'interview')
            "#,
        )
        .bind(guild_id)
        .bind(user_id)
        .bind(track_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(count as u32)
    }

    pub async fn list_recruitment_reviews(
        &self,
        application_id: Uuid,
    ) -> anyhow::Result<Vec<RecruitmentReview>> {
        let rows = sqlx::query_as::<_, RecruitmentReview>(
            r#"
            SELECT id, application_id, guild_id, reviewer_user_id, recommendation, note, created_at, updated_at
            FROM recruitment_reviews
            WHERE application_id = $1
            ORDER BY created_at ASC
            "#,
        )
        .bind(application_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn list_recruitment_reviews_for_applications(
        &self,
        application_ids: &[Uuid],
    ) -> anyhow::Result<std::collections::HashMap<Uuid, Vec<RecruitmentReview>>> {
        if application_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        let rows = sqlx::query_as::<_, RecruitmentReview>(
            r#"
            SELECT id, application_id, guild_id, reviewer_user_id, recommendation, note, created_at, updated_at
            FROM recruitment_reviews
            WHERE application_id = ANY($1)
            ORDER BY created_at ASC
            "#,
        )
        .bind(application_ids)
        .fetch_all(&self.pool)
        .await?;

        let mut map: std::collections::HashMap<Uuid, Vec<RecruitmentReview>> =
            std::collections::HashMap::new();
        for row in rows {
            map.entry(row.application_id).or_default().push(row);
        }
        Ok(map)
    }

    pub async fn get_role_fulfillments_for_applications(
        &self,
        application_ids: &[Uuid],
    ) -> anyhow::Result<std::collections::HashMap<Uuid, RecruitmentRoleFulfillment>> {
        if application_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        let rows = sqlx::query_as::<_, RecruitmentRoleFulfillment>(
            r#"
            SELECT application_id, guild_id, user_id, role_id, status, attempts, last_error, next_attempt_at, created_at, updated_at
            FROM recruitment_role_fulfillment
            WHERE application_id = ANY($1)
            "#,
        )
        .bind(application_ids)
        .fetch_all(&self.pool)
        .await?;

        let mut map = std::collections::HashMap::new();
        for row in rows {
            map.insert(row.application_id, row);
        }
        Ok(map)
    }

    pub async fn upsert_recruitment_review(
        &self,
        review: RecruitmentReview,
    ) -> anyhow::Result<RecruitmentReview> {
        validate_snowflake("guild_id", &review.guild_id)?;
        validate_snowflake("reviewer_user_id", &review.reviewer_user_id)?;

        let saved = sqlx::query_as::<_, RecruitmentReview>(
            r#"
            INSERT INTO recruitment_reviews (
                id, application_id, guild_id, reviewer_user_id, recommendation, note, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())
            ON CONFLICT (application_id, reviewer_user_id) DO UPDATE SET
                recommendation = EXCLUDED.recommendation,
                note = EXCLUDED.note,
                updated_at = NOW()
            RETURNING id, application_id, guild_id, reviewer_user_id, recommendation, note, created_at, updated_at
            "#,
        )
        .bind(review.id)
        .bind(review.application_id)
        .bind(&review.guild_id)
        .bind(&review.reviewer_user_id)
        .bind(&review.recommendation)
        .bind(&review.note)
        .fetch_one(&self.pool)
        .await?;

        Ok(saved)
    }

    pub async fn get_role_fulfillment(
        &self,
        application_id: Uuid,
    ) -> anyhow::Result<Option<RecruitmentRoleFulfillment>> {
        let row = sqlx::query_as::<_, RecruitmentRoleFulfillment>(
            r#"
            SELECT application_id, guild_id, user_id, role_id, status, attempts, last_error, next_attempt_at, created_at, updated_at
            FROM recruitment_role_fulfillment
            WHERE application_id = $1
            "#,
        )
        .bind(application_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn upsert_role_fulfillment(
        &self,
        fulfillment: RecruitmentRoleFulfillment,
    ) -> anyhow::Result<RecruitmentRoleFulfillment> {
        validate_snowflake("guild_id", &fulfillment.guild_id)?;
        validate_snowflake("user_id", &fulfillment.user_id)?;
        validate_snowflake("role_id", &fulfillment.role_id)?;

        let saved = sqlx::query_as::<_, RecruitmentRoleFulfillment>(
            r#"
            INSERT INTO recruitment_role_fulfillment (
                application_id, guild_id, user_id, role_id, status, attempts, last_error, next_attempt_at, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW(), NOW())
            ON CONFLICT (application_id) DO UPDATE SET
                status = EXCLUDED.status,
                attempts = EXCLUDED.attempts,
                last_error = EXCLUDED.last_error,
                next_attempt_at = EXCLUDED.next_attempt_at,
                updated_at = NOW()
            RETURNING application_id, guild_id, user_id, role_id, status, attempts, last_error, next_attempt_at, created_at, updated_at
            "#,
        )
        .bind(fulfillment.application_id)
        .bind(&fulfillment.guild_id)
        .bind(&fulfillment.user_id)
        .bind(&fulfillment.role_id)
        .bind(&fulfillment.status)
        .bind(fulfillment.attempts)
        .bind(&fulfillment.last_error)
        .bind(fulfillment.next_attempt_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(saved)
    }

    pub async fn list_pending_role_fulfillments(
        &self,
        guild_id: Option<&str>,
    ) -> anyhow::Result<Vec<RecruitmentRoleFulfillment>> {
        if let Some(gid) = guild_id {
            validate_snowflake("guild_id", gid)?;
        }

        let rows = sqlx::query_as::<_, RecruitmentRoleFulfillment>(
            r#"
            SELECT application_id, guild_id, user_id, role_id, status, attempts, last_error, next_attempt_at, created_at, updated_at
            FROM recruitment_role_fulfillment
            WHERE ($1::TEXT IS NULL OR guild_id = $1)
              AND status = 'pending'
              AND next_attempt_at <= NOW()
            ORDER BY next_attempt_at ASC
            "#,
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn get_help_forum_config(
        &self,
        guild_id: &str,
        forum_channel_id: &str,
    ) -> anyhow::Result<Option<HelpForumConfig>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("forum_channel_id", forum_channel_id)?;

        let row = sqlx::query_as::<_, HelpForumConfig>(
            r#"
            SELECT guild_id, forum_channel_id, enabled, idle_close_minutes, owner_can_close,
                   helper_role_ids, lock_on_close, archive_on_close, require_question_tag,
                   welcome_message, created_at, updated_at
            FROM help_forum_configs
            WHERE guild_id = $1 AND forum_channel_id = $2
            "#,
        )
        .bind(guild_id)
        .bind(forum_channel_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_help_forum_configs(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<HelpForumConfig>> {
        validate_snowflake("guild_id", guild_id)?;

        let rows = sqlx::query_as::<_, HelpForumConfig>(
            r#"
            SELECT guild_id, forum_channel_id, enabled, idle_close_minutes, owner_can_close,
                   helper_role_ids, lock_on_close, archive_on_close, require_question_tag,
                   welcome_message, created_at, updated_at
            FROM help_forum_configs
            WHERE guild_id = $1
            ORDER BY created_at ASC
            "#,
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn list_enabled_help_forum_guilds(&self) -> anyhow::Result<Vec<String>> {
        let rows = sqlx::query_scalar::<_, String>(
            r#"
            SELECT DISTINCT guild_id
            FROM help_forum_configs
            WHERE enabled = TRUE
            ORDER BY guild_id ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn upsert_help_forum_config(
        &self,
        config: HelpForumConfig,
    ) -> anyhow::Result<HelpForumConfig> {
        crate::help_forum::validate_help_forum_config(&config)?;

        let saved = sqlx::query_as::<_, HelpForumConfig>(
            r#"
            INSERT INTO help_forum_configs (
                guild_id, forum_channel_id, enabled, idle_close_minutes, owner_can_close,
                helper_role_ids, lock_on_close, archive_on_close, require_question_tag,
                welcome_message, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW())
            ON CONFLICT (guild_id, forum_channel_id) DO UPDATE SET
                enabled = EXCLUDED.enabled,
                idle_close_minutes = EXCLUDED.idle_close_minutes,
                owner_can_close = EXCLUDED.owner_can_close,
                helper_role_ids = EXCLUDED.helper_role_ids,
                lock_on_close = EXCLUDED.lock_on_close,
                archive_on_close = EXCLUDED.archive_on_close,
                require_question_tag = EXCLUDED.require_question_tag,
                welcome_message = EXCLUDED.welcome_message,
                updated_at = NOW()
            RETURNING guild_id, forum_channel_id, enabled, idle_close_minutes, owner_can_close,
                      helper_role_ids, lock_on_close, archive_on_close, require_question_tag,
                      welcome_message, created_at, updated_at
            "#,
        )
        .bind(&config.guild_id)
        .bind(&config.forum_channel_id)
        .bind(config.enabled)
        .bind(config.idle_close_minutes)
        .bind(config.owner_can_close)
        .bind(&config.helper_role_ids)
        .bind(config.lock_on_close)
        .bind(config.archive_on_close)
        .bind(config.require_question_tag)
        .bind(&config.welcome_message)
        .fetch_one(&self.pool)
        .await?;

        Ok(saved)
    }

    pub async fn delete_help_forum_config(
        &self,
        guild_id: &str,
        forum_channel_id: &str,
    ) -> anyhow::Result<bool> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("forum_channel_id", forum_channel_id)?;

        let res = sqlx::query(
            "DELETE FROM help_forum_configs WHERE guild_id = $1 AND forum_channel_id = $2",
        )
        .bind(guild_id)
        .bind(forum_channel_id)
        .execute(&self.pool)
        .await?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn get_help_forum_session(
        &self,
        guild_id: &str,
        thread_id: &str,
    ) -> anyhow::Result<Option<HelpForumSession>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("thread_id", thread_id)?;

        let row = sqlx::query_as::<_, HelpForumSession>(
            r#"
            SELECT guild_id, forum_channel_id, thread_id, owner_user_id, status, opened_at,
                   last_activity_at, first_helper_response_at, closed_at, closing_reason, generation
            FROM help_forum_sessions
            WHERE guild_id = $1 AND thread_id = $2
            "#,
        )
        .bind(guild_id)
        .bind(thread_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_help_forum_sessions(
        &self,
        guild_id: &str,
        forum_channel_id: Option<&str>,
        status: Option<&str>,
    ) -> anyhow::Result<Vec<HelpForumSession>> {
        validate_snowflake("guild_id", guild_id)?;
        if let Some(fid) = forum_channel_id {
            validate_snowflake("forum_channel_id", fid)?;
        }

        let rows = sqlx::query_as::<_, HelpForumSession>(
            r#"
            SELECT guild_id, forum_channel_id, thread_id, owner_user_id, status, opened_at,
                   last_activity_at, first_helper_response_at, closed_at, closing_reason, generation
            FROM help_forum_sessions
            WHERE guild_id = $1
              AND ($2::text IS NULL OR forum_channel_id = $2)
              AND ($3::text IS NULL OR status = $3)
            ORDER BY opened_at DESC
            "#,
        )
        .bind(guild_id)
        .bind(forum_channel_id)
        .bind(status)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn create_help_forum_session(
        &self,
        session: HelpForumSession,
    ) -> anyhow::Result<HelpForumSession> {
        validate_snowflake("guild_id", &session.guild_id)?;
        validate_snowflake("forum_channel_id", &session.forum_channel_id)?;
        validate_snowflake("thread_id", &session.thread_id)?;
        validate_snowflake("owner_user_id", &session.owner_user_id)?;

        let status = if session.status.is_empty() {
            "open"
        } else {
            &session.status
        };
        let generation = if session.generation == 0 {
            1
        } else {
            session.generation
        };

        let saved = sqlx::query_as::<_, HelpForumSession>(
            r#"
            INSERT INTO help_forum_sessions (
                guild_id, forum_channel_id, thread_id, owner_user_id, status, opened_at,
                last_activity_at, first_helper_response_at, closed_at, closing_reason, generation
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            RETURNING guild_id, forum_channel_id, thread_id, owner_user_id, status, opened_at,
                      last_activity_at, first_helper_response_at, closed_at, closing_reason, generation
            "#,
        )
        .bind(&session.guild_id)
        .bind(&session.forum_channel_id)
        .bind(&session.thread_id)
        .bind(&session.owner_user_id)
        .bind(status)
        .bind(session.opened_at)
        .bind(session.last_activity_at)
        .bind(session.first_helper_response_at)
        .bind(session.closed_at)
        .bind(&session.closing_reason)
        .bind(generation)
        .fetch_one(&self.pool)
        .await?;

        Ok(saved)
    }

    pub async fn record_help_forum_activity(
        &self,
        guild_id: &str,
        thread_id: &str,
        author_user_id: &str,
        is_helper: bool,
    ) -> anyhow::Result<Option<HelpForumSession>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("thread_id", thread_id)?;
        validate_snowflake("author_user_id", author_user_id)?;

        let updated = sqlx::query_as::<_, HelpForumSession>(
            r#"
            UPDATE help_forum_sessions
            SET generation = generation + 1,
                last_activity_at = NOW(),
                first_helper_response_at = CASE
                    WHEN $3 = true AND $4 != owner_user_id AND first_helper_response_at IS NULL THEN NOW()
                    ELSE first_helper_response_at
                END
            WHERE guild_id = $1 AND thread_id = $2 AND status = 'open'
            RETURNING guild_id, forum_channel_id, thread_id, owner_user_id, status, opened_at,
                      last_activity_at, first_helper_response_at, closed_at, closing_reason, generation
            "#,
        )
        .bind(guild_id)
        .bind(thread_id)
        .bind(is_helper)
        .bind(author_user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(updated)
    }

    pub async fn close_help_forum_session(
        &self,
        guild_id: &str,
        thread_id: &str,
        new_status: &str,
        reason: Option<&str>,
        expected_generation: Option<i64>,
    ) -> anyhow::Result<bool> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("thread_id", thread_id)?;

        let res = sqlx::query(
            r#"
            UPDATE help_forum_sessions
            SET status = $3,
                closed_at = NOW(),
                closing_reason = $4
            WHERE guild_id = $1 AND thread_id = $2 AND status IN ('open', 'needs_tag')
              AND ($5::bigint IS NULL OR generation = $5)
            "#,
        )
        .bind(guild_id)
        .bind(thread_id)
        .bind(new_status)
        .bind(reason)
        .bind(expected_generation)
        .execute(&self.pool)
        .await?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn mark_help_forum_owner_left(
        &self,
        guild_id: &str,
        owner_user_id: &str,
    ) -> anyhow::Result<usize> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("owner_user_id", owner_user_id)?;

        let res = sqlx::query(
            r#"
            UPDATE help_forum_sessions
            SET status = 'owner_left',
                closed_at = NOW(),
                closing_reason = 'owner_left'
            WHERE guild_id = $1 AND owner_user_id = $2 AND status = 'open'
            "#,
        )
        .bind(guild_id)
        .bind(owner_user_id)
        .execute(&self.pool)
        .await?;

        Ok(res.rows_affected() as usize)
    }

    pub async fn get_help_forum_stats(
        &self,
        guild_id: &str,
        forum_channel_id: &str,
    ) -> anyhow::Result<HelpForumStats> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("forum_channel_id", forum_channel_id)?;

        let sessions = sqlx::query_as::<_, HelpForumSession>(
            r#"
            SELECT guild_id, forum_channel_id, thread_id, owner_user_id, status, opened_at,
                   last_activity_at, first_helper_response_at, closed_at, closing_reason, generation
            FROM help_forum_sessions
            WHERE guild_id = $1 AND forum_channel_id = $2
            "#,
        )
        .bind(guild_id)
        .bind(forum_channel_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(crate::help_forum::calculate_stats_from_sessions(
            forum_channel_id,
            &sessions,
        ))
    }

    pub async fn record_asset_usage_batch(
        &self,
        daily_items: &[crate::asset_analytics::AssetUsageDaily],
        inventory_items: &[crate::asset_analytics::AssetInventoryItem],
    ) -> anyhow::Result<usize> {
        let mut tx = self.pool.begin().await?;

        for item in daily_items {
            validate_snowflake("guild_id", &item.guild_id)?;
            validate_snowflake("asset_id", &item.asset_id)?;

            sqlx::query(
                r#"
                INSERT INTO guild_asset_usage_daily (
                    guild_id, usage_date, asset_kind, asset_id, usage_count, last_used_at
                ) VALUES ($1, $2, $3, $4, $5, $6)
                ON CONFLICT (guild_id, usage_date, asset_kind, asset_id)
                DO UPDATE SET
                    usage_count = guild_asset_usage_daily.usage_count + EXCLUDED.usage_count,
                    last_used_at = GREATEST(guild_asset_usage_daily.last_used_at, EXCLUDED.last_used_at)
                "#,
            )
            .bind(&item.guild_id)
            .bind(item.usage_date)
            .bind(&item.asset_kind)
            .bind(&item.asset_id)
            .bind(item.usage_count)
            .bind(item.last_used_at)
            .execute(&mut *tx)
            .await?;
        }

        for item in inventory_items {
            validate_snowflake("guild_id", &item.guild_id)?;
            validate_snowflake("asset_id", &item.asset_id)?;

            sqlx::query(
                r#"
                INSERT INTO guild_asset_inventory (
                    guild_id, asset_kind, asset_id, name, animated, is_guild_owned, is_active, first_seen_at, last_seen_at
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                ON CONFLICT (guild_id, asset_kind, asset_id)
                DO UPDATE SET
                    name = COALESCE(EXCLUDED.name, guild_asset_inventory.name),
                    animated = EXCLUDED.animated,
                    is_guild_owned = COALESCE(EXCLUDED.is_guild_owned, guild_asset_inventory.is_guild_owned),
                    last_seen_at = GREATEST(guild_asset_inventory.last_seen_at, EXCLUDED.last_seen_at)
                "#,
            )
            .bind(&item.guild_id)
            .bind(&item.asset_kind)
            .bind(&item.asset_id)
            .bind(&item.name)
            .bind(item.animated)
            .bind(item.is_guild_owned)
            .bind(item.is_active)
            .bind(item.first_seen_at)
            .bind(item.last_seen_at)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(daily_items.len())
    }

    pub async fn get_asset_usage_summary(
        &self,
        guild_id: &str,
        kind: Option<crate::asset_analytics::AssetKind>,
        period: &str,
    ) -> anyhow::Result<crate::asset_analytics::AssetUsageSummary> {
        validate_snowflake("guild_id", guild_id)?;

        let now = Utc::now();
        let today = now.date_naive();
        let seven_days_ago = today - chrono::Duration::days(6);
        let thirty_days_ago = today - chrono::Duration::days(29);
        let kind_str = kind.map(|k| k.as_str().to_string());

        let aggregates = sqlx::query_as::<_, crate::asset_analytics::AssetUsageAggregate>(
            r#"
            SELECT
                u.asset_kind,
                u.asset_id,
                SUM(u.usage_count)::bigint AS usage_total,
                COALESCE(SUM(u.usage_count) FILTER (WHERE u.usage_date >= $3), 0)::bigint AS usage_7d,
                COALESCE(SUM(u.usage_count) FILTER (WHERE u.usage_date >= $4), 0)::bigint AS usage_30d,
                COALESCE(SUM(u.usage_count) FILTER (WHERE u.usage_date = $5), 0)::bigint AS usage_today,
                COALESCE(SUM(u.usage_count) FILTER (
                    WHERE CASE
                        WHEN $6 = 'today' THEN u.usage_date = $5
                        WHEN $6 = '7d' THEN u.usage_date >= $3
                        WHEN $6 = '30d' THEN u.usage_date >= $4
                        ELSE true
                    END
                ), 0)::bigint AS period_usage,
                MAX(u.last_used_at) AS last_used_at
            FROM guild_asset_usage_daily u
            WHERE u.guild_id = $1 AND ($2::TEXT IS NULL OR u.asset_kind = $2)
            GROUP BY u.asset_kind, u.asset_id
            "#,
        )
        .bind(guild_id)
        .bind(kind_str.as_deref())
        .bind(seven_days_ago)
        .bind(thirty_days_ago)
        .bind(today)
        .bind(period)
        .fetch_all(&self.pool)
        .await?;

        let inventory_rows = sqlx::query_as::<_, crate::asset_analytics::AssetInventoryItem>(
            r#"
            SELECT guild_id, asset_kind, asset_id, name, animated, is_guild_owned, is_active, first_seen_at, last_seen_at
            FROM guild_asset_inventory
            WHERE guild_id = $1 AND ($2::TEXT IS NULL OR asset_kind = $2)
            "#,
        )
        .bind(guild_id)
        .bind(kind_str.as_deref())
        .fetch_all(&self.pool)
        .await?;

        Ok(crate::asset_analytics::build_asset_summary_from_aggregates(
            guild_id,
            kind,
            period,
            &aggregates,
            &inventory_rows,
            now,
        ))
    }

    pub async fn sync_guild_inventory_active_state(
        &self,
        guild_id: &str,
        kind: crate::asset_analytics::AssetKind,
        active_ids: &[String],
    ) -> anyhow::Result<usize> {
        validate_snowflake("guild_id", guild_id)?;
        let kind_str = kind.as_str();

        let res = sqlx::query(
            r#"
            UPDATE guild_asset_inventory
            SET is_active = CASE WHEN asset_id = ANY($3) THEN true ELSE false END
            WHERE guild_id = $1 AND asset_kind = $2 AND is_guild_owned = true
            "#,
        )
        .bind(guild_id)
        .bind(kind_str)
        .bind(active_ids)
        .execute(&self.pool)
        .await?;

        Ok(res.rows_affected() as usize)
    }

    /// Atomically reconciles the full active asset list for a guild and kind (Issue #201).
    /// Upserts all active items and sets missing guild-owned items to is_active = false.
    pub async fn reconcile_guild_inventory(
        &self,
        guild_id: &str,
        kind: crate::asset_analytics::AssetKind,
        active_items: &[crate::asset_analytics::AssetInventoryItem],
    ) -> anyhow::Result<usize> {
        validate_snowflake("guild_id", guild_id)?;
        let kind_str = kind.as_str();

        let mut tx = self.pool.begin().await?;

        let mut active_ids = Vec::with_capacity(active_items.len());
        for item in active_items {
            validate_snowflake("asset_id", &item.asset_id)?;
            active_ids.push(item.asset_id.clone());

            sqlx::query(
                r#"
                INSERT INTO guild_asset_inventory (
                    guild_id, asset_kind, asset_id, name, animated, is_guild_owned, is_active, first_seen_at, last_seen_at
                ) VALUES ($1, $2, $3, $4, $5, true, true, $6, $7)
                ON CONFLICT (guild_id, asset_kind, asset_id)
                DO UPDATE SET
                    name = COALESCE(EXCLUDED.name, guild_asset_inventory.name),
                    animated = EXCLUDED.animated,
                    is_guild_owned = true,
                    is_active = true,
                    last_seen_at = GREATEST(guild_asset_inventory.last_seen_at, EXCLUDED.last_seen_at)
                "#,
            )
            .bind(guild_id)
            .bind(kind_str)
            .bind(&item.asset_id)
            .bind(&item.name)
            .bind(item.animated)
            .bind(item.first_seen_at)
            .bind(item.last_seen_at)
            .execute(&mut *tx)
            .await?;
        }

        // Deactivate all guild-owned assets for this guild and kind not in active_ids
        let res = sqlx::query(
            r#"
            UPDATE guild_asset_inventory
            SET is_active = false
            WHERE guild_id = $1 AND asset_kind = $2 AND is_guild_owned = true AND NOT (asset_id = ANY($3))
            "#,
        )
        .bind(guild_id)
        .bind(kind_str)
        .bind(&active_ids)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(res.rows_affected() as usize)
    }

    pub async fn list_asset_inventory(
        &self,
        guild_id: &str,
        kind: Option<crate::asset_analytics::AssetKind>,
    ) -> anyhow::Result<Vec<crate::asset_analytics::AssetUsageStat>> {
        validate_snowflake("guild_id", guild_id)?;
        let summary = self.get_asset_usage_summary(guild_id, kind, "all").await?;
        Ok(summary.items)
    }

    pub async fn prune_asset_analytics(
        &self,
        guild_id: Option<&str>,
        older_than_days: u32,
    ) -> anyhow::Result<usize> {
        if let Some(gid) = guild_id {
            validate_snowflake("guild_id", gid)?;
        }

        let cutoff =
            chrono::Utc::now().date_naive() - chrono::Duration::days(older_than_days as i64);

        let result = sqlx::query(
            r#"
            DELETE FROM guild_asset_usage_daily
            WHERE ($1::TEXT IS NULL OR guild_id = $1)
              AND usage_date < $2
            "#,
        )
        .bind(guild_id)
        .bind(cutoff)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() as usize)
    }
}

async fn ensure_profile_on_pool(
    pool: &PgPool,

    guild_id: &str,

    user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"

        INSERT INTO member_progress (guild_id, user_id)

        VALUES ($1, $2)

        ON CONFLICT (guild_id, user_id) DO NOTHING

        "#,
    )
    .bind(guild_id)
    .bind(user_id)
    .execute(pool)
    .await?;

    Ok(())
}

async fn fetch_profile_on_pool(
    pool: &PgPool,

    guild_id: &str,

    user_id: &str,
) -> Result<MemberProgress, sqlx::Error> {
    sqlx::query_as::<_, MemberProgress>(
        r#"

        SELECT guild_id, user_id, xp, level, coins, reputation, messages, daily_streak,

               last_xp_at, last_daily_at, created_at, updated_at

        FROM member_progress

        WHERE guild_id = $1 AND user_id = $2

        "#,
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_one(pool)
    .await
}

async fn ensure_profile(
    transaction: &mut Transaction<'_, Postgres>,

    guild_id: &str,

    user_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"

        INSERT INTO member_progress (guild_id, user_id)

        VALUES ($1, $2)

        ON CONFLICT (guild_id, user_id) DO NOTHING

        "#,
    )
    .bind(guild_id)
    .bind(user_id)
    .execute(&mut **transaction)
    .await?;

    Ok(())
}

async fn lock_profile(
    transaction: &mut Transaction<'_, Postgres>,

    guild_id: &str,

    user_id: &str,
) -> Result<MemberProgress, sqlx::Error> {
    sqlx::query_as::<_, MemberProgress>(
        r#"

        SELECT guild_id, user_id, xp, level, coins, reputation, messages, daily_streak,

               last_xp_at, last_daily_at, created_at, updated_at

        FROM member_progress

        WHERE guild_id = $1 AND user_id = $2

        FOR UPDATE

        "#,
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_one(&mut **transaction)
    .await
}

async fn write_profile(
    transaction: &mut Transaction<'_, Postgres>,

    profile: &MemberProgress,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"

        UPDATE member_progress

        SET xp = $3,

            level = $4,

            coins = $5,

            reputation = $6,

            messages = $7,

            daily_streak = $8,

            last_xp_at = $9,

            last_daily_at = $10,

            updated_at = $11

        WHERE guild_id = $1 AND user_id = $2

        "#,
    )
    .bind(&profile.guild_id)
    .bind(&profile.user_id)
    .bind(profile.xp)
    .bind(profile.level)
    .bind(profile.coins)
    .bind(profile.reputation)
    .bind(profile.messages)
    .bind(profile.daily_streak)
    .bind(profile.last_xp_at)
    .bind(profile.last_daily_at)
    .bind(profile.updated_at)
    .execute(&mut **transaction)
    .await?;

    Ok(())
}
