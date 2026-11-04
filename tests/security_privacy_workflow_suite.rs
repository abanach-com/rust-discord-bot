use std::collections::HashSet;
use std::sync::Arc;

use chrono::{Duration, Utc};
use dashmap::DashMap;
use reqwest::header::COOKIE;
use serde_json::json;
use uuid::Uuid;

use rusty_discord_bot::{
    config::Config,
    moderation_cases::{CaseKind, CaseSeverity, CreateCase},
    scheduler::{
        JobMutation, JobMutationOutcome, NewScheduledJob, STATUS_CANCELLED, STATUS_PROCESSING,
    },
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

    let guild_id = "100100100100";
    let user_id = "200200200200";
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
    // Issue #132 & #135: Progression & Moderation Privacy Data Handling
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

    // Issue #135: Seed 51 moderation cases for subject user
    let parsed_g = guild_id.parse::<u64>().unwrap();
    let parsed_u = user_id.parse::<u64>().unwrap();
    for i in 1..=51 {
        state
            .app
            .moderation_cases
            .create_case(
                CreateCase {
                    guild_id: parsed_g,
                    subject_user_id: parsed_u,
                    actor_user_id: 999999999,
                    kind: CaseKind::Warning,
                    severity: CaseSeverity::Low,
                    points: 1,
                    reason: format!("Warning number {i}"),
                    source_module: "moderation".to_string(),
                    visible_to_subject: true,
                    expires_at: None,
                },
                Utc::now(),
            )
            .await
            .expect("create subject case");
    }

    // Seed case for different subject user to verify isolation
    state
        .app
        .moderation_cases
        .create_case(
            CreateCase {
                guild_id: parsed_g,
                subject_user_id: 888888888,
                actor_user_id: 999999999,
                kind: CaseKind::Warning,
                severity: CaseSeverity::Low,
                points: 1,
                reason: "Other user warning".to_string(),
                source_module: "moderation".to_string(),
                visible_to_subject: true,
                expires_at: None,
            },
            Utc::now(),
        )
        .await
        .expect("create other user case");

    // Verify export returns all 51 cases without truncation
    let export_initial = client
        .post(format!("{base_url}/api/guilds/{guild_id}/privacy/export"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&json!({
            "user_id": user_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(export_initial.status(), 200);
    let initial_json: serde_json::Value = export_initial.json().await.unwrap();
    assert!(initial_json["progression"].is_object());
    let cases = initial_json["moderation_cases"].as_array().unwrap();
    assert_eq!(
        cases.len(),
        51,
        "Issue #135: Export must return all 51 cases without truncation to 50"
    );
    for c in cases {
        assert_eq!(c["subject_user_id"], user_id);
    }

    // Issue #135: Storage failure injection must yield HTTP 500, not fake 200
    state.app.storage.inject_progression_failure(true);
    let prog_fail_res = client
        .post(format!("{base_url}/api/guilds/{guild_id}/privacy/export"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&json!({
            "user_id": user_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        prog_fail_res.status(),
        500,
        "Issue #135: Storage progression failure must return HTTP 500"
    );
    state.app.storage.inject_progression_failure(false);

    // Issue #135: Moderation case store failure injection must yield HTTP 500, not fake 200
    state.app.moderation_cases.inject_failure(true);
    let mod_fail_res = client
        .post(format!("{base_url}/api/guilds/{guild_id}/privacy/export"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&json!({
            "user_id": user_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        mod_fail_res.status(),
        500,
        "Issue #135: Moderation store failure must return HTTP 500"
    );
    state.app.moderation_cases.inject_failure(false);

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

    // Verify export returns null for progression after genuine erasure
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
        "Exported progression must be null after genuine erasure"
    );

    // =========================================================================
    // Issue #134: ICS Timezone CRLF Injection & Timezone Validation
    // =========================================================================

    // 4a. Reject invalid timezone containing CRLF
    let invalid_event = json!({
        "title": "Malicious Event",
        "description": "Event with CRLF in timezone",
        "start_time": "2026-12-01T19:00:00Z",
        "end_time": "2026-12-01T20:00:00Z",
        "timezone": "UTC\r\nSUMMARY:Injected Summary",
        "max_participants": 10
    });
    let invalid_res = client
        .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&invalid_event)
        .send()
        .await
        .unwrap();
    assert_eq!(
        invalid_res.status(),
        400,
        "Issue #134: Timezone with CRLF must be rejected with HTTP 400"
    );

    // 4b. Create valid event with Europe/Warsaw
    let valid_event = json!({
        "title": "Community Workshop",
        "description": "Learning Rust",
        "start_time": "2026-12-01T19:00:00Z",
        "end_time": "2026-12-01T21:00:00Z",
        "timezone": "Europe/Warsaw",
        "max_participants": 20
    });
    let valid_res = client
        .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&valid_event)
        .send()
        .await
        .unwrap();
    assert_eq!(valid_res.status(), 200);
    let ev_created: serde_json::Value = valid_res.json().await.unwrap();
    let event_id = ev_created["id"].as_str().unwrap();

    // 4c. Fetch ICS and verify compliance with RFC 5545
    let ics_res = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{event_id}/ics"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(ics_res.status(), 200);
    assert_eq!(
        ics_res.headers().get("content-type").unwrap(),
        "text/calendar; charset=utf-8"
    );
    let ics_text = ics_res.text().await.unwrap();
    assert!(
        !ics_text.contains("TZID:"),
        "Standalone TZID property must not be emitted in UTC event"
    );
    assert!(ics_text.contains("DTSTART:20261201T190000Z"));
    assert!(ics_text.contains("DTEND:20261201T210000Z"));
    assert_eq!(ics_text.matches("SUMMARY:").count(), 1);
    assert_eq!(ics_text.matches("BEGIN:VEVENT").count(), 1);
    assert_eq!(ics_text.matches("END:VEVENT").count(), 1);

    // 4d. Fetch ICS from another guild -> 403 Forbidden
    let cross_res = client
        .get(format!(
            "{base_url}/api/guilds/other_guild_99999/scheduled-events/{event_id}/ics"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(cross_res.status(), 403);

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

    // =========================================================================
    // Issue #136 & #137: Event Roster Capacity & CSRF Protection
    // =========================================================================

    // Create an event with max_participants = 2
    let roster_event = json!({
        "title": "2-Player Tournament",
        "description": "Strict 2-player capacity",
        "start_time": "2026-12-01T19:00:00Z",
        "end_time": "2026-12-01T21:00:00Z",
        "timezone": "UTC",
        "max_participants": 2
    });
    let roster_ev_res = client
        .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&roster_event)
        .send()
        .await
        .unwrap();
    assert_eq!(roster_ev_res.status(), 200);
    let roster_ev_json: serde_json::Value = roster_ev_res.json().await.unwrap();
    let roster_event_id = roster_ev_json["id"].as_str().unwrap();

    // 137a. Join without CSRF token -> 403 Forbidden
    let no_csrf_res = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{roster_event_id}/roster/join"
        ))
        .header(COOKIE, &session_cookie)
        .json(&json!({ "max_main": 100 }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        no_csrf_res.status(),
        403,
        "Issue #137: join_event_roster without CSRF token must return 403"
    );

    // 137b. Join with invalid CSRF token -> 403 Forbidden
    let bad_csrf_res = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{roster_event_id}/roster/join"
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", "invalid_token_999")
        .json(&json!({ "max_main": 100 }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        bad_csrf_res.status(),
        403,
        "Issue #137: join_event_roster with bad CSRF token must return 403"
    );

    // Setup User 2 and User 3 sessions with permissions on guild_id
    let user_2 = "300300300300";
    let session_2 = "test_sec_session_token_2";
    let csrf_2 = "test_sec_csrf_token_2";
    state
        .app
        .sessions
        .save_session(
            session_2,
            DashboardUser {
                id: user_2.to_string(),
                username: "UserTwo".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Sec Guild".to_string(),
                icon: None,
                owner: false,
                permissions: "8".to_string(),
            }],
            csrf_2.to_string(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .unwrap();
    perms_cache.insert(
        (user_2.to_string(), guild_id.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );
    let cookie_2 = format!("rusty_discord_bot_session={session_2}");

    let user_3 = "400400400400";
    let session_3 = "test_sec_session_token_3";
    let csrf_3 = "test_sec_csrf_token_3";
    state
        .app
        .sessions
        .save_session(
            session_3,
            DashboardUser {
                id: user_3.to_string(),
                username: "UserThree".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Sec Guild".to_string(),
                icon: None,
                owner: false,
                permissions: "8".to_string(),
            }],
            csrf_3.to_string(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .unwrap();
    perms_cache.insert(
        (user_3.to_string(), guild_id.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );
    let cookie_3 = format!("rusty_discord_bot_session={session_3}");

    // 136a. User 1 joins with max_main: 100 (attempting to override capacity) -> gets "main" (1/2)
    let join_1 = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{roster_event_id}/roster/join"
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&json!({ "max_main": 100 }))
        .send()
        .await
        .unwrap();
    assert_eq!(join_1.status(), 200);
    let join_1_json: serde_json::Value = join_1.json().await.unwrap();
    assert_eq!(join_1_json["role_type"], "main");

    // 136b. User 2 joins -> gets "main" (2/2)
    let join_2 = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{roster_event_id}/roster/join"
        ))
        .header(COOKIE, &cookie_2)
        .header("X-CSRF-Token", csrf_2)
        .json(&json!({ "max_main": 100 }))
        .send()
        .await
        .unwrap();
    assert_eq!(join_2.status(), 200);
    let join_2_json: serde_json::Value = join_2.json().await.unwrap();
    assert_eq!(join_2_json["role_type"], "main");

    // 136c. User 3 joins with max_main: 50 -> must be put in "reserve" because event capacity is 2!
    let join_3 = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{roster_event_id}/roster/join"
        ))
        .header(COOKIE, &cookie_3)
        .header("X-CSRF-Token", csrf_3)
        .json(&json!({ "max_main": 50 }))
        .send()
        .await
        .unwrap();
    assert_eq!(join_3.status(), 200);
    let join_3_json: serde_json::Value = join_3.json().await.unwrap();
    assert_eq!(
        join_3_json["role_type"], "reserve",
        "Issue #136: User 3 must be assigned 'reserve' when event max_participants=2 is reached"
    );

    // 137c. Leave without CSRF -> 403 Forbidden
    let leave_no_csrf = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{roster_event_id}/roster/leave"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(
        leave_no_csrf.status(),
        403,
        "Issue #137: leave_event_roster without CSRF token must return 403"
    );

    // 136d & 137d. User 1 leaves with valid CSRF -> User 3 is promoted from reserve to main!
    let leave_res = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{roster_event_id}/roster/leave"
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(leave_res.status(), 200);
    let leave_json: serde_json::Value = leave_res.json().await.unwrap();
    assert_eq!(leave_json["left"], true);
    assert_eq!(leave_json["promoted_reserve_user_id"], user_3);

    // =========================================================================
    // Issue #138: Scheduler Processing Cancellation & Reliability
    // =========================================================================

    // 138a. Cancel an active job -> cancels immediately
    let active_job = state
        .app
        .scheduler
        .create_job(
            NewScheduledJob {
                guild_id: guild_id.to_string(),
                module_id: "scheduler".to_string(),
                channel_id: "123456789".to_string(),
                creator_user_id: user_id.to_string(),
                content: "Reminder 1".to_string(),
                mention_creator: false,
                run_at: Utc::now() + Duration::hours(5),
                repeat_every_seconds: None,
                remaining_runs: None,
                max_attempts: 3,
            },
            100,
        )
        .await
        .expect("create active job");

    let active_job_id = match active_job {
        rusty_discord_bot::scheduler::CreateJobOutcome::Created(j) => j.id,
        _ => panic!("Expected job created"),
    };

    let cancel_active = state
        .app
        .scheduler
        .mutate_job(
            guild_id,
            &active_job_id,
            user_id,
            false,
            JobMutation::Cancel,
        )
        .await
        .expect("mutate active job");
    match cancel_active {
        JobMutationOutcome::Updated(j) => {
            assert_eq!(j.status, STATUS_CANCELLED);
            assert!(j.completed_at.is_some());
        }
        _ => panic!("Expected updated outcome for active job cancel"),
    }

    // 138b. Cancel a processing job -> sets cancellation_requested_at, keeps lease and processing status
    let processing_job = state
        .app
        .scheduler
        .create_job(
            NewScheduledJob {
                guild_id: guild_id.to_string(),
                module_id: "scheduler".to_string(),
                channel_id: "123456789".to_string(),
                creator_user_id: user_id.to_string(),
                content: "Reminder 2".to_string(),
                mention_creator: false,
                run_at: Utc::now() - Duration::minutes(1), // due now
                repeat_every_seconds: None,
                remaining_runs: None,
                max_attempts: 3,
            },
            100,
        )
        .await
        .expect("create processing job");

    let proc_job_id = match processing_job {
        rusty_discord_bot::scheduler::CreateJobOutcome::Created(j) => j.id,
        _ => panic!("Expected job created"),
    };

    let worker_id = "test_worker_xyz";
    let claimed = state
        .app
        .scheduler
        .claim_due(worker_id, Utc::now(), Utc::now() - Duration::hours(1), 10)
        .await
        .expect("claim due job");
    assert!(claimed.iter().any(|j| j.id == proc_job_id));

    // Cancel while processing
    let cancel_proc = state
        .app
        .scheduler
        .mutate_job(guild_id, &proc_job_id, user_id, false, JobMutation::Cancel)
        .await
        .expect("mutate processing job");

    match cancel_proc {
        JobMutationOutcome::Updated(j) => {
            assert_eq!(
                j.status, STATUS_PROCESSING,
                "Issue #138: Processing job must not be immediately transitioned to terminal cancelled"
            );
            assert!(
                j.cancellation_requested_at.is_some(),
                "Issue #138: cancellation_requested_at must be populated"
            );
            assert_eq!(j.locked_by.as_deref(), Some(worker_id));
        }
        _ => panic!("Expected updated outcome for processing job cancel"),
    }

    // Check cancellation_requested for current worker vs another worker
    assert!(
        state
            .app
            .scheduler
            .cancellation_requested(&proc_job_id, worker_id)
            .await
            .expect("check cancellation requested"),
        "cancellation_requested must return true for locking worker"
    );
    assert!(
        !state
            .app
            .scheduler
            .cancellation_requested(&proc_job_id, "other_worker")
            .await
            .expect("check cancellation requested for other worker"),
        "cancellation_requested must return false for other workers"
    );

    // Worker finalizes cancellation
    let mark_res = state
        .app
        .scheduler
        .mark_cancelled(&proc_job_id, worker_id, Utc::now())
        .await
        .expect("mark cancelled");
    assert!(mark_res.is_some());
    let fin_job = mark_res.unwrap();
    assert_eq!(fin_job.status, STATUS_CANCELLED);
    assert!(fin_job.locked_by.is_none());
    assert!(fin_job.completed_at.is_some());

    // Another worker cannot reclaim the cancelled job
    let reclaim = state
        .app
        .scheduler
        .claim_due(
            "another_worker",
            Utc::now() + Duration::hours(1),
            Utc::now() - Duration::hours(1),
            10,
        )
        .await
        .expect("reclaim attempt");
    assert!(!reclaim.iter().any(|j| j.id == proc_job_id));
}
