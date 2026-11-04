use chrono::{DateTime, Duration, Utc};

use serde_json::Value;

use sqlx::{PgPool, Postgres, Transaction, postgres::PgPoolOptions};

use uuid::Uuid;

use super::{
    AdjustMemberProgressParams, AiGuildBudget, AiKnowledgeDocument, AuditLogEntry, CardCollection,
    CardTrade, CoinTransfer, CoinTransferOutcome, CommunityPoll, CountingGame, CustomAutomation,
    DailyClaimOutcome, DynamicVoiceRoom, EventRoster, FaqEntry, FeedSubscription, GameAccountLink,
    GamingHubGameStatus, GitHubDevopsMapping, GitLabSubscription, Giveaway, GuildModuleSettings,
    IntegrationConnection, IntegrationDeliveryLog, IntegrationWebhookSubscription, InviteTracking,
    KickStreamSubscription, LinkedAccount, McpToolAuthorization, MemberProgress, ModerationAppeal,
    ModerationExecutionIntent, ModuleDraftConfig, PluginGuildInstallation,
    PluginMarketplacePackage, ProgressMetric, ProgressionSeason, RedditSubscription,
    ReputationGrantOutcome, ScheduledEvent, ServerSnapshot, SocialPublicationDraft, StarboardEntry,
    SteamLfgPost, StickyMessage, StoreItem, TelegramBridge, TelegramMessageMapping, TimedRoleGrant,
    Tournament, TupperProfile, TwitchStreamSubscription, VkSubscription, WorkflowDefinition,
    WorkflowRunExecution, XpAward, YouTubeChannelSubscription, level_for_xp,
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

            WHERE guild_id = $1

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

    pub async fn get_member_progress(
        &self,

        guild_id: &str,

        user_id: &str,
    ) -> anyhow::Result<MemberProgress> {
        ensure_profile_on_pool(&self.pool, guild_id, user_id).await?;

        Ok(fetch_profile_on_pool(&self.pool, guild_id, user_id).await?)
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

        appeal_id: Uuid,

        status: &str,

        staff_notes: Option<&str>,
    ) -> anyhow::Result<bool> {
        let res = sqlx::query(
            r#"

            UPDATE moderation_appeals

            SET status = $2, staff_notes = COALESCE($3, staff_notes), updated_at = NOW()

            WHERE id = $1

            "#,
        )
        .bind(appeal_id)
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
        let id = Uuid::new_v4();

        let empty_json = serde_json::json!([]);

        let now = Utc::now();

        let rec = sqlx::query_as::<_, Giveaway>(

            r#"

            INSERT INTO giveaways (id, guild_id, channel_id, prize, winners_count, participants_json, winners_json, ended, expires_at, created_at)

            VALUES ($1, $2, $3, $4, $5, $6, $6, false, $7, $8)

            RETURNING id, guild_id, channel_id, prize, winners_count, participants_json, winners_json, ended, expires_at, created_at

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

        .fetch_one(&self.pool)

        .await?;

        Ok(rec)
    }

    pub async fn get_giveaway(&self, giveaway_id: Uuid) -> anyhow::Result<Option<Giveaway>> {
        let row = sqlx::query_as::<_, Giveaway>(

            r#"

            SELECT id, guild_id, channel_id, prize, winners_count, participants_json, winners_json, ended, expires_at, created_at

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

            SELECT id, guild_id, channel_id, prize, winners_count, participants_json, winners_json, ended, expires_at, created_at

            FROM giveaways

            WHERE id = $1

            FOR UPDATE

            "#,

        )

        .bind(giveaway_id)

        .fetch_optional(&mut *tx)

        .await?;

        let mut ga = match ga {
            Some(g) if !g.ended => g,

            _ => return Ok(false),
        };

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

    pub async fn reroll_giveaway(&self, giveaway_id: Uuid) -> anyhow::Result<Vec<String>> {
        let mut tx = self.pool.begin().await?;

        let ga = sqlx::query_as::<_, Giveaway>(

            r#"

            SELECT id, guild_id, channel_id, prize, winners_count, participants_json, winners_json, ended, expires_at, created_at

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

            None => return Ok(Vec::new()),
        };

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
    ) -> anyhow::Result<(StoreItem, i64)> {
        let mut tx = self.pool.begin().await?;

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

        profile.updated_at = Utc::now();

        write_profile(&mut tx, &profile).await?;

        let purchase_id = Uuid::new_v4();

        sqlx::query(
            r#"

            INSERT INTO store_purchases (id, guild_id, user_id, item_id, price_paid, created_at)

            VALUES ($1, $2, $3, $4, $5, $6)

            "#,
        )
        .bind(purchase_id)
        .bind(guild_id)
        .bind(user_id)
        .bind(item_id)
        .bind(item.price)
        .bind(Utc::now())
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok((item, profile.coins))
    }

    pub async fn archive_progression_season(
        &self,

        guild_id: &str,

        name: &str,
    ) -> anyhow::Result<ProgressionSeason> {
        let standings_rows: Vec<(String, i64, i64, i64)> = sqlx::query_as(
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

        max_main: i32,
    ) -> anyhow::Result<EventRoster> {
        let mut tx = self.pool.begin().await?;

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
            return Ok(r);
        }

        let main_count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM event_rosters WHERE event_id = $1 AND role_type = 'main'",
        )
        .bind(event_id)
        .fetch_one(&mut *tx)
        .await?;

        let role_type = if (main_count.0 as i32) < max_main {
            "main"
        } else {
            "reserve"
        };

        let total_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM event_rosters WHERE event_id = $1")
                .bind(event_id)
                .fetch_one(&mut *tx)
                .await?;

        let position = total_count.0 as i32 + 1;

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

                ORDER BY position ASC

                LIMIT 1

                "#,
            )
            .bind(event_id)
            .fetch_optional(&mut *tx)
            .await?;

            if let Some(reserve) = next_reserve {
                sqlx::query("UPDATE event_rosters SET role_type = 'main' WHERE event_id = $1 AND user_id = $2")

                    .bind(event_id)

                    .bind(&reserve.user_id)

                    .execute(&mut *tx)

                    .await?;

                promoted = Some(reserve.user_id);
            }
        }

        tx.commit().await?;

        Ok(promoted)
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

            SET platform_account_id = $5, verification_code = $6, verified = $7

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

            WHERE guild_id = $1 AND user_id = $2 AND platform = $3 AND verification_code = $4 AND verified = false AND expires_at > NOW()

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

        let c1 = sqlx::query_as::<_, CardCollection>(

            "SELECT id, guild_id, user_id, card_id, card_name, rarity, acquired_at FROM card_collections WHERE id = $1 FOR UPDATE"

        )

        .bind(trade.initiator_card_id)

        .fetch_optional(&mut *tx)

        .await?;

        let c1 = c1.ok_or_else(|| anyhow::anyhow!("initiator card not found"))?;

        let c2 = sqlx::query_as::<_, CardCollection>(

            "SELECT id, guild_id, user_id, card_id, card_name, rarity, acquired_at FROM card_collections WHERE id = $1 FOR UPDATE"

        )

        .bind(trade.target_card_id)

        .fetch_optional(&mut *tx)

        .await?;

        let c2 = c2.ok_or_else(|| anyhow::anyhow!("target card not found"))?;

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
    ) -> anyhow::Result<TupperProfile> {
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

        .await?;

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
    ) -> anyhow::Result<Vec<FaqEntry>> {
        let rows = if let Some(q) = query {
            let pattern = format!("%{}%", q);

            sqlx::query_as::<_, FaqEntry>(

                r#"

                SELECT id, guild_id, question, answer, aliases_json, category, created_at, updated_at

                FROM faq_entries

                WHERE guild_id = $1 AND (question ILIKE $2 OR answer ILIKE $2 OR category ILIKE $2)

                "#,

            )

            .bind(guild_id)

            .bind(pattern)

            .fetch_all(&self.pool)

            .await?
        } else {
            sqlx::query_as::<_, FaqEntry>(

                r#"

                SELECT id, guild_id, question, answer, aliases_json, category, created_at, updated_at

                FROM faq_entries

                WHERE guild_id = $1

                "#,

            )

            .bind(guild_id)

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

        .bind(provider)

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

    pub async fn get_or_create_ai_budget(&self, guild_id: &str) -> anyhow::Result<AiGuildBudget> {
        let row = sqlx::query_as::<_, AiGuildBudget>(

            r#"

            INSERT INTO ai_guild_budgets (guild_id, monthly_token_limit, used_tokens, monthly_usd_limit, used_usd, is_halted, reset_at)

            VALUES ($1, 1000000, 0, 10.0, 0.0, false, NOW() + INTERVAL '30 days')

            ON CONFLICT (guild_id) DO UPDATE SET guild_id = EXCLUDED.guild_id

            RETURNING *

            "#

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
    ) -> anyhow::Result<Vec<AiKnowledgeDocument>> {
        let rows = sqlx::query_as::<_, AiKnowledgeDocument>(

            "SELECT * FROM ai_knowledge_documents WHERE guild_id = $1 AND (title ILIKE $2 OR content ILIKE $2)"

        )

        .bind(guild_id)

        .bind(format!("%{query}%"))

        .fetch_all(&self.pool)

        .await?;

        // Filter by channel ACL

        let filtered = rows
            .into_iter()
            .filter(|e| {
                let acls: Vec<String> =
                    serde_json::from_value(e.channel_acl.clone()).unwrap_or_default();

                acls.is_empty() || acls.iter().any(|c| user_channels.contains(c))
            })
            .collect();

        Ok(filtered)
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
