use chrono::{Duration, Utc};
use rusty_discord_bot::{
    moderation_cases::{CaseKind, CaseSeverity, CreateCase, ModerationCaseStore},
    scheduler::{
        CreateJobOutcome, JobMutation, JobMutationOutcome, NewScheduledJob, SchedulerStore,
    },
    session::{DashboardGuild, DashboardUser, PostgresSessionStore},
    storage::{PostgresStore, UpdateModuleSettingsParams},
    tickets::{
        NewTicketReservation, PostgresTicketStore, ReserveTicketOutcome, STATUS_OPEN,
        TicketMutationOutcome,
    },
};
use serde_json::json;
use uuid::Uuid;

fn get_database_url() -> Option<String> {
    std::env::var("DATABASE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
}

#[tokio::test]
async fn test_postgres_integration_suite() {
    let Some(database_url) = get_database_url() else {
        eprintln!("SKIPPING PostgreSQL integration tests: DATABASE_URL not set");
        return;
    };

    println!("Running PostgreSQL integration tests against {database_url}");

    // 1. Verify migrations and storage connectivity
    let store = PostgresStore::connect(&database_url)
        .await
        .expect("PostgresStore should connect and run all migrations");
    store.ping().await.expect("ping should succeed");

    let guild_id = format!("pg_test_guild_{}", Uuid::new_v4().simple());
    let module_id = "test_module";

    // 2. Test transactional settings & audit event
    let initial = store
        .get_module_settings(&guild_id, module_id)
        .await
        .unwrap();
    assert!(initial.is_none());

    let params = UpdateModuleSettingsParams {
        guild_id: &guild_id,
        module_id,
        enabled: true,
        config: json!({"theme": "dark", "rate": 5}),
        actor_id: Some("actor_123"),
        audit_event: "module_config_updated",
        audit_data: json!({"reason": "integration_test"}),
    };

    let updated = store
        .update_module_settings_with_audit(params)
        .await
        .expect("atomic settings update and audit should succeed in transaction");
    assert!(updated.enabled);
    assert_eq!(updated.config["theme"], "dark");

    let fetched = store
        .get_module_settings(&guild_id, module_id)
        .await
        .unwrap()
        .expect("module settings should persist in PostgreSQL");
    assert!(fetched.enabled);
    assert_eq!(fetched.config["rate"], 5);

    // 3. Test Scheduler Store in PostgreSQL
    let scheduler = SchedulerStore::connect(Some(&database_url))
        .await
        .expect("SchedulerStore should connect to Postgres");

    let now = Utc::now();
    let job = NewScheduledJob {
        guild_id: guild_id.clone(),
        module_id: "scheduler".to_owned(),
        channel_id: "channel_1001".to_owned(),
        creator_user_id: "user_owner_1".to_owned(),
        content: "PostgreSQL test reminder".to_owned(),
        mention_creator: true,
        run_at: now - Duration::seconds(5), // already due
        repeat_every_seconds: None,
        remaining_runs: Some(1),
        max_attempts: 5,
    };

    let outcome = scheduler
        .create_job(job, 10)
        .await
        .expect("create job in postgres");
    let job_record = match outcome {
        CreateJobOutcome::Created(record) => record,
        other => panic!("expected Created job outcome, got {:?}", other),
    };

    // List jobs
    let jobs = scheduler
        .list_jobs(&guild_id, "user_owner_1", true, 10)
        .await
        .expect("list jobs");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, job_record.id);

    // Mutate job (Pause and Cancel)
    let pause_outcome = scheduler
        .mutate_job(
            &guild_id,
            &job_record.id,
            "user_owner_1",
            false,
            JobMutation::Pause,
        )
        .await
        .expect("pause job");
    assert!(matches!(pause_outcome, JobMutationOutcome::Updated(_)));

    let cancel_outcome = scheduler
        .mutate_job(
            &guild_id,
            &job_record.id,
            "user_owner_1",
            false,
            JobMutation::Cancel,
        )
        .await
        .expect("cancel job");
    assert!(matches!(cancel_outcome, JobMutationOutcome::Updated(_)));

    // Test claim_due and cancellation tracking in PostgreSQL (#139 regression)
    let due_job = NewScheduledJob {
        guild_id: guild_id.clone(),
        module_id: "scheduler".to_owned(),
        channel_id: "channel_1001".to_owned(),
        creator_user_id: "user_owner_1".to_owned(),
        content: "PostgreSQL claim_due test reminder".to_owned(),
        mention_creator: false,
        run_at: Utc::now() - Duration::seconds(10), // due
        repeat_every_seconds: None,
        remaining_runs: Some(1),
        max_attempts: 5,
    };
    let due_outcome = scheduler
        .create_job(due_job, 10)
        .await
        .expect("create due job in postgres");
    let due_record = match due_outcome {
        CreateJobOutcome::Created(record) => record,
        other => panic!("expected Created job outcome, got {:?}", other),
    };
    assert!(due_record.cancellation_requested_at.is_none());

    // Claim due job: verifies that claim_due correctly decodes ScheduledJob with cancellation_requested_at
    let worker_id = "worker-pg-regression-test";
    let claimed = scheduler
        .claim_due(
            worker_id,
            Utc::now(),
            Utc::now() - Duration::seconds(60),
            10,
        )
        .await
        .expect("claim_due on PostgreSQL must decode ScheduledJob with cancellation_requested_at");
    let claimed_job = claimed
        .into_iter()
        .find(|j| j.id == due_record.id)
        .expect("claimed jobs must contain due_record");
    assert_eq!(claimed_job.status, "processing");
    assert_eq!(claimed_job.locked_by.as_deref(), Some(worker_id));
    assert!(claimed_job.cancellation_requested_at.is_none());

    // Cancel while in processing state -> graceful cancellation requested
    let cancel_processing = scheduler
        .mutate_job(
            &guild_id,
            &due_record.id,
            "user_owner_1",
            false,
            JobMutation::Cancel,
        )
        .await
        .expect("cancel processing job in postgres");
    match cancel_processing {
        JobMutationOutcome::Updated(updated) => {
            assert_eq!(updated.status, "processing");
            assert!(updated.cancellation_requested_at.is_some());
        }
        other => panic!(
            "expected Updated outcome for processing cancel, got {:?}",
            other
        ),
    }

    // Verify cancellation_requested returns true
    assert!(
        scheduler
            .cancellation_requested(&due_record.id, worker_id)
            .await
            .expect("check cancellation_requested")
    );

    // Finalize cancellation
    scheduler
        .mark_cancelled(&due_record.id, worker_id, Utc::now())
        .await
        .expect("mark_cancelled on postgres");

    let final_job = scheduler
        .get_job(&due_record.id)
        .await
        .expect("get job after cancel")
        .expect("job should exist");
    assert_eq!(final_job.status, "cancelled");
    assert!(
        !scheduler
            .cancellation_requested(&due_record.id, worker_id)
            .await
            .unwrap()
    );

    // 4. Test Tickets Store in PostgreSQL
    let tickets = PostgresTicketStore::connect(&database_url)
        .await
        .expect("TicketStore should connect to Postgres");

    let reservation = NewTicketReservation {
        guild_id: guild_id.clone(),
        creator_user_id: "ticket_creator_1".to_owned(),
        subject: "Problem z kontem".to_owned(),
        description: "Nie mogę otworzyć panelu.".to_owned(),
        queue: "support".to_owned(),
    };

    let reserve_outcome = tickets
        .reserve_ticket(reservation.clone(), 1)
        .await
        .expect("reserve ticket in postgres");
    let reservation_record = match reserve_outcome {
        ReserveTicketOutcome::Reserved(record) => *record,
        other => panic!("expected Reserved outcome, got {:?}", other),
    };

    // Attempt second open ticket with limit 1 should be rejected atomically
    let limit_outcome = tickets
        .reserve_ticket(reservation, 1)
        .await
        .expect("second reserve check");
    match limit_outcome {
        ReserveTicketOutcome::LimitReached {
            limit,
            active_count,
        } => {
            assert_eq!(limit, 1);
            assert_eq!(active_count, 1);
        }
        other => panic!("expected LimitReached, got {:?}", other),
    }

    // Activate ticket
    let act_outcome = tickets
        .activate_ticket(&reservation_record.id, "channel_999", "ticket-support-1")
        .await
        .expect("activate ticket");
    match act_outcome {
        TicketMutationOutcome::Updated(rec) => {
            assert_eq!(rec.status, STATUS_OPEN);
            assert_eq!(rec.channel_id.as_deref(), Some("channel_999"));
        }
        other => panic!("expected Updated outcome, got {:?}", other),
    }

    // 5. Test Sessions Store in PostgreSQL
    let sessions = PostgresSessionStore::connect(&database_url)
        .await
        .expect("PostgresSessionStore should connect to Postgres");

    let session_id = format!("pg_sess_token_{}", Uuid::new_v4());
    let user = DashboardUser {
        id: "pg_user_42".to_string(),
        username: "postgres_admin".to_string(),
        global_name: Some("Postgres Admin".to_string()),
        avatar: None,
    };
    let guilds = vec![DashboardGuild {
        id: guild_id.clone(),
        name: "PostgreSQL Server".to_string(),
        icon: None,
        owner: true,
        permissions: "8".to_string(),
    }];

    sessions
        .save_session(
            &session_id,
            user.clone(),
            guilds.clone(),
            "pg_csrf_secret".to_string(),
            Utc::now() + Duration::hours(1),
            Some("ephemeral_discord_bearer_token".to_string()),
        )
        .await
        .expect("save session to postgres");

    // Fetch session and verify access token was discarded
    let fetched_session = sessions
        .get_session(&session_id)
        .await
        .unwrap()
        .expect("session should exist in postgres");
    assert_eq!(fetched_session.user.id, "pg_user_42");
    assert_eq!(fetched_session.user.username, "postgres_admin");
    assert_eq!(fetched_session.guilds.len(), 1);
    assert!(
        fetched_session.access_token.is_none(),
        "access_token must not be persisted at rest"
    );

    // Verify last_seen_at throttling in PostgreSQL:
    // 100 consecutive get_session calls must NOT update last_seen_at
    let initial_last_seen = sessions
        .get_stored_session_last_seen(&session_id)
        .await
        .unwrap()
        .expect("last_seen_at should exist");

    for _ in 0..100 {
        let s = sessions.get_session(&session_id).await.unwrap();
        assert!(s.is_some());
    }

    let after_100_calls = sessions
        .get_stored_session_last_seen(&session_id)
        .await
        .unwrap()
        .expect("last_seen_at should exist");

    assert_eq!(
        initial_last_seen, after_100_calls,
        "PostgreSQL last_seen_at must not be updated during repeated calls within 60s"
    );

    // Simulate session idle for > 60s by setting last_seen_at = NOW() - INTERVAL '120 seconds'
    let raw_pool = sqlx::postgres::PgPoolOptions::new()
        .connect(&database_url)
        .await
        .unwrap();
    let hash = rusty_discord_bot::session::hash_session_id(&session_id);
    sqlx::query(
        "UPDATE dashboard_sessions SET last_seen_at = NOW() - INTERVAL '120 seconds' WHERE session_hash = $1",
    )
    .bind(&hash)
    .execute(&raw_pool)
    .await
    .unwrap();

    let old_last_seen = sessions
        .get_stored_session_last_seen(&session_id)
        .await
        .unwrap()
        .unwrap();
    assert!(old_last_seen < initial_last_seen);

    // Now get_session must update last_seen_at
    let s_qualifying = sessions.get_session(&session_id).await.unwrap();
    assert!(s_qualifying.is_some());

    let updated_last_seen = sessions
        .get_stored_session_last_seen(&session_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        updated_last_seen > old_last_seen,
        "get_session must update last_seen_at after >= 60s threshold"
    );

    // Concurrent access test: multiple parallel requests on qualifying session
    sqlx::query(
        "UPDATE dashboard_sessions SET last_seen_at = NOW() - INTERVAL '120 seconds' WHERE session_hash = $1",
    )
    .bind(&hash)
    .execute(&raw_pool)
    .await
    .unwrap();

    let mut handles = Vec::new();
    for _ in 0..10 {
        let s_clone = sessions.clone();
        let sid = session_id.clone();
        handles.push(tokio::spawn(async move {
            s_clone.get_session(&sid).await.unwrap()
        }));
    }
    for h in handles {
        let res = h.await.unwrap();
        assert!(res.is_some());
    }

    // Test OAuth state single-use consumption
    let oauth_token = format!("oauth_state_{}", Uuid::new_v4());
    sessions.save_oauth_state(&oauth_token).await.unwrap();
    assert!(sessions.consume_oauth_state(&oauth_token).await.unwrap());
    assert!(!sessions.consume_oauth_state(&oauth_token).await.unwrap());

    // Test session logout deletion
    sessions.delete_session(&session_id).await.unwrap();
    assert!(sessions.get_session(&session_id).await.unwrap().is_none());

    // 6. Test ModerationCaseStore and Audit Log in PostgreSQL
    let mod_store = ModerationCaseStore::connect(Some(&database_url))
        .await
        .expect("ModerationCaseStore should connect to Postgres");
    let mod_now = Utc::now();
    let mod_case = mod_store
        .create_case(
            CreateCase {
                guild_id: 9990001,
                subject_user_id: 1110001,
                actor_user_id: 2220001,
                kind: CaseKind::Warning,
                severity: CaseSeverity::High,
                points: 5,
                reason: "PostgreSQL integration test warning".to_string(),
                source_module: "moderation".to_string(),
                visible_to_subject: true,
                expires_at: Some(mod_now + Duration::days(7)),
            },
            mod_now,
        )
        .await
        .expect("create moderation case in PostgreSQL");
    assert!(mod_case.case_number >= 1);
    assert_eq!(mod_case.points, 5);

    mod_store
        .add_note(
            9990001,
            mod_case.case_number,
            2220001,
            "Postgres test note".to_string(),
            true,
            mod_now,
        )
        .await
        .expect("add note in PostgreSQL");

    let details = mod_store
        .get_case(9990001, mod_case.case_number, mod_now)
        .await
        .expect("get case details from PostgreSQL");
    assert_eq!(details.notes.len(), 1);

    // Verify Softban case creation in PostgreSQL (0007 migration)
    let softban_case = mod_store
        .create_case(
            CreateCase {
                guild_id: 9990001,
                subject_user_id: 1110002,
                actor_user_id: 2220001,
                kind: CaseKind::Softban,
                severity: CaseSeverity::High,
                points: 3,
                reason: "PostgreSQL softban test".to_string(),
                source_module: "moderation".to_string(),
                visible_to_subject: true,
                expires_at: None,
            },
            mod_now,
        )
        .await
        .expect("create softban case in PostgreSQL");
    assert_eq!(softban_case.kind, CaseKind::Softban);

    // Verify Audit log persistence and retrieval
    let audit_events = store.list_audit_events(&guild_id, 10).await.unwrap();
    assert!(!audit_events.is_empty());
    assert_eq!(audit_events[0].module_id, "dashboard");
    assert_eq!(audit_events[0].event, "module_config_updated");
    assert_eq!(audit_events[0].data["reason"], "integration_test");

    // 6. Issue #149: Test archive_progression_season decodes level INTEGER into i32 without SQLx error
    store
        .adjust_member_progress(&guild_id, "season_user_1", 2500, 150, 0)
        .await
        .expect("seed member_progress 1");
    store
        .adjust_member_progress(&guild_id, "season_user_2", 500, 25, 0)
        .await
        .expect("seed member_progress 2");

    let season = store
        .archive_progression_season(&guild_id, "Season 1")
        .await
        .expect("archive_progression_season should decode level INTEGER into i32 successfully");
    assert_eq!(season.name, "Season 1");
    assert_eq!(season.standings_json.as_array().unwrap().len(), 2);
    let first = &season.standings_json[0];
    assert_eq!(first["user_id"], "season_user_1");
    assert_eq!(first["xp"], 2500);

    // 7. Issue #153: Test get_or_create_ai_budget does not mutate or update existing budget on repeated reads
    let ai_guild_1 = format!("pg_ai_guild_1_{}", Uuid::new_v4().simple());
    let ai_guild_2 = format!("pg_ai_guild_2_{}", Uuid::new_v4().simple());

    // 7.1. First read creates default budget with expected values
    let budget_1 = store
        .get_or_create_ai_budget(&ai_guild_1)
        .await
        .expect("initial get_or_create_ai_budget should succeed");
    assert_eq!(budget_1.guild_id, ai_guild_1);
    assert_eq!(budget_1.monthly_token_limit, 1_000_000);
    assert_eq!(budget_1.used_tokens, 0);
    assert_eq!(budget_1.monthly_usd_limit, 10.0);
    assert_eq!(budget_1.used_usd, 0.0);
    assert!(!budget_1.is_halted);

    // Retrieve initial xmin system column to verify row versioning
    let xmin_initial: i64 =
        sqlx::query_scalar("SELECT xmin::text::bigint FROM ai_guild_budgets WHERE guild_id = $1")
            .bind(&ai_guild_1)
            .fetch_one(store.pool())
            .await
            .expect("fetch initial xmin");

    // 7.2. Repeated reads (10 consecutive calls) must NOT perform UPDATE or bump xmin
    for _ in 0..10 {
        let repeated_budget = store
            .get_or_create_ai_budget(&ai_guild_1)
            .await
            .expect("repeated get_or_create_ai_budget should succeed");
        assert_eq!(repeated_budget.used_tokens, 0);
        assert_eq!(repeated_budget.used_usd, 0.0);
        assert_eq!(repeated_budget.reset_at, budget_1.reset_at);
    }

    let xmin_after_reads: i64 =
        sqlx::query_scalar("SELECT xmin::text::bigint FROM ai_guild_budgets WHERE guild_id = $1")
            .bind(&ai_guild_1)
            .fetch_one(store.pool())
            .await
            .expect("fetch xmin after reads");

    assert_eq!(
        xmin_initial, xmin_after_reads,
        "Repeated get_or_create_ai_budget calls must NOT mutate row version (xmin unchanged)"
    );

    // 7.3. Record AI usage updates usage and bumps xmin
    let updated_usage = store
        .record_ai_usage(&ai_guild_1, 5000, 0.12)
        .await
        .expect("record_ai_usage should succeed");
    assert_eq!(updated_usage.used_tokens, 5000);
    assert_eq!(updated_usage.used_usd, 0.12);

    let xmin_after_usage: i64 =
        sqlx::query_scalar("SELECT xmin::text::bigint FROM ai_guild_budgets WHERE guild_id = $1")
            .bind(&ai_guild_1)
            .fetch_one(store.pool())
            .await
            .expect("fetch xmin after usage");

    assert_ne!(
        xmin_initial, xmin_after_usage,
        "record_ai_usage must update the row (new xmin)"
    );

    // 7.4. Read after usage returns updated usage without resetting or mutating row
    let read_after_usage = store
        .get_or_create_ai_budget(&ai_guild_1)
        .await
        .expect("get_or_create_ai_budget after usage");
    assert_eq!(read_after_usage.used_tokens, 5000);
    assert_eq!(read_after_usage.used_usd, 0.12);

    let xmin_after_read_post_usage: i64 =
        sqlx::query_scalar("SELECT xmin::text::bigint FROM ai_guild_budgets WHERE guild_id = $1")
            .bind(&ai_guild_1)
            .fetch_one(store.pool())
            .await
            .expect("fetch xmin after read post usage");

    assert_eq!(
        xmin_after_usage, xmin_after_read_post_usage,
        "Reading budget after usage must NOT mutate row version (xmin unchanged)"
    );

    // 7.5. Concurrent initial reads (20 parallel requests for a new guild)
    let concurrent_guild = format!("pg_ai_guild_concurrent_{}", Uuid::new_v4().simple());
    let mut handles = Vec::new();
    for _ in 0..20 {
        let store_clone = store.clone();
        let g_id = concurrent_guild.clone();
        handles.push(tokio::spawn(async move {
            store_clone.get_or_create_ai_budget(&g_id).await
        }));
    }

    for handle in handles {
        let res = handle
            .await
            .expect("join handle")
            .expect("concurrent get_or_create_ai_budget");
        assert_eq!(res.guild_id, concurrent_guild);
        assert_eq!(res.used_tokens, 0);
    }

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ai_guild_budgets WHERE guild_id = $1")
            .bind(&concurrent_guild)
            .fetch_one(store.pool())
            .await
            .expect("count concurrent guild rows");
    assert_eq!(
        count, 1,
        "Exactly one row should exist for concurrent initialization"
    );

    // 7.6. Guild isolation (guild 2 is independent of guild 1)
    let budget_2 = store
        .get_or_create_ai_budget(&ai_guild_2)
        .await
        .expect("guild 2 get_or_create_ai_budget");
    assert_eq!(budget_2.guild_id, ai_guild_2);
    assert_eq!(budget_2.used_tokens, 0);
    assert_eq!(budget_2.used_usd, 0.0);
}
