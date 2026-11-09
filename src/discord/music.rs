use anyhow::{Context as _, Result, anyhow};
use serenity::{
    all::{GuildId, UserId},
    client::Context,
};
use songbird::input::YoutubeDl;
use url::Url;

use crate::{AppState, lua::MusicOperation};

pub const MAX_QUEUE_SIZE: usize = 100;

static GUILD_VOLUMES: std::sync::LazyLock<dashmap::DashMap<GuildId, f32>> =
    std::sync::LazyLock::new(dashmap::DashMap::new);

pub fn resolve_effective_volume(guild_id: GuildId, configured_volume: Option<f64>) -> f32 {
    let configured_gain = configured_volume
        .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
        .unwrap_or(0.5) as f32;
    GUILD_VOLUMES
        .get(&guild_id)
        .map(|v| *v)
        .unwrap_or(configured_gain)
}

pub fn set_guild_volume(guild_id: GuildId, level: f32) {
    GUILD_VOLUMES.insert(guild_id, level / 100.0);
}

pub fn clear_guild_volume(guild_id: GuildId) {
    GUILD_VOLUMES.remove(&guild_id);
}

pub async fn execute(
    ctx: &Context,
    state: &AppState,
    guild_id: GuildId,
    user_id: UserId,
    operation: MusicOperation,
    query: Option<&str>,
) -> Result<String> {
    let manager = songbird::get(ctx)
        .await
        .ok_or_else(|| anyhow!("Songbird voice manager is unavailable"))?
        .clone();

    let validated_play_query = if matches!(operation, MusicOperation::Play) {
        let q = query
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("missing song title or URL"))?
            .trim();
        validate_source(state, q)?;
        Some(q)
    } else {
        None
    };

    let existing_handler = manager.get(guild_id);
    let is_connected = match &existing_handler {
        Some(handler) => {
            let h = handler.lock().await;
            h.current_channel().is_some()
        }
        None => false,
    };

    if !is_connected {
        match operation {
            MusicOperation::Leave => {
                let _ = manager.remove(guild_id).await;
                clear_guild_volume(guild_id);
                return Ok("The bot is not connected to a voice channel.".to_owned());
            }
            MusicOperation::Queue => {
                return Ok(
                    "The queue is empty. The bot is not connected to a voice channel.".to_owned(),
                );
            }
            MusicOperation::NowPlaying => {
                return Ok(
                    "Nothing is currently playing. The bot is not connected to a voice channel."
                        .to_owned(),
                );
            }
            MusicOperation::Play => {
                // Play will connect below
            }
            _ => {
                let _ = manager.remove(guild_id).await;
                clear_guild_volume(guild_id);
                return Err(anyhow!("The bot is not connected to a voice channel."));
            }
        }
    }

    let handler_lock = if is_connected {
        let handler = existing_handler.expect("handler must exist when is_connected is true");
        // If bot is connected, control actions require user to be in the same voice channel
        if !matches!(
            operation,
            MusicOperation::Queue | MusicOperation::NowPlaying
        ) {
            let user_channel = user_voice_channel(ctx, guild_id, user_id)?;
            let bot_channel = {
                let h = handler.lock().await;
                h.current_channel()
                    .map(|ch| serenity::all::ChannelId::new(ch.0.get()))
                    .or_else(|| {
                        let bot_id = ctx.cache.current_user().id;
                        ctx.cache
                            .guild(guild_id)
                            .and_then(|g| g.voice_states.get(&bot_id).and_then(|vs| vs.channel_id))
                    })
            };
            if let Some(bot_channel) = bot_channel
                && user_channel != bot_channel
            {
                return Err(anyhow!("you must be in the same voice channel as the bot"));
            }
        }

        if matches!(operation, MusicOperation::Leave) {
            manager.remove(guild_id).await?;
            clear_guild_volume(guild_id);
            return Ok("Disconnected bot from the voice channel.".to_owned());
        }

        handler
    } else {
        let _ = manager.remove(guild_id).await;
        let channel_id = user_voice_channel(ctx, guild_id, user_id)?;
        match manager.join(guild_id, channel_id).await {
            Ok(h) => h,
            Err(err) => {
                let _ = manager.remove(guild_id).await;
                return Err(err).context("failed to join the voice channel");
            }
        }
    };

    let guild_key = guild_id.get().to_string();
    let configured_default = if matches!(operation, MusicOperation::Play) {
        let settings = state
            .storage
            .get_module_settings(&guild_key, "music")
            .await?;
        settings
            .as_ref()
            .and_then(|s| s.config.get("default_volume"))
            .and_then(serde_json::Value::as_f64)
    } else {
        None
    };

    let mut handler = handler_lock.lock().await;

    match operation {
        MusicOperation::Play => {
            let query = validated_play_query.expect("query was validated");

            if handler.queue().len() >= MAX_QUEUE_SIZE {
                return Err(anyhow!(
                    "playback queue reached maximum limit of {MAX_QUEUE_SIZE} tracks"
                ));
            }

            let source = if query.starts_with("https://") {
                YoutubeDl::new(state.media_http_client.clone(), query.to_owned())
            } else {
                YoutubeDl::new_search(state.media_http_client.clone(), query.to_owned())
            };
            let track_volume = resolve_effective_volume(guild_id, configured_default);
            let track = handler.enqueue_input(source.into()).await;
            let _ = track.set_volume(track_volume);
            Ok(format!(
                "Added to queue. Track count: {}.",
                handler.queue().len()
            ))
        }
        MusicOperation::Volume => {
            let level = query
                .and_then(|q| q.trim().parse::<f32>().ok())
                .filter(|&v| (0.0..=150.0).contains(&v))
                .ok_or_else(|| anyhow!("volume level must be a number between 0 and 150"))?;
            let factor = level / 100.0;
            set_guild_volume(guild_id, level);
            if let Some(track) = handler.queue().current() {
                let _ = track.set_volume(factor);
            }
            Ok(format!("Player volume set to {level:.0}%."))
        }
        MusicOperation::Loop => {
            if let Some(track) = handler.queue().current() {
                let info = track.get_info().await?;
                let enabled = match info.loops {
                    songbird::tracks::LoopState::Infinite => {
                        track.disable_loop()?;
                        false
                    }
                    _ => {
                        track.enable_loop()?;
                        true
                    }
                };
                Ok(if enabled {
                    "Enabled looping for current track.".to_owned()
                } else {
                    "Disabled looping for current track.".to_owned()
                })
            } else {
                Err(anyhow!("no active track to loop"))
            }
        }
        MusicOperation::Pause => {
            handler.queue().pause()?;
            Ok("Paused playback.".to_owned())
        }
        MusicOperation::Resume => {
            handler.queue().resume()?;
            Ok("Resumed playback.".to_owned())
        }
        MusicOperation::Skip => {
            handler.queue().skip()?;
            Ok("Skipped current track.".to_owned())
        }
        MusicOperation::Stop => {
            handler.queue().stop();
            clear_guild_volume(guild_id);
            Ok("Stopped playback and cleared the queue.".to_owned())
        }
        MusicOperation::Queue => Ok(format!(
            "The queue contains {} tracks.",
            handler.queue().len()
        )),
        MusicOperation::NowPlaying => {
            if let Some(track) = handler.queue().current() {
                let info = track.get_info().await?;
                let position_secs = info.position.as_secs();
                let pos_min = position_secs / 60;
                let pos_sec = position_secs % 60;
                let state_str = match info.playing {
                    songbird::tracks::PlayMode::Play => "Playing",
                    songbird::tracks::PlayMode::Pause => "Paused",
                    songbird::tracks::PlayMode::Stop => "Stopped",
                    songbird::tracks::PlayMode::End => "Ended",
                    _ => "Playing",
                };
                let track_vol = (info.volume * 100.0).round() as u32;
                Ok(format!(
                    "Now {state_str}: Track at {pos_min:02}:{pos_sec:02} (Volume: {track_vol}%)."
                ))
            } else {
                Ok("Nothing is currently playing.".to_owned())
            }
        }
        MusicOperation::Leave => unreachable!("leave is handled before taking the call lock"),
    }
}

fn user_voice_channel(
    ctx: &Context,
    guild_id: GuildId,
    user_id: UserId,
) -> Result<serenity::all::ChannelId> {
    let guild = ctx
        .cache
        .guild(guild_id)
        .ok_or_else(|| anyhow!("server is not yet in cache"))?;

    guild
        .voice_states
        .get(&user_id)
        .and_then(|voice_state| voice_state.channel_id)
        .ok_or_else(|| anyhow!("join a voice channel first"))
}

pub fn is_blocked_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            // 0.0.0.0/8
            if octets[0] == 0 {
                return true;
            }
            // 127.0.0.0/8 (loopback)
            if ipv4.is_loopback() {
                return true;
            }
            // 10.0.0.0/8 (private)
            if octets[0] == 10 {
                return true;
            }
            // 172.16.0.0/12 (private: 172.16.0.0 - 172.31.255.255)
            if octets[0] == 172 && (16..=31).contains(&octets[1]) {
                return true;
            }
            // 192.168.0.0/16 (private)
            if octets[0] == 192 && octets[1] == 168 {
                return true;
            }
            // 169.254.0.0/16 (link-local and cloud metadata like 169.254.169.254)
            if ipv4.is_link_local() || (octets[0] == 169 && octets[1] == 254) {
                return true;
            }
            // 100.64.0.0/10 (carrier-grade NAT)
            if octets[0] == 100 && (64..=127).contains(&octets[1]) {
                return true;
            }
            // 192.0.0.0/24 (IETF protocol assignments)
            if octets[0] == 192 && octets[1] == 0 && octets[2] == 0 {
                return true;
            }
            // Multicast 224.0.0.0/4 and Reserved 240.0.0.0/4
            if ipv4.is_multicast() || octets[0] >= 240 {
                return true;
            }
            // Broadcast 255.255.255.255
            if ipv4.is_broadcast() {
                return true;
            }
            false
        }
        std::net::IpAddr::V6(ipv6) => {
            if ipv6.is_loopback() || ipv6.is_unspecified() || ipv6.is_multicast() {
                return true;
            }
            let segments = ipv6.segments();
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
            // IPv4-mapped IPv6 address (::ffff:a.b.c.d) or IPv4-compatible (::a.b.c.d)
            if let Some(ipv4) = ipv6.to_ipv4_mapped().or_else(|| ipv6.to_ipv4()) {
                return is_blocked_ip(std::net::IpAddr::V4(ipv4));
            }
            false
        }
    }
}

pub fn validate_source(state: &AppState, query: &str) -> Result<()> {
    if query.chars().any(char::is_control) {
        return Err(anyhow!("zapytanie zawiera niedozwolone znaki kontrolne"));
    }
    if query.trim_start().starts_with('-') {
        return Err(anyhow!("zapytanie nie może zaczynać się od myślnika"));
    }

    if !query.contains("://") {
        if query.contains(':')
            && !query.starts_with("https://")
            && let Ok(parsed) = Url::parse(query)
            && parsed.scheme() != "https"
        {
            return Err(anyhow!("adres źródła muzyki musi używać HTTPS"));
        }
        return Ok(());
    }

    let url = Url::parse(query).context("nieprawidłowy adres źródła muzyki")?;
    if url.scheme() != "https" {
        return Err(anyhow!("adres źródła muzyki musi używać HTTPS"));
    }

    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("adres źródła muzyki nie zawiera hosta"))?
        .to_ascii_lowercase();

    // Check if the host is an IP literal targeting blocked ranges
    let ip = match url.host() {
        Some(url::Host::Ipv4(v4)) => Some(std::net::IpAddr::V4(v4)),
        Some(url::Host::Ipv6(v6)) => Some(std::net::IpAddr::V6(v6)),
        _ => host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .ok(),
    };
    if let Some(ip) = ip
        && is_blocked_ip(ip)
    {
        return Err(anyhow!(
            "adres źródła muzyki wskazuje na zablokowany prywatny adres IP: {host}"
        ));
    }

    let allowed = state
        .config
        .music_allowed_hosts
        .iter()
        .any(|candidate| host == *candidate || host.ends_with(&format!(".{candidate}")));
    if !allowed {
        return Err(anyhow!(
            "host {host} nie znajduje się na liście MUSIC_ALLOWED_HOSTS"
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn test_is_blocked_ip_detects_private_loopback_and_metadata() {
        // Loopback
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(127, 255, 255, 255))));
        assert!(is_blocked_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));

        // RFC 1918 Private ranges
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(172, 31, 255, 255))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));

        // Link-local & cloud metadata
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1))));

        // IPv6 Unique Local, Link-Local, Unspecified and Documentation
        assert!(is_blocked_ip(IpAddr::V6(Ipv6Addr::UNSPECIFIED)));
        assert!(is_blocked_ip(IpAddr::V6("fc00::1".parse().unwrap())));
        assert!(is_blocked_ip(IpAddr::V6("fe80::1".parse().unwrap())));
        assert!(is_blocked_ip(IpAddr::V6("2001:db8::1".parse().unwrap())));

        // IPv6-mapped IPv4 addresses (::ffff:x.x.x.x)
        assert!(is_blocked_ip(IpAddr::V6(
            "::ffff:127.0.0.1".parse().unwrap()
        )));
        assert!(is_blocked_ip(IpAddr::V6(
            "::ffff:10.0.0.1".parse().unwrap()
        )));
        assert!(is_blocked_ip(IpAddr::V6(
            "::ffff:172.16.0.1".parse().unwrap()
        )));
        assert!(is_blocked_ip(IpAddr::V6(
            "::ffff:192.168.1.1".parse().unwrap()
        )));
        assert!(is_blocked_ip(IpAddr::V6(
            "::ffff:169.254.169.254".parse().unwrap()
        )));

        // IPv4-compatible IPv6 addresses (::x.x.x.x)
        assert!(is_blocked_ip(IpAddr::V6("::127.0.0.1".parse().unwrap())));
        assert!(is_blocked_ip(IpAddr::V6("::10.0.0.1".parse().unwrap())));
        assert!(is_blocked_ip(IpAddr::V6(
            "::169.254.169.254".parse().unwrap()
        )));

        // Public internet IPs (must NOT be blocked)
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
        assert!(!is_blocked_ip(IpAddr::V6(
            "2001:4860:4860::8888".parse().unwrap()
        )));
        assert!(!is_blocked_ip(IpAddr::V6(
            "::ffff:8.8.8.8".parse().unwrap()
        )));
    }

    #[test]
    fn test_music_volume_resolution_isolation_and_lifecycle() {
        let guild_a = GuildId::new(990001);
        let guild_b = GuildId::new(990002);

        // Ensure clean state
        clear_guild_volume(guild_a);
        clear_guild_volume(guild_b);

        // 1. Without configuration or runtime volume, default to 0.5 (50%)
        assert_eq!(resolve_effective_volume(guild_a, None), 0.5);

        // 2. Custom guild default volume from module config
        assert_eq!(resolve_effective_volume(guild_a, Some(0.35)), 0.35);

        // 3. Invalid or out-of-bounds configured volume falls back to 0.5
        assert_eq!(resolve_effective_volume(guild_a, Some(-0.2)), 0.5);
        assert_eq!(resolve_effective_volume(guild_a, Some(1.5)), 0.5);
        assert_eq!(resolve_effective_volume(guild_a, Some(f64::NAN)), 0.5);

        // 4. Runtime /music volume command sets active volume (e.g. 80%)
        set_guild_volume(guild_a, 80.0);
        assert_eq!(resolve_effective_volume(guild_a, Some(0.35)), 0.8);

        // 5. Multi-guild isolation: Guild B is unaffected by Guild A's runtime volume
        assert_eq!(resolve_effective_volume(guild_b, Some(0.35)), 0.35);
        assert_eq!(resolve_effective_volume(guild_b, None), 0.5);

        // 6. Stop/Leave clears runtime volume, reverting Guild A to its configured default
        clear_guild_volume(guild_a);
        assert_eq!(resolve_effective_volume(guild_a, Some(0.35)), 0.35);
        assert_eq!(resolve_effective_volume(guild_a, None), 0.5);
    }
}
