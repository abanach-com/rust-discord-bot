use crate::integrations::{CapabilityStatus, ProviderAdapter, ProviderCapability};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VkCallbackPayload {
    pub r#type: String, // "confirmation" | "wall_post_new" | "wall_reply_new"
    pub group_id: u64,
    pub secret: Option<String>,
    pub object: Option<serde_json::Value>,
}

pub struct VkAdapter;

impl ProviderAdapter for VkAdapter {
    fn provider_name(&self) -> &'static str {
        "vk"
    }

    fn capabilities(&self) -> Vec<ProviderCapability> {
        vec![
            ProviderCapability {
                name: "callback_handshake".to_string(),
                description: "Atomic confirmation challenge response".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "wall_posts".to_string(),
                description: "Community post and comment notifications".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "isolated_failure".to_string(),
                description: "Provider rate limit and regional degradation containment".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "publish_post".to_string(),
                description: "Publish posts with two-step approval".to_string(),
                status: CapabilityStatus::RequiresOAuth,
                read_only: false,
                requires_write_scope: true,
            },
        ]
    }
}
