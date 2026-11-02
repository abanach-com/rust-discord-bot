use chrono::{Duration, Utc};
use rusty_discord_bot::moderation_cases::{
    CaseFilter, CaseKind, CaseSeverity, CaseStatus, CreateCase, ModerationCaseStore, UpdateCase,
};

fn warning(now: chrono::DateTime<Utc>) -> CreateCase {
    CreateCase {
        guild_id: 10,
        subject_user_id: 20,
        actor_user_id: 30,
        kind: CaseKind::Warning,
        severity: CaseSeverity::Medium,
        points: 3,
        reason: "Repeated harassment after a moderator request".to_owned(),
        source_module: "moderation".to_owned(),
        visible_to_subject: true,
        expires_at: Some(now + Duration::days(30)),
    }
}

#[tokio::test]
async fn complete_case_lifecycle_is_versioned_and_audited() {
    let store = ModerationCaseStore::memory();
    let now = Utc::now();
    let created = store.create_case(warning(now), now).await.unwrap();
    assert_eq!(created.case_number, 1);
    assert_eq!(created.version, 1);

    let note = store
        .add_note(
            10,
            created.case_number,
            30,
            "User acknowledged the warning.".to_owned(),
            true,
            now,
        )
        .await
        .unwrap();
    assert!(note.visible_to_subject);

    store
        .add_evidence(
            10,
            created.case_number,
            30,
            "message".to_owned(),
            "https://discord.com/channels/10/40/50".to_owned(),
            now,
        )
        .await
        .unwrap();

    let updated = store
        .update_case(
            10,
            created.case_number,
            30,
            1,
            UpdateCase {
                severity: Some(CaseSeverity::High),
                points: Some(5),
                ..UpdateCase::default()
            },
            now,
        )
        .await
        .unwrap();
    assert_eq!(updated.version, 2);

    let voided = store
        .void_case(
            10,
            created.case_number,
            30,
            2,
            "Appeal accepted after reviewing context.".to_owned(),
            now,
        )
        .await
        .unwrap();
    assert_eq!(voided.status, CaseStatus::Voided);

    let restored = store
        .restore_case(10, created.case_number, 30, 3, now)
        .await
        .unwrap();
    assert_eq!(restored.status, CaseStatus::Active);
    assert_eq!(restored.version, 4);

    let details = store.get_case(10, created.case_number, now).await.unwrap();
    assert_eq!(details.notes.len(), 1);
    assert_eq!(details.evidence.len(), 1);
    assert!(details.events.len() >= 6);
}

#[tokio::test]
async fn self_service_filter_never_exposes_staff_only_records() {
    let store = ModerationCaseStore::memory();
    let now = Utc::now();
    store.create_case(warning(now), now).await.unwrap();
    let mut private = warning(now);
    private.kind = CaseKind::StaffNote;
    private.visible_to_subject = false;
    private.reason = "Internal risk assessment".to_owned();
    store.create_case(private, now).await.unwrap();

    let rows = store
        .list_cases(
            10,
            CaseFilter {
                subject_user_id: Some(20),
                visible_to_subject_only: true,
                limit: 25,
                ..CaseFilter::default()
            },
            now,
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].visible_to_subject);
}

#[tokio::test]
async fn expired_cases_stop_contributing_to_active_stats() {
    let store = ModerationCaseStore::memory();
    let now = Utc::now();
    let created = store.create_case(warning(now), now).await.unwrap();
    let later = now + Duration::days(31);
    let stats = store.stats(10, None, later).await.unwrap();
    assert_eq!(stats.total_cases, 1);
    assert_eq!(stats.active_cases, 0);
    assert_eq!(stats.expired_cases, 1);
    let details = store
        .get_case(10, created.case_number, later)
        .await
        .unwrap();
    assert_eq!(details.case.status, CaseStatus::Expired);
}

#[tokio::test]
async fn lua_moderation_module_emits_valid_case_actions() {
    use rusty_discord_bot::lua::{
        LuaAction, LuaEngine, LuaExecutionContext, LuaLimits, ModerationCaseOperation,
    };
    use serde_json::json;
    use std::path::PathBuf;

    let engine = LuaEngine::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts"),
        LuaLimits {
            memory_bytes: 4 * 1024 * 1024,
            instruction_limit: 300_000,
            hook_granularity: 100,
        },
    )
    .expect("scripts should load");

    let ctx = LuaExecutionContext {
        guild_id: Some("100000000000000001".to_owned()),
        channel_id: "100000000000000002".to_owned(),
        user_id: "100000000000000003".to_owned(),
        user_name: "ModUser".to_owned(),
        member_roles: Vec::new(),
        member_permissions: "8".to_owned(), // Administrator
        locale: "pl".to_owned(),
        options: json!({
            "user": "100000000000000004",
            "reason": "Test warning reason",
            "severity": "high",
            "points": 3
        }),
        config: json!({
            "default_timeout_seconds": 600
        }),
    };

    let actions = engine
        .execute_command("moderation", "warn", ctx.clone())
        .await
        .unwrap();
    assert_eq!(actions.len(), 2);
    assert!(matches!(&actions[0], LuaAction::Reply { .. }));
    let case_action = actions
        .iter()
        .find_map(|a| match a {
            LuaAction::ModerationCase { operation } => Some(operation),
            _ => None,
        })
        .expect("expected ModerationCase action");

    assert!(case_action.validate().is_ok());
    match case_action {
        ModerationCaseOperation::Create {
            subject_user_id,
            kind,
            reason,
            severity,
            points,
            visible_to_subject,
            ..
        } => {
            assert_eq!(subject_user_id, "100000000000000004");
            assert_eq!(kind, "warning");
            assert_eq!(reason, "Test warning reason");
            assert_eq!(severity, "high");
            assert_eq!(*points, 3);
            assert!(visible_to_subject);
        }
        other => panic!("unexpected operation: {other:?}"),
    }

    // Test /case view
    let mut case_view_ctx = ctx.clone();
    case_view_ctx.options = json!({
        "action": "view",
        "case_number": 42
    });
    let view_actions = engine
        .execute_command("moderation", "case", case_view_ctx)
        .await
        .unwrap();
    let view_op = view_actions
        .iter()
        .find_map(|a| match a {
            LuaAction::ModerationCase { operation } => Some(operation),
            _ => None,
        })
        .expect("expected view operation");
    assert!(view_op.validate().is_ok());
    assert!(matches!(
        view_op,
        ModerationCaseOperation::View { case_number: 42 }
    ));

    // Test /case void
    let mut case_void_ctx = ctx.clone();
    case_void_ctx.options = json!({
        "action": "void",
        "case_number": 42,
        "text": "Mistaken identity",
        "version": 1
    });
    let void_actions = engine
        .execute_command("moderation", "case", case_void_ctx)
        .await
        .unwrap();
    let void_op = void_actions
        .iter()
        .find_map(|a| match a {
            LuaAction::ModerationCase { operation } => Some(operation),
            _ => None,
        })
        .expect("expected void operation");
    assert!(void_op.validate().is_ok());
    assert!(matches!(
        void_op,
        ModerationCaseOperation::Void {
            case_number: 42,
            expected_version: 1,
            ..
        }
    ));

    // Test /case edit
    let mut case_edit_ctx = ctx.clone();
    case_edit_ctx.options = json!({
        "action": "edit",
        "case_number": 42,
        "text": "Corrected case reason",
        "version": 2
    });
    let edit_actions = engine
        .execute_command("moderation", "case", case_edit_ctx)
        .await
        .unwrap();
    let edit_op = edit_actions
        .iter()
        .find_map(|a| match a {
            LuaAction::ModerationCase { operation } => Some(operation),
            _ => None,
        })
        .expect("expected edit operation");
    assert!(edit_op.validate().is_ok());
    assert!(matches!(
        edit_op,
        ModerationCaseOperation::Edit {
            case_number: 42,
            expected_version: 2,
            ..
        }
    ));

    // Test /moderate softban
    let mut softban_ctx = ctx.clone();
    softban_ctx.options = json!({
        "action": "softban",
        "user": "5550001",
        "reason": "Raid cleanup",
        "severity": "high",
        "points": 5
    });
    let softban_actions = engine
        .execute_command("moderation", "moderate", softban_ctx)
        .await
        .unwrap();
    let softban_op = softban_actions
        .iter()
        .find_map(|a| match a {
            LuaAction::ModerationCase { operation } => Some(operation),
            _ => None,
        })
        .expect("expected softban case operation");
    assert!(softban_op.validate().is_ok());
    assert!(matches!(
        softban_op,
        ModerationCaseOperation::Create {
            kind,
            reason,
            ..
        } if kind == "softban" && reason == "Raid cleanup"
    ));

    // Test /moderate unban
    let mut unban_ctx = ctx.clone();
    unban_ctx.options = json!({
        "action": "unban",
        "user": "5550001",
        "reason": "Appeal accepted"
    });
    let unban_actions = engine
        .execute_command("moderation", "moderate", unban_ctx)
        .await
        .unwrap();
    let unban_op = unban_actions
        .iter()
        .find_map(|a| match a {
            LuaAction::ModerationCase { operation } => Some(operation),
            _ => None,
        })
        .expect("expected unban case operation");
    assert!(unban_op.validate().is_ok());
    assert!(matches!(
        unban_op,
        ModerationCaseOperation::Create {
            kind,
            reason,
            ..
        } if kind == "unban" && reason == "Appeal accepted"
    ));
}

#[tokio::test]
async fn softban_case_lifecycle_and_kind_serialization() {
    let store = ModerationCaseStore::memory();
    let now = Utc::now();
    let created = store
        .create_case(
            CreateCase {
                guild_id: 10,
                subject_user_id: 20,
                actor_user_id: 30,
                kind: CaseKind::Softban,
                severity: CaseSeverity::High,
                points: 3,
                reason: "Spam raid wipe".to_owned(),
                source_module: "moderation".to_owned(),
                visible_to_subject: true,
                expires_at: None,
            },
            now,
        )
        .await
        .unwrap();
    assert_eq!(created.kind, CaseKind::Softban);
    assert_eq!(created.kind.as_str(), "softban");
    assert_eq!("softban".parse::<CaseKind>().unwrap(), CaseKind::Softban);

    let details = store.get_case(10, created.case_number, now).await.unwrap();
    assert_eq!(details.case.kind, CaseKind::Softban);

    let stats = store.stats(10, None, now).await.unwrap();
    assert_eq!(stats.total_cases, 1);
}

#[tokio::test]
async fn moderation_case_store_connect_and_audit_logging() {
    use rusty_discord_bot::storage::Storage;

    let store = ModerationCaseStore::connect(None).await.unwrap();
    let now = Utc::now();
    let created = store.create_case(warning(now), now).await.unwrap();
    assert_eq!(created.case_number, 1);

    let storage = Storage::connect(None).await.unwrap();
    storage
        .record_audit(
            Some("10"),
            Some("30"),
            "moderation",
            "case_created",
            serde_json::json!({ "case_number": 1 }),
        )
        .await
        .unwrap();

    let logs = storage.list_audit_events("10", 10).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].event, "case_created");
    assert_eq!(logs[0].actor_id.as_deref(), Some("30"));
}

#[tokio::test]
async fn softban_two_phase_intent_and_failure_resilience() {
    use rusty_discord_bot::storage::{ModerationExecutionIntent, Storage};
    use uuid::Uuid;

    let storage = Storage::memory();
    let mod_store = ModerationCaseStore::memory();
    let intent_id = Uuid::new_v4();
    let guild_id = "100000000000000001";
    let user_id = "200000000000000002";
    let now = Utc::now();

    // 1. Initial 2-phase intent created prior to Discord ban API call (#50)
    let intent = ModerationExecutionIntent {
        id: intent_id,
        interaction_id: None,
        guild_id: guild_id.to_string(),
        user_id: user_id.to_string(),
        kind: "softban".to_string(),
        step: "pending".to_string(),
        reason: Some("Test raid cleanup".to_string()),
        error: None,
        created_at: now,
        updated_at: now,
    };
    storage.record_moderation_intent(&intent).await.unwrap();

    // 2. Ban succeeds, intent progresses to "banned"
    storage
        .update_moderation_intent_step(intent_id, "banned", None)
        .await
        .unwrap();

    // Verify intent step transition
    let pending_banned = storage
        .list_pending_moderation_intents("softban", "banned")
        .await
        .unwrap();
    assert_eq!(pending_banned.len(), 1);
    assert_eq!(pending_banned[0].id, intent_id);

    // 3. Simulating Discord API unban failure:
    // Update intent to "unban_failed", record high-priority audit, and create actionable case
    let unban_err = "403 Forbidden: Missing Permissions to unban";
    storage
        .update_moderation_intent_step(intent_id, "unban_failed", Some(unban_err))
        .await
        .unwrap();

    storage
        .record_audit(
            Some(guild_id),
            Some("300000000000000003"),
            "moderation",
            "softban_unban_failed",
            serde_json::json!({
                "subject_user_id": user_id,
                "delete_days": 1,
                "error": unban_err,
                "alert": "Użytkownik został zbanowany, ale odbanowanie nie powiodło się; wymagana ręczna interwencja zespołu.",
            }),
        )
        .await
        .unwrap();

    let created_case = mod_store
        .create_case(
            CreateCase {
                guild_id: 100000000000000001,
                subject_user_id: 200000000000000002,
                actor_user_id: 300000000000000003,
                kind: CaseKind::Softban,
                severity: CaseSeverity::High,
                points: 3,
                reason: format!(
                    "Test raid cleanup [UWAGA: odbanowanie nie powiodło się: {unban_err}]"
                ),
                source_module: "moderation".to_string(),
                visible_to_subject: true,
                expires_at: None,
            },
            now,
        )
        .await
        .unwrap();

    // Verify case was created and is searchable, preventing an unrecorded permanent ban
    assert_eq!(created_case.kind, CaseKind::Softban);
    assert_eq!(created_case.severity, CaseSeverity::High);
    assert!(
        created_case
            .reason
            .contains("UWAGA: odbanowanie nie powiodło się")
    );

    let details = mod_store
        .get_case(100000000000000001, created_case.case_number, now)
        .await
        .unwrap();
    assert_eq!(details.case.case_number, created_case.case_number);

    // Verify audit log has the softban_unban_failed entry
    let audit_events = storage.list_audit_events(guild_id, 10).await.unwrap();
    assert_eq!(audit_events.len(), 1);
    assert_eq!(audit_events[0].event, "softban_unban_failed");

    // 4. Verify startup reconciliation discovery
    let stuck_intents = storage
        .list_pending_moderation_intents("softban", "unban_failed")
        .await
        .unwrap();
    assert_eq!(stuck_intents.len(), 1);
    assert_eq!(stuck_intents[0].id, intent_id);
    assert_eq!(stuck_intents[0].error.as_deref(), Some(unban_err));

    // When reconciliation succeeds, mark as completed
    storage
        .update_moderation_intent_step(intent_id, "completed", None)
        .await
        .unwrap();

    let remaining_stuck = storage
        .list_pending_moderation_intents("softban", "unban_failed")
        .await
        .unwrap();
    assert!(remaining_stuck.is_empty());
}

#[tokio::test]
async fn moderation_case_store_rejects_version_conflicts() {
    let store = ModerationCaseStore::memory();
    let now = Utc::now();
    let created = store.create_case(warning(now), now).await.unwrap();
    assert_eq!(created.version, 1);

    // Stale expected_version on update must fail with VersionConflict
    let update_err = store
        .update_case(
            10,
            created.case_number,
            30,
            99, // Stale/wrong version
            UpdateCase {
                points: Some(10),
                ..UpdateCase::default()
            },
            now,
        )
        .await
        .unwrap_err();

    match update_err {
        rusty_discord_bot::moderation_cases::CaseError::VersionConflict { expected, actual } => {
            assert_eq!(expected, 99);
            assert_eq!(actual, 1);
        }
        other => panic!("expected VersionConflict error, got {other:?}"),
    }

    // Stale expected_version on void must fail with VersionConflict
    let void_err = store
        .void_case(
            10,
            created.case_number,
            30,
            99, // Stale/wrong version
            "Wrong version attempt".to_string(),
            now,
        )
        .await
        .unwrap_err();

    match void_err {
        rusty_discord_bot::moderation_cases::CaseError::VersionConflict { expected, actual } => {
            assert_eq!(expected, 99);
            assert_eq!(actual, 1);
        }
        other => panic!("expected VersionConflict error, got {other:?}"),
    }
}

#[tokio::test]
async fn moderation_api_strictly_validates_expected_version_and_guards_concurrency() {
    use rusty_discord_bot::{
        AppState,
        config::Config,
        session::{DashboardGuild, DashboardUser},
    };
    use std::collections::HashSet;
    use std::net::SocketAddr;
    use std::path::PathBuf;

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    drop(listener);

    let config = Config {
        discord_token: "dummy_token".to_string(),
        discord_application_id: 123456789,
        discord_client_id: "123456789".to_string(),
        discord_client_secret: "secret".to_string(),
        discord_oauth_redirect_url: "http://127.0.0.1:8080/auth/discord/callback".to_string(),
        discord_dev_guild_id: None,
        dashboard_bind: addr,
        dashboard_public_url: format!("http://{addr}"),
        database_url: None,
        scripts_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts"),
        session_cookie_secure: false,
        session_ttl_seconds: 86400,
        lua_memory_limit_bytes: 4 * 1024 * 1024,
        lua_instruction_limit: 200_000,
        lua_hook_granularity: 100,
        music_allowed_hosts: HashSet::new(),
        trusted_proxies: ["127.0.0.1".parse().unwrap(), "::1".parse().unwrap()]
            .into_iter()
            .collect(),
    };

    let state = AppState::bootstrap(config)
        .await
        .expect("bootstrap should succeed");

    // Pre-populate a test case for guild 10
    let now = Utc::now();
    let created = state
        .moderation_cases
        .create_case(warning(now), now)
        .await
        .expect("case creation should succeed");
    assert_eq!(created.case_number, 1);
    assert_eq!(created.version, 1);

    // Setup authenticated session with admin permissions on guild 10
    let session_id = "test-session-secret-occ-123";
    let csrf_token = "test-csrf-secret-occ-456";
    let user = DashboardUser {
        id: "30".to_string(),
        username: "testmod".to_string(),
        global_name: Some("TestMod".to_string()),
        avatar: None,
    };
    let guilds = vec![DashboardGuild {
        id: "10".to_string(),
        name: "Test Guild 10".to_string(),
        icon: None,
        owner: true,
        permissions: "8".to_string(),
    }];
    state
        .sessions
        .save_session(
            session_id,
            user,
            guilds,
            csrf_token.to_string(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let serve_handle = tokio::spawn(rusty_discord_bot::web::serve_with_shutdown(
        state.clone(),
        shutdown_rx,
    ));

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}/api/guilds/10/cases/1");

    // 1. PATCH with expected_version = 0 must be rejected with 400 Bad Request
    let res = client
        .patch(&base_url)
        .header("Cookie", format!("rusty_discord_bot_session={session_id}"))
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "expected_version": 0,
            "reason": "Updated without version"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        body["error"],
        "Wymagane jest dodatnie expected_version do kontroli wersji."
    );

    // 2. PATCH with negative expected_version must be rejected with 400 Bad Request
    let res = client
        .patch(&base_url)
        .header("Cookie", format!("rusty_discord_bot_session={session_id}"))
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "expected_version": -1,
            "reason": "Updated with negative version"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);

    // 3. PATCH without expected_version field must default to 0 and be rejected with 400 Bad Request
    let res = client
        .patch(&base_url)
        .header("Cookie", format!("rusty_discord_bot_session={session_id}"))
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "reason": "Missing expected_version key"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);

    // 4. POST /void with expected_version = 0 must be rejected with 400 Bad Request
    let res = client
        .post(format!("{base_url}/void"))
        .header("Cookie", format!("rusty_discord_bot_session={session_id}"))
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "expected_version": 0,
            "reason": "Void without version"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        body["error"],
        "Wymagane jest dodatnie expected_version do kontroli wersji."
    );

    // 5. POST /void with missing expected_version must default to 0 and be rejected
    let res = client
        .post(format!("{base_url}/void"))
        .header("Cookie", format!("rusty_discord_bot_session={session_id}"))
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "reason": "Void missing version key"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);

    // 6. PATCH with correct expected_version = 1 succeeds and bumps version to 2
    let res = client
        .patch(&base_url)
        .header("Cookie", format!("rusty_discord_bot_session={session_id}"))
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "expected_version": 1,
            "reason": "Legitimate update with OCC"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let updated: serde_json::Value = res.json().await.unwrap();
    assert_eq!(updated["version"], 2);

    // 7. Stale expected_version = 1 against now version 2 returns 409 Conflict
    let res = client
        .patch(&base_url)
        .header("Cookie", format!("rusty_discord_bot_session={session_id}"))
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "expected_version": 1,
            "reason": "Stale update attempt"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::CONFLICT);

    // 8. Stale expected_version = 1 on POST /void returns 409 Conflict
    let res = client
        .post(format!("{base_url}/void"))
        .header("Cookie", format!("rusty_discord_bot_session={session_id}"))
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "expected_version": 1,
            "reason": "Stale void attempt"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::CONFLICT);

    // 9. POST /void with correct expected_version = 2 succeeds
    let res = client
        .post(format!("{base_url}/void"))
        .header("Cookie", format!("rusty_discord_bot_session={session_id}"))
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "expected_version": 2,
            "reason": "Legitimate void with matching version"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let voided: serde_json::Value = res.json().await.unwrap();
    assert_eq!(voided["status"], "voided");
    assert_eq!(voided["version"], 3);

    // Clean up server
    let _ = shutdown_tx.send(());
    let _ = serve_handle.await;
}

#[test]
fn void_case_modal_dialog_reflects_accurate_record_voiding_without_false_reversal_claims() {
    let index_html =
        std::fs::read_to_string("web/static/index.html").expect("web/static/index.html must exist");

    // Modal dialog must be present
    assert!(index_html.contains(r#"<dialog id="void-dialog""#));

    // Must clearly state that it only marks records void in database ewidencja and does not reverse Discord sanctions automatically
    assert!(
        index_html.contains("Unieważnienie sprawy unieważnia wyłącznie rekord w ewidencji i historii bazy danych. Nie cofa automatycznie aktywnego bana ani timeoutu w Discordzie – wymaga to oddzielnej czynności moderatorskiej."),
        "Dialog description must accurately explain that voiding is recorded in the DB only and does not automatically lift Discord bans/timeouts"
    );

    // Must not contain misleading claims that voiding cancels or reverses the sanction
    assert!(
        !index_html.contains("anuluje sankcję"),
        "Dialog must not falsely claim that voiding 'anuluje sankcję'"
    );

    // Verify app.js feedback message
    let app_js =
        std::fs::read_to_string("web/static/app.js").expect("web/static/app.js must exist");
    assert!(
        app_js.contains("kara w Discordzie wymaga osobnego cofnięcia"),
        "app.js must alert the moderator that the Discord sanction requires separate manual reversal"
    );
}

#[tokio::test]
async fn case_view_and_list_chunk_long_notes_and_evidence_to_stay_within_discord_limits() {
    use rusty_discord_bot::discord::executor::chunk_discord_text;

    let store = ModerationCaseStore::memory();
    let now = Utc::now();

    // 1. Create a case with extensive documentation (many long notes and evidence items)
    let created = store.create_case(warning(now), now).await.unwrap();

    // Add 12 long notes with ~200 characters each (>2400 chars of notes alone)
    for i in 1..=12 {
        let note_body = format!(
            "Notatka moderacyjna #{i}: Szczegółowe podsumowanie rozmowy wyjaśniającej z użytkownikiem odnośnie złamania regulaminu oraz warunków nałożenia sankcji tymczasowej na serwerze testowym."
        );
        store
            .add_note(10, created.case_number, 30, note_body, true, now)
            .await
            .unwrap();
    }

    // Add 6 evidence items with ~120 characters each (>700 chars of evidence)
    for i in 1..=6 {
        store
            .add_evidence(
                10,
                created.case_number,
                30,
                format!("dowod_{i}"),
                format!("https://cdn.discordapp.com/attachments/10/50/screenshot_{i}_evidence_long_context_identifier.png"),
                now,
            )
            .await
            .unwrap();
    }

    let details = store.get_case(10, created.case_number, now).await.unwrap();
    assert_eq!(details.notes.len(), 12);
    assert_eq!(details.evidence.len(), 6);

    // Format the view message as done by ModerationCaseOperation::View
    let c = &details.case;
    let mut message = format!(
        "📋 **Sprawa #{case_number}** [{kind:?} | {severity:?}] — Status: **{status:?}**\n• Cel: <@{subject}> | Moderator: <@{actor}>\n• Punkty: {points} | Wygasa: {expiry}\n• Powód: {reason}\n• Utworzono: {created}",
        case_number = c.case_number,
        kind = c.kind,
        severity = c.severity,
        status = c.status,
        subject = c.subject_user_id,
        actor = c.actor_user_id,
        points = c.points,
        expiry = c.expires_at.map(|e| e.to_rfc3339()).unwrap_or_else(|| "Nigdy".into()),
        reason = c.reason,
        created = c.created_at.to_rfc3339(),
    );

    message.push_str("\n\n**Notatki:**");
    for note in &details.notes {
        message.push_str(&format!(
            "\n- <@{}> ({}): {}",
            note.author_user_id,
            note.created_at.to_rfc3339(),
            note.body
        ));
    }

    message.push_str("\n\n**Dowody:**");
    for ev in &details.evidence {
        message.push_str(&format!("\n- [{}] {}", ev.label, ev.value));
    }

    // Verify raw message significantly exceeds Discord single message limit (2000 chars)
    assert!(
        message.len() > 3000,
        "raw case view should exceed 3000 chars to test chunking, actual: {}",
        message.len()
    );

    // Chunking to 1950 (Discord safety margin under 2000 chars)
    let chunks = chunk_discord_text(&message, 1950);
    assert!(
        chunks.len() >= 2,
        "case view message must be chunked into at least 2 parts, got {}",
        chunks.len()
    );

    for (idx, chunk) in chunks.iter().enumerate() {
        assert!(
            chunk.len() <= 1950,
            "chunk #{idx} exceeded limit 1950 with length {}",
            chunk.len()
        );
        assert!(
            chunk.len() <= 2000,
            "chunk #{idx} exceeded Discord 2000 character limit"
        );
    }

    // All notes and evidence must be preserved across chunks
    for i in 1..=12 {
        let note_needle = format!("Notatka moderacyjna #{i}");
        assert!(
            chunks.iter().any(|c| c.contains(&note_needle)),
            "note #{i} was lost during chunking"
        );
    }
    for i in 1..=6 {
        let ev_needle = format!("dowod_{i}");
        assert!(
            chunks.iter().any(|c| c.contains(&ev_needle)),
            "evidence #{i} was lost during chunking"
        );
    }

    // 2. Test /case list chunking when many cases are returned
    for i in 2..=15 {
        let mut case_req = warning(now);
        case_req.reason = format!(
            "Przypadek naruszenia zasad #{i}: Wielokrotny spam i próby ominięcia filtrów automatycznych na kanale publicznym wraz z odmową współpracy z moderacją."
        );
        store.create_case(case_req, now).await.unwrap();
    }

    let cases = store
        .list_cases(
            10,
            CaseFilter {
                subject_user_id: None,
                actor_user_id: None,
                kind: None,
                status: None,
                visible_to_subject_only: false,
                limit: 15,
            },
            now,
        )
        .await
        .unwrap();
    assert_eq!(cases.len(), 15);

    let mut list_message = format!("📋 **Sprawy moderacyjne ({})**:\n", cases.len());
    for item in &cases {
        list_message.push_str(&format!(
            "• **#{}** [{:?}] dla <@{}>: {} (Status: {:?})\n",
            item.case_number, item.kind, item.subject_user_id, item.reason, item.status
        ));
    }

    assert!(
        list_message.len() > 2000,
        "raw case list message should exceed 2000 chars, got {}",
        list_message.len()
    );

    let list_chunks = chunk_discord_text(&list_message, 1950);
    assert!(
        list_chunks.len() >= 2,
        "case list message must be chunked into at least 2 parts, got {}",
        list_chunks.len()
    );

    for (idx, chunk) in list_chunks.iter().enumerate() {
        assert!(
            chunk.len() <= 1950,
            "list chunk #{idx} exceeded limit 1950 with length {}",
            chunk.len()
        );
    }

    for item in &cases {
        let needle = format!("• **#{}**", item.case_number);
        assert!(
            list_chunks.iter().any(|c| c.contains(&needle)),
            "case #{} missing from list chunks",
            item.case_number
        );
    }
}
