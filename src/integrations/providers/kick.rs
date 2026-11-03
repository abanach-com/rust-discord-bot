use crate::integrations::{CapabilityStatus, ProviderAdapter, ProviderCapability};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KickLivestreamEvent {
    pub livestream: KickLivestreamData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KickLivestreamData {
    pub id: u64,
    pub channel_id: u64,
    pub session_title: String,
    pub created_at: String,
    pub is_live: bool,
}

pub struct KickAdapter;

impl ProviderAdapter for KickAdapter {
    fn provider_name(&self) -> &'static str {
        "kick"
    }

    fn capabilities(&self) -> Vec<ProviderCapability> {
        vec![
            ProviderCapability {
                name: "livestream_alerts".to_string(),
                description: "Live notifications with single-embed dynamic update".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "channel_stats".to_string(),
                description: "Channel followers, category and view metrics".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "chat_bridge".to_string(),
                description: "Bidirectional Kick chat gateway".to_string(),
                status: CapabilityStatus::RequiresOAuth,
                read_only: false,
                requires_write_scope: true,
            },
        ]
    }
}
