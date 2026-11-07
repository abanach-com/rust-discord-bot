use std::sync::Arc;

use anyhow::Result;

use crate::{
    config::Config,
    lua::{LuaEngine, LuaLimits},
    moderation_cases::ModerationCaseStore,
    scheduler::SchedulerStore,
    session::SessionStore,
    storage::Storage,
    tickets::TicketStore,
};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct GuildRealtimeEvent {
    pub guild_id: String,
    pub event_type: String,
    pub payload: serde_json::Value,
}

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub scripts: LuaEngine,
    pub scheduler: SchedulerStore,
    pub storage: Storage,
    pub tickets: TicketStore,
    pub moderation_cases: ModerationCaseStore,
    pub http_client: reqwest::Client,
    pub media_http_client: reqwest_songbird::Client,
    pub metrics: Arc<crate::metrics::Metrics>,
    pub sessions: SessionStore,
    pub rate_limiter: Arc<crate::rate_limit::RateLimiter>,
    pub policy_engine: Arc<crate::policy::CommandPolicyEngine>,
    pub guild_events: tokio::sync::broadcast::Sender<GuildRealtimeEvent>,
    pub asset_analytics: Arc<crate::asset_analytics::AssetAnalyticsBatcher>,
    pub delete_coordinator: Arc<crate::discord::delete_queue::DeleteCoordinator>,
    pub is_gateway_ready: Arc<std::sync::atomic::AtomicBool>,
    pub is_shutting_down: Arc<std::sync::atomic::AtomicBool>,
}

impl AppState {
    pub async fn bootstrap(config: Config) -> Result<Self> {
        let config = Arc::new(config);
        let limits = LuaLimits {
            memory_bytes: config.lua_memory_limit_bytes,
            instruction_limit: config.lua_instruction_limit,
            hook_granularity: config.lua_hook_granularity,
        };
        let scripts = LuaEngine::load(&config.scripts_dir, limits)?.with_concurrency_limits(
            config.lua_max_parallel,
            config.lua_max_per_guild,
            config.lua_command_timeout_ms,
        );
        let transcript_crypto = if let Some(ref keyring_spec) = config.transcript_keyring {
            Some(Arc::new(
                crate::tickets::TranscriptCrypto::from_keyring_spec(
                    &config.transcript_active_key_id,
                    keyring_spec,
                )?,
            ))
        } else if let Some(ref key_hex) = config.transcript_encryption_key {
            Some(Arc::new(crate::tickets::TranscriptCrypto::from_hex_key(
                &config.transcript_active_key_id,
                key_hex,
            )?))
        } else {
            None
        };

        if config.database_url.is_some()
            && transcript_crypto.is_none()
            && !config.ticket_transcripts_disabled
        {
            anyhow::bail!(
                "PostgreSQL ticket storage requires TRANSCRIPT_KEYRING or \
                 TRANSCRIPT_ENCRYPTION_KEY (32 bytes / 64 hex chars). \
                 Configure a persistent secret before starting; plaintext fallback is forbidden. \
                 Alternatively, set TICKET_TRANSCRIPTS_DISABLED=true to run without ticket transcripts."
            );
        }

        let (storage, scheduler, tickets, moderation_cases, sessions) = if let Some(url) =
            config.database_url.as_deref()
        {
            let pool = sqlx::postgres::PgPoolOptions::new()
                .max_connections(25)
                .min_connections(5)
                .acquire_timeout(std::time::Duration::from_secs(5))
                .connect(url)
                .await?;
            crate::storage::run_safe_migrations(&pool).await?;

            (
                Storage::postgres(pool.clone()),
                SchedulerStore::postgres(pool.clone()),
                TicketStore::postgres_with_crypto(pool.clone(), transcript_crypto),
                ModerationCaseStore::postgres(pool.clone()),
                SessionStore::postgres(pool),
            )
        } else {
            tracing::warn!(
                "DATABASE_URL is not set; running with volatile in-memory storage for all subsystems"
            );
            (
                Storage::memory(),
                SchedulerStore::memory(),
                TicketStore::memory(),
                ModerationCaseStore::memory(),
                SessionStore::memory(),
            )
        };
        let user_agent = concat!("RustyDiscordBot/", env!("CARGO_PKG_VERSION"));
        let http_client = reqwest::Client::builder().user_agent(user_agent).build()?;
        let media_http_client = reqwest_songbird::Client::builder()
            .user_agent(user_agent)
            .redirect(reqwest_songbird::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 10 {
                    return attempt.error("too many redirects in media player");
                }

                let next_url = attempt.url();
                if next_url.scheme() != "https" {
                    return attempt.error("media player redirect must use HTTPS protocol");
                }

                let ip = match next_url.host() {
                    Some(url::Host::Ipv4(v4)) => Some(std::net::IpAddr::V4(v4)),
                    Some(url::Host::Ipv6(v6)) => Some(std::net::IpAddr::V6(v6)),
                    _ => next_url
                        .host_str()
                        .and_then(|h| h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok()),
                };
                if let Some(ip) = ip
                    && crate::discord::music::is_blocked_ip(ip)
                {
                    return attempt.error("redirect to blocked private IP address");
                }

                attempt.follow()
            }))
            .build()?;

        let metrics = Arc::new(crate::metrics::Metrics::new());
        let scheduler = scheduler.with_metrics(metrics.clone());
        let rate_limiter = Arc::new(crate::rate_limit::RateLimiter::new());
        let policy_engine = Arc::new(crate::policy::CommandPolicyEngine::new());
        let (guild_events, _) = tokio::sync::broadcast::channel(512);
        let asset_analytics = Arc::new(crate::asset_analytics::AssetAnalyticsBatcher::new(
            storage.clone(),
        ));
        let delete_coordinator = Arc::new(crate::discord::delete_queue::DeleteCoordinator::new(
            100, 256,
        ));
        let is_gateway_ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let is_shutting_down = Arc::new(std::sync::atomic::AtomicBool::new(false));
        Ok(Self {
            config,
            scripts,
            scheduler,
            storage,
            tickets,
            moderation_cases,
            http_client,
            media_http_client,
            metrics,
            sessions,
            rate_limiter,
            policy_engine,
            guild_events,
            asset_analytics,
            delete_coordinator,
            is_gateway_ready,
            is_shutting_down,
        })
    }

    pub async fn deliver_store_role(
        &self,
        guild_id: &str,
        user_id: &str,
        role_id: &str,
    ) -> anyhow::Result<()> {
        crate::storage::validate_snowflake("guild_id", guild_id)?;
        crate::storage::validate_snowflake("user_id", user_id)?;
        crate::storage::validate_snowflake("role_id", role_id)?;

        let is_offline = self.config.is_offline_mode()
            && self
                .config
                .discord_api_base_url
                .starts_with("https://discord.com");

        if is_offline {
            tracing::debug!(
                guild_id,
                user_id,
                role_id,
                "deliver_store_role: offline/mock mode detected, simulating success"
            );
            return Ok(());
        }

        let base_url = &self.config.discord_api_base_url;
        let url_string = format!("{base_url}/guilds/{guild_id}/members/{user_id}/roles/{role_id}");
        let parsed_url = reqwest::Url::parse(&url_string)?;
        let expected_suffix = format!("/guilds/{guild_id}/members/{user_id}/roles/{role_id}");
        anyhow::ensure!(
            parsed_url.path().ends_with(&expected_suffix),
            "URL path manipulation or traversal detected"
        );

        let mut attempts = 0;
        const MAX_ATTEMPTS: u32 = 3;

        loop {
            attempts += 1;
            let response = self
                .http_client
                .put(parsed_url.clone())
                .header(
                    reqwest::header::AUTHORIZATION,
                    format!("Bot {}", self.config.discord_token),
                )
                .header(
                    reqwest::header::HeaderName::from_static("x-audit-log-reason"),
                    "Store item role delivery",
                )
                .send()
                .await;

            match response {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return Ok(());
                    }

                    if (status == reqwest::StatusCode::TOO_MANY_REQUESTS
                        || status.is_server_error())
                        && attempts < MAX_ATTEMPTS
                    {
                        let backoff = if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                            let retry_after_secs = resp
                                .headers()
                                .get(reqwest::header::RETRY_AFTER)
                                .and_then(|v| v.to_str().ok())
                                .and_then(|s| s.parse::<f64>().ok())
                                .unwrap_or(0.5);
                            std::time::Duration::from_secs_f64(retry_after_secs.min(2.0))
                        } else {
                            std::time::Duration::from_millis(100 * (1 << attempts))
                        };
                        tokio::time::sleep(backoff).await;
                        continue;
                    }

                    let error_text = resp.text().await.unwrap_or_default();
                    anyhow::bail!(
                        "Discord API returned status {status} when adding role {role_id} to member {user_id}: {error_text}"
                    );
                }
                Err(err) => {
                    if attempts < MAX_ATTEMPTS {
                        tokio::time::sleep(std::time::Duration::from_millis(100 * (1 << attempts)))
                            .await;
                        continue;
                    }
                    anyhow::bail!("Network error calling Discord API: {err}");
                }
            }
        }
    }
}
