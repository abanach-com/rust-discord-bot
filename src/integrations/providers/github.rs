use crate::integrations::{CapabilityStatus, ProviderAdapter, ProviderCapability};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubPushEvent {
    pub r#ref: String,
    pub before: String,
    pub after: String,
    pub repository: GitHubRepository,
    pub pusher: GitHubUser,
    pub commits: Vec<GitHubCommit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubPullRequestEvent {
    pub action: String,
    pub number: i32,
    pub pull_request: GitHubPullRequest,
    pub repository: GitHubRepository,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubRepository {
    pub id: u64,
    pub full_name: String,
    pub private: bool,
    pub html_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubUser {
    pub name: Option<String>,
    pub login: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubCommit {
    pub id: String,
    pub message: String,
    pub author: GitHubUser,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubPullRequest {
    pub title: String,
    pub state: String,
    pub html_url: String,
    pub merged: Option<bool>,
}

pub struct GitHubDevopsAdapter;

impl ProviderAdapter for GitHubDevopsAdapter {
    fn provider_name(&self) -> &'static str {
        "github"
    }

    fn capabilities(&self) -> Vec<ProviderCapability> {
        vec![
            ProviderCapability {
                name: "push".to_string(),
                description: "Repository commits and branch updates".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "pull_request".to_string(),
                description: "PR reviews, merges, and thread lifecycle".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "check_run".to_string(),
                description: "CI check status & workflow failure alerts".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "forum_thread_sync".to_string(),
                description: "Bidirectional mapping of Issues/PRs to Discord Forum Threads"
                    .to_string(),
                status: CapabilityStatus::Supported,
                read_only: false,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "write_actions".to_string(),
                description: "Create issues, re-run CI workflows, trigger deploys".to_string(),
                status: CapabilityStatus::RequiresOAuth,
                read_only: false,
                requires_write_scope: true,
            },
        ]
    }
}
