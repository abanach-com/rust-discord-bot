use anyhow::{Context as _, Result, anyhow, ensure};
use chrono::Utc;
use serenity::all::{ChannelId, GuildId, Permissions, UserId};
use serenity::client::Context;
use uuid::Uuid;

use crate::AppState;
use crate::lua::QuoteOperation;
use crate::storage::{QuoteRecord, validate_snowflake};

#[derive(Clone, Copy)]
pub struct QuoteExecutionContext {
    pub guild_id: Option<GuildId>,
    pub channel_id: Option<ChannelId>,
    pub actor_id: Option<UserId>,
    pub actor_permissions: Permissions,
    pub enforce_actor_permissions: bool,
}

impl QuoteExecutionContext {
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

pub async fn execute_quote_action(
    _ctx: &Context,
    state: &AppState,
    module_id: &str,
    execution: QuoteExecutionContext,
    operation: &QuoteOperation,
) -> Result<Option<String>> {
    ensure!(
        module_id == "quotes",
        "Akcja cytatów jest dozwolona wyłącznie dla modułu quotes"
    );

    let guild_id = execution
        .guild_id
        .ok_or_else(|| anyhow!("Operacje na cytatach wymagają serwera Discord"))?;
    let guild_id_str = guild_id.get().to_string();

    match operation {
        QuoteOperation::Add { author_id, text } => {
            let actor_id = execution
                .actor_id
                .ok_or_else(|| anyhow!("Brak tożsamości wykonawcy cytatu"))?;
            let actor_id_str = actor_id.get().to_string();

            validate_snowflake("author_id", author_id)
                .map_err(|e| anyhow!("Nieprawidłowe ID autora cytatu: {e}"))?;

            let trimmed = text.trim();
            let len = trimmed.chars().count();
            if len == 0 || len > 500 {
                return Err(anyhow!("Treść cytatu musi zawierać od 1 do 500 znaków"));
            }

            let quote = QuoteRecord {
                id: Uuid::new_v4(),
                guild_id: guild_id_str.clone(),
                added_by: actor_id_str.clone(),
                author_id: author_id.clone(),
                body: trimmed.to_string(),
                created_at: Utc::now(),
            };

            let saved = state
                .storage
                .create_quote(quote)
                .await
                .context("Błąd zapisu cytatu w storage")?;

            let audit_payload = serde_json::json!({
                "quote_id": saved.id,
                "author_id": saved.author_id,
                "added_by": saved.added_by,
            });

            let _ = state
                .storage
                .record_audit(
                    Some(&guild_id_str),
                    Some(&actor_id_str),
                    module_id,
                    "quote_add",
                    audit_payload,
                )
                .await;

            Ok(Some(format!(
                "💬 Dodano cytat użytkownika <@{}>:\n> {}\n*(ID: `{}`)*",
                saved.author_id, saved.body, saved.id
            )))
        }

        QuoteOperation::Random => {
            let quote = state
                .storage
                .get_random_quote(&guild_id_str)
                .await
                .context("Błąd pobierania losowego cytatu")?;

            match quote {
                Some(q) => Ok(Some(format!(
                    "💬 > {}\n— <@{}> *(dodany przez <@{}>, ID: `{}`)*",
                    q.body, q.author_id, q.added_by, q.id
                ))),
                None => Ok(Some("Brak zapisanych cytatów na tym serwerze.".to_string())),
            }
        }

        QuoteOperation::List { limit } => {
            let quotes = state
                .storage
                .list_quotes(&guild_id_str, *limit)
                .await
                .context("Błąd pobierania listy cytatów")?;

            if quotes.is_empty() {
                return Ok(Some("Brak zapisanych cytatów na tym serwerze.".to_string()));
            }

            let mut out = format!(
                "📜 **Lista cytatów serwera (ostatnie {}):**\n",
                quotes.len()
            );
            for (i, q) in quotes.iter().enumerate() {
                out.push_str(&format!(
                    "{}. „{}” — <@{}> *(ID: `{}`)*\n",
                    i + 1,
                    q.body,
                    q.author_id,
                    q.id
                ));
            }
            Ok(Some(out))
        }

        QuoteOperation::Delete { quote_id } => {
            ensure!(
                execution.enforce_actor_permissions && execution.actor_id.is_some(),
                "Quote deletion requires an authenticated interactive actor"
            );
            let perms = execution.actor_permissions;
            ensure!(
                perms.contains(Permissions::MANAGE_MESSAGES)
                    || perms.contains(Permissions::ADMINISTRATOR),
                "Wymagane uprawnienie Zarządzanie wiadomościami (MANAGE_MESSAGES) lub Administrator"
            );

            let deleted = state
                .storage
                .delete_quote(&guild_id_str, *quote_id)
                .await
                .context("Błąd usuwania cytatu")?;

            if deleted {
                let actor_id_str = execution.actor_id.map(|a| a.get().to_string());
                let _ = state
                    .storage
                    .record_audit(
                        Some(&guild_id_str),
                        actor_id_str.as_deref(),
                        module_id,
                        "quote_delete",
                        serde_json::json!({ "quote_id": quote_id }),
                    )
                    .await;

                Ok(Some(format!("🗑️ Usunięto cytat `{quote_id}`.")))
            } else {
                Err(anyhow!("Nie znaleziono cytatu o ID {quote_id}"))
            }
        }
    }
}
