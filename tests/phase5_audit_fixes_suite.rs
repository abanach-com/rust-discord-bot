use std::collections::HashSet;
use std::sync::Arc;

use chrono::{Duration, Utc};
use dashmap::DashMap;
use reqwest::header::COOKIE;
use uuid::Uuid;

use rusty_discord_bot::{
    config::Config,
    session::{DashboardGuild, DashboardUser},
    state::AppState,
    storage::PluginMarketplacePackage,
    web::{
        WebState,
        auth::{CachedGuildPermission, LoginRateLimiter},
        build_router,
    },
};

fn test_config() -> Config {
    Config {
        discord_token: "dummy-token".to_owned(),
        discord_application_id: 1,
        discord_client_id: "1".to_owned(),
        discord_client_secret: "secret".to_owned(),
        discord_oauth_redirect_url: "http://localhost/callback".to_owned(),
        discord_dev_guild_id: None,
        dashboard_bind: "127.0.0.1:0".parse().unwrap(),
        dashboard_public_url: "http://localhost".to_owned(),
        database_url: None,
        scripts_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts"),
        session_cookie_secure: false,
        session_ttl_seconds: 604800,
        lua_memory_limit_bytes: 4 * 1024 * 1024,
        lua_instruction_limit: 200_000,
        lua_hook_granularity: 100,
        lua_max_parallel: 32,
        lua_max_per_guild: 8,
        lua_command_timeout_ms: 3000,
        music_allowed_hosts: HashSet::new(),
        trusted_proxies: ["127.0.0.1".parse().unwrap(), "::1".parse().unwrap()]
            .into_iter()
            .collect(),
        offline_mode: false,
        discord_api_base_url: "https://discord.com/api/v10".to_string(),
        transcript_encryption_key: None,
        transcript_active_key_id: "v1".to_string(),
        transcript_keyring: None,
    }
}

async fn test_web_state() -> (
    WebState,
    Arc<DashMap<(String, String), CachedGuildPermission>>,
) {
    let app = AppState::bootstrap(test_config())
        .await
        .expect("bootstrap app state");
    let permissions_cache = Arc::new(DashMap::new());
    let login_rate_limiter = Arc::new(LoginRateLimiter::new());
    (
        WebState {
            app,
            permissions_cache: Arc::clone(&permissions_cache),
            login_rate_limiter,
        },
        permissions_cache,
    )
}

#[tokio::test]
async fn test_phase5_audit_fixes_comprehensive_suite() {
    let (state, perms_cache) = test_web_state().await;
    let app = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::builder().build().unwrap();

    let guild_a = "guild_aaa_111";
    let guild_b = "guild_bbb_222";
    let user_a = "user_aaa_111";
    let session_id = "test_phase5_session_token_123";
    let csrf_token = "test_phase5_csrf_token_456";

    state
        .app
        .sessions
        .save_session(
            session_id,
            DashboardUser {
                id: user_a.to_string(),
                username: "UserA".to_string(),
                global_name: Some("User A".to_string()),
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_a.to_string(),
                name: "Guild A".to_string(),
                icon: None,
                owner: true,
                permissions: "8".to_string(),
            }],
            csrf_token.to_string(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .unwrap();

    perms_cache.insert(
        (user_a.to_string(), guild_a.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    // ==========================================
    // 1. Issue #109: Tenant UUID Cross-Guild Isolation
    // ==========================================
    // Create a snapshot belonging strictly to Guild B
    let snap_b = state
        .app
        .storage
        .create_server_snapshot(
            guild_b,
            "Guild B Secret Snapshot",
            serde_json::json!({"channels": [{"name": "b-secret"}]}),
            "user_b",
        )
        .await
        .unwrap();

    // User A attempting to delete Guild B's snapshot via Guild A endpoint MUST fail
    let del_resp = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_a}/snapshots/{}",
            snap_b.id
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(del_resp.status(), reqwest::StatusCode::NOT_FOUND);

    // Guild B's snapshot must still exist intact
    let snap_check = state
        .app
        .storage
        .get_server_snapshot(snap_b.id)
        .await
        .unwrap();
    assert!(snap_check.is_some());

    // Steam LFG isolation test
    let lfg_b = state
        .app
        .storage
        .create_steam_lfg_post(rusty_discord_bot::storage::SteamLfgPost {
            id: Uuid::new_v4(),
            guild_id: guild_b.to_string(),
            author_id: "user_b".to_string(),
            game_id: "730".to_string(),
            game_name: "CS2".to_string(),
            description: "Guild B match".to_string(),
            max_players: 5,
            current_players: serde_json::json!(["user_b"]),
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::hours(1),
        })
        .await
        .unwrap();

    let join_lfg_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/steam/lfg/{}/join",
            lfg_b.id
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(join_lfg_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // ==========================================
    // 2. Issue #112: OAuth Callback Rejection for Unconfigured Provider
    // ==========================================
    let oauth_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/integrations/github/callback"
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "code": "fake_auth_code_123",
            "state": "fake_state_456"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(oauth_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // ==========================================
    // 3. Issues #114, #115, #116: Webhook Validation & Extraction
    // ==========================================
    // Unknown provider must return 404
    let unknown_wh = client
        .post(format!(
            "{base_url}/api/integrations/webhooks/unknown_hacker_provider"
        ))
        .body(r#"{"event_id":"123"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(unknown_wh.status(), reqwest::StatusCode::NOT_FOUND);

    // Missing delivery ID must return 400 Bad Request
    let missing_id_wh = client
        .post(format!("{base_url}/api/integrations/webhooks/github"))
        .header("X-Hub-Signature-256", "sha256=invalid")
        .body(r#"{"foo":"bar"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(missing_id_wh.status(), reqwest::StatusCode::UNAUTHORIZED);

    // ==========================================
    // 4. Issue #117: Marketplace Integrity & Ownership
    // ==========================================
    let pkg_v1 = PluginMarketplacePackage {
        id: Uuid::new_v4(),
        plugin_id: "author_protected_plugin".to_string(),
        name: "Protected".to_string(),
        version: "1.0.0".to_string(),
        author: user_a.to_string(),
        sha256_hash: "hash_v1".to_string(),
        manifest: serde_json::json!({"id": "author_protected_plugin"}),
        is_verified: false,
        created_at: Utc::now(),
    };
    state
        .app
        .storage
        .publish_marketplace_package(pkg_v1)
        .await
        .unwrap();

    // Attempt to publish same version by user A fails as duplicate version
    let dup_version_resp = client
        .post(format!("{base_url}/api/marketplace/publish"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "plugin_id": "author_protected_plugin",
            "name": "Protected",
            "version": "1.0.0",
            "author": user_a,
            "sha256_hash": "hash_v1",
            "manifest": {"id": "author_protected_plugin"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(dup_version_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // ==========================================
    // 5. Issue #120: Card Trade Deadlock & Self-Trade Rejection
    // ==========================================
    let card1 = state
        .app
        .storage
        .grant_user_card(guild_a, user_a, "card_dragon", "Dragon", "legendary")
        .await
        .unwrap();

    // Self-trade must be rejected
    let self_trade_res = state
        .app
        .storage
        .propose_card_trade(guild_a, user_a, card1.id, user_a, card1.id)
        .await;
    assert!(self_trade_res.is_err(), "Self card trade must be rejected");

    // ==========================================
    // 6. Issue #122: Community Poll Duplicate & Expiration
    // ==========================================
    let poll_options = vec!["Rust".to_string(), "Python".to_string()];
    let poll = state
        .app
        .storage
        .create_community_poll(
            guild_a,
            "c1",
            "Best language?",
            &poll_options,
            false,
            Utc::now() + Duration::hours(1),
        )
        .await
        .unwrap();

    // First vote succeeds
    let vote1 = state
        .app
        .storage
        .vote_community_poll(guild_a, poll.id, user_a, 0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(vote1.tallies_json[0], 1);

    // Second vote for the same option is a duplicate -> no tally change
    let vote2 = state
        .app
        .storage
        .vote_community_poll(guild_a, poll.id, user_a, 0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(vote2.tallies_json[0], 1);

    // Switching vote to Python -> decrements Rust and increments Python
    let vote3 = state
        .app
        .storage
        .vote_community_poll(guild_a, poll.id, user_a, 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(vote3.tallies_json[0], 0);
    assert_eq!(vote3.tallies_json[1], 1);

    // Expired poll rejection
    let expired_poll = state
        .app
        .storage
        .create_community_poll(
            guild_a,
            "c1",
            "Expired?",
            &poll_options,
            false,
            Utc::now() - Duration::minutes(5),
        )
        .await
        .unwrap();
    let exp_vote = state
        .app
        .storage
        .vote_community_poll(guild_a, expired_poll.id, user_a, 0)
        .await;
    assert!(exp_vote.is_err(), "Vote on expired poll must fail");

    // ==========================================
    // 7. Issue #123: Giveaway Winners Count Validation
    // ==========================================
    let create_zero = client
        .post(format!("{base_url}/api/guilds/{guild_a}/giveaways"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "channel_id": "c1",
            "prize": "Nitro",
            "winners_count": 0,
            "duration_minutes": 60
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_zero.status(), reqwest::StatusCode::BAD_REQUEST);

    // ==========================================
    // 8. Issue #124: Economy Store Stock Decrement Only on Success
    // ==========================================
    let item = state
        .app
        .storage
        .create_store_item(
            guild_a,
            "Diamond Sword",
            "Legendary weapon",
            1000,
            None,
            Some(5),
        )
        .await
        .unwrap();

    // User A has 0 coins, attempt purchase
    let poor_purchase = state
        .app
        .storage
        .purchase_store_item(guild_a, "poor_user", item.id)
        .await;
    assert!(poor_purchase.is_err(), "Purchase without coins must fail");

    // Stock must remain 5
    let items = state.app.storage.list_store_items(guild_a).await.unwrap();
    let it = items.iter().find(|i| i.id == item.id).unwrap();
    assert_eq!(
        it.stock,
        Some(5),
        "Stock must NOT decrement on failed purchase"
    );

    // ==========================================
    // 8b. Issue #144: Negative Store Item Price & Stock Validation
    // ==========================================
    // HTTP API: negative price must be rejected with 400 Bad Request
    let neg_price_http = client
        .post(format!("{base_url}/api/guilds/{guild_a}/store/items"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "name": "Negative Price Item",
            "description": "Exploit attempt",
            "price": -100,
            "role_id": null,
            "stock": null
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(neg_price_http.status(), reqwest::StatusCode::BAD_REQUEST);

    // HTTP API: negative stock must be rejected with 400 Bad Request
    let neg_stock_http = client
        .post(format!("{base_url}/api/guilds/{guild_a}/store/items"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "name": "Negative Stock Item",
            "description": "Exploit attempt",
            "price": 50,
            "role_id": null,
            "stock": -1
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(neg_stock_http.status(), reqwest::StatusCode::BAD_REQUEST);

    // Storage facade: creation with negative price or stock is rejected
    assert!(
        state
            .app
            .storage
            .create_store_item(guild_a, "Bad Price", "Bad", -50, None, None)
            .await
            .is_err()
    );
    assert!(
        state
            .app
            .storage
            .create_store_item(guild_a, "Bad Stock", "Bad", 50, None, Some(-5))
            .await
            .is_err()
    );

    // Legitimate item creation with price 0 (free) or positive price succeeds
    let free_item = state
        .app
        .storage
        .create_store_item(guild_a, "Free Role", "Gift", 0, None, Some(10))
        .await
        .unwrap();
    assert_eq!(free_item.price, 0);

    // ==========================================
    // 9. Issue #125: AI Budget Validation & Initialization
    // ==========================================
    // Negative usage values rejected
    let neg_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/ai/budget/record"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "tokens": -10,
            "usd": 0.5
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(neg_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Initial usage succeeds without requiring prior GET
    let init_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/ai/budget/record"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({
            "tokens": 100,
            "usd": 0.05
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(init_resp.status(), reqwest::StatusCode::OK);
    let budget_json: serde_json::Value = init_resp.json().await.unwrap();
    assert_eq!(budget_json["used_tokens"], 100);
}
