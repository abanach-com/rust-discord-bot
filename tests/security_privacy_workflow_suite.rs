use std::collections::HashSet;
use std::sync::Arc;

use chrono::{Duration, Utc};
use dashmap::DashMap;
use reqwest::header::COOKIE;
use serde_json::json;
use uuid::Uuid;

use rusty_discord_bot::{
    config::Config,
    session::{DashboardGuild, DashboardUser},
    state::AppState,
    storage::IntegrationConnection,
    web::{
        WebState,
        auth::{CachedGuildPermission, LoginRateLimiter},
        build_router,
    },
};

fn test_config() -> Config {
    Config {
        discord_token: "dummy-token".to_owned(),
        discord_application_id: 1,
        discord_client_id: "1".to_owned(),
        discord_client_secret: "secret".to_owned(),
        discord_oauth_redirect_url: "http://localhost/callback".to_owned(),
        discord_dev_guild_id: None,
        dashboard_bind: "127.0.0.1:0".parse().unwrap(),
        dashboard_public_url: "http://localhost".to_owned(),
        database_url: None,
        scripts_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts"),
        session_cookie_secure: false,
        session_ttl_seconds: 604800,
        lua_memory_limit_bytes: 4 * 1024 * 1024,
        lua_instruction_limit: 200_000,
        lua_hook_granularity: 100,
        music_allowed_hosts: HashSet::new(),
        trusted_proxies: ["127.0.0.1".parse().unwrap(), "::1".parse().unwrap()]
            .into_iter()
            .collect(),
    }
}

async fn test_web_state() -> (
    WebState,
    Arc<DashMap<(String, String), CachedGuildPermission>>,
) {
    let app = AppState::bootstrap(test_config())
        .await
        .expect("bootstrap app state");
    let permissions_cache = Arc::new(DashMap::new());
    let login_rate_limiter = Arc::new(LoginRateLimiter::new());
    (
        WebState {
            app,
            permissions_cache: Arc::clone(&permissions_cache),
            login_rate_limiter,
        },
        permissions_cache,
    )
}

#[tokio::test]
async fn test_security_privacy_workflow_suite() {
    let (state, perms_cache) = test_web_state().await;
    let app = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::builder().build().unwrap();

    let guild_id = "guild_sec_100";
    let user_id = "user_sec_200";
    let session_id = "test_sec_session_token_123";
    let csrf_token = "test_sec_csrf_token_456";

    state
        .app
        .sessions
        .save_session(
            session_id,
            DashboardUser {
                id: user_id.to_string(),
                username: "SecUser".to_string(),
                global_name: Some("Sec User".to_string()),
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Sec Guild".to_string(),
                icon: None,
                owner: true,
                permissions: "8".to_string(),
            }],
            csrf_token.to_string(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .unwrap();

    perms_cache.insert(
        (user_id.to_string(), guild_id.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    // =========================================================================
    // Issue #131 [P1][security]: Integration & Telegram Token Redaction
    // =========================================================================

    // 1a. Seed an integration connection with sensitive tokens
    state
        .app
        .storage
        .upsert_integration_connection(IntegrationConnection {
            id: Uuid::new_v4(),
            guild_id: guild_id.to_string(),
            provider: "github".to_string(),
            provider_account_id: Some("gh_acc_99".to_string()),
            access_token: Some("ghp_super_secret_personal_access_token_12345".to_string()),
            refresh_token: Some("ghr_secret_refresh_token_67890".to_string()),
            token_expires_at: Some(Utc::now() + Duration::days(30)),
            scopes: json!(["repo", "read:user"]),
            metadata: json!({"login": "octocat"}),
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .expect("upsert integration connection");

    // Fetch integrations endpoint
    let res = client
        .get(format!("{base_url}/api/guilds/{guild_id}/integrations"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body_text = res.text().await.unwrap();
    assert!(
        !body_text.contains("ghp_super_secret"),
        "access_token must not leak in integrations endpoint JSON"
    );
    assert!(
        !body_text.contains("ghr_secret"),
        "refresh_token must not leak in integrations endpoint JSON"
    );
    let parsed: serde_json::Value = serde_json::from_str(&body_text).unwrap();
    assert_eq!(parsed["connections"][0]["provider"], "github");
    assert_eq!(parsed["connections"][0]["connected"], true);

    // 1b. Create Telegram Bridge
    let bridge_res = client
        .post(format!("{base_url}/api/guilds/{guild_id}/telegram/bridge"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&json!({
            "discord_channel_id": "123456789",
            "telegram_chat_id": "987654321",
            "secret_token": "tg_super_secret_webhook_token_abc"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bridge_res.status(), 200);
    let bridge_body = bridge_res.text().await.unwrap();
    assert!(
        !bridge_body.contains("tg_super_secret"),
        "secret_token must not leak in create bridge response"
    );
    let bridge_json: serde_json::Value = serde_json::from_str(&bridge_body).unwrap();
    assert_eq!(bridge_json["secret_configured"], true);
    assert!(bridge_json.get("secret_token").is_none());

    // 1c. List Telegram Bridges
    let list_res = client
        .get(format!("{base_url}/api/guilds/{guild_id}/telegram/bridges"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(list_res.status(), 200);
    let list_body = list_res.text().await.unwrap();
    assert!(
        !list_body.contains("tg_super_secret"),
        "secret_token must not leak in list bridges response"
    );
    let list_json: serde_json::Value = serde_json::from_str(&list_body).unwrap();
    assert_eq!(list_json[0]["secret_configured"], true);
    assert!(list_json[0].get("secret_token").is_none());

    // =========================================================================
    // Issue #132 [P1][bug/privacy]: Genuinely Erase Progression & Associated Data
    // =========================================================================

    // Setup user progress
    let _ = state
        .app
        .storage
        .adjust_member_progress(guild_id, user_id, 100, 50, 10)
        .await
        .expect("adjust member progress");

    assert!(
        state
            .app
            .storage
            .has_member_progress(guild_id, user_id)
            .await
            .expect("check progress")
    );

    // Call DELETE /api/guilds/{id}/privacy/delete
    let del_res = client
        .post(format!("{base_url}/api/guilds/{guild_id}/privacy/delete"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&json!({
            "user_id": user_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(del_res.status(), 200);
    let del_json: serde_json::Value = del_res.json().await.unwrap();
    assert_eq!(del_json["status"], "data_erased");
    assert_eq!(del_json["report"]["progression_erased"], true);
    assert!(del_json["report"]["retained_records"].is_array());
    assert!(
        !del_json["report"]["retained_records"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // Verify storage no longer has member progress
    let has_prog = state
        .app
        .storage
        .has_member_progress(guild_id, user_id)
        .await
        .expect("check progress after erase");
    assert!(!has_prog, "Progress record must be deleted from storage");

    // Verify export returns null for progression
    let export_res = client
        .post(format!("{base_url}/api/guilds/{guild_id}/privacy/export"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&json!({
            "user_id": user_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(export_res.status(), 200);
    let export_json: serde_json::Value = export_res.json().await.unwrap();
    assert!(
        export_json["progression"].is_null(),
        "Exported progression must be null after erasure"
    );

    // =========================================================================
    // Issue #133 [P2][bug]: Workflow DAG Validation & Dynamic Dry-Run
    // =========================================================================

    // 3a. Reject cyclic graph
    let cyclic_wf = json!({
        "name": "Cyclic Workflow",
        "dag_nodes": [
            { "id": "node_1", "node_type": "trigger", "config": {} },
            { "id": "node_2", "node_type": "action", "config": {} }
        ],
        "dag_edges": [
            { "from": "node_1", "to": "node_2" },
            { "from": "node_2", "to": "node_1" }
        ]
    });
    let cyclic_res = client
        .post(format!("{base_url}/api/guilds/{guild_id}/workflows"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&cyclic_wf)
        .send()
        .await
        .unwrap();
    assert_eq!(
        cyclic_res.status(),
        400,
        "Cyclic workflow DAG must be rejected with 400"
    );
    let cyclic_err = cyclic_res.text().await.unwrap();
    assert!(cyclic_err.to_lowercase().contains("cycle"));

    // 3b. Reject invalid JSON structure for dag_nodes (e.g., number or string instead of array)
    let malformed_wf = json!({
        "name": "Malformed Workflow",
        "dag_nodes": "not an array",
        "dag_edges": []
    });
    let malformed_res = client
        .post(format!("{base_url}/api/guilds/{guild_id}/workflows"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&malformed_wf)
        .send()
        .await
        .unwrap();
    assert_eq!(
        malformed_res.status(),
        400,
        "Malformed dag_nodes must be rejected with 400"
    );

    // 3c. Reject empty node ID or duplicate node ID
    let duplicate_nodes_wf = json!({
        "name": "Duplicate Nodes",
        "dag_nodes": [
            { "id": "dup", "node_type": "trigger", "config": {} },
            { "id": "dup", "node_type": "action", "config": {} }
        ],
        "dag_edges": []
    });
    let dup_res = client
        .post(format!("{base_url}/api/guilds/{guild_id}/workflows"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&duplicate_nodes_wf)
        .send()
        .await
        .unwrap();
    assert_eq!(
        dup_res.status(),
        400,
        "Duplicate node IDs must be rejected with 400"
    );

    // 3d. Create valid workflow with 3 nodes
    let valid_wf = json!({
        "name": "Order Notification",
        "dag_nodes": [
            { "id": "step1", "node_type": "trigger", "config": { "event": "on_message" } },
            { "id": "step2", "node_type": "condition", "config": { "check": "contains_keyword" } },
            { "id": "step3", "node_type": "action", "config": { "send": "channel" } }
        ],
        "dag_edges": [
            { "from": "step1", "to": "step2" },
            { "from": "step2", "to": "step3" }
        ]
    });
    let valid_res = client
        .post(format!("{base_url}/api/guilds/{guild_id}/workflows"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&valid_wf)
        .send()
        .await
        .unwrap();
    assert_eq!(valid_res.status(), 200);
    let valid_json: serde_json::Value = valid_res.json().await.unwrap();
    let wf_id = valid_json["id"].as_str().unwrap();

    // 3e. Dry-run the created workflow
    let dry_res = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/workflows/{wf_id}/dry-run"
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(dry_res.status(), 200);
    let dry_json: serde_json::Value = dry_res.json().await.unwrap();
    assert_eq!(dry_json["dry_run"], true);
    assert_eq!(dry_json["valid"], true);
    assert_eq!(
        dry_json["simulated_steps"], 3,
        "Dry-run should report 3 simulated steps matching node count"
    );
    assert_eq!(dry_json["estimated_execution_ms"], 36);
}
