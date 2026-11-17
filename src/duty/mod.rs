pub(crate) mod authorization;

use anyhow::Context as _;
use authorization::{DutyRoleAuthorization, RemovalAuthorization};
use jiff::SignedDuration;
use jiff::civil::{Time, Weekday};
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};
use serenity::all::{GuildId, Http, RoleId, UserId};

use crate::datetime::Timestamp;
use crate::storage::validate_snowflake;

fn parse_duty_time(value: &str) -> Option<Time> {
    fn parse_component(value: &str) -> Option<i8> {
        // Stored schedules can contain one digit and leading Unicode whitespace.
        let digits = value.trim_start();
        if !(1..=2).contains(&digits.len()) || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok()
    }

    let (hour, minute) = value.split_once(':')?;
    Time::new(parse_component(hour)?, parse_component(minute)?, 0, 0).ok()
}

pub(crate) fn parse_duty_timezone(name: &str) -> Option<TimeZone> {
    let timezone = TimeZone::get(name).ok()?;
    // Preserve exact IANA spelling and aliases, including UTC.
    (timezone.iana_name() == Some(name)).then_some(timezone)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DutyTimeRange {
    pub start: String, // "HH:MM"
    pub end: String,   // "HH:MM"
}

impl DutyTimeRange {
    pub fn new(start: &str, end: &str) -> anyhow::Result<Self> {
        parse_duty_time(start).ok_or_else(|| {
            anyhow::anyhow!("Invalid start time format (expected HH:MM): {}", start)
        })?;
        parse_duty_time(end)
            .ok_or_else(|| anyhow::anyhow!("Invalid end time format (expected HH:MM): {}", end))?;
        Ok(Self {
            start: start.to_string(),
            end: end.to_string(),
        })
    }

    pub fn is_overnight(&self) -> bool {
        if let (Some(s), Some(e)) = (parse_duty_time(&self.start), parse_duty_time(&self.end)) {
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
            Weekday::Monday => &self.monday,
            Weekday::Tuesday => &self.tuesday,
            Weekday::Wednesday => &self.wednesday,
            Weekday::Thursday => &self.thursday,
            Weekday::Friday => &self.friday,
            Weekday::Saturday => &self.saturday,
            Weekday::Sunday => &self.sunday,
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        for (day, name) in [
            (Weekday::Monday, "Mon"),
            (Weekday::Tuesday, "Tue"),
            (Weekday::Wednesday, "Wed"),
            (Weekday::Thursday, "Thu"),
            (Weekday::Friday, "Fri"),
            (Weekday::Saturday, "Sat"),
            (Weekday::Sunday, "Sun"),
        ] {
            for (index, range) in self.ranges_for_weekday(day).iter().enumerate() {
                DutyTimeRange::new(&range.start, &range.end)
                    .map_err(|error| anyhow::anyhow!("{name}[{index}]: {error}"))?;
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
    pub updated_at: Timestamp,
    #[serde(skip)]
    pub role_authorized_by_user_id: Option<String>,
    #[serde(skip)]
    pub version: i64,
}

impl ModeratorDutySettings {
    pub(crate) fn unconfigured(guild_id: String) -> Self {
        Self {
            guild_id,
            staff_role_id: None,
            on_call_role_id: None,
            max_override_duration_mins: 1440,
            fallback_channel_id: None,
            updated_at: Timestamp::now(),
            role_authorized_by_user_id: None,
            version: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ModeratorDutyProfile {
    pub guild_id: String,
    pub user_id: String,
    pub timezone: String,
    pub recurring_schedule: sqlx::types::Json<RecurringSchedule>,
    pub manual_override: Option<String>,
    pub override_expires_at: Option<Timestamp>,
    pub version: i64,
    pub updated_at: Timestamp,
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
        parse_duty_timezone(&timezone)
            .ok_or_else(|| anyhow::anyhow!("Invalid IANA timezone: {}", timezone))?;
        schedule.validate()?;

        let now = Timestamp::now();
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
    pub last_reconciled_at: Option<Timestamp>,
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
pub fn compute_is_on_call(profile: &ModeratorDutyProfile, now: Timestamp, is_staff: bool) -> bool {
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
    let tz = parse_duty_timezone(&profile.timezone).unwrap_or(TimeZone::UTC);
    let local_now = tz.to_datetime(now.to_jiff());
    let current_weekday = local_now.weekday();
    let current_time = local_now.time();

    // A. Check today's scheduled shifts
    let today_ranges = profile
        .recurring_schedule
        .ranges_for_weekday(current_weekday);
    for range in today_ranges {
        match (parse_duty_time(&range.start), parse_duty_time(&range.end)) {
            (Some(start), Some(end)) => {
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
    let yesterday_weekday = current_weekday.previous();
    let yesterday_ranges = profile
        .recurring_schedule
        .ranges_for_weekday(yesterday_weekday);
    for range in yesterday_ranges {
        match (parse_duty_time(&range.start), parse_duty_time(&range.end)) {
            (Some(start), Some(end)) => {
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
    pub next_attempt_at: Timestamp,
    pub generation: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
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

#[derive(Clone, Copy)]
pub enum DutyDiscord<'a> {
    Live(&'a Http),
    Offline(&'a [String]),
}

pub(crate) fn duty_http(config: &crate::config::Config) -> Option<Http> {
    if config.is_offline_mode() {
        None
    } else if config.discord_api_base_url.starts_with("http://") {
        Some(
            serenity::http::HttpBuilder::new(&config.discord_token)
                .proxy(
                    config
                        .discord_api_base_url
                        .trim_end_matches('/')
                        .trim_end_matches("/api/v10"),
                )
                .ratelimiter_disabled(true)
                .build(),
        )
    } else {
        Some(Http::new(&config.discord_token))
    }
}

pub struct ReconcileDutyParams<'a> {
    pub storage: &'a crate::storage::Storage,
    pub settings: &'a ModeratorDutySettings,
    pub profile: &'a ModeratorDutyProfile,
    pub discord: DutyDiscord<'a>,
    pub dry_run: bool,
}

pub async fn reconcile_member_duty_state(
    params: ReconcileDutyParams<'_>,
) -> anyhow::Result<DutyReconcileOutcome> {
    let mut outcome = DutyReconcileOutcome {
        user_id: params.profile.user_id.clone(),
        ..DutyReconcileOutcome::default()
    };

    let result = match params.discord {
        DutyDiscord::Live(http) => reconcile_live_duty_member(&params, http, &mut outcome).await,
        DutyDiscord::Offline(member_roles) => {
            reconcile_offline_duty_member(&params, member_roles, &mut outcome)
        }
    };
    outcome.error = result.err().map(|error| format!("{error:#}"));

    if !params.dry_run {
        let state = ModeratorDutyState {
            guild_id: params.profile.guild_id.clone(),
            user_id: params.profile.user_id.clone(),
            expected_on_call: outcome.expected_on_call,
            state_generation: 0,
            last_reconciled_at: Some(Timestamp::now()),
            last_error: outcome.error.clone(),
        };
        params
            .storage
            .upsert_current_duty_state(params.profile, state)
            .await?;
    }

    Ok(outcome)
}

fn reconcile_offline_duty_member(
    params: &ReconcileDutyParams<'_>,
    member_roles: &[String],
    outcome: &mut DutyReconcileOutcome,
) -> anyhow::Result<()> {
    let has_role = |role_id: &str| {
        let role_id = role_id.trim_start_matches('0');
        member_roles
            .iter()
            .any(|role| role.trim_start_matches('0') == role_id)
    };
    let is_staff = params
        .settings
        .staff_role_id
        .as_deref()
        .is_none_or(|role_id| {
            role_id.trim_start_matches('0') == params.settings.guild_id || has_role(role_id)
        });
    outcome.expected_on_call = compute_is_on_call(params.profile, Timestamp::now(), is_staff);

    let role_id = params
        .settings
        .on_call_role_id
        .as_deref()
        .context("No on_call_role_id configured")?;
    let has_on_call = has_role(role_id);
    match (outcome.expected_on_call, has_on_call, params.dry_run) {
        (true, false, true) => outcome.would_grant = true,
        (false, true, true) => outcome.would_revoke = true,
        (true, false, false) => outcome.role_granted = true,
        (false, true, false) => outcome.role_revoked = true,
        _ => {}
    }
    Ok(())
}

async fn reconcile_live_duty_member(
    params: &ReconcileDutyParams<'_>,
    http: &Http,
    outcome: &mut DutyReconcileOutcome,
) -> anyhow::Result<()> {
    let profile = params.profile;
    let settings = params.settings;
    anyhow::ensure!(
        profile.guild_id == settings.guild_id,
        "Moderator Duty settings do not belong to this guild"
    );

    let guild = GuildId::new(validate_snowflake("guild_id", &profile.guild_id)?);
    let user = UserId::new(validate_snowflake("user_id", &profile.user_id)?);

    let authorization = DutyRoleAuthorization::fetch(http, guild).await?;
    let member = authorization.fetch_member(http, user).await?;
    let staff_role = settings
        .staff_role_id
        .as_deref()
        .map(|id| validate_snowflake("staff_role_id", id).map(RoleId::new))
        .transpose()?;
    let is_staff =
        staff_role.is_none_or(|id| id.get() == guild.get() || member.roles.contains(&id));
    outcome.expected_on_call = compute_is_on_call(profile, Timestamp::now(), is_staff);
    let role_key = settings
        .on_call_role_id
        .as_deref()
        .context("No on_call_role_id configured")?;
    let role = RoleId::new(validate_snowflake("on_call_role_id", role_key)?);
    let has_on_call = member.roles.contains(&role);

    let mut tracked_removal = None;
    if outcome.expected_on_call {
        authorization
            .authorize_grant(
                http,
                role,
                settings.staff_role_id.as_deref(),
                settings.role_authorized_by_user_id.as_deref(),
            )
            .await?;
        if has_on_call {
            return Ok(());
        }
    } else if has_on_call {
        tracked_removal = params
            .storage
            .get_duty_role_grant(&profile.guild_id, &profile.user_id, role_key)
            .await?
            .filter(|grant| {
                matches!(
                    grant.status.as_str(),
                    "active" | "revoke_pending" | "failed"
                )
            });
        if authorization.authorize_removal(role, tracked_removal.is_some())?
            == RemovalAuthorization::Deleted
        {
            return Ok(());
        }
        anyhow::ensure!(
            !tracked_removal.as_ref().is_some_and(|grant| {
                grant.status == "revoke_pending"
                    || grant.status == "active" && grant.next_attempt_at > Timestamp::now()
            }),
            "A Moderator Duty role operation is already pending"
        );
    } else {
        authorization.authorize_staff_role(settings.staff_role_id.as_deref())?;
        return Ok(());
    }

    if params.dry_run || !outcome.expected_on_call {
        anyhow::ensure!(
            params
                .storage
                .duty_grant_policy_is_current(settings, profile)
                .await?,
            "Moderator Duty settings or profile changed during reconciliation"
        );
    }
    if params.dry_run {
        outcome.would_grant = outcome.expected_on_call;
        outcome.would_revoke = !outcome.expected_on_call;
    } else if outcome.expected_on_call {
        let grant = params
            .storage
            .begin_duty_role_grant(settings, profile)
            .await?
            .context("Moderator Duty policy changed or a role operation is already pending")?;
        // Record intent before Discord. A timeout or lost response may still have added the role.
        let delivery = tokio::time::timeout(
            DUTY_DISCORD_OPERATION_TIMEOUT,
            http.add_member_role(guild, user, role, Some("Moderator on-duty scheduled")),
        )
        .await
        .context("Timed out adding the on-call role")?;
        if let Err(error) = &delivery
            && !matches!(
                error,
                serenity::Error::Http(serenity::http::HttpError::UnsuccessfulRequest(_))
            )
        {
            // Keep the lease when sending failed without a confirmed Discord response.
            return delivery.context("Discord could not confirm the on-call role grant");
        }
        outcome.role_granted = delivery.is_ok();
        let current = params
            .storage
            .finish_duty_role_grant(settings, profile, &grant)
            .await?;
        delivery.context("Discord could not add the on-call role")?;
        anyhow::ensure!(
            current,
            "Moderator Duty policy changed during the role grant"
        );
    } else {
        tokio::time::timeout(
            DUTY_DISCORD_OPERATION_TIMEOUT,
            http.remove_member_role(guild, user, role, Some("Moderator off-duty scheduled")),
        )
        .await
        .context("Timed out removing the on-call role")?
        .context("Discord could not remove the on-call role")?;
        outcome.role_revoked = true;
        if let Some(mut grant) = tracked_removal {
            // A newer grant can finish while Discord is delivering this DELETE response.
            grant.status = "revoked".to_owned();
            grant.last_error = None;
            params.storage.complete_duty_role_revocation(grant).await?;
        }
    }
    Ok(())
}

const DUTY_DISCORD_OPERATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub async fn process_duty_role_revocations(
    storage: &crate::storage::Storage,
    http: Option<&Http>,
    guild_id: Option<&str>,
) -> anyhow::Result<usize> {
    let Some(http) = http else {
        return Ok(0);
    };

    let pending = storage.list_pending_duty_role_revocations(guild_id).await?;
    let mut processed = 0;

    for pending_grant in pending {
        let Some(mut grant) = storage.claim_duty_role_revocation(&pending_grant).await? else {
            continue;
        };

        match tokio::time::timeout(
            DUTY_DISCORD_OPERATION_TIMEOUT,
            revoke_queued_duty_role(http, &grant),
        )
        .await
        .context("Timed out checking or revoking a Moderator Duty grant")
        .flatten()
        {
            Ok(()) => {
                grant.status = "revoked".to_owned();
                grant.last_error = None;
            }
            Err(error) => {
                grant.attempts += 1;
                grant.last_error = Some(format!("{error:#}"));
                if grant.attempts >= 5 {
                    grant.status = "failed".to_owned();
                } else {
                    let backoff_secs = 2_i64.pow(grant.attempts as u32) * 5;
                    grant.next_attempt_at =
                        Timestamp::now() + SignedDuration::from_secs(backoff_secs);
                }
            }
        }

        let revoked = grant.status == "revoked";
        if storage.complete_duty_role_revocation(grant).await? && revoked {
            processed += 1;
        }
    }

    Ok(processed)
}

async fn revoke_queued_duty_role(
    http: &Http,
    grant: &ModeratorDutyRoleGrant,
) -> anyhow::Result<()> {
    let guild = GuildId::new(validate_snowflake("guild_id", &grant.guild_id)?);
    let user = UserId::new(validate_snowflake("user_id", &grant.user_id)?);
    let role = RoleId::new(validate_snowflake("role_id", &grant.role_id)?);
    remove_tracked_duty_role(
        http,
        guild,
        user,
        role,
        "Moderator Duty profile deleted or on-call role changed",
    )
    .await
}

async fn remove_tracked_duty_role(
    http: &Http,
    guild: GuildId,
    user: UserId,
    role: RoleId,
    reason: &str,
) -> anyhow::Result<()> {
    let authorization = DutyRoleAuthorization::fetch(http, guild).await?;
    if authorization.authorize_removal(role, true)? == RemovalAuthorization::Deleted {
        return Ok(());
    }

    match http
        .remove_member_role(guild, user, role, Some(reason))
        .await
    {
        // A member or role deleted since the grant cannot hold it anymore.
        Err(serenity::Error::Http(serenity::http::HttpError::UnsuccessfulRequest(response)))
            if response.status_code == serenity::http::StatusCode::NOT_FOUND
                && matches!(response.error.code, 10007 | 10011) =>
        {
            Ok(())
        }
        result => result.context("Discord could not revoke the tracked on-call role"),
    }
}

pub async fn reconcile_all_duty_guilds(
    storage: &crate::storage::Storage,
    http: Option<&Http>,
    is_live: bool,
) -> anyhow::Result<usize> {
    let discord = if is_live {
        DutyDiscord::Live(http.context("Live Moderator Duty reconcile requires a Discord client")?)
    } else {
        DutyDiscord::Offline(&[])
    };

    if let DutyDiscord::Live(http) = discord {
        // Also service guilds whose last profile was deleted.
        if let Err(error) = process_duty_role_revocations(storage, Some(http), None).await {
            tracing::error!(%error, "Could not process queued Moderator Duty revocations");
        }
    }

    let guilds = storage.list_guilds_with_duty_profiles().await?;
    let mut total_reconciled = 0;

    for guild_id in guilds {
        let settings = storage
            .get_duty_settings(&guild_id)
            .await?
            .unwrap_or_else(|| ModeratorDutySettings::unconfigured(guild_id.clone()));

        let profiles = storage.list_duty_profiles(&guild_id).await?;
        for profile in profiles {
            if let Err(error) = reconcile_member_duty_state(ReconcileDutyParams {
                storage,
                settings: &settings,
                profile: &profile,
                discord,
                dry_run: false,
            })
            .await
            {
                tracing::error!(%error, %guild_id, user_id = %profile.user_id,
                    "Could not save Moderator Duty reconciliation state");
                continue;
            }
            total_reconciled += 1;
        }
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

    let http = duty_http(&state.config);
    let is_live = !state.config.is_offline_mode();

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

    #[test]
    fn duty_time_ranges_keep_accepting_stored_single_digit_components() {
        let range = DutyTimeRange::new("\u{2003}22: 0", "1:\u{a0}3").unwrap();
        assert!(range.is_overnight());
        assert!(DutyTimeRange::new("22 :00", "01:03").is_err());
        assert!(DutyTimeRange::new("22:00 ", "01:03").is_err());
        assert!(DutyTimeRange::new("22:00:00", "01:03").is_err());
    }

    #[test]
    fn duty_profiles_accept_exact_iana_names_and_aliases() {
        for (zone, valid) in [
            ("UTC", true),
            ("Etc/UTC", true),
            ("Europe/Berlin", true),
            ("Europe/Kiev", true),
            ("europe/berlin", false),
            ("Etc/Unknown", false),
            ("+01:00", false),
        ] {
            let profile = ModeratorDutyProfile::new(
                "100000000000000001".to_string(),
                "200000000000000001".to_string(),
                zone.to_string(),
                RecurringSchedule::default(),
            );
            assert_eq!(profile.is_ok(), valid, "{zone}");
        }
    }

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

        let now = "2026-11-02T12:00:00Z".parse::<Timestamp>().unwrap(); // Monday
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

        let now = "2026-11-02T10:00:00Z".parse::<Timestamp>().unwrap(); // Monday 10:00 UTC (inside schedule)

        // Normal schedule: on call
        assert!(compute_is_on_call(&profile, now, true));

        // Manual force_off with future expiry: overrides schedule to off
        profile.manual_override = Some("force_off".to_string());
        profile.override_expires_at = Some(now + SignedDuration::from_hours(2));
        assert!(
            !compute_is_on_call(&profile, now, true),
            "Manual force_off must override recurring schedule"
        );

        assert!(compute_is_on_call(
            &profile,
            profile.override_expires_at.unwrap(),
            true
        ));

        // After expiry: reverts to recurring schedule (on call)
        let after_expiry = now + SignedDuration::from_hours(3); // 13:00 UTC
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
        let fri_before = "2026-11-06T21:59:00Z".parse::<Timestamp>().unwrap();
        assert!(!compute_is_on_call(&profile, fri_before, true));

        // 2. Friday 22:30 UTC: on call
        let fri_during = "2026-11-06T22:30:00Z".parse::<Timestamp>().unwrap();
        assert!(compute_is_on_call(&profile, fri_during, true));

        // 3. Saturday 01:30 UTC: still on call from Friday overnight shift!
        let sat_early = "2026-11-07T01:30:00Z".parse::<Timestamp>().unwrap();
        assert!(
            compute_is_on_call(&profile, sat_early, true),
            "Overnight shift must continue on Saturday early morning"
        );

        // 4. Saturday 02:01 UTC: shift finished
        let sat_after = "2026-11-07T02:01:00Z".parse::<Timestamp>().unwrap();
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
        let wed_winter = "2026-11-04T13:30:00Z".parse::<Timestamp>().unwrap();
        assert!(
            compute_is_on_call(&profile, wed_winter, true),
            "13:30 UTC is 14:30 CET (active)"
        );

        // 12:30 UTC = 13:30 Berlin (before shift)
        let wed_winter_before = "2026-11-04T12:30:00Z".parse::<Timestamp>().unwrap();
        assert!(!compute_is_on_call(&profile, wed_winter_before, true));
    }

    #[test]
    fn duty_schedule_skips_nonexistent_wall_times_during_dst_gap() {
        let profile = ModeratorDutyProfile::new(
            "100000000000000001".to_string(),
            "200000000000000001".to_string(),
            "Europe/Berlin".to_string(),
            RecurringSchedule {
                sunday: vec![DutyTimeRange::new("02:15", "02:45").unwrap()],
                ..RecurringSchedule::default()
            },
        )
        .unwrap();

        // The same shift is active on a Sunday before the clocks change.
        assert!(compute_is_on_call(
            &profile,
            "2026-03-22T01:30:00Z".parse().unwrap(),
            true,
        ));
        for instant in [
            "2026-03-29T00:59:59Z", // 01:59:59 CET
            "2026-03-29T01:00:00Z", // 03:00:00 CEST
            "2026-03-29T01:30:00Z", // 03:30:00 CEST
            "2026-03-29T02:30:00Z", // 04:30:00 CEST
        ] {
            assert!(
                !compute_is_on_call(&profile, instant.parse().unwrap(), true),
                "{instant}"
            );
        }
    }

    #[test]
    fn duty_schedule_applies_to_both_occurrences_of_repeated_wall_times() {
        let profile = ModeratorDutyProfile::new(
            "100000000000000001".to_string(),
            "200000000000000001".to_string(),
            "Europe/Berlin".to_string(),
            RecurringSchedule {
                sunday: vec![DutyTimeRange::new("02:15", "02:45").unwrap()],
                ..RecurringSchedule::default()
            },
        )
        .unwrap();

        for (instant, expected) in [
            ("2026-10-25T00:15:00Z", true),  // 02:15 CEST
            ("2026-10-25T00:45:00Z", false), // 02:45 CEST
            ("2026-10-25T01:15:00Z", true),  // 02:15 CET
            ("2026-10-25T01:45:00Z", false), // 02:45 CET
        ] {
            assert_eq!(
                compute_is_on_call(&profile, instant.parse().unwrap(), true),
                expected,
                "{instant}"
            );
        }
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
            updated_at: Timestamp::now(),
            role_authorized_by_user_id: None,
            version: 0,
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
            settings: &settings,
            profile: &profile,
            discord: DutyDiscord::Offline(&[staff_role.clone(), on_call_role.clone()]),
            dry_run: false,
        })
        .await
        .unwrap();

        assert!(outcome.expected_on_call);
        assert!(!outcome.role_granted);
        assert!(!outcome.role_revoked);

        // Now member LOSES staff role (only has on_call_role)
        let outcome2 = reconcile_member_duty_state(ReconcileDutyParams {
            storage: &store,
            settings: &settings,
            profile: &profile,
            discord: DutyDiscord::Offline(std::slice::from_ref(&on_call_role)),
            dry_run: false,
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
            updated_at: Timestamp::now(),
            role_authorized_by_user_id: None,
            version: 0,
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
                last_reconciled_at: Some(Timestamp::now()),
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
            updated_at: Timestamp::now(),
            role_authorized_by_user_id: None,
            version: 0,
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
            updated_at: Timestamp::now(),
        };
        assert!(profile.schedule_error().is_some());
        assert!(profile.schedule_error().unwrap().contains("Mon[0]"));
    }
}
