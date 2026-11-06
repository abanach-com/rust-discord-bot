use std::{collections::HashSet, env, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result, anyhow};

#[derive(Clone, Debug)]
pub struct Config {
    pub discord_token: String,
    pub discord_application_id: u64,
    pub discord_client_id: String,
    pub discord_client_secret: String,
    pub discord_oauth_redirect_url: String,
    pub discord_dev_guild_id: Option<u64>,
    pub dashboard_bind: SocketAddr,
    pub dashboard_public_url: String,
    pub database_url: Option<String>,
    pub scripts_dir: PathBuf,
    pub session_cookie_secure: bool,
    pub session_ttl_seconds: i64,
    pub lua_memory_limit_bytes: usize,
    pub lua_instruction_limit: i64,
    pub lua_hook_granularity: u32,
    pub lua_max_parallel: usize,
    pub lua_max_per_guild: usize,
    pub lua_command_timeout_ms: u64,
    pub music_allowed_hosts: HashSet<String>,
    pub trusted_proxies: HashSet<std::net::IpAddr>,
    pub offline_mode: bool,
    pub discord_api_base_url: String,
    pub transcript_encryption_key: Option<String>,
    pub transcript_active_key_id: String,
    pub transcript_keyring: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let discord_application_id: u64 = required("DISCORD_APPLICATION_ID")?
            .parse()
            .context("DISCORD_APPLICATION_ID must be an unsigned integer")?;

        let discord_client_id =
            env::var("DISCORD_CLIENT_ID").unwrap_or_else(|_| discord_application_id.to_string());

        let dashboard_bind = env::var("DASHBOARD_BIND")
            .unwrap_or_else(|_| "0.0.0.0:8080".to_owned())
            .parse()
            .context("DASHBOARD_BIND must be in host:port format")?;

        let music_allowed_hosts = env::var("MUSIC_ALLOWED_HOSTS")
            .unwrap_or_else(|_| {
                [
                    "youtube.com",
                    "www.youtube.com",
                    "m.youtube.com",
                    "youtu.be",
                    "soundcloud.com",
                    "www.soundcloud.com",
                    "bandcamp.com",
                ]
                .join(",")
            })
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
            .collect();

        let config = Self {
            discord_token: required("DISCORD_TOKEN")?,
            discord_application_id,
            discord_client_id,
            discord_client_secret: required("DISCORD_CLIENT_SECRET")?,
            discord_oauth_redirect_url: env::var("DISCORD_OAUTH_REDIRECT_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8080/auth/discord/callback".to_owned()),
            discord_dev_guild_id: optional_parse("DISCORD_DEV_GUILD_ID")?,
            dashboard_bind,
            dashboard_public_url: env::var("DASHBOARD_PUBLIC_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8080".to_owned()),
            database_url: optional_non_empty("DATABASE_URL"),
            scripts_dir: env::var("SCRIPTS_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("scripts")),
            session_cookie_secure: env_bool("SESSION_COOKIE_SECURE", false)?,
            session_ttl_seconds: env_parse("SESSION_TTL_SECONDS", 604_800_i64)?,
            lua_memory_limit_bytes: env_parse("LUA_MEMORY_LIMIT_BYTES", 8 * 1024 * 1024_usize)?,
            lua_instruction_limit: env_parse("LUA_INSTRUCTION_LIMIT", 500_000_i64)?,
            lua_hook_granularity: env_parse("LUA_HOOK_GRANULARITY", 1_000_u32)?,
            lua_max_parallel: env_parse("LUA_MAX_PARALLEL", 32_usize)?,
            lua_max_per_guild: env_parse("LUA_MAX_PER_GUILD", 8_usize)?,
            lua_command_timeout_ms: env_parse("LUA_COMMAND_TIMEOUT_MS", 3_000_u64)?,
            music_allowed_hosts,
            trusted_proxies: env::var("TRUSTED_PROXIES")
                .unwrap_or_else(|_| "127.0.0.1,::1".to_owned())
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .filter_map(|value| value.parse::<std::net::IpAddr>().ok())
                .collect(),
            offline_mode: env_bool("OFFLINE_MODE", false)?,
            discord_api_base_url: env::var("DISCORD_API_BASE_URL")
                .unwrap_or_else(|_| "https://discord.com/api/v10".to_owned()),
            transcript_encryption_key: optional_non_empty("TRANSCRIPT_ENCRYPTION_KEY"),
            transcript_active_key_id: env::var("TRANSCRIPT_ACTIVE_KEY_ID")
                .unwrap_or_else(|_| "v1".to_owned()),
            transcript_keyring: optional_non_empty("TRANSCRIPT_KEYRING"),
        };

        config.validate()?;
        Ok(config)
    }

    pub fn is_offline_mode(&self) -> bool {
        self.discord_token.starts_with("dummy")
            || self.discord_token == "mock"
            || self.discord_token.starts_with("mock")
            || self.discord_token == "offline"
            || self.offline_mode
    }

    fn validate(&self) -> Result<()> {
        if self.session_ttl_seconds <= 0 {
            return Err(anyhow!("SESSION_TTL_SECONDS must be greater than zero"));
        }
        if self.lua_memory_limit_bytes < 256 * 1024 {
            return Err(anyhow!(
                "LUA_MEMORY_LIMIT_BYTES must be at least 262144 bytes"
            ));
        }
        if self.lua_instruction_limit <= 0 {
            return Err(anyhow!("LUA_INSTRUCTION_LIMIT must be greater than zero"));
        }
        if self.lua_hook_granularity == 0 {
            return Err(anyhow!("LUA_HOOK_GRANULARITY must be greater than zero"));
        }
        if self.lua_max_parallel == 0 {
            return Err(anyhow!("LUA_MAX_PARALLEL must be greater than zero"));
        }
        if self.lua_max_per_guild == 0 {
            return Err(anyhow!("LUA_MAX_PER_GUILD must be greater than zero"));
        }
        if self.lua_max_per_guild > self.lua_max_parallel {
            return Err(anyhow!("LUA_MAX_PER_GUILD cannot exceed LUA_MAX_PARALLEL"));
        }
        if self.lua_command_timeout_ms == 0 {
            return Err(anyhow!("LUA_COMMAND_TIMEOUT_MS must be greater than zero"));
        }
        if self.music_allowed_hosts.is_empty() {
            return Err(anyhow!("MUSIC_ALLOWED_HOSTS cannot be empty"));
        }

        Ok(())
    }
}

fn required(name: &str) -> Result<String> {
    env::var(name)
        .with_context(|| format!("{name} is required"))
        .and_then(|value| {
            if value.trim().is_empty() {
                Err(anyhow!("{name} cannot be empty"))
            } else {
                Ok(value)
            }
        })
}

fn optional_non_empty(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn optional_parse<T>(name: &str) -> Result<Option<T>>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    optional_non_empty(name)
        .map(|value| {
            value
                .parse()
                .with_context(|| format!("{name} contains an invalid value"))
        })
        .transpose()
}

fn env_parse<T>(name: &str, default: T) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    match optional_non_empty(name) {
        Some(value) => value
            .parse()
            .with_context(|| format!("{name} contains an invalid value")),
        None => Ok(default),
    }
}

fn parse_bool_str(name: &str, value: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(anyhow!(
            "{name} must be one of: true, false, 1, 0, yes, no, on, off"
        )),
    }
}

fn env_bool(name: &str, default: bool) -> Result<bool> {
    match optional_non_empty(name) {
        None => Ok(default),
        Some(value) => parse_bool_str(name, &value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_offline_mode_logic() {
        let mut cfg = Config {
            discord_token: "real_discord_bot_token_123".to_string(),
            discord_application_id: 1,
            discord_client_id: "1".to_string(),
            discord_client_secret: "secret".to_string(),
            discord_oauth_redirect_url: "http://localhost".to_string(),
            discord_dev_guild_id: None,
            dashboard_bind: "127.0.0.1:8080".parse().unwrap(),
            dashboard_public_url: "http://localhost".to_string(),
            database_url: None,
            scripts_dir: PathBuf::from("scripts"),
            session_cookie_secure: false,
            session_ttl_seconds: 604800,
            lua_memory_limit_bytes: 1024 * 1024,
            lua_instruction_limit: 100_000,
            lua_hook_granularity: 100,
            lua_max_parallel: 8,
            lua_max_per_guild: 2,
            lua_command_timeout_ms: 1000,
            music_allowed_hosts: HashSet::new(),
            trusted_proxies: HashSet::new(),
            offline_mode: false,
            discord_api_base_url: "https://discord.com/api/v10".to_string(),
            transcript_encryption_key: None,
            transcript_active_key_id: "v1".to_string(),
            transcript_keyring: None,
        };

        // Real token with offline_mode false -> NOT offline mode
        assert!(!cfg.is_offline_mode());

        // Real token with offline_mode true -> offline mode
        cfg.offline_mode = true;
        assert!(cfg.is_offline_mode());

        // Dummy/mock tokens -> always offline mode
        cfg.offline_mode = false;
        cfg.discord_token = "dummy_token".to_string();
        assert!(cfg.is_offline_mode());

        cfg.discord_token = "mock".to_string();
        assert!(cfg.is_offline_mode());

        cfg.discord_token = "offline".to_string();
        assert!(cfg.is_offline_mode());
    }

    #[test]
    fn test_parse_bool_str() {
        assert!(parse_bool_str("FLAG", "true").unwrap());
        assert!(parse_bool_str("FLAG", "1").unwrap());
        assert!(parse_bool_str("FLAG", "yes").unwrap());
        assert!(parse_bool_str("FLAG", "on").unwrap());

        assert!(!parse_bool_str("FLAG", "false").unwrap());
        assert!(!parse_bool_str("FLAG", "0").unwrap());
        assert!(!parse_bool_str("FLAG", "no").unwrap());
        assert!(!parse_bool_str("FLAG", "off").unwrap());

        assert!(parse_bool_str("FLAG", "invalid").is_err());
        assert!(parse_bool_str("FLAG", "2").is_err());

        // Default returned for unset env var
        assert!(!env_bool("NON_EXISTENT_VAR_XYZ_9876", false).unwrap());
        assert!(env_bool("NON_EXISTENT_VAR_XYZ_9876", true).unwrap());
    }
}
