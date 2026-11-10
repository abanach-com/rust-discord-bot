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
                // 100.64.0.0/10 (carrier-grade NAT)
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
                // 192.0.0.0/24 (IETF protocol assignments)
                || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
                // Multicast 224.0.0.0/4 and Reserved 240.0.0.0/4
                || v4.is_multicast()
                || octets[0] >= 240
                // Broadcast 255.255.255.255
                || v4 == Ipv4Addr::BROADCAST
        }
        IpAddr::V6(v6) => {
            // ::1 (Loopback)
            if v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() {
                return true;
            }
            let segments = v6.segments();
            // 2001:db8::/32 (Documentation)
            if segments[0] == 0x2001 && segments[1] == 0x0db8 {
                return true;
            }
            // fc00::/7 (Unique Local Address)
            if (segments[0] & 0xfe00) == 0xfc00 {
                return true;
            }
            // fe80::/10 (Link-Local Unicast)
            if (segments[0] & 0xffc0) == 0xfe80 {
                return true;
            }
            // IPv4-mapped (::ffff:a.b.c.d) or IPv4-compatible (::a.b.c.d)
            let mapped_or_compat_v4 = if let Some(v4) = v6.to_ipv4_mapped() {
                Some(v4)
            } else if segments[..6] == [0, 0, 0, 0, 0, 0] {
                let octets = v6.octets();
                Some(Ipv4Addr::new(
                    octets[12], octets[13], octets[14], octets[15],
                ))
            } else {
                None
            };
            if let Some(v4) = mapped_or_compat_v4 {
                return is_blocked_ip(IpAddr::V4(v4));
            }
            false
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

    // Check if host is an explicit IP address (IPv4, bracketed IPv6, IPv4-mapped, etc.)
    let direct_ip = match parsed.host() {
        Some(url::Host::Ipv4(v4)) => Some(IpAddr::V4(v4)),
        Some(url::Host::Ipv6(v6)) => Some(IpAddr::V6(v6)),
        _ => host_str.trim_matches(['[', ']']).parse::<IpAddr>().ok(),
    };

    if let Some(ip) = direct_ip
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_blocked_ip_v4_and_v6() {
        assert!(is_blocked_ip("127.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("10.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("192.168.1.1".parse().unwrap()));
        assert!(is_blocked_ip("169.254.169.254".parse().unwrap()));
        assert!(is_blocked_ip("::1".parse().unwrap()));
        assert!(is_blocked_ip("::".parse().unwrap()));
        assert!(is_blocked_ip("fe80::1".parse().unwrap()));
        assert!(is_blocked_ip("fc00::1".parse().unwrap()));
        assert!(is_blocked_ip("::ffff:127.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("::ffff:169.254.169.254".parse().unwrap()));
        assert!(is_blocked_ip("::ffff:10.0.0.1".parse().unwrap()));

        assert!(!is_blocked_ip("8.8.8.8".parse().unwrap()));
        assert!(!is_blocked_ip("1.1.1.1".parse().unwrap()));
        assert!(!is_blocked_ip("2606:4700:4700::1111".parse().unwrap()));
    }

    #[test]
    fn test_validate_safe_url_bracketed_ipv6_and_mapped_ipv6_issue_368() {
        for target in [
            "http://[::1]/",
            "http://[::]/",
            "https://[fe80::1]/",
            "https://[fc00::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[::ffff:169.254.169.254]/",
            "http://[::ffff:10.0.0.1]/",
            "http://127.0.0.1:8080/api",
            "http://169.254.169.254/latest/meta-data/",
        ] {
            assert!(
                validate_safe_url(target, None).is_err(),
                "Target '{target}' must be rejected by validate_safe_url"
            );
        }

        // Public URLs should succeed
        assert!(validate_safe_url("https://[2606:4700:4700::1111]/", None).is_ok());
        assert!(validate_safe_url("https://api.github.com/repos/rust-lang/rust", None).is_ok());

        // Domain allowlist check
        let mut allowed = HashSet::new();
        allowed.insert("github.com".to_string());
        assert!(validate_safe_url("https://api.github.com/something", Some(&allowed)).is_ok());
        assert!(validate_safe_url("https://gitlab.com/something", Some(&allowed)).is_err());
        assert!(validate_safe_url("http://[::1]/", Some(&allowed)).is_err());
    }
}
