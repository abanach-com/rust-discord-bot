use std::sync::Arc;

use chrono::{Duration, Utc};
use dashmap::DashMap;
use reqwest::header::COOKIE;
use rusty_discord_bot::{
    config::Config,
    discord::components::{
        ComponentError, decode_and_validate_component_id, encode_component_id,
        verify_module_manifest_checksum,
    },
    discord::music::is_blocked_ip,
    session::{DashboardGuild, DashboardUser},
    state::AppState,
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
        transcript_encryption_key: None,
        transcript_active_key_id: "v1".to_string(),
        transcript_keyring: None,
        ticket_transcripts_disabled: false,
        metrics_token: None,
        metrics_allowed_ips: std::collections::HashSet::new(),
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
async fn test_phase2_community_operations_and_media_suite() {
    let (state, _) = test_web_state().await;
    let guild_id = "100000000000000001";
    let user_id = "200000000000000001";
    let session_id = "test_phase2_session_token";
    let csrf_token = "csrf_phase2_token";

    // Seed coins for economy store testing
    let _ = state
        .app
        .storage
        .adjust_member_progress(guild_id, user_id, 500, 1000, 10)
        .await;

    // Seed session in session store
    let user = DashboardUser {
        id: user_id.to_owned(),
        username: "Phase2Admin".to_owned(),
        global_name: None,
        avatar: None,
    };
    let guilds = vec![DashboardGuild {
        id: guild_id.to_owned(),
        name: "Phase 2 Test Guild".to_owned(),
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
    // 1. Agent 07 (#78): Server Snapshots & Disaster Recovery
    // ==========================================
    let snap_req = serde_json::json!({
        "name": "Production Golden Snapshot",
        "snapshot_data": {
            "token": "DISCORD_BOT_SECRET_LEAK",
            "webhook_url": "https://discord.com/api/webhooks/leak",
            "channels": [{"name": "general", "type": 0}],
            "roles": [{"name": "Admin", "permissions": "8"}]
        }
    });

    let create_snap_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/snapshots"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&snap_req)
        .send()
        .await
        .expect("create snapshot");
    assert_eq!(create_snap_resp.status(), reqwest::StatusCode::OK);
    let snap_data: serde_json::Value = create_snap_resp.json().await.unwrap();
    let snap_id = snap_data["id"].as_str().unwrap();

    // Verify PII/token scrubbing invariant
    let data_obj = &snap_data["snapshot_json"];
    assert!(data_obj.get("token").is_none(), "token must be scrubbed");
    assert!(
        data_obj.get("webhook_url").is_none(),
        "webhook must be scrubbed"
    );

    // List snapshots
    let list_snap_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/snapshots"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(list_snap_resp.status(), reqwest::StatusCode::OK);

    // Diff snapshot
    let diff_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/snapshots/{snap_id}/diff"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(diff_resp.status(), reqwest::StatusCode::OK);
    let diff_body: serde_json::Value = diff_resp.json().await.unwrap();
    assert_eq!(diff_body["safe_to_restore"], true);

    // Restore snapshot
    let restore_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/snapshots/{snap_id}/restore"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(restore_resp.status(), reqwest::StatusCode::OK);
    let restore_body: serde_json::Value = restore_resp.json().await.unwrap();
    assert_eq!(restore_body["restored"], true);

    // ==========================================
    // 2. Agent 08 (#79): Custom Automations
    // ==========================================
    let auto_req = serde_json::json!({
        "name": "Auto-Role On Reaction",
        "trigger_type": "reaction_add",
        "conditions": {"emoji": "⭐"},
        "actions": {"add_role": "VIP"}
    });

    let create_auto_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/automations"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&auto_req)
        .send()
        .await
        .unwrap();
    assert_eq!(create_auto_resp.status(), reqwest::StatusCode::OK);
    let auto_data: serde_json::Value = create_auto_resp.json().await.unwrap();
    let auto_id = auto_data["id"].as_str().unwrap();

    // Test automation dry-run (#203)
    // 1. Non-existent automation -> 404 Not Found
    let fake_auto_id = uuid::Uuid::new_v4();
    let test_fake = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/automations/{fake_auto_id}/test"
        ))
        .header(COOKIE, &session_cookie)
        .json(&serde_json::json!({"event_payload": {"emoji": "⭐"}, "depth": 1}))
        .send()
        .await
        .unwrap();
    assert_eq!(test_fake.status(), reqwest::StatusCode::NOT_FOUND);

    // 2. Matching event payload -> planned_actions contains ["add_role"]
    let test_loop_ok = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/automations/{auto_id}/test"
        ))
        .header(COOKIE, &session_cookie)
        .json(&serde_json::json!({"event_payload": {"emoji": "⭐"}, "depth": 3}))
        .send()
        .await
        .unwrap();
    assert_eq!(test_loop_ok.status(), reqwest::StatusCode::OK);
    let loop_ok_data: serde_json::Value = test_loop_ok.json().await.unwrap();
    assert_eq!(loop_ok_data["success"], true);
    assert_eq!(loop_ok_data["dry_run"], true);
    assert_eq!(loop_ok_data["matched_conditions"], true);
    assert_eq!(
        loop_ok_data["planned_actions"],
        serde_json::json!(["add_role"])
    );

    // 3. Non-matching event payload -> planned_actions is empty
    let test_mismatch = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/automations/{auto_id}/test"
        ))
        .header(COOKIE, &session_cookie)
        .json(&serde_json::json!({"event_payload": {"emoji": "❌"}, "depth": 1}))
        .send()
        .await
        .unwrap();
    assert_eq!(test_mismatch.status(), reqwest::StatusCode::OK);
    let mismatch_data: serde_json::Value = test_mismatch.json().await.unwrap();
    assert_eq!(mismatch_data["success"], true);
    assert_eq!(mismatch_data["matched_conditions"], false);
    assert!(
        mismatch_data["planned_actions"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // 4. Depth > 5 -> loop guard
    let test_loop_exceeded = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/automations/{auto_id}/test"
        ))
        .header(COOKIE, &session_cookie)
        .json(&serde_json::json!({"event_payload": {"emoji": "⭐"}, "depth": 6}))
        .send()
        .await
        .unwrap();
    let loop_bad_data: serde_json::Value = test_loop_exceeded.json().await.unwrap();
    assert_eq!(loop_bad_data["success"], false);
    assert!(
        loop_bad_data["error"]
            .as_str()
            .unwrap()
            .contains("Cyclic trigger")
    );

    // ==========================================
    // 3. Agent 09 (#80): Component Custom ID & Signing
    // ==========================================
    let secret = "test_signing_key_secret";
    let encoded_id = encode_component_id(secret, guild_id, user_id, "verify_ticket", "123", 120);
    let decoded = decode_and_validate_component_id(secret, &encoded_id, user_id, guild_id).unwrap();
    assert_eq!(decoded.guild_id, guild_id);
    assert_eq!(decoded.action, "verify_ticket");

    // User mismatch
    assert_eq!(
        decode_and_validate_component_id(secret, &encoded_id, "stranger_user", guild_id),
        Err(ComponentError::UserMismatch)
    );

    // Manifest checksum verification
    let manifest_content = "<module><name>Test</name></module>";
    let mut hasher = sha2::Sha256::default();
    sha2::Digest::update(&mut hasher, manifest_content.as_bytes());
    let hash = hex::encode(sha2::Digest::finalize(hasher));
    assert!(verify_module_manifest_checksum(manifest_content, &hash));

    // ==========================================
    // 4. Agent 10 (#81): Sticky Messages & Broadcast
    // ==========================================
    let sticky_req = serde_json::json!({
        "channel_id": "chan_sticky_1",
        "message_id": "msg_sticky_1",
        "content": "Please read our guidelines above!",
        "cooldown_seconds": 10
    });

    let set_sticky_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/sticky"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&sticky_req)
        .send()
        .await
        .unwrap();
    assert_eq!(set_sticky_resp.status(), reqwest::StatusCode::OK);

    // Read sticky
    let get_sticky_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/sticky/chan_sticky_1"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_sticky_resp.status(), reqwest::StatusCode::OK);

    // Broadcast delivery validation (#202)
    // 1. Non-snowflake channel IDs are rejected
    let bad_broadcast_req = serde_json::json!({
        "target_channels": ["chan_1", "chan_2"],
        "title": "Server Update",
        "description": "Maintenance completed."
    });
    let bad_broadcast_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/broadcast"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&bad_broadcast_req)
        .send()
        .await
        .unwrap();
    assert_eq!(
        bad_broadcast_resp.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let bad_bcast_body: serde_json::Value = bad_broadcast_resp.json().await.unwrap();
    assert!(
        bad_bcast_body["error"]
            .as_str()
            .unwrap()
            .contains("channel_id must contain only decimal digits")
    );

    // 2. In offline/dummy mode, broadcast fails closed without claiming delivery
    let valid_broadcast_req = serde_json::json!({
        "target_channels": ["111000000000000001", "111000000000000002"],
        "title": "Server Update",
        "description": "Maintenance completed."
    });
    let broadcast_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/broadcast"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&valid_broadcast_req)
        .send()
        .await
        .unwrap();
    assert_eq!(broadcast_resp.status(), reqwest::StatusCode::BAD_REQUEST);
    let bcast_body: serde_json::Value = broadcast_resp.json().await.unwrap();
    assert!(
        bcast_body["error"]
            .as_str()
            .unwrap()
            .contains("Broadcast delivery is not supported in offline mode")
    );

    // Embed mobile preview
    let preview_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/embed/preview"))
        .json(&serde_json::json!({
            "title": "Sample",
            "description": "Sample text",
            "color": 0x5865F2
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(preview_resp.status(), reqwest::StatusCode::OK);

    // ==========================================
    // 5. Agent 11 (#82): Polls, Giveaways & Starboard
    // ==========================================
    // Create Poll
    let poll_req = serde_json::json!({
        "channel_id": "chan_general",
        "question": "Which game tonight?",
        "options": ["Valorant", "Rust", "Minecraft"],
        "is_anonymous": true
    });
    let create_poll_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/polls"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&poll_req)
        .send()
        .await
        .unwrap();
    assert_eq!(create_poll_resp.status(), reqwest::StatusCode::OK);
    let poll_body: serde_json::Value = create_poll_resp.json().await.unwrap();
    let poll_id = poll_body["id"].as_str().unwrap();

    // Issue #147: Poll duration_minutes boundary validation
    for invalid_duration in [0, -1, -500, i64::MIN, i64::MAX, 43_201] {
        let invalid_poll_resp = client
            .post(format!("{base_url}/api/guilds/{guild_id}/polls"))
            .header(COOKIE, &session_cookie)
            .header("x-csrf-token", csrf_token)
            .json(&serde_json::json!({
                "channel_id": "chan_general",
                "question": "Invalid duration poll",
                "options": ["A", "B"],
                "duration_minutes": invalid_duration
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(
            invalid_poll_resp.status(),
            reqwest::StatusCode::BAD_REQUEST,
            "duration {invalid_duration} must return 400 Bad Request"
        );
    }

    // Valid boundary durations for poll: 1 minute and 43200 minutes (30 days)
    for valid_duration in [1, 43_200] {
        let valid_poll_resp = client
            .post(format!("{base_url}/api/guilds/{guild_id}/polls"))
            .header(COOKIE, &session_cookie)
            .header("x-csrf-token", csrf_token)
            .json(&serde_json::json!({
                "channel_id": "chan_general",
                "question": "Valid duration poll",
                "options": ["A", "B"],
                "duration_minutes": valid_duration
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(valid_poll_resp.status(), reqwest::StatusCode::OK);
    }

    // Vote on option 1 ("Rust")
    let vote_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/polls/{poll_id}/vote"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({"option_idx": 1}))
        .send()
        .await
        .unwrap();
    assert_eq!(vote_resp.status(), reqwest::StatusCode::OK);
    let vote_body: serde_json::Value = vote_resp.json().await.unwrap();
    assert_eq!(vote_body["tallies_json"][1], 1);

    // Giveaway create & join
    state
        .app
        .storage
        .set_module_settings(
            guild_id,
            "giveaway",
            true,
            serde_json::json!({
                "allow_self_entry": true
            }),
        )
        .await
        .unwrap();

    let ga_req = serde_json::json!({
        "channel_id": "chan_events",
        "prize": "Discord Nitro 1 Month",
        "winners_count": 1,
        "duration_minutes": 120
    });
    let ga_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/giveaways"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&ga_req)
        .send()
        .await
        .unwrap();
    assert_eq!(ga_resp.status(), reqwest::StatusCode::OK);
    let ga_body: serde_json::Value = ga_resp.json().await.unwrap();
    let ga_id = ga_body["id"].as_str().unwrap();

    // Issue #147: Giveaway duration_minutes boundary validation
    for invalid_duration in [0, -1, -500, i64::MIN, i64::MAX, 43_201] {
        let invalid_ga_resp = client
            .post(format!("{base_url}/api/guilds/{guild_id}/giveaways"))
            .header(COOKIE, &session_cookie)
            .header("x-csrf-token", csrf_token)
            .json(&serde_json::json!({
                "channel_id": "chan_events",
                "prize": "Invalid giveaway",
                "winners_count": 1,
                "duration_minutes": invalid_duration
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(
            invalid_ga_resp.status(),
            reqwest::StatusCode::BAD_REQUEST,
            "duration {invalid_duration} must return 400 Bad Request"
        );
    }

    // Valid boundary durations for giveaway: 1 minute and 43200 minutes (30 days)
    for valid_duration in [1, 43_200] {
        let valid_ga_resp = client
            .post(format!("{base_url}/api/guilds/{guild_id}/giveaways"))
            .header(COOKIE, &session_cookie)
            .header("x-csrf-token", csrf_token)
            .json(&serde_json::json!({
                "channel_id": "chan_events",
                "prize": "Valid giveaway",
                "winners_count": 1,
                "duration_minutes": valid_duration
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(valid_ga_resp.status(), reqwest::StatusCode::OK);
    }

    // Join giveaway
    let join_ga_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{ga_id}/join"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(join_ga_resp.status(), reqwest::StatusCode::OK);

    // Reroll on active giveaway must be rejected with 409 CONFLICT (Issue #265)
    let reroll_conflict_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{ga_id}/reroll"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(reroll_conflict_resp.status(), reqwest::StatusCode::CONFLICT);

    // End giveaway explicitly
    let end_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{ga_id}/end"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(end_resp.status(), reqwest::StatusCode::OK);

    // Reroll concluded giveaway
    let reroll_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{ga_id}/reroll"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(reroll_resp.status(), reqwest::StatusCode::OK);

    // Starboard: private channel exclusion
    let star_private_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/starboard/track"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "source_channel_id": "staff_secret",
            "source_message_id": "msg_99",
            "starboard_message_id": "sb_1",
            "reaction_count": 5,
            "is_private_channel": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(star_private_resp.status(), reqwest::StatusCode::OK);
    let star_body: serde_json::Value = star_private_resp.json().await.unwrap();
    assert_eq!(star_body["posted"], false);

    // ==========================================
    // 6. Agent 12 (#83): Store, Seasons & Progress Import
    // ==========================================
    // Create store item
    let item_req = serde_json::json!({
        "name": "Custom Color Role",
        "description": "Unlock VIP pastel colors",
        "price": 250,
        "role_id": "100000000000000002",
        "stock": 5
    });

    // Verify path traversal in role_id is rejected with 400 Bad Request (#157)
    let traversal_req = serde_json::json!({
        "name": "Malicious Item",
        "description": "Exploit attempt",
        "price": 100,
        "role_id": "../../../../../guilds/333/members/444/roles/555",
        "stock": 5
    });
    let traversal_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/store/items"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&traversal_req)
        .send()
        .await
        .unwrap();
    assert_eq!(traversal_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Verify whitespace in role_id is rejected with 400 Bad Request (#165)
    let whitespace_req = serde_json::json!({
        "name": "Whitespace Item",
        "description": "Whitespace attempt",
        "price": 100,
        "role_id": " 100000000000000002 ",
        "stock": 5
    });
    let whitespace_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/store/items"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&whitespace_req)
        .send()
        .await
        .unwrap();
    assert_eq!(whitespace_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    let item_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/store/items"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&item_req)
        .send()
        .await
        .unwrap();
    assert_eq!(item_resp.status(), reqwest::StatusCode::OK);
    let item_body: serde_json::Value = item_resp.json().await.unwrap();
    let item_id = item_body["id"].as_str().unwrap();

    // Purchase store item atomically
    let purchase_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/store/purchase"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({"item_id": item_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(purchase_resp.status(), reqwest::StatusCode::OK);
    let purch_body: serde_json::Value = purchase_resp.json().await.unwrap();
    assert_eq!(purch_body["success"], true);
    assert_eq!(purch_body["remaining_coins"], 750); // 1000 - 250 = 750
    assert_eq!(purch_body["fulfillment_status"], "completed");
    assert!(purch_body["purchase_id"].is_string());

    // Verify GET /api/guilds/{guild_id}/store/purchases
    let list_purchases_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/store/purchases"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(list_purchases_resp.status(), reqwest::StatusCode::OK);
    let list_purchases_body: serde_json::Value = list_purchases_resp.json().await.unwrap();
    let purchases = list_purchases_body["purchases"].as_array().unwrap();
    assert_eq!(purchases.len(), 1);
    assert_eq!(purchases[0]["role_id"], "100000000000000002");
    assert_eq!(purchases[0]["fulfillment_status"], "completed");

    // Archive season
    let season_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/progression/season/reset"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({"season_name": "Season 1: Genesis"}))
        .send()
        .await
        .unwrap();
    assert_eq!(season_resp.status(), reqwest::StatusCode::OK);

    // Mee6 CSV/JSON import conflict preview
    let import_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/progression/import"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "format": "mee6",
            "dry_run": true,
            "rows": [
                {"id": "user_1", "xp": 1500},
                {"id": "user_2", "xp": 2400},
                {"id": "user_1", "xp": 1800} // duplicate conflict
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(import_resp.status(), reqwest::StatusCode::OK);
    let import_body: serde_json::Value = import_resp.json().await.unwrap();
    assert_eq!(import_body["conflict_count"], 1);
    assert_eq!(import_body["valid_count"], 2);
    assert_eq!(import_body["imported"], false);

    // Issue #148: dry_run=false must be rejected with 400 Bad Request until full atomic apply is implemented
    let apply_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/progression/import"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "format": "mee6",
            "dry_run": false,
            "rows": [
                {"id": "user_1", "xp": 1500}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(apply_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Issue #148: Unsupported import format must be rejected with 400 Bad Request
    let bad_format_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/progression/import"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "format": "unsupported_bot_format",
            "dry_run": true,
            "rows": [
                {"id": "user_1", "xp": 1500}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_format_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // ==========================================
    // 7. Agent 13 (#84): Events Calendar, Rosters & Tournaments
    // ==========================================
    let event_req = serde_json::json!({
        "title": "Community Raid Night",
        "description": "Defeat the ender dragon together",
        "start_time": "2026-11-15T19:00:00Z",
        "timezone": "Europe/Warsaw",
        "max_participants": 2
    });
    let create_ev_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&event_req)
        .send()
        .await
        .unwrap();
    assert_eq!(create_ev_resp.status(), reqwest::StatusCode::OK);
    let ev_body: serde_json::Value = create_ev_resp.json().await.unwrap();
    let ev_id = ev_body["id"].as_str().unwrap();

    // Export iCalendar (.ics)
    let ics_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{ev_id}/ics"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(ics_resp.status(), reqwest::StatusCode::OK);
    let ics_text = ics_resp.text().await.unwrap();
    assert!(ics_text.contains("BEGIN:VCALENDAR"));
    assert!(ics_text.contains("SUMMARY:Community Raid Night"));

    // Roster join & promotion
    let r1 = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{ev_id}/roster/join"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({"max_main": 1}))
        .send()
        .await
        .unwrap();
    assert_eq!(r1.status(), reqwest::StatusCode::OK);
    let r1_body: serde_json::Value = r1.json().await.unwrap();
    assert_eq!(r1_body["role_type"], "main");

    // Tournament creation & match advancement
    let tourney_req = serde_json::json!({
        "name": "Winter 2v2 Cup",
        "teams": ["Team Alpha", "Team Beta", "Team Gamma", "Team Delta"]
    });
    let tourney_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/tournaments"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&tourney_req)
        .send()
        .await
        .unwrap();
    assert_eq!(tourney_resp.status(), reqwest::StatusCode::OK);
    let tourney_body: serde_json::Value = tourney_resp.json().await.unwrap();
    let tourney_id = tourney_body["id"].as_str().unwrap();

    let advance_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/tournaments/{tourney_id}/advance"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({"match_idx": 0, "winner": "Team Alpha"}))
        .send()
        .await
        .unwrap();
    assert_eq!(advance_resp.status(), reqwest::StatusCode::OK);

    // ==========================================
    // 8. Agent 14 (#85): Telemetry, Retention & Invites
    // ==========================================
    let ret_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/analytics/retention"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(ret_resp.status(), reqwest::StatusCode::OK);
    let ret_body: serde_json::Value = ret_resp.json().await.unwrap();
    assert!(ret_body["cohorts"].as_array().unwrap().len() >= 2);

    let csv_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/analytics/export.csv"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(csv_resp.status(), reqwest::StatusCode::OK);
    let csv_text = csv_resp.text().await.unwrap();
    assert!(csv_text.contains("daily_active_members"));

    // Record invite
    let inv_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/analytics/invites"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "code": "discord-gg-test",
            "inviter_id": user_id,
            "campaign_tag": "twitter_promo"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(inv_resp.status(), reqwest::StatusCode::OK);

    // ==========================================
    // 9. Agent 15 (#86): Music Queue & SSRF Defense
    // ==========================================
    let queue_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/music/queue"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(queue_resp.status(), reqwest::StatusCode::OK);

    // SSRF blocked IP unit test
    assert!(is_blocked_ip("127.0.0.1".parse().unwrap()));
    assert!(is_blocked_ip("169.254.169.254".parse().unwrap()));
    assert!(is_blocked_ip("10.0.0.1".parse().unwrap()));
    assert!(!is_blocked_ip("8.8.8.8".parse().unwrap()));

    // Radio stations
    let radio_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/music/radio"))
        .send()
        .await
        .unwrap();
    assert_eq!(radio_resp.status(), reqwest::StatusCode::OK);

    // ==========================================
    // 10. Agent 16 (#87): Dynamic Voice & Cleanup
    // ==========================================
    let dyn_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/voice/dynamic"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "channel_id": "temp_voice_901",
            "owner_id": user_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(dyn_resp.status(), reqwest::StatusCode::OK);

    let empty_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/voice/dynamic/temp_voice_901/empty"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({"is_empty": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(empty_resp.status(), reqwest::StatusCode::OK);

    // ==========================================
    // 11. Agent 17 (#88): Feeds & GitHub Webhook
    // ==========================================
    let feed_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/feeds"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "channel_id": "chan_feeds",
            "feed_type": "rss",
            "url_or_target": "https://news.ycombinator.com/rss"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(feed_resp.status(), reqwest::StatusCode::OK);

    // GitHub webhook validation (Issue #235)
    // 1. Invalid signature rejection -> 401 Unauthorized
    let gh_bad_resp = client
        .post(format!("{base_url}/api/webhooks/github"))
        .header("X-Hub-Signature-256", "sha256=abcdef1234567890")
        .header("X-GitHub-Event", "push")
        .header("X-GitHub-Delivery", "delivery-p2-bad")
        .body(r#"{"repository":{"full_name":"abanach-com/rust-discord-bot"}}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(gh_bad_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 2. Valid HMAC signature acceptance -> 200 OK
    let gh_body = r#"{"repository":{"full_name":"abanach-com/rust-discord-bot"}}"#;
    let hmac_bytes = rusty_discord_bot::integrations::webhook::compute_hmac_sha256(
        "test-github-secret".as_bytes(),
        gh_body.as_bytes(),
    );
    let sig_hex = format!("sha256={}", hex::encode(hmac_bytes));

    let gh_resp = client
        .post(format!("{base_url}/api/webhooks/github"))
        .header("X-Hub-Signature-256", &sig_hex)
        .header("X-GitHub-Event", "push")
        .header("X-GitHub-Delivery", "delivery-p2-good")
        .body(gh_body)
        .send()
        .await
        .unwrap();
    assert_eq!(gh_resp.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn test_guild_store_member_access_and_purchase_isolation() {
    let (state, _) = test_web_state().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let router = build_router(state.clone());

    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve test web");
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let guild_id = "777000000000000010";

    // 1. Setup Admin session (can_manage = true)
    let admin_session_id = "sess_admin_store";
    let admin_csrf = "csrf_admin_store";
    state
        .app
        .sessions
        .save_session(
            admin_session_id,
            DashboardUser {
                id: "100000000000000010".to_owned(),
                username: "admin_user".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Store Test Guild".to_owned(),
                icon: None,
                owner: true,
                permissions: "8".to_owned(),
            }],
            admin_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    // 2. Admin creates a store item
    let admin_cookie = format!("rusty_discord_bot_session={admin_session_id}");
    let create_item_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/store/items"))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "name": "Badge Role",
            "description": "Store item for members",
            "price": 150,
            "role_id": "999000000000000010",
            "stock": 5
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_item_resp.status(), reqwest::StatusCode::OK);
    let item_json: serde_json::Value = create_item_resp.json().await.unwrap();
    let item_id = item_json["id"].as_str().unwrap();

    // 3. Setup Regular Member session (permissions = 0, owner = false)
    let member_user_id = "200000000000000010";
    let member_session_id = "sess_member_store";
    let member_csrf = "csrf_member_store";
    state
        .app
        .sessions
        .save_session(
            member_session_id,
            DashboardUser {
                id: member_user_id.to_owned(),
                username: "regular_member".to_owned(),
                global_name: Some("Regular".to_owned()),
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Store Test Guild".to_owned(),
                icon: None,
                owner: false,
                permissions: "0".to_owned(), // No manage guild or admin
            }],
            member_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    // Give member 500 coins in storage
    state
        .app
        .storage
        .adjust_member_progress(guild_id, member_user_id, 0, 500, 0)
        .await
        .unwrap();

    let member_cookie = format!("rusty_discord_bot_session={member_session_id}");

    // 4. Setup Non-Member session (not in guild_id at all)
    let non_member_session_id = "sess_non_member";
    let non_member_csrf = "csrf_non_member";
    state
        .app
        .sessions
        .save_session(
            non_member_session_id,
            DashboardUser {
                id: "300000000000000010".to_owned(),
                username: "stranger".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: "888000000000000099".to_owned(), // Different guild
                name: "Other Guild".to_owned(),
                icon: None,
                owner: false,
                permissions: "0".to_owned(),
            }],
            non_member_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();
    let non_member_cookie = format!("rusty_discord_bot_session={non_member_session_id}");

    // Test A: Unauthenticated requests -> 401
    let unauth_list = client
        .get(format!("{base_url}/api/guilds/{guild_id}/store/items"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth_list.status(), reqwest::StatusCode::UNAUTHORIZED);

    let unauth_buy = client
        .post(format!("{base_url}/api/guilds/{guild_id}/store/purchase"))
        .json(&serde_json::json!({"item_id": item_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth_buy.status(), reqwest::StatusCode::UNAUTHORIZED);

    // Test B: Non-member requests -> 403 Forbidden
    let non_member_list = client
        .get(format!("{base_url}/api/guilds/{guild_id}/store/items"))
        .header(COOKIE, &non_member_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(non_member_list.status(), reqwest::StatusCode::FORBIDDEN);

    let non_member_buy = client
        .post(format!("{base_url}/api/guilds/{guild_id}/store/purchase"))
        .header(COOKIE, &non_member_cookie)
        .header("x-csrf-token", non_member_csrf)
        .json(&serde_json::json!({"item_id": item_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(non_member_buy.status(), reqwest::StatusCode::FORBIDDEN);

    // Test C: Regular Member CAN view store items (#163)
    let member_list = client
        .get(format!("{base_url}/api/guilds/{guild_id}/store/items"))
        .header(COOKIE, &member_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(member_list.status(), reqwest::StatusCode::OK);
    let items_array: Vec<serde_json::Value> = member_list.json().await.unwrap();
    assert_eq!(items_array.len(), 1);
    assert_eq!(items_array[0]["id"], item_id);
    assert_eq!(items_array[0]["price"], 150);

    // Test D: Regular Member CANNOT create store items -> 403 Forbidden
    let member_create = client
        .post(format!("{base_url}/api/guilds/{guild_id}/store/items"))
        .header(COOKIE, &member_cookie)
        .header("x-csrf-token", member_csrf)
        .json(&serde_json::json!({
            "name": "Member Hack",
            "description": "Should fail",
            "price": 10,
            "stock": 1
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(member_create.status(), reqwest::StatusCode::FORBIDDEN);

    // Test E: Regular Member CAN purchase item (#163)
    let member_buy = client
        .post(format!("{base_url}/api/guilds/{guild_id}/store/purchase"))
        .header(COOKIE, &member_cookie)
        .header("x-csrf-token", member_csrf)
        .json(&serde_json::json!({"item_id": item_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(member_buy.status(), reqwest::StatusCode::OK);
    let buy_json: serde_json::Value = member_buy.json().await.unwrap();
    assert_eq!(buy_json["success"], true);
    assert_eq!(buy_json["remaining_coins"], 350); // 500 - 150 = 350

    // Test F: Regular Member CAN view their own purchases
    let member_purchases = client
        .get(format!("{base_url}/api/guilds/{guild_id}/store/purchases"))
        .header(COOKIE, &member_cookie)
        .header("x-csrf-token", member_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(member_purchases.status(), reqwest::StatusCode::OK);
    let member_purch_json: serde_json::Value = member_purchases.json().await.unwrap();
    let purch_list = member_purch_json["purchases"].as_array().unwrap();
    assert_eq!(purch_list.len(), 1);
    assert_eq!(purch_list[0]["role_id"], "999000000000000010");

    // Test G: Steam LFG Self-Service Member Access (#206)
    // 1. Regular member CAN list steam lfg without MANAGE_GUILD
    let member_lfg_list = client
        .get(format!("{base_url}/api/guilds/{guild_id}/steam/lfg"))
        .header(COOKIE, &member_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(member_lfg_list.status(), reqwest::StatusCode::OK);
    let lfg_posts: Vec<serde_json::Value> = member_lfg_list.json().await.unwrap();
    assert!(lfg_posts.is_empty());

    // 2. Non-member CANNOT list steam lfg -> 403 Forbidden
    let non_member_lfg_list = client
        .get(format!("{base_url}/api/guilds/{guild_id}/steam/lfg"))
        .header(COOKIE, &non_member_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(non_member_lfg_list.status(), reqwest::StatusCode::FORBIDDEN);

    // 3. Regular member CAN create steam lfg post with CSRF without MANAGE_GUILD
    let create_lfg_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/steam/lfg"))
        .header(COOKIE, &member_cookie)
        .header("x-csrf-token", member_csrf)
        .json(&serde_json::json!({
            "game_id": "730",
            "game_name": "Counter-Strike 2",
            "description": "Premier 5-stack grind",
            "max_players": 5,
            "ttl_minutes": 60
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_lfg_resp.status(), reqwest::StatusCode::OK);
    let lfg_post_data: serde_json::Value = create_lfg_resp.json().await.unwrap();
    let lfg_post_id = lfg_post_data["id"].as_str().unwrap();
    assert_eq!(lfg_post_data["author_id"], member_user_id);
    assert_eq!(lfg_post_data["max_players"], 5);

    // 4. Non-member CANNOT create steam lfg post -> 403 Forbidden
    let non_member_lfg_create = client
        .post(format!("{base_url}/api/guilds/{guild_id}/steam/lfg"))
        .header(COOKIE, &non_member_cookie)
        .header("x-csrf-token", non_member_csrf)
        .json(&serde_json::json!({
            "game_id": "730",
            "game_name": "Counter-Strike 2",
            "description": "Hacked",
            "max_players": 5
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        non_member_lfg_create.status(),
        reqwest::StatusCode::FORBIDDEN
    );

    // 5. Admin (also a member) CAN join steam lfg with CSRF
    let admin_join_lfg = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/steam/lfg/{lfg_post_id}/join"
        ))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", admin_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(admin_join_lfg.status(), reqwest::StatusCode::OK);
    let joined_lfg_data: serde_json::Value = admin_join_lfg.json().await.unwrap();
    let players = joined_lfg_data["current_players"].as_array().unwrap();
    assert_eq!(players.len(), 2);

    // 6. Non-member CANNOT join steam lfg -> 403 Forbidden
    let non_member_join_lfg = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/steam/lfg/{lfg_post_id}/join"
        ))
        .header(COOKIE, &non_member_cookie)
        .header("x-csrf-token", non_member_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(non_member_join_lfg.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_offline_mode_false_live_membership_verification() {
    let mock_member_exists = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mock_exists_clone = mock_member_exists.clone();

    // 1. Spin up mock Discord HTTP API server
    let discord_mock_app = axum::Router::new().route(
        "/guilds/{guild_id}/members/{user_id}",
        axum::routing::get(move || {
            let exists = mock_exists_clone.clone();
            async move {
                if exists.load(std::sync::atomic::Ordering::SeqCst) {
                    (
                        axum::http::StatusCode::OK,
                        axum::Json(serde_json::json!({
                            "user": {"id": "12345"},
                            "roles": []
                        })),
                    )
                } else {
                    (
                        axum::http::StatusCode::NOT_FOUND,
                        axum::Json(serde_json::json!({
                            "code": 10007,
                            "message": "Unknown Member"
                        })),
                    )
                }
            }
        }),
    );
    let discord_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind discord mock");
    let discord_addr = discord_listener.local_addr().expect("discord mock addr");
    tokio::spawn(async move {
        axum::serve(discord_listener, discord_mock_app)
            .await
            .unwrap();
    });

    // 2. Configure app state with real bot token format, offline_mode = false, pointing to mock Discord server
    let mut config = test_config();
    config.discord_token = "live_bot_token_production_secret".to_string();
    config.offline_mode = false;
    config.discord_api_base_url = format!("http://{discord_addr}");

    let app = AppState::bootstrap(config).await.expect("bootstrap");
    let permissions_cache = std::sync::Arc::new(dashmap::DashMap::new());
    let web_state = WebState {
        app,
        login_rate_limiter: std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    };

    let web_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind web listener");
    let web_addr = web_listener.local_addr().expect("web addr");
    let router = build_router(web_state.clone());
    tokio::spawn(async move {
        axum::serve(
            web_listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{web_addr}");
    let guild_id = "888000000000000001";
    let user_id = "888000000000000002";
    let session_id = "sess_live_verify_test";
    let csrf_token = "csrf_live_verify_test";

    // Create item in store
    let item = web_state
        .app
        .storage
        .create_store_item(
            guild_id,
            "Test Live Item",
            "desc",
            100,
            Some("999000000000000001"),
            Some(5),
        )
        .await
        .unwrap();
    web_state
        .app
        .storage
        .adjust_member_progress(guild_id, user_id, 0, 500, 0)
        .await
        .unwrap();

    // Session has guild snapshot from login time, but permissions = 0
    web_state
        .app
        .sessions
        .save_session(
            session_id,
            DashboardUser {
                id: user_id.to_string(),
                username: "former_member".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Guild 888".to_string(),
                icon: None,
                owner: false,
                permissions: "0".to_string(),
            }],
            csrf_token.to_string(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    // Case 1: Member was removed from guild (Discord API returns 404)
    // -> ensure_guild_member_access FAILS CLOSED with 403 Forbidden!
    mock_member_exists.store(false, std::sync::atomic::Ordering::SeqCst);

    let get_items_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/store/items"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_items_resp.status(), reqwest::StatusCode::FORBIDDEN);

    let buy_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/store/purchase"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({"item_id": item.id}))
        .send()
        .await
        .unwrap();
    assert_eq!(buy_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Coins and stock unchanged
    let progress = web_state
        .app
        .storage
        .get_member_progress(guild_id, user_id)
        .await
        .unwrap();
    assert_eq!(progress.coins, 500);

    // Case 2: Member IS in guild (Discord API returns 200)
    // -> access granted!
    mock_member_exists.store(true, std::sync::atomic::Ordering::SeqCst);

    let get_items_ok = client
        .get(format!("{base_url}/api/guilds/{guild_id}/store/items"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_items_ok.status(), reqwest::StatusCode::OK);

    let buy_ok = client
        .post(format!("{base_url}/api/guilds/{guild_id}/store/purchase"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({"item_id": item.id}))
        .send()
        .await
        .unwrap();
    assert_eq!(buy_ok.status(), reqwest::StatusCode::OK);
}
