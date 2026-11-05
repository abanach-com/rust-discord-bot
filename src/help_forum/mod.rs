use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::storage::validate_snowflake;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct HelpForumConfig {
    pub guild_id: String,
    pub forum_channel_id: String,
    pub enabled: bool,
    pub idle_close_minutes: i32,
    pub owner_can_close: bool,
    pub helper_role_ids: sqlx::types::Json<Vec<String>>,
    pub lock_on_close: bool,
    pub archive_on_close: bool,
    pub require_question_tag: bool,
    pub welcome_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Default for HelpForumConfig {
    fn default() -> Self {
        Self {
            guild_id: String::new(),
            forum_channel_id: String::new(),
            enabled: true,
            idle_close_minutes: 1440,
            owner_can_close: true,
            helper_role_ids: sqlx::types::Json(Vec::new()),
            lock_on_close: true,
            archive_on_close: true,
            require_question_tag: false,
            welcome_message: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct HelpForumSession {
    pub guild_id: String,
    pub forum_channel_id: String,
    pub thread_id: String,
    pub owner_user_id: String,
    pub status: String, // 'open', 'solved', 'inactive', 'deleted', 'owner_left'
    pub opened_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub first_helper_response_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub closing_reason: Option<String>,
    pub generation: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HelpForumStats {
    pub forum_channel_id: String,
    pub opened_count: i64,
    pub open_active_count: i64,
    pub solved_count: i64,
    pub inactive_count: i64,
    pub answered_count: i64,
    pub unanswered_count: i64,
    pub median_first_response_secs: Option<f64>,
    pub median_lifetime_secs: Option<f64>,
}

pub fn validate_help_forum_config(config: &HelpForumConfig) -> anyhow::Result<()> {
    validate_snowflake("guild_id", &config.guild_id)?;
    validate_snowflake("forum_channel_id", &config.forum_channel_id)?;

    if config.idle_close_minutes < 1 || config.idle_close_minutes > 43200 {
        anyhow::bail!("idle_close_minutes must be between 1 and 43200 (30 days)");
    }

    for role_id in &config.helper_role_ids.0 {
        validate_snowflake("helper_role_id", role_id)?;
    }

    if let Some(msg) = &config.welcome_message
        && msg.len() > 2000
    {
        anyhow::bail!("welcome_message cannot exceed 2000 characters");
    }

    Ok(())
}

pub fn can_close_session(
    session: &HelpForumSession,
    config: &HelpForumConfig,
    user_id: &str,
    user_role_ids: &[String],
    is_admin: bool,
) -> bool {
    if is_admin {
        return true;
    }

    if user_id == session.owner_user_id && config.owner_can_close {
        return true;
    }

    config
        .helper_role_ids
        .0
        .iter()
        .any(|hr| user_role_ids.iter().any(|ur| ur == hr))
}

pub fn calculate_stats_from_sessions(
    forum_channel_id: &str,
    sessions: &[HelpForumSession],
) -> HelpForumStats {
    let mut opened_count = 0;
    let mut open_active_count = 0;
    let mut solved_count = 0;
    let mut inactive_count = 0;
    let mut answered_count = 0;
    let mut unanswered_count = 0;

    let mut response_durations: Vec<f64> = Vec::new();
    let mut lifetimes: Vec<f64> = Vec::new();

    for s in sessions {
        opened_count += 1;
        match s.status.as_str() {
            "open" => open_active_count += 1,
            "solved" => solved_count += 1,
            "inactive" => inactive_count += 1,
            _ => {}
        }

        if let Some(first_resp) = s.first_helper_response_at {
            answered_count += 1;
            let diff = (first_resp - s.opened_at).num_seconds();
            if diff >= 0 {
                response_durations.push(diff as f64);
            }
        } else {
            unanswered_count += 1;
        }

        if let Some(closed) = s.closed_at {
            let diff = (closed - s.opened_at).num_seconds();
            if diff >= 0 {
                lifetimes.push(diff as f64);
            }
        }
    }

    response_durations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    lifetimes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let median_first_response_secs = if response_durations.is_empty() {
        None
    } else {
        let mid = response_durations.len() / 2;
        if response_durations.len().is_multiple_of(2) {
            Some((response_durations[mid - 1] + response_durations[mid]) / 2.0)
        } else {
            Some(response_durations[mid])
        }
    };

    let median_lifetime_secs = if lifetimes.is_empty() {
        None
    } else {
        let mid = lifetimes.len() / 2;
        if lifetimes.len().is_multiple_of(2) {
            Some((lifetimes[mid - 1] + lifetimes[mid]) / 2.0)
        } else {
            Some(lifetimes[mid])
        }
    };

    HelpForumStats {
        forum_channel_id: forum_channel_id.to_string(),
        opened_count,
        open_active_count,
        solved_count,
        inactive_count,
        answered_count,
        unanswered_count,
        median_first_response_secs,
        median_lifetime_secs,
    }
}

pub async fn reconcile_idle_help_forums(
    storage: &crate::storage::Storage,
    http: Option<&serenity::http::Http>,
    guild_id: &str,
) -> anyhow::Result<usize> {
    validate_snowflake("guild_id", guild_id)?;
    let configs = storage.list_help_forum_configs(guild_id).await?;
    let mut closed_count = 0;
    let now = Utc::now();

    for cfg in configs {
        if !cfg.enabled {
            continue;
        }

        let sessions = storage
            .list_help_forum_sessions(guild_id, Some(&cfg.forum_channel_id), Some("open"))
            .await?;

        for session in sessions {
            let idle_duration = Duration::minutes(cfg.idle_close_minutes as i64);
            if now - session.last_activity_at >= idle_duration {
                // If live Discord HTTP client available, archive and optionally lock thread
                if let (Some(client), Ok(tid_num)) = (http, session.thread_id.parse::<u64>()) {
                    let chan_id = serenity::all::ChannelId::new(tid_num);
                    let mut map = serde_json::Map::new();
                    if cfg.archive_on_close {
                        map.insert("archived".to_string(), serde_json::Value::Bool(true));
                    }
                    if cfg.lock_on_close {
                        map.insert("locked".to_string(), serde_json::Value::Bool(true));
                    }

                    let _ = client
                        .edit_channel(
                            chan_id,
                            &serde_json::Value::Object(map),
                            Some("Help forum inactivity auto-close"),
                        )
                        .await;
                }

                let closed = storage
                    .close_help_forum_session(
                        guild_id,
                        &session.thread_id,
                        "inactive",
                        Some("inactivity_auto_close"),
                        Some(session.generation),
                    )
                    .await?;

                if closed {
                    closed_count += 1;
                }
            }
        }
    }

    Ok(closed_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_can_close_session_permissions() {
        let session = HelpForumSession {
            guild_id: "100000000000000001".to_string(),
            forum_channel_id: "200000000000000001".to_string(),
            thread_id: "300000000000000001".to_string(),
            owner_user_id: "400000000000000001".to_string(),
            status: "open".to_string(),
            opened_at: Utc::now(),
            last_activity_at: Utc::now(),
            first_helper_response_at: None,
            closed_at: None,
            closing_reason: None,
            generation: 1,
        };

        let mut config = HelpForumConfig {
            guild_id: "100000000000000001".to_string(),
            forum_channel_id: "200000000000000001".to_string(),
            enabled: true,
            idle_close_minutes: 1440,
            owner_can_close: true,
            helper_role_ids: sqlx::types::Json(vec!["500000000000000001".to_string()]),
            lock_on_close: true,
            archive_on_close: true,
            require_question_tag: false,
            welcome_message: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        // 1. Owner can close when owner_can_close is true
        assert!(can_close_session(
            &session,
            &config,
            "400000000000000001",
            &[],
            false
        ));

        // 2. Owner cannot close if owner_can_close is false
        config.owner_can_close = false;
        assert!(!can_close_session(
            &session,
            &config,
            "400000000000000001",
            &[],
            false
        ));

        // 3. User with helper role can close
        assert!(can_close_session(
            &session,
            &config,
            "999000000000000001",
            &["500000000000000001".to_string()],
            false
        ));

        // 4. Random user without helper role cannot close
        assert!(!can_close_session(
            &session,
            &config,
            "999000000000000001",
            &["999000000000000002".to_string()],
            false
        ));

        // 5. Administrator can always close
        assert!(can_close_session(
            &session,
            &config,
            "999000000000000001",
            &[],
            true
        ));
    }

    #[test]
    fn test_calculate_stats_from_sessions() {
        let now = Utc::now();
        let s1 = HelpForumSession {
            guild_id: "100000000000000001".to_string(),
            forum_channel_id: "200000000000000001".to_string(),
            thread_id: "1".to_string(),
            owner_user_id: "user1".to_string(),
            status: "solved".to_string(),
            opened_at: now - Duration::seconds(600),
            last_activity_at: now,
            first_helper_response_at: Some(now - Duration::seconds(540)), // 60s first response
            closed_at: Some(now),                                         // 600s lifetime
            closing_reason: Some("solved".to_string()),
            generation: 1,
        };

        let s2 = HelpForumSession {
            guild_id: "100000000000000001".to_string(),
            forum_channel_id: "200000000000000001".to_string(),
            thread_id: "2".to_string(),
            owner_user_id: "user2".to_string(),
            status: "inactive".to_string(),
            opened_at: now - Duration::seconds(1200),
            last_activity_at: now - Duration::seconds(100),
            first_helper_response_at: Some(now - Duration::seconds(1080)), // 120s first response
            closed_at: Some(now),                                          // 1200s lifetime
            closing_reason: Some("inactive".to_string()),
            generation: 1,
        };

        let s3 = HelpForumSession {
            guild_id: "100000000000000001".to_string(),
            forum_channel_id: "200000000000000001".to_string(),
            thread_id: "3".to_string(),
            owner_user_id: "user3".to_string(),
            status: "open".to_string(),
            opened_at: now - Duration::seconds(300),
            last_activity_at: now - Duration::seconds(50),
            first_helper_response_at: None, // unanswered
            closed_at: None,
            closing_reason: None,
            generation: 1,
        };

        let stats = calculate_stats_from_sessions("200000000000000001", &[s1, s2, s3]);
        assert_eq!(stats.opened_count, 3);
        assert_eq!(stats.open_active_count, 1);
        assert_eq!(stats.solved_count, 1);
        assert_eq!(stats.inactive_count, 1);
        assert_eq!(stats.answered_count, 2);
        assert_eq!(stats.unanswered_count, 1);
        assert_eq!(stats.median_first_response_secs, Some(90.0)); // (60 + 120) / 2
        assert_eq!(stats.median_lifetime_secs, Some(900.0)); // (600 + 1200) / 2
    }
}
