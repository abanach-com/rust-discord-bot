use anyhow::{Result, anyhow};
use chrono::Utc;
use serenity::{
    all::{ChannelId, GuildId, Permissions, RoleId, UserId},
    client::Context,
};
use uuid::Uuid;

use crate::{
    AppState,
    discord::executor::ensure_role_hierarchy,
    lua::EconomyOperation,
    storage::{CrimeClaimOutcome, WorkClaimOutcome},
};

#[derive(Clone, Debug)]
pub struct EconomyExecutionContext {
    pub guild_id: Option<GuildId>,
    pub channel_id: Option<ChannelId>,
    pub actor_id: Option<UserId>,
    pub actor_permissions: Permissions,
    pub enforce_actor_permissions: bool,
    pub interaction_id: Option<String>,
}

#[derive(Clone, Debug)]
struct GuildEconomyConfig {
    min_work_payout: i64,
    max_work_payout: i64,
    crime_success_rate: f64,
    crime_payout_multiplier: f64,
    work_cooldown_seconds: u64,
    crime_cooldown_seconds: u64,
}

impl Default for GuildEconomyConfig {
    fn default() -> Self {
        Self {
            min_work_payout: 50,
            max_work_payout: 250,
            crime_success_rate: 0.6,
            crime_payout_multiplier: 2.5,
            work_cooldown_seconds: 60,
            crime_cooldown_seconds: 120,
        }
    }
}

async fn get_economy_config(state: &AppState, guild_id: &str) -> GuildEconomyConfig {
    let mut config = GuildEconomyConfig::default();
    if let Ok(Some(settings)) = state.storage.get_module_settings(guild_id, "economy").await
        && let Some(cfg) = settings.config.as_object()
    {
        if let Some(min_p) = cfg.get("min_work_payout").and_then(|v| v.as_i64()) {
            config.min_work_payout = min_p.max(1);
        }
        if let Some(max_p) = cfg.get("max_work_payout").and_then(|v| v.as_i64()) {
            config.max_work_payout = max_p.max(config.min_work_payout);
        }
        if let Some(rate) = cfg.get("crime_success_rate").and_then(|v| v.as_f64()) {
            config.crime_success_rate = rate.clamp(0.0, 1.0);
        }
        if let Some(mult) = cfg.get("crime_payout_multiplier").and_then(|v| v.as_f64()) {
            config.crime_payout_multiplier = mult.max(1.0);
        }
        if let Some(cd) = cfg.get("work_cooldown_seconds").and_then(|v| v.as_u64()) {
            config.work_cooldown_seconds = cd;
        }
        if let Some(cd) = cfg.get("crime_cooldown_seconds").and_then(|v| v.as_u64()) {
            config.crime_cooldown_seconds = cd;
        }
    }
    config
}

pub async fn execute(
    ctx: &Context,
    state: &AppState,
    module_id: &str,
    execution: EconomyExecutionContext,
    operation: &EconomyOperation,
) -> Result<Option<String>> {
    if module_id != "economy" {
        return Err(anyhow!(
            "operacje ekonomii są zarezerwowane dla zaufanego modułu economy"
        ));
    }

    let guild_id = execution
        .guild_id
        .ok_or_else(|| anyhow!("ta operacja wymaga serwera"))?;
    let actor_id = execution
        .actor_id
        .ok_or_else(|| anyhow!("ta operacja wymaga interaktywnego użytkownika"))?;
    let guild_key = guild_id.get().to_string();
    let actor_key = actor_id.get().to_string();

    let is_enabled = match state
        .storage
        .get_module_settings(&guild_key, "economy")
        .await
    {
        Ok(Some(settings)) => settings.enabled,
        Ok(None) => {
            state
                .scripts
                .is_module_enabled_in_installer("economy")
                .await
        }
        Err(err) => {
            tracing::error!(?err, "failed to check economy module settings");
            false
        }
    };

    if !is_enabled {
        return Err(anyhow!("moduł economy jest wyłączony na tym serwerze"));
    }

    match operation {
        EconomyOperation::Shop => {
            let items = state.storage.list_store_items(&guild_key).await?;
            if items.is_empty() {
                return Ok(Some(
                    "🛍️ **Sklep Serwerowy**\n\nSklep na tym serwerze jest obecnie pusty."
                        .to_string(),
                ));
            }

            let mut lines = Vec::new();
            lines.push("🛍️ **Sklep Serwerowy**\n".to_string());
            for (i, item) in items.iter().enumerate() {
                let stock_str = match item.stock {
                    Some(s) => format!(" (pozostało: {s})"),
                    None => String::new(),
                };
                let role_str = match &item.role_id {
                    Some(r) => format!(" • Rola: <@&{r}>"),
                    None => String::new(),
                };
                lines.push(format!(
                    "{}. **{}** — **{}** monet{}{}\n   _{}_",
                    i + 1,
                    item.name,
                    item.price,
                    stock_str,
                    role_str,
                    item.description
                ));
            }
            lines.push(
                "\nAby kupić przedmiot, wpisz: `/economy buy item:<nazwa_lub_id>`".to_string(),
            );
            Ok(Some(lines.join("\n")))
        }

        EconomyOperation::Inventory => {
            let purchases = state
                .storage
                .list_store_purchases(&guild_key, Some(&actor_key))
                .await?;
            if purchases.is_empty() {
                return Ok(Some(format!(
                    "🎒 **Ekwipunek <@{actor_key}>:**\nTwój ekwipunek na tym serwerze jest pusty. Odwiedź `/economy shop`!"
                )));
            }

            let items = state
                .storage
                .list_store_items(&guild_key)
                .await
                .unwrap_or_default();
            let item_names: std::collections::HashMap<Uuid, String> =
                items.into_iter().map(|it| (it.id, it.name)).collect();

            let mut lines = Vec::new();
            lines.push(format!("🎒 **Ekwipunek <@{actor_key}>:**"));
            for p in &purchases {
                let name = item_names
                    .get(&p.item_id)
                    .cloned()
                    .unwrap_or_else(|| p.item_id.to_string());
                let status_icon = match p.fulfillment_status.as_str() {
                    "completed" => "✅",
                    "pending" => "⏳",
                    "failed" => "❌",
                    _ => "ℹ️",
                };
                lines.push(format!(
                    "• {status_icon} **{name}** — Status: `{}` (Zapłacono: {} monet)",
                    p.fulfillment_status, p.price_paid
                ));
            }
            Ok(Some(lines.join("\n")))
        }

        EconomyOperation::Buy { item } => {
            let all_items = state.storage.list_store_items(&guild_key).await?;
            let item_ref = item.trim();
            let target_item = if let Ok(uuid) = Uuid::parse_str(item_ref) {
                all_items.into_iter().find(|i| i.id == uuid)
            } else {
                all_items.into_iter().find(|i| {
                    i.name.eq_ignore_ascii_case(item_ref)
                        || i.name.to_lowercase().replace([' ', '-'], "_")
                            == item_ref.to_lowercase().replace([' ', '-'], "_")
                        || i.id.to_string() == item_ref
                })
            };

            let Some(item) = target_item else {
                return Err(anyhow!(
                    "Nie znaleziono przedmiotu '{item_ref}' w sklepie tego serwera. Sprawdź `/economy shop`."
                ));
            };

            let (item, purchase, remaining_coins) = state
                .storage
                .purchase_store_item(&guild_key, &actor_key, item.id)
                .await?;

            let fulfillment_status = if let Some(role_id_str) = &item.role_id {
                let role_id_num = role_id_str
                    .parse::<u64>()
                    .map_err(|_| anyhow!("nieprawidłowy identyfikator roli w przedmiocie"))?;
                match ensure_role_hierarchy(ctx, guild_id, None, RoleId::new(role_id_num)).await {
                    Ok(()) => {
                        match state
                            .deliver_store_role(&guild_key, &actor_key, role_id_str)
                            .await
                        {
                            Ok(()) => {
                                let _ = state
                                    .storage
                                    .update_store_purchase_fulfillment(
                                        purchase.id,
                                        "completed",
                                        None,
                                        Some(Utc::now()),
                                    )
                                    .await;
                                "completed".to_string()
                            }
                            Err(err) => {
                                let err_str = err.to_string();
                                let _ = state
                                    .storage
                                    .update_store_purchase_fulfillment(
                                        purchase.id,
                                        "failed",
                                        Some(&err_str),
                                        None,
                                    )
                                    .await;
                                format!("failed: {err_str}")
                            }
                        }
                    }
                    Err(err) => {
                        let err_str = err.to_string();
                        let _ = state
                            .storage
                            .update_store_purchase_fulfillment(
                                purchase.id,
                                "failed",
                                Some(&err_str),
                                None,
                            )
                            .await;
                        format!("failed: {err_str}")
                    }
                }
            } else {
                "completed".to_string()
            };

            let _ = state
                .storage
                .record_audit(
                    Some(&guild_key),
                    Some(&actor_key),
                    module_id,
                    "economy_buy",
                    serde_json::json!({
                        "item_id": item.id,
                        "item_name": item.name,
                        "price_paid": item.price,
                        "remaining_coins": remaining_coins,
                        "fulfillment_status": fulfillment_status,
                    }),
                )
                .await;

            let _ = state.guild_events.send(crate::state::GuildRealtimeEvent {
                guild_id: guild_key.clone(),
                event_type: "audit_entry".to_string(),
                payload: serde_json::json!({
                    "module_id": module_id,
                    "event": "economy_buy",
                }),
            });

            let fulfillment_info = if item.role_id.is_some() {
                if fulfillment_status == "completed" {
                    "\n🎭 Rola została pomyślnie nadana!"
                } else {
                    "\n⚠️ Nie udało się nadać roli (sprawdź uprawnienia bota i hierarchię ról)."
                }
            } else {
                ""
            };

            Ok(Some(format!(
                "🛒 Pomyślnie zakupiono **{}** za **{}** monet! Twój stan konta: **{}** monet.{}",
                item.name, item.price, remaining_coins, fulfillment_info
            )))
        }

        EconomyOperation::Work { roll } => {
            let config = get_economy_config(state, &guild_key).await;
            let outcome = state
                .storage
                .claim_work_reward(
                    &guild_key,
                    &actor_key,
                    execution.interaction_id.as_deref(),
                    config.min_work_payout,
                    config.max_work_payout,
                    config.work_cooldown_seconds,
                    *roll,
                )
                .await?;

            match outcome {
                WorkClaimOutcome::Claimed {
                    profile,
                    reward,
                    job,
                    next_claim_at,
                } => {
                    let _ = state
                        .storage
                        .record_audit(
                            Some(&guild_key),
                            Some(&actor_key),
                            module_id,
                            "economy_work",
                            serde_json::json!({
                                "job": job,
                                "reward": reward,
                                "balance": profile.coins,
                            }),
                        )
                        .await;

                    let _ = state.guild_events.send(crate::state::GuildRealtimeEvent {
                        guild_id: guild_key.clone(),
                        event_type: "audit_entry".to_string(),
                        payload: serde_json::json!({
                            "module_id": module_id,
                            "event": "economy_work",
                        }),
                    });

                    Ok(Some(format!(
                        "💼 Pracowałeś jako **{job}** i zarobiłeś **{reward}** monet! 🪙\nTwój stan konta: **{}** monet.\nKolejna zmiana: <t:{}:R>.",
                        profile.coins,
                        next_claim_at.timestamp()
                    )))
                }
                WorkClaimOutcome::Cooldown {
                    profile,
                    next_claim_at,
                } => Ok(Some(format!(
                    "⏳ Musisz odpocząć przed kolejną zmianą! Następna praca dostępna <t:{}:R>.\nTwój aktualny stan konta: **{}** monet.",
                    next_claim_at.timestamp(),
                    profile.coins
                ))),
                WorkClaimOutcome::AlreadyProcessed {
                    profile,
                    reward,
                    job,
                } => Ok(Some(format!(
                    "💼 To żądanie pracy zostało już zrealizowane: pracowałeś jako **{job}** i otrzymałeś **{reward}** monet.\nStan konta: **{}** monet.",
                    profile.coins
                ))),
            }
        }

        EconomyOperation::Crime { roll } => {
            let config = get_economy_config(state, &guild_key).await;
            let outcome = state
                .storage
                .claim_crime_reward(
                    &guild_key,
                    &actor_key,
                    execution.interaction_id.as_deref(),
                    config.min_work_payout,
                    config.max_work_payout,
                    config.crime_success_rate,
                    config.crime_payout_multiplier,
                    config.crime_cooldown_seconds,
                    *roll,
                )
                .await?;

            match outcome {
                CrimeClaimOutcome::Success {
                    profile,
                    reward,
                    next_claim_at,
                } => {
                    let _ = state
                        .storage
                        .record_audit(
                            Some(&guild_key),
                            Some(&actor_key),
                            module_id,
                            "economy_crime",
                            serde_json::json!({
                                "outcome": "success",
                                "reward": reward,
                                "balance": profile.coins,
                            }),
                        )
                        .await;

                    let _ = state.guild_events.send(crate::state::GuildRealtimeEvent {
                        guild_id: guild_key.clone(),
                        event_type: "audit_entry".to_string(),
                        payload: serde_json::json!({
                            "module_id": module_id,
                            "event": "economy_crime",
                        }),
                    });

                    Ok(Some(format!(
                        "🕵️ Udany skok na cyber-skarbiec! Zdobyłeś **{reward}** monet! 💰\nTwój stan konta: **{}** monet.\nKolejna akcja: <t:{}:R>.",
                        profile.coins,
                        next_claim_at.timestamp()
                    )))
                }
                CrimeClaimOutcome::Failure {
                    profile,
                    penalty: _,
                    next_claim_at,
                } => {
                    let _ = state
                        .storage
                        .record_audit(
                            Some(&guild_key),
                            Some(&actor_key),
                            module_id,
                            "economy_crime",
                            serde_json::json!({
                                "outcome": "failure",
                                "penalty": 0,
                                "balance": profile.coins,
                            }),
                        )
                        .await;

                    let _ = state.guild_events.send(crate::state::GuildRealtimeEvent {
                        guild_id: guild_key.clone(),
                        event_type: "audit_entry".to_string(),
                        payload: serde_json::json!({
                            "module_id": module_id,
                            "event": "economy_crime",
                        }),
                    });

                    Ok(Some(format!(
                        "🚔 Zostałeś przyłapany przez ochronę! Skok się nie powiódł i nic nie zyskałeś.\nTwój stan konta: **{}** monet.\nKolejna próba: <t:{}:R>.",
                        profile.coins,
                        next_claim_at.timestamp()
                    )))
                }
                CrimeClaimOutcome::Cooldown {
                    profile,
                    next_claim_at,
                } => Ok(Some(format!(
                    "⏳ Po ostatniej akcji policja wciąż cię obserwuje! Następna próba możliwa <t:{}:R>.\nStan konta: **{}** monet.",
                    next_claim_at.timestamp(),
                    profile.coins
                ))),
                CrimeClaimOutcome::AlreadyProcessed {
                    profile,
                    success,
                    amount,
                } => {
                    let res_str = if success { "udany" } else { "nieudany" };
                    Ok(Some(format!(
                        "🕵️ To żądanie zostało już zrealizowane. Wynik: **{res_str}** (kwota: **{amount}** monet).\nStan konta: **{}** monet.",
                        profile.coins
                    )))
                }
            }
        }
    }
}
