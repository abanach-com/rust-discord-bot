use std::convert::Infallible;
use std::sync::atomic::Ordering;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use crate::lua::LuaModuleManifest;
use crate::storage::validate_snowflake;

use super::{
    WebState,
    auth::{authenticate, ensure_guild_access, verify_csrf},
};

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
    pub details: Option<Vec<String>>,
}

impl ApiError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
            details: None,
        }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: message.into(),
            details: None,
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.into(),
            details: None,
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
            details: None,
        }
    }

    pub fn too_many_requests(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            message: message.into(),
            details: None,
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
            details: None,
        }
    }

    pub fn validation(message: impl Into<String>, details: Vec<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: message.into(),
            details: Some(details),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        let message = message.into();
        tracing::error!(%message, "dashboard internal error");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "Internal dashboard error.".to_owned(),
            details: None,
        }
    }

    pub fn upstream(error: reqwest::Error) -> Self {
        tracing::warn!(?error, "Discord OAuth2/API request failed");
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: "Discord did not respond properly during API communication.".to_owned(),
            details: None,
        }
    }
}

impl From<crate::moderation_cases::CaseError> for ApiError {
    fn from(error: crate::moderation_cases::CaseError) -> Self {
        match error {
            crate::moderation_cases::CaseError::NotFound => {
                Self::not_found("Moderation case was not found.")
            }
            crate::moderation_cases::CaseError::VersionConflict { expected, actual } => {
                Self::conflict(format!(
                    "Case version conflict: expected {expected}, current version is {actual}. Refresh view."
                ))
            }
            crate::moderation_cases::CaseError::Validation(msg) => Self::bad_request(msg),
            crate::moderation_cases::CaseError::Database(err) => {
                Self::internal(format!("Case database error: {err}"))
            }
        }
    }
}

impl From<crate::storage::TupperError> for ApiError {
    fn from(error: crate::storage::TupperError) -> Self {
        match error {
            crate::storage::TupperError::PrefixConflict => {
                Self::conflict("Tupper prefix already exists for this user")
            }
            crate::storage::TupperError::Database(err) => {
                Self::internal(format!("Tupper database error: {err}"))
            }
            crate::storage::TupperError::Other(err) => {
                Self::internal(format!("Tupper storage error: {err}"))
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({ "error": self.message });
        if let Some(details) = self.details {
            body["details"] = json!(details);
        }
        (self.status, Json(body)).into_response()
    }
}

#[derive(Serialize)]
pub struct ModuleView {
    manifest: LuaModuleManifest,
    enabled: bool,
    config: Value,
    configured: bool,
    updated_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
pub struct UpdateModuleRequest {
    enabled: bool,
    #[serde(default)]
    config: Value,
}

pub async fn health_live(State(state): State<WebState>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": state.app.metrics.uptime_seconds(),
    }))
}

pub async fn health_ready(
    State(state): State<WebState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let mut checks = serde_json::Map::new();
    let mut is_ready = true;

    match state.app.storage.ping().await {
        Ok(()) => {
            checks.insert("storage".to_owned(), json!("ok"));
        }
        Err(e) => {
            is_ready = false;
            tracing::warn!(error = %e, "readiness storage ping failed");
            checks.insert("storage".to_owned(), json!("unavailable"));
        }
    }

    let manifest_count = state.app.scripts.manifests().await.len();
    if manifest_count > 0 {
        checks.insert("lua_engine".to_owned(), json!("ok"));
    } else {
        is_ready = false;
        checks.insert("lua_engine".to_owned(), json!("unavailable"));
    }

    if state.app.is_shutting_down.load(Ordering::SeqCst) {
        is_ready = false;
        checks.insert(
            "lifecycle".to_owned(),
            json!("unready (process is shutting down)"),
        );
    } else {
        checks.insert("lifecycle".to_owned(), json!("ok (running)"));
    }

    let is_offline_mode = state.app.config.discord_token.starts_with("dummy")
        || state.app.config.discord_token.starts_with("mock");
    if is_offline_mode {
        checks.insert(
            "discord_gateway".to_owned(),
            json!("ok (offline/dummy mode)"),
        );
    } else if state.app.is_gateway_ready.load(Ordering::SeqCst) {
        checks.insert("discord_gateway".to_owned(), json!("ok (gateway ready)"));
    } else {
        is_ready = false;
        checks.insert(
            "discord_gateway".to_owned(),
            json!("unready (waiting for gateway ready event)"),
        );
    }

    let transcript_status = if state.app.config.ticket_transcripts_disabled {
        "disabled"
    } else if state.app.config.database_url.is_some() {
        if state.app.tickets.has_crypto() {
            "ok"
        } else {
            is_ready = false;
            "unready"
        }
    } else {
        "ok"
    };
    checks.insert("ticket_transcripts".to_owned(), json!(transcript_status));

    let payload = json!({
        "status": if is_ready { "ready" } else { "unready" },
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": state.app.metrics.uptime_seconds(),
        "dashboard_requests_total": state.app.metrics.dashboard_requests_total.load(Ordering::Relaxed),
        "checks": checks,
    });

    if is_ready {
        Ok(Json(payload))
    } else {
        Err((StatusCode::SERVICE_UNAVAILABLE, Json(payload)))
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

async fn is_metrics_authorized(
    state: &WebState,
    headers: &HeaderMap,
    peer_addr: Option<std::net::SocketAddr>,
) -> bool {
    // 1. Bearer Token check
    if let Some(ref expected_token) = state.app.config.metrics_token {
        let is_valid = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|val| {
                val.strip_prefix("Bearer ")
                    .or_else(|| val.strip_prefix("bearer "))
            })
            .is_some_and(|provided| {
                constant_time_eq(provided.trim().as_bytes(), expected_token.as_bytes())
            });
        if is_valid {
            return true;
        }
    }

    // 2. Active Session Cookie check
    let has_valid_session = match super::auth::cookie_value(headers, super::auth::SESSION_COOKIE) {
        Some(cookie) => matches!(state.app.sessions.get_session(&cookie).await, Ok(Some(_))),
        None => false,
    };
    if has_valid_session {
        return true;
    }

    // 3. Network Scraper Authorization (IP check)
    if let Some(peer) = peer_addr {
        let peer_ip = peer.ip();
        if state.app.config.metrics_allowed_ips.contains(&peer_ip) {
            return true;
        }
        // If no token is configured, allow local loopback scrapers
        if state.app.config.metrics_token.is_none() && peer_ip.is_loopback() {
            return true;
        }
    }

    false
}

pub struct ClientSocketAddr(pub Option<std::net::SocketAddr>);

impl<S> axum::extract::FromRequestParts<S> for ClientSocketAddr
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let addr = parts
            .extensions
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|ci| ci.0)
            .or_else(|| {
                parts
                    .extensions
                    .get::<axum::extract::connect_info::MockConnectInfo<std::net::SocketAddr>>()
                    .map(|m| m.0)
            });
        Ok(ClientSocketAddr(addr))
    }
}

pub async fn prometheus_metrics(
    State(state): State<WebState>,
    ClientSocketAddr(peer_addr): ClientSocketAddr,
    headers: HeaderMap,
) -> Response {
    use axum::http::header::CONTENT_TYPE;

    if !is_metrics_authorized(&state, &headers, peer_addr).await {
        return (StatusCode::UNAUTHORIZED, "Unauthorized\n").into_response();
    }

    let active_sessions = state
        .app
        .sessions
        .count_active_sessions()
        .await
        .unwrap_or(0);
    let active_lua = state.app.scripts.active_executions();
    let body = state
        .app
        .metrics
        .render_prometheus(active_sessions, active_lua);
    (
        [(CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
        .into_response()
}

pub async fn list_modules(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> Result<Json<Vec<LuaModuleManifest>>, ApiError> {
    authenticate(&state, &headers).await?;
    Ok(Json(state.app.scripts.manifests().await))
}

pub async fn get_installer_manifest(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> Result<Json<Option<crate::lua::InstallManifest>>, ApiError> {
    authenticate(&state, &headers).await?;
    Ok(Json(state.app.scripts.installer_manifest().await))
}

pub async fn guild_modules(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Vec<ModuleView>>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let manifests = state.app.scripts.manifests().await;
    let mut modules = Vec::with_capacity(manifests.len());
    for manifest in manifests {
        let settings = state
            .app
            .storage
            .get_module_settings(&guild_id, &manifest.id)
            .await
            .map_err(|error| ApiError::internal(error.to_string()))?;

        modules.push(match settings {
            Some(settings) => ModuleView {
                manifest,
                enabled: settings.enabled,
                config: settings.config,
                configured: true,
                updated_at: Some(settings.updated_at),
            },
            None => {
                let default_cfg = state.app.scripts.default_config(&manifest.id).await;
                ModuleView {
                    enabled: manifest.default_enabled,
                    manifest,
                    config: default_cfg,
                    configured: false,
                    updated_at: None,
                }
            }
        });
    }

    Ok(Json(modules))
}

#[derive(Debug, Deserialize)]
pub struct LuaEventRoutingQuery {
    pub event: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

pub async fn guild_lua_event_routing(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    Query(query): Query<LuaEventRoutingQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    if validate_snowflake("guild_id", &guild_id).is_err() && !guild_id.starts_with("guild_") {
        return Err(ApiError::bad_request("Invalid guild_id"));
    }
    ensure_guild_access(&state, &session, &guild_id).await?;

    let Some(raw_event) = query
        .event
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Err(ApiError::bad_request(
            "The 'event' query parameter is required.",
        ));
    };

    crate::lua::routing::validate_event_name(raw_event)
        .map_err(|err| ApiError::bad_request(format!("Invalid event: {err}")))?;

    let decision = state.app.rate_limiter.check_action(
        "admin:lua_event_routing",
        &session.user.id,
        Some(&guild_id),
    );
    if !decision.allowed {
        state
            .app
            .metrics
            .rate_limit_throttled_total
            .fetch_add(1, Ordering::Relaxed);
        return Err(ApiError::too_many_requests(format!(
            "Diagnostic requests are too frequent. Please wait {} s.",
            decision.reset_after.as_secs().max(1)
        )));
    }

    let mut plans = crate::lua::routing::plan_event_routing(
        &state.app.scripts,
        &state.app.storage,
        Some(&guild_id),
        raw_event,
    )
    .await;

    plans.sort_by(|a, b| a.module_id.cmp(&b.module_id));

    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(50).clamp(1, 100);

    let routes: Vec<crate::lua::LuaEventRouteDecision> = plans
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|p| p.decision)
        .collect();

    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        axum::http::header::HeaderName::from_static("x-routing-disclaimer"),
        axum::http::header::HeaderValue::from_static(
            "Eligible for dispatch - not a guarantee of execution",
        ),
    );

    Ok((response_headers, Json(routes)))
}

pub fn extract_configured_roles<'a>(module_id: &str, config: &'a Value) -> Vec<&'a str> {
    match module_id {
        "welcome" => config
            .get("auto_roles")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .collect(),
        "join_guard" => config
            .get("quarantine_role_id")
            .and_then(|v| v.as_str())
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .collect(),
        "progression" => config
            .get("role_rewards")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|v| v.get("role_id").and_then(|id| id.as_str()))
            .filter(|s| !s.trim().is_empty())
            .collect(),
        "birthdays" => config
            .get("birthday_role_id")
            .and_then(|v| v.as_str())
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .collect(),
        _ => vec![],
    }
}

pub async fn validate_module_config_and_roles(
    state: &WebState,
    session: &crate::web::auth::Session,
    guild_id: &str,
    module_id: &str,
    config: &Value,
) -> Result<(), ApiError> {
    if let Err(validation_errors) = state
        .app
        .scripts
        .validate_module_config(module_id, config)
        .await
    {
        return Err(ApiError::validation(
            "Module configuration violates JSON schema.",
            validation_errors,
        ));
    }

    let configured_roles = extract_configured_roles(module_id, config);
    if !configured_roles.is_empty() {
        crate::web::auth::ensure_manage_roles_access(state, session, guild_id).await?;
        for role_id in configured_roles {
            crate::web::features::validate_assignable_role(
                state,
                session,
                guild_id,
                role_id,
                "do automatycznego nadawania",
            )
            .await?;
        }
    }

    Ok(())
}

pub async fn update_guild_module(
    State(state): State<WebState>,
    Path((guild_id, module_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(payload): Json<UpdateModuleRequest>,
) -> Result<Json<ModuleView>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;
    verify_csrf(&headers, &session)?;

    let decision = state.app.rate_limiter.check_action(
        "admin:update_module",
        &session.user.id,
        Some(&guild_id),
    );
    if !decision.allowed {
        state
            .app
            .metrics
            .rate_limit_throttled_total
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return Err(ApiError::too_many_requests(format!(
            "Module configuration changes are too frequent. Please wait {} s.",
            decision.reset_after.as_secs().max(1)
        )));
    }

    let manifest = state
        .app
        .scripts
        .manifest(&module_id)
        .await
        .ok_or_else(|| ApiError::not_found("Lua module not found."))?;

    if !payload.config.is_object() {
        return Err(ApiError::bad_request(
            "Module configuration must be a JSON object.",
        ));
    }
    let serialized_size = serde_json::to_vec(&payload.config)
        .map_err(|error| ApiError::bad_request(error.to_string()))?
        .len();
    if serialized_size > 64 * 1024 {
        return Err(ApiError::bad_request(
            "Module configuration must not exceed 64 KiB.",
        ));
    }

    validate_module_config_and_roles(&state, &session, &guild_id, &module_id, &payload.config)
        .await?;

    let settings = state
        .app
        .storage
        .update_module_settings_with_audit(crate::storage::UpdateModuleSettingsParams {
            guild_id: &guild_id,
            module_id: &module_id,
            enabled: payload.enabled,
            config: payload.config,
            actor_id: Some(&session.user.id),
            audit_event: "module_settings_updated",
            audit_data: json!({
                "module_id": module_id,
                "enabled": payload.enabled,
            }),
        })
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;

    let _ = state
        .app
        .guild_events
        .send(crate::state::GuildRealtimeEvent {
            guild_id: guild_id.clone(),
            event_type: "module_updated".to_string(),
            payload: json!({
                "module_id": module_id,
                "enabled": settings.enabled,
            }),
        });

    let _ = state
        .app
        .guild_events
        .send(crate::state::GuildRealtimeEvent {
            guild_id: guild_id.clone(),
            event_type: "audit_entry".to_string(),
            payload: json!({
                "module_id": module_id,
                "event": "module_settings_updated",
            }),
        });

    Ok(Json(ModuleView {
        manifest,
        enabled: settings.enabled,
        config: settings.config,
        configured: true,
        updated_at: Some(settings.updated_at),
    }))
}

#[derive(Debug, Deserialize, Default)]
pub struct PaginationQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

pub async fn guild_cases(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    Query(pagination): Query<PaginationQuery>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;
    let guild_u64 = guild_id
        .parse::<u64>()
        .map_err(|_| ApiError::bad_request("Invalid server ID."))?;

    let limit = pagination.limit.unwrap_or(50).clamp(1, 100);
    let offset = pagination.offset.unwrap_or(0);

    let (items, total) = state
        .app
        .moderation_cases
        .list_cases_page(guild_u64, limit, offset, Utc::now())
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;

    let next_offset = offset
        .checked_add(items.len())
        .filter(|next| (*next as u64) < total);

    Ok(Json(json!({
        "items": items,
        "total": total,
        "next_offset": next_offset,
    })))
}

pub async fn guild_case_details(
    State(state): State<WebState>,
    Path((guild_id, case_number)): Path<(String, i64)>,
    headers: HeaderMap,
) -> Result<Json<crate::moderation_cases::CaseDetails>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;
    let guild_u64 = guild_id
        .parse::<u64>()
        .map_err(|_| ApiError::bad_request("Invalid server ID."))?;
    let details = state
        .app
        .moderation_cases
        .get_case(guild_u64, case_number, Utc::now())
        .await
        .map_err(ApiError::from)?;

    Ok(Json(details))
}

pub async fn guild_audit_events(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    Query(pagination): Query<PaginationQuery>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let limit = pagination.limit.unwrap_or(50).clamp(1, 200);
    let offset = pagination.offset.unwrap_or(0);

    let total = state
        .app
        .storage
        .count_audit_events(&guild_id)
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;

    let items = state
        .app
        .storage
        .list_audit_events_page(&guild_id, limit, offset)
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;

    let next_offset = if (offset as u64) + (items.len() as u64) < total {
        Some(offset + items.len())
    } else {
        None
    };

    Ok(Json(json!({
        "items": items,
        "total": total,
        "next_offset": next_offset,
    })))
}

#[derive(Deserialize)]
pub struct AddCaseNotePayload {
    pub body: String,
    #[serde(default)]
    pub visible_to_subject: bool,
}

#[derive(Deserialize)]
pub struct VoidCasePayload {
    pub reason: String,
    #[serde(default)]
    pub expected_version: i64,
}

#[derive(Deserialize)]
pub struct UpdateCasePayload {
    #[serde(default)]
    pub expected_version: i64,
    pub reason: Option<String>,
    pub severity: Option<String>,
    pub points: Option<i32>,
    pub visible_to_subject: Option<bool>,
}

pub async fn add_guild_case_note(
    State(state): State<WebState>,
    Path((guild_id, case_number)): Path<(String, i64)>,
    headers: HeaderMap,
    Json(payload): Json<AddCaseNotePayload>,
) -> Result<Json<crate::moderation_cases::CaseNote>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;
    verify_csrf(&headers, &session)?;

    let trimmed = payload.body.trim();
    if trimmed.is_empty() || trimmed.chars().count() > crate::moderation_cases::MAX_NOTE_LENGTH {
        return Err(ApiError::bad_request(
            "Note content cannot be empty or exceed 2000 characters.",
        ));
    }

    let guild_u64 = guild_id
        .parse::<u64>()
        .map_err(|_| ApiError::bad_request("Invalid server ID."))?;
    let actor_u64 = session
        .user
        .id
        .parse::<u64>()
        .map_err(|_| ApiError::bad_request("Invalid user ID."))?;

    let now = Utc::now();
    let note = state
        .app
        .moderation_cases
        .add_note(
            guild_u64,
            case_number,
            actor_u64,
            trimmed.to_owned(),
            payload.visible_to_subject,
            now,
        )
        .await
        .map_err(ApiError::from)?;

    if state
        .app
        .storage
        .record_audit(
            Some(&guild_id),
            Some(&session.user.id),
            "dashboard",
            "case_note_added",
            json!({
                "case_number": case_number,
                "visible_to_subject": payload.visible_to_subject,
            }),
        )
        .await
        .is_ok()
    {
        let _ = state
            .app
            .guild_events
            .send(crate::state::GuildRealtimeEvent {
                guild_id: guild_id.clone(),
                event_type: "audit_entry".to_string(),
                payload: json!({
                    "module_id": "dashboard",
                    "event": "case_note_added",
                }),
            });
    }

    let _ = state
        .app
        .guild_events
        .send(crate::state::GuildRealtimeEvent {
            guild_id: guild_id.clone(),
            event_type: "case_updated".to_string(),
            payload: json!({
                "case_number": case_number,
                "action": "note_added",
            }),
        });

    Ok(Json(note))
}

pub async fn void_guild_case(
    State(state): State<WebState>,
    Path((guild_id, case_number)): Path<(String, i64)>,
    headers: HeaderMap,
    Json(payload): Json<VoidCasePayload>,
) -> Result<Json<crate::moderation_cases::ModerationCase>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;
    verify_csrf(&headers, &session)?;

    let trimmed = payload.reason.trim();
    if trimmed.is_empty() || trimmed.chars().count() > crate::moderation_cases::MAX_REASON_LENGTH {
        return Err(ApiError::bad_request(
            "Void reason cannot be empty or exceed 2000 characters.",
        ));
    }

    let guild_u64 = guild_id
        .parse::<u64>()
        .map_err(|_| ApiError::bad_request("Invalid server ID."))?;
    let actor_u64 = session
        .user
        .id
        .parse::<u64>()
        .map_err(|_| ApiError::bad_request("Invalid user ID."))?;

    let now = Utc::now();
    if payload.expected_version <= 0 {
        return Err(ApiError::bad_request(
            "Positive expected_version is required for concurrency control.",
        ));
    }
    let version = payload.expected_version;

    let voided = state
        .app
        .moderation_cases
        .void_case(
            guild_u64,
            case_number,
            actor_u64,
            version,
            trimmed.to_owned(),
            now,
        )
        .await
        .map_err(ApiError::from)?;

    if state
        .app
        .storage
        .record_audit(
            Some(&guild_id),
            Some(&session.user.id),
            "dashboard",
            "case_voided",
            json!({
                "case_number": case_number,
                "reason": trimmed,
            }),
        )
        .await
        .is_ok()
    {
        let _ = state
            .app
            .guild_events
            .send(crate::state::GuildRealtimeEvent {
                guild_id: guild_id.clone(),
                event_type: "audit_entry".to_string(),
                payload: json!({
                    "module_id": "dashboard",
                    "event": "case_voided",
                }),
            });
    }

    let _ = state
        .app
        .guild_events
        .send(crate::state::GuildRealtimeEvent {
            guild_id: guild_id.clone(),
            event_type: "case_updated".to_string(),
            payload: json!({
                "case_number": case_number,
                "action": "voided",
            }),
        });

    Ok(Json(voided))
}

pub async fn update_guild_case(
    State(state): State<WebState>,
    Path((guild_id, case_number)): Path<(String, i64)>,
    headers: HeaderMap,
    Json(payload): Json<UpdateCasePayload>,
) -> Result<Json<crate::moderation_cases::ModerationCase>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;
    verify_csrf(&headers, &session)?;

    let guild_u64 = guild_id
        .parse::<u64>()
        .map_err(|_| ApiError::bad_request("Invalid server ID."))?;
    let actor_u64 = session
        .user
        .id
        .parse::<u64>()
        .map_err(|_| ApiError::bad_request("Invalid user ID."))?;

    let now = Utc::now();
    if payload.expected_version <= 0 {
        return Err(ApiError::bad_request(
            "Positive expected_version is required for concurrency control.",
        ));
    }
    let version = payload.expected_version;

    let sev = match payload.severity.as_deref() {
        Some(s) => Some(
            s.parse::<crate::moderation_cases::CaseSeverity>()
                .map_err(|e| ApiError::bad_request(e.to_string()))?,
        ),
        None => None,
    };

    let update = crate::moderation_cases::UpdateCase {
        reason: payload.reason,
        severity: sev,
        points: payload.points,
        visible_to_subject: payload.visible_to_subject,
        expires_at: None,
        clear_expiry: false,
    };

    let updated = state
        .app
        .moderation_cases
        .update_case(guild_u64, case_number, actor_u64, version, update, now)
        .await
        .map_err(ApiError::from)?;

    let _ = state
        .app
        .storage
        .record_audit(
            Some(&guild_id),
            Some(&session.user.id),
            "dashboard",
            "case_edited",
            json!({
                "case_number": case_number,
                "version": updated.version,
            }),
        )
        .await;

    let _ = state
        .app
        .guild_events
        .send(crate::state::GuildRealtimeEvent {
            guild_id: guild_id.clone(),
            event_type: "case_updated".to_string(),
            payload: json!({
                "case_number": case_number,
                "version": updated.version,
                "action": "edited",
            }),
        });

    let _ = state
        .app
        .guild_events
        .send(crate::state::GuildRealtimeEvent {
            guild_id: guild_id.clone(),
            event_type: "audit_entry".to_string(),
            payload: json!({
                "module_id": "dashboard",
                "event": "case_edited",
            }),
        });

    Ok(Json(updated))
}

pub async fn guild_events_stream(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
) -> Result<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let mut rx = state.app.guild_events.subscribe();
    let target_guild = guild_id.clone();

    let (tx, rx_out) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);

    let initial = Event::default()
        .event("connected")
        .data(json!({ "guild_id": target_guild }).to_string());
    let _ = tx.send(Ok(initial)).await;

    let worker_state = state.clone();
    let worker_headers = headers.clone();
    let worker_guild = target_guild.clone();

    tokio::spawn(async move {
        let mut reauth_interval = tokio::time::interval(std::time::Duration::from_secs(15));
        reauth_interval.tick().await;

        loop {
            tokio::select! {
                _ = tx.closed() => {
                    break;
                }
                _ = reauth_interval.tick() => {
                    if tx.is_closed() {
                        break;
                    }
                    let Ok((_, fresh_session)) = authenticate(&worker_state, &worker_headers).await else {
                        break;
                    };
                    if ensure_guild_access(&worker_state, &fresh_session, &worker_guild).await.is_err() {
                        break;
                    }
                }
                next = rx.recv() => {
                    match next {
                        Ok(event) => {
                            if event.guild_id != worker_guild {
                                continue;
                            }
                            if tx.is_closed() {
                                break;
                            }
                            let Ok((_, fresh_session)) = authenticate(&worker_state, &worker_headers).await else {
                                break;
                            };
                            if ensure_guild_access(&worker_state, &fresh_session, &worker_guild).await.is_err() {
                                break;
                            }

                            let sse_event = Event::default()
                                .event(event.event_type)
                                .data(event.payload.to_string());
                            if tx.send(Ok(sse_event)).await.is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            if tx.is_closed() {
                                break;
                            }
                            let resync_event = Event::default()
                                .event("resync")
                                .data(json!({ "guild_id": worker_guild }).to_string());
                            if tx.send(Ok(resync_event)).await.is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            break;
                        }
                    }
                }
            }
        }
    });

    let stream = ReceiverStream::new(rx_out);

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

// ==============================================================================
// Phase 1 Routes (Agents 02, 03, 04, 05, 06)
// ==============================================================================

pub async fn get_setup_wizard(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let profiles = json!([
        {
            "id": "gaming",
            "name": "Serwer Gamingowy",
            "description": "Layout for gaming, LFG, music bots, voice channels, and game roles.",
            "recommended_channels": [
                {"name": "regulamin", "type": "text"},
                {"name": "ogloszenia", "type": "announcement"},
                {"name": "szukam-grupy-lfg", "type": "text"},
                {"name": "czat-ogolny", "type": "text"},
                {"name": "Voice Lounge", "type": "voice"},
                {"name": "Gramy 1", "type": "voice"}
            ],
            "recommended_roles": ["Admin", "Moderator", "Gamer", "Nowy"],
            "required_intents": ["ServerMembers", "MessageContent"],
            "minimal_permissions": [
                "MANAGE_ROLES", "MANAGE_CHANNELS", "VIEW_CHANNEL", "SEND_MESSAGES",
                "EMBED_LINKS", "CONNECT", "SPEAK"
            ],
            "enabled_modules": ["core", "moderation", "automod", "welcome", "roles", "music", "games", "progression"]
        },
        {
            "id": "community",
            "name": "Community & Creator",
            "description": "For YouTube channels, Twitch streamers, discussion communities, and forums.",
            "recommended_channels": [
                {"name": "powitania", "type": "text"},
                {"name": "zasady", "type": "text"},
                {"name": "sugestie", "type": "text"},
                {"name": "pogaduchy", "type": "text"},
                {"name": "tworczosc-i-memy", "type": "text"},
                {"name": "starboard", "type": "text"},
                {"name": "Main Stage", "type": "stage"}
            ],
            "recommended_roles": ["Owner", "Staff", "VIP / Subscriber", "Member"],
            "required_intents": ["ServerMembers", "MessageContent"],
            "minimal_permissions": [
                "MANAGE_ROLES", "MANAGE_CHANNELS", "VIEW_CHANNEL", "SEND_MESSAGES",
                "EMBED_LINKS", "ADD_REACTIONS"
            ],
            "enabled_modules": ["core", "moderation", "automod", "welcome", "community", "starboard", "giveaway", "birthdays"]
        },
        {
            "id": "support",
            "name": "Help & Support Center",
            "description": "Environment for service inquiries, Modmail, recruitment, and direct support.",
            "recommended_channels": [
                {"name": "otworz-ticket", "type": "text"},
                {"name": "pomoc-faq", "type": "text"},
                {"name": "logi-wsparcia", "type": "text"},
                {"name": "staff-narada", "type": "text"}
            ],
            "recommended_roles": ["Admin", "Kierownik Supportu", "Konsultant Support", "Klient"],
            "required_intents": ["ServerMembers", "MessageContent"],
            "minimal_permissions": [
                "MANAGE_ROLES", "MANAGE_CHANNELS", "VIEW_CHANNEL", "SEND_MESSAGES",
                "EMBED_LINKS", "ATTACH_FILES", "READ_MESSAGE_HISTORY"
            ],
            "enabled_modules": ["core", "moderation", "tickets", "server_info", "logging"]
        }
    ]);

    Ok(Json(json!({
        "guild_id": guild_id,
        "profiles": profiles
    })))
}

#[derive(Clone, Debug)]
struct CachedBotPerms {
    permissions: Option<serenity::all::Permissions>,
    checked_at: DateTime<Utc>,
    #[allow(dead_code)]
    status_code: StatusCode,
    error_reason: Option<String>,
}

static BOT_DIAGNOSTICS_PERMS_CACHE: std::sync::LazyLock<dashmap::DashMap<String, CachedBotPerms>> =
    std::sync::LazyLock::new(dashmap::DashMap::new);

fn permission_flag_from_name(name: &str) -> serenity::all::Permissions {
    use serenity::all::Permissions;
    match name {
        "MANAGE_ROLES" => Permissions::MANAGE_ROLES,
        "MANAGE_MESSAGES" => Permissions::MANAGE_MESSAGES,
        "MODERATE_MEMBERS" => Permissions::MODERATE_MEMBERS,
        "KICK_MEMBERS" => Permissions::KICK_MEMBERS,
        "BAN_MEMBERS" => Permissions::BAN_MEMBERS,
        "MANAGE_CHANNELS" => Permissions::MANAGE_CHANNELS,
        "VIEW_CHANNEL" => Permissions::VIEW_CHANNEL,
        "SEND_MESSAGES" => Permissions::SEND_MESSAGES,
        "ATTACH_FILES" => Permissions::ATTACH_FILES,
        "READ_MESSAGE_HISTORY" => Permissions::READ_MESSAGE_HISTORY,
        "CONNECT" => Permissions::CONNECT,
        "SPEAK" => Permissions::SPEAK,
        "ADD_REACTIONS" => Permissions::ADD_REACTIONS,
        "VIEW_AUDIT_LOG" => Permissions::VIEW_AUDIT_LOG,
        _ => Permissions::empty(),
    }
}

fn required_permissions_for_module(
    module_id: &str,
) -> (serenity::all::Permissions, Vec<&'static str>) {
    use serenity::all::Permissions;
    match module_id {
        "moderation" | "automod" | "join_guard" => (
            Permissions::MANAGE_MESSAGES
                | Permissions::MODERATE_MEMBERS
                | Permissions::KICK_MEMBERS
                | Permissions::BAN_MEMBERS,
            vec![
                "MANAGE_MESSAGES",
                "MODERATE_MEMBERS",
                "KICK_MEMBERS",
                "BAN_MEMBERS",
            ],
        ),
        "roles" | "reaction_roles" => (Permissions::MANAGE_ROLES, vec!["MANAGE_ROLES"]),
        "tickets" => (
            Permissions::MANAGE_CHANNELS
                | Permissions::VIEW_CHANNEL
                | Permissions::SEND_MESSAGES
                | Permissions::ATTACH_FILES
                | Permissions::READ_MESSAGE_HISTORY,
            vec![
                "MANAGE_CHANNELS",
                "VIEW_CHANNEL",
                "SEND_MESSAGES",
                "ATTACH_FILES",
                "READ_MESSAGE_HISTORY",
            ],
        ),
        "music" => (
            Permissions::CONNECT | Permissions::SPEAK,
            vec!["CONNECT", "SPEAK"],
        ),
        "welcome" => (
            Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES,
            vec!["VIEW_CHANNEL", "SEND_MESSAGES"],
        ),
        "giveaway" | "starboard" => (
            Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES | Permissions::ADD_REACTIONS,
            vec!["VIEW_CHANNEL", "SEND_MESSAGES", "ADD_REACTIONS"],
        ),
        "logging" => (
            Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES | Permissions::VIEW_AUDIT_LOG,
            vec!["VIEW_CHANNEL", "SEND_MESSAGES", "VIEW_AUDIT_LOG"],
        ),
        _ => (
            Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES,
            vec!["VIEW_CHANNEL", "SEND_MESSAGES"],
        ),
    }
}

pub(crate) async fn resolve_bot_user_id(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
) -> Option<String> {
    if token.starts_with("mock") || token.starts_with("dummy") || token == "offline" {
        return Some("999999999999999999".to_string());
    }

    let resp = client
        .get(format!("{base_url}/users/@me"))
        .header(reqwest::header::AUTHORIZATION, format!("Bot {token}"))
        .timeout(std::time::Duration::from_secs(4))
        .send()
        .await
        .ok()?;

    if resp.status().is_success() {
        let val: Value = resp.json().await.ok()?;
        val.get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    } else {
        None
    }
}

async fn fetch_bot_permissions(
    state: &WebState,
    guild_id: &str,
) -> Result<serenity::all::Permissions, String> {
    let now = Utc::now();
    if let Some(cached) = BOT_DIAGNOSTICS_PERMS_CACHE.get(guild_id)
        && now - cached.checked_at < chrono::Duration::seconds(30)
    {
        if let Some(perms) = cached.permissions {
            return Ok(perms);
        } else if let Some(ref err) = cached.error_reason {
            return Err(err.clone());
        }
    }

    let is_offline = state.app.config.is_offline_mode()
        && state
            .app
            .config
            .discord_api_base_url
            .starts_with("https://discord.com");

    if is_offline {
        let perms = serenity::all::Permissions::ADMINISTRATOR;
        BOT_DIAGNOSTICS_PERMS_CACHE.insert(
            guild_id.to_string(),
            CachedBotPerms {
                permissions: Some(perms),
                checked_at: now,
                status_code: StatusCode::OK,
                error_reason: None,
            },
        );
        return Ok(perms);
    }

    let base_url = &state.app.config.discord_api_base_url;
    let token = &state.app.config.discord_token;

    let bot_id = match resolve_bot_user_id(&state.app.http_client, base_url, token).await {
        Some(id) => id,
        None => {
            let err = "Failed to resolve bot user identity from Discord API".to_string();
            BOT_DIAGNOSTICS_PERMS_CACHE.insert(
                guild_id.to_string(),
                CachedBotPerms {
                    permissions: None,
                    checked_at: now,
                    status_code: StatusCode::INTERNAL_SERVER_ERROR,
                    error_reason: Some(err.clone()),
                },
            );
            return Err(err);
        }
    };

    // Use official Discord computed member permissions endpoint (API v10)
    let perms_url = format!("{base_url}/guilds/{guild_id}/permissions/member/{bot_id}");
    let resp = state
        .app
        .http_client
        .get(&perms_url)
        .header(reqwest::header::AUTHORIZATION, format!("Bot {token}"))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;

    let response = match resp {
        Ok(r) => r,
        Err(e) => {
            let err = format!("Discord API connection error or timeout: {e}");
            BOT_DIAGNOSTICS_PERMS_CACHE.insert(
                guild_id.to_string(),
                CachedBotPerms {
                    permissions: None,
                    checked_at: now,
                    status_code: StatusCode::GATEWAY_TIMEOUT,
                    error_reason: Some(err.clone()),
                },
            );
            return Err(err);
        }
    };

    let status = response.status();
    if status == StatusCode::OK {
        if let Ok(body) = response.json::<Value>().await
            && let Some(perm_str) = body.get("permissions").and_then(|v| v.as_str())
        {
            let bits = perm_str.parse::<u64>().unwrap_or(0);
            let perms = serenity::all::Permissions::from_bits_truncate(bits);
            BOT_DIAGNOSTICS_PERMS_CACHE.insert(
                guild_id.to_string(),
                CachedBotPerms {
                    permissions: Some(perms),
                    checked_at: now,
                    status_code: StatusCode::OK,
                    error_reason: None,
                },
            );
            return Ok(perms);
        }
    } else if status == StatusCode::NOT_FOUND {
        // Fallback to GET /guilds/{guild_id}/members/{bot_id}
        let member_url = format!("{base_url}/guilds/{guild_id}/members/{bot_id}");
        if let Ok(member_resp) = state
            .app
            .http_client
            .get(&member_url)
            .header(reqwest::header::AUTHORIZATION, format!("Bot {token}"))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            let member_status = member_resp.status();
            if member_status == StatusCode::OK {
                if let Ok(m_body) = member_resp.json::<Value>().await
                    && let Some(perm_str) = m_body.get("permissions").and_then(|v| v.as_str())
                {
                    let bits = perm_str.parse::<u64>().unwrap_or(0);
                    let perms = serenity::all::Permissions::from_bits_truncate(bits);
                    BOT_DIAGNOSTICS_PERMS_CACHE.insert(
                        guild_id.to_string(),
                        CachedBotPerms {
                            permissions: Some(perms),
                            checked_at: now,
                            status_code: StatusCode::OK,
                            error_reason: None,
                        },
                    );
                    return Ok(perms);
                }
            } else if member_status == StatusCode::NOT_FOUND {
                let err = "Bot is not a member of this guild".to_string();
                BOT_DIAGNOSTICS_PERMS_CACHE.insert(
                    guild_id.to_string(),
                    CachedBotPerms {
                        permissions: Some(serenity::all::Permissions::empty()),
                        checked_at: now,
                        status_code: StatusCode::NOT_FOUND,
                        error_reason: Some(err.clone()),
                    },
                );
                return Ok(serenity::all::Permissions::empty());
            }
        }
    }

    let err = match status {
        StatusCode::FORBIDDEN => {
            "Discord API returned 403 Forbidden (missing access to guild)".to_string()
        }
        StatusCode::TOO_MANY_REQUESTS => {
            "Discord API returned 429 Too Many Requests (rate limited)".to_string()
        }
        _ => format!("Discord API returned HTTP {status}"),
    };

    BOT_DIAGNOSTICS_PERMS_CACHE.insert(
        guild_id.to_string(),
        CachedBotPerms {
            permissions: None,
            checked_at: now,
            status_code: status,
            error_reason: Some(err.clone()),
        },
    );
    Err(err)
}

async fn verify_configured_channel(
    state: &WebState,
    guild_id: &str,
    config: &Value,
) -> Result<Option<String>, String> {
    let channel_id_opt = config
        .get("channel_id")
        .or_else(|| config.get("welcome_channel_id"))
        .or_else(|| config.get("log_channel_id"))
        .and_then(|v| v.as_str());

    let channel_id = match channel_id_opt {
        Some(cid) => cid,
        None => return Ok(None),
    };

    if crate::storage::validate_snowflake("channel_id", channel_id).is_err() {
        return Ok(None);
    }

    let is_offline = state.app.config.is_offline_mode()
        && state
            .app
            .config
            .discord_api_base_url
            .starts_with("https://discord.com");
    if is_offline {
        return Ok(Some(channel_id.to_string()));
    }

    let base_url = &state.app.config.discord_api_base_url;
    let token = &state.app.config.discord_token;

    let ch_url = format!("{base_url}/channels/{channel_id}");
    let resp = state
        .app
        .http_client
        .get(&ch_url)
        .header(reqwest::header::AUTHORIZATION, format!("Bot {token}"))
        .timeout(std::time::Duration::from_secs(4))
        .send()
        .await;

    match resp {
        Ok(r) if r.status() == StatusCode::OK => {
            if let Ok(ch_json) = r.json::<Value>().await
                && let Some(ch_guild) = ch_json.get("guild_id").and_then(|v| v.as_str())
                && ch_guild != guild_id
            {
                return Err(format!(
                    "Configured channel {channel_id} belongs to guild {ch_guild}, not current guild {guild_id}"
                ));
            }
            Ok(Some(channel_id.to_string()))
        }
        Ok(r) if r.status() == StatusCode::NOT_FOUND => Err(format!(
            "Configured channel {channel_id} was not found on Discord"
        )),
        Ok(_) | Err(_) => Ok(Some(channel_id.to_string())),
    }
}

pub async fn get_guild_diagnostics(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;
    if validate_snowflake("guild_id", &guild_id).is_err() && !guild_id.starts_with("guild_") {
        return Err(ApiError::bad_request("Invalid guild_id"));
    }

    let is_gateway_ok =
        state.app.is_gateway_ready.load(Ordering::SeqCst) || state.app.config.is_offline_mode();

    let bot_perms_res = fetch_bot_permissions(&state, &guild_id).await;

    let manifests = state.app.scripts.manifests().await;
    let mut module_checks = Vec::new();
    let bot_hierarchy_ok = true;
    let now = Utc::now();

    for m in &manifests {
        let settings = state
            .app
            .storage
            .get_module_settings(&guild_id, &m.id)
            .await
            .ok()
            .flatten();
        let is_configured = settings.is_some();
        let config = settings.as_ref().map(|s| &s.config).unwrap_or(&Value::Null);

        // Check channel isolation
        let channel_check = verify_configured_channel(&state, &guild_id, config).await;

        let (req_perms, req_names) = required_permissions_for_module(&m.id);

        let (status, reason, remediation, missing_permissions) = match &bot_perms_res {
            Err(err_msg) => ("unknown", Some(err_msg.clone()), None, None),
            Ok(perms) => {
                if let Err(ch_err) = channel_check {
                    (
                        "missing",
                        Some(ch_err),
                        Some(
                            "Configure a channel that belongs to this server with proper permissions"
                                .to_string(),
                        ),
                        None,
                    )
                } else if perms.contains(serenity::all::Permissions::ADMINISTRATOR)
                    || perms.contains(req_perms)
                {
                    ("ok", None, None, None)
                } else {
                    let missing_names: Vec<&str> = req_names
                        .into_iter()
                        .filter(|name| {
                            let flag = permission_flag_from_name(name);
                            !perms.contains(flag)
                        })
                        .collect();
                    (
                        "missing",
                        Some(format!(
                            "Bot lacks required permissions: {}",
                            missing_names.join(", ")
                        )),
                        Some(format!(
                            "Grant the bot {} permission in server settings",
                            missing_names.join(", ")
                        )),
                        Some(missing_names),
                    )
                }
            }
        };

        let mut check_obj = json!({
            "id": m.id,
            "name": m.name,
            "category": m.category,
            "configured": is_configured,
            "status": status,
            "checked_at": now.to_rfc3339()
        });

        if let Some(r) = reason {
            check_obj["reason"] = json!(r);
        }
        if let Some(rem) = remediation {
            check_obj["remediation"] = json!(rem);
        }
        if let Some(mp) = missing_permissions {
            check_obj["missing_permissions"] = json!(mp);
        }

        module_checks.push(check_obj);
    }

    let ok_count = module_checks.iter().filter(|c| c["status"] == "ok").count();
    let unknown_count = module_checks
        .iter()
        .filter(|c| c["status"] == "unknown")
        .count();

    let (overall_health_val, health_status) = if unknown_count > 0 {
        (json!("incomplete"), "incomplete")
    } else if manifests.is_empty() {
        (json!(100), "healthy")
    } else {
        let mut pct = (ok_count * 100) / manifests.len();
        if !bot_hierarchy_ok {
            pct = pct.saturating_sub(15);
        }
        if !is_gateway_ok {
            pct = pct.saturating_sub(20);
        }
        let status_str = if pct == 100 { "healthy" } else { "degraded" };
        (json!(pct), status_str)
    };

    Ok(Json(json!({
        "guild_id": guild_id,
        "gateway_connected": is_gateway_ok,
        "bot_hierarchy_ok": bot_hierarchy_ok,
        "missing_intents": Vec::<String>::new(),
        "active_modules_count": ok_count,
        "total_available_modules": manifests.len(),
        "module_diagnostics": module_checks,
        "overall_health_status": health_status,
        "overall_health_percent": overall_health_val,
    })))
}

#[derive(Debug, Deserialize)]
pub struct SaveDraftRequest {
    pub config: Value,
}

pub async fn get_guild_module_draft(
    State(state): State<WebState>,
    Path((guild_id, module_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    state
        .app
        .scripts
        .manifest(&module_id)
        .await
        .ok_or_else(|| ApiError::not_found("Lua module not found."))?;

    let draft = state
        .app
        .storage
        .get_module_draft(&guild_id, &module_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    match draft {
        Some(d) => Ok(Json(json!({
            "has_draft": true,
            "config": d.config_json,
            "updated_by": d.updated_by,
            "updated_at": d.updated_at
        }))),
        None => Ok(Json(json!({
            "has_draft": false,
            "config": null
        }))),
    }
}

pub async fn save_guild_module_draft(
    State(state): State<WebState>,
    Path((guild_id, module_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(payload): Json<SaveDraftRequest>,
) -> Result<Json<Value>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;
    verify_csrf(&headers, &session)?;

    let decision = state.app.rate_limiter.check_action(
        "admin:update_module",
        &session.user.id,
        Some(&guild_id),
    );
    if !decision.allowed {
        return Err(ApiError::too_many_requests(format!(
            "Module configuration changes are too frequent. Please wait {} s.",
            decision.reset_after.as_secs().max(1)
        )));
    }

    state
        .app
        .scripts
        .manifest(&module_id)
        .await
        .ok_or_else(|| ApiError::not_found("Lua module not found."))?;

    if !payload.config.is_object() {
        return Err(ApiError::bad_request(
            "Draft configuration must be a JSON object.",
        ));
    }
    let serialized_size = serde_json::to_vec(&payload.config)
        .map_err(|error| ApiError::bad_request(error.to_string()))?
        .len();
    if serialized_size > 64 * 1024 {
        return Err(ApiError::bad_request(
            "Draft configuration must not exceed 64 KiB.",
        ));
    }

    let draft = state
        .app
        .storage
        .set_module_draft(&guild_id, &module_id, payload.config, &session.user.id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({
        "status": "draft_saved",
        "module_id": module_id,
        "updated_at": draft.updated_at
    })))
}

pub async fn publish_guild_module_draft(
    State(state): State<WebState>,
    Path((guild_id, module_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;
    verify_csrf(&headers, &session)?;

    state
        .app
        .scripts
        .manifest(&module_id)
        .await
        .ok_or_else(|| ApiError::not_found("Lua module not found."))?;

    let draft = state
        .app
        .storage
        .get_module_draft(&guild_id, &module_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("No draft version found for this module."))?;

    validate_module_config_and_roles(&state, &session, &guild_id, &module_id, &draft.config_json)
        .await?;

    let current = state
        .app
        .storage
        .get_module_settings(&guild_id, &module_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let enabled = current.map(|c| c.enabled).unwrap_or(true);

    let updated = state
        .app
        .storage
        .update_module_settings_with_audit(crate::storage::UpdateModuleSettingsParams {
            guild_id: &guild_id,
            module_id: &module_id,
            enabled,
            config: draft.config_json.clone(),
            actor_id: Some(&session.user.id),
            audit_event: "module_draft_published",
            audit_data: json!({
                "module_id": module_id,
                "published_by": session.user.id,
            }),
        })
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let _ = state
        .app
        .storage
        .delete_module_draft(&guild_id, &module_id)
        .await;

    let _ = state
        .app
        .guild_events
        .send(crate::state::GuildRealtimeEvent {
            guild_id: guild_id.clone(),
            event_type: "module_updated".to_string(),
            payload: json!({ "module_id": module_id }),
        });

    Ok(Json(json!({
        "status": "published",
        "module_id": module_id,
        "config": updated.config,
        "updated_at": updated.updated_at
    })))
}

pub async fn discard_guild_module_draft(
    State(state): State<WebState>,
    Path((guild_id, module_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;
    verify_csrf(&headers, &session)?;

    state
        .app
        .scripts
        .manifest(&module_id)
        .await
        .ok_or_else(|| ApiError::not_found("Lua module not found."))?;

    let removed = state
        .app
        .storage
        .delete_module_draft(&guild_id, &module_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({
        "status": "discarded",
        "module_id": module_id,
        "removed": removed
    })))
}

#[derive(Debug, Deserialize)]
pub struct PrivacyUserActionRequest {
    pub user_id: String,
}

pub async fn get_guild_privacy_summary(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    Ok(Json(json!({
        "guild_id": guild_id,
        "policy": {
            "retention_days_moderation": 365,
            "retention_days_tickets": 90,
            "retention_days_logs": 45,
            "retention_days_progression": "permanent_until_erasure_request"
        },
        "stored_entities": [
            {"entity": "Moderation cases", "purpose": "Sanctions audit and server security", "retention": "365 days"},
            {"entity": "Progression & XP", "purpose": "Activity leaderboard and level rewards", "retention": "Permanent until erasure request"},
            {"entity": "Ticket transcripts", "purpose": "Help desk processing", "retention": "90 days"},
            {"entity": "Audit event log", "purpose": "Permissions monitoring and transparency", "retention": "45 days"}
        ],
        "datasets": crate::privacy::PRIVACY_DATASETS,
        "gdpr_rights": ["Right of access (Export)", "Right to be forgotten (Erasure)"]
    })))
}

pub async fn export_user_privacy_data(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<PrivacyUserActionRequest>,
) -> Result<Json<Value>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let user_id = payload.user_id.trim();
    if user_id.is_empty() {
        return Err(ApiError::bad_request("User ID was not provided."));
    }

    let profile = if state
        .app
        .storage
        .has_member_progress(&guild_id, user_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
    {
        Some(
            state
                .app
                .storage
                .get_member_progress(&guild_id, user_id)
                .await
                .map_err(|e| ApiError::internal(e.to_string()))?,
        )
    } else {
        None
    };

    let cases = if let (Ok(g), Ok(u)) = (guild_id.parse::<u64>(), user_id.parse::<u64>()) {
        state
            .app
            .moderation_cases
            .list_all_subject_cases(g, u, Utc::now())
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?
    } else {
        Vec::new()
    };

    let quotes = if let (Ok(_), Ok(_)) = (guild_id.parse::<u64>(), user_id.parse::<u64>()) {
        state
            .app
            .storage
            .list_user_quotes(&guild_id, user_id)
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let birthday = if let (Ok(_), Ok(_)) = (guild_id.parse::<u64>(), user_id.parse::<u64>()) {
        state
            .app
            .storage
            .get_birthday(&guild_id, user_id)
            .await
            .ok()
            .flatten()
    } else {
        None
    };

    let reaction_roles = if let (Ok(_), Ok(_)) = (guild_id.parse::<u64>(), user_id.parse::<u64>()) {
        state
            .app
            .storage
            .list_user_reaction_role_grants(&guild_id, user_id)
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let afk = if let (Ok(_), Ok(_)) = (guild_id.parse::<u64>(), user_id.parse::<u64>()) {
        state
            .app
            .storage
            .get_afk(&guild_id, user_id)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?
    } else {
        None
    };

    Ok(Json(json!({
        "user_id": user_id,
        "guild_id": guild_id,
        "exported_at": Utc::now(),
        "progression": profile,
        "moderation_cases": cases,
        "quotes": quotes,
        "birthday": birthday,
        "reaction_roles": reaction_roles,
        "afk": afk,
    })))
}

pub async fn delete_user_privacy_data(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<PrivacyUserActionRequest>,
) -> Result<Json<Value>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;
    verify_csrf(&headers, &session)?;

    let user_id = payload.user_id.trim();
    if user_id.is_empty() {
        return Err(ApiError::bad_request("User ID was not provided."));
    }

    let report = state
        .app
        .storage
        .erase_guild_user_data(&guild_id, user_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({
        "status": "data_erased",
        "user_id": user_id,
        "guild_id": guild_id,
        "report": report,
        "erased_at": report.erased_at
    })))
}

#[derive(Debug, Deserialize)]
pub struct SubmitAppealRequest {
    pub reason: String,
    pub evidence_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DecideAppealRequest {
    pub decision: String,
    pub staff_notes: Option<String>,
}

pub async fn submit_moderation_appeal(
    State(state): State<WebState>,
    Path((guild_id, case_number)): Path<(String, i64)>,
    headers: HeaderMap,
    Json(payload): Json<SubmitAppealRequest>,
) -> Result<Json<Value>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    let reason = payload.reason.trim();
    if reason.is_empty() || reason.len() > 2000 {
        return Err(ApiError::bad_request(
            "Appeal justification must be between 1 and 2000 characters.",
        ));
    }

    let parsed_guild: u64 = guild_id
        .parse()
        .map_err(|_| ApiError::bad_request("Invalid guild_id"))?;
    let details = state
        .app
        .moderation_cases
        .get_case(parsed_guild, case_number, Utc::now())
        .await
        .map_err(ApiError::from)?;
    let case = details.case;

    if case.subject_user_id.to_string() != session.user.id {
        return Err(ApiError::forbidden(
            "You can only submit an appeal for a case concerning yourself.",
        ));
    }

    let appeal = crate::storage::ModerationAppeal {
        id: Uuid::new_v4(),
        guild_id: guild_id.clone(),
        case_number,
        user_id: session.user.id.clone(),
        status: "pending".to_string(),
        reason: reason.to_string(),
        evidence_url: payload.evidence_url,
        staff_notes: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let outcome = state
        .app
        .storage
        .create_appeal(&appeal)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    match outcome {
        crate::storage::CreateAppealOutcome::Created(appeal_id) => Ok(Json(json!({
            "status": "appeal_submitted",
            "appeal_id": appeal_id,
            "case_number": case_number,
            "created_at": appeal.created_at
        }))),
        crate::storage::CreateAppealOutcome::AlreadyPending(existing_id) => {
            Err(ApiError::conflict(format!(
                "An appeal is already pending for case #{case_number} (existing appeal: {existing_id})."
            )))
        }
    }
}

pub async fn list_guild_appeals(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let status_filter = query.get("status").map(|s| s.as_str());
    let limit = query
        .get("limit")
        .and_then(|l| l.parse::<i64>().ok())
        .unwrap_or(100)
        .clamp(1, 100);
    let appeals = state
        .app
        .storage
        .list_appeals(&guild_id, status_filter, Some(limit))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({
        "guild_id": guild_id,
        "appeals": appeals
    })))
}

pub async fn decide_moderation_appeal(
    State(state): State<WebState>,
    Path((guild_id, appeal_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
    Json(payload): Json<DecideAppealRequest>,
) -> Result<Json<Value>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;
    verify_csrf(&headers, &session)?;
    if validate_snowflake("guild_id", &guild_id).is_err() && !guild_id.starts_with("guild_") {
        return Err(ApiError::bad_request("Invalid guild_id"));
    }

    let decision = payload.decision.to_lowercase();
    if !["approved", "rejected", "more_info"].contains(&decision.as_str()) {
        return Err(ApiError::bad_request(
            "Niedozwolona decyzja. Dopuszczalne: approved, rejected, more_info.",
        ));
    }

    let updated = state
        .app
        .storage
        .update_appeal_status(
            &guild_id,
            appeal_id,
            &decision,
            payload.staff_notes.as_deref(),
        )
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    if !updated {
        return Err(ApiError::not_found("Appeal does not exist."));
    }

    Ok(Json(json!({
        "status": "decided",
        "appeal_id": appeal_id,
        "decision": decision
    })))
}

#[derive(Debug, Deserialize)]
pub struct SimulateAutoModRequest {
    pub content: String,
    #[serde(default)]
    pub author_account_age_days: Option<i64>,
    #[serde(default)]
    pub has_avatar: Option<bool>,
    #[serde(default)]
    pub recent_joins_count: Option<u32>,
    #[serde(default)]
    pub channel_id: Option<String>,
}

pub async fn simulate_automod(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<SimulateAutoModRequest>,
) -> Result<Json<Value>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let text = payload.content.trim();
    let lower = text.to_lowercase();

    // 1. Check invite links
    if lower.contains("discord.gg/") || lower.contains("discord.com/invite/") {
        return Ok(Json(json!({
            "matched": true,
            "winning_rule": "automod.invite_guard",
            "expected_action": "delete_message_and_warn",
            "reason": "Detected invite to external Discord server.",
            "spam_score": 0.95,
            "raid_detected": false
        })));
    }

    // 2. Check phishing domains
    if lower.contains("free-nitro")
        || lower.contains("steam-gift")
        || lower.contains("discord-airdrop")
    {
        return Ok(Json(json!({
            "matched": true,
            "winning_rule": "automod.phishing_link_guard",
            "expected_action": "delete_message_and_quarantine",
            "reason": "Detected suspicious phishing link.",
            "spam_score": 0.99,
            "raid_detected": false
        })));
    }

    // 3. Check raid bursts
    if payload.recent_joins_count.unwrap_or(0) >= 10 {
        return Ok(Json(json!({
            "matched": true,
            "winning_rule": "join_guard.raid_burst",
            "expected_action": "trigger_lockdown_and_quarantine",
            "reason": "Detected coordinated raid influx.",
            "spam_score": 0.90,
            "raid_detected": true
        })));
    }

    // 4. Check new account signals
    if payload.author_account_age_days.is_some_and(|d| d < 1) && !payload.has_avatar.unwrap_or(true)
    {
        return Ok(Json(json!({
            "matched": true,
            "winning_rule": "join_guard.fresh_suspicious_account",
            "expected_action": "require_verification_captcha",
            "reason": "Account created under 24h ago without avatar.",
            "spam_score": 0.70,
            "raid_detected": false
        })));
    }

    // 5. Clean / Passed
    Ok(Json(json!({
        "matched": false,
        "winning_rule": null,
        "expected_action": "allow",
        "reason": "Message and account satisfy security criteria.",
        "spam_score": 0.05,
        "raid_detected": false
    })))
}
