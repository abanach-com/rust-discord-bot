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

pub fn format_inventory(
    actor_key: &str,
    page: &crate::storage::StorePurchasesPage,
    item_names: &std::collections::HashMap<Uuid, String>,
) -> String {
    if page.purchases.is_empty() {
        return format!(
            "🎒 **Ekwipunek <@{actor_key}>:**\nTwój ekwipunek na tym serwerze jest pusty. Odwiedź `/economy shop`!"
        );
    }

    let mut lines = Vec::new();
    lines.push(format!("🎒 **Ekwipunek <@{actor_key}>:**"));
    for p in &page.purchases {
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
    if page.has_more {
        lines.push(
            "\n*(Wyświetlono 15 najnowszych zakupów. Pełną historię sprawdzisz w panelu WWW)*"
                .to_string(),
        );
    }
    lines.join("\n")
}

pub fn format_shop_output(page: &crate::storage::StoreItemsPage) -> String {
    if page.items.is_empty() {
        return "🛍️ **Sklep Serwerowy**\n\nSklep na tym serwerze jest obecnie pusty.".to_string();
    }

    let mut lines = Vec::new();
    lines.push("🛍️ **Sklep Serwerowy**\n".to_string());
    for (i, item) in page.items.iter().enumerate() {
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
    if page.has_more {
        lines.push(
            "\n*(Wyświetlono 15 najnowszych produktów. Pełny katalog sprawdzisz w panelu WWW)*"
                .to_string(),
        );
    }
    lines.push("\nAby kupić przedmiot, wpisz: `/economy buy item:<nazwa_lub_id>`".to_string());
    lines.join("\n")
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
            let page = state
                .storage
                .list_store_items_page(&guild_key, 15, None)
                .await?;
            Ok(Some(format_shop_output(&page)))
        }

        EconomyOperation::Inventory => {
            let page = state
                .storage
                .list_store_purchases_page(&guild_key, Some(&actor_key), 15, None)
                .await?;
            let item_ids: Vec<Uuid> = page.purchases.iter().map(|p| p.item_id).collect();
            let item_names = state
                .storage
                .list_store_item_names_by_ids(&guild_key, &item_ids)
                .await
                .unwrap_or_default();

            Ok(Some(format_inventory(&actor_key, &page, &item_names)))
        }

        EconomyOperation::Buy { item } => {
            let item_ref = item.trim();
            let target_item = state.storage.find_store_item(&guild_key, item_ref).await?;

            let Some(item) = target_item else {
                return Err(anyhow!(
                    "Nie znaleziono przedmiotu '{item_ref}' w sklepie tego serwera. Sprawdź `/economy shop`."
                ));
            };

            let (item, purchase, remaining_coins) = state
                .storage
                .purchase_store_item(
                    &guild_key,
                    &actor_key,
                    item.id,
                    execution.interaction_id.as_deref(),
                )
                .await?;

            let fulfillment_status = if purchase.fulfillment_status == "completed" {
                "completed".to_string()
            } else if purchase.fulfillment_status == "failed" {
                let reason = purchase
                    .failure_reason
                    .as_deref()
                    .unwrap_or("błąd doręczenia roli");
                format!("failed: {reason}")
            } else if let Some(role_id_str) = &item.role_id {
                let claimed = state.storage.claim_store_role_delivery(purchase.id).await?;

                if !claimed {
                    let fresh_purchase = state
                        .storage
                        .get_store_purchase(purchase.id)
                        .await?
                        .unwrap_or_else(|| purchase.clone());
                    if fresh_purchase.fulfillment_status == "completed" {
                        "completed".to_string()
                    } else if fresh_purchase.fulfillment_status == "failed" {
                        let reason = fresh_purchase
                            .failure_reason
                            .as_deref()
                            .unwrap_or("błąd doręczenia roli");
                        format!("failed: {reason}")
                    } else {
                        "processing".to_string()
                    }
                } else {
                    let role_id_num = role_id_str
                        .parse::<u64>()
                        .map_err(|_| anyhow!("nieprawidłowy identyfikator roli w przedmiocie"))?;
                    let role_id_typed = RoleId::new(role_id_num);

                    let role_safety = if let Ok(partial_guild) =
                        guild_id.to_partial_guild(&ctx.http).await
                    {
                        if let Some(role) = partial_guild.roles.get(&role_id_typed) {
                            if role.managed {
                                Err(
                                    "Role zarządzane przez integracje nie mogą być nadawane przez sklep",
                                )
                            } else if crate::discord::reaction_roles::is_privileged_role_permissions(
                                role.permissions,
                            ) {
                                Err(
                                    "Rola posiada uprawnienia administracyjne i nie może zostać nadana",
                                )
                            } else {
                                Ok(())
                            }
                        } else {
                            Err("Rola nie istnieje na tym serwerze")
                        }
                    } else {
                        Ok(())
                    };

                    match role_safety {
                        Err(err_msg) => {
                            if let Err(e) = state
                                .storage
                                .update_store_purchase_fulfillment(
                                    purchase.id,
                                    "failed",
                                    Some(err_msg),
                                    None,
                                )
                                .await
                            {
                                tracing::error!(
                                    purchase_id = %purchase.id,
                                    error = %e,
                                    "Failed to persist failed fulfillment status in store_purchases"
                                );
                            }
                            format!("failed: {err_msg}")
                        }
                        Ok(()) => {
                            match ensure_role_hierarchy(ctx, guild_id, None, role_id_typed).await {
                                Ok(()) => {
                                    match state
                                        .deliver_store_role(&guild_key, &actor_key, role_id_str)
                                        .await
                                    {
                                        Ok(()) => {
                                            if let Err(e) = state
                                                .storage
                                                .update_store_purchase_fulfillment(
                                                    purchase.id,
                                                    "completed",
                                                    None,
                                                    Some(Utc::now()),
                                                )
                                                .await
                                            {
                                                tracing::error!(
                                                    purchase_id = %purchase.id,
                                                    error = %e,
                                                    "Failed to persist completed fulfillment status in store_purchases"
                                                );
                                            }
                                            "completed".to_string()
                                        }
                                        Err(err) => {
                                            let err_str = err.to_string();
                                            let is_transient =
                                                crate::state::is_transient_role_delivery_error(
                                                    &err_str,
                                                );
                                            let new_status =
                                                if is_transient { "pending" } else { "failed" };
                                            if let Err(e) = state
                                                .storage
                                                .update_store_purchase_fulfillment(
                                                    purchase.id,
                                                    new_status,
                                                    Some(&err_str),
                                                    None,
                                                )
                                                .await
                                            {
                                                tracing::error!(
                                                    purchase_id = %purchase.id,
                                                    error = %e,
                                                    "Failed to persist {new_status} fulfillment status in store_purchases"
                                                );
                                            }
                                            format!("{new_status}: {err_str}")
                                        }
                                    }
                                }
                                Err(err) => {
                                    let err_str = err.to_string();
                                    if let Err(e) = state
                                        .storage
                                        .update_store_purchase_fulfillment(
                                            purchase.id,
                                            "failed",
                                            Some(&err_str),
                                            None,
                                        )
                                        .await
                                    {
                                        tracing::error!(
                                            purchase_id = %purchase.id,
                                            error = %e,
                                            "Failed to persist failed fulfillment status in store_purchases"
                                        );
                                    }
                                    format!("failed: {err_str}")
                                }
                            }
                        }
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
                } else if fulfillment_status.starts_with("pending") {
                    "\n⏳ Rola zostanie nadana wkrótce (chwilowe opóźnienie Discord API)."
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{StorePurchase, StorePurchasesPage};

    #[test]
    fn test_format_inventory_bounded_display_and_footer_issue_387() {
        let actor_key = "123456789";
        let mut item_names = std::collections::HashMap::new();
        let item_id = Uuid::new_v4();
        item_names.insert(item_id, "Diamond Pickaxe".to_string());

        // 1. Empty inventory
        let empty_page = StorePurchasesPage {
            purchases: vec![],
            next_cursor: None,
            has_more: false,
        };
        let empty_text = format_inventory(actor_key, &empty_page, &item_names);
        assert!(empty_text.contains("Twój ekwipunek na tym serwerze jest pusty"));

        // 2. 5 purchases (no has_more)
        let now = Utc::now();
        let purchases_5: Vec<StorePurchase> = (0..5)
            .map(|i| StorePurchase {
                id: Uuid::new_v4(),
                guild_id: "g1".to_string(),
                user_id: actor_key.to_string(),
                item_id,
                price_paid: 100 + i * 10,
                role_id: None,
                fulfillment_status: "completed".to_string(),
                failure_reason: None,
                fulfilled_at: Some(now),
                lease_expires_at: None,
                created_at: now,
            })
            .collect();
        let page_5 = StorePurchasesPage {
            purchases: purchases_5,
            next_cursor: None,
            has_more: false,
        };
        let text_5 = format_inventory(actor_key, &page_5, &item_names);
        assert_eq!(text_5.matches("• ✅ **Diamond Pickaxe**").count(), 5);
        assert!(!text_5.contains("Wyświetlono 15 najnowszych zakupów"));

        // 3. 15 purchases with has_more: true
        let purchases_15: Vec<StorePurchase> = (0..15)
            .map(|i| StorePurchase {
                id: Uuid::new_v4(),
                guild_id: "g1".to_string(),
                user_id: actor_key.to_string(),
                item_id,
                price_paid: 50,
                role_id: None,
                fulfillment_status: if i % 2 == 0 { "completed" } else { "pending" }.to_string(),
                failure_reason: None,
                fulfilled_at: Some(now),
                lease_expires_at: None,
                created_at: now,
            })
            .collect();
        let page_15 = StorePurchasesPage {
            purchases: purchases_15,
            next_cursor: Some("dummy_cursor".to_string()),
            has_more: true,
        };
        let text_15 = format_inventory(actor_key, &page_15, &item_names);
        assert_eq!(text_15.matches("• ").count(), 15);
        assert!(text_15.contains(
            "*(Wyświetlono 15 najnowszych zakupów. Pełną historię sprawdzisz w panelu WWW)*"
        ));
    }

    #[test]
    fn test_format_shop_output_bounded_display_issue_389() {
        use crate::storage::{StoreItem, StoreItemsPage};
        let now = chrono::Utc::now();

        // 1. Empty shop
        let empty_page = StoreItemsPage {
            items: Vec::new(),
            next_cursor: None,
            has_more: false,
        };
        let empty_text = format_shop_output(&empty_page);
        assert!(empty_text.contains("jest obecnie pusty"));

        // 2. 5 items without has_more
        let items_5: Vec<StoreItem> = (0..5)
            .map(|i| StoreItem {
                id: Uuid::new_v4(),
                guild_id: "g1".to_string(),
                name: format!("Item {i}"),
                description: format!("Description {i}"),
                price: 100 + i * 20,
                role_id: if i == 0 {
                    Some("999000000000000001".to_string())
                } else {
                    None
                },
                stock: if i == 1 { Some(3) } else { None },
                created_at: now,
            })
            .collect();
        let page_5 = StoreItemsPage {
            items: items_5,
            next_cursor: None,
            has_more: false,
        };
        let text_5 = format_shop_output(&page_5);
        assert_eq!(text_5.matches("** — **").count(), 5);
        assert!(text_5.contains("• Rola: <@&999000000000000001>"));
        assert!(text_5.contains("(pozostało: 3)"));
        assert!(!text_5.contains("Wyświetlono 15 najnowszych produktów"));

        // 3. 15 items with has_more: true
        let items_15: Vec<StoreItem> = (0..15)
            .map(|i| StoreItem {
                id: Uuid::new_v4(),
                guild_id: "g1".to_string(),
                name: format!("Item {i}"),
                description: format!("Description {i}"),
                price: 50,
                role_id: None,
                stock: None,
                created_at: now,
            })
            .collect();
        let page_15 = StoreItemsPage {
            items: items_15,
            next_cursor: Some("dummy_cursor".to_string()),
            has_more: true,
        };
        let text_15 = format_shop_output(&page_15);
        assert_eq!(text_15.matches("** — **").count(), 15);
        assert!(text_15.contains(
            "*(Wyświetlono 15 najnowszych produktów. Pełny katalog sprawdzisz w panelu WWW)*"
        ));
    }
}
