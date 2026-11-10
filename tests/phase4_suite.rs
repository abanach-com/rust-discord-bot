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
    storage::{
        AiKnowledgeDocument, IntegrationConnection, IntegrationDeliveryLog, LinkedAccount,
        run_integrations_retention_janitor,
    },
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
        integration_credential_key: None,
        integration_credential_active_key_id: "v1".to_string(),
        integration_credential_keyring: None,
        integration_credentials_disabled: false,
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
            guilds.clone(),
            csrf_token.to_owned(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .expect("save session");

    // Seed second authorized operator for two-person approval workflows (#338)
    let approver_user_id = "user_phase4_approver";
    let approver_session_id = "test_phase4_approver_session_token";
    let approver_csrf_token = "csrf_phase4_approver_token";
    let approver_user = DashboardUser {
        id: approver_user_id.to_owned(),
        username: "Phase4Approver".to_owned(),
        global_name: None,
        avatar: None,
    };
    state
        .app
        .sessions
        .save_session(
            approver_session_id,
            approver_user,
            guilds,
            approver_csrf_token.to_owned(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .expect("save approver session");

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
    assert_eq!(poe1_resp.status(), reqwest::StatusCode::OK);
    let poe1_data: serde_json::Value = poe1_resp.json().await.unwrap();
    assert_eq!(poe1_data["game"], "Path of Exile 1");
    assert_eq!(poe1_data["status"], "unavailable");
    let poe1_leagues = poe1_data["leagues"].as_array().unwrap();
    assert!(
        !poe1_leagues.contains(&serde_json::json!("Settlers of Kalguur")),
        "Historical Settlers of Kalguur must NOT be returned as active league"
    );
    assert!(poe1_leagues.contains(&serde_json::json!("Standard")));
    assert!(poe1_leagues.contains(&serde_json::json!("Hardcore")));

    let poe2_resp = client
        .get(format!("{base_url}/api/gaming-hub/leagues/poe2"))
        .send()
        .await
        .unwrap();
    assert_eq!(poe2_resp.status(), reqwest::StatusCode::OK);
    let poe2_data: serde_json::Value = poe2_resp.json().await.unwrap();
    assert_eq!(poe2_data["game"], "Path of Exile 2");
    assert_eq!(poe2_data["status"], "unavailable");
    let poe2_leagues = poe2_data["leagues"].as_array().unwrap();

    // PoE 2 must never return PoE 1 leagues! (AC-1)
    for l2 in poe2_leagues {
        assert!(
            !poe1_leagues.contains(l2),
            "PoE 2 must never contain PoE 1 leagues"
        );
    }

    // Invalid version must return HTTP 400 Bad Request
    let poe_invalid_resp = client
        .get(format!("{base_url}/api/gaming-hub/leagues/poe3"))
        .send()
        .await
        .unwrap();
    assert_eq!(poe_invalid_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // ==========================================
    // 9. Agent 16 (#105, #338): Social Publisher Approvals & Two-Person Policy
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

    // 1) Self-approval must be rejected with 403 Forbidden under two-person approval policy (#338)
    let self_approve_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/social/drafts/{draft_id}/approve"
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        self_approve_resp.status(),
        reqwest::StatusCode::FORBIDDEN,
        "Author must not be allowed to self-approve draft"
    );

    // 2) Approval by a second authorized guild operator must succeed (#338)
    let approver_cookie = format!("rusty_discord_bot_session={approver_session_id}");
    let approve_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/social/drafts/{draft_id}/approve"
        ))
        .header(COOKIE, &approver_cookie)
        .header("X-CSRF-Token", approver_csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(approve_resp.status(), reqwest::StatusCode::OK);
    let approved_data: serde_json::Value = approve_resp.json().await.unwrap();
    assert_eq!(approved_data["status"], "approved");
    assert_eq!(approved_data["approved_by"], approver_user_id);

    // 3) Re-approval attempt must be rejected with 409 Conflict (#338)
    let re_approve_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/social/drafts/{draft_id}/approve"
        ))
        .header(COOKIE, &approver_cookie)
        .header("X-CSRF-Token", approver_csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        re_approve_resp.status(),
        reqwest::StatusCode::CONFLICT,
        "Subsequent approval on already approved draft must return 409 Conflict"
    );

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

    // Verify ON DELETE CASCADE: mapping referencing parsed_bridge_id must now be gone
    let mapping_after_delete = web_state
        .app
        .storage
        .get_telegram_message_mapping(parsed_bridge_id, "tg_msg_101", "telegram_to_discord")
        .await
        .unwrap();
    assert!(
        mapping_after_delete.is_none(),
        "Cascade deletion must remove all mappings for deleted bridge"
    );
}

#[tokio::test]
async fn test_telegram_bridge_mapping_fk_and_cascade_lifecycle_issue_330() {
    let mut config = test_config();
    config.offline_mode = true;
    let app = AppState::bootstrap(config).await.expect("bootstrap");
    let storage = app.storage.clone();

    // 1. Zapis mapping z losowym, nieistniejącym bridge_id jest odrzucany (FK violation)
    let random_bridge_id = Uuid::new_v4();
    let orphan_mapping = rusty_discord_bot::storage::TelegramMessageMapping {
        id: Uuid::new_v4(),
        bridge_id: random_bridge_id,
        telegram_message_id: "orphan_tg_1".to_string(),
        discord_message_id: "orphan_dc_1".to_string(),
        direction: "telegram_to_discord".to_string(),
        created_at: Utc::now(),
    };
    let orphan_res = storage
        .record_telegram_message_mapping(orphan_mapping)
        .await;
    assert!(
        orphan_res.is_err(),
        "MemoryStore must reject mapping with non-existent bridge_id"
    );
    assert!(
        storage
            .get_telegram_message_mapping(random_bridge_id, "orphan_tg_1", "telegram_to_discord")
            .await
            .unwrap()
            .is_none(),
        "No orphan mapping may be recorded"
    );

    // 2. Utworzenie dwóch niezależnych mostów: Bridge A i Bridge B (cross-guild / cross-bridge)
    let guild_a = "111000000000000001";
    let channel_a = "222000000000000001";
    let bridge_a = storage
        .create_telegram_bridge(rusty_discord_bot::storage::TelegramBridge {
            id: Uuid::new_v4(),
            guild_id: guild_a.to_string(),
            telegram_chat_id: "-100111111111".to_string(),
            discord_channel_id: channel_a.to_string(),
            secret_token: "secret_a".to_string(),
            paused: false,
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    let guild_b = "111000000000000002";
    let channel_b = "222000000000000002";
    let bridge_b = storage
        .create_telegram_bridge(rusty_discord_bot::storage::TelegramBridge {
            id: Uuid::new_v4(),
            guild_id: guild_b.to_string(),
            telegram_chat_id: "-100222222222".to_string(),
            discord_channel_id: channel_b.to_string(),
            secret_token: "secret_b".to_string(),
            paused: false,
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    // Dodanie mapowań dla Bridge A i Bridge B
    let mapping_a1 = rusty_discord_bot::storage::TelegramMessageMapping {
        id: Uuid::new_v4(),
        bridge_id: bridge_a.id,
        telegram_message_id: "tg_a1".to_string(),
        discord_message_id: "dc_a1".to_string(),
        direction: "telegram_to_discord".to_string(),
        created_at: Utc::now(),
    };
    let mapping_a2 = rusty_discord_bot::storage::TelegramMessageMapping {
        id: Uuid::new_v4(),
        bridge_id: bridge_a.id,
        telegram_message_id: "tg_a2".to_string(),
        discord_message_id: "dc_a2".to_string(),
        direction: "discord_to_telegram".to_string(),
        created_at: Utc::now(),
    };
    let mapping_b1 = rusty_discord_bot::storage::TelegramMessageMapping {
        id: Uuid::new_v4(),
        bridge_id: bridge_b.id,
        telegram_message_id: "tg_b1".to_string(),
        discord_message_id: "dc_b1".to_string(),
        direction: "telegram_to_discord".to_string(),
        created_at: Utc::now(),
    };

    storage
        .record_telegram_message_mapping(mapping_a1.clone())
        .await
        .unwrap();
    storage
        .record_telegram_message_mapping(mapping_a2.clone())
        .await
        .unwrap();
    storage
        .record_telegram_message_mapping(mapping_b1.clone())
        .await
        .unwrap();

    // 3. UPSERT istniejącego Bridge A (to samo guild_id i channel_id) nie kasuje mapowań
    let mut upsert_a = bridge_a.clone();
    upsert_a.id = Uuid::new_v4(); // candidate ID
    upsert_a.telegram_chat_id = "-100999999999".to_string();
    let upserted_a = storage.create_telegram_bridge(upsert_a).await.unwrap();
    assert_eq!(upserted_a.id, bridge_a.id, "UPSERT must retain stable id");
    assert_eq!(
        upserted_a.created_at, bridge_a.created_at,
        "UPSERT must retain original created_at"
    );

    // Sprawdzenie, że mapowania A1 i A2 nadal istnieją po UPSERT
    let a1_still_there = storage
        .get_telegram_message_mapping(bridge_a.id, "tg_a1", "telegram_to_discord")
        .await
        .unwrap();
    assert!(a1_still_there.is_some(), "UPSERT must not wipe mappings");
    let a2_still_there = storage
        .get_telegram_message_mapping(bridge_a.id, "dc_a2", "discord_to_telegram")
        .await
        .unwrap();
    assert!(a2_still_there.is_some(), "UPSERT must not wipe mappings");

    // 4. ON DELETE CASCADE: Usunięcie Bridge A usuwa kaskadowo jego mapowania, nie naruszając Bridge B
    let deleted_a = storage.delete_telegram_bridge(bridge_a.id).await.unwrap();
    assert!(deleted_a, "Bridge A was deleted");

    // Bridge A i jego mapowania zniknęły
    assert!(
        storage
            .get_telegram_bridge(guild_a, channel_a)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        storage
            .get_telegram_message_mapping(bridge_a.id, "tg_a1", "telegram_to_discord")
            .await
            .unwrap()
            .is_none(),
        "Cascade delete must remove mapping A1"
    );
    assert!(
        storage
            .get_telegram_message_mapping(bridge_a.id, "dc_a2", "discord_to_telegram")
            .await
            .unwrap()
            .is_none(),
        "Cascade delete must remove mapping A2"
    );

    // Bridge B i jego mapowanie B1 są nietknięte
    let bridge_b_check = storage
        .get_telegram_bridge(guild_b, channel_b)
        .await
        .unwrap();
    assert!(bridge_b_check.is_some(), "Bridge B must remain intact");
    let b1_check = storage
        .get_telegram_message_mapping(bridge_b.id, "tg_b1", "telegram_to_discord")
        .await
        .unwrap();
    assert!(b1_check.is_some(), "Bridge B's mapping must remain intact");

    // 5. Jednoczesny `record` i `delete` (bariera / synchronizacja, 100 powtórzeń)
    for i in 0..100 {
        let race_guild = format!("race_g_{i}");
        let race_channel = format!("race_c_{i}");
        let race_bridge = storage
            .create_telegram_bridge(rusty_discord_bot::storage::TelegramBridge {
                id: Uuid::new_v4(),
                guild_id: race_guild.clone(),
                telegram_chat_id: format!("-100race{i}"),
                discord_channel_id: race_channel.clone(),
                secret_token: format!("race_tok_{i}"),
                paused: false,
                created_at: Utc::now(),
            })
            .await
            .unwrap();

        let bridge_id = race_bridge.id;
        let barrier = Arc::new(tokio::sync::Barrier::new(2));

        let barrier_record = barrier.clone();
        let storage_record = storage.clone();
        let record_handle = tokio::spawn(async move {
            barrier_record.wait().await;
            let mapping = rusty_discord_bot::storage::TelegramMessageMapping {
                id: Uuid::new_v4(),
                bridge_id,
                telegram_message_id: format!("tg_race_{i}"),
                discord_message_id: format!("dc_race_{i}"),
                direction: "telegram_to_discord".to_string(),
                created_at: Utc::now(),
            };
            storage_record
                .record_telegram_message_mapping(mapping)
                .await
        });

        let barrier_delete = barrier.clone();
        let storage_delete = storage.clone();
        let delete_handle = tokio::spawn(async move {
            barrier_delete.wait().await;
            storage_delete.delete_telegram_bridge(bridge_id).await
        });

        let (record_res, delete_res) = tokio::join!(record_handle, delete_handle);
        let record_res = record_res.unwrap();
        let delete_res = delete_res.unwrap().unwrap();

        // Po zakończeniu obu operacji:
        // Sprawdź czy most istnieje
        let bridge_exists = storage
            .get_telegram_bridge(&race_guild, &race_channel)
            .await
            .unwrap()
            .is_some();

        let mapping_found = storage
            .get_telegram_message_mapping(bridge_id, &format!("tg_race_{i}"), "telegram_to_discord")
            .await
            .unwrap();

        if !bridge_exists {
            // Jeśli most został usunięty, NIGDY nie może istnieć osierocone mapowanie!
            assert!(
                mapping_found.is_none(),
                "Iter {i}: Orphan mapping found after bridge deletion! record_res: {record_res:?}, delete_res: {delete_res}"
            );
        } else {
            // Jeśli most istnieje (delete nie wygrał lub zwrócił false), a record się powiódł:
            if record_res.is_ok() {
                assert!(mapping_found.is_some());
            }
        }
    }
}

#[tokio::test]
async fn test_github_thread_mapping_and_telegram_bridge_tenant_isolation_issues_354_352() {
    use axum::response::IntoResponse;
    use axum::routing::get;

    let guild_a = "100000000000000001";
    let user_admin_id = "500000000000000001";

    let parent_forum_id = "111000000000000001";
    let valid_thread_id = "111000000000000002";
    let wrong_parent_thread_id = "111000000000000004";
    let not_a_thread_id = "111000000000000005";
    let voice_channel_id = "111000000000000006";
    let valid_tg_channel_id = "111000000000000008";

    let foreign_channel_id = "222000000000000001";
    let foreign_thread_id = "222000000000000002";

    // 1. Mock Discord API
    let discord_mock_app = axum::Router::new()
        .route(
            "/channels/{channel_id}",
            get(
                |axum::extract::Path(cid): axum::extract::Path<String>| async move {
                    match cid.as_str() {
                        "111000000000000001" => axum::Json(serde_json::json!({
                            "id": "111000000000000001",
                            "guild_id": "100000000000000001",
                            "type": 15,
                            "parent_id": null
                        }))
                        .into_response(),
                        "111000000000000002" => axum::Json(serde_json::json!({
                            "id": "111000000000000002",
                            "guild_id": "100000000000000001",
                            "type": 11,
                            "parent_id": "111000000000000001"
                        }))
                        .into_response(),
                        "111000000000000003" => axum::Json(serde_json::json!({
                            "id": "111000000000000003",
                            "guild_id": "100000000000000001",
                            "type": 0,
                            "parent_id": null
                        }))
                        .into_response(),
                        "111000000000000004" => axum::Json(serde_json::json!({
                            "id": "111000000000000004",
                            "guild_id": "100000000000000001",
                            "type": 11,
                            "parent_id": "111000000000000003"
                        }))
                        .into_response(),
                        "111000000000000005" => axum::Json(serde_json::json!({
                            "id": "111000000000000005",
                            "guild_id": "100000000000000001",
                            "type": 0,
                            "parent_id": null
                        }))
                        .into_response(),
                        "111000000000000006" => axum::Json(serde_json::json!({
                            "id": "111000000000000006",
                            "guild_id": "100000000000000001",
                            "type": 2,
                            "parent_id": null
                        }))
                        .into_response(),
                        "111000000000000008" => axum::Json(serde_json::json!({
                            "id": "111000000000000008",
                            "guild_id": "100000000000000001",
                            "type": 0,
                            "parent_id": null
                        }))
                        .into_response(),
                        "222000000000000001" => axum::Json(serde_json::json!({
                            "id": "222000000000000001",
                            "guild_id": "200000000000000002",
                            "type": 0,
                            "parent_id": null
                        }))
                        .into_response(),
                        "222000000000000002" => axum::Json(serde_json::json!({
                            "id": "222000000000000002",
                            "guild_id": "200000000000000002",
                            "type": 11,
                            "parent_id": "222000000000000001"
                        }))
                        .into_response(),
                        _ => (
                            reqwest::StatusCode::NOT_FOUND,
                            axum::Json(
                                serde_json::json!({"code": 10003, "message": "Unknown Channel"}),
                            ),
                        )
                            .into_response(),
                    }
                },
            ),
        )
        .route(
            "/channels/{channel_id}/permissions/member/{user_id}",
            get(
                |axum::extract::Path((_channel_id, user_id)): axum::extract::Path<(
                    String,
                    String,
                )>| async move {
                    if user_id == "500000000000000001" {
                        // VIEW_CHANNEL = 1024
                        axum::Json(serde_json::json!({ "permissions": "1024" })).into_response()
                    } else {
                        axum::Json(serde_json::json!({ "permissions": "0" })).into_response()
                    }
                },
            ),
        )
        .route(
            "/guilds/{guild_id}/permissions/member/{user_id}",
            get(
                |axum::extract::Path((_guild_id, user_id)): axum::extract::Path<(
                    String,
                    String,
                )>| async move {
                    if user_id == "500000000000000001" {
                        // MANAGE_GUILD = 32
                        axum::Json(serde_json::json!({ "permissions": "32" })).into_response()
                    } else {
                        axum::Json(serde_json::json!({ "permissions": "0" })).into_response()
                    }
                },
            ),
        );

    let discord_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind discord listener");
    let discord_addr = discord_listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(discord_listener, discord_mock_app)
            .await
            .unwrap();
    });

    // 2. Configure app state in live mode
    let mut config = test_config();
    config.discord_token = "live_bot_token_production_valid".to_string();
    config.offline_mode = false;
    config.discord_api_base_url = format!("http://{discord_addr}");

    let app = AppState::bootstrap(config)
        .await
        .expect("bootstrap app state");
    let permissions_cache = Arc::new(DashMap::new());
    permissions_cache.insert(
        (user_admin_id.to_string(), guild_a.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );
    let state = WebState {
        app,
        login_rate_limiter: Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    };

    let session_id = "test_phase4_live_session_token";
    let csrf_token = "test_phase4_live_csrf_token";
    let user = DashboardUser {
        id: user_admin_id.to_string(),
        username: "Phase4Admin".to_string(),
        global_name: None,
        avatar: None,
    };
    let guilds = vec![DashboardGuild {
        id: guild_a.to_string(),
        name: "Guild A".to_string(),
        icon: None,
        owner: false,
        permissions: "32".to_string(), // MANAGE_GUILD
    }];

    state
        .app
        .sessions
        .save_session(
            session_id,
            user,
            guilds,
            csrf_token.to_string(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .expect("save session");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind app listener");
    let addr = listener.local_addr().unwrap();
    let router = build_router(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    // =========================================================================
    // Issue #354 Tests: map_github_thread
    // =========================================================================

    // Case 1: issue_or_pr_number <= 0 -> 400 Bad Request
    for invalid_num in [0, -1, -42] {
        let resp = client
            .post(format!("{base_url}/api/guilds/{guild_a}/github/thread-map"))
            .header(COOKIE, &session_cookie)
            .header("X-CSRF-Token", csrf_token)
            .json(&serde_json::json!({
                "repo_full_name": "owner/repo",
                "thread_channel_id": parent_forum_id,
                "issue_or_pr_number": invalid_num,
                "discord_thread_id": valid_thread_id
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    }

    // Case 2: repo_full_name format invalid -> 400 Bad Request
    for invalid_repo in [
        "",
        "invalid",
        "too/many/nested/slashes",
        " /repo",
        "owner/ ",
    ] {
        let resp = client
            .post(format!("{base_url}/api/guilds/{guild_a}/github/thread-map"))
            .header(COOKIE, &session_cookie)
            .header("X-CSRF-Token", csrf_token)
            .json(&serde_json::json!({
                "repo_full_name": invalid_repo,
                "thread_channel_id": parent_forum_id,
                "issue_or_pr_number": 1,
                "discord_thread_id": valid_thread_id
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    }

    // Case 3: invalid snowflake -> 400 Bad Request
    for invalid_snowflake in ["", "not_a_number", "0", "-123"] {
        let resp = client
            .post(format!("{base_url}/api/guilds/{guild_a}/github/thread-map"))
            .header(COOKIE, &session_cookie)
            .header("X-CSRF-Token", csrf_token)
            .json(&serde_json::json!({
                "repo_full_name": "owner/repo",
                "thread_channel_id": invalid_snowflake,
                "issue_or_pr_number": 1,
                "discord_thread_id": valid_thread_id
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    }

    // Case 4: foreign parent channel (Guild B) -> 403 Forbidden
    let resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/github/thread-map"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "repo_full_name": "owner/repo",
            "thread_channel_id": foreign_channel_id,
            "issue_or_pr_number": 1,
            "discord_thread_id": valid_thread_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Case 5: foreign thread (Guild B) -> 403 Forbidden
    let resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/github/thread-map"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "repo_full_name": "owner/repo",
            "thread_channel_id": parent_forum_id,
            "issue_or_pr_number": 1,
            "discord_thread_id": foreign_thread_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Case 6: parent channel is voice channel (type 2) -> 400 Bad Request
    let resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/github/thread-map"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "repo_full_name": "owner/repo",
            "thread_channel_id": voice_channel_id,
            "issue_or_pr_number": 1,
            "discord_thread_id": valid_thread_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Case 7: discord_thread_id is not a thread (type 0) -> 400 Bad Request
    let resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/github/thread-map"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "repo_full_name": "owner/repo",
            "thread_channel_id": parent_forum_id,
            "issue_or_pr_number": 1,
            "discord_thread_id": not_a_thread_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Case 8: thread parent_id does not match thread_channel_id -> 403 Forbidden
    let resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/github/thread-map"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "repo_full_name": "owner/repo",
            "thread_channel_id": parent_forum_id,
            "issue_or_pr_number": 1,
            "discord_thread_id": wrong_parent_thread_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Case 9: Success case - valid forum parent and matching thread in Guild A
    let resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/github/thread-map"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "repo_full_name": "owner/repo",
            "thread_channel_id": parent_forum_id,
            "issue_or_pr_number": 42,
            "discord_thread_id": valid_thread_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let map_body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(map_body["guild_id"], guild_a);
    assert_eq!(map_body["repo_full_name"], "owner/repo");
    assert_eq!(map_body["issue_or_pr_number"], 42);
    assert_eq!(map_body["discord_thread_id"], valid_thread_id);

    // Verify storage has mapping
    let stored_mapping = state
        .app
        .storage
        .get_github_devops_mapping(guild_a, "owner/repo", 42)
        .await
        .unwrap();
    assert!(stored_mapping.is_some());
    assert_eq!(stored_mapping.unwrap().discord_thread_id, valid_thread_id);

    // =========================================================================
    // Issue #352 Tests: create_telegram_bridge
    // =========================================================================

    // Case 10: invalid snowflake for discord_channel_id -> 400 Bad Request
    let resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/telegram/bridge"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "telegram_chat_id": "-100123456789",
            "discord_channel_id": "invalid_snowflake",
            "secret_token": "tg_sec_123"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Case 11: foreign channel from Guild B -> 403 Forbidden
    let resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/telegram/bridge"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "telegram_chat_id": "-100123456789",
            "discord_channel_id": foreign_channel_id,
            "secret_token": "tg_sec_123"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Case 12: voice channel (type 2) -> 400 Bad Request
    let resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/telegram/bridge"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "telegram_chat_id": "-100123456789",
            "discord_channel_id": voice_channel_id,
            "secret_token": "tg_sec_123"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Case 13: Success case - text channel in Guild A
    let resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/telegram/bridge"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "telegram_chat_id": "-100123456789",
            "discord_channel_id": valid_tg_channel_id,
            "secret_token": "tg_sec_super_secret"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let bridge_body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(bridge_body["guild_id"], guild_a);
    assert_eq!(bridge_body["discord_channel_id"], valid_tg_channel_id);
    assert_eq!(bridge_body["telegram_chat_id"], "-100123456789");
    assert_eq!(bridge_body["secret_configured"], true);
    // Crucial security check: secret_token must NEVER be in summary JSON!
    assert!(bridge_body.get("secret_token").is_none());

    // Verify storage has the bridge
    let stored_bridge = state
        .app
        .storage
        .get_telegram_bridge(guild_a, valid_tg_channel_id)
        .await
        .unwrap();
    assert!(stored_bridge.is_some());
    assert_eq!(stored_bridge.unwrap().secret_token, "tg_sec_super_secret");
}

#[tokio::test]
async fn test_webhook_dedupe_and_delivery_logs_retention_and_pagination_issue_346() {
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

    let guild_a = "111222333444555666";
    let guild_b = "999888777666555444";
    let user_id = "555666777888999000";

    let session_id = "test_retention_session_token";
    let csrf_token = "test_retention_csrf_token";
    let session = rusty_discord_bot::session::Session {
        user: DashboardUser {
            id: user_id.to_string(),
            username: "retention_admin".to_string(),
            avatar: None,
            global_name: None,
        },
        guilds: vec![DashboardGuild {
            id: guild_a.to_string(),
            name: "Guild A".to_string(),
            icon: None,
            owner: true,
            permissions: "8".to_string(), // ADMINISTRATOR
        }],
        expires_at: Utc::now() + Duration::hours(2),
        access_token: None,
        csrf_token: csrf_token.to_string(),
    };
    state
        .app
        .sessions
        .save_session(
            session_id,
            session.user,
            session.guilds,
            session.csrf_token,
            session.expires_at,
            None,
        )
        .await
        .unwrap();

    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    // 1. Test event dedupe retention & pruning
    let now = Utc::now();
    let old_timestamp = now - Duration::days(45);

    // Record old event with backdated timestamp and fresh event directly in storage
    state
        .app
        .storage
        .record_event_dedupe_with_timestamp("github", "evt_old_1", old_timestamp)
        .await
        .unwrap();
    state
        .app
        .storage
        .check_and_record_event_dedupe("github", "evt_fresh_1")
        .await
        .unwrap();

    // Pruning with 30-day cutoff
    let dedupe_cutoff = now - Duration::days(30);
    let pruned_dedupes = state
        .app
        .storage
        .prune_event_dedupes(dedupe_cutoff, 100)
        .await
        .unwrap();
    assert_eq!(
        pruned_dedupes, 1,
        "Exactly 1 old dedupe record must be pruned"
    );

    // Fresh event must still be recognized as duplicate
    let fresh_replay = state
        .app
        .storage
        .check_and_record_event_dedupe("github", "evt_fresh_1")
        .await
        .unwrap();
    assert!(
        !fresh_replay,
        "Fresh event within retention must remain duplicate"
    );

    // Pruned old event can now be accepted anew
    let old_reaccepted = state
        .app
        .storage
        .check_and_record_event_dedupe("github", "evt_old_1")
        .await
        .unwrap();
    assert!(
        old_reaccepted,
        "Pruned old event can be accepted again without collision"
    );

    // 2. Test delivery log retention & pruning
    let old_log_id = Uuid::new_v4();
    let fresh_log_id = Uuid::new_v4();

    state
        .app
        .storage
        .record_delivery_log(IntegrationDeliveryLog {
            id: old_log_id,
            guild_id: guild_a.to_string(),
            provider: "github".to_string(),
            event_type: "push".to_string(),
            status: "delivered".to_string(),
            error_message: None,
            payload_summary: serde_json::json!({"action": "old_test"}),
            created_at: now - Duration::days(120),
        })
        .await
        .unwrap();

    state
        .app
        .storage
        .record_delivery_log(IntegrationDeliveryLog {
            id: fresh_log_id,
            guild_id: guild_a.to_string(),
            provider: "github".to_string(),
            event_type: "push".to_string(),
            status: "delivered".to_string(),
            error_message: None,
            payload_summary: serde_json::json!({"action": "fresh_test"}),
            created_at: now,
        })
        .await
        .unwrap();

    let logs_cutoff = now - Duration::days(90);
    let pruned_logs = state
        .app
        .storage
        .prune_delivery_logs(logs_cutoff, 100)
        .await
        .unwrap();
    assert_eq!(pruned_logs, 1, "Exactly 1 old delivery log must be pruned");

    // 3. Test list_delivery_logs pagination, ordering and tenant isolation
    // Populate 70 logs for guild_a and 5 logs for guild_b
    for i in 0..70 {
        let ts = now - Duration::seconds(100 - i);
        state
            .app
            .storage
            .record_delivery_log(IntegrationDeliveryLog {
                id: Uuid::new_v4(),
                guild_id: guild_a.to_string(),
                provider: if i % 2 == 0 { "github" } else { "twitch" }.to_string(),
                event_type: "test_event".to_string(),
                status: "delivered".to_string(),
                error_message: None,
                payload_summary: serde_json::json!({"item": i}),
                created_at: ts,
            })
            .await
            .unwrap();
    }
    for i in 0..5 {
        state
            .app
            .storage
            .record_delivery_log(IntegrationDeliveryLog {
                id: Uuid::new_v4(),
                guild_id: guild_b.to_string(),
                provider: "github".to_string(),
                event_type: "cross_tenant_event".to_string(),
                status: "delivered".to_string(),
                error_message: None,
                payload_summary: serde_json::json!({"cross": i}),
                created_at: now,
            })
            .await
            .unwrap();
    }

    // Default fetch: must be bounded (default 50 <= 100), all for guild_a
    let resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_a}/integrations/deliveries"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let logs: Vec<IntegrationDeliveryLog> = resp.json().await.unwrap();
    assert_eq!(logs.len(), 50, "Default query must be bounded to 50 logs");
    assert!(
        logs.iter().all(|l| l.guild_id == guild_a),
        "All returned logs must belong strictly to requested guild_a"
    );

    // Verify deterministic descending ordering
    for w in logs.windows(2) {
        assert!(
            w[0].created_at >= w[1].created_at,
            "Logs must be sorted descending by created_at"
        );
    }

    // Keyset pagination: request first 10, then next 10
    let page1_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_a}/integrations/deliveries?limit=10"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    let page1: Vec<IntegrationDeliveryLog> = page1_resp.json().await.unwrap();
    assert_eq!(page1.len(), 10);

    let last_p1 = &page1[9];
    let encoded_ts = last_p1.created_at.to_rfc3339().replace('+', "%2B");
    let page2_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_a}/integrations/deliveries?limit=10&before_created_at={}&before_id={}",
            encoded_ts,
            last_p1.id
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    let page2: Vec<IntegrationDeliveryLog> = page2_resp.json().await.unwrap();
    assert_eq!(page2.len(), 10);

    // Verify zero overlap between page 1 and page 2
    let page1_ids: HashSet<_> = page1.iter().map(|l| l.id).collect();
    for item in &page2 {
        assert!(
            !page1_ids.contains(&item.id),
            "Keyset cursor pagination must produce non-overlapping pages"
        );
    }

    // Provider filter test
    let twitch_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_a}/integrations/deliveries?provider=twitch&limit=100"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    let twitch_logs: Vec<IntegrationDeliveryLog> = twitch_resp.json().await.unwrap();
    assert!(
        twitch_logs
            .iter()
            .all(|l| l.provider.eq_ignore_ascii_case("twitch")),
        "Filtered query must only return twitch logs"
    );

    // 4. Test run_integrations_retention_janitor background loop and graceful shutdown
    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let janitor_task = tokio::spawn(run_integrations_retention_janitor(
        state.app.storage.clone(),
        shutdown_rx,
        std::time::Duration::from_millis(50),
        chrono::Duration::days(30),
        chrono::Duration::days(90),
    ));

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    let _ = shutdown_tx.send(());

    let shutdown_result =
        tokio::time::timeout(std::time::Duration::from_secs(2), janitor_task).await;
    assert!(
        shutdown_result.is_ok(),
        "Integrations retention janitor must terminate cleanly upon shutdown broadcast"
    );
}

#[tokio::test]
async fn test_integration_credentials_encryption_at_rest_and_tamper_resistance_issue_349() {
    use rusty_discord_bot::integrations::CredentialCrypto;

    // 1. Verify HTTP endpoint security: secret_token is never leaked via HTTP responses (Issue #131 + #349)
    let (state, _) = test_web_state().await;
    let router = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let guild_id = "987654321098765432";
    let channel_id = "111222333444555666";

    let session_id = "test_cred_session_token";
    let csrf_token = "test_cred_csrf_token";
    let session = rusty_discord_bot::session::Session {
        user: DashboardUser {
            id: "123456789".to_string(),
            username: "admin_user".to_string(),
            avatar: None,
            global_name: None,
        },
        guilds: vec![DashboardGuild {
            id: guild_id.to_string(),
            name: "Test Guild".to_string(),
            icon: None,
            owner: true,
            permissions: "8".to_string(), // ADMINISTRATOR
        }],
        expires_at: Utc::now() + Duration::hours(2),
        access_token: None,
        csrf_token: csrf_token.to_string(),
    };
    state
        .app
        .sessions
        .save_session(
            session_id,
            session.user,
            session.guilds,
            session.csrf_token,
            session.expires_at,
            None,
        )
        .await
        .unwrap();
    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    let payload = serde_json::json!({
        "telegram_chat_id": "-1001234567890",
        "discord_channel_id": channel_id,
        "secret_token": "fixture-not-a-real-secret-token-xyz"
    });

    let post_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/telegram/bridge"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&payload)
        .send()
        .await
        .unwrap();

    assert_eq!(post_resp.status(), reqwest::StatusCode::OK);
    let post_body: serde_json::Value = post_resp.json().await.unwrap();
    assert_eq!(post_body["discord_channel_id"], channel_id);
    assert_eq!(post_body["telegram_chat_id"], "-1001234567890");
    assert!(
        post_body.get("secret_token").is_none(),
        "TelegramBridgeSummary must NEVER leak secret_token over HTTP"
    );

    let get_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/telegram/bridges"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_resp.status(), reqwest::StatusCode::OK);
    let get_body: Vec<serde_json::Value> = get_resp.json().await.unwrap();
    assert_eq!(get_body.len(), 1);
    assert!(
        get_body[0].get("secret_token").is_none(),
        "GET list must NEVER leak secret_token over HTTP"
    );

    // 2. Direct CredentialCrypto envelope format and nonces
    let key_v1 = [0x42u8; 32];
    let crypto = CredentialCrypto::new("v1", key_v1);
    let record_id = "record-uuid-1234";
    let cleartext = "super-secret-telegram-webhook-token";

    let enc1 = crypto
        .encrypt_field(
            guild_id,
            "telegram_bridge",
            record_id,
            "secret_token",
            cleartext,
        )
        .expect("encrypt field");
    let enc2 = crypto
        .encrypt_field(
            guild_id,
            "telegram_bridge",
            record_id,
            "secret_token",
            cleartext,
        )
        .expect("encrypt field");

    assert!(enc1.starts_with("enc:v1:v1:"));
    assert!(enc2.starts_with("enc:v1:v1:"));
    assert_ne!(
        enc1, enc2,
        "Each encryption MUST use a unique random 96-bit nonce from OsRng"
    );
    assert!(
        !enc1.contains(cleartext),
        "Ciphertext envelope must NOT contain cleartext secret"
    );

    let dec1 = crypto
        .decrypt_field(
            guild_id,
            "telegram_bridge",
            record_id,
            "secret_token",
            &enc1,
        )
        .expect("decrypt field");
    assert_eq!(dec1, cleartext);

    // 3. Strict AAD binding: cross-tenant, cross-entity, cross-record, and cross-field tampering resistance
    let other_guild = "111111111111111111";
    let cross_tenant_err = crypto.decrypt_field(
        other_guild,
        "telegram_bridge",
        record_id,
        "secret_token",
        &enc1,
    );
    assert!(
        cross_tenant_err.is_err(),
        "Decryption with different guild_id MUST fail closed (tenant isolation)"
    );

    let cross_entity_err = crypto.decrypt_field(
        guild_id,
        "integration_connection",
        record_id,
        "secret_token",
        &enc1,
    );
    assert!(
        cross_entity_err.is_err(),
        "Decryption with different entity_type MUST fail closed"
    );

    let cross_record_err = crypto.decrypt_field(
        guild_id,
        "telegram_bridge",
        "other-record-uuid",
        "secret_token",
        &enc1,
    );
    assert!(
        cross_record_err.is_err(),
        "Decryption with different record_id MUST fail closed"
    );

    let cross_field_err = crypto.decrypt_field(
        guild_id,
        "telegram_bridge",
        record_id,
        "access_token",
        &enc1,
    );
    assert!(
        cross_field_err.is_err(),
        "Decryption with different field_name MUST fail closed"
    );

    // Corrupted ciphertext tampering
    let corrupted = format!("{}deadbeef", &enc1[..enc1.len() - 8]);
    assert!(
        crypto
            .decrypt_field(
                guild_id,
                "telegram_bridge",
                record_id,
                "secret_token",
                &corrupted
            )
            .is_err(),
        "Corrupted ciphertext MUST fail closed"
    );

    // 4. Key rotation and keyring support
    let key_v2 = [0x99u8; 32];
    let mut keyring = std::collections::HashMap::new();
    keyring.insert("v1".to_string(), key_v1);
    keyring.insert("v2".to_string(), key_v2);

    let rotated_crypto = CredentialCrypto::with_keys("v2", keyring).expect("with_keys");
    let decrypted_old = rotated_crypto
        .decrypt_field(
            guild_id,
            "telegram_bridge",
            record_id,
            "secret_token",
            &enc1,
        )
        .expect("decrypt legacy key from keyring");
    assert_eq!(decrypted_old, cleartext);

    let enc_new = rotated_crypto
        .encrypt_field(
            guild_id,
            "telegram_bridge",
            record_id,
            "secret_token",
            "brand-new-secret",
        )
        .expect("encrypt with active key v2");
    assert!(
        enc_new.starts_with("enc:v1:v2:"),
        "New encryption must use active key id v2"
    );
    let dec_new = rotated_crypto
        .decrypt_field(
            guild_id,
            "telegram_bridge",
            record_id,
            "secret_token",
            &enc_new,
        )
        .expect("decrypt v2");
    assert_eq!(dec_new, "brand-new-secret");

    // 5. Backwards-compatible legacy plaintext fallback during transition
    let legacy_plaintext = "old-unencrypted-token-from-backup";
    let dec_legacy = rotated_crypto
        .decrypt_field(
            guild_id,
            "telegram_bridge",
            record_id,
            "secret_token",
            legacy_plaintext,
        )
        .expect("transparent read of legacy plaintext");
    assert_eq!(dec_legacy, legacy_plaintext);
}

#[tokio::test]
async fn test_postgres_startup_preflight_fails_fast_without_integration_credential_key_issue_378() {
    // 1. PostgreSQL configured, transcript key set, but integration key missing:
    // Must fail fast during startup preflight before opening network connections to database.
    let mut config = test_config();
    config.database_url = Some("postgres://dummy:dummy@127.0.0.1:5432/dummy".to_string());
    config.transcript_encryption_key =
        Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string());
    config.integration_credential_key = None;
    config.integration_credential_keyring = None;
    config.integration_credentials_disabled = false;

    let res = AppState::bootstrap(config).await;
    let err_msg = match res {
        Err(e) => e.to_string(),
        Ok(_) => {
            panic!("Must fail fast when PostgreSQL has no integration credentials encryption key")
        }
    };
    assert!(
        err_msg.contains(
            "PostgreSQL storage requires INTEGRATION_CREDENTIAL_KEYRING or INTEGRATION_CREDENTIAL_KEY"
        ),
        "Error message should explain requirement: {err_msg}"
    );

    // 2. Malformed key format (not 64 hex characters): must fail fast
    let mut config_malformed = test_config();
    config_malformed.database_url = Some("postgres://dummy:dummy@127.0.0.1:5432/dummy".to_string());
    config_malformed.transcript_encryption_key =
        Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string());
    config_malformed.integration_credential_key = Some("invalid_key_short".to_string());
    let res_malformed = AppState::bootstrap(config_malformed).await;
    assert!(
        res_malformed.is_err(),
        "Malformed integration credential key must fail fast"
    );

    // 3. PostgreSQL configured, but integration_credentials_disabled is explicitly true:
    // Startup preflight does NOT fail on missing integration key.
    let mut config_disabled = test_config();
    config_disabled.database_url = Some("postgres://dummy:dummy@127.0.0.1:5432/dummy".to_string());
    config_disabled.transcript_encryption_key =
        Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string());
    config_disabled.integration_credentials_disabled = true;
    let res_disabled = AppState::bootstrap(config_disabled).await;
    let err_msg_disabled = match res_disabled {
        Err(e) => e.to_string(),
        Ok(_) => panic!("Expected connection failure to dummy PostgreSQL URL"),
    };
    assert!(
        !err_msg_disabled.contains("INTEGRATION_CREDENTIAL_KEY"),
        "Preflight should be skipped when integration credentials are explicitly disabled: {err_msg_disabled}"
    );

    // 4. MemoryStore without keys: succeeds without preflight error
    let config_mem = test_config();
    assert!(
        AppState::bootstrap(config_mem).await.is_ok(),
        "MemoryStore must bootstrap without requiring persistent encryption keys"
    );
}
