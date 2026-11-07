use anyhow::{Context as _, Result, anyhow, ensure};
use chrono::Utc;
use serenity::all::{
    ChannelId, CreateAllowedMentions, CreateMessage, GuildId, Permissions, UserId,
};
use serenity::client::Context;

use crate::AppState;
use crate::lua::AfkOperation;
use crate::storage::{AfkRecord, validate_snowflake};

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
            let mut seen = std::collections::HashSet::new();
            for mid in mentioned_user_ids {
                // Wyklucz samego autora i zduplikowane wzmianki w tej samej wiadomości
                if mid == &effective_author_id || !seen.insert(mid) {
                    continue;
                }

                if let Ok(Some(afk_record)) = state.storage.get_afk(&guild_id_str, mid).await {
                    let notice = format!(
                        "ℹ️ <@{}> jest obecnie AFK od <t:{}:R>: **{}**",
                        afk_record.user_id,
                        afk_record.set_at.timestamp(),
                        afk_record.reason
                    );
                    if let Err(e) = channel_id
                        .send_message(
                            &ctx.http,
                            CreateMessage::new()
                                .content(notice)
                                .allowed_mentions(CreateAllowedMentions::new()),
                        )
                        .await
                    {
                        tracing::warn!("Nie udało się wysłać powiadomienia o wspomnieniu AFK: {e}");
                    }
                }
            }

            Ok(None)
        }
    }
}
