use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

pub mod credentials;
pub mod oauth;
pub mod providers;
pub mod webhook;

pub use credentials::CredentialCrypto;

#[derive(Debug, Error)]
pub enum IntegrationError {
    #[error("SSRF validation failed: destination '{0}' is blocked")]
    SsrfBlocked(String),

    #[error("Capability '{capability}' is unsupported for provider '{provider}'")]
    UnsupportedCapability {
        provider: String,
        capability: String,
    },

    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    #[error("Invalid signature: {0}")]
    InvalidSignature(String),

    #[error("Rate limit exceeded: retry after {0} seconds")]
    RateLimited(u64),

    #[error("Tenant isolation violation: {0}")]
    TenantViolation(String),

    #[error("Provider error: {0}")]
    ProviderError(String),

    #[error("Storage error: {0}")]
    Storage(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityStatus {
    Supported,
    Unsupported,
    RequiresOAuth,
    ResearchGated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapability {
    pub name: String,
    pub description: String,
    pub status: CapabilityStatus,
    pub read_only: bool,
    pub requires_write_scope: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderEvent {
    pub guild_id: String,
    pub event_id: String,
    pub occurred_at: DateTime<Utc>,
    pub source: String,
    pub subject: String,
    pub pii_classified: bool,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAction {
    pub action_type: String,
    pub target: String,
    pub parameters: serde_json::Value,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectFlow {
    pub provider: String,
    pub authorization_url: String,
    pub state: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryReceipt {
    pub delivery_id: Uuid,
    pub provider: String,
    pub event_id: String,
    pub acknowledged: bool,
    pub status: String,
    pub delivered_at: DateTime<Utc>,
}

pub trait ProviderAdapter: Send + Sync {
    fn provider_name(&self) -> &'static str;
    fn capabilities(&self) -> Vec<ProviderCapability>;
    fn check_capability(&self, name: &str) -> CapabilityStatus {
        for cap in self.capabilities() {
            if cap.name == name {
                return cap.status;
            }
        }
        CapabilityStatus::Unsupported
    }
}

pub struct ProviderRegistry {
    adapters: HashMap<String, Box<dyn ProviderAdapter>>,
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            adapters: HashMap::new(),
        }
    }

    pub fn register(&mut self, adapter: Box<dyn ProviderAdapter>) {
        self.adapters
            .insert(adapter.provider_name().to_string(), adapter);
    }

    pub fn get(&self, name: &str) -> Option<&dyn ProviderAdapter> {
        self.adapters.get(name).map(|b| b.as_ref())
    }

    pub fn list_providers(&self) -> Vec<String> {
        self.adapters.keys().cloned().collect()
    }
}

// ==========================================
// SSRF Sentinel & Safe URL Validation
// ==========================================

pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            // 0.0.0.0/8
            octets[0] == 0
                // 127.0.0.0/8 (Loopback)
                || octets[0] == 127
                // 10.0.0.0/8 (Private RFC 1918)
                || octets[0] == 10
                // 172.16.0.0/12 (Private RFC 1918)
                || (octets[0] == 172 && (octets[1] >= 16 && octets[1] <= 31))
                // 192.168.0.0/16 (Private RFC 1918)
                || (octets[0] == 192 && octets[1] == 168)
                // 169.254.0.0/16 (Link-Local / Cloud Metadata 169.254.169.254)
                || (octets[0] == 169 && octets[1] == 254)
                // Broadcast 255.255.255.255
                || v4 == Ipv4Addr::BROADCAST
        }
        IpAddr::V6(v6) => {
            // ::1 (Loopback)
            v6.is_loopback()
                // :: (Unspecified)
                || v6.is_unspecified()
                // fc00::/7 (Unique Local Address)
                || ((v6.segments()[0] & 0xfe00) == 0xfc00)
                // fe80::/10 (Link-Local Unicast)
                || ((v6.segments()[0] & 0xffc0) == 0xfe80)
        }
    }
}

pub fn validate_safe_url(
    raw_url: &str,
    allowed_domains: Option<&HashSet<String>>,
) -> Result<Url, IntegrationError> {
    let parsed = Url::parse(raw_url)
        .map_err(|e| IntegrationError::SsrfBlocked(format!("Invalid URL: {e}")))?;

    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return Err(IntegrationError::SsrfBlocked(format!(
            "Forbidden scheme '{}'",
            parsed.scheme()
        )));
    }

    let host_str = parsed
        .host_str()
        .ok_or_else(|| IntegrationError::SsrfBlocked("Missing host".to_string()))?;

    // Check if host is an explicit IP address
    if let Ok(ip) = host_str.parse::<IpAddr>()
        && is_blocked_ip(ip)
    {
        return Err(IntegrationError::SsrfBlocked(format!(
            "Direct IP '{ip}' is in a restricted range"
        )));
    }

    // Check against allowed domains if provided
    if let Some(allowed) = allowed_domains {
        let is_allowed = allowed.iter().any(|domain| {
            host_str.eq_ignore_ascii_case(domain) || host_str.ends_with(&format!(".{domain}"))
        });
        if !is_allowed {
            return Err(IntegrationError::SsrfBlocked(format!(
                "Host '{host_str}' is not in the allowed domains list"
            )));
        }
    }

    Ok(parsed)
}
