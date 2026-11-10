use rusty_discord_bot::rate_limit::{RateLimitQuota, RateLimiter};
use std::time::Duration;

#[test]
fn test_rate_limiter_burst_and_refill() {
    let limiter = RateLimiter::new();
    // 3 requests allowed per 1-second window
    let quota = RateLimitQuota::new(3, 1);
    let key = "test_user_key";

    // 1st request -> allowed, 2 remaining
    let d1 = limiter.check_rate_limit(key, quota);
    assert!(d1.allowed);
    assert_eq!(d1.remaining, 2);
    assert_eq!(d1.limit, 3);

    // 2nd request -> allowed, 1 remaining
    let d2 = limiter.check_rate_limit(key, quota);
    assert!(d2.allowed);
    assert_eq!(d2.remaining, 1);

    // 3rd request -> allowed, 0 remaining
    let d3 = limiter.check_rate_limit(key, quota);
    assert!(d3.allowed);
    assert_eq!(d3.remaining, 0);

    // 4th request -> blocked (rate limit exceeded), reset_after > 0
    let d4 = limiter.check_rate_limit(key, quota);
    assert!(!d4.allowed);
    assert_eq!(d4.remaining, 0);
    assert!(d4.reset_after > Duration::ZERO);

    // Sleep until window expires
    std::thread::sleep(Duration::from_millis(1100));

    // 5th request -> allowed again after window slides!
    let d5 = limiter.check_rate_limit(key, quota);
    assert!(d5.allowed);
    assert_eq!(d5.remaining, 2);
}

#[test]
fn test_default_policies_and_custom_override() {
    let limiter = RateLimiter::new();

    // Default policy for ticket_create is 3 per 300s
    let q_ticket = limiter.quota_for("ticket_create");
    assert_eq!(q_ticket.max_requests, 3);
    assert_eq!(q_ticket.window_seconds, 300);

    // Default policy for music_play is 10 per 60s
    let q_music = limiter.quota_for("music_play");
    assert_eq!(q_music.max_requests, 10);
    assert_eq!(q_music.window_seconds, 60);

    // Default policy for destructive admin is conservative: 5 per 60s
    let q_admin = limiter.quota_for("destructive_admin");
    assert_eq!(q_admin.max_requests, 5);
    assert_eq!(q_admin.window_seconds, 60);

    // Custom policy override
    limiter.set_quota("ticket_create", RateLimitQuota::new(1, 10));
    let q_ticket_new = limiter.quota_for("ticket_create");
    assert_eq!(q_ticket_new.max_requests, 1);
    assert_eq!(q_ticket_new.window_seconds, 10);

    let d1 = limiter.check_action("ticket_create", "user_1", Some("guild_1"));
    assert!(d1.allowed);
    let d2 = limiter.check_action("ticket_create", "user_1", Some("guild_1"));
    assert!(!d2.allowed);
}

#[test]
fn test_per_user_and_per_guild_command_limits() {
    let limiter = RateLimiter::new();
    limiter.set_quota("command_user", RateLimitQuota::new(2, 60));
    limiter.set_quota("command_guild", RateLimitQuota::new(3, 60));

    let user_a = "user_a";
    let user_b = "user_b";
    let guild = "guild_shared";

    // User A executes 2 commands -> reaches per-user limit
    assert!(limiter.check_command("ping", user_a, Some(guild)).allowed);
    assert!(limiter.check_command("ping", user_a, Some(guild)).allowed);
    assert!(!limiter.check_command("ping", user_a, Some(guild)).allowed);

    // User B in same guild still has quota (1 remaining for guild)
    assert!(limiter.check_command("ping", user_b, Some(guild)).allowed);

    // Now guild limit (3) is reached!
    assert!(!limiter.check_command("ping", user_b, Some(guild)).allowed);
}

#[test]
fn test_stale_history_cleanup() {
    let limiter = RateLimiter::new();
    limiter.check_rate_limit("temp_key_short", RateLimitQuota::new(5, 60));
    limiter.check_rate_limit("temp_key_long", RateLimitQuota::new(5, 7200));

    assert_eq!(limiter.keys_count(), 2);

    // Virtual clock advanced past the max window (2 hours)
    let future = std::time::Instant::now() + Duration::from_secs(8000);
    limiter.cleanup_stale_entries_at(future);

    // All expired keys and empty entries are purged from history
    assert_eq!(limiter.keys_count(), 0);
}

#[test]
fn test_concurrent_simultaneous_requests_with_limit_one() {
    use std::sync::{Arc, Barrier};
    use std::thread;

    let limiter = Arc::new(RateLimiter::new());
    limiter.set_quota("command_user", RateLimitQuota::new(1, 60));
    limiter.set_quota("command_guild", RateLimitQuota::new(1, 60));

    let threads = 20;
    let barrier = Arc::new(Barrier::new(threads));
    let mut handles = Vec::new();

    for _ in 0..threads {
        let lim = Arc::clone(&limiter);
        let bar = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            bar.wait();
            lim.check_command("test_cmd", "concurrent_user", Some("concurrent_guild"))
        }));
    }

    let mut allowed_count = 0;
    let mut denied_count = 0;

    for handle in handles {
        let decision = handle.join().expect("thread join must succeed");
        if decision.allowed {
            allowed_count += 1;
        } else {
            denied_count += 1;
        }
    }

    // Exactly 1 allowed, 19 rejected when limit is 1!
    assert_eq!(
        allowed_count, 1,
        "exactly 1 concurrent request should succeed under limit 1"
    );
    assert_eq!(
        denied_count,
        threads - 1,
        "all other concurrent requests should be rejected"
    );
}

#[test]
fn test_composite_reservations_cannot_deadlock() {
    use std::sync::{Arc, Barrier};
    use std::thread;

    let limiter = Arc::new(RateLimiter::new());
    // Sequential loop over 1,000 distinct user/guild combinations
    for i in 0..1000 {
        let user = format!("u_{i}");
        let guild = format!("g_{i}");
        let res = limiter.check_command("cmd", &user, Some(&guild));
        assert!(res.allowed);
    }

    // Multithreaded concurrent reservations across interleaved users & guilds
    let thread_count = 16;
    let barrier = Arc::new(Barrier::new(thread_count));
    let mut handles = Vec::new();

    for t in 0..thread_count {
        let lim = Arc::clone(&limiter);
        let bar = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            bar.wait();
            for i in 0..100 {
                let user = format!("user_{}_{}", t, i % 5);
                let guild = format!("guild_{}", i % 3);
                lim.check_command("ping", &user, Some(&guild));
            }
        }));
    }

    for handle in handles {
        handle.join().expect("thread join must not deadlock");
    }
}

#[test]
fn test_multi_rate_limit_duplicate_identical_keys_consumes_single_slot_issue_370() {
    let limiter = RateLimiter::new();
    let quota = RateLimitQuota::new(2, 60);

    // Call 1: two identical keys in a single multi-check
    let first = limiter.check_multi_rate_limit(&[("same_key", quota), ("same_key", quota)]);
    assert!(first.allowed);
    assert_eq!(
        first.remaining, 1,
        "single call with duplicated identical key must consume exactly 1 token"
    );

    // Call 2: another call with two identical keys
    let second = limiter.check_multi_rate_limit(&[("same_key", quota), ("same_key", quota)]);
    assert!(second.allowed);
    assert_eq!(
        second.remaining, 0,
        "second call should consume the remaining token"
    );

    // Call 3: should be denied as 2 requests in 60s were made
    let third = limiter.check_multi_rate_limit(&[("same_key", quota), ("same_key", quota)]);
    assert!(!third.allowed);
    assert_eq!(third.remaining, 0);
}

#[test]
fn test_multi_rate_limit_duplicate_keys_with_limit_one_never_overflows_issue_370() {
    let limiter = RateLimiter::new();
    let quota = RateLimitQuota::new(1, 60);

    let first = limiter.check_multi_rate_limit(&[("single_slot", quota), ("single_slot", quota)]);
    assert!(first.allowed);
    assert_eq!(first.remaining, 0);

    let second = limiter.check_multi_rate_limit(&[("single_slot", quota), ("single_slot", quota)]);
    assert!(!second.allowed);
}

#[test]
fn test_multi_rate_limit_distinct_keys_atomic_all_or_nothing() {
    let limiter = RateLimiter::new();
    let quota_low = RateLimitQuota::new(1, 60);
    let quota_high = RateLimitQuota::new(5, 60);

    // 1st request succeeds for both keys
    let first = limiter.check_multi_rate_limit(&[("chan_1", quota_low), ("author_1", quota_high)]);
    assert!(first.allowed);
    assert_eq!(first.remaining, 0);

    // 2nd request fails because chan_1 is exhausted
    let second = limiter.check_multi_rate_limit(&[("chan_1", quota_low), ("author_1", quota_high)]);
    assert!(!second.allowed);

    // author_1 must have only consumed 1 token, remaining 4
    let author_only = limiter.check_rate_limit("author_1", quota_high);
    assert!(author_only.allowed);
    assert_eq!(
        author_only.remaining, 3,
        "failed multi-check must not consume tokens for other keys"
    );
}

#[test]
fn test_multi_rate_limit_conflicting_quotas_rejected_deterministically_issue_370() {
    let limiter = RateLimiter::new();
    let quota_a = RateLimitQuota::new(2, 60);
    let quota_b = RateLimitQuota::new(10, 5);

    let res =
        limiter.check_multi_rate_limit(&[("conflict_key", quota_a), ("conflict_key", quota_b)]);
    assert!(
        !res.allowed,
        "conflicting quotas for the same key must be rejected"
    );
    assert_eq!(res.remaining, 0);

    // Key history must not be polluted by the rejected call
    let clean_res = limiter.check_rate_limit("conflict_key", quota_a);
    assert!(clean_res.allowed);
    assert_eq!(clean_res.remaining, 1);
}

#[test]
fn test_multi_rate_limit_concurrent_duplicate_keys_limit_one_issue_370() {
    use std::sync::{Arc, Barrier};
    use std::thread;

    let limiter = Arc::new(RateLimiter::new());
    let quota = RateLimitQuota::new(1, 60);

    let threads = 20;
    let barrier = Arc::new(Barrier::new(threads));
    let mut handles = Vec::new();

    for _ in 0..threads {
        let lim = Arc::clone(&limiter);
        let bar = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            bar.wait();
            lim.check_multi_rate_limit(&[("concurrent_dup", quota), ("concurrent_dup", quota)])
        }));
    }

    let mut allowed_count = 0;
    let mut denied_count = 0;

    for handle in handles {
        let decision = handle.join().expect("thread join must succeed");
        if decision.allowed {
            allowed_count += 1;
        } else {
            denied_count += 1;
        }
    }

    assert_eq!(
        allowed_count, 1,
        "under concurrency with limit 1, exactly 1 multi-check must succeed"
    );
    assert_eq!(denied_count, threads - 1);
}
