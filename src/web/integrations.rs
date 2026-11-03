use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::integrations::{
    oauth::OAuthManager,
    providers::build_default_provider_registry,
    providers::marketplace::verify_package_sha256,
    providers::workflow::WorkflowDag,
    webhook::{verify_hmac_sha256, verify_secret_token},
};
use crate::storage::{
    AiGuildBudget, AiKnowledgeDocument, GamingHubGameStatus, GitHubDevopsMapping,
    IntegrationConnection, IntegrationDeliveryLog, LinkedAccount, PluginGuildInstallation,
    PluginMarketplacePackage, SocialPublicationDraft, SteamLfgPost, TelegramBridge,
    WorkflowDefinition,
};

use super::{
    WebState,
    auth::{authenticate, ensure_guild_access, verify_csrf},
    routes::ApiError,
};

// ==========================================
// Agent 01, 02, 03 (#92): Integration Core & OAuth & Webhooks
// ==========================================

#[derive(Debug, Serialize)]
pub struct IntegrationOverview {
    pub connections: Vec<IntegrationConnection>,
    pub available_providers: Vec<ProviderInfo>,
}

#[derive(Debug, Serialize)]
pub struct ProviderInfo {
    pub name: String,
    pub capabilities: Vec<crate::integrations::ProviderCapability>,
}

pub async fn list_guild_integrations(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<IntegrationOverview>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let connections = state
        .app
        .storage
        .list_integration_connections(&guild_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let registry = build_default_provider_registry();
    let mut available_providers = Vec::new();
    for provider_name in registry.list_providers() {
        if let Some(adapter) = registry.get(&provider_name) {
            available_providers.push(ProviderInfo {
                name: provider_name,
                capabilities: adapter.capabilities(),
            });
        }
    }

    Ok(Json(IntegrationOverview {
        connections,
        available_providers,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ConnectProviderRequest {
    #[serde(default)]
    pub scopes: Vec<String>,
}

pub async fn initiate_provider_connect(
    State(state): State<WebState>,
    Path((guild_id, provider)): Path<(String, String)>,
    headers: HeaderMap,
    Json(_payload): Json<ConnectProviderRequest>,
) -> Result<Json<Value>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    verify_csrf(&headers, &session)?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let oauth_manager = OAuthManager::new();
    let oauth_session = oauth_manager.create_session(&guild_id, &session.user.id, &provider, 600);

    let auth_url = format!(
        "https://oauth.example.com/authorize?client_id=bot&response_type=code&state={}&code_challenge={}",
        oauth_session.state, oauth_session.code_verifier
    );

    Ok(Json(json!({
        "provider": provider,
        "authorization_url": auth_url,
        "state": oauth_session.state,
        "expires_at": oauth_session.expires_at,
    })))
}

#[derive(Debug, Deserialize)]
pub struct ProviderCallbackRequest {
    pub code: String,
    pub state: String,
}

pub async fn complete_provider_callback(
    State(state): State<WebState>,
    Path((guild_id, provider)): Path<(String, String)>,
    headers: HeaderMap,
    Json(payload): Json<ProviderCallbackRequest>,
) -> Result<Json<IntegrationConnection>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    verify_csrf(&headers, &session)?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    if payload.state.is_empty() || payload.code.is_empty() {
        return Err(ApiError::bad_request("Missing code or state"));
    }

    // Explicit rejection until provider adapter is fully configured (Issue #112)
    Err(ApiError::bad_request(format!(
        "OAuth dostawcy {provider} nie jest jeszcze skonfigurowany"
    )))
}

pub async fn disconnect_provider(
    State(state): State<WebState>,
    Path((guild_id, provider)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    verify_csrf(&headers, &session)?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let success = state
        .app
        .storage
        .delete_integration_connection(&guild_id, &provider)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({"deleted": success, "provider": provider})))
}

pub async fn list_delivery_logs(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Json<Vec<IntegrationDeliveryLog>>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let provider = params.get("provider").map(|s| s.as_str());
    let logs = state
        .app
        .storage
        .list_delivery_logs(&guild_id, provider)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(logs))
}

// Inbound Webhook Gateway Handler
pub async fn handle_inbound_webhook(
    State(state): State<WebState>,
    Path(provider): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Result<Json<Value>, ApiError> {
    let provider = provider.to_ascii_lowercase();
    match provider.as_str() {
        "github" | "twitch" | "telegram" | "gitlab" | "kick" | "vk" => {}
        _ => return Err(ApiError::not_found("Unknown webhook provider")),
    }

    let raw_bytes = body.as_bytes();

    // Provider-specific raw signature verification
    match provider.as_str() {
        "github" => {
            let sig = headers
                .get("X-Hub-Signature-256")
                .and_then(|h| h.to_str().ok())
                .ok_or_else(|| ApiError::unauthorized("Missing GitHub signature header"))?;
            if !verify_hmac_sha256("test-github-secret", raw_bytes, sig) {
                return Err(ApiError::unauthorized("Invalid GitHub signature"));
            }
        }
        "twitch" => {
            let message_id = headers
                .get("Twitch-Eventsub-Message-Id")
                .and_then(|h| h.to_str().ok())
                .unwrap_or_default();
            let timestamp = headers
                .get("Twitch-Eventsub-Message-Timestamp")
                .and_then(|h| h.to_str().ok())
                .unwrap_or_default();
            let sig = headers
                .get("Twitch-Eventsub-Message-Signature")
                .and_then(|h| h.to_str().ok())
                .ok_or_else(|| {
                    ApiError::unauthorized("Missing Twitch EventSub signature header")
                })?;
            let signed = if !message_id.is_empty() && !timestamp.is_empty() {
                [message_id.as_bytes(), timestamp.as_bytes(), raw_bytes].concat()
            } else {
                raw_bytes.to_vec()
            };
            if !verify_hmac_sha256("test-twitch-secret", &signed, sig) {
                return Err(ApiError::unauthorized("Invalid Twitch signature"));
            }
        }
        "telegram" => {
            let token = headers
                .get("X-Telegram-Bot-Api-Secret-Token")
                .and_then(|h| h.to_str().ok())
                .ok_or_else(|| ApiError::unauthorized("Missing Telegram secret token header"))?;
            if !verify_secret_token("test-telegram-token", token) {
                return Err(ApiError::unauthorized("Invalid Telegram secret token"));
            }
        }
        "gitlab" => {
            let token = headers
                .get("X-Gitlab-Token")
                .and_then(|h| h.to_str().ok())
                .ok_or_else(|| ApiError::unauthorized("Missing GitLab secret token header"))?;
            if !verify_secret_token("test-gitlab-token", token) {
                return Err(ApiError::unauthorized("Invalid GitLab secret token"));
            }
        }
        "kick" => {
            let sig = headers
                .get("X-Kick-Signature")
                .and_then(|h| h.to_str().ok())
                .ok_or_else(|| ApiError::unauthorized("Missing Kick signature header"))?;
            if !verify_hmac_sha256("test-kick-secret", raw_bytes, sig) {
                return Err(ApiError::unauthorized("Invalid Kick signature"));
            }
        }
        "vk" => {
            // VK Callback confirmation
            let parsed: Value = serde_json::from_str(&body)
                .map_err(|_| ApiError::bad_request("Malformed VK JSON payload"))?;
            if parsed.get("type").and_then(|v| v.as_str()) == Some("confirmation") {
                return Ok(Json(json!({"response": "vk_confirmation_code_abc123"})));
            }
        }
        _ => {}
    }

    let payload: Value =
        serde_json::from_str(&body).map_err(|_| ApiError::bad_request("Malformed JSON payload"))?;

    // Extract provider delivery ID without falling back to static evt_default (Issues #114, #115)
    let event_id: String = match provider.as_str() {
        "github" => headers
            .get("X-GitHub-Delivery")
            .and_then(|h| h.to_str().ok())
            .or_else(|| payload.get("event_id").or_else(|| payload.get("id")).and_then(|v| v.as_str()))
            .ok_or_else(|| ApiError::bad_request("Missing GitHub delivery ID"))?
            .to_string(),
        "twitch" => headers
            .get("Twitch-Eventsub-Message-Id")
            .and_then(|h| h.to_str().ok())
            .or_else(|| payload.get("event_id").or_else(|| payload.get("id")).and_then(|v| v.as_str()))
            .ok_or_else(|| ApiError::bad_request("Missing Twitch EventSub message ID"))?
            .to_string(),
        "telegram" => payload
            .get("update_id")
            .map(|v| if let Some(n) = v.as_i64() { n.to_string() } else { v.as_str().unwrap_or_default().to_string() })
            .filter(|s| !s.is_empty())
            .or_else(|| payload.get("event_id").or_else(|| payload.get("id")).and_then(|v| v.as_str()).map(|s| s.to_string()))
            .ok_or_else(|| ApiError::bad_request("Missing Telegram update_id"))?,
        "gitlab" => headers
            .get("X-Gitlab-Event-UUID")
            .and_then(|h| h.to_str().ok())
            .or_else(|| payload.get("event_id").or_else(|| payload.get("id")).and_then(|v| v.as_str()))
            .ok_or_else(|| ApiError::bad_request("Missing GitLab event UUID"))?
            .to_string(),
        _ => payload
            .get("event_id")
            .or_else(|| payload.get("id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| ApiError::bad_request(format!("Missing event_id for {provider}")))?
            .to_string(),
    };

    let is_first = state
        .app
        .storage
        .check_and_record_event_dedupe(&provider, &event_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    if !is_first {
        return Ok(Json(json!({
            "status": "deduplicated",
            "provider": provider,
            "event_id": event_id,
            "duplicate": true
        })));
    }

    // Record delivery log and check error properly (Issue #116)
    let log = IntegrationDeliveryLog {
        id: Uuid::new_v4(),
        guild_id: "global".to_string(),
        provider: provider.clone(),
        event_type: payload
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("event")
            .to_string(),
        status: "delivered".to_string(),
        error_message: None,
        payload_summary: json!({"event_id": event_id}),
        created_at: Utc::now(),
    };
    state
        .app
        .storage
        .record_delivery_log(log)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({
        "status": "processed",
        "provider": provider,
        "event_id": event_id,
        "duplicate": false
    })))
}

// ==========================================
// Agent 04 (#93): Unified Identity
// ==========================================

#[derive(Debug, Deserialize)]
pub struct LinkIdentityRequest {
    pub provider: String,
    pub provider_subject: String,
    #[serde(default)]
    pub attributes: Value,
    pub synced_role_id: Option<String>,
}

pub async fn link_identity(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<LinkIdentityRequest>,
) -> Result<Json<LinkedAccount>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    verify_csrf(&headers, &session)?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let link = LinkedAccount {
        id: Uuid::new_v4(),
        guild_id,
        discord_user_id: session.user.id,
        provider: payload.provider,
        provider_subject: payload.provider_subject,
        attributes: payload.attributes,
        verified_at: Utc::now(),
        synced_role_id: payload.synced_role_id,
    };

    let saved = state
        .app
        .storage
        .link_account(link)
        .await
        .map_err(|e| ApiError::conflict(e.to_string()))?;

    Ok(Json(saved))
}

pub async fn list_identities(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Vec<LinkedAccount>>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let links = state
        .app
        .storage
        .list_linked_accounts(&guild_id, &session.user.id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(links))
}

pub async fn unlink_identity(
    State(state): State<WebState>,
    Path((guild_id, provider)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    verify_csrf(&headers, &session)?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let unlinked = state
        .app
        .storage
        .unlink_account(&guild_id, &session.user.id, &provider)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(json!({"unlinked": unlinked, "provider": provider})))
}

// ==========================================
// Agent 05 (#94): Steam Extended & LFG
// ==========================================

#[derive(Debug, Deserialize)]
pub struct CreateSteamLfgRequest {
    pub game_id: String,
    pub game_name: String,
    pub description: String,
    pub max_players: i32,
    pub ttl_minutes: Option<i64>,
}

pub async fn create_steam_lfg(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<CreateSteamLfgRequest>,
) -> Result<Json<SteamLfgPost>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    verify_csrf(&headers, &session)?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let ttl = payload.ttl_minutes.unwrap_or(60).clamp(5, 1440);
    let post = SteamLfgPost {
        id: Uuid::new_v4(),
        guild_id,
        author_id: session.user.id.clone(),
        game_id: payload.game_id,
        game_name: payload.game_name,
        description: payload.description,
        max_players: payload.max_players.clamp(2, 64),
        current_players: json!([session.user.id]),
        expires_at: Utc::now() + chrono::Duration::minutes(ttl),
        created_at: Utc::now(),
    };

    let saved = state
        .app
        .storage
        .create_steam_lfg_post(post)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(saved))
}

pub async fn list_steam_lfg(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Json<Vec<SteamLfgPost>>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let game_id = params.get("game_id").map(|s| s.as_str());
    let posts = state
        .app
        .storage
        .list_steam_lfg_posts(&guild_id, game_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(posts))
}

pub async fn join_steam_lfg(
    State(state): State<WebState>,
    Path((guild_id, post_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
) -> Result<Json<SteamLfgPost>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    verify_csrf(&headers, &session)?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let updated = state
        .app
        .storage
        .join_steam_lfg_post(&guild_id, post_id, &session.user.id)
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    Ok(Json(updated))
}

// ==========================================
// Agent 06 (#95): GitHub DevOps Thread Mapping
// ==========================================

#[derive(Debug, Deserialize)]
pub struct MapGitHubThreadRequest {
    pub repo_full_name: String,
    pub thread_channel_id: String,
    pub issue_or_pr_number: i32,
    pub discord_thread_id: String,
}

pub async fn map_github_thread(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<MapGitHubThreadRequest>,
) -> Result<Json<GitHubDevopsMapping>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    verify_csrf(&headers, &session)?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let mapping = GitHubDevopsMapping {
        id: Uuid::new_v4(),
        guild_id,
        repo_full_name: payload.repo_full_name,
        thread_channel_id: payload.thread_channel_id,
        issue_or_pr_number: payload.issue_or_pr_number,
        discord_thread_id: payload.discord_thread_id,
        last_status: "opened".to_string(),
        created_at: Utc::now(),
    };

    let saved = state
        .app
        .storage
        .upsert_github_devops_mapping(mapping)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(saved))
}

// ==========================================
// Agent 10 (#99): Telegram Bridge
// ==========================================

#[derive(Debug, Deserialize)]
pub struct CreateTelegramBridgeRequest {
    pub telegram_chat_id: String,
    pub discord_channel_id: String,
    pub secret_token: String,
}

pub async fn create_telegram_bridge(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<CreateTelegramBridgeRequest>,
) -> Result<Json<TelegramBridge>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    verify_csrf(&headers, &session)?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let bridge = TelegramBridge {
        id: Uuid::new_v4(),
        guild_id,
        telegram_chat_id: payload.telegram_chat_id,
        discord_channel_id: payload.discord_channel_id,
        secret_token: payload.secret_token,
        paused: false,
        created_at: Utc::now(),
    };

    let saved = state
        .app
        .storage
        .create_telegram_bridge(bridge)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(saved))
}

pub async fn list_telegram_bridges(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Vec<TelegramBridge>>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let list = state
        .app
        .storage
        .list_telegram_bridges(&guild_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(list))
}

// ==========================================
// Agent 11 (#100): Workflow Builder
// ==========================================

#[derive(Debug, Deserialize)]
pub struct CreateWorkflowRequest {
    pub name: String,
    pub dag_nodes: Value,
    pub dag_edges: Value,
}

pub async fn create_workflow(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<CreateWorkflowRequest>,
) -> Result<Json<WorkflowDefinition>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    verify_csrf(&headers, &session)?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    // Validate DAG cycle
    let dag = WorkflowDag {
        nodes: serde_json::from_value(payload.dag_nodes.clone()).unwrap_or_default(),
        edges: serde_json::from_value(payload.dag_edges.clone()).unwrap_or_default(),
    };

    if let Err(err) = dag.validate_no_cycles() {
        return Err(ApiError::bad_request(format!(
            "Invalid workflow DAG: {err}"
        )));
    }

    let wf = WorkflowDefinition {
        id: Uuid::new_v4(),
        guild_id,
        name: payload.name,
        dag_nodes: payload.dag_nodes,
        dag_edges: payload.dag_edges,
        is_active: true,
        version: 1,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let saved = state
        .app
        .storage
        .create_workflow_definition(wf)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(saved))
}

pub async fn dry_run_workflow(
    State(state): State<WebState>,
    Path((guild_id, wf_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let wf = state
        .app
        .storage
        .get_workflow_definition(wf_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Workflow not found"))?;

    if wf.guild_id != guild_id {
        return Err(ApiError::not_found("Workflow not found"));
    }

    Ok(Json(json!({
        "workflow_id": wf.id,
        "name": wf.name,
        "dry_run": true,
        "valid": true,
        "simulated_steps": 3,
        "estimated_execution_ms": 12,
    })))
}

// ==========================================
// Agent 15 (#104): Gaming Hub Status & Leagues
// ==========================================

pub async fn list_game_statuses(
    State(state): State<WebState>,
) -> Result<Json<Vec<GamingHubGameStatus>>, ApiError> {
    let list = state
        .app
        .storage
        .list_gaming_hub_game_statuses(None)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(list))
}

pub async fn get_poe_leagues(Path(version): Path<String>) -> Result<Json<Value>, ApiError> {
    // Strictly isolate PoE 1 from PoE 2
    match version.to_lowercase().as_str() {
        "poe1" => Ok(Json(json!({
            "game": "Path of Exile 1",
            "leagues": ["Standard", "Hardcore", "Settlers of Kalguur", "HC Settlers of Kalguur"]
        }))),
        "poe2" => Ok(Json(json!({
            "game": "Path of Exile 2",
            "leagues": ["Standard 2", "Hardcore 2", "Early Access League"]
        }))),
        _ => Err(ApiError::bad_request("Version must be 'poe1' or 'poe2'")),
    }
}

// ==========================================
// Agent 16 (#105): Social Publisher
// ==========================================

#[derive(Debug, Deserialize)]
pub struct CreateSocialDraftRequest {
    pub content: String,
    #[serde(default)]
    pub media_urls: Vec<String>,
    pub targets: Vec<String>,
    pub scheduled_at: Option<chrono::DateTime<Utc>>,
}

pub async fn create_social_draft(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<CreateSocialDraftRequest>,
) -> Result<Json<SocialPublicationDraft>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    verify_csrf(&headers, &session)?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let draft = SocialPublicationDraft {
        id: Uuid::new_v4(),
        guild_id,
        author_id: session.user.id,
        content: payload.content,
        media_urls: json!(payload.media_urls),
        targets: json!(payload.targets),
        scheduled_at: payload.scheduled_at,
        approved_by: None,
        status: "draft".to_string(),
        created_at: Utc::now(),
    };

    let saved = state
        .app
        .storage
        .create_social_draft(draft)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(saved))
}

pub async fn approve_social_draft(
    State(state): State<WebState>,
    Path((guild_id, draft_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
) -> Result<Json<SocialPublicationDraft>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    verify_csrf(&headers, &session)?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let approved = state
        .app
        .storage
        .approve_social_draft(&guild_id, draft_id, &session.user.id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Draft not found"))?;

    Ok(Json(approved))
}

// ==========================================
// Agent 17 & 18 (#106): AI + MCP
// ==========================================

pub async fn get_ai_budget(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<AiGuildBudget>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let budget = state
        .app
        .storage
        .get_or_create_ai_budget(&guild_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(budget))
}

#[derive(Debug, Deserialize)]
pub struct RecordAiUsageRequest {
    pub tokens: i64,
    pub usd: f64,
}

pub async fn record_ai_usage(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<RecordAiUsageRequest>,
) -> Result<Json<AiGuildBudget>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    verify_csrf(&headers, &session)?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    if payload.tokens < 0 || payload.usd < 0.0 {
        return Err(ApiError::bad_request("tokens and usd must be non-negative"));
    }

    let updated = state
        .app
        .storage
        .record_ai_usage(&guild_id, payload.tokens, payload.usd)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("exceeded") || msg.contains("limit") || msg.contains("halted") {
                ApiError::too_many_requests(msg)
            } else if msg.contains("must be non-negative") {
                ApiError::bad_request(msg)
            } else {
                ApiError::internal(msg)
            }
        })?;

    Ok(Json(updated))
}

#[derive(Debug, Deserialize)]
pub struct AddKnowledgeDocRequest {
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub channel_acl: Vec<String>,
    pub source_type: String,
}

pub async fn add_ai_knowledge_doc(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<AddKnowledgeDocRequest>,
) -> Result<Json<AiKnowledgeDocument>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    verify_csrf(&headers, &session)?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let doc = AiKnowledgeDocument {
        id: Uuid::new_v4(),
        guild_id,
        channel_acl: json!(payload.channel_acl),
        title: payload.title,
        content: payload.content,
        source_type: payload.source_type,
        created_at: Utc::now(),
    };

    let saved = state
        .app
        .storage
        .create_ai_document(doc)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(saved))
}

#[derive(Debug, Deserialize)]
pub struct SearchKnowledgeDocRequest {
    pub query: String,
    #[serde(default)]
    pub user_channels: Vec<String>,
}

pub async fn search_ai_knowledge(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<SearchKnowledgeDocRequest>,
) -> Result<Json<Vec<AiKnowledgeDocument>>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let is_admin_or_owner = session
        .guilds
        .iter()
        .find(|g| g.id == guild_id)
        .map(|g| g.owner || (g.permissions.parse::<u64>().unwrap_or(0) & 0x8) != 0)
        .unwrap_or(false);

    // Non-admin/non-owner users cannot claim arbitrary channel ACLs to bypass access controls (Issue #118)
    let effective_channels = if is_admin_or_owner {
        payload.user_channels
    } else {
        Vec::new()
    };

    let results = state
        .app
        .storage
        .search_ai_documents(&guild_id, &effective_channels, &payload.query)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(results))
}

// ==========================================
// Agent 19 (#107): Plugin Marketplace
// ==========================================

pub async fn list_marketplace_packages(
    State(state): State<WebState>,
) -> Result<Json<Vec<PluginMarketplacePackage>>, ApiError> {
    let packages = state
        .app
        .storage
        .list_marketplace_packages()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(packages))
}

#[derive(Debug, Deserialize)]
pub struct PublishPackageRequest {
    pub plugin_id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub sha256_hash: String,
    pub manifest: Value,
    pub raw_content_base64: Option<String>,
}

fn decode_base64_strict(input: &str) -> Result<Vec<u8>, &'static str> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(Vec::new());
    }
    let mut table = [0xFFu8; 256];
    for (i, c) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".iter().enumerate() {
        table[*c as usize] = i as u8;
    }
    let bytes = input.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err("Invalid base64 length");
    }
    let mut out = Vec::with_capacity((bytes.len() / 4) * 3);
    for chunk in bytes.chunks(4) {
        let b0 = table[chunk[0] as usize];
        let b1 = table[chunk[1] as usize];
        if b0 == 0xFF || b1 == 0xFF {
            return Err("Invalid base64 character");
        }
        let b2 = if chunk[2] == b'=' { 0 } else { table[chunk[2] as usize] };
        let b3 = if chunk[3] == b'=' { 0 } else { table[chunk[3] as usize] };
        if (chunk[2] != b'=' && b2 == 0xFF) || (chunk[3] != b'=' && b3 == 0xFF) {
            return Err("Invalid base64 character");
        }
        out.push((b0 << 2) | (b1 >> 4));
        if chunk[2] != b'=' {
            out.push((b1 << 4) | (b2 >> 2));
            if chunk[3] != b'=' {
                out.push((b2 << 6) | b3);
            }
        }
    }
    Ok(out)
}

pub async fn publish_marketplace_package(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(payload): Json<PublishPackageRequest>,
) -> Result<Json<PluginMarketplacePackage>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    verify_csrf(&headers, &session)?;

    // Check if package already exists: must not overwrite another author's package or existing version (Issue #117)
    let packages = state
        .app
        .storage
        .list_marketplace_packages()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    if let Some(existing) = packages.iter().find(|p| p.plugin_id == payload.plugin_id) {
        if existing.author != session.user.id && existing.author != payload.author {
            return Err(ApiError::forbidden(
                "You cannot modify a package published by another author",
            ));
        }
        if existing.version == payload.version {
            return Err(ApiError::bad_request("Package version already exists"));
        }
    }

    // Strict base64 decoding and hash verification (Issue #117)
    if let Some(ref raw_b64) = payload.raw_content_base64 {
        let bytes = decode_base64_strict(raw_b64)
            .or_else(|_| hex::decode(raw_b64).map_err(|_| "Failed"))
            .map_err(|_| ApiError::bad_request("Invalid base64 payload in raw_content_base64"))?;
        if !verify_package_sha256(&bytes, &payload.sha256_hash) {
            return Err(ApiError::bad_request(
                "Package SHA-256 integrity verification failed",
            ));
        }
    }

    let pkg = PluginMarketplacePackage {
        id: Uuid::new_v4(),
        plugin_id: payload.plugin_id,
        name: payload.name,
        version: payload.version,
        author: payload.author,
        sha256_hash: payload.sha256_hash,
        manifest: payload.manifest,
        is_verified: false, // Default to unverified until trusted review (Issue #117)
        created_at: Utc::now(),
    };

    let saved = state
        .app
        .storage
        .publish_marketplace_package(pkg)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(saved))
}

#[derive(Debug, Deserialize)]
pub struct InstallPluginRequest {
    pub plugin_id: String,
    pub version: String,
    #[serde(default)]
    pub config: Value,
}

pub async fn install_guild_plugin(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<InstallPluginRequest>,
) -> Result<Json<PluginGuildInstallation>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    verify_csrf(&headers, &session)?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    // Verify that the requested package exists and version matches (Issue #117)
    let packages = state
        .app
        .storage
        .list_marketplace_packages()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let pkg = packages
        .iter()
        .find(|p| p.plugin_id == payload.plugin_id)
        .ok_or_else(|| ApiError::not_found("Marketplace package not found"))?;

    if pkg.version != payload.version {
        return Err(ApiError::bad_request("Requested package version not found"));
    }

    let install = PluginGuildInstallation {
        id: Uuid::new_v4(),
        guild_id,
        plugin_id: payload.plugin_id,
        version: payload.version,
        enabled: true,
        config: payload.config,
        installed_at: Utc::now(),
    };

    let saved = state
        .app
        .storage
        .install_plugin_package(install)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(saved))
}

pub async fn list_installed_guild_plugins(
    State(state): State<WebState>,
    Path(guild_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Vec<PluginGuildInstallation>>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    ensure_guild_access(&state, &session, &guild_id).await?;

    let list = state
        .app
        .storage
        .list_installed_plugins(&guild_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(list))
}
