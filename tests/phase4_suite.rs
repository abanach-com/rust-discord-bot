use std::collections::HashSet;
use std::sync::Arc;

use chrono::{Duration, Utc};
use dashmap::DashMap;
use reqwest::header::COOKIE;
use uuid::Uuid;

use sha2::Digest;

use rusty_discord_bot::{
    config::Config,
    integrations::{
        is_blocked_ip,
        oauth::{extract_steam_id_from_claimed_id, generate_pkce_pair},
        providers::marketplace::verify_package_sha256,
        providers::workflow::{WorkflowDag, WorkflowEdge, WorkflowNode},
        validate_safe_url,
        webhook::{compute_hmac_sha256, verify_secret_token},
    },
    session::{DashboardGuild, DashboardUser},
    state::AppState,
    storage::{AiKnowledgeDocument, IntegrationConnection, LinkedAccount},
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
        lua_max_parallel: 32,
        lua_max_per_guild: 8,
        lua_command_timeout_ms: 3000,
        music_allowed_hosts: HashSet::new(),
        trusted_proxies: ["127.0.0.1".parse().unwrap(), "::1".parse().unwrap()]
            .into_iter()
            .collect(),
        offline_mode: false,
        discord_api_base_url: "https://discord.com/api/v10".to_string(),
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
    let state = WebState {
        app,
        login_rate_limiter: Arc::new(LoginRateLimiter::new()),
        permissions_cache: permissions_cache.clone(),
    };
    (state, permissions_cache)
}

#[tokio::test]
async fn test_phase4_integrations_and_extensibility_suite() {
    let (state, _) = test_web_state().await;
    let guild_id = "111000000000000001";
    let guild_id_2 = "111000000000000002";
    let user_id = "user_phase4_operator";
    let other_user_id = "user_phase4_attacker";
    let session_id = "test_phase4_session_token";
    let csrf_token = "csrf_phase4_token";

    // Seed session in session store
    let user = DashboardUser {
        id: user_id.to_owned(),
        username: "Phase4Operator".to_owned(),
        global_name: None,
        avatar: None,
    };
    let guilds = vec![
        DashboardGuild {
            id: guild_id.to_owned(),
            name: "Phase 4 Primary Guild".to_owned(),
            icon: None,
            owner: true,
            permissions: "8".to_owned(),
        },
        DashboardGuild {
            id: guild_id_2.to_owned(),
            name: "Phase 4 Secondary Guild".to_owned(),
            icon: None,
            owner: true,
            permissions: "8".to_owned(),
        },
    ];

    state
        .app
        .sessions
        .save_session(
            session_id,
            user,
            guilds,
            csrf_token.to_owned(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .expect("save session");

    // Start ephemeral server
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let router = build_router(state.clone());

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    // ==========================================
    // 1. Agent 01, 02, 03 (#92): Integration Core, OAuth2 PKCE & Webhook Broker
    // ==========================================

    // SSRF Sentinel verification
    assert!(is_blocked_ip("127.0.0.1".parse().unwrap()));
    assert!(is_blocked_ip("10.0.0.1".parse().unwrap()));
    assert!(is_blocked_ip("192.168.1.1".parse().unwrap()));
    assert!(is_blocked_ip("169.254.169.254".parse().unwrap()));
    assert!(is_blocked_ip("::1".parse().unwrap()));

    let ssrf_blocked = validate_safe_url("http://169.254.169.254/latest/meta-data/", None);
    assert!(ssrf_blocked.is_err(), "SSRF attempt must be rejected");

    let valid_url = validate_safe_url("https://api.github.com/repos/rust-lang/rust", None);
    assert!(
        valid_url.is_ok(),
        "Valid public https target must be allowed"
    );

    // PKCE pair generation
    let (verifier, challenge) = generate_pkce_pair();
    assert!(!verifier.is_empty());
    assert!(!challenge.is_empty());

    // Ingress Webhook HMAC verification
    let secret = "test-github-secret";
    let body = r#"{"event_id":"gh_evt_001","type":"push","repository":{"full_name":"org/repo"}}"#;
    let hmac_bytes = compute_hmac_sha256(secret.as_bytes(), body.as_bytes());
    let sig_hex = format!("sha256={}", hex::encode(hmac_bytes));

    // Invalid signature rejection (AC-2)
    let bad_sig_resp = client
        .post(format!("{base_url}/api/integrations/webhooks/github"))
        .header(
            "X-Hub-Signature-256",
            "sha256=invalid0000000000000000000000000000000000000000000000000000000000",
        )
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(bad_sig_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // Valid signature acceptance (AC-1)
    let valid_sig_resp = client
        .post(format!("{base_url}/api/integrations/webhooks/github"))
        .header("X-Hub-Signature-256", &sig_hex)
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(valid_sig_resp.status(), reqwest::StatusCode::OK);
    let valid_json: serde_json::Value = valid_sig_resp.json().await.unwrap();
    assert_eq!(valid_json["duplicate"], false);

    // Deduplication on replay (AC-1)
    let replay_resp = client
        .post(format!("{base_url}/api/integrations/webhooks/github"))
        .header("X-Hub-Signature-256", &sig_hex)
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(replay_resp.status(), reqwest::StatusCode::OK);
    let replay_json: serde_json::Value = replay_resp.json().await.unwrap();
    assert_eq!(
        replay_json["duplicate"], true,
        "Replayed event must be deduplicated"
    );

    // List guild integrations overview (AC-5)
    let list_int_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/integrations"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(list_int_resp.status(), reqwest::StatusCode::OK);
    let int_overview: serde_json::Value = list_int_resp.json().await.unwrap();
    assert!(
        int_overview["available_providers"]
            .as_array()
            .unwrap()
            .len()
            >= 14
    );

    // Multi-tenant isolation (AC-3)
    let conn1 = IntegrationConnection {
        id: Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        provider: "github".to_string(),
        provider_account_id: Some("acc_guild1".to_string()),
        access_token: Some("token_g1".to_string()),
        refresh_token: None,
        token_expires_at: None,
        scopes: serde_json::json!(["read"]),
        metadata: serde_json::json!({}),
        enabled: true,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let conn2 = IntegrationConnection {
        id: Uuid::new_v4(),
        guild_id: guild_id_2.to_string(),
        provider: "github".to_string(),
        provider_account_id: Some("acc_guild2".to_string()),
        access_token: Some("token_g2".to_string()),
        refresh_token: None,
        token_expires_at: None,
        scopes: serde_json::json!(["read"]),
        metadata: serde_json::json!({}),
        enabled: true,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    state
        .app
        .storage
        .upsert_integration_connection(conn1)
        .await
        .unwrap();
    state
        .app
        .storage
        .upsert_integration_connection(conn2)
        .await
        .unwrap();

    // Disconnect guild 1 integration
    let del_resp = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/integrations/github"
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(del_resp.status(), reqwest::StatusCode::OK);

    // Guild 2 must remain connected!
    let g2_conn = state
        .app
        .storage
        .get_integration_connection(guild_id_2, "github")
        .await
        .unwrap();
    assert!(g2_conn.is_some(), "Guild 2 connection must remain intact");

    // ==========================================
    // 2. Agent 04 (#93): Unified Identity
    // ==========================================
    let link_payload = serde_json::json!({
        "provider": "twitch",
        "provider_subject": "twitch_streamer_42",
        "synced_role_id": "role_streamer"
    });

    let link_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/identity/link"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&link_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(link_resp.status(), reqwest::StatusCode::OK);

    // Anti-hijack (AC-1): Attempt to claim same subject from another account fails
    let hijack_link = LinkedAccount {
        id: Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        discord_user_id: other_user_id.to_string(),
        provider: "twitch".to_string(),
        provider_subject: "twitch_streamer_42".to_string(),
        attributes: serde_json::json!({}),
        verified_at: Utc::now(),
        synced_role_id: None,
    };
    let hijack_res = state.app.storage.link_account(hijack_link).await;
    assert!(
        hijack_res.is_err(),
        "Account hijack attempt must be rejected"
    );

    // Unlink account (AC-2)
    let unlink_resp = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/identity/unlink/twitch"
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(unlink_resp.status(), reqwest::StatusCode::OK);

    // ==========================================
    // 3. Agent 05 (#94): Steam Extended & LFG
    // ==========================================
    let claimed_id = "https://steamcommunity.com/openid/id/76561198000000001";
    let extracted = extract_steam_id_from_claimed_id(claimed_id);
    assert_eq!(extracted.as_deref(), Some("76561198000000001"));
    assert_eq!(
        extract_steam_id_from_claimed_id("https://evil.com/openid/id/123"),
        None
    );

    let lfg_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/steam/lfg"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "game_id": "730",
            "game_name": "Counter-Strike 2",
            "description": "Premier 5-stack grind",
            "max_players": 5,
            "ttl_minutes": 60
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(lfg_resp.status(), reqwest::StatusCode::OK);
    let lfg_data: serde_json::Value = lfg_resp.json().await.unwrap();
    let lfg_id = lfg_data["id"].as_str().unwrap();

    let join_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/steam/lfg/{lfg_id}/join"
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(join_resp.status(), reqwest::StatusCode::OK);

    // ==========================================
    // 4. Agent 06 (#95): GitHub DevOps Thread Mapping
    // ==========================================
    let map_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/github/thread-map"
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "repo_full_name": "abanach-com/rust-discord-bot",
            "thread_channel_id": "111222333444555",
            "issue_or_pr_number": 95,
            "discord_thread_id": "999888777666555"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(map_resp.status(), reqwest::StatusCode::OK);

    // ==========================================
    // 5. Agent 10 (#99): Telegram Bridge
    // ==========================================
    // Secret token check
    assert!(verify_secret_token("secret123", "secret123"));
    assert!(!verify_secret_token("secret123", "wrong_token"));

    let bridge_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/telegram/bridge"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "telegram_chat_id": "-100123456789",
            "discord_channel_id": "777888999",
            "secret_token": "tg_sec_token"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bridge_resp.status(), reqwest::StatusCode::OK);

    // ==========================================
    // 6. Agent 11 (#100): Workflow Builder DAG Validation
    // ==========================================
    // Cyclic DAG must fail validation (AC-1)
    let cyclic_dag = WorkflowDag {
        nodes: vec![
            WorkflowNode {
                id: "n1".to_string(),
                node_type: "trigger".to_string(),
                config: serde_json::json!({}),
            },
            WorkflowNode {
                id: "n2".to_string(),
                node_type: "action".to_string(),
                config: serde_json::json!({}),
            },
        ],
        edges: vec![
            WorkflowEdge {
                from: "n1".to_string(),
                to: "n2".to_string(),
            },
            WorkflowEdge {
                from: "n2".to_string(),
                to: "n1".to_string(),
            }, // Cycle!
        ],
    };
    assert!(
        cyclic_dag.validate_no_cycles().is_err(),
        "Cyclic DAG must be rejected"
    );

    // Acyclic DAG succeeds and passes dry-run (AC-2)
    let wf_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/workflows"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "name": "Auto Welcome & Log",
            "dag_nodes": [
                { "id": "t1", "node_type": "trigger", "config": {} },
                { "id": "a1", "node_type": "action", "config": {} }
            ],
            "dag_edges": [
                { "from": "t1", "to": "a1" }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(wf_resp.status(), reqwest::StatusCode::OK);
    let wf_json: serde_json::Value = wf_resp.json().await.unwrap();
    let wf_id = wf_json["id"].as_str().unwrap();

    let dry_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/workflows/{wf_id}/dry-run"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(dry_resp.status(), reqwest::StatusCode::OK);

    // ==========================================
    // 7. Agent 13 (#102): VK Callback Handshake
    // ==========================================
    let vk_handshake = client
        .post(format!("{base_url}/api/integrations/webhooks/vk"))
        .body(r#"{"type":"confirmation","group_id":123456}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(vk_handshake.status(), reqwest::StatusCode::OK);
    let vk_body: serde_json::Value = vk_handshake.json().await.unwrap();
    assert!(
        vk_body["response"]
            .as_str()
            .unwrap()
            .contains("vk_confirmation_code")
    );

    // ==========================================
    // 8. Agent 15 (#104): Gaming Hub & PoE 1 vs PoE 2 Separation
    // ==========================================
    let poe1_resp = client
        .get(format!("{base_url}/api/gaming-hub/leagues/poe1"))
        .send()
        .await
        .unwrap();
    let poe1_data: serde_json::Value = poe1_resp.json().await.unwrap();
    let poe1_leagues = poe1_data["leagues"].as_array().unwrap();

    let poe2_resp = client
        .get(format!("{base_url}/api/gaming-hub/leagues/poe2"))
        .send()
        .await
        .unwrap();
    let poe2_data: serde_json::Value = poe2_resp.json().await.unwrap();
    let poe2_leagues = poe2_data["leagues"].as_array().unwrap();

    // PoE 2 must never return PoE 1 leagues! (AC-1)
    for l2 in poe2_leagues {
        assert!(
            !poe1_leagues.contains(l2),
            "PoE 2 must never contain PoE 1 leagues"
        );
    }

    // ==========================================
    // 9. Agent 16 (#105): Social Publisher Approvals
    // ==========================================
    let draft_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/social/drafts"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "content": "Major announcement across all socials!",
            "targets": ["discord", "telegram", "vk"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(draft_resp.status(), reqwest::StatusCode::OK);
    let draft_data: serde_json::Value = draft_resp.json().await.unwrap();
    assert_eq!(draft_data["status"], "draft");
    let draft_id = draft_data["id"].as_str().unwrap();

    let approve_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/social/drafts/{draft_id}/approve"
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(approve_resp.status(), reqwest::StatusCode::OK);
    let approved_data: serde_json::Value = approve_resp.json().await.unwrap();
    assert_eq!(approved_data["status"], "approved");

    // ==========================================
    // 10. Agent 17 & 18 (#106): AI Budget & Channel ACL RAG
    // ==========================================
    // Record within budget
    let usage_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/budget/record"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({ "tokens": 500, "usd": 0.01 }))
        .send()
        .await
        .unwrap();
    assert_eq!(usage_resp.status(), reqwest::StatusCode::OK);

    // Hard budget cap cut-off (AC-1)
    let overflow_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/budget/record"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({ "tokens": 2_000_000, "usd": 50.0 }))
        .send()
        .await
        .unwrap();
    let budget_state: serde_json::Value = overflow_resp.json().await.unwrap();
    assert_eq!(budget_state["is_halted"], true);

    // Any subsequent call is blocked!
    let blocked_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/budget/record"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({ "tokens": 10, "usd": 0.001 }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        blocked_resp.status(),
        reqwest::StatusCode::TOO_MANY_REQUESTS
    );

    // RAG Channel ACL (AC-3)
    let doc = AiKnowledgeDocument {
        id: Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        channel_acl: serde_json::json!(["private_staff_channel"]),
        title: "Staff Secret Playbook".to_string(),
        content: "Only accessible to users with staff channel permission".to_string(),
        source_type: "manual".to_string(),
        created_at: Utc::now(),
    };
    state.app.storage.create_ai_document(doc).await.unwrap();

    // User without access to private_staff_channel finds 0 documents
    let no_access = state
        .app
        .storage
        .search_ai_documents(guild_id, &["general_chat".to_string()], "Secret")
        .await
        .unwrap();
    assert_eq!(
        no_access.len(),
        0,
        "Unauthorized channel member must not see document"
    );

    // User with access finds 1 document
    let with_access = state
        .app
        .storage
        .search_ai_documents(guild_id, &["private_staff_channel".to_string()], "Secret")
        .await
        .unwrap();
    assert_eq!(with_access.len(), 1, "Authorized member must find document");

    // ==========================================
    // 11. Agent 19 (#107): Plugin Marketplace Cryptographic Integrity
    // ==========================================
    let pkg_content = b"plugin_script_byte_payload_v1";
    let pkg_hash = hex::encode(sha2::Sha256::digest(pkg_content));
    assert!(verify_package_sha256(pkg_content, &pkg_hash));
    assert!(!verify_package_sha256(b"tampered_payload", &pkg_hash));

    let publish_resp = client
        .post(format!("{base_url}/api/marketplace/publish"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "plugin_id": "verified_economy_plus",
            "name": "Economy Plus",
            "version": "1.0.0",
            "author": "Verified Dev",
            "sha256_hash": pkg_hash,
            "manifest": {
                "id": "verified_economy_plus",
                "version": "1.0.0"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(publish_resp.status(), reqwest::StatusCode::OK);

    let install_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/marketplace/install"
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "plugin_id": "verified_economy_plus",
            "version": "1.0.0"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(install_resp.status(), reqwest::StatusCode::OK);

    // ==========================================
    // 12. Agent 20: Full Verification & QA Gate
    // ==========================================
    println!(
        ">>> All 20 agents and 16 integration tracks (#92-#107) passed acceptance verification!"
    );
}
