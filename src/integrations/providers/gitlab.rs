use crate::integrations::{CapabilityStatus, ProviderAdapter, ProviderCapability};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitLabPipelineEvent {
    pub object_kind: String,
    pub object_attributes: GitLabPipelineAttributes,
    pub project: GitLabProject,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitLabPipelineAttributes {
    pub id: u64,
    pub r#ref: String,
    pub status: String,
    pub detailed_status: String,
    pub duration: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitLabProject {
    pub id: u64,
    pub name: String,
    pub web_url: String,
    pub path_with_namespace: String,
}

pub struct GitLabAdapter;

impl ProviderAdapter for GitLabAdapter {
    fn provider_name(&self) -> &'static str {
        "gitlab"
    }

    fn capabilities(&self) -> Vec<ProviderCapability> {
        vec![
            ProviderCapability {
                name: "pipeline".to_string(),
                description: "CI/CD Pipeline status and failure tracking".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "merge_request".to_string(),
                description: "MR lifecycle and review notifications".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "self_hosted_allowlist".to_string(),
                description: "Safe self-hosted instance connections with SSRF filtering"
                    .to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "ci_retry_action".to_string(),
                description: "Explicitly authorized retry for failed pipelines".to_string(),
                status: CapabilityStatus::RequiresOAuth,
                read_only: false,
                requires_write_scope: true,
            },
        ]
    }
}
