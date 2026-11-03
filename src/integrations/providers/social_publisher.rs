use crate::integrations::{CapabilityStatus, ProviderAdapter, ProviderCapability};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetDeliveryStatus {
    pub target: String, // "discord" | "telegram" | "vk"
    pub delivered: bool,
    pub external_post_id: Option<String>,
    pub error: Option<String>,
}

pub struct SocialPublisherAdapter;

impl ProviderAdapter for SocialPublisherAdapter {
    fn provider_name(&self) -> &'static str {
        "social_publisher"
    }

    fn capabilities(&self) -> Vec<ProviderCapability> {
        vec![
            ProviderCapability {
                name: "multi_target_composer".to_string(),
                description: "Cross-platform authoring with per-platform length and media validation".to_string(),
                status: CapabilityStatus::Supported,
                read_only: false,
                requires_write_scope: true,
            },
            ProviderCapability {
                name: "scheduled_publishing".to_string(),
                description: "Timezone-aware queue with retry idempotency".to_string(),
                status: CapabilityStatus::Supported,
                read_only: false,
                requires_write_scope: true,
            },
            ProviderCapability {
                name: "two_person_approval".to_string(),
                description: "Mandatory staff signoff gate prior to publication".to_string(),
                status: CapabilityStatus::Supported,
                read_only: false,
                requires_write_scope: true,
            },
            ProviderCapability {
                name: "partial_failure_isolation".to_string(),
                description: "Isolates failures so successful platform deliveries are not duplicated on retry".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
        ]
    }
}
