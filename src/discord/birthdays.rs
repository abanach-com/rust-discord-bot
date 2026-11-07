use anyhow::{Context as _, Result, anyhow, ensure};
use chrono::Utc;
use serenity::all::{ChannelId, GuildId, Permissions, UserId};
use serenity::client::Context;

use crate::AppState;
use crate::lua::BirthdayOperation;
use crate::storage::{BirthdayRecord, validate_birthday_date, validate_snowflake};

#[derive(Clone, Copy)]
pub struct BirthdayExecutionContext {
    pub guild_id: Option<GuildId>,
    pub channel_id: Option<ChannelId>,
    pub actor_id: Option<UserId>,
    pub actor_permissions: Permissions,
    pub enforce_actor_permissions: bool,
}

impl BirthdayExecutionContext {
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

pub async fn execute_birthday_action(
    _ctx: &Context,
    state: &AppState,
    module_id: &str,
    execution: BirthdayExecutionContext,
    operation: &BirthdayOperation,
) -> Result<Option<String>> {
    ensure!(
        module_id == "birthdays",
        "Akcja urodzin jest dozwolona wyłącznie dla modułu birthdays"
    );

    let guild_id = execution
        .guild_id
        .ok_or_else(|| anyhow!("Operacje na urodzinach wymagają serwera Discord"))?;
    let guild_id_str = guild_id.get().to_string();

    match operation {
        BirthdayOperation::Set { day, month } => {
            let actor_id = execution
                .actor_id
                .ok_or_else(|| anyhow!("Brak tożsamości wykonawcy akcji urodzin"))?;
            let actor_id_str = actor_id.get().to_string();

            validate_birthday_date(*month as i16, *day as i16)
                .map_err(|e| anyhow!("Nieprawidłowa data urodzin: {e}"))?;

            let record = BirthdayRecord {
                guild_id: guild_id_str.clone(),
                user_id: actor_id_str.clone(),
                month: *month as i16,
                day: *day as i16,
                updated_at: Utc::now(),
            };

            let saved = state
                .storage
                .set_birthday(record)
                .await
                .context("Błąd zapisu urodzin w bazie danych")?;

            let audit_payload = serde_json::json!({
                "user_id": saved.user_id,
                "month": saved.month,
                "day": saved.day,
            });

            let _ = state
                .storage
                .record_audit(
                    Some(&guild_id_str),
                    Some(&actor_id_str),
                    module_id,
                    "birthday_set",
                    audit_payload,
                )
                .await;

            Ok(Some(format!(
                "🎂 Pomyślnie zapisano Twoje urodziny: **{:02}.{:02}**!",
                saved.day, saved.month
            )))
        }

        BirthdayOperation::Check { user_id } => {
            let target_user_id = match user_id {
                Some(uid) => {
                    validate_snowflake("user_id", uid)
                        .map_err(|e| anyhow!("Nieprawidłowe ID użytkownika: {e}"))?;
                    uid.clone()
                }
                None => {
                    let actor_id = execution
                        .actor_id
                        .ok_or_else(|| anyhow!("Brak tożsamości wykonawcy akcji urodzin"))?;
                    actor_id.get().to_string()
                }
            };

            let birthday = state
                .storage
                .get_birthday(&guild_id_str, &target_user_id)
                .await
                .context("Błąd pobierania urodzin")?;

            match birthday {
                Some(b) => Ok(Some(format!(
                    "🎂 Urodziny użytkownika <@{}>: **{:02}.{:02}**",
                    b.user_id, b.day, b.month
                ))),
                None => Ok(Some(format!(
                    "Nie znaleziono zapisanych urodzin dla <@{target_user_id}>."
                ))),
            }
        }

        BirthdayOperation::Upcoming { limit } => {
            let birthdays = state
                .storage
                .list_upcoming_birthdays(&guild_id_str, *limit)
                .await
                .context("Błąd pobierania nadchodzących urodzin")?;

            if birthdays.is_empty() {
                return Ok(Some(
                    "Brak nadchodzących urodzin na tym serwerze.".to_string(),
                ));
            }

            let mut out = format!(
                "🎉 **Nadchodzące urodziny na serwerze (najbliższe {}):**\n",
                birthdays.len()
            );
            for (i, b) in birthdays.iter().enumerate() {
                out.push_str(&format!(
                    "{}. <@{}> — **{:02}.{:02}**\n",
                    i + 1,
                    b.user_id,
                    b.day,
                    b.month
                ));
            }
            Ok(Some(out))
        }

        BirthdayOperation::Delete => {
            let actor_id = execution
                .actor_id
                .ok_or_else(|| anyhow!("Brak tożsamości wykonawcy akcji urodzin"))?;
            let actor_id_str = actor_id.get().to_string();

            let deleted = state
                .storage
                .delete_birthday(&guild_id_str, &actor_id_str)
                .await
                .context("Błąd usuwania urodzin")?;

            if deleted {
                let _ = state
                    .storage
                    .record_audit(
                        Some(&guild_id_str),
                        Some(&actor_id_str),
                        module_id,
                        "birthday_delete",
                        serde_json::json!({ "user_id": actor_id_str }),
                    )
                    .await;

                Ok(Some(
                    "🗑️ Twoje urodziny zostały usunięte z kalendarza.".to_string(),
                ))
            } else {
                Ok(Some(
                    "Nie masz zapisanych urodzin na tym serwerze.".to_string(),
                ))
            }
        }
    }
}
