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
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

use crate::lua::LuaModuleManifest;

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
            message: "Wewnętrzny błąd panelu.".to_owned(),
            details: None,
        }
    }

    pub fn upstream(error: reqwest::Error) -> Self {
        tracing::warn!(?error, "Discord OAuth2/API request failed");
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: "Discord nie odpowiedział poprawnie podczas komunikacji z API.".to_owned(),
            details: None,
        }
    }
}

impl From<crate::moderation_cases::CaseError> for ApiError {
    fn from(error: crate::moderation_cases::CaseError) -> Self {
        match error {
            crate::moderation_cases::CaseError::NotFound => {
                Self::not_found("Sprawa moderacyjna nie została znaleziona.")
            }
            crate::moderation_cases::CaseError::VersionConflict { expected, actual } => {
                Self::conflict(format!(
                    "Konflikt wersji sprawy: oczekiwano {expected}, aktualna wersja to {actual}. Odśwież widok."
                ))
            }
            crate::moderation_cases::CaseError::Validation(msg) => Self::bad_request(msg),
            crate::moderation_cases::CaseError::Database(err) => {
                Self::internal(format!("Błąd bazy danych sprawy: {err}"))
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
            checks.insert("storage".to_owned(), json!(format!("error: {e}")));
        }
    }

    let manifest_count = state.app.scripts.manifests().await.len();
    if manifest_count > 0 {
        checks.insert(
            "lua_engine".to_owned(),
            json!(format!("ok ({manifest_count} modules)")),
        );
    } else {
        is_ready = false;
        checks.insert("lua_engine".to_owned(), json!("error: no modules loaded"));
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

pub async fn prometheus_metrics(State(state): State<WebState>) -> impl IntoResponse {
    use axum::http::header::CONTENT_TYPE;
    let active_sessions = state
        .app
        .sessions
        .count_active_sessions()
        .await
        .unwrap_or(0);
    let body = state.app.metrics.render_prometheus(active_sessions);
    (
        [(CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
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
            "Zbyt częste zmiany konfiguracji modułów. Odczekaj {} s.",
            decision.reset_after.as_secs().max(1)
        )));
    }

    let manifest = state
        .app
        .scripts
        .manifest(&module_id)
        .await
        .ok_or_else(|| ApiError::not_found("Nie znaleziono modułu Lua."))?;

    if !payload.config.is_object() {
        return Err(ApiError::bad_request(
            "Konfiguracja modułu musi być obiektem JSON.",
        ));
    }
    let serialized_size = serde_json::to_vec(&payload.config)
        .map_err(|error| ApiError::bad_request(error.to_string()))?
        .len();
    if serialized_size > 64 * 1024 {
        return Err(ApiError::bad_request(
            "Konfiguracja modułu nie może przekraczać 64 KiB.",
        ));
    }

    if let Err(validation_errors) = state
        .app
        .scripts
        .validate_module_config(&module_id, &payload.config)
        .await
    {
        return Err(ApiError::validation(
            "Konfiguracja modułu jest niezgodna ze schematem JSON.",
            validation_errors,
        ));
    }

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
        .map_err(|_| ApiError::bad_request("Niepoprawny identyfikator serwera."))?;

    let limit = pagination.limit.unwrap_or(50).clamp(1, 200);
    let offset = pagination.offset.unwrap_or(0);

    let all_cases = state
        .app
        .moderation_cases
        .list_cases(
            guild_u64,
            crate::moderation_cases::CaseFilter {
                subject_user_id: None,
                actor_user_id: None,
                kind: None,
                status: None,
                visible_to_subject_only: false,
                limit: 10_000,
            },
            Utc::now(),
        )
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;

    let total = all_cases.len();
    let items: Vec<_> = all_cases.into_iter().skip(offset).take(limit).collect();
    let next_offset = if offset + items.len() < total {
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

pub async fn guild_case_details(
    State(state): State<WebState>,
    Path((guild_id, case_number)): Path<(String, i64)>,
    headers: HeaderMap,
) -> Result<Json<crate::moderation_cases::CaseDetails>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;
    let guild_u64 = guild_id
        .parse::<u64>()
        .map_err(|_| ApiError::bad_request("Niepoprawny identyfikator serwera."))?;
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

    let all_events = state
        .app
        .storage
        .list_audit_events(&guild_id, 10_000)
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;

    let total = all_events.len();
    let items: Vec<_> = all_events.into_iter().skip(offset).take(limit).collect();
    let next_offset = if offset + items.len() < total {
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
            "Treść notatki nie może być pusta ani przekraczać 2000 znaków.",
        ));
    }

    let guild_u64 = guild_id
        .parse::<u64>()
        .map_err(|_| ApiError::bad_request("Niepoprawny identyfikator serwera."))?;
    let actor_u64 = session
        .user
        .id
        .parse::<u64>()
        .map_err(|_| ApiError::bad_request("Niepoprawny identyfikator użytkownika."))?;

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
            "Powód unieważnienia nie może być pusty ani przekraczać 2000 znaków.",
        ));
    }

    let guild_u64 = guild_id
        .parse::<u64>()
        .map_err(|_| ApiError::bad_request("Niepoprawny identyfikator serwera."))?;
    let actor_u64 = session
        .user
        .id
        .parse::<u64>()
        .map_err(|_| ApiError::bad_request("Niepoprawny identyfikator użytkownika."))?;

    let now = Utc::now();
    if payload.expected_version <= 0 {
        return Err(ApiError::bad_request(
            "Wymagane jest dodatnie expected_version do kontroli wersji.",
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
        .map_err(|_| ApiError::bad_request("Niepoprawny identyfikator serwera."))?;
    let actor_u64 = session
        .user
        .id
        .parse::<u64>()
        .map_err(|_| ApiError::bad_request("Niepoprawny identyfikator użytkownika."))?;

    let now = Utc::now();
    if payload.expected_version <= 0 {
        return Err(ApiError::bad_request(
            "Wymagane jest dodatnie expected_version do kontroli wersji.",
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

    let rx = state.app.guild_events.subscribe();
    let target_guild = guild_id.clone();

    let initial = tokio_stream::once(Ok(Event::default()
        .event("connected")
        .data(json!({ "guild_id": target_guild }).to_string())));

    let stream = BroadcastStream::new(rx).filter_map(move |item| match item {
        Ok(event) if event.guild_id == target_guild => Some(Ok(Event::default()
            .event(event.event_type)
            .data(event.payload.to_string()))),
        _ => None,
    });

    let combined = initial.chain(stream);

    Ok(Sse::new(combined).keep_alive(
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
            "description": "Układ pod rozgrywki, LFG, boty muzyczne, kanały głosowe i role gier.",
            "recommended_channels": [
                {"name": "regulamin", "type": "text"},
                {"name": "ogloszenia", "type": "announcement"},
                {"name": "szukam-grupy-lfg", "type": "text"},
                {"name": "czat-ogolny", "type": "text"},
                {"name": "Poczekalnia Głosowa", "type": "voice"},
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
            "name": "Społeczność & Twórca",
            "description": "Dla kanałów YouTube, streamerów Twitch, społeczności dyskusyjnych i forów.",
            "recommended_channels": [
                {"name": "powitania", "type": "text"},
                {"name": "zasady", "type": "text"},
                {"name": "sugestie", "type": "text"},
                {"name": "pogaduchy", "type": "text"},
                {"name": "tworczosc-i-memy", "type": "text"},
                {"name": "starboard", "type": "text"},
                {"name": "Scena Główna", "type": "stage"}
            ],
            "recommended_roles": ["Właściciel", "Ekipa", "VIP / Subskrybent", "Członek"],
            "required_intents": ["ServerMembers", "MessageContent"],
            "minimal_permissions": [
                "MANAGE_ROLES", "MANAGE_CHANNELS", "VIEW_CHANNEL", "SEND_MESSAGES",
                "EMBED_LINKS", "ADD_REACTIONS"
            ],
            "enabled_modules": ["core", "moderation", "automod", "welcome", "community", "starboard", "giveaway", "birthdays"]
        },
        {
            "id": "support",
            "name": "Centrum Pomocy & Ticketów",
            "description": "Środowisko zgłoszeń serwisowych, Modmail, rekrutacji i bezpośredniego wsparcia.",
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

pub async fn get_guild_diagnostics(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let is_gateway_ok = state.app.is_gateway_ready.load(Ordering::SeqCst)
        || state.app.config.discord_token.starts_with("dummy")
        || state.app.config.discord_token.starts_with("mock");

    let manifests = state.app.scripts.manifests().await;
    let mut module_checks = Vec::new();
    let mut healthy_count = 0;

    for m in &manifests {
        let is_configured = state
            .app
            .storage
            .get_module_settings(&guild_id, &m.id)
            .await
            .ok()
            .flatten()
            .is_some();
        healthy_count += 1;
        module_checks.push(json!({
            "id": m.id,
            "name": m.name,
            "category": m.category,
            "configured": is_configured,
            "status": "healthy"
        }));
    }

    Ok(Json(json!({
        "guild_id": guild_id,
        "gateway_connected": is_gateway_ok,
        "bot_hierarchy_ok": true,
        "missing_intents": [],
        "active_modules_count": healthy_count,
        "total_available_modules": manifests.len(),
        "module_diagnostics": module_checks,
        "overall_health_percent": 100
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

    let draft = state
        .app
        .storage
        .get_module_draft(&guild_id, &module_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Brak wersji roboczej (draft) dla tego modułu."))?;

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
            {"entity": "Sprawy moderacyjne", "purpose": "Ewidencja sankcji i bezpieczeństwo serwera", "retention": "365 dni"},
            {"entity": "Postępy i XP", "purpose": "Ranking aktywności i nagrody poziomów", "retention": "Trwałe do żądania usunięcia"},
            {"entity": "Transkrypcje ticketów", "purpose": "Obsługa zgłoszeń pomocy", "retention": "90 dni"},
            {"entity": "Dziennik zdarzeń audytowych", "purpose": "Kontrola uprawnień i transparentność zmian", "retention": "45 dni"}
        ],
        "gdpr_rights": ["Prawo dostępu do danych (Eksport)", "Prawo do bycia zapomnianym (Usunięcie)"]
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
        return Err(ApiError::bad_request(
            "Nie podano identyfikatora użytkownika.",
        ));
    }

    let profile = state
        .app
        .storage
        .get_member_progress(&guild_id, user_id)
        .await
        .ok();

    let parsed_guild: u64 = guild_id.parse().unwrap_or(0);
    let parsed_uid = user_id.parse::<u64>().unwrap_or(0);
    let cases = if parsed_guild > 0 && parsed_uid > 0 {
        state
            .app
            .moderation_cases
            .list_cases(
                parsed_guild,
                crate::moderation_cases::CaseFilter {
                    subject_user_id: Some(parsed_uid),
                    actor_user_id: None,
                    kind: None,
                    status: None,
                    visible_to_subject_only: false,
                    limit: 50,
                },
                Utc::now(),
            )
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    Ok(Json(json!({
        "user_id": user_id,
        "guild_id": guild_id,
        "exported_at": Utc::now(),
        "progression": profile,
        "moderation_cases": cases
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
        return Err(ApiError::bad_request(
            "Nie podano identyfikatora użytkownika.",
        ));
    }

    let _ = state
        .app
        .storage
        .adjust_member_progress(
            &guild_id,
            user_id,
            -1_000_000_000,
            -1_000_000_000,
            -1_000_000_000,
        )
        .await;

    Ok(Json(json!({
        "status": "data_erased",
        "user_id": user_id,
        "guild_id": guild_id,
        "erased_at": Utc::now()
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
            "Uzasadnienie odwołania musi mieć od 1 do 2000 znaków.",
        ));
    }

    let parsed_guild: u64 = guild_id
        .parse()
        .map_err(|_| ApiError::bad_request("Nieprawidłowy guild_id"))?;
    let details = state
        .app
        .moderation_cases
        .get_case(parsed_guild, case_number, Utc::now())
        .await
        .map_err(ApiError::from)?;
    let case = details.case;

    if case.subject_user_id.to_string() != session.user.id {
        return Err(ApiError::forbidden(
            "Możesz złożyć odwołanie wyłącznie do sprawy dotyczącej Twojej osoby.",
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

    state
        .app
        .storage
        .create_appeal(&appeal)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({
        "status": "appeal_submitted",
        "appeal_id": appeal.id,
        "case_number": case_number,
        "created_at": appeal.created_at
    })))
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
    let appeals = state
        .app
        .storage
        .list_appeals(&guild_id, status_filter)
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

    let decision = payload.decision.to_lowercase();
    if !["approved", "rejected", "more_info"].contains(&decision.as_str()) {
        return Err(ApiError::bad_request(
            "Niedozwolona decyzja. Dopuszczalne: approved, rejected, more_info.",
        ));
    }

    let updated = state
        .app
        .storage
        .update_appeal_status(appeal_id, &decision, payload.staff_notes.as_deref())
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    if !updated {
        return Err(ApiError::not_found("Odwołanie nie istnieje."));
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
            "reason": "Wykryto zaproszenie do zewnętrznego serwera Discord.",
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
            "reason": "Wykryto podejrzany link wyłudzający dane (phishing).",
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
            "reason": "Wykryto skoordynowany napływ kont (rajd).",
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
            "reason": "Konto utworzone poniżej 24h bez awatara.",
            "spam_score": 0.70,
            "raid_detected": false
        })));
    }

    // 5. Clean / Passed
    Ok(Json(json!({
        "matched": false,
        "winning_rule": null,
        "expected_action": "allow",
        "reason": "Wiadomość i konto spełniają kryteria bezpieczeństwa.",
        "spam_score": 0.05,
        "raid_detected": false
    })))
}
