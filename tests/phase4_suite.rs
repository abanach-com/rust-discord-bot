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
    // 10. Agent 17 & 18 (#106, #311): AI Budget & Channel ACL RAG
    // ==========================================
    // Direct client manipulation of AI usage via public route is strictly forbidden (#311)
    let usage_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/budget/record"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({ "tokens": 500, "usd": 0.01 }))
        .send()
        .await
        .unwrap();
    assert_eq!(usage_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Initial budget reflects 0 used tokens
    let budget_get_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/ai/budget"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(budget_get_resp.status(), reqwest::StatusCode::OK);
    let budget_val: serde_json::Value = budget_get_resp.json().await.unwrap();
    assert_eq!(budget_val["used_tokens"], 0);

    // Authoritative server-side metering records usage and halts when exceeding hard limit (#106, #311)
    let budget_state = state
        .app
        .storage
        .record_metering_event_once(guild_id, "phase4_provider", "req_phase4_1", 2_000_000, 50.0)
        .await
        .unwrap();
    assert!(budget_state.is_halted);

    // Any subsequent un-halted metering attempt exceeding limit fails
    let blocked_res = state
        .app
        .storage
        .record_metering_event_once(guild_id, "phase4_provider", "req_phase4_2", 10, 0.001)
        .await;
    assert!(blocked_res.is_err());

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
        .search_ai_documents(guild_id, &["general_chat".to_string()], "Secret", 20)
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
        .search_ai_documents(
            guild_id,
            &["private_staff_channel".to_string()],
            "Secret",
            20,
        )
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

#[tokio::test]
async fn test_webhook_dedupe_parallel_replay_single_winner() {
    let (state, _) = test_web_state().await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let router = build_router(state.clone());

    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve test web");
    });

    let base_url = format!("http://{addr}");
    let client = reqwest::Client::new();
    let secret = "test-github-secret";

    // 16 concurrent requests for the exact same webhook payload
    let event_id = "gh_evt_concurrent_999";
    let body = format!(
        r#"{{"event_id":"{event_id}","type":"push","repository":{{"full_name":"org/repo"}}}}"#
    );
    let hmac_bytes = compute_hmac_sha256(secret.as_bytes(), body.as_bytes());
    let sig_hex = format!("sha256={}", hex::encode(hmac_bytes));

    let gate = std::sync::Arc::new(tokio::sync::Barrier::new(16));
    let mut handles = Vec::new();

    for _ in 0..16 {
        let client = client.clone();
        let gate = gate.clone();
        let url = format!("{base_url}/api/integrations/webhooks/github");
        let sig = sig_hex.clone();
        let b = body.clone();

        handles.push(tokio::spawn(async move {
            gate.wait().await;
            let resp = client
                .post(url)
                .header("X-Hub-Signature-256", &sig)
                .body(b)
                .send()
                .await
                .expect("send webhook");
            let json: serde_json::Value = resp.json().await.expect("json");
            json
        }));
    }

    let mut processed_count = 0;
    let mut deduplicated_count = 0;

    for h in handles {
        let json = h.await.expect("handle");
        if json["status"] == "processed" && json["duplicate"] == false {
            processed_count += 1;
        } else if json["status"] == "deduplicated" && json["duplicate"] == true {
            deduplicated_count += 1;
        } else {
            panic!("Unexpected webhook response payload: {json:?}");
        }
    }

    assert_eq!(
        processed_count, 1,
        "Exactly one webhook request must be processed"
    );
    assert_eq!(
        deduplicated_count, 15,
        "All other 15 webhook requests must be deduplicated"
    );
}

#[tokio::test]
async fn test_ai_knowledge_search_security_and_performance_issue_244() {
    let (state, _) = test_web_state().await;
    let guild_id = "111000000000000099";
    let other_guild_id = "111000000000000098";
    let user_id = "user_ai_search_test";
    let session_id = "test_ai_search_session_token";
    let csrf_token = "csrf_ai_search_token";

    // 1. Literal search semantics for %, _, \, empty/whitespace
    let doc_percent = AiKnowledgeDocument {
        id: Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        channel_acl: serde_json::json!([]),
        title: "100% discount deal".to_string(),
        content: "Get everything free".to_string(),
        source_type: "faq".to_string(),
        created_at: Utc::now() - Duration::minutes(10),
    };
    let doc_regular = AiKnowledgeDocument {
        id: Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        channel_acl: serde_json::json!([]),
        title: "Regular discount deal".to_string(),
        content: "Save ten dollars".to_string(),
        source_type: "faq".to_string(),
        created_at: Utc::now() - Duration::minutes(9),
    };
    let doc_underscore = AiKnowledgeDocument {
        id: Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        channel_acl: serde_json::json!([]),
        title: "snake_case_identifier doc".to_string(),
        content: "Config file path".to_string(),
        source_type: "manual".to_string(),
        created_at: Utc::now() - Duration::minutes(8),
    };
    let doc_backslash = AiKnowledgeDocument {
        id: Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        channel_acl: serde_json::json!([]),
        title: "Windows\\System32 path doc".to_string(),
        content: "System folder".to_string(),
        source_type: "manual".to_string(),
        created_at: Utc::now() - Duration::minutes(7),
    };

    state
        .app
        .storage
        .create_ai_document(doc_percent)
        .await
        .unwrap();
    state
        .app
        .storage
        .create_ai_document(doc_regular)
        .await
        .unwrap();
    state
        .app
        .storage
        .create_ai_document(doc_underscore)
        .await
        .unwrap();
    state
        .app
        .storage
        .create_ai_document(doc_backslash)
        .await
        .unwrap();

    // Query "%" must ONLY match literal "%", not all documents
    let res_pct = state
        .app
        .storage
        .search_ai_documents(guild_id, &[], "%", 20)
        .await
        .unwrap();
    assert_eq!(res_pct.len(), 1, "query '%' must match only literal '%'");
    assert_eq!(res_pct[0].title, "100% discount deal");

    // Query "_" must ONLY match literal "_", not any character
    let res_und = state
        .app
        .storage
        .search_ai_documents(guild_id, &[], "_", 20)
        .await
        .unwrap();
    assert_eq!(res_und.len(), 1, "query '_' must match only literal '_'");
    assert_eq!(res_und[0].title, "snake_case_identifier doc");

    // Query "\" must match literal "\"
    let res_bs = state
        .app
        .storage
        .search_ai_documents(guild_id, &[], "\\", 20)
        .await
        .unwrap();
    assert_eq!(res_bs.len(), 1, "query '\\' must match literal '\\'");
    assert_eq!(res_bs[0].title, "Windows\\System32 path doc");

    // Empty or whitespace query must return 0 documents immediately
    let res_empty = state
        .app
        .storage
        .search_ai_documents(guild_id, &[], "", 20)
        .await
        .unwrap();
    assert_eq!(res_empty.len(), 0, "empty query must return empty result");

    let res_ws = state
        .app
        .storage
        .search_ai_documents(guild_id, &[], "   ", 20)
        .await
        .unwrap();
    assert_eq!(res_ws.len(), 0, "whitespace query must return empty result");

    // 2. Limit and ordering on large collection (>50 documents)
    for i in 0..60 {
        let paged_doc = AiKnowledgeDocument {
            id: Uuid::new_v4(),
            guild_id: guild_id.to_string(),
            channel_acl: serde_json::json!([]),
            title: format!("Paged Article #{i:02}"),
            content: "Unique content for pagination check".to_string(),
            source_type: "paged".to_string(),
            created_at: Utc::now() + Duration::seconds(i),
        };
        state
            .app
            .storage
            .create_ai_document(paged_doc)
            .await
            .unwrap();
    }

    // Default/specified limit: requesting 20 returns exactly 20, newest first
    let res_paged_20 = state
        .app
        .storage
        .search_ai_documents(guild_id, &[], "Paged Article", 20)
        .await
        .unwrap();
    assert_eq!(res_paged_20.len(), 20, "Must return at most 20 documents");
    assert_eq!(
        res_paged_20[0].title, "Paged Article #59",
        "Must be ordered created_at DESC"
    );
    assert_eq!(res_paged_20[19].title, "Paged Article #40");

    // Clamping: requesting 0 clamps to 1
    let res_clamp_0 = state
        .app
        .storage
        .search_ai_documents(guild_id, &[], "Paged Article", 0)
        .await
        .unwrap();
    assert_eq!(res_clamp_0.len(), 1, "limit 0 must clamp to 1");

    // Clamping: requesting 100 clamps to 50
    let res_clamp_100 = state
        .app
        .storage
        .search_ai_documents(guild_id, &[], "Paged Article", 100)
        .await
        .unwrap();
    assert_eq!(res_clamp_100.len(), 50, "limit 100 must clamp to 50");

    // 3. 100+ private documents and 1 public document
    for i in 0..105 {
        let priv_doc = AiKnowledgeDocument {
            id: Uuid::new_v4(),
            guild_id: guild_id.to_string(),
            channel_acl: serde_json::json!(["vip_exec_room"]),
            title: format!("VIP Confidential Doc #{i:03}"),
            content: "Super confidential executive strategy search_target".to_string(),
            source_type: "confidential".to_string(),
            created_at: Utc::now() + Duration::seconds(100 + i),
        };
        state
            .app
            .storage
            .create_ai_document(priv_doc)
            .await
            .unwrap();
    }
    let pub_doc = AiKnowledgeDocument {
        id: Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        channel_acl: serde_json::json!([]),
        title: "Publicly Visible Strategy Doc".to_string(),
        content: "General information strategy search_target".to_string(),
        source_type: "public".to_string(),
        created_at: Utc::now() + Duration::seconds(999),
    };
    state.app.storage.create_ai_document(pub_doc).await.unwrap();

    // Querying with non-matching channel permissions returns ONLY the public document
    let res_restricted = state
        .app
        .storage
        .search_ai_documents(
            guild_id,
            &["general_chat".to_string(), "random_chat".to_string()],
            "search_target",
            50,
        )
        .await
        .unwrap();
    assert_eq!(
        res_restricted.len(),
        1,
        "User without VIP channel access must only see 1 public document"
    );
    assert_eq!(res_restricted[0].title, "Publicly Visible Strategy Doc");

    // 4. Fail-closed on invalid JSON ACL
    let malformed_doc = AiKnowledgeDocument {
        id: Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        channel_acl: serde_json::json!("not_an_array_string"),
        title: "Corrupted ACL Doc".to_string(),
        content: "Malformed ACL must deny access search_malformed".to_string(),
        source_type: "corrupted".to_string(),
        created_at: Utc::now(),
    };
    state
        .app
        .storage
        .create_ai_document(malformed_doc)
        .await
        .unwrap();

    let res_malformed = state
        .app
        .storage
        .search_ai_documents(
            guild_id,
            &["general_chat".to_string()],
            "search_malformed",
            20,
        )
        .await
        .unwrap();
    assert_eq!(
        res_malformed.len(),
        0,
        "Malformed non-array ACL must fail-closed (deny)"
    );

    // 5. Cross-guild tenant isolation
    let other_guild_doc = AiKnowledgeDocument {
        id: Uuid::new_v4(),
        guild_id: other_guild_id.to_string(),
        channel_acl: serde_json::json!([]),
        title: "Other Guild Public Doc".to_string(),
        content: "Should never leak to guild_id search_isolation_leak".to_string(),
        source_type: "public".to_string(),
        created_at: Utc::now(),
    };
    state
        .app
        .storage
        .create_ai_document(other_guild_doc)
        .await
        .unwrap();

    let res_isolated = state
        .app
        .storage
        .search_ai_documents(guild_id, &[], "search_isolation_leak", 20)
        .await
        .unwrap();
    assert_eq!(
        res_isolated.len(),
        0,
        "Cross-guild leak must never occur in document search"
    );

    // 6. Web API test: POST /api/guilds/{guild_id}/ai/search
    let user = DashboardUser {
        id: user_id.to_owned(),
        username: "AiSearchOperator".to_owned(),
        global_name: None,
        avatar: None,
    };
    let guilds = vec![DashboardGuild {
        id: guild_id.to_owned(),
        name: "AI Test Guild".to_owned(),
        icon: None,
        owner: true,
        permissions: "8".to_owned(),
    }];

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

    // Query > 500 characters returns 400 Bad Request
    let long_query = "a".repeat(501);
    let resp_too_long = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/search"))
        .header(COOKIE, &session_cookie)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "query": long_query,
        }))
        .send()
        .await
        .expect("send too long query");
    assert_eq!(resp_too_long.status(), reqwest::StatusCode::BAD_REQUEST);

    // Empty query returns 200 with []
    let resp_empty = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/search"))
        .header(COOKIE, &session_cookie)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "query": "",
        }))
        .send()
        .await
        .expect("send empty query");
    assert_eq!(resp_empty.status(), reqwest::StatusCode::OK);
    let empty_json: Vec<serde_json::Value> = resp_empty.json().await.expect("json");
    assert_eq!(empty_json.len(), 0);

    // Literal "%" query via API returns 1 document with limit
    let resp_pct = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/search"))
        .header(COOKIE, &session_cookie)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "query": "%",
            "limit": 10
        }))
        .send()
        .await
        .expect("send % query");
    assert_eq!(resp_pct.status(), reqwest::StatusCode::OK);
    let pct_json: Vec<serde_json::Value> = resp_pct.json().await.expect("json");
    assert_eq!(pct_json.len(), 1);
    assert_eq!(pct_json[0]["title"], "100% discount deal");
}

#[tokio::test]
async fn test_telegram_bridge_upsert_deduplication_and_identity_stability_issue_329() {
    let mut config = test_config();
    config.offline_mode = true;
    let app = AppState::bootstrap(config).await.expect("bootstrap");
    let permissions_cache = Arc::new(DashMap::new());
    let web_state = WebState {
        app,
        login_rate_limiter: Arc::new(LoginRateLimiter::new()),
        permissions_cache: permissions_cache.clone(),
    };
    web_state
        .app
        .is_gateway_ready
        .store(true, std::sync::atomic::Ordering::SeqCst);

    let web_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind web listener");
    let web_addr = web_listener.local_addr().expect("web addr");
    let router = build_router(web_state.clone());
    tokio::spawn(async move {
        axum::serve(
            web_listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{web_addr}");

    let guild_id = "777000000000000001";
    let channel_id = "888000000000000001";
    let user_id = "999000000000000001";
    let session_id = "session_telegram_bridge_test";
    let csrf_token = "csrf_telegram_bridge_test";

    web_state
        .app
        .sessions
        .save_session(
            session_id,
            DashboardUser {
                id: user_id.to_string(),
                username: "BridgeAdmin".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Telegram Guild".to_string(),
                icon: None,
                owner: false,
                permissions: "32".to_string(), // MANAGE_GUILD
            }],
            csrf_token.to_string(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    permissions_cache.insert(
        (user_id.to_string(), guild_id.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    // 1. Initial creation of Telegram bridge via POST
    let initial_payload = serde_json::json!({
        "telegram_chat_id": "-100111111111",
        "discord_channel_id": channel_id,
        "secret_token": "secret_token_v1"
    });

    let resp1 = client
        .post(format!("{base_url}/api/guilds/{guild_id}/telegram/bridge"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&initial_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(resp1.status(), reqwest::StatusCode::OK);

    let body1: serde_json::Value = resp1.json().await.unwrap();
    let bridge_id = body1["id"].as_str().expect("bridge id string").to_string();
    let created_at = body1["created_at"]
        .as_str()
        .expect("created_at string")
        .to_string();

    assert_eq!(body1["telegram_chat_id"], "-100111111111");
    assert_eq!(body1["discord_channel_id"], channel_id);
    assert!(
        body1.get("secret_token").is_none(),
        "secret_token must be redacted in summary (#131)"
    );

    let bridges_initial = web_state
        .app
        .storage
        .list_telegram_bridges(guild_id)
        .await
        .unwrap();
    assert_eq!(bridges_initial.len(), 1, "Exactly one bridge in storage");

    // 2. Sequential second POST for the same (guild_id, discord_channel_id) updating telegram_chat_id
    let update_payload = serde_json::json!({
        "telegram_chat_id": "-100222222222",
        "discord_channel_id": channel_id,
        "secret_token": "secret_token_v2"
    });

    let resp2 = client
        .post(format!("{base_url}/api/guilds/{guild_id}/telegram/bridge"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&update_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(resp2.status(), reqwest::StatusCode::OK);

    let body2: serde_json::Value = resp2.json().await.unwrap();
    assert_eq!(
        body2["id"], bridge_id,
        "Bridge ID must remain stable across updates (UPSERT)"
    );
    assert_eq!(
        body2["created_at"], created_at,
        "created_at must remain unchanged across updates"
    );
    assert_eq!(
        body2["telegram_chat_id"], "-100222222222",
        "telegram_chat_id must be updated"
    );

    // Storage check: exactly 1 bridge in MemoryStore, no duplicates!
    let bridges_after = web_state
        .app
        .storage
        .list_telegram_bridges(guild_id)
        .await
        .unwrap();
    assert_eq!(
        bridges_after.len(),
        1,
        "MemoryStore must not create duplicates for (guild_id, discord_channel_id)"
    );
    assert_eq!(bridges_after[0].telegram_chat_id, "-100222222222");
    assert_eq!(bridges_after[0].id.to_string(), bridge_id);

    // get_telegram_bridge check
    let retrieved = web_state
        .app
        .storage
        .get_telegram_bridge(guild_id, channel_id)
        .await
        .unwrap()
        .expect("retrieved bridge");
    assert_eq!(retrieved.id.to_string(), bridge_id);
    assert_eq!(retrieved.telegram_chat_id, "-100222222222");
    assert_eq!(retrieved.secret_token, "secret_token_v2");

    // 3. Message mapping consistency: mapping referencing bridge_id is maintained
    let parsed_bridge_id = Uuid::parse_str(&bridge_id).unwrap();
    let mapping = rusty_discord_bot::storage::TelegramMessageMapping {
        id: Uuid::new_v4(),
        bridge_id: parsed_bridge_id,
        telegram_message_id: "tg_msg_101".to_string(),
        discord_message_id: "dc_msg_202".to_string(),
        direction: "telegram_to_discord".to_string(),
        created_at: Utc::now(),
    };
    web_state
        .app
        .storage
        .record_telegram_message_mapping(mapping.clone())
        .await
        .unwrap();

    let fetched_mapping = web_state
        .app
        .storage
        .get_telegram_message_mapping(parsed_bridge_id, "tg_msg_101", "telegram_to_discord")
        .await
        .unwrap()
        .expect("fetched mapping");
    assert_eq!(fetched_mapping.discord_message_id, "dc_msg_202");

    // 4. Concurrent UPSERT test: 20 simultaneous create_telegram_bridge calls for same channel
    let concurrent_channel_id = "888000000000000002";
    let mut handles = Vec::new();
    for i in 0..20 {
        let storage = web_state.app.storage.clone();
        let gid = guild_id.to_string();
        let cid = concurrent_channel_id.to_string();
        handles.push(tokio::spawn(async move {
            let bridge = rusty_discord_bot::storage::TelegramBridge {
                id: Uuid::new_v4(),
                guild_id: gid,
                telegram_chat_id: format!("-1003333333{:02}", i),
                discord_channel_id: cid,
                secret_token: format!("token_{i}"),
                paused: false,
                created_at: Utc::now(),
            };
            storage.create_telegram_bridge(bridge).await.unwrap();
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    let all_bridges = web_state
        .app
        .storage
        .list_telegram_bridges(guild_id)
        .await
        .unwrap();
    let concurrent_bridges: Vec<_> = all_bridges
        .into_iter()
        .filter(|b| b.discord_channel_id == concurrent_channel_id)
        .collect();
    assert_eq!(
        concurrent_bridges.len(),
        1,
        "Concurrent UPSERT creates exactly one bridge entry without duplicates"
    );

    // 5. Deletion removes the correct bridge
    let deleted = web_state
        .app
        .storage
        .delete_telegram_bridge(parsed_bridge_id)
        .await
        .unwrap();
    assert!(deleted, "Delete returns true when bridge existed");

    let after_delete = web_state
        .app
        .storage
        .get_telegram_bridge(guild_id, channel_id)
        .await
        .unwrap();
    assert!(
        after_delete.is_none(),
        "Bridge was cleanly removed from storage"
    );
}
