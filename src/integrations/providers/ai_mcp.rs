use crate::integrations::{CapabilityStatus, ProviderAdapter, ProviderCapability};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolCallRequest {
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub guild_id: String,
    pub actor_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDescriptor {
    pub name: String,
    pub description: String,
    pub parameters_schema: serde_json::Value,
    pub is_mutating: bool,
}

pub struct AiMcpAdapter;

impl ProviderAdapter for AiMcpAdapter {
    fn provider_name(&self) -> &'static str {
        "ai_mcp"
    }

    fn capabilities(&self) -> Vec<ProviderCapability> {
        vec![
            ProviderCapability {
                name: "hard_budget_cap".to_string(),
                description: "Monthly token and USD limits per guild with emergency cut-off".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "channel_acl_rag".to_string(),
                description: "Document knowledge base with Discord channel permissions filtering".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "mcp_tool_allowlist".to_string(),
                description: "Model Context Protocol client/server with strict tool authorization".to_string(),
                status: CapabilityStatus::Supported,
                read_only: false,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "prompt_injection_guard".to_string(),
                description: "Input sanitization and redacting secrets before forwarding to LLM".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "side_effect_approval".to_string(),
                description: "Require explicit human confirmation before executing mutating actions (ban, post, delete)".to_string(),
                status: CapabilityStatus::Supported,
                read_only: false,
                requires_write_scope: true,
            },
        ]
    }
}
