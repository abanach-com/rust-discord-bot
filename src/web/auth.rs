use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};

use axum::{
    Json,
    extract::{Query, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{COOKIE, SET_COOKIE},
    },
    response::{IntoResponse, Redirect, Response},
};
use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use serde::Deserialize;
use url::Url;
use uuid::Uuid;

use super::{WebState, routes::ApiError};

const SESSION_COOKIE: &str = "rusty_discord_bot_session";
const OAUTH_STATE_COOKIE: &str = "rusty_oauth_state";
const DISCORD_AUTHORIZE_URL: &str = "https://discord.com/oauth2/authorize";
const DISCORD_TOKEN_URL: &str = "https://discord.com/api/v10/oauth2/token";
const DISCORD_USER_URL: &str = "https://discord.com/api/v10/users/@me";
const DISCORD_GUILDS_URL: &str = "https://discord.com/api/v10/users/@me/guilds";
const ADMINISTRATOR: u64 = 1 << 3;
const MANAGE_GUILD: u64 = 1 << 5;

pub const MAX_PENDING_OAUTH_STATES: usize = 1_000;
pub const MAX_LOGIN_REQUESTS_PER_MINUTE_PER_IP: u32 = 10;
pub const MAX_GLOBAL_LOGIN_REQUESTS_PER_MINUTE: u32 = 60;
pub const GUILD_PERMISSIONS_CACHE_TTL_SECONDS: i64 = 60;

#[derive(Clone, Debug)]
pub struct CachedGuildPermission {
    pub can_manage: bool,
    pub checked_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct LoginRateLimiter {
    per_ip: DashMap<String, (DateTime<Utc>, u32)>,
    global: AtomicU32,
    global_window_start: AtomicI64,
    global_lock: std::sync::Mutex<()>,
}

impl Default for LoginRateLimiter {
    fn default() -> Self {
        Self {
            per_ip: DashMap::new(),
            global: AtomicU32::new(0),
            global_window_start: AtomicI64::new(Utc::now().timestamp()),
            global_lock: std::sync::Mutex::new(()),
        }
    }
}

impl LoginRateLimiter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn check_and_record(&self, ip: &str) -> Result<(), ApiError> {
        let now = Utc::now();
        let now_ts = now.timestamp();

        // 1. Evaluate per-IP limit first without consuming global accepted-login slots
        {
            let mut entry = self.per_ip.entry(ip.to_owned()).or_insert_with(|| (now, 0));
            let (ip_window_start, count) = entry.value_mut();
            if now - *ip_window_start >= Duration::seconds(60) {
                *ip_window_start = now;
                *count = 1;
            } else {
                if *count >= MAX_LOGIN_REQUESTS_PER_MINUTE_PER_IP {
                    return Err(ApiError::too_many_requests(
                        "Too many login attempts from your IP address. Please wait a moment.",
                    ));
                }
                *count = count.saturating_add(1);
            }
        }

        // 2. Per-IP check passed: atomically reserve global budget
        let _guard = self.global_lock.lock().unwrap_or_else(|e| e.into_inner());
        let window_start = self.global_window_start.load(Ordering::SeqCst);
        if now_ts.saturating_sub(window_start) >= 60 {
            self.global_window_start.store(now_ts, Ordering::SeqCst);
            self.global.store(1, Ordering::SeqCst);
        } else {
            let current = self.global.load(Ordering::SeqCst);
            if current >= MAX_GLOBAL_LOGIN_REQUESTS_PER_MINUTE {
                // Roll back per-IP increment so failed global attempt does not consume user's slot
                if let Some(mut entry) = self.per_ip.get_mut(ip) {
                    let (_, count) = entry.value_mut();
                    *count = count.saturating_sub(1);
                }
                return Err(ApiError::too_many_requests(
                    "Too many login attempts on the server. Please try again shortly.",
                ));
            }
            self.global.store(current + 1, Ordering::SeqCst);
        }

        Ok(())
    }

    pub fn cleanup_old_windows(&self) {
        let cutoff = Utc::now() - Duration::seconds(120);
        self.per_ip.retain(|_, (timestamp, _)| *timestamp > cutoff);
    }
}

pub use crate::session::{DashboardGuild, DashboardUser, Session};

#[derive(Deserialize, Clone)]
pub struct OAuthCallback {
    code: String,
    state: String,
}

#[derive(Deserialize)]
struct OAuthToken {
    access_token: String,
}

pub async fn login(
    State(state): State<WebState>,
    axum::extract::ConnectInfo(peer_addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let client_ip = resolve_client_ip(
        Some(peer_addr.ip()),
        &headers,
        &state.app.config.trusted_proxies,
    );
    state.login_rate_limiter.check_and_record(&client_ip)?;

    let pending = state
        .app
        .sessions
        .count_pending_oauth_states()
        .await
        .unwrap_or(0);
    if pending >= MAX_PENDING_OAUTH_STATES {
        cleanup_stale_records(&state).await;
        let pending_after = state
            .app
            .sessions
            .count_pending_oauth_states()
            .await
            .unwrap_or(0);
        if pending_after >= MAX_PENDING_OAUTH_STATES {
            return Err(ApiError::too_many_requests(
                "Pending login attempts limit reached. Please try again shortly.",
            ));
        }
    }

    let csrf_state = Uuid::new_v4().to_string();
    state
        .app
        .sessions
        .save_oauth_state(&csrf_state)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let mut url =
        Url::parse(DISCORD_AUTHORIZE_URL).map_err(|error| ApiError::internal(error.to_string()))?;
    url.query_pairs_mut()
        .append_pair("client_id", &state.app.config.discord_client_id)
        .append_pair("redirect_uri", &state.app.config.discord_oauth_redirect_url)
        .append_pair("response_type", "code")
        .append_pair("scope", "identify guilds")
        .append_pair("state", &csrf_state)
        .append_pair("prompt", "none");

    let mut response = Redirect::temporary(url.as_str()).into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&oauth_state_cookie(
            &csrf_state,
            state.app.config.session_cookie_secure,
        ))
        .map_err(|e| ApiError::internal(e.to_string()))?,
    );
    Ok(response)
}

pub async fn callback(
    State(state): State<WebState>,
    headers: HeaderMap,
    Query(query): Query<OAuthCallback>,
) -> Result<Response, ApiError> {
    if cookie_value(&headers, OAUTH_STATE_COOKIE).as_deref() != Some(query.state.as_str()) {
        return Err(ApiError::bad_request(
            "OAuth login was not initiated in this browser.",
        ));
    }

    let valid_state = state
        .app
        .sessions
        .consume_oauth_state(&query.state)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if !valid_state {
        return Err(ApiError::bad_request(
            "Invalid OAuth2 login state or session expired.",
        ));
    }

    let token = state
        .app
        .http_client
        .post(DISCORD_TOKEN_URL)
        .basic_auth(
            &state.app.config.discord_client_id,
            Some(&state.app.config.discord_client_secret),
        )
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", query.code.as_str()),
            (
                "redirect_uri",
                state.app.config.discord_oauth_redirect_url.as_str(),
            ),
        ])
        .send()
        .await
        .map_err(ApiError::upstream)?
        .error_for_status()
        .map_err(ApiError::upstream)?
        .json::<OAuthToken>()
        .await
        .map_err(ApiError::upstream)?;

    let user = state
        .app
        .http_client
        .get(DISCORD_USER_URL)
        .bearer_auth(&token.access_token)
        .send()
        .await
        .map_err(ApiError::upstream)?
        .error_for_status()
        .map_err(ApiError::upstream)?
        .json::<DashboardUser>()
        .await
        .map_err(ApiError::upstream)?;

    let guilds = state
        .app
        .http_client
        .get(DISCORD_GUILDS_URL)
        .bearer_auth(&token.access_token)
        .send()
        .await
        .map_err(ApiError::upstream)?
        .error_for_status()
        .map_err(ApiError::upstream)?
        .json::<Vec<DashboardGuild>>()
        .await
        .map_err(ApiError::upstream)?;

    let session_id = Uuid::new_v4().to_string();
    let csrf_token = Uuid::new_v4().to_string();
    let expires_at = Utc::now() + Duration::seconds(state.app.config.session_ttl_seconds);

    state
        .app
        .sessions
        .save_session(
            &session_id,
            user,
            guilds,
            csrf_token,
            expires_at,
            None, // Discord access token is discarded after OAuth lookup
        )
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let mut response = Redirect::to("/").into_response();
    response.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&session_cookie(
            &session_id,
            state.app.config.session_ttl_seconds,
            state.app.config.session_cookie_secure,
        ))
        .map_err(|error| ApiError::internal(error.to_string()))?,
    );
    response.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&expired_oauth_state_cookie(
            state.app.config.session_cookie_secure,
        ))
        .map_err(|error| ApiError::internal(error.to_string()))?,
    );
    Ok(response)
}

pub async fn current_session(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> Result<Json<Session>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    Ok(Json(session))
}

pub async fn logout(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let (session_id, session) = authenticate(&state, &headers).await?;
    verify_csrf(&headers, &session)?;
    let _ = state.app.sessions.delete_session(&session_id).await;
    state
        .permissions_cache
        .retain(|(user_id, _), _| user_id != &session.user.id);

    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&expired_session_cookie(
            state.app.config.session_cookie_secure,
        ))
        .map_err(|error| ApiError::internal(error.to_string()))?,
    );
    Ok(response)
}

pub async fn authenticate(
    state: &WebState,
    headers: &HeaderMap,
) -> Result<(String, Session), ApiError> {
    let session_id = cookie_value(headers, SESSION_COOKIE)
        .ok_or_else(|| ApiError::unauthorized("Please log in via Discord."))?;
    let session = state
        .app
        .sessions
        .get_session(&session_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::unauthorized("Session expired. Please log in again."))?;

    Ok((session_id, session))
}

pub fn verify_csrf(headers: &HeaderMap, session: &Session) -> Result<(), ApiError> {
    let provided = headers
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok());
    if provided != Some(session.csrf_token.as_str()) {
        return Err(ApiError::forbidden("Invalid CSRF token."));
    }
    Ok(())
}

pub async fn ensure_guild_access(
    state: &WebState,
    session: &Session,
    guild_id: &str,
) -> Result<(), ApiError> {
    if !session.guilds.iter().any(|guild| guild.id == guild_id) {
        return Err(ApiError::forbidden(
            "You do not have permission to manage this server.",
        ));
    }

    let cache_key = (session.user.id.clone(), guild_id.to_owned());
    let now = Utc::now();

    if let Some(entry) = state.permissions_cache.get(&cache_key)
        && now - entry.checked_at < Duration::seconds(GUILD_PERMISSIONS_CACHE_TTL_SECONDS)
    {
        if entry.can_manage {
            return Ok(());
        } else {
            return Err(ApiError::forbidden(
                "Permissions to manage this server have been revoked.",
            ));
        }
    }

    let can_manage = revalidate_guild_permission(state, session, guild_id).await?;
    state.permissions_cache.insert(
        cache_key,
        CachedGuildPermission {
            can_manage,
            checked_at: now,
        },
    );

    if can_manage {
        Ok(())
    } else {
        Err(ApiError::forbidden(
            "Permissions to manage this server have been revoked.",
        ))
    }
}

pub async fn ensure_guild_member_access(
    state: &WebState,
    session: &Session,
    guild_id: &str,
) -> Result<(), ApiError> {
    // 1. Fast rejection if guild is not in session.guilds
    if !session.guilds.iter().any(|g| g.id == guild_id) {
        return Err(ApiError::forbidden("You are not a member of this server."));
    }

    // 2. If user is in session guilds and has manage rights (or manage rights already cached), they are definitely a member
    let cache_key = (session.user.id.clone(), guild_id.to_owned());
    let now = Utc::now();
    if let Some(entry) = state.permissions_cache.get(&cache_key)
        && now - entry.checked_at < Duration::seconds(GUILD_PERMISSIONS_CACHE_TTL_SECONDS)
        && entry.can_manage
    {
        return Ok(());
    }

    // 3. In mock or test mode, session.guilds presence is sufficient
    if state.app.config.is_offline_mode() {
        return Ok(());
    }

    // 4. In live mode with Discord API: verify membership via Bot token
    crate::storage::validate_snowflake("guild_id", guild_id)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    crate::storage::validate_snowflake("user_id", &session.user.id)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    let base_url = &state.app.config.discord_api_base_url;
    let member_url = format!("{base_url}/guilds/{guild_id}/members/{}", session.user.id);
    let response = state
        .app
        .http_client
        .get(&member_url)
        .header(
            reqwest::header::AUTHORIZATION,
            format!("Bot {}", state.app.config.discord_token),
        )
        .send()
        .await
        .map_err(ApiError::upstream)?;

    match response.status() {
        StatusCode::OK => Ok(()),
        StatusCode::NOT_FOUND | StatusCode::FORBIDDEN => {
            Err(ApiError::forbidden("You are not a member of this server."))
        }
        status => {
            tracing::warn!(
                %status,
                guild_id,
                user_id = %session.user.id,
                "Discord API returned error when checking guild membership; failing closed"
            );
            Err(ApiError::forbidden(
                "Guild membership could not be verified at this time.",
            ))
        }
    }
}

pub async fn revalidate_guild_permission(
    state: &WebState,
    session: &Session,
    guild_id: &str,
) -> Result<bool, ApiError> {
    if state.app.config.is_offline_mode() {
        return Ok(session
            .guilds
            .iter()
            .any(|g| g.id == guild_id && can_manage(g)));
    }

    if let Some(ref access_token) = session.access_token {
        let response = state
            .app
            .http_client
            .get(DISCORD_GUILDS_URL)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(ApiError::upstream)?;

        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(ApiError::unauthorized(
                "Discord authorization token expired or was revoked. Please log in again.",
            ));
        }

        let guilds = response
            .error_for_status()
            .map_err(ApiError::upstream)?
            .json::<Vec<DashboardGuild>>()
            .await
            .map_err(ApiError::upstream)?;

        let can_manage_guild = guilds.iter().any(|g| g.id == guild_id && can_manage(g));
        return Ok(can_manage_guild);
    }

    // Access token was discarded at rest for security; revalidate using Bot token via Discord API.
    // In Discord API v10, computed member permissions are retrieved via:
    // GET /guilds/{guild_id}/permissions/member/{user_id} -> { "permissions": "<bitfield>" }
    let perms_url = format!(
        "https://discord.com/api/v10/guilds/{guild_id}/permissions/member/{}",
        session.user.id
    );
    let response = state
        .app
        .http_client
        .get(&perms_url)
        .header(
            reqwest::header::AUTHORIZATION,
            format!("Bot {}", state.app.config.discord_token),
        )
        .send()
        .await
        .map_err(ApiError::upstream)?;

    // If 404, fall back to GET /guilds/{guild_id}/members/{user_id} for mocks or environments
    // where permissions/member is unavailable
    let (response, is_fallback_member) = if response.status() == StatusCode::NOT_FOUND {
        let member_url = format!(
            "https://discord.com/api/v10/guilds/{guild_id}/members/{}",
            session.user.id
        );
        let resp2 = state
            .app
            .http_client
            .get(&member_url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bot {}", state.app.config.discord_token),
            )
            .send()
            .await
            .map_err(ApiError::upstream)?;
        (resp2, true)
    } else {
        (response, false)
    };

    if response.status() == StatusCode::NOT_FOUND || response.status() == StatusCode::FORBIDDEN {
        return Ok(false);
    }

    if !response.status().is_success() {
        // FAIL-CLOSED: Discord API error (e.g. 5xx/429); do not fall back to stale session snapshot
        tracing::warn!(
            status = %response.status(),
            guild_id,
            user_id = %session.user.id,
            "Discord API returned non-success when revalidating permissions; failing closed"
        );
        return Ok(false);
    }

    let member_json: serde_json::Value = response.json().await.map_err(ApiError::upstream)?;
    if let Some(perms_str) = member_json.get("permissions").and_then(|p| p.as_str())
        && let Ok(perms) = perms_str.parse::<u64>()
    {
        return Ok((perms & (ADMINISTRATOR | MANAGE_GUILD)) != 0);
    }

    // If fallback member endpoint confirmed membership and user is the recorded owner, permit
    if is_fallback_member && session.guilds.iter().any(|g| g.id == guild_id && g.owner) {
        return Ok(true);
    }

    // FAIL-CLOSED: Ambiguous member response (missing permissions field); deny write access
    tracing::warn!(
        guild_id,
        user_id = %session.user.id,
        "Discord API member response lacked permissions field; failing closed"
    );
    Ok(false)
}

pub async fn cleanup_stale_records(state: &WebState) -> (usize, usize, usize) {
    let states_removed = state
        .app
        .sessions
        .cleanup_expired_oauth_states()
        .await
        .unwrap_or(0);
    let sessions_removed = state
        .app
        .sessions
        .cleanup_expired_sessions()
        .await
        .unwrap_or(0);
    state.login_rate_limiter.cleanup_old_windows();

    let now = Utc::now();
    let before_perms = state.permissions_cache.len();
    let perms_cutoff = Duration::seconds(GUILD_PERMISSIONS_CACHE_TTL_SECONDS * 2);
    state
        .permissions_cache
        .retain(|_, entry| now - entry.checked_at <= perms_cutoff);
    let perms_removed = before_perms.saturating_sub(state.permissions_cache.len());

    (
        states_removed as usize,
        sessions_removed as usize,
        perms_removed,
    )
}

#[cfg(test)]
pub fn cleanup_stale_data(
    oauth_states: &DashMap<String, DateTime<Utc>>,
    sessions: &DashMap<String, Session>,
    limiter: &LoginRateLimiter,
    permissions_cache: &DashMap<(String, String), CachedGuildPermission>,
) -> (usize, usize, usize) {
    let now = Utc::now();
    let max_state_age = Duration::minutes(10);

    let before_states = oauth_states.len();
    oauth_states.retain(|_, created_at| now - *created_at <= max_state_age);
    let states_removed = before_states.saturating_sub(oauth_states.len());

    let before_sessions = sessions.len();
    sessions.retain(|_, session| session.expires_at > now);
    let sessions_removed = before_sessions.saturating_sub(sessions.len());

    limiter.cleanup_old_windows();

    let before_perms = permissions_cache.len();
    let perms_cutoff = Duration::seconds(GUILD_PERMISSIONS_CACHE_TTL_SECONDS * 2);
    permissions_cache.retain(|_, entry| now - entry.checked_at <= perms_cutoff);
    let perms_removed = before_perms.saturating_sub(permissions_cache.len());

    (states_removed, sessions_removed, perms_removed)
}

pub fn resolve_client_ip(
    peer_ip: Option<std::net::IpAddr>,
    headers: &HeaderMap,
    trusted_proxies: &std::collections::HashSet<std::net::IpAddr>,
) -> String {
    if let Some(peer) = peer_ip {
        if trusted_proxies.contains(&peer) {
            if let Some(forwarded) = headers
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.split(',').next())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .and_then(|s| s.parse::<std::net::IpAddr>().ok())
            {
                return forwarded.to_string();
            }
            if let Some(real_ip) = headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .and_then(|s| s.parse::<std::net::IpAddr>().ok())
            {
                return real_ip.to_string();
            }
        }
        peer.to_string()
    } else {
        headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split(',').next())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                headers
                    .get("x-real-ip")
                    .and_then(|v| v.to_str().ok())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or("127.0.0.1")
            .to_owned()
    }
}

fn can_manage(guild: &DashboardGuild) -> bool {
    if guild.owner {
        return true;
    }
    guild
        .permissions
        .parse::<u64>()
        .is_ok_and(|bits| bits & (ADMINISTRATOR | MANAGE_GUILD) != 0)
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then(|| value.to_owned()))
}

fn session_cookie(session_id: &str, max_age: i64, secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}={session_id}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{}",
        if secure { "; Secure" } else { "" }
    )
}

fn expired_session_cookie(secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{}",
        if secure { "; Secure" } else { "" }
    )
}

fn oauth_state_cookie(state: &str, secure: bool) -> String {
    format!(
        "{OAUTH_STATE_COOKIE}={state}; Path=/auth/discord; HttpOnly; SameSite=Lax; Max-Age=600{}",
        if secure { "; Secure" } else { "" }
    )
}

fn expired_oauth_state_cookie(secure: bool) -> String {
    format!(
        "{OAUTH_STATE_COOKIE}=; Path=/auth/discord; HttpOnly; SameSite=Lax; Max-Age=0{}",
        if secure { "; Secure" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn test_config() -> crate::config::Config {
        crate::config::Config {
            discord_token: "dummy-token".to_owned(),
            discord_application_id: 1,
            discord_client_id: "1".to_owned(),
            discord_client_secret: "secret".to_owned(),
            discord_oauth_redirect_url: "http://localhost/callback".to_owned(),
            discord_dev_guild_id: None,
            dashboard_bind: "127.0.0.1:8080".parse().unwrap(),
            dashboard_public_url: "http://localhost:8080".to_owned(),
            database_url: None,
            scripts_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts"),
            session_cookie_secure: false,
            session_ttl_seconds: 604800,
            lua_memory_limit_bytes: 4 * 1024 * 1024,
            lua_instruction_limit: 200_000,
            lua_hook_granularity: 100,
            lua_max_parallel: 32,
            lua_max_per_guild: 8,
            lua_command_timeout_ms: 3000,
            music_allowed_hosts: std::collections::HashSet::new(),
            trusted_proxies: ["127.0.0.1".parse().unwrap(), "::1".parse().unwrap()]
                .into_iter()
                .collect(),
            offline_mode: false,
            discord_api_base_url: "https://discord.com/api/v10".to_string(),
        }
    }

    #[test]
    fn test_login_rate_limiter_blocks_excessive_attempts() {
        let limiter = LoginRateLimiter::new();
        let ip = "192.168.1.100";
        for _ in 0..MAX_LOGIN_REQUESTS_PER_MINUTE_PER_IP {
            assert!(limiter.check_and_record(ip).is_ok());
        }
        let blocked = limiter.check_and_record(ip);
        assert!(blocked.is_err());
        assert_eq!(blocked.unwrap_err().status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn test_cleanup_stale_records() {
        let sessions = DashMap::new();
        let oauth_states = DashMap::new();
        let limiter = LoginRateLimiter::new();

        oauth_states.insert("active_state".to_owned(), Utc::now());
        oauth_states.insert("stale_state".to_owned(), Utc::now() - Duration::minutes(15));

        let user = DashboardUser {
            id: "123".to_owned(),
            username: "tester".to_owned(),
            global_name: None,
            avatar: None,
        };

        sessions.insert(
            "active_session".to_owned(),
            Session {
                user: user.clone(),
                guilds: vec![],
                csrf_token: "csrf1".to_owned(),
                expires_at: Utc::now() + Duration::hours(1),
                access_token: None,
            },
        );

        sessions.insert(
            "expired_session".to_owned(),
            Session {
                user,
                guilds: vec![],
                csrf_token: "csrf2".to_owned(),
                expires_at: Utc::now() - Duration::minutes(1),
                access_token: None,
            },
        );

        let perms_cache = DashMap::new();
        perms_cache.insert(
            ("123".to_owned(), "guild_stale".to_owned()),
            CachedGuildPermission {
                can_manage: true,
                checked_at: Utc::now() - Duration::minutes(10),
            },
        );

        let (st_cleaned, sess_cleaned, perms_cleaned) =
            cleanup_stale_data(&oauth_states, &sessions, &limiter, &perms_cache);
        assert_eq!(st_cleaned, 1);
        assert_eq!(sess_cleaned, 1);
        assert_eq!(perms_cleaned, 1);
        assert!(oauth_states.contains_key("active_state"));
        assert!(!oauth_states.contains_key("stale_state"));
        assert!(sessions.contains_key("active_session"));
        assert!(!sessions.contains_key("expired_session"));
        assert!(perms_cache.is_empty());
    }

    #[tokio::test]
    async fn test_ensure_guild_access_allows_and_revokes() {
        let app = crate::state::AppState::bootstrap(test_config())
            .await
            .expect("bootstrap");
        let permissions_cache = Arc::new(DashMap::new());
        let state = WebState {
            app,
            login_rate_limiter: Arc::new(LoginRateLimiter::new()),
            permissions_cache: permissions_cache.clone(),
        };

        let mut session = Session {
            user: DashboardUser {
                id: "user_42".to_owned(),
                username: "admin".to_owned(),
                global_name: None,
                avatar: None,
            },
            guilds: vec![DashboardGuild {
                id: "guild_1".to_owned(),
                name: "Server 1".to_owned(),
                icon: None,
                owner: false,
                permissions: "32".to_owned(), // MANAGE_GUILD
            }],
            csrf_token: "csrf_token".to_owned(),
            expires_at: Utc::now() + Duration::days(7),
            access_token: Some("mock_oauth_token".to_owned()),
        };

        // 1. Initial check: user has permission -> should succeed and populate cache
        let initial_check = ensure_guild_access(&state, &session, "guild_1").await;
        assert!(initial_check.is_ok());
        assert!(permissions_cache.contains_key(&("user_42".to_owned(), "guild_1".to_owned())));
        let cached = permissions_cache
            .get(&("user_42".to_owned(), "guild_1".to_owned()))
            .unwrap();
        assert!(cached.can_manage);
        drop(cached);

        // 2. Fast cache hit: still permitted
        let cache_hit = ensure_guild_access(&state, &session, "guild_1").await;
        assert!(cache_hit.is_ok());

        // 3. Admin revokes permission in Discord (simulated here by demoting user in guild permissions)
        session.guilds[0].permissions = "0".to_owned();

        // 4. While cache is fresh (< 60s), cache still allows
        let before_expiry = ensure_guild_access(&state, &session, "guild_1").await;
        assert!(before_expiry.is_ok());

        // 5. Expire cache entry (> 60s)
        permissions_cache.insert(
            ("user_42".to_owned(), "guild_1".to_owned()),
            CachedGuildPermission {
                can_manage: true,
                checked_at: Utc::now() - Duration::seconds(65),
            },
        );

        // 6. Next write/sensitive check revalidates and fails closed with 403 Forbidden!
        let after_expiry = ensure_guild_access(&state, &session, "guild_1").await;
        assert!(after_expiry.is_err());
        let err = after_expiry.unwrap_err();
        assert_eq!(err.status, StatusCode::FORBIDDEN);
        assert_eq!(
            err.message,
            "Permissions to manage this server have been revoked."
        );

        // 7. Verify cache now records can_manage = false
        let cached_revoked = permissions_cache
            .get(&("user_42".to_owned(), "guild_1".to_owned()))
            .unwrap();
        assert!(!cached_revoked.can_manage);
    }

    #[test]
    fn test_login_rate_limiter_cross_ip_isolation_and_atomic_budget() {
        let limiter = LoginRateLimiter::new();
        let ip_a = "198.51.100.1";
        let ip_b = "198.51.100.2";

        // Client A spams 60 requests
        for _ in 0..MAX_LOGIN_REQUESTS_PER_MINUTE_PER_IP {
            assert!(
                limiter.check_and_record(ip_a).is_ok(),
                "first 10 requests from client A must pass"
            );
        }

        // Next 50 requests from Client A are rejected by per-IP limit
        for _ in 0..50 {
            let blocked = limiter.check_and_record(ip_a);
            assert!(blocked.is_err(), "excess requests from A must be blocked");
            assert_eq!(blocked.unwrap_err().status, StatusCode::TOO_MANY_REQUESTS);
        }

        // Client B arrives with their first request:
        // Must SUCCEED because Client A's rejected attempts did not consume global budget!
        let client_b_attempt = limiter.check_and_record(ip_b);
        assert!(
            client_b_attempt.is_ok(),
            "independent client B must not be starved by client A's rejections"
        );
    }

    #[test]
    fn test_resolve_client_ip_direct_trusted_proxy_and_spoofed_header() {
        use std::collections::HashSet;
        use std::net::IpAddr;

        let trusted_proxies: HashSet<IpAddr> = [
            "127.0.0.1".parse().unwrap(),
            "10.0.0.1".parse().unwrap(),
            "::1".parse().unwrap(),
        ]
        .into_iter()
        .collect();

        // 1. Untrusted direct peer connecting directly with spoofed X-Forwarded-For
        let mut headers_spoofed = HeaderMap::new();
        headers_spoofed.insert("x-forwarded-for", "8.8.8.8".parse().unwrap());
        headers_spoofed.insert("x-real-ip", "8.8.8.8".parse().unwrap());

        let direct_peer: IpAddr = "198.51.100.50".parse().unwrap();
        let resolved = resolve_client_ip(Some(direct_peer), &headers_spoofed, &trusted_proxies);
        assert_eq!(
            resolved, "198.51.100.50",
            "untrusted peer socket address must be used, spoofed proxy headers ignored"
        );

        // 2. Direct connection without headers
        let headers_empty = HeaderMap::new();
        let direct_clean = resolve_client_ip(Some(direct_peer), &headers_empty, &trusted_proxies);
        assert_eq!(direct_clean, "198.51.100.50");

        // 3. Trusted reverse proxy forwarding real client IP
        let trusted_peer: IpAddr = "127.0.0.1".parse().unwrap();
        let mut headers_valid_proxy = HeaderMap::new();
        headers_valid_proxy.insert(
            "x-forwarded-for",
            "203.0.113.195, 10.0.0.1".parse().unwrap(),
        );

        let proxied = resolve_client_ip(Some(trusted_peer), &headers_valid_proxy, &trusted_proxies);
        assert_eq!(
            proxied, "203.0.113.195",
            "trusted proxy forwarded header must be extracted"
        );
    }

    #[test]
    fn test_oauth_state_cookie_formatting() {
        let cookie = oauth_state_cookie("test_state_123", true);
        assert_eq!(
            cookie,
            "rusty_oauth_state=test_state_123; Path=/auth/discord; HttpOnly; SameSite=Lax; Max-Age=600; Secure"
        );

        let cookie_insecure = oauth_state_cookie("test_state_123", false);
        assert_eq!(
            cookie_insecure,
            "rusty_oauth_state=test_state_123; Path=/auth/discord; HttpOnly; SameSite=Lax; Max-Age=600"
        );

        let expired = expired_oauth_state_cookie(true);
        assert_eq!(
            expired,
            "rusty_oauth_state=; Path=/auth/discord; HttpOnly; SameSite=Lax; Max-Age=0; Secure"
        );
    }

    #[tokio::test]
    async fn test_oauth_login_sets_binding_cookie() {
        let app = crate::state::AppState::bootstrap(test_config())
            .await
            .expect("bootstrap");
        let state = WebState {
            app,
            login_rate_limiter: Arc::new(LoginRateLimiter::new()),
            permissions_cache: Arc::new(DashMap::new()),
        };

        let headers = HeaderMap::new();
        let peer_addr = "127.0.0.1:12345".parse().unwrap();
        let response = login(
            State(state.clone()),
            axum::extract::ConnectInfo(peer_addr),
            headers,
        )
        .await
        .expect("login should succeed");

        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        let loc = response
            .headers()
            .get("location")
            .unwrap()
            .to_str()
            .unwrap();
        let set_cookie = response
            .headers()
            .get(SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(set_cookie.starts_with("rusty_oauth_state="));
        assert!(set_cookie.contains("Path=/auth/discord"));
        assert!(set_cookie.contains("HttpOnly"));
        assert!(set_cookie.contains("SameSite=Lax"));
        assert!(set_cookie.contains("Max-Age=600"));

        let url = Url::parse(loc).unwrap();
        let state_param = url
            .query_pairs()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.to_string())
            .unwrap();
        assert!(set_cookie.contains(&format!("rusty_oauth_state={state_param}")));
        assert!(
            state
                .app
                .sessions
                .consume_oauth_state(&state_param)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn test_oauth_callback_browser_binding_verification() {
        let app = crate::state::AppState::bootstrap(test_config())
            .await
            .expect("bootstrap");
        let state = WebState {
            app,
            login_rate_limiter: Arc::new(LoginRateLimiter::new()),
            permissions_cache: Arc::new(DashMap::new()),
        };

        let test_state = "valid-oauth-state-12345";
        state
            .app
            .sessions
            .save_oauth_state(test_state)
            .await
            .expect("save oauth state");

        // 1. Missing cookie -> rejected with 400 Bad Request
        let mut headers = HeaderMap::new();
        let query = OAuthCallback {
            code: "dummy_code".to_string(),
            state: test_state.to_string(),
        };
        let err = callback(State(state.clone()), headers.clone(), Query(query.clone()))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(
            err.message,
            "OAuth login was not initiated in this browser."
        );

        // 2. Mismatched cookie -> rejected with 400 Bad Request
        headers.insert(
            COOKIE,
            HeaderValue::from_static("rusty_oauth_state=wrong-state"),
        );
        let err = callback(State(state.clone()), headers.clone(), Query(query.clone()))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(
            err.message,
            "OAuth login was not initiated in this browser."
        );

        // State must NOT have been consumed by either failed call
        assert!(
            state
                .app
                .sessions
                .consume_oauth_state(test_state)
                .await
                .unwrap()
        );

        // 3. Matching cookie but unknown/expired state in store -> rejected with 400 Bad Request
        let mut non_existent_headers = HeaderMap::new();
        non_existent_headers.insert(
            COOKIE,
            HeaderValue::from_static("rusty_oauth_state=unknown-state"),
        );
        let query_unknown = OAuthCallback {
            code: "dummy_code".to_string(),
            state: "unknown-state".to_string(),
        };
        let err = callback(
            State(state.clone()),
            non_existent_headers,
            Query(query_unknown),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(
            err.message,
            "Invalid OAuth2 login state or session expired."
        );
    }
}
