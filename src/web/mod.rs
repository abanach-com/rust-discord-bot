pub mod auth;
pub mod features;
pub mod integrations;
pub mod routes;
pub mod static_files;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::Result;
use axum::{
    Router,
    extract::{Request, State},
    middleware::{self, Next},
    response::Response,
    routing::{delete, get, patch, post, put},
};
use dashmap::DashMap;
use tokio::net::TcpListener;

use crate::AppState;

#[derive(Clone)]
pub struct WebState {
    pub app: AppState,
    pub login_rate_limiter: Arc<auth::LoginRateLimiter>,
    pub permissions_cache: Arc<DashMap<(String, String), auth::CachedGuildPermission>>,
}

pub async fn serve(app: AppState) -> Result<()> {
    let (_tx, rx) = tokio::sync::broadcast::channel(1);
    serve_with_shutdown(app, rx).await
}

pub async fn serve_with_shutdown(
    app: AppState,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) -> Result<()> {
    let bind = app.config.dashboard_bind;
    let state = WebState {
        app,
        login_rate_limiter: Arc::new(auth::LoginRateLimiter::new()),
        permissions_cache: Arc::new(DashMap::new()),
    };

    let cleanup_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            let (states, sessions, perms) = auth::cleanup_stale_records(&cleanup_state).await;
            cleanup_state.app.rate_limiter.cleanup_stale_entries();
            if states > 0 || sessions > 0 || perms > 0 {
                tracing::debug!(
                    states,
                    sessions,
                    perms,
                    "pruned stale OAuth states, expired sessions, and permissions"
                );
            }
        }
    });

    let router = build_router(state);

    let listener = TcpListener::bind(bind).await?;
    tracing::info!(%bind, "dashboard is listening");
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = shutdown_rx.recv().await;
        tracing::info!("dashboard received shutdown signal");
    })
    .await?;

    Ok(())
}

async fn track_dashboard_requests(
    State(state): State<WebState>,
    request: Request,
    next: Next,
) -> Response {
    state
        .app
        .metrics
        .dashboard_requests_total
        .fetch_add(1, Ordering::Relaxed);
    next.run(request).await
}

pub fn build_router(state: WebState) -> Router {
    Router::new()
        .route("/", get(static_files::index))
        .route("/favicon.ico", get(static_files::favicon))
        .route("/favicon.svg", get(static_files::favicon))
        .route("/assets/styles.css", get(static_files::styles))
        .route("/assets/app.js", get(static_files::javascript))
        .route("/healthz", get(routes::health_live))
        .route("/health/live", get(routes::health_live))
        .route("/readyz", get(routes::health_ready))
        .route("/health/ready", get(routes::health_ready))
        .route("/metrics", get(routes::prometheus_metrics))
        .route("/auth/discord", get(auth::login))
        .route("/auth/discord/callback", get(auth::callback))
        .route("/api/session", get(auth::current_session))
        .route("/api/logout", post(auth::logout))
        .route("/api/installer", get(routes::get_installer_manifest))
        .route("/api/modules", get(routes::list_modules))
        .route("/api/guilds/{guild_id}/modules", get(routes::guild_modules))
        .route(
            "/api/guilds/{guild_id}/modules/{module_id}",
            put(routes::update_guild_module),
        )
        .route("/api/guilds/{guild_id}/cases", get(routes::guild_cases))
        .route(
            "/api/guilds/{guild_id}/cases/{case_number}",
            get(routes::guild_case_details),
        )
        .route(
            "/api/guilds/{guild_id}/cases/{case_number}/notes",
            post(routes::add_guild_case_note),
        )
        .route(
            "/api/guilds/{guild_id}/cases/{case_number}/void",
            post(routes::void_guild_case),
        )
        .route(
            "/api/guilds/{guild_id}/cases/{case_number}",
            patch(routes::update_guild_case),
        )
        .route(
            "/api/guilds/{guild_id}/audit",
            get(routes::guild_audit_events),
        )
        .route(
            "/api/guilds/{guild_id}/events",
            get(routes::guild_events_stream),
        )
        // Phase 1 endpoints
        .route(
            "/api/guilds/{guild_id}/wizard",
            get(routes::get_setup_wizard),
        )
        .route(
            "/api/guilds/{guild_id}/diagnostics",
            get(routes::get_guild_diagnostics),
        )
        .route(
            "/api/guilds/{guild_id}/modules/{module_id}/draft",
            get(routes::get_guild_module_draft)
                .post(routes::save_guild_module_draft)
                .delete(routes::discard_guild_module_draft),
        )
        .route(
            "/api/guilds/{guild_id}/modules/{module_id}/publish",
            post(routes::publish_guild_module_draft),
        )
        .route(
            "/api/guilds/{guild_id}/privacy",
            get(routes::get_guild_privacy_summary),
        )
        .route(
            "/api/guilds/{guild_id}/privacy/export",
            post(routes::export_user_privacy_data),
        )
        .route(
            "/api/guilds/{guild_id}/privacy/delete",
            post(routes::delete_user_privacy_data),
        )
        .route(
            "/api/guilds/{guild_id}/cases/{case_number}/appeal",
            post(routes::submit_moderation_appeal),
        )
        .route(
            "/api/guilds/{guild_id}/appeals",
            get(routes::list_guild_appeals),
        )
        .route(
            "/api/guilds/{guild_id}/appeals/{appeal_id}/decide",
            post(routes::decide_moderation_appeal),
        )
        .route(
            "/api/guilds/{guild_id}/automod/simulate",
            post(routes::simulate_automod),
        )
        // Phase 2 & 3 routes
        .route(
            "/api/guilds/{guild_id}/snapshots",
            get(features::list_snapshots).post(features::create_snapshot),
        )
        .route(
            "/api/guilds/{guild_id}/snapshots/{snapshot_id}",
            get(features::get_snapshot).delete(features::delete_snapshot),
        )
        .route(
            "/api/guilds/{guild_id}/snapshots/{snapshot_id}/diff",
            post(features::diff_snapshot),
        )
        .route(
            "/api/guilds/{guild_id}/snapshots/{snapshot_id}/restore",
            post(features::restore_snapshot),
        )
        .route(
            "/api/guilds/{guild_id}/automations",
            get(features::list_automations).post(features::create_automation),
        )
        .route(
            "/api/guilds/{guild_id}/automations/{automation_id}",
            delete(features::delete_automation),
        )
        .route(
            "/api/guilds/{guild_id}/automations/{automation_id}/test",
            post(features::test_automation),
        )
        .route(
            "/api/guilds/{guild_id}/sticky/{channel_id}",
            get(features::get_sticky).delete(features::delete_sticky),
        )
        .route("/api/guilds/{guild_id}/sticky", post(features::set_sticky))
        .route(
            "/api/guilds/{guild_id}/broadcast",
            post(features::broadcast_announcement),
        )
        .route(
            "/api/guilds/{guild_id}/embed/preview",
            post(features::preview_embed),
        )
        .route("/api/guilds/{guild_id}/polls", post(features::create_poll))
        .route(
            "/api/guilds/{guild_id}/polls/{poll_id}",
            get(features::get_poll),
        )
        .route(
            "/api/guilds/{guild_id}/polls/{poll_id}/vote",
            post(features::vote_poll),
        )
        .route(
            "/api/guilds/{guild_id}/giveaways",
            post(features::create_giveaway),
        )
        .route(
            "/api/guilds/{guild_id}/giveaways/{giveaway_id}",
            get(features::get_giveaway),
        )
        .route(
            "/api/guilds/{guild_id}/giveaways/{giveaway_id}/join",
            post(features::join_giveaway),
        )
        .route(
            "/api/guilds/{guild_id}/giveaways/{giveaway_id}/reroll",
            post(features::reroll_giveaway),
        )
        .route(
            "/api/guilds/{guild_id}/starboard/track",
            post(features::track_starboard),
        )
        .route(
            "/api/guilds/{guild_id}/store/items",
            get(features::list_store_items).post(features::create_store_item),
        )
        .route(
            "/api/guilds/{guild_id}/store/purchase",
            post(features::purchase_store_item),
        )
        .route(
            "/api/guilds/{guild_id}/progression/season/reset",
            post(features::reset_progression_season),
        )
        .route(
            "/api/guilds/{guild_id}/progression/import",
            post(features::import_progression),
        )
        .route(
            "/api/guilds/{guild_id}/scheduled-events",
            get(features::list_events).post(features::create_event),
        )
        .route(
            "/api/guilds/{guild_id}/scheduled-events/{event_id}/ics",
            get(features::get_event_ics),
        )
        .route(
            "/api/guilds/{guild_id}/scheduled-events/{event_id}/roster/join",
            post(features::join_event_roster),
        )
        .route(
            "/api/guilds/{guild_id}/scheduled-events/{event_id}/roster/leave",
            post(features::leave_event_roster),
        )
        .route(
            "/api/guilds/{guild_id}/tournaments",
            post(features::create_tournament),
        )
        .route(
            "/api/guilds/{guild_id}/tournaments/{tournament_id}/advance",
            post(features::advance_tournament_match),
        )
        .route(
            "/api/guilds/{guild_id}/analytics/overview",
            get(features::analytics_overview),
        )
        .route(
            "/api/guilds/{guild_id}/analytics/retention",
            get(features::analytics_retention),
        )
        .route(
            "/api/guilds/{guild_id}/analytics/invites",
            get(features::analytics_invites).post(features::record_invite),
        )
        .route(
            "/api/guilds/{guild_id}/analytics/export.csv",
            get(features::analytics_export_csv),
        )
        .route(
            "/api/guilds/{guild_id}/music/queue",
            get(features::get_music_queue),
        )
        .route(
            "/api/guilds/{guild_id}/music/control",
            post(features::control_music),
        )
        .route(
            "/api/guilds/{guild_id}/music/radio",
            get(features::get_radio_stations),
        )
        .route(
            "/api/guilds/{guild_id}/voice/dynamic",
            post(features::register_dynamic_voice),
        )
        .route(
            "/api/guilds/{guild_id}/voice/dynamic/{channel_id}/empty",
            post(features::set_dynamic_voice_empty),
        )
        .route(
            "/api/guilds/{guild_id}/voice/dynamic/cleanup",
            get(features::cleanup_dynamic_voice),
        )
        .route(
            "/api/guilds/{guild_id}/feeds",
            get(features::list_feeds).post(features::create_feed),
        )
        .route(
            "/api/webhooks/github",
            post(features::handle_github_webhook),
        )
        .route(
            "/api/guilds/{guild_id}/games/link",
            post(features::link_game_account),
        )
        .route(
            "/api/guilds/{guild_id}/games/verify",
            post(features::verify_game_account),
        )
        .route(
            "/api/guilds/{guild_id}/games/profiles",
            get(features::list_game_profiles),
        )
        .route(
            "/api/guilds/{guild_id}/games/minecraft/relay",
            post(features::minecraft_chat_relay),
        )
        .route(
            "/api/guilds/{guild_id}/counting/{channel_id}",
            post(features::submit_counting_number),
        )
        .route(
            "/api/guilds/{guild_id}/cards",
            get(features::list_user_cards),
        )
        .route(
            "/api/guilds/{guild_id}/cards/trade",
            post(features::propose_card_trade),
        )
        .route(
            "/api/guilds/{guild_id}/cards/trade/{trade_id}/accept",
            post(features::accept_card_trade),
        )
        .route(
            "/api/guilds/{guild_id}/tuppers",
            get(features::list_tuppers).post(features::create_tupper),
        )
        .route(
            "/api/guilds/{guild_id}/faq",
            get(features::list_faq).post(features::create_faq),
        )
        .route(
            "/api/guilds/{guild_id}/faq/{faq_id}",
            delete(features::delete_faq),
        )
        .route(
            "/api/guilds/{guild_id}/translate",
            post(features::translate_text),
        )
        .route(
            "/api/guilds/{guild_id}/ai/query",
            post(features::query_ai_assistant),
        )
        // Phase 4: Integrations & Extensibility (#92-#107)
        .route(
            "/api/guilds/{guild_id}/integrations",
            get(integrations::list_guild_integrations),
        )
        .route(
            "/api/guilds/{guild_id}/integrations/{provider}/connect",
            post(integrations::initiate_provider_connect),
        )
        .route(
            "/api/guilds/{guild_id}/integrations/{provider}/callback",
            post(integrations::complete_provider_callback),
        )
        .route(
            "/api/guilds/{guild_id}/integrations/{provider}",
            delete(integrations::disconnect_provider),
        )
        .route(
            "/api/guilds/{guild_id}/integrations/deliveries",
            get(integrations::list_delivery_logs),
        )
        .route(
            "/api/integrations/webhooks/{provider}",
            post(integrations::handle_inbound_webhook),
        )
        .route(
            "/api/guilds/{guild_id}/identity/link",
            post(integrations::link_identity),
        )
        .route(
            "/api/guilds/{guild_id}/identity/links",
            get(integrations::list_identities),
        )
        .route(
            "/api/guilds/{guild_id}/identity/unlink/{provider}",
            delete(integrations::unlink_identity),
        )
        .route(
            "/api/guilds/{guild_id}/steam/lfg",
            get(integrations::list_steam_lfg).post(integrations::create_steam_lfg),
        )
        .route(
            "/api/guilds/{guild_id}/steam/lfg/{post_id}/join",
            post(integrations::join_steam_lfg),
        )
        .route(
            "/api/guilds/{guild_id}/github/thread-map",
            post(integrations::map_github_thread),
        )
        .route(
            "/api/guilds/{guild_id}/telegram/bridge",
            post(integrations::create_telegram_bridge),
        )
        .route(
            "/api/guilds/{guild_id}/telegram/bridges",
            get(integrations::list_telegram_bridges),
        )
        .route(
            "/api/guilds/{guild_id}/workflows",
            post(integrations::create_workflow),
        )
        .route(
            "/api/guilds/{guild_id}/workflows/{wf_id}/dry-run",
            post(integrations::dry_run_workflow),
        )
        .route(
            "/api/gaming-hub/status",
            get(integrations::list_game_statuses),
        )
        .route(
            "/api/gaming-hub/leagues/{version}",
            get(integrations::get_poe_leagues),
        )
        .route(
            "/api/guilds/{guild_id}/social/drafts",
            post(integrations::create_social_draft),
        )
        .route(
            "/api/guilds/{guild_id}/social/drafts/{draft_id}/approve",
            post(integrations::approve_social_draft),
        )
        .route(
            "/api/guilds/{guild_id}/ai/budget",
            get(integrations::get_ai_budget),
        )
        .route(
            "/api/guilds/{guild_id}/ai/budget/record",
            post(integrations::record_ai_usage),
        )
        .route(
            "/api/guilds/{guild_id}/ai/documents",
            post(integrations::add_ai_knowledge_doc),
        )
        .route(
            "/api/guilds/{guild_id}/ai/search",
            post(integrations::search_ai_knowledge),
        )
        .route(
            "/api/marketplace/packages",
            get(integrations::list_marketplace_packages),
        )
        .route(
            "/api/marketplace/publish",
            post(integrations::publish_marketplace_package),
        )
        .route(
            "/api/guilds/{guild_id}/marketplace/install",
            post(integrations::install_guild_plugin),
        )
        .route(
            "/api/guilds/{guild_id}/marketplace/installed",
            get(integrations::list_installed_guild_plugins),
        )
        .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            track_dashboard_requests,
        ))
        .with_state(state)
}
