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
pub struct TagExecutionContext {
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

pub fn validate_tag_execution(
    module_id: &str,
    execution: &TagExecutionContext,
    operation: &TagOperation,
) -> Result<GuildId> {
    ensure!(
        module_id == "tags",
        "tag actions require the trusted tags module"
    );

    let guild_id = execution
        .guild_id
        .ok_or_else(|| anyhow!("Operacje na tagach są dostępne wyłącznie na serwerze"))?;

    if matches!(
        operation,
        TagOperation::Create { .. } | TagOperation::Delete { .. }
    ) {
        ensure!(
            execution.enforce_actor_permissions && execution.actor_id.is_some(),
            "tag mutations require an interactive authorized actor"
        );
        ensure!(
            can_manage_tags(execution.actor_permissions),
            "Tworzenie i usuwanie tagów wymaga uprawnienia Zarządzanie wiadomościami lub Administrator."
        );
    }

    Ok(guild_id)
}

pub async fn execute_tag_action(
    _ctx: &Context,
    state: &AppState,
    module_id: &str,
    execution: TagExecutionContext,
    operation: &TagOperation,
) -> Result<Option<String>> {
    let guild_id = validate_tag_execution(module_id, &execution, operation)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_can_manage_tags() {
        assert!(can_manage_tags(Permissions::ADMINISTRATOR));
        assert!(can_manage_tags(Permissions::MANAGE_MESSAGES));
        assert!(can_manage_tags(Permissions::all()));
        assert!(!can_manage_tags(Permissions::MANAGE_CHANNELS));
        assert!(!can_manage_tags(Permissions::SEND_MESSAGES));
        assert!(!can_manage_tags(Permissions::empty()));
    }

    #[test]
    fn test_validate_tag_execution_untrusted_module() {
        let execution = TagExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(200)),
            Some(UserId::new(300)),
            Permissions::MANAGE_MESSAGES,
            Permissions::all(),
            true,
        );
        let op = TagOperation::Get {
            name: "test".to_string(),
        };

        let res = validate_tag_execution("community", &execution, &op);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("trusted tags module"));
    }

    #[test]
    fn test_validate_tag_execution_create_delete_non_interactive_rejected() {
        // Missing actor
        let exec_no_actor = TagExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(200)),
            None,
            Permissions::MANAGE_MESSAGES,
            Permissions::all(),
            true,
        );
        let op_create = TagOperation::Create {
            name: "rule1".to_string(),
            content: "Be nice".to_string(),
        };
        let res = validate_tag_execution("tags", &exec_no_actor, &op_create);
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("interactive authorized actor")
        );

        // Event origin (enforce_actor_permissions = false)
        let exec_event = TagExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(200)),
            Some(UserId::new(300)),
            Permissions::MANAGE_MESSAGES,
            Permissions::all(),
            false,
        );
        let op_delete = TagOperation::Delete {
            name: "rule1".to_string(),
        };
        let res_del = validate_tag_execution("tags", &exec_event, &op_delete);
        assert!(res_del.is_err());
        assert!(
            res_del
                .unwrap_err()
                .to_string()
                .contains("interactive authorized actor")
        );
    }

    #[test]
    fn test_validate_tag_execution_missing_permission_rejected() {
        let exec_low_perms = TagExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(200)),
            Some(UserId::new(300)),
            Permissions::SEND_MESSAGES,
            Permissions::all(),
            true,
        );
        let op = TagOperation::Create {
            name: "info".to_string(),
            content: "Some info".to_string(),
        };
        let res = validate_tag_execution("tags", &exec_low_perms, &op);
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("Zarządzanie wiadomościami lub Administrator")
        );
    }

    #[test]
    fn test_validate_tag_execution_get_allowed_without_actor_permission() {
        // Get operation is read-only and does not require MANAGE_MESSAGES
        let exec_reader = TagExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(200)),
            Some(UserId::new(300)),
            Permissions::empty(),
            Permissions::all(),
            true,
        );
        let op = TagOperation::Get {
            name: "rules".to_string(),
        };
        let res = validate_tag_execution("tags", &exec_reader, &op);
        assert_eq!(res.unwrap(), GuildId::new(100));
    }

    #[test]
    fn test_validate_tag_execution_valid_mutations() {
        let exec_admin = TagExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(200)),
            Some(UserId::new(300)),
            Permissions::MANAGE_MESSAGES,
            Permissions::all(),
            true,
        );
        let op_create = TagOperation::Create {
            name: "rules".to_string(),
            content: "No spam".to_string(),
        };
        let res_create = validate_tag_execution("tags", &exec_admin, &op_create);
        assert_eq!(res_create.unwrap(), GuildId::new(100));

        let op_del = TagOperation::Delete {
            name: "rules".to_string(),
        };
        let res_del = validate_tag_execution("tags", &exec_admin, &op_del);
        assert_eq!(res_del.unwrap(), GuildId::new(100));
    }
}
