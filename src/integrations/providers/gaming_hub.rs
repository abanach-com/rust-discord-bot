use crate::integrations::{
    CapabilityStatus, IntegrationError, ProviderAdapter, ProviderCapability,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameServerStatusReport {
    pub game_id: String,
    pub region: String,
    pub server_name: String,
    pub status: String, // "online" | "offline" | "maintenance" | "unknown"
    pub player_count: i32,
    pub last_checked_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatalogStatus {
    Fresh,
    Stale,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LeagueKind {
    Permanent,
    Challenge,
    Event,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PoeLeagueEntry {
    pub name: String,
    pub kind: LeagueKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LeagueCatalogResponse {
    pub game: String,
    pub leagues: Vec<String>,
    pub status: CatalogStatus,
    pub observed_at: Option<DateTime<Utc>>,
    pub source: Option<String>,
}

#[derive(Debug, Clone)]
struct CachedLeagues {
    leagues: Vec<String>,
    observed_at: DateTime<Utc>,
    source: String,
}

pub struct PoeLeagueCatalog {
    pub ttl: Duration,
    pub stale_grace: Duration,
    poe1_cache: RwLock<Option<CachedLeagues>>,
    poe2_cache: RwLock<Option<CachedLeagues>>,
}

impl PoeLeagueCatalog {
    pub fn new(ttl: Duration, stale_grace: Duration) -> Self {
        Self {
            ttl,
            stale_grace,
            poe1_cache: RwLock::new(None),
            poe2_cache: RwLock::new(None),
        }
    }

    pub fn get_leagues(
        &self,
        version: &str,
        now: DateTime<Utc>,
    ) -> Result<LeagueCatalogResponse, &'static str> {
        let (game_name, cache_lock, permanent) = match version.to_lowercase().as_str() {
            "poe1" => (
                "Path of Exile 1",
                &self.poe1_cache,
                vec!["Standard".to_string(), "Hardcore".to_string()],
            ),
            "poe2" => (
                "Path of Exile 2",
                &self.poe2_cache,
                vec!["Standard 2".to_string(), "Hardcore 2".to_string()],
            ),
            _ => return Err("Version must be 'poe1' or 'poe2'"),
        };

        let guard = cache_lock
            .read()
            .map_err(|_| "Failed to acquire league cache read lock")?;

        if let Some(cached) = guard.as_ref() {
            let age = now.signed_duration_since(cached.observed_at);
            if age <= self.ttl {
                return Ok(LeagueCatalogResponse {
                    game: game_name.to_string(),
                    leagues: cached.leagues.clone(),
                    status: CatalogStatus::Fresh,
                    observed_at: Some(cached.observed_at),
                    source: Some(cached.source.clone()),
                });
            } else if age <= self.ttl + self.stale_grace {
                return Ok(LeagueCatalogResponse {
                    game: game_name.to_string(),
                    leagues: cached.leagues.clone(),
                    status: CatalogStatus::Stale,
                    observed_at: Some(cached.observed_at),
                    source: Some(cached.source.clone()),
                });
            }
        }

        // No cache or expired beyond stale grace -> Unavailable, fallback to permanent leagues only
        Ok(LeagueCatalogResponse {
            game: game_name.to_string(),
            leagues: permanent,
            status: CatalogStatus::Unavailable,
            observed_at: None,
            source: None,
        })
    }

    pub fn update_leagues(
        &self,
        version: &str,
        leagues: Vec<String>,
        source: &str,
        observed_at: DateTime<Utc>,
    ) -> Result<(), &'static str> {
        match version.to_lowercase().as_str() {
            "poe1" => {
                // PoE 1 must not contain PoE 2 leagues
                for league in &leagues {
                    if league.contains("Standard 2")
                        || league.contains("Hardcore 2")
                        || league.contains("Early Access League")
                    {
                        return Err("PoE 1 must not contain PoE 2 leagues");
                    }
                    // Issue #347: Settlers of Kalguur ended in 2026; cannot be recorded as fresh
                    if league.eq_ignore_ascii_case("Settlers of Kalguur")
                        || league.eq_ignore_ascii_case("HC Settlers of Kalguur")
                    {
                        return Err(
                            "Settlers of Kalguur is a historical league and cannot be recorded as active",
                        );
                    }
                }
                let mut guard = self
                    .poe1_cache
                    .write()
                    .map_err(|_| "Failed to acquire poe1 cache lock")?;
                *guard = Some(CachedLeagues {
                    leagues,
                    observed_at,
                    source: source.to_string(),
                });
                Ok(())
            }
            "poe2" => {
                // PoE 2 must not contain PoE 1 leagues
                for league in &leagues {
                    if (league == "Standard" || league == "Hardcore")
                        || league.contains("Settlers of Kalguur")
                    {
                        return Err("PoE 2 must not contain PoE 1 leagues");
                    }
                }
                let mut guard = self
                    .poe2_cache
                    .write()
                    .map_err(|_| "Failed to acquire poe2 cache lock")?;
                *guard = Some(CachedLeagues {
                    leagues,
                    observed_at,
                    source: source.to_string(),
                });
                Ok(())
            }
            _ => Err("Version must be 'poe1' or 'poe2'"),
        }
    }
}

pub fn global_poe_catalog() -> &'static PoeLeagueCatalog {
    static INSTANCE: OnceLock<PoeLeagueCatalog> = OnceLock::new();
    INSTANCE.get_or_init(|| PoeLeagueCatalog::new(Duration::hours(1), Duration::hours(24)))
}

#[derive(Debug, Clone)]
pub struct PoeUpstreamClient {
    pub allowed_hosts: Vec<String>,
}

impl Default for PoeUpstreamClient {
    fn default() -> Self {
        Self {
            allowed_hosts: vec![
                "api.pathofexile.com".to_string(),
                "127.0.0.1".to_string(),
                "localhost".to_string(),
            ],
        }
    }
}

impl PoeUpstreamClient {
    pub fn new(allowed_hosts: Vec<String>) -> Self {
        Self { allowed_hosts }
    }

    pub fn validate_url(&self, target_url: &str) -> Result<url::Url, IntegrationError> {
        let parsed = url::Url::parse(target_url)
            .map_err(|e| IntegrationError::SsrfBlocked(format!("Invalid URL: {e}")))?;

        match parsed.scheme() {
            "http" | "https" => {}
            other => {
                return Err(IntegrationError::SsrfBlocked(format!(
                    "Unsupported scheme '{other}'"
                )));
            }
        }

        let host = parsed
            .host_str()
            .ok_or_else(|| IntegrationError::SsrfBlocked("Missing host".to_string()))?;

        if !self
            .allowed_hosts
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(host))
        {
            return Err(IntegrationError::SsrfBlocked(format!(
                "Host '{host}' is not in allowlist"
            )));
        }

        Ok(parsed)
    }

    pub fn handle_mock_response(
        &self,
        status_code: u16,
        body: Option<&str>,
        auth_token_for_leak_check: Option<&str>,
    ) -> Result<Vec<String>, IntegrationError> {
        if status_code == 429 {
            return Err(IntegrationError::RateLimited(60));
        }
        if (500..=599).contains(&status_code) {
            return Err(IntegrationError::ProviderError(format!(
                "Upstream returned HTTP {status_code}"
            )));
        }
        if status_code != 200 {
            return Err(IntegrationError::ProviderError(format!(
                "Unexpected status {status_code}"
            )));
        }

        let raw_body = body.unwrap_or("[]");
        if auth_token_for_leak_check.is_some_and(|token| raw_body.contains(token)) {
            return Err(IntegrationError::AuthenticationFailed(
                "Token leak detected in upstream payload".to_string(),
            ));
        }

        let parsed: Vec<String> = serde_json::from_str(raw_body).map_err(|e| {
            IntegrationError::ProviderError(format!("Failed to parse leagues JSON: {e}"))
        })?;

        Ok(parsed)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_poe_catalog_initial_state_is_unavailable_and_no_settlers_of_kalguur() {
        let catalog = PoeLeagueCatalog::new(Duration::hours(1), Duration::hours(24));
        let now = Utc::now();

        let poe1 = catalog.get_leagues("poe1", now).unwrap();
        assert_eq!(poe1.game, "Path of Exile 1");
        assert_eq!(poe1.status, CatalogStatus::Unavailable);
        assert_eq!(poe1.leagues, vec!["Standard", "Hardcore"]);
        assert!(
            !poe1.leagues.contains(&"Settlers of Kalguur".to_string()),
            "Historical Settlers of Kalguur must NOT be returned as active"
        );
        assert!(poe1.observed_at.is_none());
        assert!(poe1.source.is_none());

        let poe2 = catalog.get_leagues("poe2", now).unwrap();
        assert_eq!(poe2.game, "Path of Exile 2");
        assert_eq!(poe2.status, CatalogStatus::Unavailable);
        assert_eq!(poe2.leagues, vec!["Standard 2", "Hardcore 2"]);
        assert!(poe2.observed_at.is_none());
    }

    #[test]
    fn test_poe_catalog_fresh_stale_lifecycle_and_poe2_dated_leagues() {
        let catalog = PoeLeagueCatalog::new(Duration::hours(1), Duration::hours(24));
        let t0 = Utc::now();

        // 1. PoE 2 receives dated leagues (e.g. Forbidden Rites, Runes of Aldur)
        catalog
            .update_leagues(
                "poe2",
                vec![
                    "Standard 2".to_string(),
                    "Hardcore 2".to_string(),
                    "Forbidden Rites".to_string(),
                    "Runes of Aldur".to_string(),
                ],
                "https://api.pathofexile.com/leagues",
                t0,
            )
            .unwrap();

        // Immediately after -> Fresh
        let res_fresh = catalog
            .get_leagues("poe2", t0 + Duration::minutes(10))
            .unwrap();
        assert_eq!(res_fresh.status, CatalogStatus::Fresh);
        assert_eq!(res_fresh.observed_at, Some(t0));
        assert!(res_fresh.leagues.contains(&"Forbidden Rites".to_string()));
        assert!(res_fresh.leagues.contains(&"Runes of Aldur".to_string()));

        // After TTL but before stale grace (e.g. +2 hours) -> Stale
        let res_stale = catalog
            .get_leagues("poe2", t0 + Duration::hours(2))
            .unwrap();
        assert_eq!(res_stale.status, CatalogStatus::Stale);
        assert_eq!(res_stale.observed_at, Some(t0));

        // After stale grace (e.g. +26 hours) -> Unavailable with permanent fallback
        let res_unavail = catalog
            .get_leagues("poe2", t0 + Duration::hours(26))
            .unwrap();
        assert_eq!(res_unavail.status, CatalogStatus::Unavailable);
        assert_eq!(res_unavail.leagues, vec!["Standard 2", "Hardcore 2"]);
    }

    #[test]
    fn test_poe1_rejects_settlers_and_cross_game_contamination() {
        let catalog = PoeLeagueCatalog::new(Duration::hours(1), Duration::hours(24));
        let now = Utc::now();

        // Reject Settlers of Kalguur in PoE 1
        let err_settlers = catalog.update_leagues(
            "poe1",
            vec!["Standard".to_string(), "Settlers of Kalguur".to_string()],
            "test_source",
            now,
        );
        assert!(err_settlers.is_err());

        // Reject PoE 2 leagues in PoE 1
        let err_poe2_in_poe1 =
            catalog.update_leagues("poe1", vec!["Standard 2".to_string()], "test_source", now);
        assert!(err_poe2_in_poe1.is_err());

        // Reject PoE 1 leagues in PoE 2
        let err_poe1_in_poe2 =
            catalog.update_leagues("poe2", vec!["Standard".to_string()], "test_source", now);
        assert!(err_poe1_in_poe2.is_err());

        // Invalid version returns error
        assert!(catalog.get_leagues("poe3", now).is_err());
    }

    #[test]
    fn test_upstream_client_ssrf_and_error_handling() {
        let client = PoeUpstreamClient::default();

        // Valid URLs
        assert!(
            client
                .validate_url("https://api.pathofexile.com/leagues")
                .is_ok()
        );
        assert!(client.validate_url("http://127.0.0.1:8080/mock").is_ok());

        // SSRF blocked
        assert!(
            client
                .validate_url("https://malicious.internal.corp/secret")
                .is_err()
        );
        assert!(client.validate_url("file:///etc/passwd").is_err());

        // Mock status 200
        let parsed = client
            .handle_mock_response(200, Some("[\"Standard\", \"Hardcore\"]"), None)
            .unwrap();
        assert_eq!(parsed, vec!["Standard", "Hardcore"]);

        // Mock 429
        assert!(matches!(
            client.handle_mock_response(429, None, None),
            Err(IntegrationError::RateLimited(_))
        ));

        // Mock 500
        assert!(matches!(
            client.handle_mock_response(500, None, None),
            Err(IntegrationError::ProviderError(_))
        ));

        // Token leak detection
        let leak_res = client.handle_mock_response(
            200,
            Some("[\"secret_token_12345\"]"),
            Some("secret_token_12345"),
        );
        assert!(matches!(
            leak_res,
            Err(IntegrationError::AuthenticationFailed(_))
        ));
    }
}
