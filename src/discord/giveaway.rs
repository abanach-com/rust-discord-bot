use anyhow::{Context as _, Result, anyhow, ensure};
use chrono::{Duration, Utc};
use serenity::{
    all::{ChannelId, ChannelType, GuildId, Permissions, UserId},
    builder::{CreateAllowedMentions, CreateMessage},
    client::Context,
};
use uuid::Uuid;

use super::executor::{ActionOrigin, ChannelPurpose, resolve_and_validate_channel};
use crate::{AppState, lua::GiveawayOperation};

#[derive(Clone, Copy)]
pub struct GiveawayExecutionContext {
    pub guild_id: Option<GuildId>,
    pub channel_id: Option<ChannelId>,
    pub actor_id: Option<UserId>,
    pub actor_permissions: Permissions,
    pub app_permissions: Permissions,
    pub enforce_actor_permissions: bool,
}

impl GiveawayExecutionContext {
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

pub fn can_manage_giveaways(permissions: Permissions) -> bool {
    permissions.contains(Permissions::ADMINISTRATOR)
        || permissions.contains(Permissions::MANAGE_GUILD)
}

fn format_winners(winners: &[String]) -> String {
    if winners.is_empty() {
        "Brak zwycięzców (brak uczestników)".to_string()
    } else {
        winners
            .iter()
            .map(|w| format!("<@{w}>"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub fn validate_giveaway_execution(
    module_id: &str,
    execution: &GiveawayExecutionContext,
) -> Result<GuildId> {
    ensure!(
        module_id == "giveaway",
        "giveaway operations are reserved for the trusted giveaway module"
    );
    ensure!(
        execution.enforce_actor_permissions && execution.actor_id.is_some(),
        "giveaway operations require an interactive user command"
    );
    ensure!(
        can_manage_giveaways(execution.actor_permissions),
        "giveaway operations require Manage Server or Administrator permissions."
    );

    execution
        .guild_id
        .ok_or_else(|| anyhow!("Giveaway operations operate exclusively within a server"))
}

pub async fn execute(
    ctx: &Context,
    state: &AppState,
    module_id: &str,
    execution: GiveawayExecutionContext,
    operation: &GiveawayOperation,
) -> Result<Option<String>> {
    let guild_id = validate_giveaway_execution(module_id, &execution)?;

    let guild_id_str = guild_id.get().to_string();
    let actor_id_str = execution.actor_id.map(|id| id.get().to_string());

    match operation {
        GiveawayOperation::Start {
            prize,
            duration_minutes,
            winners,
            channel_id,
        } => {
            ensure!(!prize.trim().is_empty(), "Giveaway prize cannot be empty");
            ensure!(
                prize.chars().count() <= 256,
                "Giveaway prize cannot exceed 256 characters"
            );
            ensure!(
                *duration_minutes >= 1 && *duration_minutes <= 10080,
                "Giveaway duration must be between 1 and 10080 minutes (7 days)"
            );
            ensure!(
                *winners >= 1 && *winners <= 20,
                "Giveaway winners count must be between 1 and 20"
            );

            let giveaway_settings = state
                .storage
                .get_module_settings(&guild_id_str, "giveaway")
                .await
                .ok()
                .flatten();
            let module_config = giveaway_settings
                .as_ref()
                .map(|s| &s.config)
                .unwrap_or(&serde_json::Value::Null);

            let configured_max_winners = module_config
                .get("max_winners")
                .and_then(|v| v.as_i64())
                .unwrap_or(10)
                .clamp(1, 20) as i32;

            ensure!(
                *winners <= configured_max_winners,
                "Liczba zwycięzców ({winners}) przekracza maksymalny limit skonfigurowany dla serwera ({configured_max_winners})"
            );

            let configured_channel = module_config
                .get("announcement_channel_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());

            let effective_channel_id = match channel_id {
                Some(c) if !c.trim().is_empty() => Some(c.as_str()),
                _ => configured_channel,
            };

            let origin = ActionOrigin {
                guild_id: execution.guild_id,
                channel_id: execution.channel_id,
                actor_id: execution.actor_id,
                actor_permissions: execution.actor_permissions,
                actor_roles: Vec::new(),
                app_permissions: execution.app_permissions,
                enforce_actor_permissions: execution.enforce_actor_permissions,
            };

            let target_channel = resolve_and_validate_channel(
                ctx,
                &origin,
                effective_channel_id,
                ChannelPurpose::SendMessage,
            )
            .await?;

            let expires_at = Utc::now() + Duration::minutes(*duration_minutes);
            let target_channel_str = target_channel.get().to_string();
            let ga_id = Uuid::new_v4();

            let end_unix = expires_at.timestamp();
            let host_mention = actor_id_str
                .as_deref()
                .map(|a| format!("<@{a}>"))
                .unwrap_or_else(|| "Organizator".to_string());

            let announcement = format!(
                "🎉 **NOWY KONKURS (GIVEAWAY)!** 🎉\n\n\
                🎁 **Nagroda:** {prize}\n\
                👥 **Liczba zwycięzców:** {winners}\n\
                ⏳ **Koniec:** <t:{end_unix}:R> (<t:{end_unix}:f>)\n\
                Host: {host_mention}\n\n\
                Aby dołączyć, użyj komendy lub dołącz przez panel!\n\
                ID losowania: `{ga_id}`"
            );

            let send_res = target_channel
                .send_message(
                    &ctx.http,
                    CreateMessage::new()
                        .content(announcement)
                        .allowed_mentions(CreateAllowedMentions::new()),
                )
                .await;

            let delivered = match send_res {
                Ok(msg) => msg,
                Err(e) => {
                    tracing::warn!(error = %e, "Could not post giveaway announcement message");
                    return Err(anyhow!(
                        "Nie udało się opublikować ogłoszenia konkursu na kanale <#{target_channel_str}>: {e}"
                    ));
                }
            };

            let giveaway = state
                .storage
                .create_giveaway_with_id(
                    ga_id,
                    &guild_id_str,
                    &target_channel_str,
                    prize,
                    *winners,
                    expires_at,
                    actor_id_str.as_deref(),
                )
                .await
                .context("Failed to create giveaway in database")?;

            let audit_payload = serde_json::json!({
                "giveaway_id": giveaway.id.to_string(),
                "prize": prize,
                "winners_count": winners,
                "duration_minutes": duration_minutes,
                "channel_id": target_channel_str,
                "message_id": delivered.id.to_string(),
            });

            let _ = state
                .storage
                .record_audit(
                    Some(&guild_id_str),
                    actor_id_str.as_deref(),
                    module_id,
                    "giveaway_start",
                    audit_payload,
                )
                .await;

            Ok(Some(format!(
                "🎉 Konkurs na **{prize}** został pomyślnie utworzony i ogłoszony!\nID: `{}`\nKanał: <#{target_channel_str}>",
                giveaway.id
            )))
        }

        GiveawayOperation::End { giveaway_id } => {
            let ga_uuid = Uuid::parse_str(giveaway_id)
                .map_err(|e| anyhow!("Invalid giveaway ID `{giveaway_id}`: {e}"))?;

            let ga = state
                .storage
                .get_giveaway(ga_uuid)
                .await
                .context("Failed to query giveaway")?
                .ok_or_else(|| anyhow!("Giveaway `{ga_uuid}` was not found in this guild"))?;

            if ga.guild_id != guild_id_str {
                return Err(anyhow!("Giveaway `{ga_uuid}` was not found in this guild"));
            }

            let winners = state
                .storage
                .finish_giveaway(ga_uuid)
                .await
                .context("Failed to end giveaway")?;

            let send_res = if let Ok(chan_num) = ga.channel_id.parse::<u64>() {
                let chan_id = ChannelId::new(chan_num);
                match validate_giveaway_delivery_channel(&ctx.http, chan_id, &ga.guild_id).await {
                    Ok(_) => {
                        let winners_text = format_winners(&winners);
                        let finish_announcement = format!(
                            "🏁 **KONKURS ZAKOŃCZONY!** 🏁\n\n\
                            🎁 **Nagroda:** {}\n\
                            🎉 **Zwycięzcy:** {}\n\
                            ID konkursu: `{}`\n\
                            Gratulacje!",
                            ga.prize, winners_text, ga_uuid
                        );

                        chan_id
                            .send_message(
                                &ctx.http,
                                CreateMessage::new()
                                    .content(finish_announcement)
                                    .allowed_mentions(
                                        CreateAllowedMentions::new().users(
                                            winners
                                                .iter()
                                                .filter_map(|w| w.parse::<u64>().ok())
                                                .map(UserId::new),
                                        ),
                                    ),
                            )
                            .await
                    }
                    Err(err_msg) => {
                        tracing::error!(
                            giveaway_id = %ga_uuid,
                            channel_id = %chan_id,
                            reason = %err_msg,
                            "Channel validation failed for giveaway finish announcement"
                        );
                        Err(serenity::Error::Other(err_msg))
                    }
                }
            } else {
                Err(serenity::Error::Other("Invalid channel ID on giveaway"))
            };

            if let Err(ref e) = send_res {
                tracing::warn!(giveaway_id = %ga_uuid, error = %e, "Could not post giveaway finish announcement");
            }

            let audit_payload = serde_json::json!({
                "giveaway_id": ga_uuid.to_string(),
                "winners": winners,
                "announcement_delivered": send_res.is_ok(),
            });

            let _ = state
                .storage
                .record_audit(
                    Some(&guild_id_str),
                    actor_id_str.as_deref(),
                    module_id,
                    "giveaway_end",
                    audit_payload,
                )
                .await;

            match send_res {
                Ok(_) => Ok(Some(format!(
                    "🏁 Konkurs `{ga_uuid}` został zakończony.\nWyłonieni zwycięzcy: {}",
                    format_winners(&winners)
                ))),
                Err(e) => Ok(Some(format!(
                    "🏁 Konkurs `{ga_uuid}` został zakończony (wyłonieni zwycięzcy: {}), lecz nie udało się opublikować ogłoszenia na kanale: {e}",
                    format_winners(&winners)
                ))),
            }
        }

        GiveawayOperation::Reroll { giveaway_id } => {
            let ga_uuid = Uuid::parse_str(giveaway_id)
                .map_err(|e| anyhow!("Invalid giveaway ID `{giveaway_id}`: {e}"))?;

            let ga = state
                .storage
                .get_giveaway(ga_uuid)
                .await
                .context("Failed to query giveaway")?
                .ok_or_else(|| anyhow!("Giveaway `{ga_uuid}` was not found in this guild"))?;

            if ga.guild_id != guild_id_str {
                return Err(anyhow!("Giveaway `{ga_uuid}` was not found in this guild"));
            }

            if !ga.ended {
                return Err(anyhow!(
                    "Giveaway `{ga_uuid}` is still active; end it before rerolling winners."
                ));
            }

            let winners = state
                .storage
                .reroll_giveaway(ga_uuid)
                .await
                .context("Failed to reroll giveaway")?;

            let send_res = if let Ok(chan_num) = ga.channel_id.parse::<u64>() {
                let chan_id = ChannelId::new(chan_num);
                match validate_giveaway_delivery_channel(&ctx.http, chan_id, &ga.guild_id).await {
                    Ok(_) => {
                        let winners_text = format_winners(&winners);
                        let reroll_announcement = format!(
                            "🎲 **PONOWNE LOSOWANIE (REROLL)!** 🎲\n\n\
                            🎁 **Nagroda:** {}\n\
                            🎉 **Nowi zwycięzcy:** {}\n\
                            ID konkursu: `{}`\n\
                            Gratulacje!",
                            ga.prize, winners_text, ga_uuid
                        );

                        chan_id
                            .send_message(
                                &ctx.http,
                                CreateMessage::new()
                                    .content(reroll_announcement)
                                    .allowed_mentions(
                                        CreateAllowedMentions::new().users(
                                            winners
                                                .iter()
                                                .filter_map(|w| w.parse::<u64>().ok())
                                                .map(UserId::new),
                                        ),
                                    ),
                            )
                            .await
                    }
                    Err(err_msg) => {
                        tracing::error!(
                            giveaway_id = %ga_uuid,
                            channel_id = %chan_id,
                            reason = %err_msg,
                            "Channel validation failed for giveaway reroll announcement"
                        );
                        Err(serenity::Error::Other(err_msg))
                    }
                }
            } else {
                Err(serenity::Error::Other("Invalid channel ID on giveaway"))
            };

            if let Err(ref e) = send_res {
                tracing::warn!(giveaway_id = %ga_uuid, error = %e, "Could not post giveaway reroll announcement");
            }

            let audit_payload = serde_json::json!({
                "giveaway_id": ga_uuid.to_string(),
                "new_winners": winners,
                "announcement_delivered": send_res.is_ok(),
            });

            let _ = state
                .storage
                .record_audit(
                    Some(&guild_id_str),
                    actor_id_str.as_deref(),
                    module_id,
                    "giveaway_reroll",
                    audit_payload,
                )
                .await;

            match send_res {
                Ok(_) => Ok(Some(format!(
                    "🎲 Ponowne losowanie dla konkursu `{ga_uuid}` zakończone!\nNowi zwycięzcy: {}",
                    format_winners(&winners)
                ))),
                Err(e) => Ok(Some(format!(
                    "🎲 Ponowne losowanie dla konkursu `{ga_uuid}` zakończone (nowi zwycięzcy: {}), lecz nie udało się opublikować ogłoszenia na kanale: {e}",
                    format_winners(&winners)
                ))),
            }
        }
    }
}

pub async fn validate_giveaway_delivery_channel(
    http: &serenity::http::Http,
    chan_id: ChannelId,
    expected_guild_id: &str,
) -> Result<serenity::model::channel::GuildChannel, &'static str> {
    let channel = match http.get_channel(chan_id).await {
        Ok(c) => c,
        Err(_) => return Err("channel inaccessible or not found"),
    };
    let Some(guild_chan) = channel.guild() else {
        return Err("channel is not a guild channel");
    };
    if guild_chan.guild_id.get().to_string() != expected_guild_id {
        return Err("tenant isolation violation: channel belongs to different guild");
    }
    if guild_chan.kind == ChannelType::Category {
        return Err("cannot send messages to category channels");
    }
    Ok(guild_chan)
}

pub async fn process_expired_giveaways_cycle(
    http: &serenity::http::Http,
    state: &AppState,
) -> Result<usize> {
    match state.storage.finish_expired_giveaways().await {
        Ok(finished_list) => {
            let count = finished_list.len();
            for (ga, winners) in finished_list {
                tracing::info!(giveaway_id = %ga.id, "Automated finish for expired giveaway");
                let mut delivered = false;

                if let Ok(chan_num) = ga.channel_id.parse::<u64>() {
                    let chan_id = ChannelId::new(chan_num);

                    match http.get_channel(chan_id).await {
                        Ok(channel) => {
                            if let Some(guild_chan) = channel.guild() {
                                if guild_chan.guild_id.get().to_string() != ga.guild_id {
                                    tracing::error!(
                                        giveaway_id = %ga.id,
                                        expected_guild = %ga.guild_id,
                                        actual_guild = %guild_chan.guild_id,
                                        channel_id = %chan_id,
                                        "Tenant isolation violation in giveaway worker: channel belongs to different guild, skipping message delivery"
                                    );
                                    let _ = state
                                        .storage
                                        .record_audit(
                                            Some(&ga.guild_id),
                                            None,
                                            "giveaway",
                                            "giveaway_cross_guild_blocked",
                                            serde_json::json!({
                                                "giveaway_id": ga.id.to_string(),
                                                "channel_id": ga.channel_id,
                                                "expected_guild_id": ga.guild_id,
                                                "detected_guild_id": guild_chan.guild_id.get().to_string(),
                                                "winners": winners,
                                            }),
                                        )
                                        .await;
                                    continue;
                                }

                                if guild_chan.kind == ChannelType::Category {
                                    tracing::warn!(
                                        giveaway_id = %ga.id,
                                        channel_id = %chan_id,
                                        "Giveaway channel is a category; skipping announcement delivery"
                                    );
                                    let _ = state
                                        .storage
                                        .record_audit(
                                            Some(&ga.guild_id),
                                            None,
                                            "giveaway",
                                            "giveaway_delivery_failed",
                                            serde_json::json!({
                                                "giveaway_id": ga.id.to_string(),
                                                "channel_id": ga.channel_id,
                                                "error": "channel is a category",
                                                "winners": winners,
                                            }),
                                        )
                                        .await;
                                    continue;
                                }

                                let winners_text = format_winners(&winners);
                                let announcement = format!(
                                    "🏁 **KONKURS ZAKOŃCZONY (AUTOMATYCZNIE)!** 🏁\n\n\
                                    🎁 **Nagroda:** {}\n\
                                    🎉 **Zwycięzcy:** {}\n\
                                    ID konkursu: `{}`\n\
                                    Gratulacje!",
                                    ga.prize, winners_text, ga.id
                                );
                                let send_res = chan_id
                                    .send_message(
                                        http,
                                        CreateMessage::new()
                                            .content(announcement)
                                            .allowed_mentions(
                                                CreateAllowedMentions::new().users(
                                                    winners
                                                        .iter()
                                                        .filter_map(|w| w.parse::<u64>().ok())
                                                        .map(UserId::new),
                                                ),
                                            ),
                                    )
                                    .await;

                                if let Err(ref e) = send_res {
                                    tracing::warn!(giveaway_id = %ga.id, error = %e, "Could not post automated giveaway finish announcement");
                                } else {
                                    delivered = true;
                                }
                            } else {
                                tracing::error!(
                                    giveaway_id = %ga.id,
                                    channel_id = %chan_id,
                                    "Giveaway channel is not a guild channel; skipping announcement delivery"
                                );
                                let _ = state
                                    .storage
                                    .record_audit(
                                        Some(&ga.guild_id),
                                        None,
                                        "giveaway",
                                        "giveaway_cross_guild_blocked",
                                        serde_json::json!({
                                            "giveaway_id": ga.id.to_string(),
                                            "channel_id": ga.channel_id,
                                            "expected_guild_id": ga.guild_id,
                                            "detected_type": "private_channel",
                                            "winners": winners,
                                        }),
                                    )
                                    .await;
                                continue;
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                giveaway_id = %ga.id,
                                channel_id = %chan_id,
                                ?e,
                                "Cannot fetch channel in giveaway worker; skipping announcement delivery"
                            );
                            let _ = state
                                .storage
                                .record_audit(
                                    Some(&ga.guild_id),
                                    None,
                                    "giveaway",
                                    "giveaway_delivery_failed",
                                    serde_json::json!({
                                        "giveaway_id": ga.id.to_string(),
                                        "channel_id": ga.channel_id,
                                        "error": e.to_string(),
                                        "winners": winners,
                                    }),
                                )
                                .await;
                            continue;
                        }
                    }
                }

                let _ = state
                    .storage
                    .record_audit(
                        Some(&ga.guild_id),
                        None,
                        "giveaway",
                        "giveaway_auto_end",
                        serde_json::json!({
                            "giveaway_id": ga.id.to_string(),
                            "winners": winners,
                            "delivered": delivered,
                        }),
                    )
                    .await;
            }
            Ok(count)
        }
        Err(err) => {
            tracing::error!(?err, "Error checking expired giveaways in worker");
            Err(err)
        }
    }
}

pub async fn run_giveaway_worker(http: std::sync::Arc<serenity::http::Http>, state: AppState) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
    loop {
        interval.tick().await;
        let _ = process_expired_giveaways_cycle(&http, &state).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_can_manage_giveaways() {
        assert!(can_manage_giveaways(Permissions::ADMINISTRATOR));
        assert!(can_manage_giveaways(Permissions::MANAGE_GUILD));
        assert!(can_manage_giveaways(Permissions::all()));
        assert!(!can_manage_giveaways(Permissions::MANAGE_CHANNELS));
        assert!(!can_manage_giveaways(Permissions::SEND_MESSAGES));
        assert!(!can_manage_giveaways(Permissions::empty()));
    }

    #[test]
    fn test_validate_giveaway_execution_untrusted_module() {
        let execution = GiveawayExecutionContext::new(
            Some(GuildId::new(123)),
            Some(ChannelId::new(456)),
            Some(UserId::new(789)),
            Permissions::MANAGE_GUILD,
            Permissions::all(),
            true,
        );

        let res = validate_giveaway_execution("community", &execution);
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("trusted giveaway module")
        );
    }

    #[test]
    fn test_validate_giveaway_execution_non_command_or_missing_actor() {
        // Missing actor
        let exec_no_actor = GiveawayExecutionContext::new(
            Some(GuildId::new(123)),
            Some(ChannelId::new(456)),
            None,
            Permissions::MANAGE_GUILD,
            Permissions::all(),
            true,
        );
        let res = validate_giveaway_execution("giveaway", &exec_no_actor);
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("interactive user command")
        );

        // Event / background (enforce_actor_permissions = false)
        let exec_event = GiveawayExecutionContext::new(
            Some(GuildId::new(123)),
            Some(ChannelId::new(456)),
            Some(UserId::new(789)),
            Permissions::MANAGE_GUILD,
            Permissions::all(),
            false,
        );
        let res2 = validate_giveaway_execution("giveaway", &exec_event);
        assert!(res2.is_err());
        assert!(
            res2.unwrap_err()
                .to_string()
                .contains("interactive user command")
        );
    }

    #[test]
    fn test_validate_giveaway_execution_missing_permission() {
        let exec_low_perms = GiveawayExecutionContext::new(
            Some(GuildId::new(123)),
            Some(ChannelId::new(456)),
            Some(UserId::new(789)),
            Permissions::SEND_MESSAGES,
            Permissions::all(),
            true,
        );
        let res = validate_giveaway_execution("giveaway", &exec_low_perms);
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("Manage Server or Administrator")
        );
    }

    #[test]
    fn test_validate_giveaway_execution_valid() {
        let valid_exec = GiveawayExecutionContext::new(
            Some(GuildId::new(123)),
            Some(ChannelId::new(456)),
            Some(UserId::new(789)),
            Permissions::MANAGE_GUILD,
            Permissions::all(),
            true,
        );
        let res = validate_giveaway_execution("giveaway", &valid_exec);
        assert_eq!(res.unwrap(), GuildId::new(123));
    }
}
