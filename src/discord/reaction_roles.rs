use anyhow::{Context as _, Result, anyhow, ensure};
use chrono::Utc;
use serenity::{
    all::{ChannelId, GuildId, Permissions, Reaction, ReactionType, RoleId, UserId},
    client::Context,
};
use uuid::Uuid;

use crate::{
    AppState,
    lua::ReactionRoleOperation,
    storage::{ReactionRoleBinding, validate_snowflake},
};

pub fn canonicalize_emoji(input: &str) -> String {
    let trimmed = input.trim();
    if (trimmed.starts_with("<:") || trimmed.starts_with("<a:"))
        && trimmed.ends_with('>')
        && let Some(last_colon) = trimmed.rfind(':')
    {
        let id_part = &trimmed[last_colon + 1..trimmed.len() - 1];
        if !id_part.is_empty() && id_part.chars().all(|c| c.is_ascii_digit()) {
            return id_part.to_string();
        }
    }
    if let Some((_name, id_part)) = trimmed.split_once(':')
        && !id_part.is_empty()
        && id_part.chars().all(|c| c.is_ascii_digit())
        && id_part.len() >= 17
    {
        return id_part.to_string();
    }
    if trimmed.chars().all(|c| c.is_ascii_digit()) && (17..=20).contains(&trimmed.len()) {
        return trimmed.to_string();
    }
    trimmed.to_string()
}

pub fn reaction_emoji_to_key(emoji: &ReactionType) -> String {
    match emoji {
        ReactionType::Unicode(s) => s.clone(),
        ReactionType::Custom { id, .. } => id.get().to_string(),
        _ => emoji.as_data(),
    }
}

#[derive(Clone, Copy)]
pub struct ReactionRoleExecutionContext {
    pub guild_id: Option<GuildId>,
    pub channel_id: Option<ChannelId>,
    pub actor_id: Option<UserId>,
    pub actor_permissions: Permissions,
    pub app_permissions: Permissions,
    pub enforce_actor_permissions: bool,
}

impl ReactionRoleExecutionContext {
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

pub fn can_manage_reaction_roles(permissions: Permissions) -> bool {
    permissions.contains(Permissions::ADMINISTRATOR)
        || permissions.contains(Permissions::MANAGE_ROLES)
}

pub fn validate_reaction_role_execution(
    module_id: &str,
    execution: &ReactionRoleExecutionContext,
    _operation: &ReactionRoleOperation,
) -> Result<GuildId> {
    ensure!(
        module_id == "reaction_roles",
        "reaction_role actions require the trusted reaction_roles module"
    );

    let guild_id = execution
        .guild_id
        .ok_or_else(|| anyhow!("Operacje na reaction roles są dostępne wyłącznie na serwerze"))?;

    ensure!(
        execution.enforce_actor_permissions && execution.actor_id.is_some(),
        "reaction_role mutations require an interactive authorized actor"
    );

    ensure!(
        can_manage_reaction_roles(execution.actor_permissions),
        "Tworzenie i usuwanie mapowań reaction role wymaga uprawnienia Zarządzanie rolami lub Administrator."
    );

    ensure!(
        can_manage_reaction_roles(execution.app_permissions),
        "Bot nie posiada uprawnienia Zarządzanie rolami na tym serwerze."
    );

    Ok(guild_id)
}

pub async fn execute_reaction_role_action(
    ctx: &Context,
    state: &AppState,
    module_id: &str,
    execution: ReactionRoleExecutionContext,
    operation: &ReactionRoleOperation,
) -> Result<Option<String>> {
    let guild_id = validate_reaction_role_execution(module_id, &execution, operation)?;
    let guild_id_str = guild_id.get().to_string();
    let actor_id_str = execution.actor_id.map(|a| a.get().to_string());

    match operation {
        ReactionRoleOperation::Create {
            message_id,
            emoji,
            role_id,
        } => {
            validate_snowflake("message_id", message_id)
                .map_err(|e| anyhow!("Nieprawidłowe ID wiadomości: {e}"))?;
            validate_snowflake("role_id", role_id)
                .map_err(|e| anyhow!("Nieprawidłowe ID roli: {e}"))?;

            let role_id_u64: u64 = role_id
                .parse()
                .map_err(|_| anyhow!("Nieprawidłowe ID roli"))?;
            let role_id_typed = RoleId::new(role_id_u64);

            crate::discord::executor::ensure_role_hierarchy(
                ctx,
                guild_id,
                execution.actor_id,
                role_id_typed,
            )
            .await?;

            let emoji_key = canonicalize_emoji(emoji);

            let binding = ReactionRoleBinding {
                id: Uuid::new_v4(),
                guild_id: guild_id_str.clone(),
                channel_id: execution.channel_id.map(|c| c.get().to_string()),
                message_id: message_id.clone(),
                emoji_key: emoji_key.clone(),
                role_id: role_id.clone(),
                created_by: actor_id_str.clone(),
                created_at: Utc::now(),
            };

            state
                .storage
                .upsert_reaction_role_binding(binding)
                .await
                .context("Błąd zapisu mapowania reaction role w storage")?;

            let audit_payload = serde_json::json!({
                "message_id": message_id,
                "emoji": emoji_key,
                "role_id": role_id,
            });

            let _ = state
                .storage
                .record_audit(
                    Some(&guild_id_str),
                    actor_id_str.as_deref(),
                    module_id,
                    "reaction_role_create",
                    audit_payload,
                )
                .await;

            Ok(Some(format!(
                "✅ Skonfigurowano rolę za reakcję: reakcja {emoji} nadaje rolę <@&{role_id}>."
            )))
        }

        ReactionRoleOperation::Remove { message_id, emoji } => {
            validate_snowflake("message_id", message_id)
                .map_err(|e| anyhow!("Nieprawidłowe ID wiadomości: {e}"))?;

            let emoji_key = canonicalize_emoji(emoji);

            let deleted = state
                .storage
                .delete_reaction_role_binding(&guild_id_str, message_id, &emoji_key)
                .await
                .context("Błąd usuwania mapowania reaction role ze storage")?;

            let audit_payload = serde_json::json!({
                "message_id": message_id,
                "emoji": emoji_key,
                "deleted": deleted,
            });

            let _ = state
                .storage
                .record_audit(
                    Some(&guild_id_str),
                    actor_id_str.as_deref(),
                    module_id,
                    "reaction_role_remove",
                    audit_payload,
                )
                .await;

            if deleted {
                Ok(Some(format!(
                    "✅ Usunięto powiązanie roli dla reakcji {emoji}."
                )))
            } else {
                Ok(Some(format!(
                    "Nie znaleziono powiązania roli dla wiadomości `{message_id}` i reakcji {emoji}."
                )))
            }
        }

        ReactionRoleOperation::List => {
            let bindings = state
                .storage
                .list_reaction_role_bindings(&guild_id_str)
                .await
                .context("Błąd pobierania listy reaction roles")?;

            if bindings.is_empty() {
                return Ok(Some(
                    "Brak aktywnych powiązań reaction role na tym serwerze.".to_string(),
                ));
            }

            let mut lines = vec!["**Aktywne powiązania Reaction Roles:**".to_string()];
            for b in bindings {
                lines.push(format!(
                    "• Wiadomość `{}`: reakcja `{}` ➔ <@&{}>",
                    b.message_id, b.emoji_key, b.role_id
                ));
            }

            Ok(Some(lines.join("\n")))
        }
    }
}

pub async fn handle_reaction_role_event(
    ctx: &Context,
    state: &AppState,
    reaction: &Reaction,
    is_add: bool,
) -> Result<()> {
    let Some(guild_id) = reaction.guild_id else {
        return Ok(());
    };
    let guild_id_str = guild_id.get().to_string();

    let Some(user_id) = reaction.user_id else {
        return Ok(());
    };

    let current_user_id = ctx.cache.current_user().id;
    if user_id.get() == current_user_id.get() {
        return Ok(());
    }

    let is_bot = if let Some(member) = &reaction.member {
        member.user.bot
    } else if let Ok(user) = reaction.user(&ctx.http).await {
        user.bot
    } else {
        false
    };
    if is_bot {
        return Ok(());
    }

    let is_enabled = match state
        .storage
        .get_module_settings(&guild_id_str, "reaction_roles")
        .await
    {
        Ok(Some(settings)) => settings.enabled,
        Ok(None) => {
            state
                .scripts
                .is_module_enabled_in_installer("reaction_roles")
                .await
        }
        Err(err) => {
            tracing::error!(?err, "failed to check reaction_roles module settings");
            false
        }
    };

    if !is_enabled {
        return Ok(());
    }

    let emoji_key = reaction_emoji_to_key(&reaction.emoji);
    let message_id_str = reaction.message_id.get().to_string();

    let binding = state
        .storage
        .get_reaction_role_binding(&guild_id_str, &message_id_str, &emoji_key)
        .await?;

    let Some(binding) = binding else {
        return Ok(());
    };

    let role_id_u64: u64 = binding
        .role_id
        .parse()
        .context("invalid role_id in reaction role binding")?;
    let role_id = RoleId::new(role_id_u64);

    if is_add {
        match ctx
            .http
            .add_member_role(guild_id, user_id, role_id, Some("Reaction role"))
            .await
        {
            Ok(()) => {
                let _ = state
                    .storage
                    .record_audit(
                        Some(&guild_id_str),
                        Some(&user_id.get().to_string()),
                        "reaction_roles",
                        "reaction_role_granted",
                        serde_json::json!({
                            "message_id": message_id_str,
                            "emoji": emoji_key,
                            "role_id": binding.role_id,
                        }),
                    )
                    .await;
            }
            Err(err) => {
                tracing::warn!(
                    ?err,
                    guild_id = %guild_id_str,
                    user_id = %user_id.get(),
                    role_id = %binding.role_id,
                    "failed to add reaction role to member"
                );
                let _ = state
                    .storage
                    .record_audit(
                        Some(&guild_id_str),
                        Some(&user_id.get().to_string()),
                        "reaction_roles",
                        "reaction_role_grant_failed",
                        serde_json::json!({
                            "message_id": message_id_str,
                            "emoji": emoji_key,
                            "role_id": binding.role_id,
                            "error": err.to_string(),
                        }),
                    )
                    .await;
            }
        }
    } else {
        match ctx
            .http
            .remove_member_role(guild_id, user_id, role_id, Some("Reaction role removed"))
            .await
        {
            Ok(()) => {
                let _ = state
                    .storage
                    .record_audit(
                        Some(&guild_id_str),
                        Some(&user_id.get().to_string()),
                        "reaction_roles",
                        "reaction_role_revoked",
                        serde_json::json!({
                            "message_id": message_id_str,
                            "emoji": emoji_key,
                            "role_id": binding.role_id,
                        }),
                    )
                    .await;
            }
            Err(err) => {
                tracing::warn!(
                    ?err,
                    guild_id = %guild_id_str,
                    user_id = %user_id.get(),
                    role_id = %binding.role_id,
                    "failed to remove reaction role from member"
                );
            }
        }
    }

    Ok(())
}
