use crate::integrations::{CapabilityStatus, ProviderAdapter, ProviderCapability};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteamPlayerSummary {
    pub steamid: String,
    pub personaname: String,
    pub profileurl: String,
    pub communityvisibilitystate: i32, // 3 = public, 1 = private
    pub gameid: Option<String>,
    pub gameextrainfo: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteamOwnedGame {
    pub appid: u64,
    pub name: String,
    pub playtime_forever: i32,
    pub playtime_2weeks: Option<i32>,
}

pub struct SteamAdapter;

impl ProviderAdapter for SteamAdapter {
    fn provider_name(&self) -> &'static str {
        "steam"
    }

    fn capabilities(&self) -> Vec<ProviderCapability> {
        vec![
            ProviderCapability {
                name: "profile".to_string(),
                description: "Steam Player Profile & Current Activity".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "library".to_string(),
                description: "Steam Owned Games (Respects Community Visibility)".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "achievements".to_string(),
                description: "Player Game Achievements".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "lfg".to_string(),
                description: "Cross-Guild Steam Looking For Group".to_string(),
                status: CapabilityStatus::Supported,
                read_only: false,
                requires_write_scope: false,
            },
        ]
    }
}
