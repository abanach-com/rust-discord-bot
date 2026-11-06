use std::str::FromStr;

use chrono::{DateTime, Datelike, NaiveTime, Utc, Weekday};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::storage::validate_snowflake;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DutyTimeRange {
    pub start: String, // "HH:MM"
    pub end: String,   // "HH:MM"
}

impl DutyTimeRange {
    pub fn new(start: &str, end: &str) -> anyhow::Result<Self> {
        NaiveTime::parse_from_str(start, "%H:%M").map_err(|_| {
            anyhow::anyhow!("Invalid start time format (expected HH:MM): {}", start)
        })?;
        NaiveTime::parse_from_str(end, "%H:%M")
            .map_err(|_| anyhow::anyhow!("Invalid end time format (expected HH:MM): {}", end))?;
        Ok(Self {
            start: start.to_string(),
            end: end.to_string(),
        })
    }

    pub fn is_overnight(&self) -> bool {
        if let (Ok(s), Ok(e)) = (
            NaiveTime::parse_from_str(&self.start, "%H:%M"),
            NaiveTime::parse_from_str(&self.end, "%H:%M"),
        ) {
            s > e
        } else {
            false
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecurringSchedule {
    #[serde(default)]
    pub monday: Vec<DutyTimeRange>,
    #[serde(default)]
    pub tuesday: Vec<DutyTimeRange>,
    #[serde(default)]
    pub wednesday: Vec<DutyTimeRange>,
    #[serde(default)]
    pub thursday: Vec<DutyTimeRange>,
    #[serde(default)]
    pub friday: Vec<DutyTimeRange>,
    #[serde(default)]
    pub saturday: Vec<DutyTimeRange>,
    #[serde(default)]
    pub sunday: Vec<DutyTimeRange>,
}

impl RecurringSchedule {
    pub fn ranges_for_weekday(&self, weekday: Weekday) -> &[DutyTimeRange] {
        match weekday {
            Weekday::Mon => &self.monday,
            Weekday::Tue => &self.tuesday,
            Weekday::Wed => &self.wednesday,
            Weekday::Thu => &self.thursday,
            Weekday::Fri => &self.friday,
            Weekday::Sat => &self.saturday,
            Weekday::Sun => &self.sunday,
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        for day in [
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
            Weekday::Sat,
            Weekday::Sun,
        ] {
            for (index, range) in self.ranges_for_weekday(day).iter().enumerate() {
                DutyTimeRange::new(&range.start, &range.end)
                    .map_err(|error| anyhow::anyhow!("{day:?}[{index}]: {error}"))?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ModeratorDutySettings {
    pub guild_id: String,
    pub staff_role_id: Option<String>,
    pub on_call_role_id: Option<String>,
    pub max_override_duration_mins: i32,
    pub fallback_channel_id: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ModeratorDutyProfile {
    pub guild_id: String,
    pub user_id: String,
    pub timezone: String,
    pub recurring_schedule: sqlx::types::Json<RecurringSchedule>,
    pub manual_override: Option<String>,
    pub override_expires_at: Option<DateTime<Utc>>,
    pub version: i64,
    pub updated_at: DateTime<Utc>,
}

impl ModeratorDutyProfile {
    pub fn new(
        guild_id: String,
        user_id: String,
        timezone: String,
        schedule: RecurringSchedule,
    ) -> anyhow::Result<Self> {
        validate_snowflake("guild_id", &guild_id)?;
        validate_snowflake("user_id", &user_id)?;
        let _ = Tz::from_str(&timezone)
            .map_err(|_| anyhow::anyhow!("Invalid IANA timezone: {}", timezone))?;
        schedule.validate()?;

        let now = Utc::now();
        Ok(Self {
            guild_id,
            user_id,
            timezone,
            recurring_schedule: sqlx::types::Json(schedule),
            manual_override: None,
            override_expires_at: None,
            version: 0,
            updated_at: now,
        })
    }

    pub fn schedule_error(&self) -> Option<String> {
        self.recurring_schedule
            .validate()
            .err()
            .map(|e| e.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct ModeratorDutyState {
    pub guild_id: String,
    pub user_id: String,
    pub expected_on_call: bool,
    pub state_generation: i64,
    pub last_reconciled_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StaffAudience {
    OnCall,
    Moderators,
    Administrators,
}

/// Computes whether a moderator should currently have the On-Call role.
/// Rules:
/// 1. If user is NOT staff, result is strictly false.
/// 2. If manual_override is active and not expired:
///    - 'force_on' -> true
///    - 'force_off' -> false
/// 3. Otherwise evaluate recurring schedule in moderator's local timezone (handling DST & overnight shifts).
pub fn compute_is_on_call(
    profile: &ModeratorDutyProfile,
    now: DateTime<Utc>,
    is_staff: bool,
) -> bool {
    if !is_staff {
        return false;
    }

    // 2. Check active manual override
    if let Some(ref override_type) = profile.manual_override {
        let is_valid = match profile.override_expires_at {
            Some(exp) => now < exp,
            None => true,
        };

        if is_valid {
            match override_type.as_str() {
                "force_on" => return true,
                "force_off" => return false,
                _ => {}
            }
        }
    }

    // 3. Evaluate recurring schedule
    let tz = Tz::from_str(&profile.timezone).unwrap_or(chrono_tz::UTC);
    let local_now = now.with_timezone(&tz);
    let current_weekday = local_now.weekday();
    let current_time = local_now.time();

    // A. Check today's scheduled shifts
    let today_ranges = profile
        .recurring_schedule
        .ranges_for_weekday(current_weekday);
    for range in today_ranges {
        match (
            NaiveTime::parse_from_str(&range.start, "%H:%M"),
            NaiveTime::parse_from_str(&range.end, "%H:%M"),
        ) {
            (Ok(start), Ok(end)) => {
                if start <= end {
                    // Same-day shift: e.g. 18:00 - 22:00
                    if current_time >= start && current_time < end {
                        return true;
                    }
                } else {
                    // Overnight shift starting today: e.g. 22:00 - 02:00
                    if current_time >= start {
                        return true;
                    }
                }
            }
            _ => {
                tracing::warn!(
                    guild_id = %profile.guild_id,
                    user_id = %profile.user_id,
                    weekday = ?current_weekday,
                    start = %range.start,
                    end = %range.end,
                    "ignoring invalid duty schedule time range in stored profile"
                );
            }
        }
    }

    // B. Check yesterday's overnight shifts that carry over into today
    let yesterday_weekday = current_weekday.pred();
    let yesterday_ranges = profile
        .recurring_schedule
        .ranges_for_weekday(yesterday_weekday);
    for range in yesterday_ranges {
        match (
            NaiveTime::parse_from_str(&range.start, "%H:%M"),
            NaiveTime::parse_from_str(&range.end, "%H:%M"),
        ) {
            (Ok(start), Ok(end)) => {
                if start > end && current_time < end {
                    return true;
                }
            }
            _ => {
                tracing::warn!(
                    guild_id = %profile.guild_id,
                    user_id = %profile.user_id,
                    weekday = ?yesterday_weekday,
                    start = %range.start,
                    end = %range.end,
                    "ignoring invalid duty schedule time range in stored profile for overnight carryover"
                );
            }
        }
    }

    false
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct ModeratorDutyRoleGrant {
    pub guild_id: String,
    pub user_id: String,
    pub role_id: String,
    pub status: String, // "active", "revoke_pending", "revoked", "failed"
    pub attempts: i32,
    pub last_error: Option<String>,
    pub next_attempt_at: DateTime<Utc>,
    pub generation: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DutyReconcileOutcome {
    pub user_id: String,
    pub expected_on_call: bool,
    pub role_granted: bool,
    pub role_revoked: bool,
    #[serde(default)]
    pub would_grant: bool,
    #[serde(default)]
    pub would_revoke: bool,
    pub error: Option<String>,
}

pub async fn resolve_staff_audience(
    storage: &crate::storage::Storage,
    guild_id: &str,
    audience: StaffAudience,
) -> anyhow::Result<Option<String>> {
    let settings = storage.get_duty_settings(guild_id).await?;
    match audience {
        StaffAudience::OnCall => {
            let states = storage.list_duty_states(guild_id).await?;
            let has_active_on_call = states.iter().any(|s| s.expected_on_call);
            if has_active_on_call {
                Ok(settings.and_then(|s| s.on_call_role_id))
            } else {
                Ok(settings.and_then(|s| s.staff_role_id.or(s.fallback_channel_id)))
            }
        }
        StaffAudience::Moderators | StaffAudience::Administrators => {
            Ok(settings.and_then(|s| s.staff_role_id))
        }
    }
}

pub struct ReconcileDutyParams<'a> {
    pub storage: &'a crate::storage::Storage,
    pub guild_id: &'a str,
    pub user_id: &'a str,
    pub settings: &'a ModeratorDutySettings,
    pub profile: &'a ModeratorDutyProfile,
    pub member_roles: &'a [String],
    pub bot_highest_role_pos: Option<i64>,
    pub on_call_role_pos: Option<i64>,
    pub is_live_discord: bool,
    pub dry_run: bool,
    pub http: Option<&'a serenity::all::Http>,
}

pub async fn reconcile_member_duty_state(
    params: ReconcileDutyParams<'_>,
) -> anyhow::Result<DutyReconcileOutcome> {
    let is_staff = match params.settings.staff_role_id {
        Some(ref sr) => params.member_roles.contains(sr),
        None => true,
    };

    let expected = compute_is_on_call(params.profile, Utc::now(), is_staff);

    let on_call_role_id = match params.settings.on_call_role_id {
        Some(ref r) => r.clone(),
        None => {
            if !params.dry_run {
                let state = ModeratorDutyState {
                    guild_id: params.guild_id.to_string(),
                    user_id: params.user_id.to_string(),
                    expected_on_call: expected,
                    state_generation: 0,
                    last_reconciled_at: Some(Utc::now()),
                    last_error: Some("No on_call_role_id configured".to_string()),
                };
                let _ = params.storage.upsert_duty_state(state).await?;
            }
            return Ok(DutyReconcileOutcome {
                user_id: params.user_id.to_string(),
                expected_on_call: expected,
                role_granted: false,
                role_revoked: false,
                would_grant: false,
                would_revoke: false,
                error: Some("No on_call_role_id configured".to_string()),
            });
        }
    };

    let has_on_call = params.member_roles.contains(&on_call_role_id);
    let mut role_granted = false;
    let mut role_revoked = false;
    let mut would_grant = false;
    let mut would_revoke = false;
    let mut error = None;

    if expected != has_on_call {
        // Verify role hierarchy
        if let (Some(bot_pos), Some(role_pos)) =
            (params.bot_highest_role_pos, params.on_call_role_pos)
            && bot_pos <= role_pos
        {
            error = Some(
                "Role hierarchy failure: bot role is lower than or equal to on-call role"
                    .to_string(),
            );
        }

        if error.is_none() {
            if params.dry_run {
                // Issue #187: In dry-run mode, never mutate Discord or falsify role_granted/role_revoked
                if expected {
                    would_grant = true;
                } else {
                    would_revoke = true;
                }
            } else if params.is_live_discord {
                if let Some(h) = params.http
                    && let (Ok(g_num), Ok(u_num), Ok(r_num)) = (
                        params.guild_id.parse::<u64>(),
                        params.user_id.parse::<u64>(),
                        on_call_role_id.parse::<u64>(),
                    )
                {
                    let g_id = serenity::all::GuildId::new(g_num);
                    let u_id = serenity::all::UserId::new(u_num);
                    let r_id = serenity::all::RoleId::new(r_num);

                    if expected {
                        if let Err(e) = h
                            .add_member_role(g_id, u_id, r_id, Some("Moderator on-duty scheduled"))
                            .await
                        {
                            error = Some(format!("Discord API add_role error: {e}"));
                        } else {
                            role_granted = true;
                            // Issue #188: Record bot-owned active grant
                            let _ = params
                                .storage
                                .record_duty_role_grant(
                                    params.guild_id,
                                    params.user_id,
                                    &on_call_role_id,
                                    "active",
                                )
                                .await;
                        }
                    } else if let Err(e) = h
                        .remove_member_role(g_id, u_id, r_id, Some("Moderator off-duty scheduled"))
                        .await
                    {
                        error = Some(format!("Discord API remove_role error: {e}"));
                    } else {
                        role_revoked = true;
                        // Issue #188: Mark grant revoked
                        let _ = params
                            .storage
                            .record_duty_role_grant(
                                params.guild_id,
                                params.user_id,
                                &on_call_role_id,
                                "revoked",
                            )
                            .await;
                    }
                }
            } else if expected {
                role_granted = true;
            } else {
                role_revoked = true;
            }
        }
    }

    // Issue #187: In dry_run mode, do NOT mutate ModeratorDutyState in database
    if !params.dry_run {
        let state = ModeratorDutyState {
            guild_id: params.guild_id.to_string(),
            user_id: params.user_id.to_string(),
            expected_on_call: expected,
            state_generation: 0,
            last_reconciled_at: Some(Utc::now()),
            last_error: error.clone(),
        };
        let _ = params.storage.upsert_duty_state(state).await?;
    }

    Ok(DutyReconcileOutcome {
        user_id: params.user_id.to_string(),
        expected_on_call: expected,
        role_granted,
        role_revoked,
        would_grant,
        would_revoke,
        error,
    })
}

pub async fn process_duty_role_revocations(
    storage: &crate::storage::Storage,
    http: Option<&serenity::all::Http>,
    guild_id: Option<&str>,
) -> anyhow::Result<usize> {
    if let Some(gid) = guild_id {
        validate_snowflake("guild_id", gid)?;
    }

    let pending = storage.list_pending_duty_role_revocations(guild_id).await?;
    let mut processed = 0;

    for mut grant in pending {
        if let Some(target_gid) = guild_id
            && grant.guild_id != target_gid
        {
            continue;
        }

        if let Some(client) = http {
            if let (Ok(g_num), Ok(u_num), Ok(r_num)) = (
                grant.guild_id.parse::<u64>(),
                grant.user_id.parse::<u64>(),
                grant.role_id.parse::<u64>(),
            ) {
                let g_id = serenity::all::GuildId::new(g_num);
                let u_id = serenity::all::UserId::new(u_num);
                let r_id = serenity::all::RoleId::new(r_num);

                match client
                    .remove_member_role(
                        g_id,
                        u_id,
                        r_id,
                        Some("Moderator duty role revoked: profile deleted or role changed"),
                    )
                    .await
                {
                    Ok(_) => {
                        grant.status = "revoked".to_string();
                        grant.last_error = None;
                        grant.updated_at = Utc::now();
                        processed += 1;
                    }
                    Err(e) => {
                        let err_msg = e.to_string();
                        grant.attempts += 1;
                        grant.last_error = Some(err_msg);
                        grant.updated_at = Utc::now();
                        if grant.attempts >= 5 {
                            grant.status = "failed".to_string();
                        } else {
                            let backoff_secs = 2u64.pow(grant.attempts.min(6) as u32) * 5;
                            grant.next_attempt_at =
                                Utc::now() + chrono::Duration::seconds(backoff_secs as i64);
                        }
                    }
                }
            } else {
                grant.status = "failed".to_string();
                grant.last_error = Some("Invalid snowflake ID in duty role grant".to_string());
            }
        } else {
            // Offline / mock mode without live Discord client
            continue;
        }

        storage.upsert_duty_role_grant(grant).await?;
    }

    Ok(processed)
}

#[derive(Clone, Debug, Default)]
pub struct DiscordMemberDutyContext {
    pub member_roles: Vec<String>,
    pub bot_highest_role_pos: Option<i64>,
    pub on_call_role_pos: Option<i64>,
}

pub async fn fetch_discord_member_duty_context(
    http: &serenity::all::Http,
    guild_id: &str,
    user_id: &str,
    on_call_role_id: Option<&str>,
) -> anyhow::Result<DiscordMemberDutyContext> {
    let g_num = guild_id.parse::<u64>()?;
    let u_num = user_id.parse::<u64>()?;
    let g_id = serenity::all::GuildId::new(g_num);
    let u_id = serenity::all::UserId::new(u_num);

    let member = http.get_member(g_id, u_id).await?;
    let member_roles: Vec<String> = member.roles.iter().map(|r| r.get().to_string()).collect();

    let mut bot_highest_role_pos = None;
    let mut on_call_role_pos = None;

    if let Ok(guild) = http.get_guild(g_id).await {
        if let Some(on_call_id) = on_call_role_id
            && let Ok(on_call_num) = on_call_id.parse::<u64>()
            && let Some(role) = guild.roles.get(&serenity::all::RoleId::new(on_call_num))
        {
            on_call_role_pos = Some(role.position as i64);
        }

        if let Ok(bot_user) = http.get_current_user().await
            && let Ok(bot_member) = http.get_member(g_id, bot_user.id).await
        {
            let max_pos = bot_member
                .roles
                .iter()
                .filter_map(|rid| guild.roles.get(rid))
                .map(|r| r.position as i64)
                .max()
                .unwrap_or(0);
            bot_highest_role_pos = Some(max_pos);
        }
    }

    Ok(DiscordMemberDutyContext {
        member_roles,
        bot_highest_role_pos,
        on_call_role_pos,
    })
}

pub async fn reconcile_all_duty_guilds(
    storage: &crate::storage::Storage,
    http: Option<&serenity::all::Http>,
    is_live: bool,
) -> anyhow::Result<usize> {
    let guilds = storage.list_guilds_with_duty_profiles().await?;
    let mut total_reconciled = 0;

    for guild_id in guilds {
        let settings = storage
            .get_duty_settings(&guild_id)
            .await?
            .unwrap_or_else(|| ModeratorDutySettings {
                guild_id: guild_id.clone(),
                staff_role_id: None,
                on_call_role_id: None,
                max_override_duration_mins: 1440,
                fallback_channel_id: None,
                updated_at: Utc::now(),
            });

        let profiles = storage.list_duty_profiles(&guild_id).await?;
        for profile in profiles {
            let (member_roles, bot_pos, on_call_pos) = if is_live && let Some(h) = http {
                match fetch_discord_member_duty_context(
                    h,
                    &guild_id,
                    &profile.user_id,
                    settings.on_call_role_id.as_deref(),
                )
                .await
                {
                    Ok(ctx) => (
                        ctx.member_roles,
                        ctx.bot_highest_role_pos,
                        ctx.on_call_role_pos,
                    ),
                    Err(e) => {
                        let _ = storage
                            .upsert_duty_state(ModeratorDutyState {
                                guild_id: guild_id.clone(),
                                user_id: profile.user_id.clone(),
                                expected_on_call: false,
                                state_generation: 0,
                                last_reconciled_at: Some(Utc::now()),
                                last_error: Some(format!("Failed to fetch Discord member: {e}")),
                            })
                            .await;
                        continue;
                    }
                }
            } else {
                (Vec::new(), None, None)
            };

            let _ = reconcile_member_duty_state(ReconcileDutyParams {
                storage,
                guild_id: &guild_id,
                user_id: &profile.user_id,
                settings: &settings,
                profile: &profile,
                member_roles: &member_roles,
                bot_highest_role_pos: bot_pos,
                on_call_role_pos: on_call_pos,
                is_live_discord: is_live && http.is_some(),
                dry_run: false,
                http,
            })
            .await;
            total_reconciled += 1;
        }

        // Issue #188: Process durable role revocations for orphaned roles
        let _ = process_duty_role_revocations(storage, http, Some(&guild_id)).await;
    }

    Ok(total_reconciled)
}

pub async fn run_moderator_duty_reconciler_loop(
    state: crate::AppState,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
    interval_duration: std::time::Duration,
) {
    tracing::info!("Starting Moderator Duty reconciliation watchdog loop");
    let mut interval = tokio::time::interval(interval_duration);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let is_live = !state.config.discord_token.starts_with("dummy")
        && state.config.discord_token != "mock"
        && state.config.discord_token != "offline"
        && !state.config.discord_token.is_empty();

    let http = if is_live {
        Some(serenity::all::Http::new(&state.config.discord_token))
    } else {
        None
    };

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                tracing::info!("Stopping Moderator Duty reconciliation loop due to shutdown");
                break;
            }
            _ = interval.tick() => {
                if let Err(e) = reconcile_all_duty_guilds(&state.storage, http.as_ref(), is_live).await {
                    tracing::error!(error = %e, "Error in moderator duty watchdog loop iteration");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_non_staff_never_on_call() {
        let mut schedule = RecurringSchedule::default();
        schedule
            .monday
            .push(DutyTimeRange::new("00:00", "23:59").unwrap());

        let profile = ModeratorDutyProfile::new(
            "100000000000000001".to_string(),
            "200000000000000001".to_string(),
            "UTC".to_string(),
            schedule,
        )
        .unwrap();

        let now = Utc.with_ymd_and_hms(2026, 11, 2, 12, 0, 0).unwrap(); // Monday
        assert!(
            compute_is_on_call(&profile, now, true),
            "Staff should be on call"
        );
        assert!(
            !compute_is_on_call(&profile, now, false),
            "Non-staff must never have on-call role"
        );
    }

    #[test]
    fn test_manual_override_precedence_and_expiry() {
        let mut schedule = RecurringSchedule::default();
        schedule
            .monday
            .push(DutyTimeRange::new("08:00", "16:00").unwrap());

        let mut profile = ModeratorDutyProfile::new(
            "100000000000000001".to_string(),
            "200000000000000001".to_string(),
            "UTC".to_string(),
            schedule,
        )
        .unwrap();

        let now = Utc.with_ymd_and_hms(2026, 11, 2, 10, 0, 0).unwrap(); // Monday 10:00 UTC (inside schedule)

        // Normal schedule: on call
        assert!(compute_is_on_call(&profile, now, true));

        // Manual force_off with future expiry: overrides schedule to off
        profile.manual_override = Some("force_off".to_string());
        profile.override_expires_at = Some(now + chrono::Duration::hours(2));
        assert!(
            !compute_is_on_call(&profile, now, true),
            "Manual force_off must override recurring schedule"
        );

        // After expiry: reverts to recurring schedule (on call)
        let after_expiry = now + chrono::Duration::hours(3); // 13:00 UTC
        assert!(
            compute_is_on_call(&profile, after_expiry, true),
            "After manual override expiry, should revert to schedule"
        );
    }

    #[test]
    fn test_overnight_shift_across_midnight() {
        let mut schedule = RecurringSchedule::default();
        // Friday 22:00 to 02:00 overnight shift
        schedule
            .friday
            .push(DutyTimeRange::new("22:00", "02:00").unwrap());

        let profile = ModeratorDutyProfile::new(
            "100000000000000001".to_string(),
            "200000000000000001".to_string(),
            "UTC".to_string(),
            schedule,
        )
        .unwrap();

        // 1. Friday 21:59 UTC: not on call
        let fri_before = Utc.with_ymd_and_hms(2026, 11, 6, 21, 59, 0).unwrap();
        assert!(!compute_is_on_call(&profile, fri_before, true));

        // 2. Friday 22:30 UTC: on call
        let fri_during = Utc.with_ymd_and_hms(2026, 11, 6, 22, 30, 0).unwrap();
        assert!(compute_is_on_call(&profile, fri_during, true));

        // 3. Saturday 01:30 UTC: still on call from Friday overnight shift!
        let sat_early = Utc.with_ymd_and_hms(2026, 11, 7, 1, 30, 0).unwrap();
        assert!(
            compute_is_on_call(&profile, sat_early, true),
            "Overnight shift must continue on Saturday early morning"
        );

        // 4. Saturday 02:01 UTC: shift finished
        let sat_after = Utc.with_ymd_and_hms(2026, 11, 7, 2, 1, 0).unwrap();
        assert!(!compute_is_on_call(&profile, sat_after, true));
    }

    #[test]
    fn test_timezone_conversion_and_dst() {
        let mut schedule = RecurringSchedule::default();
        // Shift 14:00 - 18:00 Berlin time
        schedule
            .wednesday
            .push(DutyTimeRange::new("14:00", "18:00").unwrap());

        let profile = ModeratorDutyProfile::new(
            "100000000000000001".to_string(),
            "200000000000000001".to_string(),
            "Europe/Berlin".to_string(),
            schedule,
        )
        .unwrap();

        // In winter (CET, UTC+1): 13:00 UTC = 14:00 Berlin (start of shift)
        let wed_winter = Utc.with_ymd_and_hms(2026, 11, 4, 13, 30, 0).unwrap();
        assert!(
            compute_is_on_call(&profile, wed_winter, true),
            "13:30 UTC is 14:30 CET (active)"
        );

        // 12:30 UTC = 13:30 Berlin (before shift)
        let wed_winter_before = Utc.with_ymd_and_hms(2026, 11, 4, 12, 30, 0).unwrap();
        assert!(!compute_is_on_call(&profile, wed_winter_before, true));
    }

    #[tokio::test]
    async fn test_reconcile_member_duty_loss_of_staff_revokes_role() {
        let store = crate::storage::Storage::Memory(crate::storage::MemoryStore::default());
        let guild_id = "100000000000000001";
        let user_id = "200000000000000001";
        let staff_role = "300000000000000001".to_string();
        let on_call_role = "400000000000000001".to_string();

        let settings = ModeratorDutySettings {
            guild_id: guild_id.to_string(),
            staff_role_id: Some(staff_role.clone()),
            on_call_role_id: Some(on_call_role.clone()),
            max_override_duration_mins: 1440,
            fallback_channel_id: None,
            updated_at: Utc::now(),
        };

        let mut profile = ModeratorDutyProfile::new(
            guild_id.to_string(),
            user_id.to_string(),
            "UTC".to_string(),
            RecurringSchedule::default(),
        )
        .unwrap();
        // Force on
        profile.manual_override = Some("force_on".to_string());

        // Member currently has both staff and on_call roles
        let outcome = reconcile_member_duty_state(ReconcileDutyParams {
            storage: &store,
            guild_id,
            user_id,
            settings: &settings,
            profile: &profile,
            member_roles: &[staff_role.clone(), on_call_role.clone()],
            bot_highest_role_pos: Some(100),
            on_call_role_pos: Some(50),
            is_live_discord: false,
            dry_run: false,
            http: None,
        })
        .await
        .unwrap();

        assert!(outcome.expected_on_call);
        assert!(!outcome.role_granted);
        assert!(!outcome.role_revoked);

        // Now member LOSES staff role (only has on_call_role)
        let outcome2 = reconcile_member_duty_state(ReconcileDutyParams {
            storage: &store,
            guild_id,
            user_id,
            settings: &settings,
            profile: &profile,
            member_roles: std::slice::from_ref(&on_call_role),
            bot_highest_role_pos: Some(100),
            on_call_role_pos: Some(50),
            is_live_discord: false,
            dry_run: false,
            http: None,
        })
        .await
        .unwrap();

        assert!(
            !outcome2.expected_on_call,
            "Losing staff role must force expected_on_call = false"
        );
        assert!(
            outcome2.role_revoked,
            "On-call role must be revoked immediately when staff status is lost"
        );
    }

    #[tokio::test]
    async fn test_reconcile_role_hierarchy_failure() {
        let store = crate::storage::Storage::Memory(crate::storage::MemoryStore::default());
        let guild_id = "100000000000000001";
        let user_id = "200000000000000001";
        let on_call_role = "400000000000000001".to_string();

        let settings = ModeratorDutySettings {
            guild_id: guild_id.to_string(),
            staff_role_id: None,
            on_call_role_id: Some(on_call_role.clone()),
            max_override_duration_mins: 1440,
            fallback_channel_id: None,
            updated_at: Utc::now(),
        };

        let mut profile = ModeratorDutyProfile::new(
            guild_id.to_string(),
            user_id.to_string(),
            "UTC".to_string(),
            RecurringSchedule::default(),
        )
        .unwrap();
        profile.manual_override = Some("force_on".to_string());

        // Bot role pos = 10, On-call role pos = 20 (bot is LOWER than on_call role!)
        let outcome = reconcile_member_duty_state(ReconcileDutyParams {
            storage: &store,
            guild_id,
            user_id,
            settings: &settings,
            profile: &profile,
            member_roles: &[],
            bot_highest_role_pos: Some(10),
            on_call_role_pos: Some(20),
            is_live_discord: false,
            dry_run: false,
            http: None,
        })
        .await
        .unwrap();

        assert!(outcome.expected_on_call);
        assert!(!outcome.role_granted);
        assert!(outcome.error.is_some());
        assert!(outcome.error.unwrap().contains("hierarchy"));
    }

    #[tokio::test]
    async fn test_staff_audience_resolution_fallback() {
        let store = crate::storage::Storage::Memory(crate::storage::MemoryStore::default());
        let guild_id = "100000000000000001";
        let staff_role = "300000000000000001".to_string();
        let on_call_role = "400000000000000001".to_string();
        let fallback_chan = "500000000000000001".to_string();

        let settings = ModeratorDutySettings {
            guild_id: guild_id.to_string(),
            staff_role_id: Some(staff_role.clone()),
            on_call_role_id: Some(on_call_role.clone()),
            max_override_duration_mins: 1440,
            fallback_channel_id: Some(fallback_chan.clone()),
            updated_at: Utc::now(),
        };
        store.upsert_duty_settings(settings).await.unwrap();

        // When nobody is on call: StaffAudience::OnCall falls back to staff_role
        let target = resolve_staff_audience(&store, guild_id, StaffAudience::OnCall)
            .await
            .unwrap();
        assert_eq!(target, Some(staff_role.clone()));

        // When someone is on call:
        store
            .upsert_duty_state(ModeratorDutyState {
                guild_id: guild_id.to_string(),
                user_id: "200000000000000001".to_string(),
                expected_on_call: true,
                state_generation: 1,
                last_reconciled_at: Some(Utc::now()),
                last_error: None,
            })
            .await
            .unwrap();

        let target_on_call = resolve_staff_audience(&store, guild_id, StaffAudience::OnCall)
            .await
            .unwrap();
        assert_eq!(target_on_call, Some(on_call_role));
    }

    #[tokio::test]
    async fn test_reconcile_all_duty_guilds_watchdog_iteration() {
        let store = crate::storage::Storage::Memory(crate::storage::MemoryStore::default());
        let guild_1 = "100000000000000001";
        let user_1 = "200000000000000001";
        let on_call_role = "400000000000000001".to_string();

        let settings = ModeratorDutySettings {
            guild_id: guild_1.to_string(),
            staff_role_id: None,
            on_call_role_id: Some(on_call_role),
            max_override_duration_mins: 1440,
            fallback_channel_id: None,
            updated_at: Utc::now(),
        };
        store.upsert_duty_settings(settings).await.unwrap();

        let mut profile = ModeratorDutyProfile::new(
            guild_1.to_string(),
            user_1.to_string(),
            "UTC".to_string(),
            RecurringSchedule::default(),
        )
        .unwrap();
        profile.manual_override = Some("force_on".to_string());
        store.upsert_duty_profile(profile).await.unwrap();

        let count = reconcile_all_duty_guilds(&store, None, false)
            .await
            .unwrap();
        assert_eq!(count, 1);

        let state = store
            .get_duty_state(guild_1, user_1)
            .await
            .unwrap()
            .expect("duty state exists");
        assert!(state.expected_on_call);
        assert!(state.last_reconciled_at.is_some());
        assert!(state.last_error.is_none());
    }

    #[test]
    fn test_recurring_schedule_validation_and_diagnostics() {
        // 1. Default empty schedule is valid
        let default_sched = RecurringSchedule::default();
        assert!(default_sched.validate().is_ok());

        // 2. Valid day shifts and overnight shifts pass
        let mut valid_sched = RecurringSchedule::default();
        valid_sched.monday.push(DutyTimeRange {
            start: "18:00".to_string(),
            end: "22:00".to_string(),
        });
        valid_sched.friday.push(DutyTimeRange {
            start: "22:00".to_string(),
            end: "02:00".to_string(),
        });
        valid_sched.sunday.push(DutyTimeRange {
            start: "12:00".to_string(),
            end: "12:00".to_string(),
        });
        assert!(valid_sched.validate().is_ok());

        // 3. Invalid hour: 25:00
        let mut invalid_hour = RecurringSchedule::default();
        invalid_hour.monday.push(DutyTimeRange {
            start: "18:00".to_string(),
            end: "25:00".to_string(),
        });
        let err = invalid_hour.validate().unwrap_err().to_string();
        assert!(err.contains("Mon[0]"), "Expected Mon[0] in error: {err}");
        assert!(err.contains("25:00"), "Expected 25:00 in error: {err}");

        // 4. Invalid text: tomorrow
        let mut invalid_text = RecurringSchedule::default();
        invalid_text.tuesday.push(DutyTimeRange {
            start: "tomorrow".to_string(),
            end: "22:00".to_string(),
        });
        let err = invalid_text.validate().unwrap_err().to_string();
        assert!(err.contains("Tue[0]"), "Expected Tue[0] in error: {err}");
        assert!(
            err.contains("tomorrow"),
            "Expected tomorrow in error: {err}"
        );

        // 5. Empty string
        let mut invalid_empty = RecurringSchedule::default();
        invalid_empty.wednesday.push(DutyTimeRange {
            start: "".to_string(),
            end: "22:00".to_string(),
        });
        let err = invalid_empty.validate().unwrap_err().to_string();
        assert!(err.contains("Wed[0]"), "Expected Wed[0] in error: {err}");

        // 6. Invalid minute: 18:60
        let mut invalid_minute = RecurringSchedule::default();
        invalid_minute.thursday.push(DutyTimeRange {
            start: "18:60".to_string(),
            end: "22:00".to_string(),
        });
        let err = invalid_minute.validate().unwrap_err().to_string();
        assert!(err.contains("Thu[0]"), "Expected Thu[0] in error: {err}");

        // 7. ModeratorDutyProfile::new rejects invalid schedule
        let res = ModeratorDutyProfile::new(
            "100000000000000001".to_string(),
            "200000000000000001".to_string(),
            "Europe/Berlin".to_string(),
            invalid_hour.clone(),
        );
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("Mon[0]"));

        // 8. schedule_error helper reports diagnostics
        let profile = ModeratorDutyProfile {
            guild_id: "100000000000000001".to_string(),
            user_id: "200000000000000001".to_string(),
            timezone: "Europe/Berlin".to_string(),
            recurring_schedule: sqlx::types::Json(invalid_hour),
            manual_override: None,
            override_expires_at: None,
            version: 0,
            updated_at: Utc::now(),
        };
        assert!(profile.schedule_error().is_some());
        assert!(profile.schedule_error().unwrap().contains("Mon[0]"));
    }
}
