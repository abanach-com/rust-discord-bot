use anyhow::{Context as _, Result, anyhow, ensure};
use chrono::Utc;
use serenity::all::{
    ChannelId, CreateAllowedMentions, CreateMessage, GuildId, Permissions, UserId,
};
use serenity::client::Context;

use crate::AppState;
use crate::lua::AfkOperation;
use crate::rate_limit::RateLimitQuota;
use crate::storage::{AfkRecord, validate_snowflake};

/// Bezpiecznie przycina ciąg znaków UTF-8 bez rozcinania znaków wielobajtowych (Unicode scalar values).
/// Jeśli długość bajtowa przekracza `max_bytes`, obcina ciąg na granicy znaku Unicode,
/// tak aby `safe_end + suffix.len() <= max_bytes`, po czym dopisuje `suffix`.
pub fn truncate_to_utf8_boundary(s: &mut String, max_bytes: usize, suffix: &str) {
    if s.len() <= max_bytes {
        return;
    }
    let budget = max_bytes.saturating_sub(suffix.len());
    let safe_end = s
        .char_indices()
        .take_while(|(start, ch)| start + ch.len_utf8() <= budget)
        .map(|(start, ch)| start + ch.len_utf8())
        .last()
        .unwrap_or(0);
    s.truncate(safe_end);
    s.push_str(suffix);
}

#[derive(Clone, Copy)]
pub struct AfkExecutionContext {
    pub guild_id: Option<GuildId>,
    pub channel_id: Option<ChannelId>,
    pub actor_id: Option<UserId>,
    pub actor_permissions: Permissions,
    pub enforce_actor_permissions: bool,
}

impl AfkExecutionContext {
    pub fn new(
        guild_id: Option<GuildId>,
        channel_id: Option<ChannelId>,
        actor_id: Option<UserId>,
        actor_permissions: Permissions,
        enforce_actor_permissions: bool,
    ) -> Self {
        Self {
            guild_id,
            channel_id,
            actor_id,
            actor_permissions,
            enforce_actor_permissions,
        }
    }
}

pub async fn execute_afk_action(
    ctx: &Context,
    state: &AppState,
    module_id: &str,
    execution: AfkExecutionContext,
    operation: &AfkOperation,
) -> Result<Option<String>> {
    ensure!(
        module_id == "afk",
        "Akcja AFK jest dozwolona wyłącznie dla modułu afk"
    );

    let guild_id = execution
        .guild_id
        .ok_or_else(|| anyhow!("Operacje AFK wymagają serwera Discord"))?;
    let guild_id_str = guild_id.get().to_string();

    match operation {
        AfkOperation::Set { reason } => {
            let actor_id = execution
                .actor_id
                .ok_or_else(|| anyhow!("Brak tożsamości wykonawcy akcji AFK"))?;
            let actor_id_str = actor_id.get().to_string();

            let trimmed_reason = match reason {
                Some(r) if !r.trim().is_empty() => {
                    let t = r.trim();
                    if t.chars().count() > 256 {
                        t.chars().take(256).collect::<String>()
                    } else {
                        t.to_string()
                    }
                }
                _ => "AFK".to_string(),
            };

            let record = AfkRecord {
                guild_id: guild_id_str.clone(),
                user_id: actor_id_str.clone(),
                reason: trimmed_reason.clone(),
                set_at: Utc::now(),
            };

            state
                .storage
                .set_afk(record)
                .await
                .context("Błąd zapisu statusu AFK")?;

            let _ = state
                .storage
                .record_audit(
                    Some(&guild_id_str),
                    Some(&actor_id_str),
                    module_id,
                    "afk_set",
                    serde_json::json!({
                        "user_id": actor_id_str,
                        "reason": trimmed_reason,
                    }),
                )
                .await;

            Ok(Some(format!(
                "💤 Ustawiono Twój status AFK: **{}**",
                trimmed_reason
            )))
        }

        AfkOperation::Clear => {
            let actor_id = execution
                .actor_id
                .ok_or_else(|| anyhow!("Brak tożsamości wykonawcy akcji AFK"))?;
            let actor_id_str = actor_id.get().to_string();

            let cleared = state
                .storage
                .clear_afk(&guild_id_str, &actor_id_str)
                .await
                .context("Błąd czyszczenia statusu AFK")?;

            if cleared {
                let _ = state
                    .storage
                    .record_audit(
                        Some(&guild_id_str),
                        Some(&actor_id_str),
                        module_id,
                        "afk_clear",
                        serde_json::json!({ "user_id": actor_id_str }),
                    )
                    .await;

                Ok(Some("Twój status AFK został wyłączony.".to_string()))
            } else {
                Ok(Some("Nie masz obecnie aktywnego statusu AFK.".to_string()))
            }
        }

        AfkOperation::Status { user_id } => {
            let target_id = match user_id {
                Some(uid) => {
                    validate_snowflake("user_id", uid)
                        .map_err(|e| anyhow!("Nieprawidłowe ID użytkownika: {e}"))?;
                    uid.clone()
                }
                None => {
                    let actor_id = execution
                        .actor_id
                        .ok_or_else(|| anyhow!("Brak tożsamości wykonawcy akcji AFK"))?;
                    actor_id.get().to_string()
                }
            };

            let afk = state
                .storage
                .get_afk(&guild_id_str, &target_id)
                .await
                .context("Błąd pobierania statusu AFK")?;

            match afk {
                Some(record) => Ok(Some(format!(
                    "💤 <@{}> jest AFK od <t:{}:R>: **{}**",
                    record.user_id,
                    record.set_at.timestamp(),
                    record.reason
                ))),
                None => Ok(Some(format!(
                    "Użytkownik <@{target_id}> nie ma obecnie aktywnego statusu AFK."
                ))),
            }
        }

        AfkOperation::OnMessage {
            author_id,
            mentioned_user_ids,
            auto_clear,
        } => {
            let channel_id = execution
                .channel_id
                .ok_or_else(|| anyhow!("Akcja on_message AFK wymaga kanału Discord"))?;

            // 1. Sprawdzenie autora wiadomości: jeśli auto_clear i autor był AFK, zdejmij status
            let effective_author_id = execution
                .actor_id
                .map(|id| id.get().to_string())
                .unwrap_or_else(|| author_id.clone());

            if *auto_clear {
                let cleared = state
                    .storage
                    .clear_afk(&guild_id_str, &effective_author_id)
                    .await
                    .context("Błąd auto-clear AFK")?;

                if cleared {
                    let _ = state
                        .storage
                        .record_audit(
                            Some(&guild_id_str),
                            Some(&effective_author_id),
                            module_id,
                            "afk_auto_clear",
                            serde_json::json!({ "user_id": effective_author_id }),
                        )
                        .await;

                    let welcome_back = format!(
                        "👋 Witaj z powrotem <@{effective_author_id}>! Twój status AFK został wyłączony."
                    );
                    if let Err(e) = channel_id
                        .send_message(
                            &ctx.http,
                            CreateMessage::new()
                                .content(welcome_back)
                                .allowed_mentions(CreateAllowedMentions::new()),
                        )
                        .await
                    {
                        tracing::warn!("Nie udało się wysłać powiadomienia o powrocie z AFK: {e}");
                    }
                }
            }

            // 2. Sprawdzenie wspomnianych użytkowników
            const MAX_MENTIONS_PER_AFK_NOTICE: usize = 5;
            let mut seen = std::collections::HashSet::new();
            let mut candidate_ids = Vec::new();

            for mid in mentioned_user_ids {
                // Wyklucz samego autora i zduplikowane wzmianki w tej samej wiadomości
                if mid != &effective_author_id && seen.insert(mid.clone()) {
                    candidate_ids.push(mid.clone());
                    if candidate_ids.len() >= MAX_MENTIONS_PER_AFK_NOTICE {
                        break;
                    }
                }
            }

            if candidate_ids.is_empty() {
                return Ok(None);
            }

            const MAX_DISCORD_MESSAGE_LEN: usize = 2000;
            let mut selected_notices = Vec::new();
            let mut selected_target_keys = Vec::new();
            let mut current_total_len = 0;
            let target_quota = RateLimitQuota::new(1, 60);

            for mid in &candidate_ids {
                let target_key = format!(
                    "afk:target:guild:{guild_id_str}:channel:{}:user:{mid}",
                    channel_id.get()
                );

                if state
                    .rate_limiter
                    .is_rate_limited(&target_key, target_quota)
                {
                    continue;
                }

                if let Ok(Some(afk_record)) = state.storage.get_afk(&guild_id_str, mid).await {
                    let mut notice = format!(
                        "ℹ️ <@{}> jest obecnie AFK od <t:{}:R>: **{}**",
                        afk_record.user_id,
                        afk_record.set_at.timestamp(),
                        afk_record.reason
                    );

                    if selected_notices.is_empty() {
                        if notice.len() > MAX_DISCORD_MESSAGE_LEN {
                            truncate_to_utf8_boundary(&mut notice, MAX_DISCORD_MESSAGE_LEN, "...");
                        }
                        current_total_len = notice.len();
                        selected_notices.push(notice);
                        selected_target_keys.push(target_key);
                    } else if current_total_len + 1 + notice.len() <= MAX_DISCORD_MESSAGE_LEN {
                        current_total_len += 1 + notice.len();
                        selected_notices.push(notice);
                        selected_target_keys.push(target_key);
                    } else {
                        tracing::debug!(
                            guild_id = %guild_id_str,
                            user_id = %mid,
                            "Powiadomienie AFK pominięte: łączne powiadomienie przekracza limit długości Discorda"
                        );
                    }
                }
            }

            if selected_notices.is_empty() {
                return Ok(None);
            }

            let channel_rate_key = format!(
                "afk:event:guild:{guild_id_str}:channel:{}",
                channel_id.get()
            );
            let author_rate_key =
                format!("afk:event:guild:{guild_id_str}:author:{effective_author_id}");

            let rate_checks = [
                (&channel_rate_key[..], RateLimitQuota::new(2, 30)),
                (&author_rate_key[..], RateLimitQuota::new(3, 30)),
            ];

            let decision = state.rate_limiter.check_multi_rate_limit(&rate_checks);
            if !decision.allowed {
                state
                    .metrics
                    .afk_notifications_suppressed_total
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                state
                    .metrics
                    .rate_limit_throttled_total
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                tracing::debug!(
                    guild_id = %guild_id_str,
                    channel_id = %channel_id.get(),
                    author_id = %effective_author_id,
                    "AFK notice throttled by rate limiter"
                );

                let _ = state
                    .storage
                    .record_audit(
                        Some(&guild_id_str),
                        Some(&effective_author_id),
                        module_id,
                        "afk_notice_throttled",
                        serde_json::json!({
                            "channel_id": channel_id.get().to_string(),
                            "suppressed_notices": selected_notices.len(),
                        }),
                    )
                    .await;

                return Ok(None);
            }

            // Konsumuj cooldown per-target wyłącznie dla użytkowników, których powiadomienie faktycznie wysyłamy
            for key in selected_target_keys {
                state.rate_limiter.check_rate_limit(&key, target_quota);
            }

            let mut content = selected_notices.join("\n");
            if content.len() > MAX_DISCORD_MESSAGE_LEN {
                truncate_to_utf8_boundary(&mut content, MAX_DISCORD_MESSAGE_LEN, "...");
            }

            if let Err(e) = channel_id
                .send_message(
                    &ctx.http,
                    CreateMessage::new()
                        .content(content)
                        .allowed_mentions(CreateAllowedMentions::new()),
                )
                .await
            {
                tracing::warn!("Nie udało się wysłać powiadomienia o wspomnieniu AFK: {e}");
            }

            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_to_utf8_boundary_ascii_within_limit() {
        let mut s = "hello world".to_string();
        truncate_to_utf8_boundary(&mut s, 20, "...");
        assert_eq!(s, "hello world");
    }

    #[test]
    fn test_truncate_to_utf8_boundary_ascii_exceeding_limit() {
        let mut s = "abcdefghij".to_string();
        truncate_to_utf8_boundary(&mut s, 7, "...");
        assert_eq!(s, "abcd...");
        assert_eq!(s.len(), 7);
    }

    #[test]
    fn test_truncate_to_utf8_boundary_multibyte_emoji_never_panics() {
        // "🫠" is 4 bytes: [0xF0, 0x9F, 0xAB, 0xA0]
        let mut s = "🫠".repeat(500); // 2000 bytes
        assert_eq!(s.len(), 2000);

        // Truncate at an arbitrary byte limit that cuts through a 4-byte emoji
        truncate_to_utf8_boundary(&mut s, 1997, "...");
        assert!(s.ends_with("..."));
        assert!(s.len() <= 1997);
        // Ensure string is valid UTF-8 and no char was sliced
        let _ = s.chars().count();
    }

    #[test]
    fn test_truncate_to_utf8_boundary_polish_chars() {
        // "zażółć gęślą jaźń"
        let mut s = "zażółć gęślą jaźń".repeat(100);
        let orig_len = s.len();
        truncate_to_utf8_boundary(&mut s, 50, "...");
        assert!(s.ends_with("..."));
        assert!(s.len() <= 50);
        let _ = s.chars().count();
        assert!(orig_len > 50);
    }

    #[test]
    fn test_truncate_to_utf8_boundary_small_budget() {
        let mut s = "🫠🫠🫠".to_string();
        truncate_to_utf8_boundary(&mut s, 3, "...");
        assert_eq!(s, "...");
        assert_eq!(s.len(), 3);
    }
}
