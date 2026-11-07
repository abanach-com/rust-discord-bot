use std::sync::Arc;

use chrono::{Duration, Utc};
use rusty_discord_bot::{
    moderation_cases::{CaseKind, CaseSeverity, CreateCase, ModerationCaseStore},
    scheduler::{
        CreateJobOutcome, JobMutation, JobMutationOutcome, NewScheduledJob, SchedulerStore,
    },
    session::{DashboardGuild, DashboardUser, PostgresSessionStore},
    storage::{PostgresStore, UpdateModuleSettingsParams},
    tickets::{
        NewTicketReservation, NewTicketTranscript, PostgresTicketStore, ReserveTicketOutcome,
        STATUS_OPEN, TicketMutationOutcome, TranscriptCrypto,
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

    // 3b. Regression test for Issue #259: claim_due handles terminalization failure gracefully and updates metrics
    let pg_pool = match &scheduler {
        SchedulerStore::Postgres(pg) => pg.pool().clone(),
        _ => unreachable!(),
    };
    let test_metrics = Arc::new(rusty_discord_bot::metrics::Metrics::new());
    let scheduler_with_metrics = scheduler.clone().with_metrics(test_metrics.clone());

    // Create job A: max_attempts = 1, will be in processing with expired lease
    let user_a = format!("user_a_{}", Uuid::new_v4().simple());
    let job_a = NewScheduledJob {
        guild_id: guild_id.clone(),
        module_id: "scheduler".to_owned(),
        channel_id: "channel_a".to_owned(),
        creator_user_id: user_a.clone(),
        content: "Job A expired lease".to_owned(),
        mention_creator: false,
        run_at: Utc::now() - Duration::seconds(200),
        repeat_every_seconds: None,
        remaining_runs: Some(1),
        max_attempts: 1,
    };
    let outcome_a = scheduler_with_metrics
        .create_job(job_a, 1)
        .await
        .expect("create job A");
    let record_a = match outcome_a {
        CreateJobOutcome::Created(record) => record,
        other => panic!("expected Created, got {:?}", other),
    };

    // Transition job A directly to processing with attempts=1, locked_at in the past
    let stale_locked_at = Utc::now() - Duration::seconds(120);
    sqlx::query(
        "UPDATE scheduled_jobs SET status = 'processing', attempts = 1, locked_by = 'worker-stale', locked_at = $1 WHERE id = $2::uuid",
    )
    .bind(stale_locked_at)
    .bind(&record_a.id)
    .execute(&pg_pool)
    .await
    .expect("setup job A in expired processing state");

    // Create job B: status = 'active', run_at <= now
    let user_b = format!("user_b_{}", Uuid::new_v4().simple());
    let job_b = NewScheduledJob {
        guild_id: guild_id.clone(),
        module_id: "scheduler".to_owned(),
        channel_id: "channel_b".to_owned(),
        creator_user_id: user_b.clone(),
        content: "Job B active due".to_owned(),
        mention_creator: false,
        run_at: Utc::now() - Duration::seconds(10),
        repeat_every_seconds: None,
        remaining_runs: Some(1),
        max_attempts: 3,
    };
    let outcome_b = scheduler_with_metrics
        .create_job(job_b, 1)
        .await
        .expect("create job B");
    let record_b = match outcome_b {
        CreateJobOutcome::Created(record) => record,
        other => panic!("expected Created, got {:?}", other),
    };

    // Verify user A has reached their limit because Job A is in processing
    let job_a2 = NewScheduledJob {
        guild_id: guild_id.clone(),
        module_id: "scheduler".to_owned(),
        channel_id: "channel_a2".to_owned(),
        creator_user_id: user_a.clone(),
        content: "Job A second attempt".to_owned(),
        mention_creator: false,
        run_at: Utc::now(),
        repeat_every_seconds: None,
        remaining_runs: Some(1),
        max_attempts: 1,
    };
    let limit_check = scheduler_with_metrics
        .create_job(job_a2.clone(), 1)
        .await
        .expect("create job A2 should check limit");
    assert!(matches!(
        limit_check,
        CreateJobOutcome::LimitReached { limit: 1 }
    ));

    // Inject temporary trigger on scheduled_jobs to simulate controlled failure of processing -> failed
    sqlx::query(
        r#"
        CREATE OR REPLACE FUNCTION fail_terminalize_processing() RETURNS trigger AS $$
        BEGIN
            IF OLD.status = 'processing' AND NEW.status = 'failed' THEN
                RAISE EXCEPTION 'simulated failure in expired lease terminalization';
            END IF;
            RETURN NEW;
        END;
        $$ LANGUAGE plpgsql;

        CREATE TRIGGER trigger_fail_terminalize
        BEFORE UPDATE ON scheduled_jobs
        FOR EACH ROW
        EXECUTE FUNCTION fail_terminalize_processing();
        "#,
    )
    .execute(&pg_pool)
    .await
    .expect("install simulated failure trigger");

    // Call claim_due: cleanup will encounter the trigger failure, but must NOT drop error silently
    // and must NOT block Job B from being claimed (preventing regression #60).
    let now = Utc::now();
    let stale_before = now - Duration::seconds(60);
    let claimed_jobs = scheduler_with_metrics
        .claim_due("worker-claim-resilient", now, stale_before, 10)
        .await
        .expect("claim_due must not fail when terminalization query fails");

    // Job B was successfully claimed
    assert!(claimed_jobs.iter().any(|j| j.id == record_b.id));

    // Job A was NOT updated due to trigger failure
    let job_a_stuck = scheduler_with_metrics
        .get_job(&record_a.id)
        .await
        .expect("get job A")
        .expect("job A exists");
    assert_eq!(job_a_stuck.status, "processing");

    // Operator telemetry: cleanup error counter incremented
    assert_eq!(
        test_metrics
            .scheduler_expired_lease_cleanup_errors_total
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );

    // Drop the simulated trigger
    sqlx::query(
        r#"
        DROP TRIGGER IF EXISTS trigger_fail_terminalize ON scheduled_jobs;
        DROP FUNCTION IF EXISTS fail_terminalize_processing();
        "#,
    )
    .execute(&pg_pool)
    .await
    .expect("drop simulated failure trigger");

    // Call finalize_expired_leases or claim_due again: now cleanup succeeds
    let terminalized_count = scheduler_with_metrics
        .finalize_expired_leases(now, stale_before)
        .await
        .expect("finalize_expired_leases should succeed after trigger dropped");
    assert!(terminalized_count >= 1);

    // Job A is now terminalized to failed
    let job_a_final = scheduler_with_metrics
        .get_job(&record_a.id)
        .await
        .expect("get job A")
        .expect("job A exists");
    assert_eq!(job_a_final.status, "failed");
    assert_eq!(
        job_a_final.last_error.as_deref(),
        Some("worker lease expired after reaching maximum attempts")
    );
    assert!(job_a_final.locked_by.is_none());
    assert!(job_a_final.locked_at.is_none());

    // Terminalized metrics incremented
    assert!(
        test_metrics
            .scheduler_expired_lease_terminalized_total
            .load(std::sync::atomic::Ordering::Relaxed)
            >= 1
    );

    // User A quota is now freed: user A can create a job
    let unblocked_outcome = scheduler_with_metrics
        .create_job(job_a2, 1)
        .await
        .expect("create job A2 should succeed now");
    assert!(matches!(unblocked_outcome, CreateJobOutcome::Created(_)));

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

    // 4b. Test Tickets Transcript Encryption in PostgreSQL (#254)
    let transcript_content = "Operator: Support session initiated.\nUser: Need assistance.";
    let new_transcript = NewTicketTranscript {
        ticket_id: reservation_record.id.clone(),
        guild_id: guild_id.clone(),
        channel_id: "channel_999".to_string(),
        generated_by_user_id: "ticket_creator_1".to_string(),
        message_count: 2,
        content: transcript_content.to_string(),
    };

    // Storing without crypto must fail closed (no plaintext in DB)
    let fail_closed_res = tickets.store_transcript(new_transcript.clone()).await;
    assert!(
        fail_closed_res.is_err(),
        "store_transcript without encryption keys configured must fail closed"
    );

    // Now configure crypto on PostgresTicketStore
    let key_hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let crypto = Arc::new(TranscriptCrypto::from_hex_key("v1", key_hex).unwrap());
    let mut encrypted_tickets = tickets.clone();
    encrypted_tickets.set_crypto(Some(crypto.clone()));

    let stored = encrypted_tickets
        .store_transcript(new_transcript)
        .await
        .expect("storing encrypted transcript should succeed");
    assert_eq!(stored.content, transcript_content);

    // Verify directly in DB that content is NULL at rest and ciphertext is stored
    #[derive(sqlx::FromRow)]
    struct RawTranscriptRow {
        content: Option<String>,
        ciphertext: Option<Vec<u8>>,
        nonce: Option<Vec<u8>>,
        key_id: Option<String>,
        crypto_version: Option<i16>,
    }
    let raw_row = sqlx::query_as::<_, RawTranscriptRow>(
        "SELECT content, ciphertext, nonce, key_id, crypto_version FROM ticket_transcripts WHERE id = $1::uuid",
    )
    .bind(&stored.id)
    .fetch_one(tickets.pool())
    .await
    .expect("fetch raw transcript from DB");

    assert!(
        raw_row.content.is_none(),
        "content column must be NULL at rest in DB"
    );
    assert!(raw_row.ciphertext.is_some(), "ciphertext must be populated");
    assert_eq!(
        raw_row.nonce.map(|n| n.len()),
        Some(12),
        "nonce must be 12 bytes"
    );
    assert_eq!(raw_row.key_id.as_deref(), Some("v1"));
    assert_eq!(raw_row.crypto_version, Some(1));

    // Fetch and decrypt
    let fetched = encrypted_tickets
        .get_transcript(&guild_id, &stored.id)
        .await
        .expect("get_transcript should succeed")
        .expect("transcript should exist");
    assert_eq!(fetched.content, transcript_content);

    // Tenant isolation: fetching with another guild fails to find or decrypt
    let foreign_fetch = encrypted_tickets
        .get_transcript("wrong_guild_snowflake", &stored.id)
        .await
        .expect("query should succeed");
    assert!(
        foreign_fetch.is_none(),
        "Foreign guild must not see ticket transcript"
    );

    // Latest transcript retrieval
    let latest = encrypted_tickets
        .get_latest_transcript(&guild_id, &reservation_record.id)
        .await
        .expect("get_latest_transcript should succeed")
        .expect("latest transcript should exist");
    assert_eq!(latest.id, stored.id);
    assert_eq!(latest.content, transcript_content);

    // Test legacy unencrypted row read and backfill migration
    let legacy_id = Uuid::new_v4().to_string();
    let legacy_content = "Legacy unencrypted ticket message";
    sqlx::query(
        r#"
        INSERT INTO ticket_transcripts (
            id, ticket_id, guild_id, channel_id, generated_by_user_id, message_count, content, ciphertext, nonce, key_id, crypto_version
        )
        VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6, $7, NULL, NULL, NULL, 1)
        "#,
    )
    .bind(&legacy_id)
    .bind(&reservation_record.id)
    .bind(&guild_id)
    .bind("channel_999")
    .bind("ticket_creator_1")
    .bind(1)
    .bind(legacy_content)
    .execute(tickets.pool())
    .await
    .expect("insert legacy unencrypted transcript");

    // Reading legacy row works during migration window
    let legacy_fetched = encrypted_tickets
        .get_transcript(&guild_id, &legacy_id)
        .await
        .expect("fetch legacy transcript")
        .expect("legacy transcript should exist");
    assert_eq!(legacy_fetched.content, legacy_content);

    // Run backfill
    let backfilled_count = encrypted_tickets
        .backfill_encrypt_transcripts(100)
        .await
        .expect("backfill transcripts");
    assert!(backfilled_count >= 1);

    // Verify legacy row is now encrypted in DB
    let backfilled_row = sqlx::query_as::<_, RawTranscriptRow>(
        "SELECT content, ciphertext, nonce, key_id, crypto_version FROM ticket_transcripts WHERE id = $1::uuid",
    )
    .bind(&legacy_id)
    .fetch_one(tickets.pool())
    .await
    .expect("fetch backfilled raw transcript");
    assert!(
        backfilled_row.content.is_none(),
        "backfilled row content must be NULL at rest"
    );
    assert!(
        backfilled_row.ciphertext.is_some(),
        "backfilled row ciphertext must be populated"
    );

    // Decrypting backfilled row works
    let backfilled_decrypted = encrypted_tickets
        .get_transcript(&guild_id, &legacy_id)
        .await
        .expect("fetch decrypted backfilled transcript")
        .expect("backfilled transcript should exist");
    assert_eq!(backfilled_decrypted.content, legacy_content);

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

    // 8. Issue #154: Test Tupper profile prefix uniqueness and conflict handling in PostgreSQL
    let tupper_guild_1 = format!("pg_tupper_g1_{}", Uuid::new_v4().simple());
    let tupper_guild_2 = format!("pg_tupper_g2_{}", Uuid::new_v4().simple());
    let tupper_user_a = "pg_tupper_user_a";
    let tupper_user_b = "pg_tupper_user_b";

    // 8.1. Initial creation succeeds
    let t1 = store
        .create_tupper_profile(&tupper_guild_1, tupper_user_a, "Hero A", None, "t:")
        .await
        .expect("initial tupper creation in PostgreSQL");
    assert_eq!(t1.name, "Hero A");
    assert_eq!(t1.prefix_or_brackets, "t:");

    // 8.2. Duplicate prefix for same user and guild returns TupperError::PrefixConflict
    let dup_res = store
        .create_tupper_profile(
            &tupper_guild_1,
            tupper_user_a,
            "Hero A Duplicate",
            None,
            "t:",
        )
        .await;
    match dup_res {
        Err(rusty_discord_bot::storage::TupperError::PrefixConflict) => {}
        other => panic!("Expected TupperError::PrefixConflict, got {other:?}"),
    }

    // Verify row was not inserted/modified in PostgreSQL
    let profiles = store
        .list_tupper_profiles(&tupper_guild_1, tupper_user_a)
        .await
        .expect("list tupper profiles");
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].name, "Hero A");

    // 8.3. Same prefix for distinct user in same guild succeeds
    let t_user_b = store
        .create_tupper_profile(&tupper_guild_1, tupper_user_b, "Hero B", None, "t:")
        .await
        .expect("same prefix for distinct user in PostgreSQL");
    assert_eq!(t_user_b.user_id, tupper_user_b);

    // 8.4. Same prefix for same user in distinct guild succeeds
    let t_guild_2 = store
        .create_tupper_profile(&tupper_guild_2, tupper_user_a, "Hero Guild 2", None, "t:")
        .await
        .expect("same prefix in distinct guild in PostgreSQL");
    assert_eq!(t_guild_2.guild_id, tupper_guild_2);

    // 8.5. 20 concurrent creation attempts for the same (guild, user, prefix) in PostgreSQL
    let concurrent_user = "pg_tupper_concurrent_user";
    let mut handles = Vec::new();
    for i in 0..20 {
        let store_clone = store.clone();
        let g_id = tupper_guild_1.clone();
        handles.push(tokio::spawn(async move {
            store_clone
                .create_tupper_profile(
                    &g_id,
                    concurrent_user,
                    &format!("Concurrent Tupper {i}"),
                    None,
                    "c:",
                )
                .await
        }));
    }

    let mut successes = 0;
    let mut conflicts = 0;
    for handle in handles {
        match handle.await.expect("join handle") {
            Ok(_) => successes += 1,
            Err(rusty_discord_bot::storage::TupperError::PrefixConflict) => conflicts += 1,
            Err(e) => panic!("Unexpected error: {e:?}"),
        }
    }
    assert_eq!(
        successes, 1,
        "Exactly one concurrent attempt must succeed in PostgreSQL"
    );
    assert_eq!(
        conflicts, 19,
        "Exactly 19 concurrent attempts must fail with PrefixConflict in PostgreSQL"
    );

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM tupper_profiles WHERE guild_id = $1 AND user_id = $2 AND prefix_or_brackets = $3",
    )
    .bind(&tupper_guild_1)
    .bind(concurrent_user)
    .bind("c:")
    .fetch_one(store.pool())
    .await
    .expect("count concurrent tupper rows");
    assert_eq!(
        count, 1,
        "Exactly one tupper profile row must exist in PostgreSQL"
    );

    // 9. Issue #155: Event roster monotonic position assignment and deterministic FIFO promotion in PostgreSQL
    let event = store
        .create_scheduled_event(
            &guild_id,
            "PostgreSQL Raid Event",
            "Event roster test",
            Utc::now(),
            None,
            "UTC",
            Some(1), // capacity = 1
        )
        .await
        .expect("create scheduled event in PostgreSQL");

    // 9.1. A joins -> main, pos 1
    let a = store
        .join_event_roster(event.id, "pg_user_a")
        .await
        .unwrap();
    assert_eq!(a.role_type, "main");
    assert_eq!(a.position, 1);

    // 9.2. B joins -> reserve, pos 2
    let b = store
        .join_event_roster(event.id, "pg_user_b")
        .await
        .unwrap();
    assert_eq!(b.role_type, "reserve");
    assert_eq!(b.position, 2);

    // 9.3. C joins -> reserve, pos 3
    let c = store
        .join_event_roster(event.id, "pg_user_c")
        .await
        .unwrap();
    assert_eq!(c.role_type, "reserve");
    assert_eq!(c.position, 3);

    // 9.4. B leaves -> pos 2 deleted, remaining: A[1], C[3]
    let left_b = store
        .leave_event_roster(event.id, "pg_user_b")
        .await
        .unwrap();
    assert_eq!(left_b, None);

    // 9.5. D joins -> receives pos 4 (MAX(position) + 1), NOT pos 3 (Issue #155)
    let d = store
        .join_event_roster(event.id, "pg_user_d")
        .await
        .unwrap();
    assert_eq!(d.role_type, "reserve");
    assert_eq!(
        d.position, 4,
        "PostgreSQL roster position must be MAX(position) + 1 = 4"
    );

    // Verify all active rows have unique positions in PostgreSQL
    let active = store.get_event_rosters(event.id).await.unwrap();
    let positions: Vec<i32> = active.iter().map(|r| r.position).collect();
    assert_eq!(positions, vec![1, 3, 4]);

    // 9.6. A leaves (main) -> C (reserve pos 3) MUST be promoted over D (reserve pos 4)
    let promoted = store
        .leave_event_roster(event.id, "pg_user_a")
        .await
        .unwrap();
    assert_eq!(
        promoted,
        Some("pg_user_c".to_string()),
        "Oldest reserve C must be promoted in PostgreSQL"
    );

    let after_leave = store.get_event_rosters(event.id).await.unwrap();
    assert_eq!(after_leave.len(), 2);
    let c_active = after_leave
        .iter()
        .find(|r| r.user_id == "pg_user_c")
        .unwrap();
    assert_eq!(c_active.role_type, "main");
    assert_eq!(c_active.position, 3);

    let d_active = after_leave
        .iter()
        .find(|r| r.user_id == "pg_user_d")
        .unwrap();
    assert_eq!(d_active.role_type, "reserve");
    assert_eq!(d_active.position, 4);

    // 9.7. Concurrent joins: 20 parallel requests for a new event must all receive unique positions
    let concurrent_event = store
        .create_scheduled_event(
            &guild_id,
            "Concurrent Event",
            "Test parallel joins",
            Utc::now(),
            None,
            "UTC",
            Some(5),
        )
        .await
        .unwrap();

    let mut handles = Vec::new();
    for i in 0..20 {
        let store_clone = store.clone();
        let ev_id = concurrent_event.id;
        let u_id = format!("concurrent_roster_user_{i}");
        handles.push(tokio::spawn(async move {
            store_clone.join_event_roster(ev_id, &u_id).await
        }));
    }

    for handle in handles {
        handle
            .await
            .expect("join handle")
            .expect("join event roster");
    }

    let concurrent_roster = store.get_event_rosters(concurrent_event.id).await.unwrap();
    assert_eq!(concurrent_roster.len(), 20);

    let mut seen_pos = std::collections::HashSet::new();
    for r in &concurrent_roster {
        assert!(
            seen_pos.insert(r.position),
            "Duplicate position {} in concurrent event roster in PostgreSQL",
            r.position
        );
    }

    // 10. Issue #156 & #158: Store purchase role snapshot and fulfillment tracking in PostgreSQL
    let store_user = format!("pg_store_buyer_{}", Uuid::new_v4().simple());
    sqlx::query(
        r#"
        INSERT INTO member_progress (guild_id, user_id, xp, level, coins, updated_at)
        VALUES ($1, $2, 500, 5, 1000, NOW())
        "#,
    )
    .bind(&guild_id)
    .bind(&store_user)
    .execute(store.pool())
    .await
    .unwrap();

    let item = store
        .create_store_item(
            &guild_id,
            "Emerald VIP",
            "Special role for VIPs",
            400,
            Some("999000000000000001"),
            Some(5),
        )
        .await
        .unwrap();

    let (purchased_item, purchase, remaining_coins) = store
        .purchase_store_item(&guild_id, &store_user, item.id)
        .await
        .unwrap();

    assert_eq!(purchased_item.id, item.id);
    assert_eq!(remaining_coins, 600);
    assert_eq!(purchase.price_paid, 400);
    assert_eq!(purchase.role_id, Some("999000000000000001".to_string()));
    assert_eq!(purchase.fulfillment_status, "pending");
    assert!(purchase.fulfilled_at.is_none());

    let now_pg = Utc::now();
    store
        .update_store_purchase_fulfillment(purchase.id, "completed", None, Some(now_pg))
        .await
        .unwrap();

    let fetched_purchase = store
        .get_store_purchase(purchase.id)
        .await
        .unwrap()
        .expect("purchase should exist");
    assert_eq!(fetched_purchase.fulfillment_status, "completed");
    assert_eq!(
        fetched_purchase.role_id,
        Some("999000000000000001".to_string())
    );

    let purchases_list = store
        .list_store_purchases(&guild_id, Some(&store_user))
        .await
        .unwrap();
    assert_eq!(purchases_list.len(), 1);
    assert_eq!(purchases_list[0].id, purchase.id);

    // 21. Test Member Intelligence in PostgreSQL
    use rusty_discord_bot::storage::{MemberSearchFilter, UpsertMemberSnapshotParams};

    let intel_guild_id = "777000000000000002";
    let member_user = "888000000000000001";
    let joined_dt = Utc::now() - Duration::days(5);

    store
        .upsert_member_snapshot(UpsertMemberSnapshotParams {
            guild_id: intel_guild_id,
            user_id: member_user,
            username: "bob_builder",
            global_name: Some("Bob B"),
            nickname: Some("The Builder"),
            role_ids: &["architect".to_string(), "builder".to_string()],
            joined_at: Some(joined_dt),
        })
        .await
        .expect("upsert member snapshot in postgres");

    // Identity history tracking & deduplication
    let changed_1 = store
        .record_member_identity_change(
            intel_guild_id,
            member_user,
            "nickname",
            None,
            Some("The Builder"),
            30,
        )
        .await
        .unwrap();
    assert!(
        changed_1.is_some(),
        "first identity insert should be recorded"
    );

    let changed_dupe = store
        .record_member_identity_change(
            intel_guild_id,
            member_user,
            "nickname",
            Some("The Builder"),
            Some("The Builder"),
            30,
        )
        .await
        .unwrap();
    assert!(
        changed_dupe.is_none(),
        "identical consecutive identity should be deduplicated"
    );

    let changed_2 = store
        .record_member_identity_change(
            intel_guild_id,
            member_user,
            "username",
            Some("bob_builder"),
            Some("bob_the_mighty"),
            30,
        )
        .await
        .unwrap();
    assert!(changed_2.is_some(), "changed username should be recorded");

    let history = store
        .get_member_identity_history(intel_guild_id, member_user, 10)
        .await
        .unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].kind, "username");
    assert_eq!(history[0].new_value, Some("bob_the_mighty".to_string()));
    assert_eq!(history[1].kind, "nickname");
    assert_eq!(history[1].new_value, Some("The Builder".to_string()));

    // 21b. Test Identity History Janitor & Expiration Cleanup in Postgres
    let expired_id = Uuid::new_v4();
    let past_observed = Utc::now() - chrono::Duration::days(120);
    let past_expiry = Utc::now() - chrono::Duration::days(30);
    sqlx::query(
        r#"
        INSERT INTO member_identity_history
            (id, guild_id, user_id, kind, old_value, new_value, observed_at, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(expired_id)
    .bind(intel_guild_id)
    .bind(member_user)
    .bind("global_name")
    .bind(Some("OldBob"))
    .bind(Some("ExpiredBob"))
    .bind(past_observed)
    .bind(past_expiry)
    .execute(store.pool())
    .await
    .expect("insert expired identity history record");

    let count_before: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM member_identity_history WHERE guild_id = $1 AND user_id = $2",
    )
    .bind(intel_guild_id)
    .bind(member_user)
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(count_before.0, 3);

    let history_before = store
        .get_member_identity_history(intel_guild_id, member_user, 10)
        .await
        .unwrap();
    assert_eq!(history_before.len(), 2);

    let pruned = store
        .cleanup_expired_identity_history()
        .await
        .expect("cleanup expired identity history");
    assert!(pruned >= 1, "at least 1 record should be pruned");

    let pruned_again = store
        .cleanup_expired_identity_history()
        .await
        .expect("second cleanup");
    assert_eq!(pruned_again, 0);

    let count_after: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM member_identity_history WHERE guild_id = $1 AND user_id = $2",
    )
    .bind(intel_guild_id)
    .bind(member_user)
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(count_after.0, 2);

    let history_after = store
        .get_member_identity_history(intel_guild_id, member_user, 10)
        .await
        .unwrap();
    assert_eq!(history_after.len(), 2);

    // Search members with role filter
    let results = store
        .search_members(
            intel_guild_id,
            &MemberSearchFilter {
                role_ids: vec!["builder".to_string()],
                exclude_role_ids: vec!["banned".to_string()],
                limit: Some(10),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].user_id, member_user);
    assert_eq!(results[0].username, "bob_builder");
    assert_eq!(results[0].moderation_case_count, 0);

    // 21c. Test Keyset Pagination and SQL LIMIT in PostgreSQL (#169)
    let pg_user_1 = "900719925474099991";
    let pg_user_2 = "900719925474099992";
    let pg_user_3 = "900719925474099993";
    let pg_now = Utc::now();
    let pg_t1 = pg_now - chrono::Duration::hours(1);
    let pg_t2 = pg_now - chrono::Duration::hours(2);

    store
        .upsert_member_snapshot(rusty_discord_bot::storage::UpsertMemberSnapshotParams {
            guild_id: intel_guild_id,
            user_id: pg_user_1,
            username: "pg_u1",
            global_name: None,
            nickname: None,
            role_ids: &["keyset_role".to_string()],
            joined_at: Some(pg_t1),
        })
        .await
        .unwrap();

    store
        .upsert_member_snapshot(rusty_discord_bot::storage::UpsertMemberSnapshotParams {
            guild_id: intel_guild_id,
            user_id: pg_user_2,
            username: "pg_u2",
            global_name: None,
            nickname: None,
            role_ids: &["keyset_role".to_string()],
            joined_at: Some(pg_t2),
        })
        .await
        .unwrap();

    store
        .upsert_member_snapshot(rusty_discord_bot::storage::UpsertMemberSnapshotParams {
            guild_id: intel_guild_id,
            user_id: pg_user_3,
            username: "pg_u3",
            global_name: None,
            nickname: None,
            role_ids: &["keyset_role".to_string()],
            joined_at: None,
        })
        .await
        .unwrap();

    // Query with limit=1: must return only pg_user_1
    let pg_page1 = store
        .search_members(
            intel_guild_id,
            &MemberSearchFilter {
                role_ids: vec!["keyset_role".to_string()],
                limit: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(pg_page1.len(), 1);
    assert_eq!(pg_page1[0].user_id, pg_user_1);

    let cur1 = rusty_discord_bot::storage::encode_member_search_cursor(
        pg_page1[0].joined_at,
        &pg_page1[0].user_id,
    );

    // Query page 2 with cursor: must return pg_user_2
    let pg_page2 = store
        .search_members(
            intel_guild_id,
            &MemberSearchFilter {
                role_ids: vec!["keyset_role".to_string()],
                limit: Some(1),
                cursor: Some(cur1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(pg_page2.len(), 1);
    assert_eq!(pg_page2[0].user_id, pg_user_2);

    // Delete pg_user_2 and verify cursor resilience
    let cur2 = rusty_discord_bot::storage::encode_member_search_cursor(
        pg_page2[0].joined_at,
        &pg_page2[0].user_id,
    );
    store
        .remove_member_snapshot(intel_guild_id, pg_user_2)
        .await
        .unwrap();

    // Page 3 query using deleted user cursor continues to pg_user_3 without restarting
    let pg_page3 = store
        .search_members(
            intel_guild_id,
            &MemberSearchFilter {
                role_ids: vec!["keyset_role".to_string()],
                limit: Some(1),
                cursor: Some(cur2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(pg_page3.len(), 1);
    assert_eq!(pg_page3[0].user_id, pg_user_3);

    // 22. Test Cleanup Plans in PostgreSQL
    let cleanup_actor_1 = "444000000000000001";
    let cleanup_actor_2 = "444000000000000002";
    let cleanup_channel = "555000000000000001";
    let test_messages = vec![
        "666000000000000001".to_string(),
        "666000000000000002".to_string(),
    ];

    let plan = store
        .create_cleanup_plan(
            intel_guild_id,
            cleanup_channel,
            cleanup_actor_1,
            &test_messages,
            60,
        )
        .await
        .expect("create cleanup plan in postgres");
    assert_eq!(plan.guild_id, intel_guild_id);
    assert_eq!(plan.channel_id, cleanup_channel);
    assert_eq!(plan.actor_user_id, cleanup_actor_1);
    assert_eq!(plan.message_ids_vec(), test_messages);
    assert!(plan.consumed_at.is_none());

    // Cross-actor consumption fails
    let wrong_actor = store
        .consume_cleanup_plan(intel_guild_id, plan.id, cleanup_actor_2)
        .await
        .unwrap();
    assert!(wrong_actor.is_none(), "Wrong actor must be rejected");

    // Correct actor consumption succeeds
    let consumed = store
        .consume_cleanup_plan(intel_guild_id, plan.id, cleanup_actor_1)
        .await
        .unwrap()
        .expect("consumption should succeed");
    assert!(consumed.consumed_at.is_some());

    // Second consumption fails (atomic / idempotent)
    let second_consumed = store
        .consume_cleanup_plan(intel_guild_id, plan.id, cleanup_actor_1)
        .await
        .unwrap();
    assert!(
        second_consumed.is_none(),
        "Already consumed plan must not be consumed again"
    );

    // ==========================================
    // 12. Voice Companion Channels (#161)
    // ==========================================
    let companion_guild_id = "777000000000000010";
    let companion_voice_id = "888000000000000010";
    let companion_text_id = "888000000000000020";
    let companion_user_id = "999000000000000010";
    let companion_gen = Uuid::new_v4();

    // 12.1. Create mapping
    let mapping = store
        .upsert_companion_mapping(
            companion_guild_id,
            companion_voice_id,
            companion_text_id,
            true,
            true,
        )
        .await
        .expect("upsert companion mapping in postgres");
    assert_eq!(mapping.voice_channel_id, companion_voice_id);
    assert_eq!(mapping.text_channel_id, companion_text_id);
    assert!(mapping.enabled, "mapping must have enabled = true (#174)");

    // 12.2. Get by voice & list mappings
    let by_voice = store
        .get_companion_mapping_by_voice(companion_guild_id, companion_voice_id)
        .await
        .expect("get mapping by voice")
        .expect("mapping must exist");
    assert_eq!(by_voice.id, mapping.id);
    assert!(
        by_voice.enabled,
        "mapping by voice must have enabled = true (#174)"
    );

    let list_mappings = store
        .list_companion_mappings(companion_guild_id)
        .await
        .expect("list companion mappings");
    assert_eq!(list_mappings.len(), 1);
    assert!(list_mappings[0].enabled);

    // 12.2b. Test Mapping Destination Change V->A to V->B invalidates old lease (#175)
    let companion_text_id_2 = "888000000000000030";
    let lease_pre = store
        .record_companion_lease(rusty_discord_bot::storage::RecordCompanionLeaseParams {
            guild_id: companion_guild_id,
            voice_channel_id: companion_voice_id,
            text_channel_id: companion_text_id,
            user_id: companion_user_id,
            generation: companion_gen,
            allow_bits: 1024,
            deny_bits: 0,
        })
        .await
        .expect("record active lease on text A");
    assert_eq!(lease_pre.status, "active");

    let updated_mapping = store
        .upsert_companion_mapping(
            companion_guild_id,
            companion_voice_id,
            companion_text_id_2,
            true,
            true,
        )
        .await
        .expect("upsert updated text destination in postgres");
    assert_eq!(
        updated_mapping.id, mapping.id,
        "mapping ID must remain stable across updates"
    );
    assert_eq!(updated_mapping.text_channel_id, companion_text_id_2);
    assert!(updated_mapping.enabled);

    let old_lease = store
        .get_companion_lease(companion_guild_id, companion_text_id, companion_user_id)
        .await
        .expect("get old lease")
        .expect("old lease exists");
    assert_eq!(
        old_lease.status, "revoke_pending",
        "updating mapping target must mark old lease revoke_pending (#175)"
    );

    // Restore text destination back to companion_text_id for remaining test steps
    let restored_mapping = store
        .upsert_companion_mapping(
            companion_guild_id,
            companion_voice_id,
            companion_text_id,
            true,
            true,
        )
        .await
        .expect("restore mapping");
    assert_eq!(restored_mapping.id, mapping.id);

    // 12.3. Record lease
    let lease = store
        .record_companion_lease(rusty_discord_bot::storage::RecordCompanionLeaseParams {
            guild_id: companion_guild_id,
            voice_channel_id: companion_voice_id,
            text_channel_id: companion_text_id,
            user_id: companion_user_id,
            generation: companion_gen,
            allow_bits: 1024,
            deny_bits: 0,
        })
        .await
        .expect("record companion lease in postgres");
    assert_eq!(lease.generation, companion_gen);
    assert_eq!(lease.status, "active");
    assert_eq!(lease.allow_bits, 1024);

    // 12.3b. Outbox lifecycle: mark revoke_pending and record failure
    let pending = store
        .mark_companion_lease_revoke_pending(
            companion_guild_id,
            companion_text_id,
            companion_user_id,
        )
        .await
        .expect("mark revoke pending")
        .expect("lease exists");
    assert_eq!(pending.status, "revoke_pending");

    store
        .record_companion_lease_failure(
            companion_guild_id,
            companion_text_id,
            companion_user_id,
            "Discord Rate Limited 429",
        )
        .await
        .expect("record lease failure");

    let failed_lease = store
        .get_companion_lease(companion_guild_id, companion_text_id, companion_user_id)
        .await
        .expect("get lease")
        .expect("lease exists");
    assert_eq!(failed_lease.revoke_attempts, 1);
    assert!(failed_lease.last_error.unwrap().contains("Rate Limited"));

    // 12.4. Get lease
    let fetched_lease = store
        .get_companion_lease(companion_guild_id, companion_text_id, companion_user_id)
        .await
        .expect("get companion lease")
        .expect("lease must exist");
    assert_eq!(fetched_lease.voice_channel_id, companion_voice_id);

    // 12.5. List active leases vs pending revoke leases
    let leases = store
        .list_active_companion_leases(companion_guild_id)
        .await
        .expect("list active companion leases");
    assert_eq!(leases.len(), 0);

    let pending_leases = store
        .list_pending_revoke_companion_leases(companion_guild_id)
        .await
        .expect("list pending revoke leases");
    assert_eq!(pending_leases.len(), 1);

    // 12.5b. Targeted per-member lease retrieval (#261)
    let member_leases = store
        .list_member_companion_leases(companion_guild_id, companion_user_id)
        .await
        .expect("list member companion leases");
    assert_eq!(member_leases.len(), 1);
    assert_eq!(member_leases[0].user_id, companion_user_id);
    assert_eq!(member_leases[0].status, "revoke_pending");

    let empty_member_leases = store
        .list_member_companion_leases(companion_guild_id, "999000000000000099")
        .await
        .expect("list member companion leases for unknown user");
    assert!(empty_member_leases.is_empty());

    // 12.6. User erasure clears leases
    let erasure = store
        .erase_guild_user_data(companion_guild_id, companion_user_id)
        .await
        .expect("erasure should clear companion leases");
    assert!(erasure.related_records_erased >= 1);
    let leases_after_erasure = store
        .list_active_companion_leases(companion_guild_id)
        .await
        .expect("list after erasure");
    assert!(leases_after_erasure.is_empty());

    // 12.7. Delete mapping
    let deleted_m = store
        .delete_companion_mapping(companion_guild_id, mapping.id)
        .await
        .expect("delete companion mapping");
    assert!(deleted_m);
    assert!(
        store
            .list_companion_mappings(companion_guild_id)
            .await
            .expect("list mappings")
            .is_empty()
    );

    // ==========================================
    // 13. Member Intelligence Snapshot Departure (#167)
    // ==========================================
    let departure_user = "999000000000000099";
    let departure_guild = "777000000000000099";

    store
        .upsert_member_snapshot(rusty_discord_bot::storage::UpsertMemberSnapshotParams {
            guild_id: departure_guild,
            user_id: departure_user,
            username: "departing_member",
            global_name: Some("Departing"),
            nickname: None,
            role_ids: &[],
            joined_at: Some(Utc::now()),
        })
        .await
        .expect("upsert snapshot in postgres");

    let before_removal = store
        .search_members(
            departure_guild,
            &rusty_discord_bot::storage::MemberSearchFilter::default(),
        )
        .await
        .expect("search before departure");
    assert_eq!(before_removal.len(), 1);

    let removed = store
        .remove_member_snapshot(departure_guild, departure_user)
        .await
        .expect("remove snapshot in postgres");
    assert!(removed);

    let after_removal = store
        .search_members(
            departure_guild,
            &rusty_discord_bot::storage::MemberSearchFilter::default(),
        )
        .await
        .expect("search after departure");
    assert!(after_removal.is_empty());

    // Issue #171: Command Policy Engine PostgreSQL Lifecycle
    let policy_guild = "999888777000111222";
    let policy_user = "111222333444555666";
    let policy_channel = "333444555666777888";

    let init_rev = store
        .get_policy_revision(policy_guild)
        .await
        .expect("get initial policy revision");
    assert_eq!(init_rev, 1);

    let rule_user = rusty_discord_bot::storage::CommandPolicyRule::new_user_rule(
        policy_guild.to_string(),
        policy_user.to_string(),
        rusty_discord_bot::policy::PolicyResource::Command {
            module_id: "fun".to_string(),
            command_name: "meme".to_string(),
        },
        rusty_discord_bot::policy::PolicyEffect::Deny,
        5,
        "admin".to_string(),
    )
    .expect("construct user rule");

    let rule_channel = rusty_discord_bot::storage::CommandPolicyRule::new_channel_rule(
        policy_guild.to_string(),
        policy_channel.to_string(),
        rusty_discord_bot::policy::PolicyResource::Command {
            module_id: "fun".to_string(),
            command_name: "meme".to_string(),
        },
        rusty_discord_bot::policy::PolicyEffect::Allow,
        0,
        "admin".to_string(),
    )
    .expect("construct channel rule");

    let saved_u = store
        .upsert_policy_rule(rule_user.clone())
        .await
        .expect("upsert user rule");
    assert_eq!(saved_u.id, rule_user.id);

    let saved_c = store
        .upsert_policy_rule(rule_channel.clone())
        .await
        .expect("upsert channel rule");
    assert_eq!(saved_c.id, rule_channel.id);

    let rev_after_upsert = store
        .get_policy_revision(policy_guild)
        .await
        .expect("get revision after upsert");
    assert!(rev_after_upsert >= 2);

    let listed = store
        .list_policy_rules(policy_guild)
        .await
        .expect("list policy rules");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, rule_user.id);

    let deleted = store
        .delete_policy_rule(policy_guild, rule_user.id)
        .await
        .expect("delete policy rule");
    assert!(deleted);

    let reset_count = store
        .reset_policy_rules(policy_guild)
        .await
        .expect("reset policy rules");
    assert_eq!(reset_count, 1);

    let after_reset = store
        .list_policy_rules(policy_guild)
        .await
        .expect("list rules after reset");
    assert!(after_reset.is_empty());

    // Issue #176: Cross-guild UUID overwrite rejection in PostgreSQL
    let guild_other = "999888777666555444";
    let rule_guild1 = rusty_discord_bot::storage::CommandPolicyRule::new_user_rule(
        policy_guild.to_string(),
        policy_user.to_string(),
        rusty_discord_bot::policy::PolicyResource::Command {
            module_id: "music".to_string(),
            command_name: "play".to_string(),
        },
        rusty_discord_bot::policy::PolicyEffect::Deny,
        10,
        "admin".to_string(),
    )
    .expect("build rule_guild1");

    let saved_g1 = store
        .upsert_policy_rule(rule_guild1.clone())
        .await
        .expect("upsert rule_guild1");
    let rev_g1_before = store.get_policy_revision(policy_guild).await.unwrap();
    let rev_other_before = store.get_policy_revision(guild_other).await.unwrap();

    let mut malicious_other = rule_guild1.clone();
    malicious_other.guild_id = guild_other.to_string();
    malicious_other.effect = "allow".to_string();

    let cross_res = store.upsert_policy_rule(malicious_other).await;
    assert!(
        cross_res.is_err(),
        "cross-guild upsert must fail in PostgreSQL"
    );
    assert!(
        cross_res
            .unwrap_err()
            .to_string()
            .contains("belongs to another guild")
    );

    assert_eq!(
        store.get_policy_revision(policy_guild).await.unwrap(),
        rev_g1_before
    );
    assert_eq!(
        store.get_policy_revision(guild_other).await.unwrap(),
        rev_other_before
    );

    // Issue #178: Atomic Replace Policy Rules in PostgreSQL
    let mut invalid_rule = rule_guild1.clone();
    invalid_rule.id = uuid::Uuid::new_v4();
    invalid_rule.effect = "corrupt".to_string();

    let rep_err = store
        .replace_policy_rules(policy_guild, Some(rev_g1_before), vec![invalid_rule])
        .await;
    assert!(
        rep_err.is_err(),
        "replace with invalid rule must fail validation"
    );

    // Existing rules must still be intact
    let listed_intact = store.list_policy_rules(policy_guild).await.unwrap();
    assert_eq!(listed_intact.len(), 1);
    assert_eq!(listed_intact[0].id, saved_g1.id);

    // Stale expected_revision must fail with revision conflict
    let rep_conflict = store
        .replace_policy_rules(
            policy_guild,
            Some(rev_g1_before + 99),
            vec![rule_channel.clone()],
        )
        .await;
    assert!(rep_conflict.is_err());
    assert!(
        rep_conflict
            .unwrap_err()
            .to_string()
            .contains("Revision conflict")
    );

    // Successful replace
    let rep_ok = store
        .replace_policy_rules(
            policy_guild,
            Some(rev_g1_before),
            vec![rule_channel.clone()],
        )
        .await
        .expect("successful replace");
    assert_eq!(rep_ok.len(), 1);
    assert_eq!(rep_ok[0].id, rule_channel.id);
    assert_eq!(
        store.get_policy_revision(policy_guild).await.unwrap(),
        rev_g1_before + 1
    );

    // Clean up
    let _ = store.reset_policy_rules(policy_guild).await;

    // Issue #168: Moderator Duty PostgreSQL Lifecycle
    let duty_guild = "888777666555444333";
    let duty_user = "222333444555666777";
    let staff_role = "111000111000111000";
    let on_call_role = "222000222000222000";

    let duty_settings = rusty_discord_bot::storage::ModeratorDutySettings {
        guild_id: duty_guild.to_string(),
        staff_role_id: Some(staff_role.to_string()),
        on_call_role_id: Some(on_call_role.to_string()),
        max_override_duration_mins: 720,
        fallback_channel_id: None,
        updated_at: Utc::now(),
    };

    let saved_dsettings = store
        .upsert_duty_settings(duty_settings)
        .await
        .expect("upsert duty settings");
    assert_eq!(saved_dsettings.max_override_duration_mins, 720);

    let duty_profile = rusty_discord_bot::storage::ModeratorDutyProfile::new(
        duty_guild.to_string(),
        duty_user.to_string(),
        "Europe/Berlin".to_string(),
        rusty_discord_bot::storage::RecurringSchedule::default(),
    )
    .expect("construct duty profile");

    let saved_dprofile = store
        .upsert_duty_profile(duty_profile)
        .await
        .expect("upsert duty profile");
    assert_eq!(saved_dprofile.timezone, "Europe/Berlin");

    let duty_state = rusty_discord_bot::storage::ModeratorDutyState {
        guild_id: duty_guild.to_string(),
        user_id: duty_user.to_string(),
        expected_on_call: true,
        state_generation: 0,
        last_reconciled_at: Some(Utc::now()),
        last_error: None,
    };

    let saved_dstate = store
        .upsert_duty_state(duty_state)
        .await
        .expect("upsert duty state");
    assert!(saved_dstate.expected_on_call);

    let fetched_dstate = store
        .get_duty_state(duty_guild, duty_user)
        .await
        .expect("get duty state");
    assert!(fetched_dstate.unwrap().expected_on_call);

    let deleted_dprof = store
        .delete_duty_profile(duty_guild, duty_user)
        .await
        .expect("delete duty profile");
    assert!(deleted_dprof);

    let after_ddelete = store
        .get_duty_profile(duty_guild, duty_user)
        .await
        .expect("get duty profile after delete");
    assert!(after_ddelete.is_none());

    // Issue #166: Staff Recruitment Pipeline PostgreSQL Lifecycle
    let rec_guild = "666555444333222111";
    let rec_applicant = "111222333444555666";
    let rec_reviewer1 = "777888999000111222";
    let rec_reviewer2 = "888999000111222333";
    let rec_role = "333444555666777888";

    let track = store
        .upsert_recruitment_track(rusty_discord_bot::storage::RecruitmentTrack {
            id: uuid::Uuid::new_v4(),
            guild_id: rec_guild.to_string(),
            slug: "moderator".to_string(),
            name: "Moderator Applications".to_string(),
            enabled: true,
            target_role_id: Some(rec_role.to_string()),
            config: sqlx::types::Json(rusty_discord_bot::storage::RecruitmentTrackConfig {
                final_approvals: 2,
                ..Default::default()
            }),
            version: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .expect("upsert recruitment track in pg");
    assert_eq!(track.slug, "moderator");

    let app = store
        .create_recruitment_application(rusty_discord_bot::storage::RecruitmentApplication {
            id: uuid::Uuid::new_v4(),
            guild_id: rec_guild.to_string(),
            track_id: track.id,
            applicant_user_id: rec_applicant.to_string(),
            track_version: track.version,
            status: "submitted".to_string(),
            submitted_answers: sqlx::types::Json(vec![
                rusty_discord_bot::storage::SubmittedAnswer {
                    question_id: "q1".to_string(),
                    answer: "My experience".to_string(),
                },
            ]),
            current_stage: "submitted".to_string(),
            version: 1,
            submitted_at: Utc::now(),
            updated_at: Utc::now(),
            decided_at: None,
            decision_reason: None,
            decided_by: None,
        })
        .await
        .expect("create recruitment app in pg");
    assert_eq!(app.status, "submitted");

    let active_apps = store
        .count_active_user_applications(rec_guild, rec_applicant, track.id)
        .await
        .expect("count active apps");
    assert_eq!(active_apps, 1);

    let rev1 = store
        .upsert_recruitment_review(rusty_discord_bot::storage::RecruitmentReview {
            id: uuid::Uuid::new_v4(),
            application_id: app.id,
            guild_id: rec_guild.to_string(),
            reviewer_user_id: rec_reviewer1.to_string(),
            recommendation: "approve".to_string(),
            note: Some("Looks solid".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .expect("upsert review 1 in pg");
    assert_eq!(rev1.recommendation, "approve");

    let rev2 = store
        .upsert_recruitment_review(rusty_discord_bot::storage::RecruitmentReview {
            id: uuid::Uuid::new_v4(),
            application_id: app.id,
            guild_id: rec_guild.to_string(),
            reviewer_user_id: rec_reviewer2.to_string(),
            recommendation: "approve".to_string(),
            note: Some("Seconded".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .expect("upsert review 2 in pg");
    assert_eq!(rev2.recommendation, "approve");

    let reviews = store
        .list_recruitment_reviews(app.id)
        .await
        .expect("list reviews in pg");
    assert_eq!(reviews.len(), 2);

    // Stale version optimistic concurrency test
    let stale_trans = store
        .update_recruitment_application_transition(
            rusty_discord_bot::storage::TransitionApplicationParams {
                id: app.id,
                guild_id: rec_guild,
                expected_version: 999,
                new_status: "accepted",
                new_stage: "accepted",
                decided_by: Some(rec_reviewer1),
                decision_reason: Some("Accepted"),
            },
        )
        .await;
    assert!(stale_trans.is_err());
    assert!(
        stale_trans
            .unwrap_err()
            .to_string()
            .contains("Optimistic concurrency conflict")
    );

    // Successful transition
    let accepted_app = store
        .update_recruitment_application_transition(
            rusty_discord_bot::storage::TransitionApplicationParams {
                id: app.id,
                guild_id: rec_guild,
                expected_version: 1,
                new_status: "accepted",
                new_stage: "accepted",
                decided_by: Some(rec_reviewer1),
                decision_reason: Some("Accepted!"),
            },
        )
        .await
        .expect("transition app in pg");
    assert_eq!(accepted_app.status, "accepted");
    assert_eq!(accepted_app.version, 2);

    // Role fulfillment outbox
    let fulfillment = store
        .upsert_role_fulfillment(rusty_discord_bot::storage::RecruitmentRoleFulfillment {
            application_id: app.id,
            guild_id: rec_guild.to_string(),
            user_id: rec_applicant.to_string(),
            role_id: rec_role.to_string(),
            status: "pending".to_string(),
            attempts: 0,
            last_error: None,
            next_attempt_at: Utc::now(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .expect("upsert role fulfillment in pg");
    assert_eq!(fulfillment.status, "pending");

    let pending_fulfillments = store
        .list_pending_role_fulfillments(Some(rec_guild))
        .await
        .expect("list pending fulfillments in pg");
    assert!(
        pending_fulfillments
            .iter()
            .any(|f| f.application_id == app.id)
    );

    // Cleanup track and cascade
    let deleted_track = store
        .delete_recruitment_track(rec_guild, track.id)
        .await
        .expect("delete recruitment track in pg");
    assert!(deleted_track);

    let app_after_cascade = store
        .get_recruitment_application(rec_guild, app.id)
        .await
        .expect("get app after cascade");
    assert!(app_after_cascade.is_none());

    // Issue #164: Help Forum Lifecycle PostgreSQL Lifecycle
    let hf_guild = "777666555444333222";
    let hf_forum = "888777666555444333";
    let hf_thread = "999888777666555444";
    let hf_owner = "111222111222111222";
    let hf_helper = "333444333444333444";

    let hf_cfg = store
        .upsert_help_forum_config(rusty_discord_bot::storage::HelpForumConfig {
            guild_id: hf_guild.to_string(),
            forum_channel_id: hf_forum.to_string(),
            enabled: true,
            idle_close_minutes: 120,
            owner_can_close: true,
            helper_role_ids: sqlx::types::Json(vec![hf_helper.to_string()]),
            lock_on_close: true,
            archive_on_close: true,
            require_question_tag: false,
            welcome_message: Some("Need help? Ask here.".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .expect("upsert help forum config in pg");
    assert_eq!(hf_cfg.idle_close_minutes, 120);

    let hf_session = store
        .create_help_forum_session(rusty_discord_bot::storage::HelpForumSession {
            guild_id: hf_guild.to_string(),
            forum_channel_id: hf_forum.to_string(),
            thread_id: hf_thread.to_string(),
            owner_user_id: hf_owner.to_string(),
            status: "open".to_string(),
            opened_at: Utc::now(),
            last_activity_at: Utc::now(),
            first_helper_response_at: None,
            closed_at: None,
            closing_reason: None,
            generation: 1,
        })
        .await
        .expect("create help forum session in pg");
    assert_eq!(hf_session.status, "open");

    // Record activity as helper
    let updated_act = store
        .record_help_forum_activity(hf_guild, hf_thread, "888123456789012345", true)
        .await
        .expect("record helper activity in pg");
    assert!(updated_act.is_some());
    let act_sess = updated_act.unwrap();
    assert_eq!(act_sess.generation, 2);
    assert!(act_sess.first_helper_response_at.is_some());

    // Close session with stale generation check
    let stale_close = store
        .close_help_forum_session(hf_guild, hf_thread, "solved", Some("test"), Some(1))
        .await
        .expect("close with stale generation in pg");
    assert!(!stale_close);

    // Close session with matching generation
    let match_close = store
        .close_help_forum_session(
            hf_guild,
            hf_thread,
            "solved",
            Some("solved_by_user"),
            Some(2),
        )
        .await
        .expect("close with matching generation in pg");
    assert!(match_close);

    // Verify stats
    let hf_stats = store
        .get_help_forum_stats(hf_guild, hf_forum)
        .await
        .expect("get stats in pg");
    assert_eq!(hf_stats.opened_count, 1);
    assert_eq!(hf_stats.solved_count, 1);
    assert_eq!(hf_stats.answered_count, 1);

    // Cleanup
    let del_cfg = store
        .delete_help_forum_config(hf_guild, hf_forum)
        .await
        .expect("delete help forum config in pg");
    assert!(del_cfg);

    // 25. Game account links PostgreSQL schema and TTL parity (Issue #231)
    let gl_guild = "pg_game_link_guild_1";
    let gl_user = "pg_game_link_user_1";
    let gl_platform = "steam";
    let gl_link = store
        .create_game_link(
            gl_guild,
            gl_user,
            gl_platform,
            "steam_id_100",
            Some("code_100"),
        )
        .await
        .expect("create game link in pg");
    assert!(!gl_link.verified);
    assert_eq!(gl_link.verification_code.as_deref(), Some("code_100"));

    // Verify invalid code
    let wrong_ver = store
        .verify_game_link(gl_guild, gl_user, gl_platform, "wrong_code")
        .await
        .expect("verify game link with wrong code in pg");
    assert!(!wrong_ver);

    // Verify valid code executes SQL query with created_at > NOW() - INTERVAL '15 minutes'
    let ok_ver = store
        .verify_game_link(gl_guild, gl_user, gl_platform, "code_100")
        .await
        .expect("verify game link with correct code in pg");
    assert!(ok_ver);

    // Replay fails
    let replay_ver = store
        .verify_game_link(gl_guild, gl_user, gl_platform, "code_100")
        .await
        .expect("verify replay in pg");
    assert!(!replay_ver);
}
