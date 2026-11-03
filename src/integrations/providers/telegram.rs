use crate::integrations::{CapabilityStatus, ProviderAdapter, ProviderCapability};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramWebhookUpdate {
    pub update_id: u64,
    pub message: Option<TelegramMessage>,
    pub edited_message: Option<TelegramMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramMessage {
    pub message_id: u64,
    pub from: Option<TelegramUser>,
    pub chat: TelegramChat,
    pub date: u64,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramUser {
    pub id: u64,
    pub is_bot: bool,
    pub first_name: String,
    pub username: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramChat {
    pub id: i64,
    pub r#type: String,
    pub title: Option<String>,
}

pub struct TelegramBridgeAdapter;

impl ProviderAdapter for TelegramBridgeAdapter {
    fn provider_name(&self) -> &'static str {
        "telegram"
    }

    fn capabilities(&self) -> Vec<ProviderCapability> {
        vec![
            ProviderCapability {
                name: "bidirectional_relay".to_string(),
                description: "Synchronize text and replies between Discord and Telegram"
                    .to_string(),
                status: CapabilityStatus::Supported,
                read_only: false,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "anti_loop".to_string(),
                description: "Origin tagging to eliminate infinite message echoes".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "media_whitelist".to_string(),
                description: "Safe attachment relay with MIME type allowlists & size limits"
                    .to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
        ]
    }
}
