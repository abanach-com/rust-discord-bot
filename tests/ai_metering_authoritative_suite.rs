use std::collections::HashSet;
use std::sync::Arc;

use axum::http::header::COOKIE;
use dashmap::DashMap;
use rusty_discord_bot::{
    config::Config,
    session::{DashboardGuild, DashboardUser},
    state::AppState,
    storage::AiKnowledgeDocument,
    web::{
        WebState,
        auth::{CachedGuildPermission, LoginRateLimiter},
        build_router,
    },
};
use serde_json::json;
use uuid::Uuid;

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
        lua_max_parallel: 32,
        lua_max_per_guild: 8,
        lua_command_timeout_ms: 3000,
        music_allowed_hosts: HashSet::new(),
        trusted_proxies: ["127.0.0.1".parse().unwrap(), "::1".parse().unwrap()]
            .into_iter()
            .collect(),
        offline_mode: false,
        discord_api_base_url: "https://discord.com/api/v10".to_string(),
        transcript_encryption_key: None,
        transcript_active_key_id: "v1".to_string(),
        transcript_keyring: None,
        ticket_transcripts_disabled: false,
        metrics_token: None,
        metrics_allowed_ips: HashSet::new(),
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
async fn test_client_ai_usage_recording_forbidden_and_does_not_mutate_budget() {
    let (state, perms_cache) = test_web_state().await;
    let app = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::builder().build().unwrap();

    let guild_id = "777000000000000001";
    let user_id = "888000000000000001";
    let session_id = "test_ai_metering_session_1";
    let csrf_token = "test_ai_metering_csrf_1";

    state
        .app
        .sessions
        .save_session(
            session_id,
            DashboardUser {
                id: user_id.to_string(),
                username: "AiAdmin".to_string(),
                global_name: Some("AI Admin".to_string()),
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "AI Test Guild".to_string(),
                icon: None,
                owner: true,
                permissions: "8".to_string(),
            }],
            csrf_token.to_string(),
            chrono::Utc::now() + chrono::Duration::hours(2),
            None,
        )
        .await
        .unwrap();

    perms_cache.insert(
        (user_id.to_string(), guild_id.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: chrono::Utc::now(),
        },
    );

    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    // Add a knowledge document to enable answering /ai/query
    state
        .app
        .storage
        .create_ai_document(AiKnowledgeDocument {
            id: Uuid::new_v4(),
            guild_id: guild_id.to_string(),
            channel_acl: json!([]),
            title: "Rules".to_string(),
            content: "Be respectful and friendly.".to_string(),
            source_type: "manual".to_string(),
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    // 1. Attempting to record arbitrary usage from HTTP client must be rejected with 403 Forbidden (#311 AC-1)
    let record_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/budget/record"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&json!({
            "tokens": 2_000_000,
            "usd": 50.0
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        record_resp.status(),
        reqwest::StatusCode::FORBIDDEN,
        "Direct client manipulation of AI usage must fail with 403 Forbidden"
    );

    // 2. Verify authoritative budget counters remain intact at 0
    let get_budget = client
        .get(format!("{base_url}/api/guilds/{guild_id}/ai/budget"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();

    assert_eq!(get_budget.status(), reqwest::StatusCode::OK);
    let budget_val: serde_json::Value = get_budget.json().await.unwrap();
    assert_eq!(budget_val["used_tokens"], 0);
    assert_eq!(budget_val["used_usd"], 0.0);
    assert_eq!(budget_val["is_halted"], false);

    // 3. Verify /ai/query continues to function and is NOT halted (#311 AC-5)
    let query_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/query"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&json!({
            "question": "What are the rules?"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(query_resp.status(), reqwest::StatusCode::OK);
    let query_data: serde_json::Value = query_resp.json().await.unwrap();
    assert_eq!(query_data["answered"], true);
    assert_ne!(query_data["status"], "budget_exceeded");
}

#[tokio::test]
async fn test_authoritative_metering_idempotency_and_quota_halt() {
    let (state, perms_cache) = test_web_state().await;
    let app = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::builder().build().unwrap();

    let guild_a = "777000000000000002";
    let guild_b = "777000000000000003";
    let user_id = "888000000000000002";
    let session_id = "test_ai_metering_session_2";
    let csrf_token = "test_ai_metering_csrf_2";

    state
        .app
        .sessions
        .save_session(
            session_id,
            DashboardUser {
                id: user_id.to_string(),
                username: "AiAdmin2".to_string(),
                global_name: Some("AI Admin 2".to_string()),
                avatar: None,
            },
            vec![
                DashboardGuild {
                    id: guild_a.to_string(),
                    name: "Guild A".to_string(),
                    icon: None,
                    owner: true,
                    permissions: "8".to_string(),
                },
                DashboardGuild {
                    id: guild_b.to_string(),
                    name: "Guild B".to_string(),
                    icon: None,
                    owner: true,
                    permissions: "8".to_string(),
                },
            ],
            csrf_token.to_string(),
            chrono::Utc::now() + chrono::Duration::hours(2),
            None,
        )
        .await
        .unwrap();

    perms_cache.insert(
        (user_id.to_string(), guild_a.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: chrono::Utc::now(),
        },
    );
    perms_cache.insert(
        (user_id.to_string(), guild_b.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: chrono::Utc::now(),
        },
    );

    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    // 1. Initial authoritative metering recording
    let b1 = state
        .app
        .storage
        .record_metering_event_once(guild_a, "provider_x", "req_event_001", 1500, 0.03)
        .await
        .expect("initial metering record");
    assert_eq!(b1.used_tokens, 1500);
    assert!((b1.used_usd - 0.03).abs() < 1e-6);
    assert!(!b1.is_halted);

    // 2. Duplicate metering event with the SAME request_id must be idempotent (#311 AC-2)
    let b1_dup = state
        .app
        .storage
        .record_metering_event_once(guild_a, "provider_x", "req_event_001", 1500, 0.03)
        .await
        .expect("idempotent replay of metering record");
    assert_eq!(
        b1_dup.used_tokens, 1500,
        "Duplicate metering event must not double-count tokens"
    );
    assert!(
        (b1_dup.used_usd - 0.03).abs() < 1e-6,
        "Duplicate metering event must not double-count usd"
    );

    // 3. Different request_id increments correctly
    let b2 = state
        .app
        .storage
        .record_metering_event_once(guild_a, "provider_x", "req_event_002", 500, 0.01)
        .await
        .expect("second distinct metering record");
    assert_eq!(b2.used_tokens, 2000);
    assert!((b2.used_usd - 0.04).abs() < 1e-6);

    // 4. Tenant isolation: guild B must remain unaffected at 0 (#311 AC-4)
    let b_guild_b = state
        .app
        .storage
        .get_or_create_ai_budget(guild_b)
        .await
        .expect("get guild b budget");
    assert_eq!(b_guild_b.used_tokens, 0);
    assert_eq!(b_guild_b.used_usd, 0.0);

    // 5. Exceeding monthly limit halts the budget (#311 AC-5)
    let b_overflow = state
        .app
        .storage
        .record_metering_event_once(guild_a, "provider_x", "req_overflow", 2_000_000, 50.0)
        .await
        .expect("record overflow metering");
    assert!(b_overflow.is_halted);

    // 6. Querying /ai/query now returns budget_exceeded
    let query_halted = client
        .post(format!("{base_url}/api/guilds/{guild_a}/ai/query"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&json!({
            "question": "What is the policy?"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(query_halted.status(), reqwest::StatusCode::OK);
    let halted_data: serde_json::Value = query_halted.json().await.unwrap();
    assert_eq!(halted_data["answered"], false);
    assert_eq!(halted_data["status"], "budget_exceeded");

    // 7. Input validation: negative values must fail
    assert!(
        state
            .app
            .storage
            .record_metering_event_once(guild_a, "provider_x", "req_neg_tok", -1, 0.0)
            .await
            .is_err()
    );
    assert!(
        state
            .app
            .storage
            .record_metering_event_once(guild_a, "provider_x", "req_neg_usd", 10, -0.01)
            .await
            .is_err()
    );
}
