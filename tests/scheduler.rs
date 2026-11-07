use std::path::PathBuf;

use chrono::{Duration, Utc};
use rusty_discord_bot::{
    lua::{LuaAction, LuaEngine, LuaExecutionContext, LuaLimits, SchedulerOperation},
    scheduler::{
        CreateJobOutcome, JobMutation, JobMutationOutcome, NewScheduledJob, STATUS_ACTIVE,
        STATUS_COMPLETED, STATUS_FAILED, STATUS_PROCESSING, SchedulerStore,
    },
};
use serde_json::json;

fn limits() -> LuaLimits {
    LuaLimits {
        memory_bytes: 4 * 1024 * 1024,
        instruction_limit: 200_000,
        hook_granularity: 100,
    }
}

fn engine() -> LuaEngine {
    let scripts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts");
    LuaEngine::load(scripts, limits()).expect("bundled Lua modules should load")
}

fn lua_context(options: serde_json::Value) -> LuaExecutionContext {
    LuaExecutionContext {
        guild_id: Some("100000000000000001".to_owned()),
        channel_id: "100000000000000002".to_owned(),
        user_id: "100000000000000003".to_owned(),
        user_name: "Tester".to_owned(),
        member_roles: Vec::new(),
        member_permissions: "0".to_owned(),
        locale: "pl".to_owned(),
        options,
        config: json!({
            "user_reminders_enabled": true,
            "max_user_jobs": 10,
            "minimum_delay_seconds": 30,
            "maximum_delay_days": 365,
            "allow_user_mentions": true
        }),
    }
}

fn new_job(run_at: chrono::DateTime<Utc>) -> NewScheduledJob {
    NewScheduledJob {
        guild_id: "100000000000000001".to_owned(),
        module_id: "scheduler".to_owned(),
        channel_id: "100000000000000002".to_owned(),
        creator_user_id: "100000000000000003".to_owned(),
        content: "Test reminder".to_owned(),
        mention_creator: true,
        run_at,
        repeat_every_seconds: None,
        remaining_runs: Some(1),
        max_attempts: 5,
    }
}

async fn created_job(
    store: &SchedulerStore,
    job: NewScheduledJob,
    max_jobs: u16,
) -> rusty_discord_bot::scheduler::ScheduledJob {
    match store
        .create_job(job, max_jobs)
        .await
        .expect("memory scheduler create should succeed")
    {
        CreateJobOutcome::Created(job) => *job,
        CreateJobOutcome::LimitReached { limit } => {
            panic!("unexpected scheduler limit {limit}")
        }
    }
}

#[tokio::test]
async fn lua_reminder_emits_a_valid_scheduler_action() {
    let engine = engine();
    assert_eq!(
        engine.module_for_command("remind").await.as_deref(),
        Some("scheduler")
    );

    let actions = engine
        .execute_command(
            "scheduler",
            "remind",
            lua_context(json!({
                "when": "15m",
                "message": "Sprawdź wdrożenie",
                "mention": true
            })),
        )
        .await
        .expect("remind command should execute in the sandbox");

    assert!(matches!(
        actions.first(),
        Some(LuaAction::Scheduler {
            operation: SchedulerOperation::Create {
                when,
                mention_creator: true,
                ..
            }
        }) if when == "15m"
    ));
}

#[tokio::test]
async fn per_user_pending_job_limit_is_enforced() {
    let store = SchedulerStore::memory();
    let first = new_job(Utc::now() + Duration::minutes(5));
    created_job(&store, first.clone(), 1).await;

    let outcome = store
        .create_job(first, 1)
        .await
        .expect("memory scheduler create should succeed");
    assert!(matches!(
        outcome,
        CreateJobOutcome::LimitReached { limit: 1 }
    ));
}

#[tokio::test]
async fn lease_prevents_two_workers_from_claiming_the_same_job() {
    let store = SchedulerStore::memory();
    let now = Utc::now();
    let created = created_job(&store, new_job(now - Duration::seconds(1)), 10).await;

    let first = store
        .claim_due("worker-a", now, now - Duration::seconds(60), 25)
        .await
        .expect("first claim should succeed");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].id, created.id);
    assert_eq!(first[0].status, STATUS_PROCESSING);

    let second = store
        .claim_due("worker-b", now, now - Duration::seconds(60), 25)
        .await
        .expect("second claim should succeed");
    assert!(second.is_empty());
}

#[tokio::test]
async fn finite_recurring_job_reschedules_then_completes() {
    let store = SchedulerStore::memory();
    let now = Utc::now();
    let mut recurring = new_job(now - Duration::seconds(1));
    recurring.repeat_every_seconds = Some(60);
    recurring.remaining_runs = Some(2);
    let created = created_job(&store, recurring, 10).await;

    let first_claim = store
        .claim_due("worker-a", now, now - Duration::seconds(60), 1)
        .await
        .expect("claim should succeed");
    assert_eq!(first_claim.len(), 1);
    let after_first = store
        .mark_succeeded(&created.id, "worker-a", now)
        .await
        .expect("completion should succeed")
        .expect("worker should still own the lease");
    assert_eq!(after_first.status, STATUS_ACTIVE);
    assert_eq!(after_first.remaining_runs, Some(1));

    let second_now = after_first.run_at + Duration::seconds(1);
    let second_claim = store
        .claim_due(
            "worker-a",
            second_now,
            second_now - Duration::seconds(60),
            1,
        )
        .await
        .expect("second claim should succeed");
    assert_eq!(second_claim.len(), 1);
    let completed = store
        .mark_succeeded(&created.id, "worker-a", second_now)
        .await
        .expect("second completion should succeed")
        .expect("worker should own the second lease");
    assert_eq!(completed.status, STATUS_COMPLETED);
    assert_eq!(completed.run_count, 2);
}

#[tokio::test]
async fn non_owner_cannot_modify_another_users_job() {
    let store = SchedulerStore::memory();
    let created = created_job(&store, new_job(Utc::now() + Duration::minutes(5)), 10).await;

    let denied = store
        .mutate_job(
            &created.guild_id,
            &created.id,
            "100000000000000099",
            false,
            JobMutation::Cancel,
        )
        .await
        .expect("mutation should return an authorization outcome");
    assert!(matches!(denied, JobMutationOutcome::Forbidden));

    let allowed = store
        .mutate_job(
            &created.guild_id,
            &created.id,
            "100000000000000099",
            true,
            JobMutation::Pause,
        )
        .await
        .expect("staff mutation should succeed");
    assert!(matches!(allowed, JobMutationOutcome::Updated(_)));
}

#[tokio::test]
async fn lease_renewal_prevents_reclaim_during_long_processing() {
    let store = SchedulerStore::memory();
    let now = Utc::now();
    let created = created_job(&store, new_job(now - Duration::seconds(1)), 10).await;

    // Worker A claims the job at T=0
    let claimed = store
        .claim_due("worker-a", now, now - Duration::seconds(60), 1)
        .await
        .expect("worker-a claim should succeed");
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, created.id);

    // Processing takes 75 seconds total.
    // At T=35 seconds, worker-a renews the lease heartbeat
    let renewed = store
        .renew_lease(&created.id, "worker-a", now + Duration::seconds(35))
        .await
        .expect("renew_lease should succeed");
    assert!(renewed, "lease renewal by worker-a should return true");

    // At T=65 seconds (> 60s from initial claim), worker-b polls for due/stale jobs.
    // Stale threshold is (T=65) - 60s = T=5s.
    // Because locked_at is T=35s >= T=5s, worker-b CANNOT steal the job.
    let stale_before = (now + Duration::seconds(65)) - Duration::seconds(60);
    let worker_b_claim = store
        .claim_due("worker-b", now + Duration::seconds(65), stale_before, 10)
        .await
        .expect("worker-b claim should succeed");
    assert!(
        worker_b_claim.is_empty(),
        "active job with renewed lease should not be reclaimed by worker-b"
    );

    // At T=75 seconds, worker-a completes delivery and marks the job succeeded
    let completed = store
        .mark_succeeded(&created.id, "worker-a", now + Duration::seconds(75))
        .await
        .expect("mark_succeeded should succeed")
        .expect("worker-a should still own the lease");
    assert_eq!(completed.status, STATUS_COMPLETED);
    assert_eq!(completed.run_count, 1);
}

#[tokio::test]
async fn worker_termination_allows_lease_recovery_after_expiry() {
    let store = SchedulerStore::memory();
    let now = Utc::now();
    let created = created_job(&store, new_job(now - Duration::seconds(1)), 10).await;

    // Worker A claims the job at T=0 and abruptly terminates (does not renew or complete)
    let claimed = store
        .claim_due("worker-a", now, now - Duration::seconds(60), 1)
        .await
        .expect("worker-a claim should succeed");
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].attempts, 1);

    // At T=65 seconds (> 60s lease window), worker-b polls for stale jobs
    let stale_before = (now + Duration::seconds(65)) - Duration::seconds(60);
    let recovered = store
        .claim_due("worker-b", now + Duration::seconds(65), stale_before, 10)
        .await
        .expect("worker-b claim should succeed");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].id, created.id);
    assert_eq!(recovered[0].locked_by.as_deref(), Some("worker-b"));
    assert_eq!(recovered[0].attempts, 2);

    // Worker B completes the job
    let completed = store
        .mark_succeeded(&created.id, "worker-b", now + Duration::seconds(66))
        .await
        .expect("mark_succeeded should succeed")
        .expect("worker-b owns the recovered lease");
    assert_eq!(completed.status, STATUS_COMPLETED);
}

#[tokio::test]
async fn renew_lease_rejects_unauthorized_worker_or_inactive_job() {
    let store = SchedulerStore::memory();
    let now = Utc::now();
    let created = created_job(&store, new_job(now - Duration::seconds(1)), 10).await;

    // Job is still active, not processing: cannot renew
    let renewed_before_claim = store
        .renew_lease(&created.id, "worker-a", now)
        .await
        .expect("renew_lease call should succeed");
    assert!(!renewed_before_claim);

    // Worker A claims the job
    let _ = store
        .claim_due("worker-a", now, now - Duration::seconds(60), 1)
        .await
        .expect("claim should succeed");

    // Worker B tries to renew worker A's lease: rejected
    let foreign_renewal = store
        .renew_lease(&created.id, "worker-b", now + Duration::seconds(10))
        .await
        .expect("renew_lease call should succeed");
    assert!(!foreign_renewal);

    // Worker A completes the job
    let _ = store
        .mark_succeeded(&created.id, "worker-a", now + Duration::seconds(20))
        .await
        .expect("mark_succeeded should succeed");

    // Worker A tries to renew completed job: rejected
    let completed_renewal = store
        .renew_lease(&created.id, "worker-a", now + Duration::seconds(30))
        .await
        .expect("renew_lease call should succeed");
    assert!(!completed_renewal);
}

#[tokio::test]
async fn expired_lease_with_exhausted_attempts_marks_job_failed_and_frees_quota() {
    let store = SchedulerStore::memory();
    let now = Utc::now();
    let mut job = new_job(now - Duration::seconds(1));
    job.max_attempts = 1;
    let created = created_job(&store, job, 1).await;

    // Worker A claims the job (attempt 1 of 1)
    let claimed = store
        .claim_due("worker-a", now, now - Duration::seconds(60), 1)
        .await
        .expect("claim should succeed");
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].attempts, 1);

    // User cannot create a second job because limit is 1 and job is in processing
    let mut job2 = new_job(now + Duration::seconds(100));
    job2.max_attempts = 1;
    let outcome = store
        .create_job(job2.clone(), 1)
        .await
        .expect("create_job should succeed");
    assert!(matches!(outcome, CreateJobOutcome::LimitReached { .. }));

    // Time passes past lease (65s > 60s lease window)
    let stale_before = (now + Duration::seconds(65)) - Duration::seconds(60);
    // Worker B calls claim_due
    let claimed_b = store
        .claim_due("worker-b", now + Duration::seconds(65), stale_before, 10)
        .await
        .expect("claim_due should succeed");
    assert!(
        claimed_b.is_empty(),
        "exhausted attempts should not be reclaimed as active"
    );

    // The job was marked as failed in the store
    let failed_job = store
        .get_job(&created.id)
        .await
        .expect("get_job should succeed")
        .expect("job should exist");
    assert_eq!(failed_job.status, STATUS_FAILED);
    assert!(
        failed_job
            .last_error
            .as_deref()
            .unwrap_or("")
            .contains("expired")
    );

    // User limit is now freed, job2 can be created!
    let outcome2 = store
        .create_job(job2, 1)
        .await
        .expect("create_job should succeed");
    assert!(matches!(outcome2, CreateJobOutcome::Created(_)));
}

#[tokio::test]
async fn multi_guild_polling_is_not_blocked_by_expired_job_in_another_guild() {
    let store = SchedulerStore::memory();
    let now = Utc::now();

    // Guild 1: Job that is processing and will expire with exhausted attempts
    let mut guild1_job = new_job(now - Duration::seconds(10));
    guild1_job.guild_id = "guild-100".to_owned();
    guild1_job.max_attempts = 1;
    let created_g1 = created_job(&store, guild1_job, 10).await;

    // Worker claims Guild 1 job at T=0
    let claimed_g1 = store
        .claim_due("worker-1", now, now - Duration::seconds(60), 1)
        .await
        .expect("worker-1 claim should succeed");
    assert_eq!(claimed_g1.len(), 1);
    assert_eq!(claimed_g1[0].id, created_g1.id);

    // Guild 2: Job that is due
    let mut guild2_job = new_job(now - Duration::seconds(5));
    guild2_job.guild_id = "guild-200".to_owned();
    guild2_job.creator_user_id = "user-200".to_owned();
    let created_g2 = created_job(&store, guild2_job, 10).await;

    // Time passes past lease threshold (65s > 60s)
    let claim_time = now + Duration::seconds(65);
    let stale_before = claim_time - Duration::seconds(60);

    // Worker polls for due jobs across guilds
    let claimed = store
        .claim_due("worker-2", claim_time, stale_before, 10)
        .await
        .expect("claim_due should succeed");

    // Guild 1 job expired and failed; Guild 2 job was claimed without blocking
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, created_g2.id);
    assert_eq!(claimed[0].guild_id, "guild-200");

    let g1_after = store.get_job(&created_g1.id).await.unwrap().unwrap();
    assert_eq!(g1_after.status, STATUS_FAILED);
}

#[tokio::test]
async fn completed_jobs_with_completed_at_are_not_claimable_or_resurrected() {
    let store = SchedulerStore::memory();
    let now = Utc::now();
    let created = created_job(&store, new_job(now - Duration::seconds(10)), 10).await;

    // Claim and complete the job
    let claimed = store
        .claim_due("worker-1", now, now - Duration::seconds(60), 1)
        .await
        .expect("claim should succeed");
    assert_eq!(claimed.len(), 1);

    let completed = store
        .mark_succeeded(&created.id, "worker-1", now)
        .await
        .expect("mark_succeeded should succeed")
        .expect("job exists");
    assert_eq!(completed.status, STATUS_COMPLETED);
    assert!(completed.completed_at.is_some());

    // Advance time further into the future
    let future_now = now + Duration::minutes(30);
    let stale_before = future_now - Duration::seconds(60);

    // Ensure completed job is not claimable
    assert!(!completed.is_claimable(future_now, stale_before));

    // Polling must not resurrect or return the completed job
    let claimed_future = store
        .claim_due("worker-2", future_now, stale_before, 10)
        .await
        .expect("claim_due should succeed");
    assert!(claimed_future.is_empty());

    // Fetch from store and verify it is still completed
    let job_in_store = store.get_job(&created.id).await.unwrap().unwrap();
    assert_eq!(job_in_store.status, STATUS_COMPLETED);
    assert!(job_in_store.completed_at.is_some());
}

#[tokio::test]
async fn test_scheduler_finalize_expired_leases_metrics_and_quota_freeing() {
    let metrics = std::sync::Arc::new(rusty_discord_bot::metrics::Metrics::new());
    let store = SchedulerStore::memory().with_metrics(metrics.clone());
    let now = Utc::now();

    // Create a job with max_attempts = 1 and max user jobs limit = 1
    let mut job = new_job(now - Duration::seconds(10));
    job.max_attempts = 1;
    let created = created_job(&store, job.clone(), 1).await;

    // Worker claims the job at T=0
    let claimed = store
        .claim_due("worker-1", now, now - Duration::seconds(60), 1)
        .await
        .expect("claim should succeed");
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, created.id);
    assert_eq!(claimed[0].status, STATUS_PROCESSING);

    // While in processing state, quota limit is reached (pending count = 1 >= max_jobs = 1)
    let second_job = new_job(now - Duration::seconds(5));
    let outcome = store
        .create_job(second_job.clone(), 1)
        .await
        .expect("create_job check");
    assert!(matches!(
        outcome,
        CreateJobOutcome::LimitReached { limit: 1 }
    ));

    // Advance time past lease window (70s > 60s)
    let claim_time = now + Duration::seconds(70);
    let stale_before = claim_time - Duration::seconds(60);

    // Finalize expired leases
    let terminalized = store
        .finalize_expired_leases(claim_time, stale_before)
        .await
        .expect("finalize_expired_leases should succeed");
    assert_eq!(terminalized, 1);

    // Verify job transitioned to failed and released worker lease
    let job_after = store.get_job(&created.id).await.unwrap().unwrap();
    assert_eq!(job_after.status, STATUS_FAILED);
    assert_eq!(
        job_after.last_error.as_deref(),
        Some("worker lease expired after reaching maximum attempts")
    );
    assert!(job_after.completed_at.is_some());
    assert!(job_after.locked_by.is_none());
    assert!(job_after.locked_at.is_none());

    // Verify metrics telemetry
    assert_eq!(
        metrics
            .scheduler_expired_lease_terminalized_total
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        metrics
            .scheduler_jobs_failed_total
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );

    // Verify user quota is unblocked: user can now create a new job
    let second_outcome = store
        .create_job(second_job, 1)
        .await
        .expect("create_job after terminalization");
    assert!(matches!(second_outcome, CreateJobOutcome::Created(_)));
}

#[tokio::test]
async fn test_scheduler_expired_lease_with_remaining_attempts_is_not_terminalized() {
    let metrics = std::sync::Arc::new(rusty_discord_bot::metrics::Metrics::new());
    let store = SchedulerStore::memory().with_metrics(metrics.clone());
    let now = Utc::now();

    // Create a job with max_attempts = 3
    let mut job = new_job(now - Duration::seconds(10));
    job.max_attempts = 3;
    let created = created_job(&store, job, 10).await;

    // Worker claims attempt 1
    let claimed = store
        .claim_due("worker-1", now, now - Duration::seconds(60), 1)
        .await
        .expect("claim should succeed");
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].attempts, 1);

    // Lease expires (locked_at < stale_before), but attempts (1) < max_attempts (3)
    let claim_time = now + Duration::seconds(70);
    let stale_before = claim_time - Duration::seconds(60);

    let terminalized = store
        .finalize_expired_leases(claim_time, stale_before)
        .await
        .expect("finalize_expired_leases");
    assert_eq!(
        terminalized, 0,
        "must not terminalize jobs with attempts < max_attempts"
    );

    // Job remains processing / claimable for retry
    let reclaimed = store
        .claim_due("worker-2", claim_time, stale_before, 1)
        .await
        .expect("claim_due should reclaim job for retry");
    assert_eq!(reclaimed.len(), 1);
    assert_eq!(reclaimed[0].id, created.id);
    assert_eq!(reclaimed[0].attempts, 2);
    assert_eq!(reclaimed[0].locked_by.as_deref(), Some("worker-2"));
}
