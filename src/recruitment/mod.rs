use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::storage::validate_snowflake;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionType {
    ShortText,
    Paragraph,
    Select,
    Checkbox,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecruitmentQuestion {
    pub id: String,
    pub label: String,
    pub field_type: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmittedAnswer {
    pub question_id: String,
    pub answer: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecruitmentTrackConfig {
    #[serde(default)]
    pub questions: Vec<RecruitmentQuestion>,
    #[serde(default)]
    pub review_role_ids: Vec<String>,
    #[serde(default = "default_max_active")]
    pub max_active_per_user: u32,
    #[serde(default = "default_cooldown_days")]
    pub cooldown_days: u32,
    #[serde(default)]
    pub require_interview: bool,
    #[serde(default = "default_final_approvals")]
    pub final_approvals: u32,
    #[serde(default = "default_true")]
    pub auto_role_on_accept: bool,
}

fn default_max_active() -> u32 {
    1
}

fn default_cooldown_days() -> u32 {
    30
}

fn default_final_approvals() -> u32 {
    1
}

fn default_true() -> bool {
    true
}

impl Default for RecruitmentTrackConfig {
    fn default() -> Self {
        Self {
            questions: Vec::new(),
            review_role_ids: Vec::new(),
            max_active_per_user: default_max_active(),
            cooldown_days: default_cooldown_days(),
            require_interview: false,
            final_approvals: default_final_approvals(),
            auto_role_on_accept: default_true(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct RecruitmentTrack {
    pub id: Uuid,
    pub guild_id: String,
    pub slug: String,
    pub name: String,
    pub enabled: bool,
    pub target_role_id: Option<String>,
    pub config: sqlx::types::Json<RecruitmentTrackConfig>,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct RecruitmentApplication {
    pub id: Uuid,
    pub guild_id: String,
    pub track_id: Uuid,
    pub applicant_user_id: String,
    pub track_version: i64,
    pub status: String,
    pub submitted_answers: sqlx::types::Json<Vec<SubmittedAnswer>>,
    pub current_stage: String,
    pub version: i64,
    pub submitted_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub decision_reason: Option<String>,
    pub decided_by: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct RecruitmentReview {
    pub id: Uuid,
    pub application_id: Uuid,
    pub guild_id: String,
    pub reviewer_user_id: String,
    pub recommendation: String,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct RecruitmentRoleFulfillment {
    pub application_id: Uuid,
    pub guild_id: String,
    pub user_id: String,
    pub role_id: String,
    pub status: String,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub next_attempt_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Candidate-facing view strictly stripped of staff-only notes, reviewer identity, and internal reviews.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandidateApplicationView {
    pub id: Uuid,
    pub guild_id: String,
    pub track_id: Uuid,
    pub applicant_user_id: String,
    pub status: String,
    pub current_stage: String,
    pub submitted_answers: Vec<SubmittedAnswer>,
    pub submitted_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub decision_reason: Option<String>,
}

/// Staff-facing view including full review history, reviewer notes, and fulfillment status.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StaffApplicationView {
    pub id: Uuid,
    pub guild_id: String,
    pub track_id: Uuid,
    pub applicant_user_id: String,
    pub track_version: i64,
    pub status: String,
    pub current_stage: String,
    pub submitted_answers: Vec<SubmittedAnswer>,
    pub version: i64,
    pub submitted_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub decision_reason: Option<String>,
    pub decided_by: Option<String>,
    pub reviews: Vec<RecruitmentReview>,
    pub fulfillment: Option<RecruitmentRoleFulfillment>,
}

#[derive(Clone, Copy, Debug)]
pub struct TransitionApplicationParams<'a> {
    pub id: Uuid,
    pub guild_id: &'a str,
    pub expected_version: i64,
    pub new_status: &'a str,
    pub new_stage: &'a str,
    pub decided_by: Option<&'a str>,
    pub decision_reason: Option<&'a str>,
}

impl RecruitmentApplication {
    pub fn to_candidate_view(&self) -> CandidateApplicationView {
        CandidateApplicationView {
            id: self.id,
            guild_id: self.guild_id.clone(),
            track_id: self.track_id,
            applicant_user_id: self.applicant_user_id.clone(),
            status: self.status.clone(),
            current_stage: self.current_stage.clone(),
            submitted_answers: self.submitted_answers.0.clone(),
            submitted_at: self.submitted_at,
            updated_at: self.updated_at,
            decided_at: self.decided_at,
            decision_reason: self.decision_reason.clone(),
        }
    }

    pub fn to_staff_view(
        &self,
        reviews: Vec<RecruitmentReview>,
        fulfillment: Option<RecruitmentRoleFulfillment>,
    ) -> StaffApplicationView {
        StaffApplicationView {
            id: self.id,
            guild_id: self.guild_id.clone(),
            track_id: self.track_id,
            applicant_user_id: self.applicant_user_id.clone(),
            track_version: self.track_version,
            status: self.status.clone(),
            current_stage: self.current_stage.clone(),
            submitted_answers: self.submitted_answers.0.clone(),
            version: self.version,
            submitted_at: self.submitted_at,
            updated_at: self.updated_at,
            decided_at: self.decided_at,
            decision_reason: self.decision_reason.clone(),
            decided_by: self.decided_by.clone(),
            reviews,
            fulfillment,
        }
    }
}

pub fn validate_submitted_answers(
    questions: &[RecruitmentQuestion],
    answers: &[SubmittedAnswer],
) -> anyhow::Result<()> {
    for q in questions {
        let answer = answers.iter().find(|a| a.question_id == q.id);
        if q.required {
            let Some(ans) = answer else {
                anyhow::bail!("Missing required answer for question '{}'", q.label);
            };
            if ans.answer.trim().is_empty() {
                anyhow::bail!("Answer for required question '{}' cannot be empty", q.label);
            }
        }

        if let Some(ans) = answer
            && q.field_type == "select"
            && !ans.answer.trim().is_empty()
            && !q.options.is_empty()
            && !q.options.contains(&ans.answer)
        {
            anyhow::bail!("Invalid option '{}' for question '{}'", ans.answer, q.label);
        }
    }
    Ok(())
}

pub fn count_distinct_approvals(reviews: &[RecruitmentReview]) -> usize {
    reviews
        .iter()
        .filter(|r| r.recommendation == "approve")
        .map(|r| &r.reviewer_user_id)
        .collect::<std::collections::HashSet<_>>()
        .len()
}

pub fn validate_track_fields(
    guild_id: &str,
    slug: &str,
    name: &str,
    target_role_id: Option<&str>,
) -> anyhow::Result<()> {
    validate_snowflake("guild_id", guild_id)?;
    if slug.trim().is_empty() || slug.len() > 64 {
        anyhow::bail!("Track slug must be between 1 and 64 characters");
    }
    if !slug
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        anyhow::bail!(
            "Track slug must contain only alphanumeric characters, dashes, or underscores"
        );
    }
    if name.trim().is_empty() || name.len() > 100 {
        anyhow::bail!("Track name must be between 1 and 100 characters");
    }
    if let Some(rid) = target_role_id {
        validate_snowflake("target_role_id", rid)?;
    }
    Ok(())
}

pub async fn process_recruitment_role_fulfillments(
    storage: &crate::storage::Storage,
    http: Option<&serenity::http::Http>,
) -> anyhow::Result<usize> {
    let pending = storage.list_pending_role_fulfillments().await?;
    let mut processed = 0;
    for mut item in pending {
        processed += 1;
        item.attempts += 1;
        item.updated_at = Utc::now();

        if let Some(client) = http {
            let g_res = item
                .guild_id
                .parse::<u64>()
                .map(serenity::all::GuildId::new);
            let u_res = item.user_id.parse::<u64>().map(serenity::all::UserId::new);
            let r_res = item.role_id.parse::<u64>().map(serenity::all::RoleId::new);

            match (g_res, u_res, r_res) {
                (Ok(g), Ok(u), Ok(r)) => {
                    match client
                        .add_member_role(g, u, r, Some("Staff recruitment accepted"))
                        .await
                    {
                        Ok(_) => {
                            item.status = "completed".to_string();
                            item.last_error = None;
                        }
                        Err(e) => {
                            let err_msg = e.to_string();
                            item.last_error = Some(err_msg);
                            if item.attempts >= 5 {
                                item.status = "failed".to_string();
                            } else {
                                let backoff_secs = 2u64.pow(item.attempts.min(6) as u32) * 5;
                                item.next_attempt_at =
                                    Utc::now() + chrono::Duration::seconds(backoff_secs as i64);
                            }
                        }
                    }
                }
                _ => {
                    item.status = "failed".to_string();
                    item.last_error =
                        Some("Invalid snowflake ID in fulfillment record".to_string());
                }
            }
        } else {
            // Dry run / offline mode
            item.status = "completed".to_string();
            item.last_error = None;
        }

        let _ = storage.upsert_role_fulfillment(item).await;
    }
    Ok(processed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_submitted_answers() {
        let questions = vec![
            RecruitmentQuestion {
                id: "q1".to_string(),
                label: "Why join?".to_string(),
                field_type: "paragraph".to_string(),
                required: true,
                options: Vec::new(),
            },
            RecruitmentQuestion {
                id: "q2".to_string(),
                label: "Timezone".to_string(),
                field_type: "select".to_string(),
                required: false,
                options: vec!["UTC".to_string(), "CET".to_string()],
            },
        ];

        // Missing required
        let empty_answers = vec![];
        assert!(validate_submitted_answers(&questions, &empty_answers).is_err());

        // Valid answers
        let valid_answers = vec![
            SubmittedAnswer {
                question_id: "q1".to_string(),
                answer: "I want to help the community".to_string(),
            },
            SubmittedAnswer {
                question_id: "q2".to_string(),
                answer: "CET".to_string(),
            },
        ];
        assert!(validate_submitted_answers(&questions, &valid_answers).is_ok());

        // Invalid select option
        let invalid_select = vec![
            SubmittedAnswer {
                question_id: "q1".to_string(),
                answer: "Help".to_string(),
            },
            SubmittedAnswer {
                question_id: "q2".to_string(),
                answer: "EST".to_string(),
            },
        ];
        assert!(validate_submitted_answers(&questions, &invalid_select).is_err());
    }

    #[test]
    fn test_candidate_view_masks_staff_notes() {
        let app = RecruitmentApplication {
            id: Uuid::new_v4(),
            guild_id: "100000000000000001".to_string(),
            track_id: Uuid::new_v4(),
            applicant_user_id: "200000000000000001".to_string(),
            track_version: 1,
            status: "screening".to_string(),
            submitted_answers: sqlx::types::Json(vec![]),
            current_stage: "screening".to_string(),
            version: 1,
            submitted_at: Utc::now(),
            updated_at: Utc::now(),
            decided_at: None,
            decision_reason: None,
            decided_by: Some("staff_admin".to_string()),
        };

        let view = app.to_candidate_view();
        assert_eq!(view.id, app.id);
        assert_eq!(view.status, "screening");
        let serialized = serde_json::to_string(&view).unwrap();
        assert!(!serialized.contains("staff_admin"));
        assert!(!serialized.contains("reviews"));
    }

    #[test]
    fn test_distinct_approvals_counting() {
        let app_id = Uuid::new_v4();
        let reviews = vec![
            RecruitmentReview {
                id: Uuid::new_v4(),
                application_id: app_id,
                guild_id: "100000000000000001".to_string(),
                reviewer_user_id: "rev_1".to_string(),
                recommendation: "approve".to_string(),
                note: Some("Looks good".to_string()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            RecruitmentReview {
                id: Uuid::new_v4(),
                application_id: app_id,
                guild_id: "100000000000000001".to_string(),
                reviewer_user_id: "rev_1".to_string(), // Duplicate reviewer ID
                recommendation: "approve".to_string(),
                note: Some("Updated note".to_string()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            RecruitmentReview {
                id: Uuid::new_v4(),
                application_id: app_id,
                guild_id: "100000000000000001".to_string(),
                reviewer_user_id: "rev_2".to_string(),
                recommendation: "approve".to_string(),
                note: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            RecruitmentReview {
                id: Uuid::new_v4(),
                application_id: app_id,
                guild_id: "100000000000000001".to_string(),
                reviewer_user_id: "rev_3".to_string(),
                recommendation: "reject".to_string(),
                note: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        ];

        // 2 distinct approvals (rev_1, rev_2)
        assert_eq!(count_distinct_approvals(&reviews), 2);
    }
}
