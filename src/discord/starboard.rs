use anyhow::{Context as _, Result, anyhow};
use serenity::all::{Channel, ChannelId, ChannelType, Context, GuildId, Permissions, UserId};

use crate::lua::StarboardOperation;
use crate::state::AppState;
use crate::storage::validate_snowflake;

pub struct StarboardExecutionContext {
    pub guild_id: Option<GuildId>,
    pub channel_id: Option<ChannelId>,
    pub actor_id: Option<UserId>,
    pub actor_permissions: Permissions,
    pub enforce_actor_permissions: bool,
}

impl StarboardExecutionContext {
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

pub async fn execute_starboard_action(
    ctx: &Context,
    state: &AppState,
    module_id: &str,
    execution: StarboardExecutionContext,
    operation: &StarboardOperation,
) -> Result<Option<String>> {
    if module_id != "starboard" {
        return Err(anyhow!(
            "Akcja starboard jest dozwolona wyłącznie dla modułu starboard"
        ));
    }

    let guild_id = execution
        .guild_id
        .ok_or_else(|| anyhow!("Polecenie wymaga serwera Discord"))?;
    let guild_id_str = guild_id.get().to_string();

    let actor_id = execution
        .actor_id
        .ok_or_else(|| anyhow!("Brak tożsamości aktora polecenia"))?;
    let actor_id_str = actor_id.get().to_string();

    let guild = guild_id
        .to_partial_guild(&ctx.http)
        .await
        .with_context(|| format!("Nie można pobrać danych serwera {guild_id}"))?;

    if execution.enforce_actor_permissions && actor_id != guild.owner_id {
        let perms = execution.actor_permissions;
        if !perms.contains(Permissions::MANAGE_GUILD) && !perms.contains(Permissions::ADMINISTRATOR)
        {
            return Err(anyhow!(
                "Wymagane uprawnienie Zarządzanie serwerem (MANAGE_GUILD)"
            ));
        }
    }

    match operation {
        StarboardOperation::Configure {
            channel_id,
            threshold,
        } => {
            validate_snowflake("channel_id", channel_id)
                .map_err(|e| anyhow!("Nieprawidłowe ID kanału: {e}"))?;

            if !(1..=25).contains(threshold) {
                return Err(anyhow!("Próg gwiazdek musi być z zakresu 1-25"));
            }

            let target_channel_id_u64: u64 = channel_id
                .parse()
                .map_err(|_| anyhow!("Nieprawidłowy format ID kanału"))?;
            let target_channel_id = ChannelId::new(target_channel_id_u64);

            let ch = target_channel_id
                .to_channel(&ctx.http)
                .await
                .with_context(|| {
                    format!("Nie można pobrać kanału {target_channel_id} z Discord API")
                })?;

            let guild_channel = match ch {
                Channel::Guild(gc) => gc,
                _ => return Err(anyhow!("Kanał docelowy musi być kanałem serwerowym")),
            };

            if guild_channel.guild_id != guild_id {
                return Err(anyhow!("Kanał docelowy należy do innego serwera Discord"));
            }

            if guild_channel.kind == ChannelType::Category {
                return Err(anyhow!("Kategoria kanałów nie może być kanałem starboard"));
            }

            if guild_channel.kind != ChannelType::Text && guild_channel.kind != ChannelType::News {
                return Err(anyhow!(
                    "Kanał docelowy starboard musi być kanałem tekstowym"
                ));
            }

            // Verify bot permissions in target channel
            let bot_id = {
                let id = ctx.cache.current_user().id;
                if id.get() == 0 {
                    ctx.http
                        .get_current_user()
                        .await
                        .map(|u| u.id)
                        .unwrap_or(id)
                } else {
                    id
                }
            };

            let bot_member = guild_id
                .member(&ctx.http, bot_id)
                .await
                .with_context(|| format!("Nie można pobrać danych bota na serwerze {guild_id}"))?;

            let bot_perms = crate::discord::scheduler::calculate_effective_channel_permissions(
                &guild,
                &bot_member.roles,
                bot_id,
                &guild_channel,
            );

            if !bot_perms.contains(Permissions::VIEW_CHANNEL) {
                return Err(anyhow!(
                    "Bot nie posiada uprawnienia VIEW_CHANNEL w kanale docelowym"
                ));
            }

            if !bot_perms.contains(Permissions::SEND_MESSAGES) {
                return Err(anyhow!(
                    "Bot nie posiada uprawnienia SEND_MESSAGES w kanale docelowym"
                ));
            }

            // Atomically patch starboard config in storage without clobbering enabled state or concurrent web edits
            state
                .storage
                .patch_starboard_config_with_audit(
                    &guild_id_str,
                    channel_id,
                    *threshold,
                    Some(&actor_id_str),
                )
                .await
                .context("Błąd zapisu konfiguracji starboard w storage")?;

            Ok(Some(format!(
                "⭐ Skonfigurowano Starboard: kanał <#{channel_id}> z progiem **{threshold}** gwiazdek."
            )))
        }
    }
}
