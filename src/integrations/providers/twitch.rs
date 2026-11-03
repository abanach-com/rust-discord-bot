use crate::integrations::{CapabilityStatus, ProviderAdapter, ProviderCapability};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitchEventSubNotification {
    pub subscription: TwitchSubscriptionMetadata,
    pub event: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitchSubscriptionMetadata {
    pub id: String,
    pub status: String,
    pub r#type: String,
    pub version: String,
    pub condition: serde_json::Value,
}

pub struct TwitchExtendedAdapter;

impl ProviderAdapter for TwitchExtendedAdapter {
    fn provider_name(&self) -> &'static str {
        "twitch"
    }

    fn capabilities(&self) -> Vec<ProviderCapability> {
        vec![
            ProviderCapability {
                name: "stream_online".to_string(),
                description: "Live broadcast notifications with single-embed update".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "stream_offline".to_string(),
                description: "Stream end status and live role revocation".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "channel_update".to_string(),
                description: "Title and game category synchronization".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "chat_bridge".to_string(),
                description: "Two-way IRC/Chat relay with anti-loop filter".to_string(),
                status: CapabilityStatus::RequiresOAuth,
                read_only: false,
                requires_write_scope: true,
            },
        ]
    }
}
