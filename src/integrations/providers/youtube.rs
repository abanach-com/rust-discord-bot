use crate::integrations::{CapabilityStatus, ProviderAdapter, ProviderCapability};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YouTubeVideoNotification {
    pub video_id: String,
    pub channel_id: String,
    pub title: String,
    pub published_at: String,
    pub is_short_heuristic: bool,
    pub is_premiere: bool,
}

pub struct YouTubeExtendedAdapter;

impl ProviderAdapter for YouTubeExtendedAdapter {
    fn provider_name(&self) -> &'static str {
        "youtube"
    }

    fn capabilities(&self) -> Vec<ProviderCapability> {
        vec![
            ProviderCapability {
                name: "websub_feed".to_string(),
                description: "Real-time push notifications via PubSubHubbub / WebSub".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "video_upload".to_string(),
                description: "New video notifications without expensive search polling".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "shorts_detection".to_string(),
                description: "Best-effort heuristic categorization for Shorts".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "comments_moderation".to_string(),
                description: "Channel comments read & moderation actions".to_string(),
                status: CapabilityStatus::RequiresOAuth,
                read_only: false,
                requires_write_scope: true,
            },
        ]
    }
}
