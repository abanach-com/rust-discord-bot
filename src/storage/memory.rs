use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use chrono::{Duration, Utc};
use dashmap::{DashMap, DashSet};
use serde_json::Value;
use uuid::Uuid;

use super::{
    AdjustMemberProgressParams, AuditLogEntry, CardCollection, CardTrade, CoinTransfer,
    CoinTransferOutcome, CommunityPoll, CountingGame, CustomAutomation, DailyClaimOutcome,
    DynamicVoiceRoom, EventRoster, FaqEntry, FeedSubscription, GameAccountLink, Giveaway,
    GuildModuleSettings, InviteTracking, MemberProgress, ModerationAppeal,
    ModerationExecutionIntent, ModuleDraftConfig, ProgressMetric, ProgressionSeason,
    ReputationGrantOutcome, ScheduledEvent, ServerSnapshot, StarboardEntry, StickyMessage,
    StoreItem, StorePurchase, TimedRoleGrant, Tournament, TupperProfile, XpAward, level_for_xp,
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
    store_items: Arc<DashMap<Uuid, StoreItem>>,
    store_purchases: Arc<DashMap<Uuid, StorePurchase>>,
    progression_seasons: Arc<DashMap<Uuid, ProgressionSeason>>,
    events: Arc<DashMap<Uuid, ScheduledEvent>>,
    rosters: Arc<DashMap<Uuid, Vec<EventRoster>>>,
    tournaments: Arc<DashMap<Uuid, Tournament>>,
    invites: Arc<DashMap<(String, String), InviteTracking>>,
    dynamic_voice: Arc<DashMap<String, DynamicVoiceRoom>>,
    feeds: Arc<DashMap<Uuid, FeedSubscription>>,
    game_links: Arc<DashMap<Uuid, GameAccountLink>>,
    counting_games: Arc<DashMap<(String, String), CountingGame>>,
    cards: Arc<DashMap<Uuid, CardCollection>>,
    card_trades: Arc<DashMap<Uuid, CardTrade>>,
    tuppers: Arc<DashMap<Uuid, TupperProfile>>,
    faqs: Arc<DashMap<Uuid, FaqEntry>>,
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
        let mut entries: Vec<AuditLogEntry> = self
            .audit_log
            .iter()
            .filter_map(|entry| {
                let val = entry.value();
                let gid = val.get("guild_id").and_then(|v| v.as_str())?;
                if gid != guild_id {
                    return None;
                }
                serde_json::from_value(val.clone()).ok()
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
        appeal_id: Uuid,
        status: &str,
        staff_notes: Option<&str>,
    ) -> bool {
        if let Some(mut entry) = self.appeals.get_mut(&appeal_id) {
            let appeal = entry.value_mut();
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

    pub fn delete_server_snapshot(&self, snapshot_id: Uuid) -> bool {
        self.server_snapshots.remove(&snapshot_id).is_some()
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

    pub fn delete_custom_automation(&self, automation_id: Uuid) -> bool {
        self.custom_automations.remove(&automation_id).is_some()
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

    pub fn vote_community_poll(&self, poll_id: Uuid, option_idx: usize) -> Option<CommunityPoll> {
        let mut poll = self.polls.get_mut(&poll_id)?;
        if let Some(val) = poll
            .tallies_json
            .as_array_mut()
            .and_then(|t| t.get_mut(option_idx))
        {
            let count = val.as_i64().unwrap_or(0);
            *val = serde_json::json!(count + 1);
        }
        Some(poll.clone())
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
        if ga.ended {
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
        let mut winners = Vec::new();
        for i in 0..count {
            winners.push(
                participants
                    [(i + (Utc::now().timestamp_subsec_millis() as usize)) % participants.len()]
                .clone(),
            );
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
    ) -> anyhow::Result<(StoreItem, i64)> {
        let _guard = self.progression_guard();
        let mut item = self
            .store_items
            .get_mut(&item_id)
            .ok_or_else(|| anyhow::anyhow!("store item not found"))?;
        if item.guild_id != guild_id {
            return Err(anyhow::anyhow!("item does not belong to this guild"));
        }
        if let Some(stock) = item.stock {
            if stock <= 0 {
                return Err(anyhow::anyhow!("item is out of stock"));
            }
            item.stock = Some(stock - 1);
        }
        let price = item.price;
        let cloned_item = item.clone();
        drop(item);

        let now = Utc::now();
        let mut profile = self.profile(guild_id, user_id, now);
        if profile.coins < price {
            return Err(anyhow::anyhow!("insufficient coins balance"));
        }
        profile.coins -= price;
        profile.updated_at = now;
        let remaining = profile.coins;
        self.store_profile(profile);

        let purchase = StorePurchase {
            id: Uuid::new_v4(),
            guild_id: guild_id.to_owned(),
            user_id: user_id.to_owned(),
            item_id,
            price_paid: price,
            created_at: Utc::now(),
        };
        self.store_purchases.insert(purchase.id, purchase);
        Ok((cloned_item, remaining))
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

    pub fn join_event_roster(&self, event_id: Uuid, user_id: &str, max_main: i32) -> EventRoster {
        let mut roster_list = self.rosters.entry(event_id).or_default();
        if let Some(existing) = roster_list.iter().find(|r| r.user_id == user_id) {
            return existing.clone();
        }
        let main_count = roster_list.iter().filter(|r| r.role_type == "main").count() as i32;
        let role_type = if main_count < max_main {
            "main"
        } else {
            "reserve"
        };
        let position = roster_list.len() as i32 + 1;
        let entry = EventRoster {
            event_id,
            user_id: user_id.to_owned(),
            role_type: role_type.to_owned(),
            position,
            joined_at: Utc::now(),
        };
        roster_list.push(entry.clone());
        entry
    }

    pub fn leave_event_roster(&self, event_id: Uuid, user_id: &str) -> Option<String> {
        let mut roster_list = self.rosters.get_mut(&event_id)?;
        let index = roster_list.iter().position(|r| r.user_id == user_id)?;
        let removed = roster_list.remove(index);
        if removed.role_type == "main" {
            let promoted = roster_list
                .iter_mut()
                .find(|r| r.role_type == "reserve")
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
        tournament_id: Uuid,
        match_idx: usize,
        winner: &str,
    ) -> anyhow::Result<Tournament> {
        let mut tourney = self
            .tournaments
            .get_mut(&tournament_id)
            .ok_or_else(|| anyhow::anyhow!("tournament not found"))?;
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

    pub fn update_dynamic_voice_empty_state(&self, channel_id: &str, is_empty: bool) {
        if let Some(mut room) = self.dynamic_voice.get_mut(channel_id) {
            room.empty_since = if is_empty { Some(Utc::now()) } else { None };
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
        self.game_links.insert(id, link.clone());
        link
    }

    pub fn verify_game_link(
        &self,
        guild_id: &str,
        user_id: &str,
        platform: &str,
        code: &str,
    ) -> bool {
        for mut link in self.game_links.iter_mut() {
            if link.guild_id == guild_id
                && link.user_id == user_id
                && link.platform == platform
                && link.verification_code.as_deref() == Some(code)
            {
                link.verified = true;
                return true;
            }
        }
        false
    }

    pub fn get_game_links(&self, guild_id: &str, user_id: &str) -> Vec<GameAccountLink> {
        self.game_links
            .iter()
            .filter(|e| e.guild_id == guild_id && e.user_id == user_id)
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

        let mut c1 = self
            .cards
            .get_mut(&trade.initiator_card_id)
            .ok_or_else(|| anyhow::anyhow!("initiator card not found"))?;
        let mut c2 = self
            .cards
            .get_mut(&trade.target_card_id)
            .ok_or_else(|| anyhow::anyhow!("target card not found"))?;

        if c1.user_id != trade.initiator_user_id || c2.user_id != trade.target_user_id {
            return Err(anyhow::anyhow!("ownership changed before trade completion"));
        }

        c1.user_id = trade.target_user_id.clone();
        c2.user_id = trade.initiator_user_id.clone();
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
    ) -> TupperProfile {
        let id = Uuid::new_v4();
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
        profile
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

    pub fn list_faq_entries(&self, guild_id: &str, query: Option<&str>) -> Vec<FaqEntry> {
        let q = query.map(|s| s.to_lowercase());
        self.faqs
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
            .collect()
    }

    pub fn delete_faq_entry(&self, faq_id: Uuid) -> bool {
        self.faqs.remove(&faq_id).is_some()
    }
}
