use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use chrono::{DateTime, Duration, Utc};

use dashmap::{DashMap, DashSet};

use serde_json::Value;

use uuid::Uuid;

use super::{
    AdjustMemberProgressParams, AiGuildBudget, AiKnowledgeDocument, AuditLogEntry, CardCollection,
    CardTrade, CleanupPlan, CoinTransfer, CoinTransferOutcome, CommandPolicyRule, CommunityPoll,
    CompanionChannelLease, CompanionChannelMapping, CompanionMappingItem, CountingGame,
    CustomAutomation, DailyClaimOutcome, DynamicVoiceRoom, EventRoster, FaqEntry, FeedSubscription,
    GameAccountLink, GamingHubGameStatus, GitHubDevopsMapping, GitLabSubscription, Giveaway,
    GuildModuleSettings, IntegrationConnection, IntegrationDeliveryLog,
    IntegrationWebhookSubscription, InviteTracking, KickStreamSubscription, LinkedAccount,
    MAX_COMPANION_MAPPINGS_PER_GUILD, McpToolAuthorization, MemberIdentityRecord, MemberProgress,
    MemberSearchFilter, MemberSearchRow, MemberSnapshot, ModerationAppeal,
    ModerationExecutionIntent, ModeratorDutyProfile, ModeratorDutySettings, ModeratorDutyState,
    ModuleDraftConfig, PluginGuildInstallation, PluginMarketplacePackage, ProgressMetric,
    ProgressionSeason, RecordCompanionLeaseParams, RedditSubscription, ReputationGrantOutcome,
    RetainedRecordNotice, ScheduledEvent, ServerSnapshot, SocialPublicationDraft, StarboardEntry,
    SteamLfgPost, StickyMessage, StoreItem, StorePurchase, TelegramBridge, TelegramMessageMapping,
    TimedRoleGrant, Tournament, TupperError, TupperProfile, TwitchStreamSubscription,
    UpsertMemberSnapshotParams, UserErasureReport, VkSubscription, WorkflowDefinition,
    WorkflowRunExecution, XpAward, YouTubeChannelSubscription, is_member_after_cursor,
    level_for_xp, parse_member_search_cursor, snowflake_created_at, validate_snowflake,
};

type ModuleKey = (String, String);

type MemberKey = (String, String);

type ReputationKey = (String, String);

#[derive(Clone, Default)]
pub struct MemoryStore {
    settings: Arc<DashMap<ModuleKey, GuildModuleSettings>>,

    settings_lock: Arc<Mutex<()>>,

    audit_log: Arc<DashMap<u64, Value>>,

    next_audit_id: Arc<std::sync::atomic::AtomicU64>,

    audit_failure_injected: Arc<std::sync::atomic::AtomicBool>,

    progression_failure_injected: Arc<std::sync::atomic::AtomicBool>,

    pub(crate) ping_failure_injected: Arc<std::sync::atomic::AtomicBool>,

    progression: Arc<DashMap<MemberKey, MemberProgress>>,

    reputation_cooldowns: Arc<DashMap<ReputationKey, chrono::DateTime<Utc>>>,

    progression_lock: Arc<Mutex<()>>,

    role_rewards: Arc<DashSet<(String, String, String)>>,

    moderation_intents: Arc<DashMap<Uuid, ModerationExecutionIntent>>,

    module_drafts: Arc<DashMap<ModuleKey, ModuleDraftConfig>>,

    appeals: Arc<DashMap<Uuid, ModerationAppeal>>,

    timed_roles: Arc<DashMap<Uuid, TimedRoleGrant>>,

    persistent_roles: Arc<DashMap<MemberKey, Vec<String>>>,

    server_snapshots: Arc<DashMap<Uuid, ServerSnapshot>>,

    custom_automations: Arc<DashMap<Uuid, CustomAutomation>>,

    sticky_messages: Arc<DashMap<(String, String), StickyMessage>>,

    polls: Arc<DashMap<Uuid, CommunityPoll>>,

    giveaways: Arc<DashMap<Uuid, Giveaway>>,

    starboard_entries: Arc<DashMap<(String, String), StarboardEntry>>,

    pub(crate) store_items: Arc<DashMap<Uuid, StoreItem>>,

    store_purchases: Arc<DashMap<Uuid, StorePurchase>>,

    progression_seasons: Arc<DashMap<Uuid, ProgressionSeason>>,

    events: Arc<DashMap<Uuid, ScheduledEvent>>,

    rosters: Arc<DashMap<Uuid, Vec<EventRoster>>>,

    tournaments: Arc<DashMap<Uuid, Tournament>>,

    invites: Arc<DashMap<(String, String), InviteTracking>>,

    dynamic_voice: Arc<DashMap<String, DynamicVoiceRoom>>,

    feeds: Arc<DashMap<Uuid, FeedSubscription>>,

    game_links: Arc<DashMap<(String, String, String), GameAccountLink>>,

    counting_games: Arc<DashMap<(String, String), CountingGame>>,

    cards: Arc<DashMap<Uuid, CardCollection>>,

    card_trades: Arc<DashMap<Uuid, CardTrade>>,

    card_trade_lock: Arc<Mutex<()>>,

    poll_votes: Arc<DashMap<(Uuid, String), usize>>,

    tuppers: Arc<DashMap<Uuid, TupperProfile>>,

    tupper_prefix_index: Arc<DashMap<(String, String, String), Uuid>>,

    identity_history: Arc<DashMap<Uuid, MemberIdentityRecord>>,

    member_snapshots: Arc<DashMap<(String, String), MemberSnapshot>>,

    member_case_counts: Arc<DashMap<(String, String), i64>>,

    cleanup_plans: Arc<DashMap<Uuid, CleanupPlan>>,

    companion_mappings: Arc<DashMap<Uuid, CompanionChannelMapping>>,

    companion_leases: Arc<DashMap<(String, String, String), CompanionChannelLease>>,

    companion_lock: Arc<Mutex<()>>,

    policy_rules: Arc<DashMap<Uuid, CommandPolicyRule>>,

    policy_revisions: Arc<DashMap<String, i64>>,

    duty_settings: Arc<DashMap<String, ModeratorDutySettings>>,

    duty_profiles: Arc<DashMap<(String, String), ModeratorDutyProfile>>,

    duty_states: Arc<DashMap<(String, String), ModeratorDutyState>>,

    duty_role_grants: Arc<DashMap<(String, String, String), crate::duty::ModeratorDutyRoleGrant>>,

    recruitment_tracks: Arc<DashMap<Uuid, crate::recruitment::RecruitmentTrack>>,

    recruitment_applications: Arc<DashMap<Uuid, crate::recruitment::RecruitmentApplication>>,

    recruitment_reviews: Arc<DashMap<Uuid, crate::recruitment::RecruitmentReview>>,

    recruitment_role_fulfillments:
        Arc<DashMap<Uuid, crate::recruitment::RecruitmentRoleFulfillment>>,

    recruitment_lock: Arc<Mutex<()>>,

    help_forum_configs: Arc<DashMap<(String, String), crate::help_forum::HelpForumConfig>>,

    help_forum_sessions: Arc<DashMap<(String, String), crate::help_forum::HelpForumSession>>,

    faqs: Arc<DashMap<Uuid, FaqEntry>>,

    integration_connections: Arc<DashMap<Uuid, IntegrationConnection>>,

    integration_webhook_subs: Arc<DashMap<Uuid, IntegrationWebhookSubscription>>,

    integration_event_dedupes: Arc<DashMap<(String, String), chrono::DateTime<Utc>>>,

    integration_delivery_logs: Arc<DashMap<Uuid, IntegrationDeliveryLog>>,

    linked_accounts: Arc<DashMap<Uuid, LinkedAccount>>,

    steam_lfg_posts: Arc<DashMap<Uuid, SteamLfgPost>>,

    github_devops_mappings: Arc<DashMap<(String, String, i32), GitHubDevopsMapping>>,

    twitch_subs: Arc<DashMap<(String, String), TwitchStreamSubscription>>,

    kick_subs: Arc<DashMap<(String, String), KickStreamSubscription>>,

    youtube_subs: Arc<DashMap<(String, String), YouTubeChannelSubscription>>,

    telegram_bridges: Arc<DashMap<Uuid, TelegramBridge>>,

    telegram_mappings: Arc<DashMap<Uuid, TelegramMessageMapping>>,

    workflows: Arc<DashMap<Uuid, WorkflowDefinition>>,

    workflow_runs: Arc<DashMap<Uuid, WorkflowRunExecution>>,

    gitlab_subs: Arc<DashMap<Uuid, GitLabSubscription>>,

    vk_subs: Arc<DashMap<Uuid, VkSubscription>>,

    reddit_subs: Arc<DashMap<Uuid, RedditSubscription>>,

    gaming_hub_statuses: Arc<DashMap<(String, String), GamingHubGameStatus>>,

    social_drafts: Arc<DashMap<Uuid, SocialPublicationDraft>>,

    ai_budgets: Arc<DashMap<String, AiGuildBudget>>,

    ai_documents: Arc<DashMap<Uuid, AiKnowledgeDocument>>,

    mcp_tool_auths: Arc<DashMap<(String, String), McpToolAuthorization>>,

    marketplace_packages: Arc<DashMap<String, PluginMarketplacePackage>>,

    plugin_guild_installs: Arc<DashMap<(String, String), PluginGuildInstallation>>,

    asset_usage_daily: Arc<
        DashMap<
            (String, chrono::NaiveDate, String, String),
            crate::asset_analytics::AssetUsageDaily,
        >,
    >,

    asset_inventory:
        Arc<DashMap<(String, String, String), crate::asset_analytics::AssetInventoryItem>>,

    asset_analytics_failure_injected: Arc<std::sync::atomic::AtomicBool>,
}

impl MemoryStore {
    pub fn get_module_settings(
        &self,

        guild_id: &str,

        module_id: &str,
    ) -> Option<GuildModuleSettings> {
        self.settings
            .get(&(guild_id.to_owned(), module_id.to_owned()))
            .map(|entry| entry.value().clone())
    }

    pub fn set_module_settings(
        &self,

        guild_id: &str,

        module_id: &str,

        enabled: bool,

        config: Value,
    ) -> GuildModuleSettings {
        let value = GuildModuleSettings {
            guild_id: guild_id.to_owned(),

            module_id: module_id.to_owned(),

            enabled,

            config,

            updated_at: Utc::now(),
        };

        self.settings
            .insert((guild_id.to_owned(), module_id.to_owned()), value.clone());

        value
    }

    pub fn inject_audit_failure(&self, fail: bool) {
        self.audit_failure_injected
            .store(fail, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn inject_ping_failure(&self, fail: bool) {
        self.ping_failure_injected
            .store(fail, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn inject_progression_failure(&self, fail: bool) {
        self.progression_failure_injected
            .store(fail, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn is_progression_failure_injected(&self) -> bool {
        self.progression_failure_injected
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn inject_asset_analytics_failure(&self, fail: bool) {
        self.asset_analytics_failure_injected
            .store(fail, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn record_audit(
        &self,

        guild_id: Option<&str>,

        actor_id: Option<&str>,

        module_id: &str,

        event: &str,

        data: Value,
    ) -> anyhow::Result<()> {
        use std::sync::atomic::Ordering;

        if self.audit_failure_injected.load(Ordering::SeqCst) {
            anyhow::bail!("simulated audit failure");
        }

        let id = self.next_audit_id.fetch_add(1, Ordering::Relaxed);

        self.audit_log.insert(
            id,
            serde_json::json!({

                "id": id as i64,

                "guild_id": guild_id,

                "actor_id": actor_id,

                "module_id": module_id,

                "event": event,

                "data": data,

                "created_at": Utc::now(),

            }),
        );

        Ok(())
    }

    pub fn list_audit_events(&self, guild_id: &str, limit: i64) -> Vec<AuditLogEntry> {
        let cutoff = Utc::now() - Duration::days(45);
        let mut entries: Vec<AuditLogEntry> = self
            .audit_log
            .iter()
            .filter_map(|entry| {
                let val = entry.value();

                let gid = val.get("guild_id").and_then(|v| v.as_str())?;

                if gid != guild_id {
                    return None;
                }

                let parsed: AuditLogEntry = serde_json::from_value(val.clone()).ok()?;
                if parsed.created_at < cutoff {
                    return None;
                }

                Some(parsed)
            })
            .collect();

        entries.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.id.cmp(&a.id))
        });

        if limit > 0 && entries.len() > limit as usize {
            entries.truncate(limit as usize);
        }

        entries
    }

    pub fn list_audit_events_page(
        &self,
        guild_id: &str,
        limit: usize,
        offset: usize,
    ) -> Vec<AuditLogEntry> {
        let cutoff = Utc::now() - Duration::days(45);
        let mut entries: Vec<AuditLogEntry> = self
            .audit_log
            .iter()
            .filter_map(|entry| {
                let val = entry.value();

                let gid = val.get("guild_id").and_then(|v| v.as_str())?;

                if gid != guild_id {
                    return None;
                }

                let parsed: AuditLogEntry = serde_json::from_value(val.clone()).ok()?;
                if parsed.created_at < cutoff {
                    return None;
                }

                Some(parsed)
            })
            .collect();

        entries.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.id.cmp(&a.id))
        });

        entries.into_iter().skip(offset).take(limit).collect()
    }

    pub fn count_audit_events(&self, guild_id: &str) -> u64 {
        let cutoff = Utc::now() - Duration::days(45);
        self.audit_log
            .iter()
            .filter(|entry| {
                let val = entry.value();
                let Some(gid) = val.get("guild_id").and_then(|v| v.as_str()) else {
                    return false;
                };
                if gid != guild_id {
                    return false;
                }
                if let Some(created_at_val) = val.get("created_at")
                    && let Ok(created_at) =
                        serde_json::from_value::<DateTime<Utc>>(created_at_val.clone())
                {
                    return created_at >= cutoff;
                }
                false
            })
            .count() as u64
    }

    pub fn prune_audit_events(&self, cutoff: DateTime<Utc>, limit: usize) -> u64 {
        let limit = limit.clamp(1, 5000);
        let mut expired_keys = Vec::new();
        for entry in self.audit_log.iter() {
            let val = entry.value();
            if let Some(created_at_val) = val.get("created_at")
                && let Ok(created_at) =
                    serde_json::from_value::<DateTime<Utc>>(created_at_val.clone())
                && created_at < cutoff
            {
                expired_keys.push(*entry.key());
                if expired_keys.len() >= limit {
                    break;
                }
            }
        }

        let mut deleted = 0u64;
        for key in expired_keys {
            if self.audit_log.remove(&key).is_some() {
                deleted += 1;
            }
        }
        deleted
    }

    pub fn update_module_settings_with_audit(
        &self,

        params: super::UpdateModuleSettingsParams<'_>,
    ) -> anyhow::Result<GuildModuleSettings> {
        let _guard = self
            .settings_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        let key = (params.guild_id.to_owned(), params.module_id.to_owned());

        let previous = self.settings.get(&key).map(|entry| entry.value().clone());

        let new_settings = GuildModuleSettings {
            guild_id: params.guild_id.to_owned(),

            module_id: params.module_id.to_owned(),

            enabled: params.enabled,

            config: params.config,

            updated_at: Utc::now(),
        };

        self.settings.insert(key.clone(), new_settings.clone());

        if let Err(audit_err) = self.record_audit(
            Some(params.guild_id),
            params.actor_id,
            "dashboard",
            params.audit_event,
            params.audit_data,
        ) {
            match previous {
                Some(prev) => {
                    self.settings.insert(key, prev);
                }

                None => {
                    self.settings.remove(&key);
                }
            }

            return Err(audit_err);
        }

        Ok(new_settings)
    }

    pub fn get_member_progress(&self, guild_id: &str, user_id: &str) -> MemberProgress {
        let _guard = self.progression_guard();

        self.profile(guild_id, user_id, Utc::now())
    }

    pub fn has_member_progress(&self, guild_id: &str, user_id: &str) -> anyhow::Result<bool> {
        if self
            .progression_failure_injected
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            anyhow::bail!("simulated progression failure");
        }
        let _guard = self.progression_guard();
        let key = (guild_id.to_owned(), user_id.to_owned());
        Ok(self.progression.contains_key(&key))
    }

    pub fn erase_guild_user_data(&self, guild_id: &str, user_id: &str) -> UserErasureReport {
        let _guard = self.progression_guard();
        let key = (guild_id.to_owned(), user_id.to_owned());
        let progression_erased = self.progression.remove(&key).is_some();

        let mut related_erased = 0;
        if self.persistent_roles.remove(&key).is_some() {
            related_erased += 1;
        }

        let before_roles = self.role_rewards.len();
        self.role_rewards
            .retain(|(g, u, _)| !(g == guild_id && u == user_id));
        related_erased += before_roles.saturating_sub(self.role_rewards.len());

        let before_rep = self.reputation_cooldowns.len();
        self.reputation_cooldowns
            .retain(|(g, from), _| !(g == guild_id && from == user_id));
        related_erased += before_rep.saturating_sub(self.reputation_cooldowns.len());

        let before_identities = self.identity_history.len();
        self.identity_history
            .retain(|_, r| !(r.guild_id == guild_id && r.user_id == user_id));
        related_erased += before_identities.saturating_sub(self.identity_history.len());

        if self
            .member_snapshots
            .remove(&(guild_id.to_string(), user_id.to_string()))
            .is_some()
        {
            related_erased += 1;
        }

        let before_plans = self.cleanup_plans.len();
        self.cleanup_plans
            .retain(|_, p| !(p.guild_id == guild_id && p.actor_user_id == user_id));
        related_erased += before_plans.saturating_sub(self.cleanup_plans.len());

        let before_leases = self.companion_leases.len();
        self.companion_leases
            .retain(|(g, _, u), _| !(g == guild_id && u == user_id));
        related_erased += before_leases.saturating_sub(self.companion_leases.len());

        let before_policy = self.policy_rules.len();
        self.policy_rules.retain(|_, r| {
            !(r.guild_id == guild_id
                && r.subject_kind == "user"
                && r.subject_id.as_deref() == Some(user_id))
        });
        related_erased += before_policy.saturating_sub(self.policy_rules.len());

        let before_duty = self.duty_profiles.len();
        self.duty_profiles
            .retain(|(g, u), _| !(g == guild_id && u == user_id));
        related_erased += before_duty.saturating_sub(self.duty_profiles.len());

        self.duty_states
            .retain(|(g, u), _| !(g == guild_id && u == user_id));

        self.duty_role_grants
            .retain(|(g, u, _), _| !(g == guild_id && u == user_id));

        let before_rec_apps = self.recruitment_applications.len();
        self.recruitment_applications
            .retain(|_, a| !(a.guild_id == guild_id && a.applicant_user_id == user_id));
        related_erased += before_rec_apps.saturating_sub(self.recruitment_applications.len());

        let before_rec_revs = self.recruitment_reviews.len();
        self.recruitment_reviews
            .retain(|_, r| !(r.guild_id == guild_id && r.reviewer_user_id == user_id));
        related_erased += before_rec_revs.saturating_sub(self.recruitment_reviews.len());

        let before_forum_sessions = self.help_forum_sessions.len();
        self.help_forum_sessions
            .retain(|(g, _), s| !(g == guild_id && s.owner_user_id == user_id));
        related_erased += before_forum_sessions.saturating_sub(self.help_forum_sessions.len());

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

        UserErasureReport {
            user_id: user_id.to_string(),
            guild_id: guild_id.to_string(),
            progression_erased,
            related_records_erased: related_erased,
            retained_records,
            erased_at: Utc::now(),
        }
    }

    pub fn award_message_xp(
        &self,

        guild_id: &str,

        user_id: &str,

        amount: i64,

        cooldown_seconds: u32,
    ) -> XpAward {
        let _guard = self.progression_guard();

        let now = Utc::now();

        let mut profile = self.profile(guild_id, user_id, now);

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

        self.store_profile(profile.clone());

        XpAward {
            profile,

            awarded,

            previous_level,
        }
    }

    pub fn claim_daily(
        &self,

        guild_id: &str,

        user_id: &str,

        base_reward: i64,

        streak_bonus: i64,

        max_streak_bonus: i32,
    ) -> DailyClaimOutcome {
        let _guard = self.progression_guard();

        let now = Utc::now();

        let mut profile = self.profile(guild_id, user_id, now);

        if let Some(last_claim) = profile.last_daily_at {
            let next_claim_at = last_claim + Duration::hours(24);

            if now < next_claim_at {
                return DailyClaimOutcome::Cooldown {
                    profile,

                    next_claim_at,
                };
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

        self.store_profile(profile.clone());

        DailyClaimOutcome::Claimed {
            profile,

            reward,

            next_claim_at: now + Duration::hours(24),
        }
    }

    pub fn transfer_coins(
        &self,

        guild_id: &str,

        sender_user_id: &str,

        recipient_user_id: &str,

        amount: i64,
    ) -> anyhow::Result<CoinTransferOutcome> {
        if sender_user_id == recipient_user_id {
            return Ok(CoinTransferOutcome::SameAccount);
        }

        let _guard = self.progression_guard();

        let now = Utc::now();

        let mut sender = self.profile(guild_id, sender_user_id, now);

        if amount <= 0 || sender.coins < amount {
            return Ok(CoinTransferOutcome::InsufficientFunds {
                balance: sender.coins,
            });
        }

        let mut recipient = self.profile(guild_id, recipient_user_id, now);

        // Attempt audit recording first inside atomic lock

        self.record_audit(
            Some(guild_id),
            Some(sender_user_id),
            "progression",
            "coins_transferred",
            serde_json::json!({

                "recipient_user_id": recipient_user_id,

                "amount": amount,

                "sender_balance": sender.coins - amount,

                "recipient_balance": recipient.coins.saturating_add(amount),

            }),
        )?;

        // Only commit profiles after audit write succeeds

        sender.coins -= amount;

        recipient.coins = recipient.coins.saturating_add(amount);

        sender.updated_at = now;

        recipient.updated_at = now;

        self.store_profile(sender.clone());

        self.store_profile(recipient.clone());

        Ok(CoinTransferOutcome::Completed(Box::new(CoinTransfer {
            sender,

            recipient,

            amount,
        })))
    }

    pub fn give_reputation(
        &self,

        guild_id: &str,

        giver_user_id: &str,

        target_user_id: &str,

        amount: i64,

        cooldown_seconds: u32,
    ) -> ReputationGrantOutcome {
        if giver_user_id == target_user_id {
            return ReputationGrantOutcome::SameAccount;
        }

        let _guard = self.progression_guard();

        let now = Utc::now();

        let cooldown_key = (guild_id.to_owned(), giver_user_id.to_owned());

        if let Some(last_given_at) = self.reputation_cooldowns.get(&cooldown_key) {
            let next_available_at = *last_given_at + Duration::seconds(i64::from(cooldown_seconds));

            if now < next_available_at {
                return ReputationGrantOutcome::Cooldown { next_available_at };
            }
        }

        let mut profile = self.profile(guild_id, target_user_id, now);

        let amount = amount.max(0);

        profile.reputation = profile.reputation.saturating_add(amount);

        profile.updated_at = now;

        self.store_profile(profile.clone());

        self.reputation_cooldowns.insert(cooldown_key, now);

        ReputationGrantOutcome::Granted {
            profile,

            amount,

            next_available_at: now + Duration::seconds(i64::from(cooldown_seconds)),
        }
    }

    pub fn adjust_member_progress(
        &self,

        guild_id: &str,

        user_id: &str,

        xp_delta: i64,

        coins_delta: i64,

        reputation_delta: i64,
    ) -> MemberProgress {
        let _guard = self.progression_guard();

        let now = Utc::now();

        let mut profile = self.profile(guild_id, user_id, now);

        profile.xp = profile.xp.saturating_add(xp_delta).max(0);

        profile.coins = profile.coins.saturating_add(coins_delta).max(0);

        profile.reputation = profile.reputation.saturating_add(reputation_delta).max(0);

        profile.level = level_for_xp(profile.xp);

        profile.updated_at = now;

        self.store_profile(profile.clone());

        profile
    }

    pub fn adjust_member_progress_with_audit(
        &self,

        params: AdjustMemberProgressParams<'_>,
    ) -> anyhow::Result<MemberProgress> {
        let _guard = self.progression_guard();

        let now = Utc::now();

        let mut profile = self.profile(params.guild_id, params.user_id, now);

        profile.xp = profile.xp.saturating_add(params.xp_delta).max(0);

        profile.coins = profile.coins.saturating_add(params.coins_delta).max(0);

        profile.reputation = profile
            .reputation
            .saturating_add(params.reputation_delta)
            .max(0);

        profile.level = level_for_xp(profile.xp);

        profile.updated_at = now;

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

        // Record audit first inside guard before saving profile to ensure atomic commit

        self.record_audit(
            Some(params.guild_id),
            params.actor_id,
            "progression",
            "progress_adjusted",
            audit_data,
        )?;

        self.store_profile(profile.clone());

        Ok(profile)
    }

    pub fn progression_leaderboard(
        &self,

        guild_id: &str,

        metric: ProgressMetric,

        limit: u8,
    ) -> Vec<MemberProgress> {
        let _guard = self.progression_guard();

        let mut profiles = self
            .progression
            .iter()
            .filter(|entry| entry.value().guild_id == guild_id)
            .map(|entry| entry.value().clone())
            .filter(|profile| metric.value(profile) > 0)
            .collect::<Vec<_>>();

        profiles.sort_by(|left, right| {
            metric
                .value(right)
                .cmp(&metric.value(left))
                .then_with(|| left.user_id.cmp(&right.user_id))
        });

        profiles.truncate(usize::from(limit));

        profiles
    }

    pub fn is_role_reward_granted(&self, guild_id: &str, user_id: &str, role_id: &str) -> bool {
        self.role_rewards
            .contains(&(guild_id.to_owned(), user_id.to_owned(), role_id.to_owned()))
    }

    pub fn record_role_reward_granted(
        &self,

        guild_id: &str,

        user_id: &str,

        role_id: &str,

        _level: i64,
    ) {
        self.role_rewards
            .insert((guild_id.to_owned(), user_id.to_owned(), role_id.to_owned()));
    }

    pub fn record_moderation_intent(&self, intent: ModerationExecutionIntent) {
        self.moderation_intents.insert(intent.id, intent);
    }

    pub fn update_moderation_intent_step(&self, id: Uuid, step: &str, error: Option<&str>) {
        if let Some(mut entry) = self.moderation_intents.get_mut(&id) {
            entry.step = step.to_owned();

            entry.error = error.map(str::to_owned);

            entry.updated_at = Utc::now();
        }
    }

    pub fn list_pending_moderation_intents(
        &self,

        kind: &str,

        step: &str,
    ) -> Vec<ModerationExecutionIntent> {
        let mut results: Vec<ModerationExecutionIntent> = self
            .moderation_intents
            .iter()
            .filter(|entry| entry.kind == kind && entry.step == step)
            .map(|entry| entry.value().clone())
            .collect();

        results.sort_by_key(|i| i.created_at);

        results
    }

    fn progression_guard(&self) -> MutexGuard<'_, ()> {
        self.progression_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn profile(&self, guild_id: &str, user_id: &str, now: chrono::DateTime<Utc>) -> MemberProgress {
        let key = (guild_id.to_owned(), user_id.to_owned());

        self.progression.get(&key).map_or_else(
            || {
                let profile = MemberProgress::empty(guild_id, user_id, now);

                self.progression.insert(key, profile.clone());

                profile
            },
            |entry| entry.value().clone(),
        )
    }

    fn store_profile(&self, profile: MemberProgress) {
        self.progression
            .insert((profile.guild_id.clone(), profile.user_id.clone()), profile);
    }

    pub fn get_module_draft(&self, guild_id: &str, module_id: &str) -> Option<ModuleDraftConfig> {
        self.module_drafts
            .get(&(guild_id.to_owned(), module_id.to_owned()))
            .map(|entry| entry.value().clone())
    }

    pub fn set_module_draft(
        &self,

        guild_id: &str,

        module_id: &str,

        config: Value,

        updated_by: &str,
    ) -> ModuleDraftConfig {
        let draft = ModuleDraftConfig {
            guild_id: guild_id.to_owned(),

            module_id: module_id.to_owned(),

            config_json: config,

            updated_by: updated_by.to_owned(),

            updated_at: Utc::now(),
        };

        self.module_drafts
            .insert((guild_id.to_owned(), module_id.to_owned()), draft.clone());

        draft
    }

    pub fn delete_module_draft(&self, guild_id: &str, module_id: &str) -> bool {
        self.module_drafts
            .remove(&(guild_id.to_owned(), module_id.to_owned()))
            .is_some()
    }

    pub fn create_appeal(&self, appeal: ModerationAppeal) {
        self.appeals.insert(appeal.id, appeal);
    }

    pub fn list_appeals(&self, guild_id: &str, status: Option<&str>) -> Vec<ModerationAppeal> {
        let mut results: Vec<ModerationAppeal> = self
            .appeals
            .iter()
            .filter(|e| e.guild_id == guild_id && status.is_none_or(|s| e.status == s))
            .map(|e| e.value().clone())
            .collect();

        results.sort_by_key(|a| std::cmp::Reverse(a.created_at));

        results
    }

    pub fn get_appeal(&self, appeal_id: Uuid) -> Option<ModerationAppeal> {
        self.appeals.get(&appeal_id).map(|e| e.value().clone())
    }

    pub fn update_appeal_status(
        &self,
        guild_id: &str,
        appeal_id: Uuid,
        status: &str,
        staff_notes: Option<&str>,
    ) -> bool {
        if let Some(mut entry) = self.appeals.get_mut(&appeal_id) {
            let appeal = entry.value_mut();
            if appeal.guild_id != guild_id {
                return false;
            }

            appeal.status = status.to_owned();

            if let Some(notes) = staff_notes {
                appeal.staff_notes = Some(notes.to_owned());
            }

            appeal.updated_at = Utc::now();

            true
        } else {
            false
        }
    }

    pub fn grant_timed_role(
        &self,

        guild_id: &str,

        user_id: &str,

        role_id: &str,

        expires_at: chrono::DateTime<Utc>,
    ) -> Uuid {
        let id = Uuid::new_v4();

        self.timed_roles.insert(
            id,
            TimedRoleGrant {
                id,

                guild_id: guild_id.to_owned(),

                user_id: user_id.to_owned(),

                role_id: role_id.to_owned(),

                expires_at,

                created_at: Utc::now(),
            },
        );

        id
    }

    pub fn get_active_timed_roles(&self, guild_id: &str, user_id: &str) -> Vec<TimedRoleGrant> {
        let now = Utc::now();

        self.timed_roles
            .iter()
            .filter(|e| e.guild_id == guild_id && e.user_id == user_id && e.expires_at > now)
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn save_persistent_roles(&self, guild_id: &str, user_id: &str, role_ids: &[String]) {
        self.persistent_roles
            .insert((guild_id.to_owned(), user_id.to_owned()), role_ids.to_vec());
    }

    pub fn get_persistent_roles(&self, guild_id: &str, user_id: &str) -> Vec<String> {
        self.persistent_roles
            .get(&(guild_id.to_owned(), user_id.to_owned()))
            .map(|e| e.value().clone())
            .unwrap_or_default()
    }

    pub fn create_server_snapshot(
        &self,

        guild_id: &str,

        name: &str,

        snapshot_json: Value,

        created_by: &str,
    ) -> ServerSnapshot {
        let id = Uuid::new_v4();

        let snapshot = ServerSnapshot {
            id,

            guild_id: guild_id.to_owned(),

            name: name.to_owned(),

            snapshot_json,

            created_by: created_by.to_owned(),

            created_at: Utc::now(),
        };

        self.server_snapshots.insert(id, snapshot.clone());

        snapshot
    }

    pub fn list_server_snapshots(&self, guild_id: &str) -> Vec<ServerSnapshot> {
        let mut list: Vec<_> = self
            .server_snapshots
            .iter()
            .filter(|e| e.guild_id == guild_id)
            .map(|e| e.value().clone())
            .collect();

        list.sort_by_key(|e| std::cmp::Reverse(e.created_at));

        list
    }

    pub fn get_server_snapshot(&self, snapshot_id: Uuid) -> Option<ServerSnapshot> {
        self.server_snapshots
            .get(&snapshot_id)
            .map(|e| e.value().clone())
    }

    pub fn delete_server_snapshot(&self, guild_id: &str, snapshot_id: Uuid) -> bool {
        if let Some(entry) = self.server_snapshots.get(&snapshot_id)
            && entry.guild_id == guild_id
        {
            drop(entry);

            return self.server_snapshots.remove(&snapshot_id).is_some();
        }

        false
    }

    pub fn create_custom_automation(
        &self,

        guild_id: &str,

        name: &str,

        trigger_type: &str,

        conditions_json: Value,

        actions_json: Value,
    ) -> CustomAutomation {
        let id = Uuid::new_v4();

        let now = Utc::now();

        let automation = CustomAutomation {
            id,

            guild_id: guild_id.to_owned(),

            name: name.to_owned(),

            trigger_type: trigger_type.to_owned(),

            conditions_json,

            actions_json,

            enabled: true,

            created_at: now,

            updated_at: now,
        };

        self.custom_automations.insert(id, automation.clone());

        automation
    }

    pub fn list_custom_automations(&self, guild_id: &str) -> Vec<CustomAutomation> {
        self.custom_automations
            .iter()
            .filter(|e| e.guild_id == guild_id)
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn delete_custom_automation(&self, guild_id: &str, automation_id: Uuid) -> bool {
        if let Some(entry) = self.custom_automations.get(&automation_id)
            && entry.guild_id == guild_id
        {
            drop(entry);

            return self.custom_automations.remove(&automation_id).is_some();
        }

        false
    }

    pub fn get_sticky_message(&self, guild_id: &str, channel_id: &str) -> Option<StickyMessage> {
        self.sticky_messages
            .get(&(guild_id.to_owned(), channel_id.to_owned()))
            .map(|e| e.value().clone())
    }

    pub fn set_sticky_message(
        &self,

        guild_id: &str,

        channel_id: &str,

        message_id: &str,

        content: &str,

        cooldown_seconds: i32,
    ) -> StickyMessage {
        let entry = StickyMessage {
            guild_id: guild_id.to_owned(),

            channel_id: channel_id.to_owned(),

            message_id: message_id.to_owned(),

            content: content.to_owned(),

            cooldown_seconds,

            last_updated_at: Utc::now(),
        };

        self.sticky_messages
            .insert((guild_id.to_owned(), channel_id.to_owned()), entry.clone());

        entry
    }

    pub fn delete_sticky_message(&self, guild_id: &str, channel_id: &str) -> bool {
        self.sticky_messages
            .remove(&(guild_id.to_owned(), channel_id.to_owned()))
            .is_some()
    }

    pub fn create_community_poll(
        &self,

        guild_id: &str,

        channel_id: &str,

        question: &str,

        options: &[String],

        is_anonymous: bool,

        expires_at: chrono::DateTime<Utc>,
    ) -> CommunityPoll {
        let id = Uuid::new_v4();

        let tallies = vec![0i64; options.len()];

        let poll = CommunityPoll {
            id,

            guild_id: guild_id.to_owned(),

            channel_id: channel_id.to_owned(),

            question: question.to_owned(),

            options_json: serde_json::json!(options),

            tallies_json: serde_json::json!(tallies),

            is_anonymous,

            expires_at,

            created_at: Utc::now(),
        };

        self.polls.insert(id, poll.clone());

        poll
    }

    pub fn get_community_poll(&self, poll_id: Uuid) -> Option<CommunityPoll> {
        self.polls.get(&poll_id).map(|e| e.value().clone())
    }

    pub fn vote_community_poll(
        &self,

        guild_id: &str,

        poll_id: Uuid,

        user_id: &str,

        option_idx: usize,
    ) -> anyhow::Result<Option<CommunityPoll>> {
        let mut poll = match self.polls.get_mut(&poll_id) {
            Some(p) => p,

            None => return Ok(None),
        };

        if poll.guild_id != guild_id {
            return Ok(None);
        }

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

        let key = (poll_id, user_id.to_string());

        if let Some(prev) = self.poll_votes.get(&key) {
            let prev_idx = *prev;

            if prev_idx == option_idx {
                return Ok(Some(poll.clone()));
            }

            if let Some(val) = tallies.get_mut(prev_idx) {
                let count = val.as_i64().unwrap_or(0);

                *val = serde_json::json!((count - 1).max(0));
            }
        }

        if let Some(val) = tallies.get_mut(option_idx) {
            let count = val.as_i64().unwrap_or(0);

            *val = serde_json::json!(count + 1);
        }

        self.poll_votes.insert(key, option_idx);

        Ok(Some(poll.clone()))
    }

    pub fn create_giveaway(
        &self,

        guild_id: &str,

        channel_id: &str,

        prize: &str,

        winners_count: i32,

        expires_at: chrono::DateTime<Utc>,
    ) -> Giveaway {
        let id = Uuid::new_v4();

        let giveaway = Giveaway {
            id,

            guild_id: guild_id.to_owned(),

            channel_id: channel_id.to_owned(),

            prize: prize.to_owned(),

            winners_count,

            participants_json: serde_json::json!([]),

            winners_json: serde_json::json!([]),

            ended: false,

            expires_at,

            created_at: Utc::now(),
        };

        self.giveaways.insert(id, giveaway.clone());

        giveaway
    }

    pub fn get_giveaway(&self, giveaway_id: Uuid) -> Option<Giveaway> {
        self.giveaways.get(&giveaway_id).map(|e| e.value().clone())
    }

    pub fn join_giveaway(&self, giveaway_id: Uuid, user_id: &str) -> bool {
        let mut ga = match self.giveaways.get_mut(&giveaway_id) {
            Some(g) => g,

            None => return false,
        };

        if ga.ended || chrono::Utc::now() >= ga.expires_at {
            return false;
        }

        let list = match ga.participants_json.as_array_mut() {
            Some(l) => l,

            None => return false,
        };

        let user_val = serde_json::json!(user_id);

        if !list.contains(&user_val) {
            list.push(user_val);

            true
        } else {
            false
        }
    }

    pub fn reroll_giveaway(&self, giveaway_id: Uuid) -> Vec<String> {
        let mut ga = match self.giveaways.get_mut(&giveaway_id) {
            Some(g) => g,

            None => return Vec::new(),
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
            return Vec::new();
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

        ga.ended = true;

        winners
    }

    pub fn record_starboard_entry(
        &self,

        guild_id: &str,

        source_channel_id: &str,

        source_message_id: &str,

        starboard_message_id: &str,

        reaction_count: i32,
    ) -> StarboardEntry {
        let entry = StarboardEntry {
            guild_id: guild_id.to_owned(),

            source_channel_id: source_channel_id.to_owned(),

            source_message_id: source_message_id.to_owned(),

            starboard_message_id: starboard_message_id.to_owned(),

            reaction_count,

            created_at: Utc::now(),
        };

        self.starboard_entries.insert(
            (guild_id.to_owned(), source_message_id.to_owned()),
            entry.clone(),
        );

        entry
    }

    pub fn get_starboard_entry(
        &self,

        guild_id: &str,

        source_message_id: &str,
    ) -> Option<StarboardEntry> {
        self.starboard_entries
            .get(&(guild_id.to_owned(), source_message_id.to_owned()))
            .map(|e| e.value().clone())
    }

    pub fn list_store_items(&self, guild_id: &str) -> Vec<StoreItem> {
        self.store_items
            .iter()
            .filter(|e| e.guild_id == guild_id)
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn create_store_item(
        &self,

        guild_id: &str,

        name: &str,

        description: &str,

        price: i64,

        role_id: Option<&str>,

        stock: Option<i32>,
    ) -> StoreItem {
        let id = Uuid::new_v4();

        let item = StoreItem {
            id,

            guild_id: guild_id.to_owned(),

            name: name.to_owned(),

            description: description.to_owned(),

            price,

            role_id: role_id.map(|s| s.to_owned()),

            stock,

            created_at: Utc::now(),
        };

        self.store_items.insert(id, item.clone());

        item
    }

    pub fn purchase_store_item(
        &self,

        guild_id: &str,

        user_id: &str,

        item_id: Uuid,
    ) -> anyhow::Result<(StoreItem, StorePurchase, i64)> {
        let _guard = self.progression_guard();

        let mut item = self
            .store_items
            .get_mut(&item_id)
            .ok_or_else(|| anyhow::anyhow!("store item not found"))?;

        if item.guild_id != guild_id {
            return Err(anyhow::anyhow!("item does not belong to this guild"));
        }

        if let Some(stock) = item.stock
            && stock <= 0
        {
            return Err(anyhow::anyhow!("item is out of stock"));
        }

        let price = item.price;
        anyhow::ensure!(price >= 0, "stored store item has invalid negative price");
        if let Some(ref r_id) = item.role_id {
            validate_snowflake("role_id", r_id)?;
        }

        let now = Utc::now();

        let mut profile = self.profile(guild_id, user_id, now);

        if profile.coins < price {
            return Err(anyhow::anyhow!("insufficient coins balance"));
        }

        // Decrement stock only after verifying coins balance

        if let Some(stock) = item.stock {
            item.stock = Some(stock - 1);
        }

        let cloned_item = item.clone();

        drop(item);

        profile.coins -= price;

        profile.updated_at = now;

        let remaining = profile.coins;

        self.store_profile(profile);

        let initial_status = if cloned_item.role_id.is_some() {
            "pending".to_string()
        } else {
            "completed".to_string()
        };

        let purchase = StorePurchase {
            id: Uuid::new_v4(),

            guild_id: guild_id.to_owned(),

            user_id: user_id.to_owned(),

            item_id,

            price_paid: price,

            role_id: cloned_item.role_id.clone(),

            fulfillment_status: initial_status,

            failure_reason: None,

            fulfilled_at: if cloned_item.role_id.is_none() {
                Some(now)
            } else {
                None
            },

            created_at: now,
        };

        self.store_purchases.insert(purchase.id, purchase.clone());

        Ok((cloned_item, purchase, remaining))
    }

    pub fn get_store_purchase(&self, purchase_id: Uuid) -> Option<StorePurchase> {
        self.store_purchases.get(&purchase_id).map(|p| p.clone())
    }

    pub fn list_store_purchases(
        &self,
        guild_id: &str,
        user_id: Option<&str>,
    ) -> Vec<StorePurchase> {
        let mut list: Vec<StorePurchase> = self
            .store_purchases
            .iter()
            .filter(|p| p.guild_id == guild_id && user_id.is_none_or(|uid| p.user_id == uid))
            .map(|p| p.clone())
            .collect();
        list.sort_by_key(|a| std::cmp::Reverse(a.created_at));
        list
    }

    pub fn update_store_purchase_fulfillment(
        &self,
        purchase_id: Uuid,
        status: &str,
        failure_reason: Option<&str>,
        fulfilled_at: Option<DateTime<Utc>>,
    ) -> anyhow::Result<()> {
        let mut purchase = self
            .store_purchases
            .get_mut(&purchase_id)
            .ok_or_else(|| anyhow::anyhow!("store purchase not found"))?;
        purchase.fulfillment_status = status.to_string();
        purchase.failure_reason = failure_reason.map(|s| s.to_string());
        purchase.fulfilled_at = fulfilled_at;
        Ok(())
    }

    pub fn record_member_identity_change(
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

        let record = MemberIdentityRecord {
            id: Uuid::new_v4(),
            guild_id: guild_id.to_string(),
            user_id: user_id.to_string(),
            kind: kind.to_string(),
            old_value: old_clean.map(|s| s.to_string()),
            new_value: new_clean.map(|s| s.to_string()),
            observed_at: now,
            expires_at,
        };

        self.identity_history.insert(record.id, record.clone());
        Ok(Some(record))
    }

    pub fn get_member_identity_history(
        &self,
        guild_id: &str,
        user_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<MemberIdentityRecord>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("user_id", user_id)?;

        let now = Utc::now();
        let max_limit = limit.clamp(1, 100);

        let mut list: Vec<MemberIdentityRecord> = self
            .identity_history
            .iter()
            .filter(|r| r.guild_id == guild_id && r.user_id == user_id && r.expires_at > now)
            .map(|r| r.clone())
            .collect();

        list.sort_by_key(|r| std::cmp::Reverse(r.observed_at));
        if list.len() > max_limit {
            list.truncate(max_limit);
        }
        Ok(list)
    }

    pub fn cleanup_expired_identity_history(&self) -> u64 {
        let now = Utc::now();
        let before = self.identity_history.len();
        self.identity_history.retain(|_, r| r.expires_at > now);
        before.saturating_sub(self.identity_history.len()) as u64
    }

    pub fn insert_raw_identity_record(&self, record: MemberIdentityRecord) {
        self.identity_history.insert(record.id, record);
    }

    pub fn raw_identity_history_count(&self) -> usize {
        self.identity_history.len()
    }

    pub fn set_member_case_count(&self, guild_id: &str, user_id: &str, count: i64) {
        self.member_case_counts
            .insert((guild_id.to_string(), user_id.to_string()), count);
    }

    pub fn upsert_member_snapshot(
        &self,
        params: UpsertMemberSnapshotParams<'_>,
    ) -> anyhow::Result<MemberSnapshot> {
        let u_num = validate_snowflake("user_id", params.user_id)?;
        validate_snowflake("guild_id", params.guild_id)?;

        let account_created_at = snowflake_created_at(u_num);
        let now = Utc::now();

        let snapshot = MemberSnapshot {
            guild_id: params.guild_id.to_string(),
            user_id: params.user_id.to_string(),
            username: params.username.to_string(),
            global_name: params.global_name.map(|s| s.to_string()),
            nickname: params.nickname.map(|s| s.to_string()),
            role_ids: serde_json::to_value(params.role_ids)
                .unwrap_or_else(|_| serde_json::json!([])),
            joined_at: params.joined_at,
            account_created_at: Some(account_created_at),
            updated_at: now,
        };

        self.member_snapshots.insert(
            (params.guild_id.to_string(), params.user_id.to_string()),
            snapshot.clone(),
        );
        Ok(snapshot)
    }

    pub fn remove_member_snapshot(&self, guild_id: &str, user_id: &str) -> anyhow::Result<bool> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("user_id", user_id)?;
        let key = (guild_id.to_string(), user_id.to_string());
        Ok(self.member_snapshots.remove(&key).is_some())
    }

    pub fn search_members(
        &self,
        guild_id: &str,
        filter: &MemberSearchFilter,
    ) -> anyhow::Result<Vec<MemberSearchRow>> {
        validate_snowflake("guild_id", guild_id)?;

        let limit = filter.limit.unwrap_or(25).clamp(1, 100) as usize;
        let now = Utc::now();

        // Keyset cursor extraction with legacy user_id fallback
        let (cursor_joined_at, cursor_user_id) = if let Some(ref cur) = filter.cursor {
            if let Some((parsed_joined, parsed_user)) = parse_member_search_cursor(cur) {
                if parsed_joined.is_none() && cur.chars().all(|c| c.is_ascii_digit()) {
                    let key = (guild_id.to_string(), parsed_user.clone());
                    let resolved_joined = self.member_snapshots.get(&key).and_then(|s| s.joined_at);
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

        // Pre-aggregate active history for this guild in a single O(H) pass
        let mut history_stats: HashMap<String, (i64, Option<DateTime<Utc>>)> = HashMap::new();
        for r in self.identity_history.iter() {
            if r.guild_id == guild_id && r.expires_at > now {
                let entry = history_stats.entry(r.user_id.clone()).or_insert((0, None));
                entry.0 += 1;
                let max_observed = match entry.1 {
                    Some(prev) => prev.max(r.observed_at),
                    None => r.observed_at,
                };
                entry.1 = Some(max_observed);
            }
        }

        let mut matched = Vec::new();

        for entry in self.member_snapshots.iter() {
            let snap = entry.value();
            if snap.guild_id != guild_id {
                continue;
            }

            // Keyset seek filter: skip anything before or at the cursor
            if let Some(ref cur_user) = cursor_user_id
                && !is_member_after_cursor(
                    snap.joined_at,
                    &snap.user_id,
                    cursor_joined_at,
                    cur_user,
                )
            {
                continue;
            }

            let member_roles: Vec<String> =
                serde_json::from_value(snap.role_ids.clone()).unwrap_or_default();

            if !filter.role_ids.is_empty()
                && !filter.role_ids.iter().all(|r| member_roles.contains(r))
            {
                continue;
            }

            if !filter.exclude_role_ids.is_empty()
                && filter
                    .exclude_role_ids
                    .iter()
                    .any(|r| member_roles.contains(r))
            {
                continue;
            }

            if let Some(joined_after) = filter.joined_after {
                match snap.joined_at {
                    Some(j) if j >= joined_after => {}
                    _ => continue,
                }
            }
            if let Some(joined_before) = filter.joined_before {
                match snap.joined_at {
                    Some(j) if j <= joined_before => {}
                    _ => continue,
                }
            }

            if let Some(created_after) = filter.account_created_after {
                match snap.account_created_at {
                    Some(c) if c >= created_after => {}
                    _ => continue,
                }
            }
            if let Some(created_before) = filter.account_created_before {
                match snap.account_created_at {
                    Some(c) if c <= created_before => {}
                    _ => continue,
                }
            }

            let case_count = self
                .member_case_counts
                .get(&(guild_id.to_string(), snap.user_id.clone()))
                .map(|v| *v)
                .unwrap_or(0);

            if let Some(min_cases) = filter.min_case_count
                && case_count < min_cases
            {
                continue;
            }

            let (recent_name_changes, last_change) = history_stats
                .get(&snap.user_id)
                .map(|(cnt, last)| (*cnt, *last))
                .unwrap_or((0, None));

            if let Some(after) = filter.identity_changed_after {
                match last_change {
                    Some(lc) if lc >= after => {}
                    _ => continue,
                }
            }

            let prog_key = (guild_id.to_string(), snap.user_id.clone());
            let (xp, level, message_count) = if let Some(p) = self.progression.get(&prog_key) {
                (Some(p.xp), Some(p.level), Some(p.messages))
            } else {
                (None, None, None)
            };

            matched.push(MemberSearchRow {
                user_id: snap.user_id.clone(),
                username: snap.username.clone(),
                global_name: snap.global_name.clone(),
                nickname: snap.nickname.clone(),
                role_ids: member_roles,
                joined_at: snap.joined_at,
                account_created_at: snap.account_created_at,
                moderation_case_count: case_count,
                xp,
                level,
                message_count,
                recent_name_changes,
            });
        }

        matched.sort_by(|a, b| {
            b.joined_at
                .cmp(&a.joined_at)
                .then_with(|| a.user_id.cmp(&b.user_id))
        });

        matched.truncate(limit);
        Ok(matched)
    }

    pub fn create_cleanup_plan(
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

        let now = Utc::now();
        let plan = CleanupPlan {
            id: Uuid::new_v4(),
            guild_id: guild_id.to_string(),
            channel_id: channel_id.to_string(),
            actor_user_id: actor_user_id.to_string(),
            message_ids: serde_json::to_value(message_ids)
                .unwrap_or_else(|_| serde_json::json!([])),
            created_at: now,
            expires_at: now + chrono::Duration::seconds(ttl_seconds),
            consumed_at: None,
        };
        self.cleanup_plans.insert(plan.id, plan.clone());
        Ok(plan)
    }

    pub fn get_cleanup_plan(
        &self,
        guild_id: &str,
        plan_id: Uuid,
    ) -> anyhow::Result<Option<CleanupPlan>> {
        validate_snowflake("guild_id", guild_id)?;
        if let Some(plan) = self.cleanup_plans.get(&plan_id)
            && plan.guild_id == guild_id
        {
            return Ok(Some(plan.clone()));
        }
        Ok(None)
    }

    pub fn consume_cleanup_plan(
        &self,
        guild_id: &str,
        plan_id: Uuid,
        actor_user_id: &str,
    ) -> anyhow::Result<Option<CleanupPlan>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("actor_user_id", actor_user_id)?;

        let now = Utc::now();
        if let Some(mut plan) = self.cleanup_plans.get_mut(&plan_id)
            && plan.guild_id == guild_id
            && plan.actor_user_id == actor_user_id
            && plan.consumed_at.is_none()
            && plan.expires_at >= now
        {
            plan.consumed_at = Some(now);
            return Ok(Some(plan.clone()));
        }
        Ok(None)
    }

    pub fn list_companion_mappings(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<CompanionChannelMapping>> {
        validate_snowflake("guild_id", guild_id)?;
        let mut list: Vec<_> = self
            .companion_mappings
            .iter()
            .filter(|m| m.guild_id == guild_id)
            .map(|m| m.clone())
            .collect();
        list.sort_by_key(|a| a.created_at);
        Ok(list)
    }

    pub fn upsert_companion_mapping(
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
        };
        let mut results = self.batch_upsert_companion_mappings(guild_id, &[item])?;
        results
            .pop()
            .ok_or_else(|| anyhow::anyhow!("Failed to upsert companion mapping"))
    }

    pub fn batch_upsert_companion_mappings(
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

        let _guard = self
            .companion_lock
            .lock()
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let mut existing_for_guild: HashMap<String, (Uuid, DateTime<Utc>, String)> = self
            .companion_mappings
            .iter()
            .filter(|m| m.guild_id == guild_id)
            .map(|m| {
                (
                    m.voice_channel_id.clone(),
                    (m.id, m.created_at, m.text_channel_id.clone()),
                )
            })
            .collect();

        let mut final_voice_ids: HashSet<String> = existing_for_guild.keys().cloned().collect();
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
            let (id, created_at, old_text_opt) = if let Some(&(old_id, old_created, ref old_text)) =
                existing_for_guild.get(&item.voice_channel_id)
            {
                (old_id, old_created, Some(old_text.clone()))
            } else {
                (Uuid::new_v4(), now, None)
            };

            let mapping = CompanionChannelMapping {
                id,
                guild_id: guild_id.to_string(),
                voice_channel_id: item.voice_channel_id.clone(),
                text_channel_id: item.text_channel_id.clone(),
                allow_send_messages: item.allow_send_messages,
                allow_read_history: item.allow_read_history,
                created_at,
                enabled: true,
            };

            self.companion_mappings.insert(mapping.id, mapping.clone());
            existing_for_guild.insert(
                item.voice_channel_id.clone(),
                (id, created_at, item.text_channel_id.clone()),
            );

            // Issue #175: Invalidate any active/pending leases for this voice channel that target an old text channel
            if let Some(old_text) = old_text_opt
                && old_text != item.text_channel_id
            {
                for mut lease_entry in self.companion_leases.iter_mut() {
                    let lease = lease_entry.value_mut();
                    if lease.guild_id == guild_id
                        && lease.voice_channel_id == item.voice_channel_id
                        && lease.text_channel_id != item.text_channel_id
                        && (lease.status == "active" || lease.status == "revoke_pending")
                    {
                        lease.status = "revoke_pending".to_string();
                        lease.next_attempt_at = Some(now);
                    }
                }
            }

            saved.push(mapping);
        }

        Ok(saved)
    }

    pub fn delete_companion_mapping(
        &self,
        guild_id: &str,
        mapping_id: Uuid,
    ) -> anyhow::Result<bool> {
        validate_snowflake("guild_id", guild_id)?;
        if let Some((_, m)) = self.companion_mappings.remove(&mapping_id) {
            if m.guild_id == guild_id {
                return Ok(true);
            }
            // Put back if different guild
            self.companion_mappings.insert(mapping_id, m);
        }
        Ok(false)
    }

    pub fn get_companion_mapping_by_voice(
        &self,
        guild_id: &str,
        voice_channel_id: &str,
    ) -> anyhow::Result<Option<CompanionChannelMapping>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("voice_channel_id", voice_channel_id)?;
        let mapping = self
            .companion_mappings
            .iter()
            .find(|m| m.guild_id == guild_id && m.voice_channel_id == voice_channel_id)
            .map(|m| m.clone());
        Ok(mapping)
    }

    pub fn record_companion_lease(
        &self,
        params: RecordCompanionLeaseParams<'_>,
    ) -> anyhow::Result<CompanionChannelLease> {
        validate_snowflake("guild_id", params.guild_id)?;
        validate_snowflake("voice_channel_id", params.voice_channel_id)?;
        validate_snowflake("text_channel_id", params.text_channel_id)?;
        validate_snowflake("user_id", params.user_id)?;

        let lease = CompanionChannelLease {
            guild_id: params.guild_id.to_string(),
            voice_channel_id: params.voice_channel_id.to_string(),
            text_channel_id: params.text_channel_id.to_string(),
            user_id: params.user_id.to_string(),
            granted_at: Utc::now(),
            generation: params.generation,
            status: "active".to_string(),
            revoke_attempts: 0,
            next_attempt_at: None,
            last_error: None,
            revoked_at: None,
            allow_bits: params.allow_bits,
            deny_bits: params.deny_bits,
        };
        let key = (
            params.guild_id.to_string(),
            params.text_channel_id.to_string(),
            params.user_id.to_string(),
        );
        self.companion_leases.insert(key, lease.clone());
        Ok(lease)
    }

    pub fn mark_companion_lease_revoke_pending(
        &self,
        guild_id: &str,
        text_channel_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Option<CompanionChannelLease>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("text_channel_id", text_channel_id)?;
        validate_snowflake("user_id", user_id)?;

        let key = (
            guild_id.to_string(),
            text_channel_id.to_string(),
            user_id.to_string(),
        );
        if let Some(mut lease) = self.companion_leases.get_mut(&key) {
            lease.status = "revoke_pending".to_string();
            return Ok(Some(lease.clone()));
        }
        Ok(None)
    }

    pub fn record_companion_lease_failure(
        &self,
        guild_id: &str,
        text_channel_id: &str,
        user_id: &str,
        error: &str,
    ) -> anyhow::Result<()> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("text_channel_id", text_channel_id)?;
        validate_snowflake("user_id", user_id)?;

        let key = (
            guild_id.to_string(),
            text_channel_id.to_string(),
            user_id.to_string(),
        );
        if let Some(mut lease) = self.companion_leases.get_mut(&key) {
            lease.revoke_attempts += 1;
            lease.last_error = Some(error.to_string());
            lease.next_attempt_at = Some(Utc::now() + Duration::seconds(30));
        }
        Ok(())
    }

    pub fn get_companion_lease(
        &self,
        guild_id: &str,
        text_channel_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Option<CompanionChannelLease>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("text_channel_id", text_channel_id)?;
        validate_snowflake("user_id", user_id)?;

        let key = (
            guild_id.to_string(),
            text_channel_id.to_string(),
            user_id.to_string(),
        );
        Ok(self.companion_leases.get(&key).map(|l| l.clone()))
    }

    pub fn delete_companion_lease(
        &self,
        guild_id: &str,
        text_channel_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Option<CompanionChannelLease>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("text_channel_id", text_channel_id)?;
        validate_snowflake("user_id", user_id)?;

        let key = (
            guild_id.to_string(),
            text_channel_id.to_string(),
            user_id.to_string(),
        );
        Ok(self.companion_leases.remove(&key).map(|(_, l)| l))
    }

    pub fn list_active_companion_leases(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<CompanionChannelLease>> {
        validate_snowflake("guild_id", guild_id)?;
        let list: Vec<_> = self
            .companion_leases
            .iter()
            .filter(|l| l.guild_id == guild_id && l.status == "active")
            .map(|l| l.clone())
            .collect();
        Ok(list)
    }

    pub fn list_pending_revoke_companion_leases(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<CompanionChannelLease>> {
        validate_snowflake("guild_id", guild_id)?;
        let list: Vec<_> = self
            .companion_leases
            .iter()
            .filter(|l| l.guild_id == guild_id && l.status == "revoke_pending")
            .map(|l| l.clone())
            .collect();
        Ok(list)
    }

    pub fn list_member_companion_leases(
        &self,
        guild_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Vec<CompanionChannelLease>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("user_id", user_id)?;
        let mut list: Vec<_> = self
            .companion_leases
            .iter()
            .filter(|l| {
                l.guild_id == guild_id
                    && l.user_id == user_id
                    && (l.status == "active" || l.status == "revoke_pending")
            })
            .map(|l| l.clone())
            .collect();
        list.sort_by_key(|l| l.granted_at);
        Ok(list)
    }

    pub fn list_policy_rules(&self, guild_id: &str) -> anyhow::Result<Vec<CommandPolicyRule>> {
        validate_snowflake("guild_id", guild_id)?;
        let mut list: Vec<_> = self
            .policy_rules
            .iter()
            .filter(|r| r.guild_id == guild_id)
            .map(|r| r.value().clone())
            .collect();
        list.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.created_at.cmp(&b.created_at))
        });
        Ok(list)
    }

    pub fn upsert_policy_rule(
        &self,
        mut rule: CommandPolicyRule,
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
        if let Some(existing) = self.policy_rules.get(&rule.id) {
            if existing.guild_id != rule.guild_id {
                anyhow::bail!("policy rule ID belongs to another guild");
            }
            rule.version = existing.version + 1;
        }
        let now = Utc::now();
        rule.updated_at = now;
        self.policy_rules.insert(rule.id, rule.clone());
        self.bump_policy_revision(&rule.guild_id)?;
        Ok(rule)
    }

    pub fn delete_policy_rule(&self, guild_id: &str, rule_id: Uuid) -> anyhow::Result<bool> {
        validate_snowflake("guild_id", guild_id)?;
        if let Some((_, r)) = self.policy_rules.remove(&rule_id) {
            if r.guild_id == guild_id {
                self.bump_policy_revision(guild_id)?;
                return Ok(true);
            } else {
                self.policy_rules.insert(r.id, r);
            }
        }
        Ok(false)
    }

    pub fn reset_policy_rules(&self, guild_id: &str) -> anyhow::Result<usize> {
        validate_snowflake("guild_id", guild_id)?;
        let before = self.policy_rules.len();
        self.policy_rules.retain(|_, r| r.guild_id != guild_id);
        let removed = before.saturating_sub(self.policy_rules.len());
        self.bump_policy_revision(guild_id)?;
        Ok(removed)
    }

    pub fn replace_policy_rules(
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

        let current_rev = self.get_policy_revision(guild_id).unwrap_or(1);
        if let Some(expected) = expected_revision
            && expected != current_rev
        {
            anyhow::bail!(
                "Revision conflict: current revision is {}, expected {}",
                current_rev,
                expected
            );
        }

        self.policy_rules.retain(|_, r| r.guild_id != guild_id);
        let now = Utc::now();
        let mut inserted = Vec::with_capacity(rules.len());
        for mut rule in rules {
            rule.updated_at = now;
            self.policy_rules.insert(rule.id, rule.clone());
            inserted.push(rule);
        }
        self.bump_policy_revision(guild_id)?;
        Ok(inserted)
    }

    pub fn get_policy_revision(&self, guild_id: &str) -> anyhow::Result<i64> {
        validate_snowflake("guild_id", guild_id)?;
        Ok(self
            .policy_revisions
            .get(guild_id)
            .map(|r| *r.value())
            .unwrap_or(1))
    }

    pub fn bump_policy_revision(&self, guild_id: &str) -> anyhow::Result<i64> {
        validate_snowflake("guild_id", guild_id)?;
        let mut entry = self
            .policy_revisions
            .entry(guild_id.to_string())
            .or_insert(1);
        *entry += 1;
        Ok(*entry)
    }

    pub fn get_duty_settings(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Option<ModeratorDutySettings>> {
        validate_snowflake("guild_id", guild_id)?;
        Ok(self.duty_settings.get(guild_id).map(|r| r.value().clone()))
    }

    pub fn upsert_duty_settings(
        &self,
        mut settings: ModeratorDutySettings,
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
        settings.updated_at = Utc::now();
        self.duty_settings
            .insert(settings.guild_id.clone(), settings.clone());
        Ok(settings)
    }

    pub fn get_duty_profile(
        &self,
        guild_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Option<ModeratorDutyProfile>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("user_id", user_id)?;
        Ok(self
            .duty_profiles
            .get(&(guild_id.to_string(), user_id.to_string()))
            .map(|r| r.value().clone()))
    }

    pub fn list_duty_profiles(&self, guild_id: &str) -> anyhow::Result<Vec<ModeratorDutyProfile>> {
        validate_snowflake("guild_id", guild_id)?;
        let mut list: Vec<_> = self
            .duty_profiles
            .iter()
            .filter(|r| r.key().0 == guild_id)
            .map(|r| r.value().clone())
            .collect();
        list.sort_by(|a, b| a.user_id.cmp(&b.user_id));
        Ok(list)
    }

    pub fn list_guilds_with_duty_profiles(&self) -> anyhow::Result<Vec<String>> {
        let mut list: Vec<String> = self
            .duty_profiles
            .iter()
            .map(|r| r.key().0.clone())
            .collect();
        list.sort();
        list.dedup();
        Ok(list)
    }

    pub fn upsert_duty_profile(
        &self,
        mut profile: ModeratorDutyProfile,
    ) -> anyhow::Result<ModeratorDutyProfile> {
        validate_snowflake("guild_id", &profile.guild_id)?;
        validate_snowflake("user_id", &profile.user_id)?;
        profile.version += 1;
        profile.updated_at = Utc::now();
        self.duty_profiles.insert(
            (profile.guild_id.clone(), profile.user_id.clone()),
            profile.clone(),
        );
        Ok(profile)
    }

    pub fn delete_duty_profile(&self, guild_id: &str, user_id: &str) -> anyhow::Result<bool> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("user_id", user_id)?;
        let key = (guild_id.to_string(), user_id.to_string());
        let deleted = self.duty_profiles.remove(&key).is_some();
        if deleted {
            self.duty_states.remove(&key);
            let _ = self.mark_duty_role_grants_for_revocation(guild_id, Some(user_id), None);
        }
        Ok(deleted)
    }

    pub fn get_duty_state(
        &self,
        guild_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Option<ModeratorDutyState>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("user_id", user_id)?;
        Ok(self
            .duty_states
            .get(&(guild_id.to_string(), user_id.to_string()))
            .map(|r| r.value().clone()))
    }

    pub fn list_duty_states(&self, guild_id: &str) -> anyhow::Result<Vec<ModeratorDutyState>> {
        validate_snowflake("guild_id", guild_id)?;
        let mut list: Vec<_> = self
            .duty_states
            .iter()
            .filter(|r| r.key().0 == guild_id)
            .map(|r| r.value().clone())
            .collect();
        list.sort_by(|a, b| a.user_id.cmp(&b.user_id));
        Ok(list)
    }

    pub fn upsert_duty_state(
        &self,
        mut state: ModeratorDutyState,
    ) -> anyhow::Result<ModeratorDutyState> {
        validate_snowflake("guild_id", &state.guild_id)?;
        validate_snowflake("user_id", &state.user_id)?;
        state.state_generation += 1;
        state.last_reconciled_at = Some(Utc::now());
        self.duty_states.insert(
            (state.guild_id.clone(), state.user_id.clone()),
            state.clone(),
        );
        Ok(state)
    }

    pub fn get_duty_role_grant(
        &self,
        guild_id: &str,
        user_id: &str,
        role_id: &str,
    ) -> anyhow::Result<Option<crate::duty::ModeratorDutyRoleGrant>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("user_id", user_id)?;
        validate_snowflake("role_id", role_id)?;
        Ok(self
            .duty_role_grants
            .get(&(
                guild_id.to_string(),
                user_id.to_string(),
                role_id.to_string(),
            ))
            .map(|r| r.value().clone()))
    }

    pub fn upsert_duty_role_grant(
        &self,
        mut grant: crate::duty::ModeratorDutyRoleGrant,
    ) -> anyhow::Result<crate::duty::ModeratorDutyRoleGrant> {
        validate_snowflake("guild_id", &grant.guild_id)?;
        validate_snowflake("user_id", &grant.user_id)?;
        validate_snowflake("role_id", &grant.role_id)?;
        grant.updated_at = Utc::now();
        self.duty_role_grants.insert(
            (
                grant.guild_id.clone(),
                grant.user_id.clone(),
                grant.role_id.clone(),
            ),
            grant.clone(),
        );
        Ok(grant)
    }

    pub fn record_duty_role_grant(
        &self,
        guild_id: &str,
        user_id: &str,
        role_id: &str,
        status: &str,
    ) -> anyhow::Result<()> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("user_id", user_id)?;
        validate_snowflake("role_id", role_id)?;

        let key = (
            guild_id.to_string(),
            user_id.to_string(),
            role_id.to_string(),
        );
        let now = Utc::now();

        if let Some(mut existing) = self.duty_role_grants.get_mut(&key) {
            existing.status = status.to_string();
            if status == "active" {
                existing.attempts = 0;
                existing.last_error = None;
            }
            existing.generation += 1;
            existing.updated_at = now;
        } else {
            let grant = crate::duty::ModeratorDutyRoleGrant {
                guild_id: guild_id.to_string(),
                user_id: user_id.to_string(),
                role_id: role_id.to_string(),
                status: status.to_string(),
                attempts: 0,
                last_error: None,
                next_attempt_at: now,
                generation: 1,
                created_at: now,
                updated_at: now,
            };
            self.duty_role_grants.insert(key, grant);
        }

        Ok(())
    }

    pub fn mark_duty_role_grants_for_revocation(
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

        let now = Utc::now();
        let mut updated = Vec::new();

        for mut entry in self.duty_role_grants.iter_mut() {
            if entry.guild_id == guild_id
                && user_id.is_none_or(|uid| entry.user_id == uid)
                && role_id.is_none_or(|rid| entry.role_id == rid)
                && entry.status == "active"
            {
                entry.status = "revoke_pending".to_string();
                entry.next_attempt_at = now;
                entry.generation += 1;
                entry.updated_at = now;
                updated.push(entry.value().clone());
            }
        }

        Ok(updated)
    }

    pub fn list_pending_duty_role_revocations(
        &self,
        guild_id: Option<&str>,
    ) -> anyhow::Result<Vec<crate::duty::ModeratorDutyRoleGrant>> {
        if let Some(gid) = guild_id {
            validate_snowflake("guild_id", gid)?;
        }

        let now = Utc::now();
        let list: Vec<_> = self
            .duty_role_grants
            .iter()
            .filter(|g| {
                g.status == "revoke_pending"
                    && g.next_attempt_at <= now
                    && guild_id.is_none_or(|gid| g.guild_id == gid)
            })
            .map(|g| g.value().clone())
            .collect();

        Ok(list)
    }

    pub fn archive_progression_season(&self, guild_id: &str, name: &str) -> ProgressionSeason {
        let standings = self
            .progression
            .iter()
            .filter(|e| e.guild_id == guild_id)
            .map(|e| {
                serde_json::json!({

                    "user_id": e.user_id,

                    "level": e.level,

                    "xp": e.xp,

                    "coins": e.coins

                })
            })
            .collect::<Vec<_>>();

        let season_num = self
            .progression_seasons
            .iter()
            .filter(|e| e.guild_id == guild_id)
            .count() as i32
            + 1;

        let season = ProgressionSeason {
            id: Uuid::new_v4(),

            guild_id: guild_id.to_owned(),

            season_number: season_num,

            name: name.to_owned(),

            standings_json: serde_json::json!(standings),

            archived_at: Utc::now(),
        };

        self.progression_seasons.insert(season.id, season.clone());

        season
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_scheduled_event(
        &self,

        guild_id: &str,

        title: &str,

        description: &str,

        start_time: chrono::DateTime<Utc>,

        end_time: Option<chrono::DateTime<Utc>>,

        timezone: &str,

        max_participants: Option<i32>,
    ) -> ScheduledEvent {
        let id = Uuid::new_v4();

        let ev = ScheduledEvent {
            id,

            guild_id: guild_id.to_owned(),

            title: title.to_owned(),

            description: description.to_owned(),

            start_time,

            end_time,

            timezone: timezone.to_owned(),

            max_participants,

            created_at: Utc::now(),
        };

        self.events.insert(id, ev.clone());

        ev
    }

    pub fn list_scheduled_events(&self, guild_id: &str) -> Vec<ScheduledEvent> {
        let mut list: Vec<_> = self
            .events
            .iter()
            .filter(|e| e.guild_id == guild_id)
            .map(|e| e.value().clone())
            .collect();

        list.sort_by_key(|e| e.start_time);

        list
    }

    pub fn get_scheduled_event(&self, event_id: Uuid) -> Option<ScheduledEvent> {
        self.events.get(&event_id).map(|e| e.value().clone())
    }

    pub fn join_event_roster(&self, event_id: Uuid, user_id: &str) -> anyhow::Result<EventRoster> {
        let event = self
            .events
            .get(&event_id)
            .ok_or_else(|| anyhow::anyhow!("Scheduled event not found"))?;
        let capacity = event.max_participants;

        let mut roster_list = self.rosters.entry(event_id).or_default();

        if let Some(existing) = roster_list.iter().find(|r| r.user_id == user_id) {
            return Ok(existing.clone());
        }

        let main_count = roster_list.iter().filter(|r| r.role_type == "main").count() as i32;

        let role_type = match capacity {
            Some(limit) if limit > 0 => {
                if main_count < limit {
                    "main"
                } else {
                    "reserve"
                }
            }
            Some(_) => "reserve",
            None => "main",
        };

        let position = roster_list
            .iter()
            .map(|r| r.position)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("roster position overflow"))?;

        let entry = EventRoster {
            event_id,

            user_id: user_id.to_owned(),

            role_type: role_type.to_owned(),

            position,

            joined_at: Utc::now(),
        };

        roster_list.push(entry.clone());

        Ok(entry)
    }

    pub fn leave_event_roster(&self, event_id: Uuid, user_id: &str) -> Option<String> {
        let mut roster_list = self.rosters.get_mut(&event_id)?;

        let index = roster_list.iter().position(|r| r.user_id == user_id)?;

        let removed = roster_list.remove(index);

        if removed.role_type == "main" {
            let promoted = roster_list
                .iter_mut()
                .filter(|r| r.role_type == "reserve")
                .min_by(|a, b| {
                    a.position
                        .cmp(&b.position)
                        .then_with(|| a.joined_at.cmp(&b.joined_at))
                        .then_with(|| a.user_id.cmp(&b.user_id))
                })
                .map(|reserve| {
                    reserve.role_type = "main".to_owned();

                    reserve.user_id.clone()
                });

            if promoted.is_some() {
                return promoted;
            }
        }

        None
    }

    pub fn get_event_rosters(&self, event_id: Uuid) -> Vec<EventRoster> {
        let mut list = self
            .rosters
            .get(&event_id)
            .map(|r| r.clone())
            .unwrap_or_default();
        list.sort_by(|a, b| {
            a.position
                .cmp(&b.position)
                .then_with(|| a.joined_at.cmp(&b.joined_at))
                .then_with(|| a.user_id.cmp(&b.user_id))
        });
        list
    }

    pub fn create_tournament(&self, guild_id: &str, name: &str, teams: &[String]) -> Tournament {
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

        let tournament = Tournament {
            id,

            guild_id: guild_id.to_owned(),

            name: name.to_owned(),

            bracket_json: serde_json::json!({

                "teams": teams,

                "matches": matches,

            }),

            status: "in_progress".to_owned(),

            created_at: Utc::now(),
        };

        self.tournaments.insert(id, tournament.clone());

        tournament
    }

    pub fn advance_tournament_match(
        &self,
        guild_id: &str,
        tournament_id: Uuid,
        match_idx: usize,
        winner: &str,
    ) -> anyhow::Result<Tournament> {
        let mut tourney = self
            .tournaments
            .get_mut(&tournament_id)
            .ok_or_else(|| anyhow::anyhow!("tournament not found"))?;

        if tourney.guild_id != guild_id {
            anyhow::bail!("tournament not found in guild");
        }

        let matches = tourney
            .bracket_json
            .get_mut("matches")
            .and_then(|m| m.as_array_mut())
            .ok_or_else(|| anyhow::anyhow!("malformed bracket matches"))?;

        let match_obj = matches
            .get_mut(match_idx)
            .ok_or_else(|| anyhow::anyhow!("match index out of bounds"))?;

        match_obj["winner"] = serde_json::json!(winner);

        let cloned = tourney.clone();

        Ok(cloned)
    }

    pub fn record_invite(
        &self,

        guild_id: &str,

        code: &str,

        inviter_id: &str,

        campaign_tag: Option<&str>,
    ) -> InviteTracking {
        let key = (guild_id.to_owned(), code.to_owned());

        let mut inv = self.invites.entry(key).or_insert_with(|| InviteTracking {
            guild_id: guild_id.to_owned(),

            code: code.to_owned(),

            inviter_id: inviter_id.to_owned(),

            uses: 0,

            campaign_tag: campaign_tag.map(|s| s.to_owned()),

            created_at: Utc::now(),
        });

        inv.uses += 1;

        inv.clone()
    }

    pub fn list_invites(&self, guild_id: &str) -> Vec<InviteTracking> {
        self.invites
            .iter()
            .filter(|e| e.guild_id == guild_id)
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn register_dynamic_voice_room(
        &self,

        channel_id: &str,

        guild_id: &str,

        owner_id: &str,
    ) -> DynamicVoiceRoom {
        let room = DynamicVoiceRoom {
            channel_id: channel_id.to_owned(),

            guild_id: guild_id.to_owned(),

            owner_id: owner_id.to_owned(),

            empty_since: None,

            created_at: Utc::now(),
        };

        self.dynamic_voice
            .insert(channel_id.to_owned(), room.clone());

        room
    }

    pub fn update_dynamic_voice_empty_state(
        &self,
        guild_id: &str,
        channel_id: &str,
        is_empty: bool,
    ) -> bool {
        if let Some(mut room) = self.dynamic_voice.get_mut(channel_id) {
            if room.guild_id != guild_id {
                return false;
            }
            room.empty_since = if is_empty { Some(Utc::now()) } else { None };
            true
        } else {
            false
        }
    }

    pub fn get_stale_dynamic_voice_rooms(&self, threshold_seconds: i64) -> Vec<DynamicVoiceRoom> {
        let now = Utc::now();

        self.dynamic_voice
            .iter()
            .filter(|e| {
                if let Some(empty_since) = e.empty_since {
                    (now - empty_since).num_seconds() >= threshold_seconds
                } else {
                    false
                }
            })
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn remove_dynamic_voice_room(&self, channel_id: &str) -> bool {
        self.dynamic_voice.remove(channel_id).is_some()
    }

    pub fn get_dynamic_voice_room(&self, channel_id: &str) -> Option<DynamicVoiceRoom> {
        self.dynamic_voice
            .get(channel_id)
            .map(|r| r.value().clone())
    }

    pub fn set_dynamic_voice_empty_since(
        &self,
        channel_id: &str,
        empty_since: Option<DateTime<Utc>>,
    ) -> bool {
        if let Some(mut room) = self.dynamic_voice.get_mut(channel_id) {
            room.empty_since = empty_since;
            true
        } else {
            false
        }
    }

    pub fn cleanup_stale_dynamic_voice_rooms(
        &self,
        guild_id: &str,
        threshold_seconds: i64,
    ) -> Vec<String> {
        let cutoff = Utc::now() - Duration::seconds(threshold_seconds);
        let mut removed = Vec::new();
        self.dynamic_voice.retain(|channel_id, room| {
            if room.guild_id == guild_id
                && let Some(empty_since) = room.empty_since
                && empty_since <= cutoff
            {
                removed.push(channel_id.clone());
                return false;
            }
            true
        });
        removed
    }

    pub fn create_feed_subscription(
        &self,

        guild_id: &str,

        channel_id: &str,

        feed_type: &str,

        url_or_target: &str,
    ) -> FeedSubscription {
        let id = Uuid::new_v4();

        let sub = FeedSubscription {
            id,

            guild_id: guild_id.to_owned(),

            channel_id: channel_id.to_owned(),

            feed_type: feed_type.to_owned(),

            url_or_target: url_or_target.to_owned(),

            watermark: None,

            last_polled_at: None,

            created_at: Utc::now(),
        };

        self.feeds.insert(id, sub.clone());

        sub
    }

    pub fn list_feed_subscriptions(&self, guild_id: &str) -> Vec<FeedSubscription> {
        self.feeds
            .iter()
            .filter(|e| e.guild_id == guild_id)
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn update_feed_watermark(&self, subscription_id: Uuid, watermark: &str) {
        if let Some(mut sub) = self.feeds.get_mut(&subscription_id) {
            sub.watermark = Some(watermark.to_owned());

            sub.last_polled_at = Some(Utc::now());
        }
    }

    pub fn create_game_link(
        &self,

        guild_id: &str,

        user_id: &str,

        platform: &str,

        platform_account_id: &str,

        code: Option<&str>,
    ) -> GameAccountLink {
        let id = Uuid::new_v4();

        let link = GameAccountLink {
            id,

            guild_id: guild_id.to_owned(),

            user_id: user_id.to_owned(),

            platform: platform.to_owned(),

            platform_account_id: platform_account_id.to_owned(),

            verification_code: code.map(|c| c.to_owned()),

            verified: code.is_none(),

            created_at: Utc::now(),
        };

        let key = (guild_id.to_owned(), user_id.to_owned(), platform.to_owned());

        self.game_links.insert(key, link.clone());

        link
    }

    pub fn verify_game_link(
        &self,

        guild_id: &str,

        user_id: &str,

        platform: &str,

        code: &str,
    ) -> bool {
        let key = (guild_id.to_owned(), user_id.to_owned(), platform.to_owned());
        if let Some(mut link) = self.game_links.get_mut(&key)
            && link.verification_code.as_deref() == Some(code)
        {
            if link.verified || Utc::now() - link.created_at > chrono::Duration::minutes(15) {
                return false;
            }

            link.verified = true;

            link.verification_code = None; // Invalidate code so it cannot be reused

            return true;
        }

        false
    }

    pub fn get_game_links(&self, guild_id: &str, user_id: &str) -> Vec<GameAccountLink> {
        self.game_links
            .iter()
            .filter(|e| e.key().0 == guild_id && e.key().1 == user_id)
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn get_or_create_counting_game(&self, guild_id: &str, channel_id: &str) -> CountingGame {
        let key = (guild_id.to_owned(), channel_id.to_owned());

        self.counting_games
            .entry(key)
            .or_insert_with(|| CountingGame {
                guild_id: guild_id.to_owned(),

                channel_id: channel_id.to_owned(),

                current_count: 0,

                last_user_id: None,

                highest_count: 0,
            })
            .clone()
    }

    pub fn submit_count(
        &self,

        guild_id: &str,

        channel_id: &str,

        user_id: &str,

        count: i64,
    ) -> (bool, CountingGame) {
        let key = (guild_id.to_owned(), channel_id.to_owned());

        let mut game = self
            .counting_games
            .entry(key)
            .or_insert_with(|| CountingGame {
                guild_id: guild_id.to_owned(),

                channel_id: channel_id.to_owned(),

                current_count: 0,

                last_user_id: None,

                highest_count: 0,
            });

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

        (valid, game.clone())
    }

    pub fn list_user_cards(&self, guild_id: &str, user_id: &str) -> Vec<CardCollection> {
        self.cards
            .iter()
            .filter(|e| e.guild_id == guild_id && e.user_id == user_id)
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn grant_user_card(
        &self,

        guild_id: &str,

        user_id: &str,

        card_id: &str,

        card_name: &str,

        rarity: &str,
    ) -> CardCollection {
        let id = Uuid::new_v4();

        let card = CardCollection {
            id,

            guild_id: guild_id.to_owned(),

            user_id: user_id.to_owned(),

            card_id: card_id.to_owned(),

            card_name: card_name.to_owned(),

            rarity: rarity.to_owned(),

            acquired_at: Utc::now(),
        };

        self.cards.insert(id, card.clone());

        card
    }

    pub fn propose_card_trade(
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

        anyhow::ensure!(initiator_id != target_id, "cannot trade with yourself");

        let c1 = self
            .cards
            .get(&initiator_card_id)
            .ok_or_else(|| anyhow::anyhow!("initiator card not found"))?;

        if c1.guild_id != guild_id || c1.user_id != initiator_id {
            return Err(anyhow::anyhow!("initiator does not own card"));
        }

        let c2 = self
            .cards
            .get(&target_card_id)
            .ok_or_else(|| anyhow::anyhow!("target card not found"))?;

        if c2.guild_id != guild_id || c2.user_id != target_id {
            return Err(anyhow::anyhow!("target does not own card"));
        }

        let trade = CardTrade {
            id: Uuid::new_v4(),

            guild_id: guild_id.to_owned(),

            initiator_user_id: initiator_id.to_owned(),

            initiator_card_id,

            target_user_id: target_id.to_owned(),

            target_card_id,

            status: "pending".to_owned(),

            created_at: Utc::now(),

            updated_at: Utc::now(),
        };

        self.card_trades.insert(trade.id, trade.clone());

        Ok(trade)
    }

    pub fn accept_card_trade(
        &self,

        guild_id: &str,

        trade_id: Uuid,

        target_id: &str,
    ) -> anyhow::Result<bool> {
        let _guard = self
            .card_trade_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let mut trade = self
            .card_trades
            .get_mut(&trade_id)
            .ok_or_else(|| anyhow::anyhow!("trade not found"))?;

        if trade.guild_id != guild_id || trade.target_user_id != target_id {
            return Err(anyhow::anyhow!("unauthorized to accept trade"));
        }

        if trade.status != "pending" {
            return Err(anyhow::anyhow!("trade is not pending"));
        }

        anyhow::ensure!(
            trade.initiator_card_id != trade.target_card_id,
            "invalid trade cards"
        );

        // Verify ownership without holding mutable references to avoid DashMap shard deadlock (#120)

        {
            let c1 = self
                .cards
                .get(&trade.initiator_card_id)
                .ok_or_else(|| anyhow::anyhow!("initiator card not found"))?;

            let c2 = self
                .cards
                .get(&trade.target_card_id)
                .ok_or_else(|| anyhow::anyhow!("target card not found"))?;

            if c1.guild_id != guild_id || c2.guild_id != guild_id {
                return Err(anyhow::anyhow!("card belongs to different guild"));
            }

            if c1.user_id != trade.initiator_user_id || c2.user_id != trade.target_user_id {
                return Err(anyhow::anyhow!("ownership changed before trade completion"));
            }
        }

        // Mutate sequentially

        {
            let mut c1 = self
                .cards
                .get_mut(&trade.initiator_card_id)
                .ok_or_else(|| anyhow::anyhow!("initiator card not found"))?;

            c1.user_id = trade.target_user_id.clone();
        }

        {
            let mut c2 = self
                .cards
                .get_mut(&trade.target_card_id)
                .ok_or_else(|| anyhow::anyhow!("target card not found"))?;

            c2.user_id = trade.initiator_user_id.clone();
        }

        trade.status = "accepted".to_owned();

        trade.updated_at = Utc::now();

        Ok(true)
    }

    pub fn create_tupper_profile(
        &self,

        guild_id: &str,

        user_id: &str,

        name: &str,

        avatar_url: Option<&str>,

        prefix_or_brackets: &str,
    ) -> Result<TupperProfile, TupperError> {
        let key = (
            guild_id.to_owned(),
            user_id.to_owned(),
            prefix_or_brackets.to_owned(),
        );

        match self.tupper_prefix_index.entry(key) {
            dashmap::mapref::entry::Entry::Occupied(_) => Err(TupperError::PrefixConflict),
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                let id = Uuid::new_v4();
                entry.insert(id);

                let profile = TupperProfile {
                    id,

                    guild_id: guild_id.to_owned(),

                    user_id: user_id.to_owned(),

                    name: name.to_owned(),

                    avatar_url: avatar_url.map(|s| s.to_owned()),

                    prefix_or_brackets: prefix_or_brackets.to_owned(),

                    created_at: Utc::now(),
                };

                self.tuppers.insert(id, profile.clone());

                Ok(profile)
            }
        }
    }

    pub fn list_tupper_profiles(&self, guild_id: &str, user_id: &str) -> Vec<TupperProfile> {
        self.tuppers
            .iter()
            .filter(|e| e.guild_id == guild_id && e.user_id == user_id)
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn create_faq_entry(
        &self,

        guild_id: &str,

        question: &str,

        answer: &str,

        aliases: &[String],

        category: &str,
    ) -> FaqEntry {
        let id = Uuid::new_v4();

        let now = Utc::now();

        let entry = FaqEntry {
            id,

            guild_id: guild_id.to_owned(),

            question: question.to_owned(),

            answer: answer.to_owned(),

            aliases_json: serde_json::json!(aliases),

            category: category.to_owned(),

            created_at: now,

            updated_at: now,
        };

        self.faqs.insert(id, entry.clone());

        entry
    }

    pub fn list_faq_entries(
        &self,
        guild_id: &str,
        query: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Vec<FaqEntry> {
        let q = query.map(|s| s.to_lowercase());
        let limit = limit.clamp(1, 50);

        let mut matched: Vec<FaqEntry> = self
            .faqs
            .iter()
            .filter(|e| {
                if e.guild_id != guild_id {
                    return false;
                }

                if let Some(ref search) = q {
                    e.question.to_lowercase().contains(search)
                        || e.answer.to_lowercase().contains(search)
                        || e.category.to_lowercase().contains(search)
                } else {
                    true
                }
            })
            .map(|e| e.value().clone())
            .collect();

        matched.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.id.cmp(&a.id))
        });

        matched.into_iter().skip(offset).take(limit).collect()
    }

    pub fn delete_faq_entry(&self, guild_id: &str, faq_id: Uuid) -> bool {
        if let Some(entry) = self.faqs.get(&faq_id)
            && entry.guild_id == guild_id
        {
            drop(entry);

            return self.faqs.remove(&faq_id).is_some();
        }

        false
    }

    // ==========================================

    // Phase 4: Integrations & Extensibility (#92-#107)

    // ==========================================

    pub fn upsert_integration_connection(
        &self,

        conn: IntegrationConnection,
    ) -> IntegrationConnection {
        self.integration_connections.insert(conn.id, conn.clone());

        conn
    }

    pub fn get_integration_connection(
        &self,

        guild_id: &str,

        provider: &str,
    ) -> Option<IntegrationConnection> {
        self.integration_connections
            .iter()
            .find(|e| e.guild_id == guild_id && e.provider.eq_ignore_ascii_case(provider))
            .map(|e| e.value().clone())
    }

    pub fn list_integration_connections(&self, guild_id: &str) -> Vec<IntegrationConnection> {
        self.integration_connections
            .iter()
            .filter(|e| e.guild_id == guild_id)
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn delete_integration_connection(&self, guild_id: &str, provider: &str) -> bool {
        let to_remove: Vec<Uuid> = self
            .integration_connections
            .iter()
            .filter(|e| e.guild_id == guild_id && e.provider.eq_ignore_ascii_case(provider))
            .map(|e| e.id)
            .collect();

        let count = to_remove.len();

        for id in to_remove {
            self.integration_connections.remove(&id);
        }

        count > 0
    }

    pub fn create_webhook_subscription(
        &self,

        sub: IntegrationWebhookSubscription,
    ) -> IntegrationWebhookSubscription {
        self.integration_webhook_subs.insert(sub.id, sub.clone());

        sub
    }

    pub fn list_webhook_subscriptions(
        &self,

        guild_id: &str,

        provider: Option<&str>,
    ) -> Vec<IntegrationWebhookSubscription> {
        self.integration_webhook_subs
            .iter()
            .filter(|e| {
                e.guild_id == guild_id
                    && provider
                        .map(|p| e.provider.eq_ignore_ascii_case(p))
                        .unwrap_or(true)
            })
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn delete_webhook_subscription(&self, id: Uuid) -> bool {
        self.integration_webhook_subs.remove(&id).is_some()
    }

    pub fn check_and_record_event_dedupe(&self, provider: &str, event_id: &str) -> bool {
        let key = (provider.to_ascii_lowercase(), event_id.to_string());

        match self.integration_event_dedupes.entry(key) {
            dashmap::mapref::entry::Entry::Occupied(_) => false,
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                entry.insert(Utc::now());
                true
            }
        }
    }

    pub fn record_delivery_log(&self, log: IntegrationDeliveryLog) -> IntegrationDeliveryLog {
        self.integration_delivery_logs.insert(log.id, log.clone());

        log
    }

    pub fn list_delivery_logs(
        &self,

        guild_id: &str,

        provider: Option<&str>,
    ) -> Vec<IntegrationDeliveryLog> {
        let mut list: Vec<_> = self
            .integration_delivery_logs
            .iter()
            .filter(|e| {
                e.guild_id == guild_id
                    && provider
                        .map(|p| e.provider.eq_ignore_ascii_case(p))
                        .unwrap_or(true)
            })
            .map(|e| e.value().clone())
            .collect();

        list.sort_by_key(|a| std::cmp::Reverse(a.created_at));

        list
    }

    // Agent 04 (#93): Unified Identity

    pub fn link_account(&self, link: LinkedAccount) -> anyhow::Result<LinkedAccount> {
        // Enforce uniqueness

        for item in self.linked_accounts.iter() {
            if item.guild_id == link.guild_id && item.provider == link.provider {
                if item.provider_subject == link.provider_subject
                    && item.discord_user_id != link.discord_user_id
                {
                    anyhow::bail!("Provider account already linked to another Discord user");
                }

                if item.discord_user_id == link.discord_user_id
                    && item.provider_subject != link.provider_subject
                {
                    anyhow::bail!("User already linked to a different account for this provider");
                }
            }
        }

        self.linked_accounts.insert(link.id, link.clone());

        Ok(link)
    }

    pub fn get_linked_account(
        &self,

        guild_id: &str,

        user_id: &str,

        provider: &str,
    ) -> Option<LinkedAccount> {
        self.linked_accounts
            .iter()
            .find(|e| {
                e.guild_id == guild_id
                    && e.discord_user_id == user_id
                    && e.provider.eq_ignore_ascii_case(provider)
            })
            .map(|e| e.value().clone())
    }

    pub fn list_linked_accounts(&self, guild_id: &str, user_id: &str) -> Vec<LinkedAccount> {
        self.linked_accounts
            .iter()
            .filter(|e| e.guild_id == guild_id && e.discord_user_id == user_id)
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn unlink_account(&self, guild_id: &str, user_id: &str, provider: &str) -> bool {
        let to_remove: Vec<Uuid> = self
            .linked_accounts
            .iter()
            .filter(|e| {
                e.guild_id == guild_id
                    && e.discord_user_id == user_id
                    && e.provider.eq_ignore_ascii_case(provider)
            })
            .map(|e| e.id)
            .collect();

        let found = !to_remove.is_empty();

        for id in to_remove {
            self.linked_accounts.remove(&id);
        }

        found
    }

    // Agent 05 (#94) & Agent 15 (#104): Steam & Gaming Hub LFG

    pub fn create_steam_lfg_post(&self, post: SteamLfgPost) -> SteamLfgPost {
        self.steam_lfg_posts.insert(post.id, post.clone());

        post
    }

    pub fn count_active_steam_lfg_posts(&self, guild_id: &str, author_id: &str) -> usize {
        let now = Utc::now();
        self.steam_lfg_posts
            .iter()
            .filter(|e| e.guild_id == guild_id && e.author_id == author_id && e.expires_at > now)
            .count()
    }

    pub fn count_active_steam_lfg_posts_guild(&self, guild_id: &str) -> usize {
        let now = Utc::now();
        self.steam_lfg_posts
            .iter()
            .filter(|e| e.guild_id == guild_id && e.expires_at > now)
            .count()
    }

    pub fn prune_steam_lfg_posts(&self, cutoff: DateTime<Utc>) -> u64 {
        let before = self.steam_lfg_posts.len();
        self.steam_lfg_posts
            .retain(|_, post| post.expires_at > cutoff);
        before.saturating_sub(self.steam_lfg_posts.len()) as u64
    }

    pub fn list_steam_lfg_posts(&self, guild_id: &str, game_id: Option<&str>) -> Vec<SteamLfgPost> {
        let now = Utc::now();

        self.steam_lfg_posts
            .iter()
            .filter(|e| {
                e.guild_id == guild_id
                    && e.expires_at > now
                    && game_id.map(|g| e.game_id == g).unwrap_or(true)
            })
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn join_steam_lfg_post(
        &self,
        guild_id: &str,
        id: Uuid,
        user_id: &str,
    ) -> anyhow::Result<SteamLfgPost> {
        let mut entry = self
            .steam_lfg_posts
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("LFG listing not found"))?;

        if entry.guild_id != guild_id {
            anyhow::bail!("LFG listing not found in guild");
        }

        if entry.expires_at <= Utc::now() {
            anyhow::bail!("LFG listing has expired");
        }

        let mut players: Vec<String> =
            serde_json::from_value(entry.current_players.clone()).unwrap_or_default();

        if players.contains(&user_id.to_string()) {
            return Ok(entry.clone());
        }

        if players.len() as i32 >= entry.max_players {
            anyhow::bail!("LFG post is full");
        }

        players.push(user_id.to_string());

        entry.current_players = serde_json::json!(players);

        Ok(entry.clone())
    }

    // Agent 06 (#95): GitHub DevOps

    pub fn upsert_github_devops_mapping(
        &self,

        mapping: GitHubDevopsMapping,
    ) -> GitHubDevopsMapping {
        let key = (
            mapping.guild_id.clone(),
            mapping.repo_full_name.to_ascii_lowercase(),
            mapping.issue_or_pr_number,
        );
        match self.github_devops_mappings.entry(key) {
            dashmap::mapref::entry::Entry::Occupied(mut occ) => {
                let existing = occ.get_mut();
                existing.thread_channel_id = mapping.thread_channel_id;
                existing.discord_thread_id = mapping.discord_thread_id;
                existing.last_status = mapping.last_status;
                existing.clone()
            }
            dashmap::mapref::entry::Entry::Vacant(vac) => {
                vac.insert(mapping.clone());
                mapping
            }
        }
    }

    pub fn get_github_devops_mapping(
        &self,

        guild_id: &str,

        repo: &str,

        number: i32,
    ) -> Option<GitHubDevopsMapping> {
        let key = (guild_id.to_string(), repo.to_ascii_lowercase(), number);
        self.github_devops_mappings
            .get(&key)
            .map(|e| e.value().clone())
    }

    // Agent 07 (#96): Twitch Extended

    pub fn upsert_twitch_subscription(
        &self,

        sub: TwitchStreamSubscription,
    ) -> TwitchStreamSubscription {
        let key = (
            sub.guild_id.clone(),
            sub.streamer_login.to_ascii_lowercase(),
        );
        match self.twitch_subs.entry(key) {
            dashmap::mapref::entry::Entry::Occupied(mut occ) => {
                let existing = occ.get_mut();
                existing.streamer_id = sub.streamer_id;
                existing.discord_channel_id = sub.discord_channel_id;
                existing.live_role_id = sub.live_role_id;
                existing.last_message_id = sub.last_message_id;
                existing.is_live = sub.is_live;
                existing.last_title = sub.last_title;
                existing.last_game = sub.last_game;
                existing.clone()
            }
            dashmap::mapref::entry::Entry::Vacant(vac) => {
                vac.insert(sub.clone());
                sub
            }
        }
    }

    pub fn get_twitch_subscription(
        &self,

        guild_id: &str,

        streamer: &str,
    ) -> Option<TwitchStreamSubscription> {
        let key = (guild_id.to_string(), streamer.to_ascii_lowercase());
        self.twitch_subs.get(&key).map(|e| e.value().clone())
    }

    // Agent 08 (#97): Kick Integration

    pub fn upsert_kick_subscription(&self, sub: KickStreamSubscription) -> KickStreamSubscription {
        let key = (sub.guild_id.clone(), sub.slug.to_ascii_lowercase());
        match self.kick_subs.entry(key) {
            dashmap::mapref::entry::Entry::Occupied(mut occ) => {
                let existing = occ.get_mut();
                existing.channel_id = sub.channel_id;
                existing.discord_channel_id = sub.discord_channel_id;
                existing.live_role_id = sub.live_role_id;
                existing.last_message_id = sub.last_message_id;
                existing.is_live = sub.is_live;
                existing.clone()
            }
            dashmap::mapref::entry::Entry::Vacant(vac) => {
                vac.insert(sub.clone());
                sub
            }
        }
    }

    pub fn get_kick_subscription(
        &self,

        guild_id: &str,

        slug: &str,
    ) -> Option<KickStreamSubscription> {
        let key = (guild_id.to_string(), slug.to_ascii_lowercase());
        self.kick_subs.get(&key).map(|e| e.value().clone())
    }

    // Agent 09 (#98): YouTube Extended

    pub fn upsert_youtube_subscription(
        &self,

        sub: YouTubeChannelSubscription,
    ) -> YouTubeChannelSubscription {
        let key = (sub.guild_id.clone(), sub.channel_id.clone());
        match self.youtube_subs.entry(key) {
            dashmap::mapref::entry::Entry::Occupied(mut occ) => {
                let existing = occ.get_mut();
                existing.channel_title = sub.channel_title;
                existing.discord_channel_id = sub.discord_channel_id;
                existing.last_video_id = sub.last_video_id;
                existing.clone()
            }
            dashmap::mapref::entry::Entry::Vacant(vac) => {
                vac.insert(sub.clone());
                sub
            }
        }
    }

    pub fn get_youtube_subscription(
        &self,

        guild_id: &str,

        channel_id: &str,
    ) -> Option<YouTubeChannelSubscription> {
        let key = (guild_id.to_string(), channel_id.to_string());
        self.youtube_subs.get(&key).map(|e| e.value().clone())
    }

    // Agent 10 (#99): Telegram Bridge

    pub fn create_telegram_bridge(&self, bridge: TelegramBridge) -> TelegramBridge {
        self.telegram_bridges.insert(bridge.id, bridge.clone());

        bridge
    }

    pub fn get_telegram_bridge(&self, guild_id: &str, channel_id: &str) -> Option<TelegramBridge> {
        self.telegram_bridges
            .iter()
            .find(|e| e.guild_id == guild_id && e.discord_channel_id == channel_id)
            .map(|e| e.value().clone())
    }

    pub fn list_telegram_bridges(&self, guild_id: &str) -> Vec<TelegramBridge> {
        self.telegram_bridges
            .iter()
            .filter(|e| e.guild_id == guild_id)
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn record_telegram_message_mapping(
        &self,

        mapping: TelegramMessageMapping,
    ) -> TelegramMessageMapping {
        self.telegram_mappings.insert(mapping.id, mapping.clone());

        mapping
    }

    pub fn get_telegram_message_mapping(
        &self,

        bridge_id: Uuid,

        msg_id: &str,

        direction: &str,
    ) -> Option<TelegramMessageMapping> {
        self.telegram_mappings
            .iter()
            .find(|e| {
                e.bridge_id == bridge_id
                    && e.direction == direction
                    && (e.telegram_message_id == msg_id || e.discord_message_id == msg_id)
            })
            .map(|e| e.value().clone())
    }

    pub fn delete_telegram_bridge(&self, bridge_id: Uuid) -> bool {
        self.telegram_bridges.remove(&bridge_id).is_some()
    }

    // Agent 11 (#100): Workflow Builder

    pub fn create_workflow_definition(&self, wf: WorkflowDefinition) -> WorkflowDefinition {
        self.workflows.insert(wf.id, wf.clone());

        wf
    }

    pub fn get_workflow_definition(&self, id: Uuid) -> Option<WorkflowDefinition> {
        self.workflows.get(&id).map(|e| e.value().clone())
    }

    pub fn list_workflow_definitions(&self, guild_id: &str) -> Vec<WorkflowDefinition> {
        self.workflows
            .iter()
            .filter(|e| e.guild_id == guild_id)
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn record_workflow_run(&self, run: WorkflowRunExecution) -> WorkflowRunExecution {
        self.workflow_runs.insert(run.id, run.clone());

        run
    }

    // Agent 12 (#101): GitLab

    pub fn upsert_gitlab_subscription(&self, sub: GitLabSubscription) -> GitLabSubscription {
        self.gitlab_subs.insert(sub.id, sub.clone());

        sub
    }

    pub fn get_gitlab_subscription(
        &self,

        guild_id: &str,

        host: &str,

        project_id: &str,
    ) -> Option<GitLabSubscription> {
        self.gitlab_subs
            .iter()
            .find(|e| {
                e.guild_id == guild_id
                    && e.host.eq_ignore_ascii_case(host)
                    && e.project_id == project_id
            })
            .map(|e| e.value().clone())
    }

    // Agent 13 (#102): VK Integration

    pub fn upsert_vk_subscription(&self, sub: VkSubscription) -> VkSubscription {
        self.vk_subs.insert(sub.id, sub.clone());

        sub
    }

    pub fn get_vk_subscription(&self, guild_id: &str, group_id: &str) -> Option<VkSubscription> {
        self.vk_subs
            .iter()
            .find(|e| e.guild_id == guild_id && e.group_id == group_id)
            .map(|e| e.value().clone())
    }

    // Agent 14 (#103): Reddit

    pub fn upsert_reddit_subscription(&self, sub: RedditSubscription) -> RedditSubscription {
        self.reddit_subs.insert(sub.id, sub.clone());

        sub
    }

    pub fn get_reddit_subscription(
        &self,

        guild_id: &str,

        subreddit: &str,
    ) -> Option<RedditSubscription> {
        self.reddit_subs
            .iter()
            .find(|e| e.guild_id == guild_id && e.subreddit.eq_ignore_ascii_case(subreddit))
            .map(|e| e.value().clone())
    }

    // Agent 15 (#104): Gaming Hub

    pub fn update_gaming_hub_game_status(
        &self,

        status: GamingHubGameStatus,
    ) -> GamingHubGameStatus {
        let key = (status.game_id.clone(), status.region.clone());

        self.gaming_hub_statuses.insert(key, status.clone());

        status
    }

    pub fn list_gaming_hub_game_statuses(&self, game_id: Option<&str>) -> Vec<GamingHubGameStatus> {
        self.gaming_hub_statuses
            .iter()
            .filter(|e| game_id.map(|g| e.game_id == g).unwrap_or(true))
            .map(|e| e.value().clone())
            .collect()
    }

    // Agent 16 (#105): Social Publisher

    pub fn create_social_draft(&self, draft: SocialPublicationDraft) -> SocialPublicationDraft {
        self.social_drafts.insert(draft.id, draft.clone());

        draft
    }

    pub fn list_social_drafts(&self, guild_id: &str) -> Vec<SocialPublicationDraft> {
        self.social_drafts
            .iter()
            .filter(|e| e.guild_id == guild_id)
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn approve_social_draft(
        &self,

        guild_id: &str,

        id: Uuid,

        approver_id: &str,
    ) -> Option<SocialPublicationDraft> {
        if let Some(mut draft) = self.social_drafts.get_mut(&id) {
            if draft.guild_id != guild_id {
                return None;
            }

            draft.approved_by = Some(approver_id.to_string());

            draft.status = "approved".to_string();

            Some(draft.clone())
        } else {
            None
        }
    }

    // Agent 17 & 18 (#106): AI + MCP

    pub fn get_or_create_ai_budget(&self, guild_id: &str) -> AiGuildBudget {
        self.ai_budgets
            .entry(guild_id.to_string())
            .or_insert_with(|| AiGuildBudget {
                guild_id: guild_id.to_string(),

                monthly_token_limit: 1_000_000,

                used_tokens: 0,

                monthly_usd_limit: 10.0,

                used_usd: 0.0,

                is_halted: false,

                reset_at: Utc::now() + chrono::Duration::days(30),
            })
            .clone()
    }

    pub fn record_ai_usage(
        &self,

        guild_id: &str,

        tokens: i64,

        usd: f64,
    ) -> anyhow::Result<AiGuildBudget> {
        anyhow::ensure!(tokens >= 0, "tokens must be non-negative");

        anyhow::ensure!(usd >= 0.0, "usd must be non-negative");

        let mut budget = self
            .ai_budgets
            .entry(guild_id.to_string())
            .or_insert_with(|| AiGuildBudget {
                guild_id: guild_id.to_string(),

                monthly_token_limit: 1_000_000,

                used_tokens: 0,

                monthly_usd_limit: 10.0,

                used_usd: 0.0,

                is_halted: false,

                reset_at: Utc::now() + chrono::Duration::days(30),
            });

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

        budget.used_tokens = new_tokens;

        budget.used_usd += usd;

        if budget.used_tokens >= budget.monthly_token_limit
            || budget.used_usd >= budget.monthly_usd_limit
        {
            budget.is_halted = true;
        }

        Ok(budget.clone())
    }

    pub fn create_ai_document(&self, doc: AiKnowledgeDocument) -> AiKnowledgeDocument {
        self.ai_documents.insert(doc.id, doc.clone());

        doc
    }

    pub fn search_ai_documents(
        &self,
        guild_id: &str,
        user_channels: &[String],
        query: &str,
        limit: usize,
    ) -> Vec<AiKnowledgeDocument> {
        if query.trim().is_empty() {
            return Vec::new();
        }

        let q = query.to_lowercase();
        let limit = limit.clamp(1, 50);

        let mut matched: Vec<AiKnowledgeDocument> = self
            .ai_documents
            .iter()
            .filter(|e| {
                if e.guild_id != guild_id {
                    return false;
                }

                let has_access = match &e.channel_acl {
                    serde_json::Value::Array(arr) => {
                        if arr.is_empty() {
                            true
                        } else {
                            arr.iter().any(|val| {
                                val.as_str()
                                    .map(|ch| user_channels.contains(&ch.to_string()))
                                    .unwrap_or(false)
                            })
                        }
                    }
                    _ => false, // Non-array is invalid ACL -> fail-closed (deny)
                };

                if !has_access {
                    return false;
                }

                e.title.to_lowercase().contains(&q) || e.content.to_lowercase().contains(&q)
            })
            .map(|e| e.value().clone())
            .collect();

        matched.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.id.cmp(&a.id))
        });

        matched.truncate(limit);
        matched
    }

    pub fn upsert_mcp_tool_authorization(
        &self,

        auth: McpToolAuthorization,
    ) -> McpToolAuthorization {
        let key = (auth.guild_id.clone(), auth.tool_name.clone());

        self.mcp_tool_auths.insert(key, auth.clone());

        auth
    }

    pub fn get_mcp_tool_authorizations(&self, guild_id: &str) -> Vec<McpToolAuthorization> {
        self.mcp_tool_auths
            .iter()
            .filter(|e| e.guild_id == guild_id)
            .map(|e| e.value().clone())
            .collect()
    }

    // Agent 19 (#107): Plugin Marketplace

    pub fn publish_marketplace_package(
        &self,

        pkg: PluginMarketplacePackage,
    ) -> PluginMarketplacePackage {
        self.marketplace_packages
            .insert(pkg.plugin_id.clone(), pkg.clone());

        pkg
    }

    pub fn list_marketplace_packages(&self) -> Vec<PluginMarketplacePackage> {
        self.marketplace_packages
            .iter()
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn install_plugin_package(
        &self,

        install: PluginGuildInstallation,
    ) -> PluginGuildInstallation {
        let key = (install.guild_id.clone(), install.plugin_id.clone());

        self.plugin_guild_installs.insert(key, install.clone());

        install
    }

    pub fn list_installed_plugins(&self, guild_id: &str) -> Vec<PluginGuildInstallation> {
        self.plugin_guild_installs
            .iter()
            .filter(|e| e.guild_id == guild_id)
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn uninstall_plugin_package(&self, guild_id: &str, plugin_id: &str) -> bool {
        let key = (guild_id.to_string(), plugin_id.to_string());

        self.plugin_guild_installs.remove(&key).is_some()
    }

    // ==========================================
    // Staff Recruitment Pipeline (Feature #166)
    // ==========================================

    pub fn get_recruitment_track(
        &self,
        guild_id: &str,
        track_id: Uuid,
    ) -> anyhow::Result<Option<crate::recruitment::RecruitmentTrack>> {
        validate_snowflake("guild_id", guild_id)?;
        Ok(self
            .recruitment_tracks
            .get(&track_id)
            .filter(|t| t.guild_id == guild_id)
            .map(|t| t.value().clone()))
    }

    pub fn get_recruitment_track_by_slug(
        &self,
        guild_id: &str,
        slug: &str,
    ) -> anyhow::Result<Option<crate::recruitment::RecruitmentTrack>> {
        validate_snowflake("guild_id", guild_id)?;
        Ok(self
            .recruitment_tracks
            .iter()
            .find(|t| t.guild_id == guild_id && t.slug == slug)
            .map(|t| t.value().clone()))
    }

    pub fn list_recruitment_tracks(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<crate::recruitment::RecruitmentTrack>> {
        validate_snowflake("guild_id", guild_id)?;
        let mut list: Vec<_> = self
            .recruitment_tracks
            .iter()
            .filter(|t| t.guild_id == guild_id)
            .map(|t| t.value().clone())
            .collect();
        list.sort_by_key(|a| a.created_at);
        Ok(list)
    }

    pub fn upsert_recruitment_track(
        &self,
        mut track: crate::recruitment::RecruitmentTrack,
    ) -> anyhow::Result<crate::recruitment::RecruitmentTrack> {
        validate_snowflake("guild_id", &track.guild_id)?;
        crate::recruitment::validate_track_fields(
            &track.guild_id,
            &track.slug,
            &track.name,
            track.target_role_id.as_deref(),
        )?;

        if self
            .recruitment_tracks
            .iter()
            .any(|t| t.guild_id == track.guild_id && t.slug == track.slug && t.id != track.id)
        {
            anyhow::bail!(
                "Recruitment track slug '{}' already exists in this server",
                track.slug
            );
        }

        if let Some(existing) = self.recruitment_tracks.get(&track.id) {
            if existing.guild_id != track.guild_id {
                anyhow::bail!("Track ID belongs to another guild");
            }
            track.version = existing.version + 1;
        }

        track.updated_at = Utc::now();
        self.recruitment_tracks.insert(track.id, track.clone());
        Ok(track)
    }

    pub fn delete_recruitment_track(&self, guild_id: &str, track_id: Uuid) -> anyhow::Result<bool> {
        validate_snowflake("guild_id", guild_id)?;
        if let Some(existing) = self.recruitment_tracks.get(&track_id)
            && existing.guild_id == guild_id
        {
            drop(existing);
            self.recruitment_tracks.remove(&track_id);
            let app_ids: Vec<Uuid> = self
                .recruitment_applications
                .iter()
                .filter(|a| a.track_id == track_id)
                .map(|a| a.id)
                .collect();
            for aid in app_ids {
                self.recruitment_applications.remove(&aid);
                self.recruitment_reviews
                    .retain(|_, r| r.application_id != aid);
                self.recruitment_role_fulfillments.remove(&aid);
            }
            return Ok(true);
        }
        Ok(false)
    }

    pub fn get_recruitment_application(
        &self,
        guild_id: &str,
        id: Uuid,
    ) -> anyhow::Result<Option<crate::recruitment::RecruitmentApplication>> {
        validate_snowflake("guild_id", guild_id)?;
        Ok(self
            .recruitment_applications
            .get(&id)
            .filter(|a| a.guild_id == guild_id)
            .map(|a| a.value().clone()))
    }

    pub fn list_recruitment_applications(
        &self,
        guild_id: &str,
        track_id: Option<Uuid>,
        status: Option<&str>,
        applicant_user_id: Option<&str>,
    ) -> anyhow::Result<Vec<crate::recruitment::RecruitmentApplication>> {
        validate_snowflake("guild_id", guild_id)?;
        let mut list: Vec<_> = self
            .recruitment_applications
            .iter()
            .filter(|a| {
                a.guild_id == guild_id
                    && track_id.is_none_or(|tid| a.track_id == tid)
                    && status.is_none_or(|st| a.status == st)
                    && applicant_user_id.is_none_or(|uid| a.applicant_user_id == uid)
            })
            .map(|a| a.value().clone())
            .collect();
        list.sort_by_key(|a| std::cmp::Reverse(a.submitted_at));
        Ok(list)
    }

    pub fn create_recruitment_application(
        &self,
        mut app: crate::recruitment::RecruitmentApplication,
    ) -> anyhow::Result<crate::recruitment::RecruitmentApplication> {
        validate_snowflake("guild_id", &app.guild_id)?;
        validate_snowflake("applicant_user_id", &app.applicant_user_id)?;
        let now = Utc::now();
        app.submitted_at = now;
        app.updated_at = now;
        self.recruitment_applications.insert(app.id, app.clone());
        Ok(app)
    }

    pub fn create_recruitment_application_checked(
        &self,
        mut app: crate::recruitment::RecruitmentApplication,
        max_active: u32,
    ) -> anyhow::Result<crate::recruitment::RecruitmentApplication> {
        validate_snowflake("guild_id", &app.guild_id)?;
        validate_snowflake("applicant_user_id", &app.applicant_user_id)?;

        let _guard = self
            .recruitment_lock
            .lock()
            .map_err(|e| anyhow::anyhow!("Recruitment lock error: {e}"))?;

        let active_count = self
            .recruitment_applications
            .iter()
            .filter(|a| {
                a.guild_id == app.guild_id
                    && a.applicant_user_id == app.applicant_user_id
                    && a.track_id == app.track_id
                    && ["draft", "submitted", "screening", "interview"].contains(&a.status.as_str())
            })
            .count() as u32;

        if active_count >= max_active {
            anyhow::bail!(
                "Active applications limit ({}) exceeded for user",
                max_active
            );
        }

        let now = Utc::now();
        app.submitted_at = now;
        app.updated_at = now;
        self.recruitment_applications.insert(app.id, app.clone());
        Ok(app)
    }

    pub fn update_recruitment_application_transition(
        &self,
        params: crate::recruitment::TransitionApplicationParams<'_>,
    ) -> anyhow::Result<crate::recruitment::RecruitmentApplication> {
        validate_snowflake("guild_id", params.guild_id)?;
        let mut entry = self
            .recruitment_applications
            .get_mut(&params.id)
            .ok_or_else(|| anyhow::anyhow!("Application not found"))?;
        if entry.guild_id != params.guild_id {
            anyhow::bail!("Application not found in guild");
        }
        if entry.version != params.expected_version {
            anyhow::bail!(
                "Optimistic concurrency conflict: application was modified by another reviewer"
            );
        }

        entry.status = params.new_status.to_string();
        entry.current_stage = params.new_stage.to_string();
        entry.version += 1;
        entry.updated_at = Utc::now();
        if params.new_status == "accepted"
            || params.new_status == "rejected"
            || params.new_status == "withdrawn"
        {
            entry.decided_at = Some(entry.updated_at);
            entry.decided_by = params.decided_by.map(|s| s.to_string());
            entry.decision_reason = params.decision_reason.map(|s| s.to_string());
        }

        Ok(entry.value().clone())
    }

    pub fn accept_recruitment_application_with_role_enqueue(
        &self,
        params: crate::recruitment::TransitionApplicationParams<'_>,
        target_role_id: Option<&str>,
    ) -> anyhow::Result<(
        crate::recruitment::RecruitmentApplication,
        Option<crate::recruitment::RecruitmentRoleFulfillment>,
    )> {
        validate_snowflake("guild_id", params.guild_id)?;
        if let Some(rid) = target_role_id {
            validate_snowflake("target_role_id", rid)?;
        }

        let _guard = self
            .recruitment_lock
            .lock()
            .map_err(|e| anyhow::anyhow!("Recruitment lock error: {e}"))?;

        let mut entry = self
            .recruitment_applications
            .get_mut(&params.id)
            .ok_or_else(|| anyhow::anyhow!("Application not found"))?;
        if entry.guild_id != params.guild_id {
            anyhow::bail!("Application not found in guild");
        }
        if entry.version != params.expected_version {
            anyhow::bail!(
                "Optimistic concurrency conflict: application was modified by another reviewer"
            );
        }

        entry.status = "accepted".to_string();
        entry.current_stage = params.new_stage.to_string();
        entry.version += 1;
        entry.updated_at = Utc::now();
        entry.decided_at = Some(entry.updated_at);
        entry.decided_by = params.decided_by.map(|s| s.to_string());
        entry.decision_reason = params.decision_reason.map(|s| s.to_string());
        let updated_app = entry.value().clone();

        let mut fulfillment = None;
        if let Some(role_id) = target_role_id {
            let f = crate::recruitment::RecruitmentRoleFulfillment {
                application_id: updated_app.id,
                guild_id: updated_app.guild_id.clone(),
                user_id: updated_app.applicant_user_id.clone(),
                role_id: role_id.to_string(),
                status: "pending".to_string(),
                attempts: 0,
                last_error: None,
                next_attempt_at: Utc::now(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            self.recruitment_role_fulfillments
                .insert(f.application_id, f.clone());
            fulfillment = Some(f);
        }

        Ok((updated_app, fulfillment))
    }

    pub fn cancel_pending_role_fulfillments_for_application(
        &self,
        application_id: Uuid,
    ) -> anyhow::Result<usize> {
        let mut cancelled = 0;
        if let Some(mut entry) = self.recruitment_role_fulfillments.get_mut(&application_id)
            && entry.status == "pending"
        {
            entry.status = "cancelled".to_string();
            entry.updated_at = Utc::now();
            cancelled = 1;
        }
        Ok(cancelled)
    }

    pub fn count_active_user_applications(
        &self,
        guild_id: &str,
        user_id: &str,
        track_id: Uuid,
    ) -> anyhow::Result<u32> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("user_id", user_id)?;
        let count = self
            .recruitment_applications
            .iter()
            .filter(|a| {
                a.guild_id == guild_id
                    && a.applicant_user_id == user_id
                    && a.track_id == track_id
                    && ["draft", "submitted", "screening", "interview"].contains(&a.status.as_str())
            })
            .count();
        Ok(count as u32)
    }

    pub fn list_recruitment_reviews(
        &self,
        application_id: Uuid,
    ) -> anyhow::Result<Vec<crate::recruitment::RecruitmentReview>> {
        let mut list: Vec<_> = self
            .recruitment_reviews
            .iter()
            .filter(|r| r.application_id == application_id)
            .map(|r| r.value().clone())
            .collect();
        list.sort_by_key(|a| a.created_at);
        Ok(list)
    }

    pub fn list_recruitment_reviews_for_applications(
        &self,
        application_ids: &[Uuid],
    ) -> anyhow::Result<HashMap<Uuid, Vec<crate::recruitment::RecruitmentReview>>> {
        if application_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let target_set: HashSet<Uuid> = application_ids.iter().copied().collect();
        let mut map: HashMap<Uuid, Vec<crate::recruitment::RecruitmentReview>> = HashMap::new();
        for entry in self.recruitment_reviews.iter() {
            if target_set.contains(&entry.application_id) {
                map.entry(entry.application_id)
                    .or_default()
                    .push(entry.value().clone());
            }
        }
        for reviews in map.values_mut() {
            reviews.sort_by_key(|a| a.created_at);
        }
        Ok(map)
    }

    pub fn get_role_fulfillments_for_applications(
        &self,
        application_ids: &[Uuid],
    ) -> anyhow::Result<HashMap<Uuid, crate::recruitment::RecruitmentRoleFulfillment>> {
        if application_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let target_set: HashSet<Uuid> = application_ids.iter().copied().collect();
        let mut map = HashMap::new();
        for entry in self.recruitment_role_fulfillments.iter() {
            if target_set.contains(&entry.application_id) {
                map.insert(entry.application_id, entry.value().clone());
            }
        }
        Ok(map)
    }

    pub fn upsert_recruitment_review(
        &self,
        mut review: crate::recruitment::RecruitmentReview,
    ) -> anyhow::Result<crate::recruitment::RecruitmentReview> {
        validate_snowflake("guild_id", &review.guild_id)?;
        validate_snowflake("reviewer_user_id", &review.reviewer_user_id)?;
        review.updated_at = Utc::now();
        if let Some(existing) = self.recruitment_reviews.iter().find(|r| {
            r.application_id == review.application_id
                && r.reviewer_user_id == review.reviewer_user_id
        }) {
            review.id = existing.id;
        }
        self.recruitment_reviews.insert(review.id, review.clone());
        Ok(review)
    }

    pub fn get_role_fulfillment(
        &self,
        application_id: Uuid,
    ) -> anyhow::Result<Option<crate::recruitment::RecruitmentRoleFulfillment>> {
        Ok(self
            .recruitment_role_fulfillments
            .get(&application_id)
            .map(|f| f.value().clone()))
    }

    pub fn upsert_role_fulfillment(
        &self,
        mut fulfillment: crate::recruitment::RecruitmentRoleFulfillment,
    ) -> anyhow::Result<crate::recruitment::RecruitmentRoleFulfillment> {
        validate_snowflake("guild_id", &fulfillment.guild_id)?;
        validate_snowflake("user_id", &fulfillment.user_id)?;
        validate_snowflake("role_id", &fulfillment.role_id)?;
        fulfillment.updated_at = Utc::now();
        self.recruitment_role_fulfillments
            .insert(fulfillment.application_id, fulfillment.clone());
        Ok(fulfillment)
    }

    pub fn list_pending_role_fulfillments(
        &self,
        guild_id: Option<&str>,
    ) -> anyhow::Result<Vec<crate::recruitment::RecruitmentRoleFulfillment>> {
        if let Some(gid) = guild_id {
            validate_snowflake("guild_id", gid)?;
        }
        let now = Utc::now();
        let list: Vec<_> = self
            .recruitment_role_fulfillments
            .iter()
            .filter(|f| {
                f.status == "pending"
                    && f.next_attempt_at <= now
                    && guild_id.is_none_or(|gid| f.guild_id == gid)
            })
            .map(|f| f.value().clone())
            .collect();
        Ok(list)
    }

    pub fn get_help_forum_config(
        &self,
        guild_id: &str,
        forum_channel_id: &str,
    ) -> anyhow::Result<Option<crate::help_forum::HelpForumConfig>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("forum_channel_id", forum_channel_id)?;
        Ok(self
            .help_forum_configs
            .get(&(guild_id.to_string(), forum_channel_id.to_string()))
            .map(|c| c.value().clone()))
    }

    pub fn list_help_forum_configs(
        &self,
        guild_id: &str,
    ) -> anyhow::Result<Vec<crate::help_forum::HelpForumConfig>> {
        validate_snowflake("guild_id", guild_id)?;
        let mut list: Vec<_> = self
            .help_forum_configs
            .iter()
            .filter(|c| c.guild_id == guild_id)
            .map(|c| c.value().clone())
            .collect();
        list.sort_by_key(|a| a.created_at);
        Ok(list)
    }

    pub fn list_enabled_help_forum_guilds(&self) -> anyhow::Result<Vec<String>> {
        let mut guilds = std::collections::BTreeSet::new();
        for item in self.help_forum_configs.iter() {
            if item.enabled {
                guilds.insert(item.guild_id.clone());
            }
        }
        Ok(guilds.into_iter().collect())
    }

    pub fn upsert_help_forum_config(
        &self,
        mut config: crate::help_forum::HelpForumConfig,
    ) -> anyhow::Result<crate::help_forum::HelpForumConfig> {
        crate::help_forum::validate_help_forum_config(&config)?;
        config.updated_at = Utc::now();
        let key = (config.guild_id.clone(), config.forum_channel_id.clone());
        self.help_forum_configs.insert(key, config.clone());
        Ok(config)
    }

    pub fn delete_help_forum_config(
        &self,
        guild_id: &str,
        forum_channel_id: &str,
    ) -> anyhow::Result<bool> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("forum_channel_id", forum_channel_id)?;
        let key = (guild_id.to_string(), forum_channel_id.to_string());
        Ok(self.help_forum_configs.remove(&key).is_some())
    }

    pub fn get_help_forum_session(
        &self,
        guild_id: &str,
        thread_id: &str,
    ) -> anyhow::Result<Option<crate::help_forum::HelpForumSession>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("thread_id", thread_id)?;
        let key = (guild_id.to_string(), thread_id.to_string());
        Ok(self
            .help_forum_sessions
            .get(&key)
            .map(|s| s.value().clone()))
    }

    pub fn list_help_forum_sessions(
        &self,
        guild_id: &str,
        forum_channel_id: Option<&str>,
        status: Option<&str>,
    ) -> anyhow::Result<Vec<crate::help_forum::HelpForumSession>> {
        validate_snowflake("guild_id", guild_id)?;
        if let Some(fid) = forum_channel_id {
            validate_snowflake("forum_channel_id", fid)?;
        }
        let mut list: Vec<_> = self
            .help_forum_sessions
            .iter()
            .filter(|s| {
                s.guild_id == guild_id
                    && forum_channel_id.is_none_or(|f| s.forum_channel_id == f)
                    && status.is_none_or(|st| s.status == st)
            })
            .map(|s| s.value().clone())
            .collect();
        list.sort_by_key(|a| std::cmp::Reverse(a.opened_at));
        Ok(list)
    }

    pub fn create_help_forum_session(
        &self,
        mut session: crate::help_forum::HelpForumSession,
    ) -> anyhow::Result<crate::help_forum::HelpForumSession> {
        validate_snowflake("guild_id", &session.guild_id)?;
        validate_snowflake("forum_channel_id", &session.forum_channel_id)?;
        validate_snowflake("thread_id", &session.thread_id)?;
        validate_snowflake("owner_user_id", &session.owner_user_id)?;

        let now = Utc::now();
        if session.opened_at == DateTime::<Utc>::UNIX_EPOCH {
            session.opened_at = now;
        }
        if session.last_activity_at == DateTime::<Utc>::UNIX_EPOCH {
            session.last_activity_at = now;
        }
        if session.generation == 0 {
            session.generation = 1;
        }
        if session.status.is_empty() {
            session.status = "open".to_string();
        }
        let key = (session.guild_id.clone(), session.thread_id.clone());
        self.help_forum_sessions.insert(key, session.clone());
        Ok(session)
    }

    pub fn record_help_forum_activity(
        &self,
        guild_id: &str,
        thread_id: &str,
        author_user_id: &str,
        is_helper: bool,
    ) -> anyhow::Result<Option<crate::help_forum::HelpForumSession>> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("thread_id", thread_id)?;
        validate_snowflake("author_user_id", author_user_id)?;

        let key = (guild_id.to_string(), thread_id.to_string());
        let mut entry = match self.help_forum_sessions.get_mut(&key) {
            Some(e) => e,
            None => return Ok(None),
        };

        if entry.status != "open" {
            return Ok(Some(entry.value().clone()));
        }

        let now = Utc::now();
        entry.generation += 1;
        entry.last_activity_at = now;

        if is_helper
            && author_user_id != entry.owner_user_id
            && entry.first_helper_response_at.is_none()
        {
            entry.first_helper_response_at = Some(now);
        }

        Ok(Some(entry.value().clone()))
    }

    pub fn close_help_forum_session(
        &self,
        guild_id: &str,
        thread_id: &str,
        new_status: &str,
        reason: Option<&str>,
        expected_generation: Option<i64>,
    ) -> anyhow::Result<bool> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("thread_id", thread_id)?;

        let key = (guild_id.to_string(), thread_id.to_string());
        let mut entry = match self.help_forum_sessions.get_mut(&key) {
            Some(e) => e,
            None => return Ok(false),
        };

        if entry.status != "open" && entry.status != "needs_tag" {
            return Ok(false);
        }

        if expected_generation.is_some_and(|exp_gen| entry.generation != exp_gen) {
            return Ok(false);
        }

        entry.status = new_status.to_string();
        entry.closed_at = Some(Utc::now());
        entry.closing_reason = reason.map(|s| s.to_string());
        Ok(true)
    }

    pub fn mark_help_forum_owner_left(
        &self,
        guild_id: &str,
        owner_user_id: &str,
    ) -> anyhow::Result<usize> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("owner_user_id", owner_user_id)?;

        let mut changed = 0;
        let now = Utc::now();
        for mut item in self.help_forum_sessions.iter_mut() {
            if item.guild_id == guild_id
                && item.owner_user_id == owner_user_id
                && item.status == "open"
            {
                item.status = "owner_left".to_string();
                item.closed_at = Some(now);
                item.closing_reason = Some("owner_left".to_string());
                changed += 1;
            }
        }
        Ok(changed)
    }

    pub fn get_help_forum_stats(
        &self,
        guild_id: &str,
        forum_channel_id: &str,
    ) -> anyhow::Result<crate::help_forum::HelpForumStats> {
        validate_snowflake("guild_id", guild_id)?;
        validate_snowflake("forum_channel_id", forum_channel_id)?;

        let sessions: Vec<_> = self
            .help_forum_sessions
            .iter()
            .filter(|s| s.guild_id == guild_id && s.forum_channel_id == forum_channel_id)
            .map(|s| s.value().clone())
            .collect();

        Ok(crate::help_forum::calculate_stats_from_sessions(
            forum_channel_id,
            &sessions,
        ))
    }

    pub fn record_asset_usage_batch(
        &self,
        daily_items: &[crate::asset_analytics::AssetUsageDaily],
        inventory_items: &[crate::asset_analytics::AssetInventoryItem],
    ) -> anyhow::Result<usize> {
        if self
            .asset_analytics_failure_injected
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            anyhow::bail!("Injected database error during asset analytics batch flush");
        }

        let mut count = 0;
        for item in daily_items {
            validate_snowflake("guild_id", &item.guild_id)?;
            validate_snowflake("asset_id", &item.asset_id)?;
            let key = (
                item.guild_id.clone(),
                item.usage_date,
                item.asset_kind.clone(),
                item.asset_id.clone(),
            );
            self.asset_usage_daily
                .entry(key)
                .and_modify(|existing| {
                    existing.usage_count += item.usage_count;
                    if item.last_used_at > existing.last_used_at {
                        existing.last_used_at = item.last_used_at;
                    }
                })
                .or_insert_with(|| item.clone());
            count += 1;
        }

        for item in inventory_items {
            validate_snowflake("guild_id", &item.guild_id)?;
            validate_snowflake("asset_id", &item.asset_id)?;
            let key = (
                item.guild_id.clone(),
                item.asset_kind.clone(),
                item.asset_id.clone(),
            );
            self.asset_inventory
                .entry(key)
                .and_modify(|existing| {
                    if item.name.is_some() {
                        existing.name = item.name.clone();
                    }
                    existing.animated = item.animated;
                    if item.is_guild_owned.is_some() {
                        existing.is_guild_owned = item.is_guild_owned;
                    }
                    // Issue #201: DO NOT overwrite existing.is_active from delayed usage batch
                    if item.last_seen_at > existing.last_seen_at {
                        existing.last_seen_at = item.last_seen_at;
                    }
                    if item.first_seen_at < existing.first_seen_at {
                        existing.first_seen_at = item.first_seen_at;
                    }
                })
                .or_insert_with(|| item.clone());
        }

        Ok(count)
    }

    pub fn reconcile_guild_inventory(
        &self,
        guild_id: &str,
        kind: crate::asset_analytics::AssetKind,
        active_items: &[crate::asset_analytics::AssetInventoryItem],
    ) -> anyhow::Result<usize> {
        validate_snowflake("guild_id", guild_id)?;
        let kind_str = kind.as_str();
        let mut active_set = std::collections::HashSet::new();

        for item in active_items {
            validate_snowflake("asset_id", &item.asset_id)?;
            active_set.insert(item.asset_id.clone());

            let key = (
                guild_id.to_string(),
                kind_str.to_string(),
                item.asset_id.clone(),
            );
            self.asset_inventory
                .entry(key)
                .and_modify(|existing| {
                    if item.name.is_some() {
                        existing.name = item.name.clone();
                    }
                    existing.animated = item.animated;
                    existing.is_guild_owned = Some(true);
                    existing.is_active = true;
                    if item.last_seen_at > existing.last_seen_at {
                        existing.last_seen_at = item.last_seen_at;
                    }
                    if item.first_seen_at < existing.first_seen_at {
                        existing.first_seen_at = item.first_seen_at;
                    }
                })
                .or_insert_with(|| {
                    let mut i = item.clone();
                    i.is_guild_owned = Some(true);
                    i.is_active = true;
                    i
                });
        }

        let mut deactivated = 0;
        for mut entry in self.asset_inventory.iter_mut() {
            let item = entry.value_mut();
            if item.guild_id == guild_id
                && item.asset_kind == kind_str
                && item.is_guild_owned == Some(true)
                && !active_set.contains(&item.asset_id)
                && item.is_active
            {
                item.is_active = false;
                deactivated += 1;
            }
        }

        Ok(deactivated)
    }

    pub fn get_asset_usage_summary(
        &self,
        guild_id: &str,
        kind: Option<crate::asset_analytics::AssetKind>,
        period: &str,
    ) -> anyhow::Result<crate::asset_analytics::AssetUsageSummary> {
        validate_snowflake("guild_id", guild_id)?;

        let daily_rows: Vec<crate::asset_analytics::AssetUsageDaily> = self
            .asset_usage_daily
            .iter()
            .filter(|entry| entry.guild_id == guild_id)
            .map(|entry| entry.value().clone())
            .collect();

        let inventory_rows: Vec<crate::asset_analytics::AssetInventoryItem> = self
            .asset_inventory
            .iter()
            .filter(|entry| entry.guild_id == guild_id)
            .map(|entry| entry.value().clone())
            .collect();

        Ok(crate::asset_analytics::calculate_asset_summary(
            guild_id,
            kind,
            period,
            &daily_rows,
            &inventory_rows,
        ))
    }

    pub fn sync_guild_inventory_active_state(
        &self,
        guild_id: &str,
        kind: crate::asset_analytics::AssetKind,
        active_ids: &[String],
    ) -> anyhow::Result<usize> {
        validate_snowflake("guild_id", guild_id)?;
        let active_set: std::collections::HashSet<&str> =
            active_ids.iter().map(|s| s.as_str()).collect();

        let mut count = 0;
        let kind_str = kind.as_str();
        for mut entry in self.asset_inventory.iter_mut() {
            let item = entry.value_mut();
            if item.guild_id == guild_id
                && item.asset_kind == kind_str
                && item.is_guild_owned == Some(true)
            {
                let is_active = active_set.contains(item.asset_id.as_str());
                if item.is_active != is_active {
                    item.is_active = is_active;
                    count += 1;
                }
            }
        }

        Ok(count)
    }

    pub fn list_asset_inventory(
        &self,
        guild_id: &str,
        kind: Option<crate::asset_analytics::AssetKind>,
    ) -> anyhow::Result<Vec<crate::asset_analytics::AssetUsageStat>> {
        validate_snowflake("guild_id", guild_id)?;
        let summary = self.get_asset_usage_summary(guild_id, kind, "all")?;
        Ok(summary.items)
    }

    pub fn prune_asset_analytics(
        &self,
        guild_id: Option<&str>,
        older_than_days: u32,
    ) -> anyhow::Result<usize> {
        if let Some(gid) = guild_id {
            validate_snowflake("guild_id", gid)?;
        }

        let cutoff =
            chrono::Utc::now().date_naive() - chrono::Duration::days(older_than_days as i64);
        let mut pruned = 0;

        self.asset_usage_daily.retain(|(gid, date, _, _), _| {
            let matches_guild = match guild_id {
                Some(target) => gid == target,
                None => true,
            };
            if matches_guild && *date < cutoff {
                pruned += 1;
                false
            } else {
                true
            }
        });

        Ok(pruned)
    }
}
