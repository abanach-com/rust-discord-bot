use crate::integrations::{CapabilityStatus, ProviderAdapter, ProviderCapability};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameServerStatusReport {
    pub game_id: String,
    pub region: String,
    pub server_name: String,
    pub status: String, // "online" | "offline" | "maintenance" | "unknown"
    pub player_count: i32,
    pub last_checked_at: String,
}

pub struct GamingHubAdapter;

impl ProviderAdapter for GamingHubAdapter {
    fn provider_name(&self) -> &'static str {
        "gaming_hub"
    }

    fn capabilities(&self) -> Vec<ProviderCapability> {
        vec![
            ProviderCapability {
                name: "poe_separation".to_string(),
                description: "Strict isolation of PoE 1 and PoE 2 namespaces & leagues".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "crossplay_lfg".to_string(),
                description: "Aggregated Looking For Group across Steam, MC, PoE".to_string(),
                status: CapabilityStatus::Supported,
                read_only: false,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "server_status".to_string(),
                description: "Official game server status monitor with unknown-state handling"
                    .to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
        ]
    }
}
