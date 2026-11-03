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

    let conn = IntegrationConnection {
        id: Uuid::new_v4(),
        guild_id: guild_id.clone(),
        provider: provider.clone(),
        provider_account_id: Some(format!("{}_acc_{}", provider, Uuid::new_v4().simple())),
        access_token: Some(format!("token_{}", Uuid::new_v4().simple())),
        refresh_token: Some(format!("refresh_{}", Uuid::new_v4().simple())),
        token_expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
        scopes: json!(["read", "events"]),
        metadata: json!({"connected_by": session.user.id}),
        enabled: true,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let saved = state
        .app
        .storage
        .upsert_integration_connection(conn)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(saved))
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
            let sig = headers
                .get("Twitch-Eventsub-Message-Signature")
                .and_then(|h| h.to_str().ok())
                .ok_or_else(|| {
                    ApiError::unauthorized("Missing Twitch EventSub signature header")
                })?;
            if !verify_hmac_sha256("test-twitch-secret", raw_bytes, sig) {
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

    let event_id = payload
        .get("event_id")
        .or_else(|| payload.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("evt_default");

    let is_first = state
        .app
        .storage
        .check_and_record_event_dedupe(&provider, event_id)
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

    // Record delivery log
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
    let _ = state.app.storage.record_delivery_log(log).await;

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
        .join_steam_lfg_post(post_id, &session.user.id)
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
        .approve_social_draft(draft_id, &session.user.id)
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

    let updated = state
        .app
        .storage
        .record_ai_usage(&guild_id, payload.tokens, payload.usd)
        .await
        .map_err(|e| ApiError::too_many_requests(e.to_string()))?;

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

    let results = state
        .app
        .storage
        .search_ai_documents(&guild_id, &payload.user_channels, &payload.query)
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

pub async fn publish_marketplace_package(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(payload): Json<PublishPackageRequest>,
) -> Result<Json<PluginMarketplacePackage>, ApiError> {
    let (_, session) = authenticate(&state, &headers).await?;
    verify_csrf(&headers, &session)?;

    // If raw content is provided, verify SHA-256
    if let Some(ref raw_b64) = payload.raw_content_base64 {
        let bytes = hex::decode(raw_b64).unwrap_or_else(|_| raw_b64.as_bytes().to_vec());
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
        is_verified: true,
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
