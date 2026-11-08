use chrono::{DateTime, Utc};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
pub struct Metrics {
    pub process_start_time: DateTime<Utc>,
    pub discord_events_total: AtomicU64,
    pub lua_commands_total: AtomicU64,
    pub lua_command_errors_total: AtomicU64,
    pub scheduler_jobs_claimed_total: AtomicU64,
    pub scheduler_jobs_delivered_total: AtomicU64,
    pub scheduler_jobs_failed_total: AtomicU64,
    pub scheduler_jobs_cancelled_total: AtomicU64,
    pub dashboard_requests_total: AtomicU64,
    pub rate_limit_throttled_total: AtomicU64,
    pub message_updates_unknown_author_total: AtomicU64,
    pub lua_events_overloaded_total: AtomicU64,
    pub lua_commands_overloaded_total: AtomicU64,
    pub scheduler_expired_lease_cleanup_errors_total: AtomicU64,
    pub scheduler_expired_lease_terminalized_total: AtomicU64,
    pub economy_idempotency_pruned_total: AtomicU64,
    pub discord_rest_requests_total: AtomicU64,
    pub afk_notifications_suppressed_total: AtomicU64,
    pub ai_metering_pruned_total: AtomicU64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            process_start_time: Utc::now(),
            discord_events_total: AtomicU64::new(0),
            lua_commands_total: AtomicU64::new(0),
            lua_command_errors_total: AtomicU64::new(0),
            scheduler_jobs_claimed_total: AtomicU64::new(0),
            scheduler_jobs_delivered_total: AtomicU64::new(0),
            scheduler_jobs_failed_total: AtomicU64::new(0),
            scheduler_jobs_cancelled_total: AtomicU64::new(0),
            dashboard_requests_total: AtomicU64::new(0),
            rate_limit_throttled_total: AtomicU64::new(0),
            message_updates_unknown_author_total: AtomicU64::new(0),
            lua_events_overloaded_total: AtomicU64::new(0),
            lua_commands_overloaded_total: AtomicU64::new(0),
            scheduler_expired_lease_cleanup_errors_total: AtomicU64::new(0),
            scheduler_expired_lease_terminalized_total: AtomicU64::new(0),
            economy_idempotency_pruned_total: AtomicU64::new(0),
            discord_rest_requests_total: AtomicU64::new(0),
            afk_notifications_suppressed_total: AtomicU64::new(0),
            ai_metering_pruned_total: AtomicU64::new(0),
        }
    }
}

impl Metrics {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn uptime_seconds(&self) -> i64 {
        (Utc::now() - self.process_start_time).num_seconds().max(0)
    }

    pub fn render_prometheus(
        &self,
        active_sessions: usize,
        active_lua_executions: usize,
    ) -> String {
        format!(
            "# HELP rusty_discord_bot_uptime_seconds Process uptime in seconds\n\
             # TYPE rusty_discord_bot_uptime_seconds gauge\n\
             rusty_discord_bot_uptime_seconds {0}\n\
             zuckerbot_uptime_seconds {0}\n\
             # HELP rusty_discord_bot_discord_events_total Total number of received Discord gateway events\n\
             # TYPE rusty_discord_bot_discord_events_total counter\n\
             rusty_discord_bot_discord_events_total {1}\n\
             zuckerbot_discord_events_total {1}\n\
             # HELP rusty_discord_bot_lua_commands_total Total number of Lua commands executed\n\
             # TYPE rusty_discord_bot_lua_commands_total counter\n\
             rusty_discord_bot_lua_commands_total {2}\n\
             zuckerbot_lua_commands_total {2}\n\
             # HELP rusty_discord_bot_lua_command_errors_total Total number of failed Lua commands\n\
             # TYPE rusty_discord_bot_lua_command_errors_total counter\n\
             rusty_discord_bot_lua_command_errors_total {3}\n\
             zuckerbot_lua_command_errors_total {3}\n\
             # HELP rusty_discord_bot_scheduler_claimed_total Total number of scheduler jobs claimed\n\
             # TYPE rusty_discord_bot_scheduler_claimed_total counter\n\
             rusty_discord_bot_scheduler_claimed_total {4}\n\
             zuckerbot_scheduler_claimed_total {4}\n\
             # HELP rusty_discord_bot_scheduler_delivered_total Total number of successfully delivered scheduled jobs\n\
             # TYPE rusty_discord_bot_scheduler_delivered_total counter\n\
             rusty_discord_bot_scheduler_delivered_total {5}\n\
             zuckerbot_scheduler_delivered_total {5}\n\
             # HELP rusty_discord_bot_scheduler_failed_total Total number of failed scheduled jobs\n\
             # TYPE rusty_discord_bot_scheduler_failed_total counter\n\
             rusty_discord_bot_scheduler_failed_total {6}\n\
             zuckerbot_scheduler_failed_total {6}\n\
             # HELP rusty_discord_bot_dashboard_requests_total Total number of HTTP requests processed by dashboard\n\
             # TYPE rusty_discord_bot_dashboard_requests_total counter\n\
             rusty_discord_bot_dashboard_requests_total {7}\n\
             zuckerbot_dashboard_requests_total {7}\n\
             # HELP rusty_discord_bot_rate_limit_throttled_total Total number of operations rejected by abuse prevention / rate limiter\n\
             # TYPE rusty_discord_bot_rate_limit_throttled_total counter\n\
             rusty_discord_bot_rate_limit_throttled_total {8}\n\
             zuckerbot_rate_limit_throttled_total {8}\n\
             # HELP rusty_discord_bot_active_sessions Current number of active dashboard sessions\n\
             # TYPE rusty_discord_bot_active_sessions gauge\n\
             rusty_discord_bot_active_sessions {9}\n\
             zuckerbot_active_sessions {9}\n\
             # HELP rusty_discord_bot_message_updates_unknown_author_total Total number of message_update events skipped due to missing/unknown author\n\
             # TYPE rusty_discord_bot_message_updates_unknown_author_total counter\n\
             rusty_discord_bot_message_updates_unknown_author_total {10}\n\
             zuckerbot_message_updates_unknown_author_total {10}\n\
             # HELP rusty_discord_bot_lua_active_executions Current number of concurrently running Lua VM tasks\n\
             # TYPE rusty_discord_bot_lua_active_executions gauge\n\
             rusty_discord_bot_lua_active_executions {11}\n\
             zuckerbot_lua_active_executions {11}\n\
             # HELP rusty_discord_bot_lua_events_overloaded_total Total number of Lua events dropped due to capacity exhaustion\n\
             # TYPE rusty_discord_bot_lua_events_overloaded_total counter\n\
             rusty_discord_bot_lua_events_overloaded_total {12}\n\
             zuckerbot_lua_events_overloaded_total {12}\n\
             # HELP rusty_discord_bot_lua_commands_overloaded_total Total number of Lua commands rejected due to capacity exhaustion\n\
             # TYPE rusty_discord_bot_lua_commands_overloaded_total counter\n\
             rusty_discord_bot_lua_commands_overloaded_total {13}\n\
             zuckerbot_lua_commands_overloaded_total {13}\n\
             # HELP rusty_discord_bot_scheduler_expired_lease_cleanup_errors_total Total number of database errors encountered when attempting to terminalize expired scheduler leases\n\
             # TYPE rusty_discord_bot_scheduler_expired_lease_cleanup_errors_total counter\n\
             rusty_discord_bot_scheduler_expired_lease_cleanup_errors_total {14}\n\
             zuckerbot_scheduler_expired_lease_cleanup_errors_total {14}\n\
             # HELP rusty_discord_bot_scheduler_expired_lease_terminalized_total Total number of expired processing leases terminalized to failed after exceeding max attempts\n\
             # TYPE rusty_discord_bot_scheduler_expired_lease_terminalized_total counter\n\
             rusty_discord_bot_scheduler_expired_lease_terminalized_total {15}\n\
             zuckerbot_scheduler_expired_lease_terminalized_total {15}\n\
             # HELP rusty_discord_bot_economy_idempotency_pruned_total Total number of expired economy purchase requests and idempotency keys pruned\n\
             # TYPE rusty_discord_bot_economy_idempotency_pruned_total counter\n\
             rusty_discord_bot_economy_idempotency_pruned_total {16}\n\
             zuckerbot_economy_idempotency_pruned_total {16}\n\
             # HELP rusty_discord_bot_discord_rest_requests_total Total number of outbound Discord REST API HTTP requests dispatched\n\
             # TYPE rusty_discord_bot_discord_rest_requests_total counter\n\
             rusty_discord_bot_discord_rest_requests_total {17}\n\
             zuckerbot_discord_rest_requests_total {17}\n\
             # HELP rusty_discord_bot_afk_notifications_suppressed_total Total number of AFK mention notifications suppressed by rate limiter\n\
             # TYPE rusty_discord_bot_afk_notifications_suppressed_total counter\n\
             rusty_discord_bot_afk_notifications_suppressed_total {18}\n\
             zuckerbot_afk_notifications_suppressed_total {18}\n\
             # HELP rusty_discord_bot_ai_metering_pruned_total Total number of expired AI metering events pruned\n\
             # TYPE rusty_discord_bot_ai_metering_pruned_total counter\n\
             rusty_discord_bot_ai_metering_pruned_total {19}\n\
             zuckerbot_ai_metering_pruned_total {19}\n",
            self.uptime_seconds(),
            self.discord_events_total.load(Ordering::Relaxed),
            self.lua_commands_total.load(Ordering::Relaxed),
            self.lua_command_errors_total.load(Ordering::Relaxed),
            self.scheduler_jobs_claimed_total.load(Ordering::Relaxed),
            self.scheduler_jobs_delivered_total.load(Ordering::Relaxed),
            self.scheduler_jobs_failed_total.load(Ordering::Relaxed),
            self.dashboard_requests_total.load(Ordering::Relaxed),
            self.rate_limit_throttled_total.load(Ordering::Relaxed),
            active_sessions,
            self.message_updates_unknown_author_total
                .load(Ordering::Relaxed),
            active_lua_executions,
            self.lua_events_overloaded_total.load(Ordering::Relaxed),
            self.lua_commands_overloaded_total.load(Ordering::Relaxed),
            self.scheduler_expired_lease_cleanup_errors_total
                .load(Ordering::Relaxed),
            self.scheduler_expired_lease_terminalized_total
                .load(Ordering::Relaxed),
            self.economy_idempotency_pruned_total
                .load(Ordering::Relaxed),
            self.discord_rest_requests_total.load(Ordering::Relaxed),
            self.afk_notifications_suppressed_total
                .load(Ordering::Relaxed),
            self.ai_metering_pruned_total.load(Ordering::Relaxed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_prometheus_rendering() {
        let metrics = Metrics::new();
        metrics.discord_events_total.store(15, Ordering::Relaxed);
        metrics.lua_commands_total.store(7, Ordering::Relaxed);
        metrics
            .message_updates_unknown_author_total
            .store(4, Ordering::Relaxed);
        metrics
            .lua_events_overloaded_total
            .store(2, Ordering::Relaxed);
        metrics
            .lua_commands_overloaded_total
            .store(1, Ordering::Relaxed);
        metrics
            .scheduler_expired_lease_cleanup_errors_total
            .store(3, Ordering::Relaxed);
        metrics
            .scheduler_expired_lease_terminalized_total
            .store(8, Ordering::Relaxed);
        metrics
            .economy_idempotency_pruned_total
            .store(42, Ordering::Relaxed);
        metrics
            .discord_rest_requests_total
            .store(13, Ordering::Relaxed);
        metrics
            .afk_notifications_suppressed_total
            .store(5, Ordering::Relaxed);
        metrics
            .ai_metering_pruned_total
            .store(11, Ordering::Relaxed);

        let rendered = metrics.render_prometheus(3, 2);
        assert!(rendered.contains("zuckerbot_discord_events_total 15"));
        assert!(rendered.contains("zuckerbot_lua_commands_total 7"));
        assert!(rendered.contains("zuckerbot_message_updates_unknown_author_total 4"));
        assert!(rendered.contains("zuckerbot_lua_active_executions 2"));
        assert!(rendered.contains("zuckerbot_lua_events_overloaded_total 2"));
        assert!(rendered.contains("zuckerbot_lua_commands_overloaded_total 1"));
        assert!(rendered.contains("zuckerbot_scheduler_expired_lease_cleanup_errors_total 3"));
        assert!(rendered.contains("zuckerbot_scheduler_expired_lease_terminalized_total 8"));
        assert!(rendered.contains("zuckerbot_economy_idempotency_pruned_total 42"));
        assert!(rendered.contains("rusty_discord_bot_economy_idempotency_pruned_total 42"));
        assert!(rendered.contains("zuckerbot_discord_rest_requests_total 13"));
        assert!(rendered.contains("rusty_discord_bot_discord_rest_requests_total 13"));
        assert!(rendered.contains("zuckerbot_afk_notifications_suppressed_total 5"));
        assert!(rendered.contains("rusty_discord_bot_afk_notifications_suppressed_total 5"));
        assert!(rendered.contains("zuckerbot_ai_metering_pruned_total 11"));
        assert!(rendered.contains("rusty_discord_bot_ai_metering_pruned_total 11"));
        assert!(rendered.contains("zuckerbot_active_sessions 3"));
        assert!(rendered.contains("zuckerbot_uptime_seconds"));
    }
}
