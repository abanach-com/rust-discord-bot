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
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
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
        if let (Ok(start), Ok(end)) = (
            NaiveTime::parse_from_str(&range.start, "%H:%M"),
            NaiveTime::parse_from_str(&range.end, "%H:%M"),
        ) {
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
    }

    // B. Check yesterday's overnight shifts that carry over into today
    let yesterday_weekday = current_weekday.pred();
    let yesterday_ranges = profile
        .recurring_schedule
        .ranges_for_weekday(yesterday_weekday);
    for range in yesterday_ranges {
        if let (Ok(start), Ok(end)) = (
            NaiveTime::parse_from_str(&range.start, "%H:%M"),
            NaiveTime::parse_from_str(&range.end, "%H:%M"),
        ) && start > end
            && current_time < end
        {
            return true;
        }
    }

    false
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DutyReconcileOutcome {
    pub user_id: String,
    pub expected_on_call: bool,
    pub role_granted: bool,
    pub role_revoked: bool,
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
            let state = ModeratorDutyState {
                guild_id: params.guild_id.to_string(),
                user_id: params.user_id.to_string(),
                expected_on_call: expected,
                state_generation: 0,
                last_reconciled_at: Some(Utc::now()),
                last_error: Some("No on_call_role_id configured".to_string()),
            };
            let _ = params.storage.upsert_duty_state(state).await?;
            return Ok(DutyReconcileOutcome {
                user_id: params.user_id.to_string(),
                expected_on_call: expected,
                role_granted: false,
                role_revoked: false,
                error: Some("No on_call_role_id configured".to_string()),
            });
        }
    };

    let has_on_call = params.member_roles.contains(&on_call_role_id);
    let mut role_granted = false;
    let mut role_revoked = false;
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
            if params.is_live_discord {
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
                        }
                    } else if let Err(e) = h
                        .remove_member_role(g_id, u_id, r_id, Some("Moderator off-duty scheduled"))
                        .await
                    {
                        error = Some(format!("Discord API remove_role error: {e}"));
                    } else {
                        role_revoked = true;
                    }
                }
            } else if expected {
                role_granted = true;
            } else {
                role_revoked = true;
            }
        }
    }

    let state = ModeratorDutyState {
        guild_id: params.guild_id.to_string(),
        user_id: params.user_id.to_string(),
        expected_on_call: expected,
        state_generation: 0,
        last_reconciled_at: Some(Utc::now()),
        last_error: error.clone(),
    };
    let _ = params.storage.upsert_duty_state(state).await?;

    Ok(DutyReconcileOutcome {
        user_id: params.user_id.to_string(),
        expected_on_call: expected,
        role_granted,
        role_revoked,
        error,
    })
}

pub async fn run_moderator_duty_reconciler_loop(
    _storage: crate::storage::Storage,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
    interval_duration: std::time::Duration,
) {
    tracing::info!("Starting Moderator Duty reconciliation watchdog loop");
    let mut interval = tokio::time::interval(interval_duration);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                tracing::info!("Stopping Moderator Duty reconciliation loop due to shutdown");
                break;
            }
            _ = interval.tick() => {
                // Periodic watchdog iteration
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
}
