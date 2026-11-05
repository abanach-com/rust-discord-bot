use std::sync::Arc;

use chrono::{Duration, Utc};
use dashmap::DashMap;
use reqwest::header::COOKIE;
use rusty_discord_bot::{
    config::Config,
    session::{DashboardGuild, DashboardUser},
    state::AppState,
    storage::AiKnowledgeDocument,
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
        music_allowed_hosts: std::collections::HashSet::new(),
        trusted_proxies: ["127.0.0.1".parse().unwrap(), "::1".parse().unwrap()]
            .into_iter()
            .collect(),
        offline_mode: false,
        discord_api_base_url: "https://discord.com/api/v10".to_string(),
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
    let state = WebState {
        app,
        login_rate_limiter: Arc::new(LoginRateLimiter::new()),
        permissions_cache: permissions_cache.clone(),
    };
    (state, permissions_cache)
}

#[tokio::test]
async fn test_phase3_games_rp_and_knowledge_suite() {
    let (state, _) = test_web_state().await;
    let guild_id = "guild_phase3";
    let user_id = "user_phase3_player1";
    let target_user_id = "user_phase3_player2";
    let session_id = "test_phase3_session_token";
    let csrf_token = "csrf_phase3_token";

    // Seed session in session store
    let user = DashboardUser {
        id: user_id.to_owned(),
        username: "Phase3Player".to_owned(),
        global_name: None,
        avatar: None,
    };
    let guilds = vec![DashboardGuild {
        id: guild_id.to_owned(),
        name: "Phase 3 Test Guild".to_owned(),
        icon: None,
        owner: true,
        permissions: "8".to_owned(),
    }];

    state
        .app
        .sessions
        .save_session(
            session_id,
            user,
            guilds,
            csrf_token.to_owned(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .expect("save session");

    // Start server
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let router = build_router(state.clone());

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    // ==========================================
    // 1. Agent 18 (#89): Game Bridges (Steam, Minecraft, PoE 1/2)
    // ==========================================
    let link_req = serde_json::json!({
        "platform": "steam",
        "platform_account_id": "76561198000000000"
    });
    let link_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/games/link"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&link_req)
        .send()
        .await
        .unwrap();
    assert_eq!(link_resp.status(), reqwest::StatusCode::OK);
    let link_body: serde_json::Value = link_resp.json().await.unwrap();
    let challenge = link_body["challenge_code"].as_str().unwrap();

    // Verify game link
    let verify_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/games/verify"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "platform": "steam",
            "code": challenge
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(verify_resp.status(), reqwest::StatusCode::OK);
    let verify_body: serde_json::Value = verify_resp.json().await.unwrap();
    assert_eq!(verify_body["verified"], true);

    // List verified accounts
    let profiles_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/games/profiles"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(profiles_resp.status(), reqwest::StatusCode::OK);

    // Minecraft chat relay with loopback suppression invariant
    let relay_loopback_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/games/minecraft/relay"
        ))
        .header(COOKIE, &session_cookie)
        .json(&serde_json::json!({
            "sender": "DiscordBot",
            "message": "Hello from discord",
            "origin_tag": "discord_relay"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(relay_loopback_resp.status(), reqwest::StatusCode::OK);
    let relay_loop_body: serde_json::Value = relay_loopback_resp.json().await.unwrap();
    assert_eq!(relay_loop_body["suppressed"], true);

    // ==========================================
    // 2. Agent 19 (#90): Counting, Cards & Tupper RP
    // ==========================================
    // Counting game
    let count1_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/counting/chan_count_1"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({"count": 1}))
        .send()
        .await
        .unwrap();
    assert_eq!(count1_resp.status(), reqwest::StatusCode::OK);
    let count1_body: serde_json::Value = count1_resp.json().await.unwrap();
    assert_eq!(count1_body["valid"], true);
    assert_eq!(count1_body["current_count"], 1);

    // Same user cannot count twice in a row
    let count2_same_user = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/counting/chan_count_1"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({"count": 2}))
        .send()
        .await
        .unwrap();
    assert_eq!(count2_same_user.status(), reqwest::StatusCode::OK);
    let count2_body: serde_json::Value = count2_same_user.json().await.unwrap();
    assert_eq!(count2_body["valid"], false);
    assert_eq!(count2_body["current_count"], 0); // resets on fail

    // Card Collections & Atomic Trades
    let card1 = state
        .app
        .storage
        .grant_user_card(guild_id, user_id, "card_dragon", "Red Dragon", "legendary")
        .await
        .unwrap();
    let card2 = state
        .app
        .storage
        .grant_user_card(
            guild_id,
            target_user_id,
            "card_knight",
            "Silver Knight",
            "rare",
        )
        .await
        .unwrap();

    let trade_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/cards/trade"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "initiator_card_id": card1.id,
            "target_user_id": target_user_id,
            "target_card_id": card2.id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(trade_resp.status(), reqwest::StatusCode::OK);
    let trade_body: serde_json::Value = trade_resp.json().await.unwrap();
    let trade_id = trade_body["id"].as_str().unwrap();

    // Accept trade using target user session
    let accept_trade_res = state
        .app
        .storage
        .accept_card_trade(
            guild_id,
            uuid::Uuid::parse_str(trade_id).unwrap(),
            target_user_id,
        )
        .await
        .unwrap();
    assert!(accept_trade_res);

    // Verify card ownership swapped
    let p1_cards = state
        .app
        .storage
        .list_user_cards(guild_id, user_id)
        .await
        .unwrap();
    assert!(p1_cards.iter().any(|c| c.card_id == "card_knight"));

    // Issue #145: Re-accepting already accepted trade fails
    let reaccept_res = state
        .app
        .storage
        .accept_card_trade(
            guild_id,
            uuid::Uuid::parse_str(trade_id).unwrap(),
            target_user_id,
        )
        .await;
    assert!(reaccept_res.is_err(), "Cannot re-accept non-pending trade");

    // Issue #145: Trading a card with itself is rejected at propose time
    let self_trade_res = state
        .app
        .storage
        .propose_card_trade(guild_id, user_id, card1.id, target_user_id, card1.id)
        .await;
    assert!(self_trade_res.is_err(), "Cannot trade a card with itself");

    // Issue #145: Card belonging to a different guild is rejected
    let other_guild_trade = state
        .app
        .storage
        .accept_card_trade(
            "different_guild_999",
            uuid::Uuid::parse_str(trade_id).unwrap(),
            target_user_id,
        )
        .await;
    assert!(
        other_guild_trade.is_err(),
        "Card trade in different guild must be rejected"
    );

    // Tupper RP profiles
    let tupper_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/tuppers"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "name": "Eldrin the Mage",
            "prefix_or_brackets": "m:"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(tupper_resp.status(), reqwest::StatusCode::OK);

    // Duplicate prefix for the same user & guild must return 409 Conflict (Issue #154)
    let dup_tupper_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/tuppers"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "name": "Eldrin Second Form",
            "prefix_or_brackets": "m:"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        dup_tupper_resp.status(),
        reqwest::StatusCode::CONFLICT,
        "Duplicate tupper prefix for the same user must return 409 Conflict"
    );
    let dup_body: serde_json::Value = dup_tupper_resp.json().await.unwrap();
    assert_eq!(
        dup_body["error"],
        "Tupper prefix already exists for this user"
    );

    // Creating profile with distinct prefix succeeds
    let distinct_tupper_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/tuppers"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "name": "Grommash the Warrior",
            "prefix_or_brackets": "w:"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(distinct_tupper_resp.status(), reqwest::StatusCode::OK);

    // List tuppers verifies exactly 2 profiles exist, and original was not mutated
    let list_tuppers_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/tuppers"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(list_tuppers_resp.status(), reqwest::StatusCode::OK);
    let tupper_list: Vec<serde_json::Value> = list_tuppers_resp.json().await.unwrap();
    assert_eq!(tupper_list.len(), 2);
    let original = tupper_list
        .iter()
        .find(|t| t["prefix_or_brackets"] == "m:")
        .unwrap();
    assert_eq!(
        original["name"], "Eldrin the Mage",
        "Original profile name must not be mutated"
    );

    // ==========================================
    // 3. Agent 20 (#91): FAQ, Translations & AI Assistant
    // ==========================================
    // Create FAQ
    let faq_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/faq"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "question": "How to join the Minecraft server?",
            "answer": "Use IP play.example.com and link your Discord account with /verify.",
            "aliases": ["mc-ip", "minecraft"],
            "category": "gaming"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(faq_resp.status(), reqwest::StatusCode::OK);

    // Search FAQ
    let search_faq_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/faq?query=minecraft"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(search_faq_resp.status(), reqwest::StatusCode::OK);
    let search_body: serde_json::Value = search_faq_resp.json().await.unwrap();
    assert_eq!(search_body.as_array().unwrap().len(), 1);

    // On-demand translation
    let trans_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/translate"))
        .header(COOKIE, &session_cookie)
        .json(&serde_json::json!({
            "text": "Dzień dobry wszystkim!",
            "target_language": "en",
            "ephemeral": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(trans_resp.status(), reqwest::StatusCode::OK);
    let trans_body: serde_json::Value = trans_resp.json().await.unwrap();
    assert_eq!(trans_body["target_language"], "en");
    assert_eq!(trans_body["ephemeral"], true);

    // Grounded AI assistant (#204):
    // 1. Without indexed sources -> answered: false, empty citations, no fake escalation
    let ai_empty_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/query"))
        .header(COOKIE, &session_cookie)
        .json(&serde_json::json!({
            "question": "Jaki jest regulamin serwera?",
            "monthly_token_budget_used": 50000
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(ai_empty_resp.status(), reqwest::StatusCode::OK);
    let ai_empty_body: serde_json::Value = ai_empty_resp.json().await.unwrap();
    assert_eq!(ai_empty_body["answered"], false);
    assert!(ai_empty_body["citations"].as_array().unwrap().is_empty());
    assert_eq!(ai_empty_body["escalated_to_staff"], false);
    assert_eq!(ai_empty_body["status"], "no_sources");

    // 2. Index an authentic knowledge document
    let test_doc = AiKnowledgeDocument {
        id: uuid::Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        channel_acl: serde_json::json!([]),
        title: "Regulamin Społeczności".to_string(),
        content: "Zasada 1: Szanuj innych użytkowników. Obowiązuje kultura osobista.".to_string(),
        source_type: "community_rules".to_string(),
        created_at: Utc::now(),
    };
    state
        .app
        .storage
        .create_ai_document(test_doc)
        .await
        .expect("create ai doc");

    // 3. Querying for regulamin now finds the authentic source
    let ai_found_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/query"))
        .header(COOKIE, &session_cookie)
        .json(&serde_json::json!({
            "question": "Jaki jest regulamin serwera?",
            "monthly_token_budget_used": 50000
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(ai_found_resp.status(), reqwest::StatusCode::OK);
    let ai_found_body: serde_json::Value = ai_found_resp.json().await.unwrap();
    assert_eq!(ai_found_body["answered"], true);
    assert_eq!(
        ai_found_body["response"],
        "Zasada 1: Szanuj innych użytkowników. Obowiązuje kultura osobista."
    );
    assert_eq!(
        ai_found_body["citations"],
        serde_json::json!(["community_rules: Regulamin Społeczności"])
    );
    assert_eq!(ai_found_body["escalated_to_staff"], false);

    // 4. Querying for an unindexed topic -> answered: false, empty citations
    let ai_unknown_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/query"))
        .header(COOKIE, &session_cookie)
        .json(&serde_json::json!({
            "question": "Jaki jest przepis na sernik?",
            "monthly_token_budget_used": 50000
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(ai_unknown_resp.status(), reqwest::StatusCode::OK);
    let ai_unknown_body: serde_json::Value = ai_unknown_resp.json().await.unwrap();
    assert_eq!(ai_unknown_body["answered"], false);
    assert!(ai_unknown_body["citations"].as_array().unwrap().is_empty());
    assert_eq!(ai_unknown_body["escalated_to_staff"], false);

    // 5. Server-side token budget check: exhausted server budget halts queries,
    //    even if client attempts to pass monthly_token_budget_used: 0
    state
        .app
        .storage
        .record_ai_usage(guild_id, 1_000_000, 10.0)
        .await
        .expect("record ai usage to exceed limit");

    let ai_budget_halted = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/query"))
        .header(COOKIE, &session_cookie)
        .json(&serde_json::json!({
            "question": "Jaki jest regulamin serwera?",
            "monthly_token_budget_used": 0 // Client attempt to spoof budget
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(ai_budget_halted.status(), reqwest::StatusCode::OK);
    let ai_budget_body: serde_json::Value = ai_budget_halted.json().await.unwrap();
    assert_eq!(ai_budget_body["answered"], false);
    assert_eq!(ai_budget_body["status"], "budget_exceeded");
    assert_eq!(ai_budget_body["escalated_to_staff"], false);
}
