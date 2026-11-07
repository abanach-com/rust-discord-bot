use anyhow::{Context as _, Result, anyhow, ensure};
use chrono::{Duration, Utc};
use serenity::{
    all::{ChannelId, GuildId, Permissions, UserId},
    builder::{CreateAllowedMentions, CreateMessage},
    client::Context,
};
use uuid::Uuid;

use super::executor::{ActionOrigin, ChannelPurpose, resolve_and_validate_channel};
use crate::{AppState, lua::GiveawayOperation};

#[derive(Clone, Copy)]
pub(super) struct GiveawayExecutionContext {
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

pub(super) async fn execute(
    ctx: &Context,
    state: &AppState,
    module_id: &str,
    execution: GiveawayExecutionContext,
    operation: &GiveawayOperation,
) -> Result<Option<String>> {
    let guild_id = execution
        .guild_id
        .ok_or_else(|| anyhow!("Giveaway operations operate exclusively within a server"))?;

    if execution.enforce_actor_permissions && !can_manage_giveaways(execution.actor_permissions) {
        return Err(anyhow!(
            "Giveaway operations require Manage Server or Administrator permissions."
        ));
    }

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
                channel_id.as_deref(),
                ChannelPurpose::SendMessage,
            )
            .await?;

            let expires_at = Utc::now() + Duration::minutes(*duration_minutes);
            let target_channel_str = target_channel.get().to_string();

            let giveaway = state
                .storage
                .create_giveaway(
                    &guild_id_str,
                    &target_channel_str,
                    prize,
                    *winners,
                    expires_at,
                )
                .await
                .context("Failed to create giveaway in database")?;

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
                ID losowania: `{}`",
                giveaway.id
            );

            let send_res = target_channel
                .send_message(
                    &ctx.http,
                    CreateMessage::new()
                        .content(announcement)
                        .allowed_mentions(CreateAllowedMentions::new()),
                )
                .await;

            if let Err(e) = send_res {
                tracing::warn!(error = %e, "Could not post giveaway announcement message");
            }

            let audit_payload = serde_json::json!({
                "giveaway_id": giveaway.id.to_string(),
                "prize": prize,
                "winners_count": winners,
                "duration_minutes": duration_minutes,
                "channel_id": target_channel_str,
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
                "🎉 Konkurs na **{prize}** został pomyślnie utworzony!\nID: `{}`\nKanał: <#{target_channel_str}>",
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

            if let Ok(chan_num) = ga.channel_id.parse::<u64>() {
                let chan_id = ChannelId::new(chan_num);
                let winners_text = format_winners(&winners);
                let finish_announcement = format!(
                    "🏁 **KONKURS ZAKOŃCZONY!** 🏁\n\n\
                    🎁 **Nagroda:** {}\n\
                    🎉 **Zwycięzcy:** {}\n\
                    ID konkursu: `{}`\n\
                    Gratulacje!",
                    ga.prize, winners_text, ga_uuid
                );

                let _ = chan_id
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
                    .await;
            }

            let audit_payload = serde_json::json!({
                "giveaway_id": ga_uuid.to_string(),
                "winners": winners,
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

            Ok(Some(format!(
                "🏁 Konkurs `{ga_uuid}` został zakończony.\nWyłonieni zwycięzcy: {}",
                format_winners(&winners)
            )))
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

            if let Ok(chan_num) = ga.channel_id.parse::<u64>() {
                let chan_id = ChannelId::new(chan_num);
                let winners_text = format_winners(&winners);
                let reroll_announcement = format!(
                    "🎲 **PONOWNE LOSOWANIE (REROLL)!** 🎲\n\n\
                    🎁 **Nagroda:** {}\n\
                    🎉 **Nowi zwycięzcy:** {}\n\
                    ID konkursu: `{}`\n\
                    Gratulacje!",
                    ga.prize, winners_text, ga_uuid
                );

                let _ = chan_id
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
                    .await;
            }

            let audit_payload = serde_json::json!({
                "giveaway_id": ga_uuid.to_string(),
                "new_winners": winners,
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

            Ok(Some(format!(
                "🎲 Ponowne losowanie dla konkursu `{ga_uuid}` zakończone!\nNowi zwycięzcy: {}",
                format_winners(&winners)
            )))
        }
    }
}
