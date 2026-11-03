use crate::integrations::{CapabilityStatus, ProviderAdapter, ProviderCapability};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedditPostData {
    pub id: String,
    pub name: String, // e.g. "t3_xxxxxx"
    pub title: String,
    pub author: String,
    pub subreddit: String,
    pub score: i32,
    pub link_flair_text: Option<String>,
    pub over_18: bool,
    pub url: String,
}

pub struct RedditAdapter;

impl ProviderAdapter for RedditAdapter {
    fn provider_name(&self) -> &'static str {
        "reddit"
    }

    fn capabilities(&self) -> Vec<ProviderCapability> {
        vec![
            ProviderCapability {
                name: "subreddit_feed".to_string(),
                description: "Subreddit polling with cursor pagination & backfill limits"
                    .to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "flair_filter".to_string(),
                description: "Filter posts by flair and minimum score".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "moderator_actions".to_string(),
                description: "Moderator tools with dry-run verification".to_string(),
                status: CapabilityStatus::RequiresOAuth,
                read_only: false,
                requires_write_scope: true,
            },
        ]
    }
}
