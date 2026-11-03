use crate::integrations::{CapabilityStatus, ProviderAdapter, ProviderCapability};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginPackageManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub permissions_required: Vec<String>,
    pub memory_limit_bytes: Option<usize>,
    pub instruction_limit: Option<usize>,
    pub min_bot_version: String,
}

pub fn verify_package_sha256(content: &[u8], expected_hash: &str) -> bool {
    let mut hasher = Sha256::new();
    hasher.update(content);
    let hash = hasher.finalize();
    let computed_hex = hex::encode(hash);
    computed_hex.eq_ignore_ascii_case(expected_hash)
}

pub struct MarketplaceAdapter;

impl ProviderAdapter for MarketplaceAdapter {
    fn provider_name(&self) -> &'static str {
        "marketplace"
    }

    fn capabilities(&self) -> Vec<ProviderCapability> {
        vec![
            ProviderCapability {
                name: "signed_manifest".to_string(),
                description:
                    "Semver manifests with cryptographic origin and SHA-256 integrity verification"
                        .to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "transactional_install".to_string(),
                description: "Atomic installation, staging, and automatic rollback on failure"
                    .to_string(),
                status: CapabilityStatus::Supported,
                read_only: false,
                requires_write_scope: true,
            },
            ProviderCapability {
                name: "runtime_quotas".to_string(),
                description:
                    "Per-plugin instruction, memory, and API quota enforcement in Lua sandbox"
                        .to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "permission_boundary_scan".to_string(),
                description: "Static analysis ensuring plugins cannot escape declarative sandbox"
                    .to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
        ]
    }
}
