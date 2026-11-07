use anyhow::{Context as _, Result, anyhow, ensure};
use chrono::Utc;
use serenity::{
    all::{ChannelId, GuildId, Permissions, UserId},
    client::Context,
};

use crate::{
    AppState,
    lua::{TagOperation, normalize_tag_name},
    storage::TagRecord,
};

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub(super) struct TagExecutionContext {
    pub guild_id: Option<GuildId>,
    pub channel_id: Option<ChannelId>,
    pub actor_id: Option<UserId>,
    pub actor_permissions: Permissions,
    pub app_permissions: Permissions,
    pub enforce_actor_permissions: bool,
}

impl TagExecutionContext {
    #[must_use]
    pub const fn new(
        guild_id: Option<GuildId>,
        channel_id: Option<ChannelId>,
        actor_id: Option<UserId>,
        actor_permissions: Permissions,
        app_permissions: Permissions,
        enforce_actor_permissions: bool,
    ) -> Self {
        Self {
            guild_id,
            channel_id,
            actor_id,
            actor_permissions,
            app_permissions,
            enforce_actor_permissions,
        }
    }
}

pub fn can_manage_tags(permissions: Permissions) -> bool {
    permissions.contains(Permissions::ADMINISTRATOR)
        || permissions.contains(Permissions::MANAGE_MESSAGES)
}

pub(super) async fn execute_tag_action(
    _ctx: &Context,
    state: &AppState,
    module_id: &str,
    execution: TagExecutionContext,
    operation: &TagOperation,
) -> Result<Option<String>> {
    let guild_id = execution
        .guild_id
        .ok_or_else(|| anyhow!("Operacje na tagach są dostępne wyłącznie na serwerze"))?;
    let guild_id_str = guild_id.get().to_string();
    let actor_id_str = execution.actor_id.map(|a| a.get().to_string());

    match operation {
        TagOperation::Get { name } => {
            let norm = normalize_tag_name(name);
            let tag = state
                .storage
                .get_tag(&guild_id_str, &norm)
                .await
                .context("Błąd odczytu tagu ze storage")?;

            match tag {
                Some(record) => Ok(Some(record.content)),
                None => Ok(Some(format!("Nie znaleziono tagu `{name}`."))),
            }
        }

        TagOperation::Create { name, content } => {
            if execution.enforce_actor_permissions {
                ensure!(
                    can_manage_tags(execution.actor_permissions),
                    "Tworzenie tagów wymaga uprawnienia Zarządzanie wiadomościami lub Administrator."
                );
            }

            let norm = normalize_tag_name(name);
            let tag_record = TagRecord {
                name: norm.clone(),
                content: content.clone(),
                author_id: actor_id_str.clone().unwrap_or_else(|| "0".to_string()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };

            state
                .storage
                .set_tag(&guild_id_str, tag_record)
                .await
                .context("Błąd zapisu tagu w storage")?;

            let audit_payload = serde_json::json!({
                "tag_name": norm,
                "content_len": content.len(),
            });

            let _ = state
                .storage
                .record_audit(
                    Some(&guild_id_str),
                    actor_id_str.as_deref(),
                    module_id,
                    "tag_create",
                    audit_payload,
                )
                .await;

            Ok(Some(format!("Tag `{norm}` został pomyślnie utworzony!")))
        }

        TagOperation::Delete { name } => {
            if execution.enforce_actor_permissions {
                ensure!(
                    can_manage_tags(execution.actor_permissions),
                    "Usuwanie tagów wymaga uprawnienia Zarządzanie wiadomościami lub Administrator."
                );
            }

            let norm = normalize_tag_name(name);
            let deleted = state
                .storage
                .delete_tag(&guild_id_str, &norm)
                .await
                .context("Błąd usuwania tagu ze storage")?;

            if deleted {
                let audit_payload = serde_json::json!({
                    "tag_name": norm,
                });

                let _ = state
                    .storage
                    .record_audit(
                        Some(&guild_id_str),
                        actor_id_str.as_deref(),
                        module_id,
                        "tag_delete",
                        audit_payload,
                    )
                    .await;

                Ok(Some(format!("Tag `{norm}` został pomyślnie usunięty!")))
            } else {
                Ok(Some(format!("Tag `{norm}` nie istnieje.")))
            }
        }
    }
}
