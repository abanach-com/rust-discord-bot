use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use axum::{
    Router,
    extract::{State, WebSocketUpgrade},
    http::{Method, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{any, get},
};

use rusty_discord_bot::{
    AppState,
    config::Config,
    discord::{
        Handler,
        executor::{
            ActionOrigin, ChannelPurpose, ensure_member_hierarchy, execute_event_actions,
            resolve_and_validate_channel,
        },
    },
    lua::LuaAction,
    storage::UpdateModuleSettingsParams,
};

use serde_json::{Value, json};

use serenity::{
    all::{ChannelId, GatewayIntents, GuildId, Message, MessageUpdateEvent, Ready, UserId},
    client::{Context, EventHandler},
    http::HttpBuilder,
};

use tokio::sync::{Mutex, oneshot};

#[derive(Clone, Default)]

struct MockServerState {
    port: u16,

    modifying_requests_count: Arc<AtomicUsize>,

    recorded_calls: Arc<Mutex<Vec<(Method, String)>>>,
}

async fn handle_ws(ws: WebSocketUpgrade) -> Response {
    println!("MOCK WS UPGRADED!");

    ws.on_upgrade(|mut socket| async move {
        println!("MOCK WS SOCKET CONNECTED!");

        // Send OP 10 Hello

        let hello = json!({

            "op": 10,

            "d": {

                "heartbeat_interval": 45000

            }

        });

        if socket
            .send(axum::extract::ws::Message::Text(hello.to_string().into()))
            .await
            .is_err()
        {
            return;
        }

        // Wait for OP 2 Identify

        while let Some(res) = socket.recv().await {
            println!("MOCK WS RECV: {:?}", res);

            if let Ok(msg) = res {
                let text = match msg {
                    axum::extract::ws::Message::Text(t) => t.to_string(),

                    axum::extract::ws::Message::Binary(b) => {
                        String::from_utf8_lossy(&b).to_string()
                    }

                    _ => continue,
                };

                if let Ok(v) = serde_json::from_str::<Value>(&text)
                    && v.get("op").and_then(|op| op.as_i64()) == Some(2)
                {
                    println!("MOCK WS GOT IDENTIFY: sending READY");

                    // Send OP 0 Ready

                    let ready = json!({

                        "op": 0,

                        "s": 1,

                        "t": "READY",

                        "d": {

                            "v": 10,

                            "user": {

                                "id": "999999999999999999",

                                "username": "MockBot",

                                "discriminator": "0000",

                                "avatar": null,

                                "bot": true

                            },

                            "guilds": [],

                            "session_id": "test_session_id",

                            "resume_gateway_url": "ws://127.0.0.1/ws",

                            "application": {

                                "id": "999999999999999999",

                                "flags": 0

                            }

                        }

                    });

                    let _ = socket
                        .send(axum::extract::ws::Message::Text(ready.to_string().into()))
                        .await;

                    break;
                }
            }
        }

        // Keep socket alive while test runs

        while let Some(Ok(_)) = socket.recv().await {}
    })
}

async fn mock_discord_handler(
    State(mock): State<MockServerState>,

    method: Method,

    uri: axum::http::Uri,

    bytes: axum::body::Bytes,
) -> Response {
    let path = uri.path().to_string();

    let body_str = String::from_utf8_lossy(&bytes).to_string();

    println!("MOCK HTTP RECV: {} {} -> {}", method, path, body_str);

    if matches!(
        method,
        Method::POST | Method::DELETE | Method::PATCH | Method::PUT
    ) {
        mock.modifying_requests_count.fetch_add(1, Ordering::SeqCst);
    }

    mock.recorded_calls
        .lock()
        .await
        .push((method.clone(), format!("{}|{}", path, body_str)));

    // Gateway URL

    if path == "/api/v10/gateway" || path == "/api/v10/gateway/bot" {
        return Json(json!({

            "url": format!("ws://127.0.0.1:{}/gateway/ws", mock.port),

            "shards": 1,

            "session_start_limit": {

                "total": 1000,

                "remaining": 999,

                "reset_after": 14400000,

                "max_concurrency": 1

            }

        }))
        .into_response();
    }

    // Bot user

    if path == "/api/v10/users/@me" {
        return Json(json!({

            "id": "999999999999999999",

            "username": "MockBot",

            "discriminator": "0000",

            "avatar": null,

            "bot": true

        }))
        .into_response();
    }

    // Channel queries: GET /api/v10/channels/{channel_id}

    if method == Method::GET && path.starts_with("/api/v10/channels/") {
        let channel_id = path.trim_start_matches("/api/v10/channels/");

        return match channel_id {
            "101" => Json(json!({

                "id": "101",

                "guild_id": "100",

                "type": 0, // GuildText

                "name": "general",

                "position": 0

            }))
            .into_response(),

            "102" => Json(json!({

                "id": "102",

                "guild_id": "100",

                "type": 0, // GuildText

                "name": "read-only-channel",

                "position": 1,

                "permission_overwrites": [

                    {

                        "id": "111111111111111111",

                        "type": 1,

                        "allow": "0",

                        "deny": "2048"

                    }

                ]

            }))
            .into_response(),

            "201" => Json(json!({

                "id": "201",

                "guild_id": "200", // Foreign guild

                "type": 0,

                "name": "other-guild-text",

                "position": 0

            }))
            .into_response(),

            "301" => Json(json!({

                "id": "301",

                "type": 1, // DM

                "recipients": [{

                    "id": "555555555555555555",

                    "username": "testuser"

                }]

            }))
            .into_response(),

            "401" => Json(json!({

                "id": "401",

                "guild_id": "100",

                "type": 4, // Category

                "name": "MY CATEGORY",

                "position": 0

            }))
            .into_response(),

            _ => (
                StatusCode::NOT_FOUND,
                Json(json!({"message": "Unknown Channel", "code": 10003})),
            )
                .into_response(),
        };
    }

    // Send Message: POST /api/v10/channels/{channel_id}/messages

    if method == Method::POST && path.contains("/messages") {
        return (
            StatusCode::OK,
            Json(json!({

                "id": "900000000000000001",

                "channel_id": "101",

                "author": {

                    "id": "999999999999999999",

                    "username": "MockBot",

                    "avatar": null,

                    "bot": true

                },

                "content": "message",

                "timestamp": "2026-10-31T07:00:00Z",

                "tts": false,

                "mention_everyone": false,

                "mentions": [],

                "mention_roles": [],

                "attachments": [],

                "embeds": [],

                "pinned": false,

                "type": 0

            })),
        )
            .into_response();
    }

    // Delete Message: DELETE /api/v10/channels/{channel_id}/messages/{message_id}

    if method == Method::DELETE && path.contains("/messages/") {
        if path.contains("denied") || path.contains("fail") {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"message": "Missing Permissions to delete message", "code": 50013})),
            )
                .into_response();
        }

        return StatusCode::NO_CONTENT.into_response();
    }

    // Guild query: GET /api/v10/guilds/{guild_id}

    if method == Method::GET && (path == "/api/v10/guilds/100" || path == "/api/v10/guilds/100/") {
        return Json(json!({

            "id": "100",

            "name": "Test Guild",

            "owner_id": "111111111111111111", // Server Owner

            "verification_level": 0,

            "default_message_notifications": 0,

            "explicit_content_filter": 0,

            "afk_timeout": 300,

            "mfa_level": 0,

            "system_channel_flags": 0,

            "nsfw_level": 0,

            "premium_tier": 0,

            "preferred_locale": "en-US",

            "emojis": [],

            "stickers": [],

            "features": [],

            "premium_progress_bar_enabled": false,

            "roles": [

                {

                    "id": "1001",

                    "name": "AdminRole",

                    "position": 50,

                    "permissions": "8", // Administrator

                    "color": 0,

                    "colors": {

                        "primary_color": 0

                    },

                    "hoist": false,

                    "managed": false,

                    "mentionable": false,

                    "flags": 0

                },

                {

                    "id": "1002",

                    "name": "MemberRole",

                    "position": 10,

                    "permissions": "0",

                    "color": 0,

                    "colors": {

                        "primary_color": 0

                    },

                    "hoist": false,

                    "managed": false,

                    "mentionable": false,

                    "flags": 0

                }

            ]

        }))
        .into_response();
    }

    // Guild Member queries: GET /api/v10/guilds/100/members/{user_id}

    if method == Method::GET && path.starts_with("/api/v10/guilds/100/members/") {
        let user_id = path.trim_start_matches("/api/v10/guilds/100/members/");

        return match user_id {

            // Owner

            "111111111111111111" => Json(json!({

                "user": { "id": "111111111111111111", "username": "Owner", "avatar": null, "bot": false },

                "roles": ["1001"],

                "joined_at": "2026-01-01T00:00:00Z",

                "deaf": false,

                "mute": false,

                "flags": 0,

                "pending": false

            })).into_response(),

            // Bot

            "999999999999999999" => Json(json!({

                "user": { "id": "999999999999999999", "username": "MockBot", "avatar": null, "bot": true },

                "roles": ["1001"], // Admin role (rank 50)

                "joined_at": "2026-01-01T00:00:00Z",

                "deaf": false,

                "mute": false,

                "flags": 0,

                "pending": false

            })).into_response(),

            // Regular raid target member

            "222222222222222222" => Json(json!({

                "user": { "id": "222222222222222222", "username": "RaidSpammer", "avatar": null, "bot": false },

                "roles": ["1002"], // Member role (rank 10)

                "joined_at": "2026-10-31T06:59:00Z",

                "deaf": false,

                "mute": false,

                "flags": 0,

                "pending": false

            })).into_response(),

            _ => (StatusCode::NOT_FOUND, Json(json!({"message": "Unknown Member", "code": 10007}))).into_response(),

        };
    }

    // Kick: DELETE /api/v10/guilds/100/members/{user_id}

    if method == Method::DELETE && path.starts_with("/api/v10/guilds/100/members/") {
        return StatusCode::NO_CONTENT.into_response();
    }

    // Timeout: PATCH /api/v10/guilds/100/members/{user_id}

    if method == Method::PATCH && path.starts_with("/api/v10/guilds/100/members/") {
        return Json(json!({

            "user": { "id": "222222222222222222", "username": "RaidSpammer" },

            "roles": ["1002"],

            "joined_at": "2026-10-31T06:59:00Z",

            "communication_disabled_until": "2026-10-31T08:00:00Z"

        }))
        .into_response();
    }

    // Default fallback

    (StatusCode::OK, Json(json!({"status": "ok"}))).into_response()
}

struct TestReadyHandler {
    tx: Arc<Mutex<Option<oneshot::Sender<Context>>>>,
}

#[serenity::async_trait]

impl EventHandler for TestReadyHandler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        println!(
            "Mock Discord Gateway READY received for bot: {}",
            ready.user.name
        );

        if let Some(tx) = self.tx.lock().await.take() {
            let _ = tx.send(ctx);
        }
    }
}

async fn start_mock_discord_environment()
-> (Context, MockServerState, Arc<serenity::all::ShardManager>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");

    let port = listener.local_addr().unwrap().port();

    let mock_state = MockServerState {
        port,

        ..Default::default()
    };

    let app = Router::new()
        .route("/gateway/ws", get(handle_ws))
        .fallback(any(mock_discord_handler))
        .with_state(mock_state.clone());

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let (ready_tx, ready_rx) = oneshot::channel();

    let handler = TestReadyHandler {
        tx: Arc::new(Mutex::new(Some(ready_tx))),
    };

    let http = HttpBuilder::new("mock-token")
        .proxy(format!("http://127.0.0.1:{port}"))
        .ratelimiter_disabled(true)
        .build();

    let mut client = serenity::client::ClientBuilder::new_with_http(http, GatewayIntents::all())
        .event_handler(handler)
        .await
        .expect("build serenity client");

    let shard_manager = client.shard_manager.clone();

    tokio::spawn(async move {
        let _ = client.start().await;
    });

    let ctx = tokio::time::timeout(std::time::Duration::from_secs(5), ready_rx)
        .await
        .expect("timed out waiting for gateway ready")
        .expect("channel rx");

    (ctx, mock_state, shard_manager)
}

async fn create_test_state() -> AppState {
    let bind: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();

    let config = Config {
        discord_token: "dummy_token".to_string(),

        discord_application_id: 123456789,

        discord_client_id: "123456789".to_string(),

        discord_client_secret: "secret".to_string(),

        discord_oauth_redirect_url: "http://127.0.0.1:8080/auth/discord/callback".to_string(),

        discord_dev_guild_id: None,

        dashboard_bind: bind,

        dashboard_public_url: format!("http://{}", bind),

        database_url: None,

        scripts_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts"),

        session_cookie_secure: false,

        session_ttl_seconds: 86400,

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
    };

    AppState::bootstrap(config)
        .await
        .expect("bootstrap should succeed")
}

#[tokio::test]

async fn test_channel_resolution_server_isolation_and_error_handling() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;

    let origin_guild_id = GuildId::new(100);

    let origin = ActionOrigin {
        guild_id: Some(origin_guild_id),

        channel_id: Some(ChannelId::new(101)),

        actor_id: Some(UserId::new(111111111111111111)),

        actor_permissions: serenity::all::Permissions::all(),

        actor_roles: Vec::new(),

        app_permissions: serenity::all::Permissions::all(),

        enforce_actor_permissions: true,
    };

    // 1. Correct channel on same guild -> Ok

    let resolved =
        resolve_and_validate_channel(&ctx, &origin, Some("101"), ChannelPurpose::SendMessage).await;

    assert!(
        resolved.is_ok(),
        "channel 101 on guild 100 should resolve successfully"
    );

    assert_eq!(resolved.unwrap().get(), 101);

    let baseline_modifications = mock.modifying_requests_count.load(Ordering::SeqCst);

    // 2. Foreign server channel -> Rejected BEFORE sending modifying requests

    let foreign =
        resolve_and_validate_channel(&ctx, &origin, Some("201"), ChannelPurpose::SendMessage).await;

    assert!(foreign.is_err(), "foreign channel 201 should be rejected");

    let foreign_err = foreign.unwrap_err().to_string();

    assert!(foreign_err.contains("different server") || foreign_err.contains("obcego serwera"));

    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_modifications,
        "Zero modifying requests should be made for foreign channel"
    );

    // 3. DM channel in guild context -> Rejected

    let dm =
        resolve_and_validate_channel(&ctx, &origin, Some("301"), ChannelPurpose::SendMessage).await;

    assert!(
        dm.is_err(),
        "DM channel 301 in guild context should be rejected"
    );

    let dm_err = dm.unwrap_err().to_string();

    assert!(dm_err.contains("private channel") || dm_err.contains("kanał prywatny"));

    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_modifications,
        "Zero modifying requests should be made for DM channel"
    );

    // 4. Category channel -> Rejected for SendMessage

    let category =
        resolve_and_validate_channel(&ctx, &origin, Some("401"), ChannelPurpose::SendMessage).await;

    assert!(
        category.is_err(),
        "Category channel 401 should be rejected for SendMessage"
    );

    let cat_err = category.unwrap_err().to_string();

    assert!(cat_err.contains("channel categories") || cat_err.contains("kategoria kanałów"));

    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_modifications,
        "Zero modifying requests should be made for Category channel"
    );

    // 5. Non-existent channel -> Rejected on read error

    let not_found =
        resolve_and_validate_channel(&ctx, &origin, Some("999999"), ChannelPurpose::SendMessage)
            .await;

    assert!(
        not_found.is_err(),
        "Non-existent channel should fail to read"
    );

    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_modifications,
        "Zero modifying requests should be made when reading channel fails"
    );

    // 6. Channel on same guild but actor denied SEND_MESSAGES via overwrite -> Rejected BEFORE modifying requests

    let non_admin_origin = ActionOrigin {
        guild_id: Some(origin_guild_id),
        channel_id: Some(ChannelId::new(101)),
        actor_id: Some(UserId::new(111111111111111111)),
        actor_permissions: serenity::all::Permissions::SEND_MESSAGES,
        actor_roles: Vec::new(),
        app_permissions: serenity::all::Permissions::all(),
        enforce_actor_permissions: true,
    };

    let denied_channel = resolve_and_validate_channel(
        &ctx,
        &non_admin_origin,
        Some("102"),
        ChannelPurpose::SendMessage,
    )
    .await;

    assert!(
        denied_channel.is_err(),
        "channel 102 with denied SEND_MESSAGES should be rejected"
    );

    let denied_err = denied_channel.unwrap_err().to_string();

    assert!(
        denied_err.contains("missing user permissions")
            || denied_err.contains("brak uprawnień użytkownika")
    );

    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_modifications,
        "Zero modifying requests should be made when actor lacks permission in target channel"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]

async fn test_join_guard_raid_kick_timeout_and_self_punishment_contract() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;

    let state = create_test_state().await;

    let guild_id = GuildId::new(100);

    let target_raid_user = UserId::new(222222222222222222);

    let initial_modifications = mock.modifying_requests_count.load(Ordering::SeqCst);

    // Case A: Full event path for kick and timeout from automated Join Guard

    // Actor is the new member (target_raid_user), but automated rules must NOT trigger self-punishment rejection!

    let actions = vec![
        LuaAction::KickMember {
            user_id: target_raid_user.get().to_string(),

            reason: Some("Join Guard: raid lockdown".to_string()),
        },
        LuaAction::SendMessage {
            channel_id: Some("101".to_string()),

            content: "🚨 Staff Alert: user kicked".to_string(),
        },
    ];

    let result = execute_event_actions(
        &ctx,
        &state,
        "join_guard",
        Some(guild_id),
        Some(ChannelId::new(101)),
        Some(target_raid_user),
        &actions,
    )
    .await;

    assert!(
        result.is_ok(),
        "automated event actions must succeed without self-punishment error"
    );

    // Both the Kick (DELETE) and the Alert (POST) must have been registered

    let modifications_after_raid = mock.modifying_requests_count.load(Ordering::SeqCst);

    assert_eq!(
        modifications_after_raid - initial_modifications,
        2,
        "Kick and staff alert message must both be dispatched"
    );

    // Case B: Protected target (Server Owner 111111111111111111)

    // Server owner cannot be moderated, action is denied, but audit entry is created and alert message IS STILL SENT!

    let owner_user = UserId::new(111111111111111111);

    let protected_actions = vec![
        LuaAction::TimeoutMember {
            user_id: owner_user.get().to_string(),

            seconds: 3600,

            reason: Some("Attempt to moderate owner".to_string()),
        },
        LuaAction::SendMessage {
            channel_id: Some("101".to_string()),

            content: "🛡️ Staff Alert: attempted action on owner".to_string(),
        },
    ];

    let baseline = mock.modifying_requests_count.load(Ordering::SeqCst);

    let protected_result = execute_event_actions(
        &ctx,
        &state,
        "join_guard",
        Some(guild_id),
        Some(ChannelId::new(101)),
        Some(owner_user),
        &protected_actions,
    )
    .await;

    assert!(
        protected_result.is_ok(),
        "execute_event_actions gracefully absorbs denied actions"
    );

    // Zero modifying requests for the timeout (owner protected!), exactly 1 for the staff alert!

    let after_protected = mock.modifying_requests_count.load(Ordering::SeqCst);

    assert_eq!(
        after_protected - baseline,
        1,
        "Owner timeout was rejected, but staff alert was still successfully sent"
    );

    // Case C: Manual moderator command - self punishment is strictly rejected

    let self_check = ensure_member_hierarchy(
        &ctx,
        guild_id,
        Some(target_raid_user), // moderator acting manually
        target_raid_user,       // target is self
    )
    .await;

    assert!(
        self_check.is_err(),
        "manual command on self must be rejected"
    );

    let self_err = self_check.unwrap_err().to_string();

    assert!(
        self_err.contains("cannot perform this action on yourself")
            || self_err.contains("nie możesz wykonać tej akcji na sobie")
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]

async fn test_event_action_failures_suppress_sent_audit_and_adjust_message_delete() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;

    let state = create_test_state().await;

    let guild_id = GuildId::new(100);

    // 1. Welcome action failure: channel "999" does not exist (returns 404 in mock).

    // The sequence is: SendMessage (fails) -> Audit "welcome_sent"

    let welcome_actions = vec![
        LuaAction::SendMessage {
            channel_id: Some("999".to_string()),

            content: "Witaj na serwerze!".to_string(),
        },
        LuaAction::Audit {
            event: "welcome_sent".to_string(),

            data: BTreeMap::from([
                ("user_id".to_string(), json!("123456")),
                ("channel_id".to_string(), json!("999")),
            ]),
        },
    ];

    let res = execute_event_actions(
        &ctx,
        &state,
        "welcome",
        Some(guild_id),
        None,
        Some(UserId::new(123456)),
        &welcome_actions,
    )
    .await;

    assert!(
        res.is_ok(),
        "execute_event_actions gracefully absorbs failures"
    );

    let audit_events = state.storage.list_audit_events("100", 20).await.unwrap();

    // Must NOT contain false positive welcome_sent

    assert!(
        !audit_events.iter().any(|e| e.event == "welcome_sent"),
        "failed welcome SendMessage must not emit false positive welcome_sent audit event"
    );

    // Must contain action_denied for failed message sending

    assert!(
        audit_events.iter().any(|e| e.event == "action_denied"),
        "failure should be recorded as action_denied audit entry"
    );

    // 2. Goodbye action failure: channel "999" does not exist

    let goodbye_actions = vec![
        LuaAction::SendMessage {
            channel_id: Some("999".to_string()),

            content: "Żegnaj!".to_string(),
        },
        LuaAction::Audit {
            event: "goodbye_sent".to_string(),

            data: BTreeMap::from([
                ("user_id".to_string(), json!("123456")),
                ("channel_id".to_string(), json!("999")),
            ]),
        },
    ];

    let res_goodbye = execute_event_actions(
        &ctx,
        &state,
        "welcome",
        Some(guild_id),
        None,
        Some(UserId::new(123456)),
        &goodbye_actions,
    )
    .await;

    assert!(res_goodbye.is_ok());

    let audit_events_after = state.storage.list_audit_events("100", 20).await.unwrap();

    assert!(
        !audit_events_after.iter().any(|e| e.event == "goodbye_sent"),
        "failed goodbye SendMessage must not emit false positive goodbye_sent audit event"
    );

    // 3. AutoMod message deletion failure:

    // Message ID "denied_msg_777" fails with 403 Forbidden.

    // Subsequent SendMessage notice should be adjusted:

    // "została usunięta przez AutoMod" -> "narusza zasady AutoMod (usunięcie zablokowane)"

    let automod_actions = vec![
        LuaAction::DeleteMessage {
            channel_id: "101".to_string(),

            message_id: "denied_msg_777".to_string(),
        },
        LuaAction::SendMessage {
            channel_id: Some("101".to_string()),

            content: "<@123456> Wiadomość została usunięta przez AutoMod (blocked_word)."
                .to_string(),
        },
    ];

    let res_automod = execute_event_actions(
        &ctx,
        &state,
        "automod",
        Some(guild_id),
        Some(ChannelId::new(101)),
        Some(UserId::new(123456)),
        &automod_actions,
    )
    .await;

    assert!(res_automod.is_ok());

    // Verify calls recorded by the mock server

    let calls = mock.recorded_calls.lock().await.clone();

    let post_calls: Vec<_> = calls
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();

    assert!(
        !post_calls.is_empty(),
        "AutoMod notice message must still be dispatched"
    );

    let last_post = post_calls.last().unwrap();

    assert!(
        last_post
            .1
            .contains("narusza zasady AutoMod (usunięcie zablokowane)"),
        "dependent notice content should have been adjusted to indicate deletion was blocked, got: {}",
        last_post.1
    );

    assert!(
        !last_post.1.contains("została usunięta przez AutoMod"),
        "dependent notice must not falsely claim message was deleted when deletion failed, got: {}",
        last_post.1
    );

    let final_audits = state.storage.list_audit_events("100", 20).await.unwrap();

    assert!(
        final_audits
            .iter()
            .any(|e| e.event == "action_denied" && e.module_id == "automod"),
        "AutoMod message deletion failure must record action_denied audit entry"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_message_update_author_verification_and_automod_guard() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state().await;
    let handler = Handler::new(state.clone());

    // 1. Enable AutoMod on guild 100 with invite blocking and message deletion
    state
        .storage
        .update_module_settings_with_audit(UpdateModuleSettingsParams {
            guild_id: "100",
            module_id: "automod",
            enabled: true,
            config: json!({
                "block_invites": true,
                "delete_messages": true,
                "send_notice": true,
            }),
            actor_id: Some("111111111111111111"),
            audit_event: "module_configured",
            audit_data: json!({"enabled": true}),
        })
        .await
        .expect("enable automod settings");

    let baseline_modifications = mock.modifying_requests_count.load(Ordering::SeqCst);
    let initial_unknown_author = state
        .metrics
        .message_updates_unknown_author_total
        .load(Ordering::Relaxed);

    // Scenario A: Partial message_update with author=None, new=None, blocked invite content
    // Criteria: 0 modifying actions executed (no delete, no notice), 0 automod audit entries, counter incremented
    let event_no_author: MessageUpdateEvent = serde_json::from_value(json!({
        "id": "900000000000000010",
        "channel_id": "101",
        "guild_id": "100",
        "content": "check this out: https://discord.gg/malicious-invite",
    }))
    .expect("deserialize event_no_author");

    handler
        .message_update(ctx.clone(), None, None, event_no_author)
        .await;

    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_modifications,
        "Partial message_update with unknown author must NOT trigger any modifying requests (fail-closed)"
    );

    let audits_a = state.storage.list_audit_events("100", 20).await.unwrap();
    assert!(
        !audits_a
            .iter()
            .any(|e| e.event == "automod_rule_match" || e.event == "automod_edit_rule_match"),
        "No AutoMod rule match audit should be recorded when author is unknown"
    );

    assert_eq!(
        state
            .metrics
            .message_updates_unknown_author_total
            .load(Ordering::Relaxed),
        initial_unknown_author + 1,
        "Diagnostic counter message_updates_unknown_author_total must be incremented"
    );

    // Scenario B: message_update with event.author.bot=true, new=None
    // Criteria: 0 modifying actions executed, 0 automod audit entries
    let event_bot_author: MessageUpdateEvent = serde_json::from_value(json!({
        "id": "900000000000000011",
        "channel_id": "101",
        "guild_id": "100",
        "content": "bot message with https://discord.gg/malicious-invite",
        "author": {
            "id": "888888888888888888",
            "username": "SomeBot",
            "discriminator": "0000",
            "avatar": null,
            "bot": true,
        },
    }))
    .expect("deserialize event_bot_author");

    handler
        .message_update(ctx.clone(), None, None, event_bot_author)
        .await;

    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_modifications,
        "Bot message_update must NOT trigger AutoMod actions"
    );

    let audits_b = state.storage.list_audit_events("100", 20).await.unwrap();
    assert!(
        !audits_b
            .iter()
            .any(|e| e.event == "automod_rule_match" || e.event == "automod_edit_rule_match"),
        "No AutoMod rule match audit should be recorded for bot message update"
    );

    // Scenario C: message_update with event.author=None, but new.author.bot=true (cached bot message)
    // Criteria: 0 modifying actions executed, 0 automod audit entries
    let event_cached_bot: MessageUpdateEvent = serde_json::from_value(json!({
        "id": "900000000000000012",
        "channel_id": "101",
        "guild_id": "100",
        "content": "cached bot message with https://discord.gg/malicious-invite",
    }))
    .expect("deserialize event_cached_bot");

    let new_bot_msg: Message = serde_json::from_value(json!({
        "id": "900000000000000012",
        "channel_id": "101",
        "guild_id": "100",
        "author": {
            "id": "888888888888888888",
            "username": "CachedBot",
            "discriminator": "0000",
            "avatar": null,
            "bot": true,
        },
        "content": "cached bot message with https://discord.gg/malicious-invite",
        "timestamp": "2026-10-31T07:00:00Z",
        "tts": false,
        "mention_everyone": false,
        "mentions": [],
        "mention_roles": [],
        "attachments": [],
        "embeds": [],
        "pinned": false,
        "type": 0,
    }))
    .expect("deserialize new_bot_msg");

    handler
        .message_update(ctx.clone(), None, Some(new_bot_msg), event_cached_bot)
        .await;

    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_modifications,
        "Cached bot message_update must NOT trigger AutoMod actions"
    );

    let audits_c = state.storage.list_audit_events("100", 20).await.unwrap();
    assert!(
        !audits_c
            .iter()
            .any(|e| e.event == "automod_rule_match" || e.event == "automod_edit_rule_match"),
        "No AutoMod rule match audit should be recorded for cached bot message update"
    );

    // Scenario D: message_update with confirmed human author (author.bot=false)
    // Criteria: Normal AutoMod path executes without regression (deletes violating message, sends notice, records audit)
    let event_human_author: MessageUpdateEvent = serde_json::from_value(json!({
        "id": "900000000000000013",
        "channel_id": "101",
        "guild_id": "100",
        "content": "human edit containing https://discord.gg/malicious-invite",
        "author": {
            "id": "777777777777777777",
            "username": "HumanMember",
            "discriminator": "0000",
            "avatar": null,
            "bot": false,
        },
    }))
    .expect("deserialize event_human_author");

    handler
        .message_update(ctx.clone(), None, None, event_human_author)
        .await;

    // Must have executed delete_message (DELETE) and notice (POST)
    assert!(
        mock.modifying_requests_count.load(Ordering::SeqCst) > baseline_modifications,
        "Verified human author edit must trigger AutoMod modifying requests"
    );

    let audits_d = state.storage.list_audit_events("100", 20).await.unwrap();
    assert!(
        audits_d
            .iter()
            .any(|e| e.event == "automod_edit_rule_match" && e.module_id == "automod"),
        "AutoMod edit rule match audit must be recorded for confirmed human author"
    );

    // Scenario E: Embed-only message_update with confirmed human author in cache but content=None (Issue #266)
    // Criteria: 0 modifying actions executed (no delete, no notice), 0 new automod audit entries
    let baseline_after_d = mock.modifying_requests_count.load(Ordering::SeqCst);
    let audits_count_after_d = state
        .storage
        .list_audit_events("100", 50)
        .await
        .unwrap()
        .len();

    let event_embed_only: MessageUpdateEvent = serde_json::from_value(json!({
        "id": "900000000000000014",
        "channel_id": "101",
        "guild_id": "100",
        "content": null,
        "embeds": [{
            "title": "Embedded Link Preview",
            "type": "link",
            "url": "https://discord.gg/malicious-invite"
        }],
    }))
    .expect("deserialize event_embed_only");

    let cached_human_msg: Message = serde_json::from_value(json!({
        "id": "900000000000000014",
        "channel_id": "101",
        "guild_id": "100",
        "author": {
            "id": "777777777777777777",
            "username": "HumanMember",
            "discriminator": "0000",
            "avatar": null,
            "bot": false,
        },
        "content": "message with https://discord.gg/malicious-invite",
        "timestamp": "2026-10-31T07:00:00Z",
        "tts": false,
        "mention_everyone": false,
        "mentions": [],
        "mention_roles": [],
        "attachments": [],
        "embeds": [],
        "pinned": false,
        "type": 0,
    }))
    .expect("deserialize cached_human_msg");

    handler
        .message_update(
            ctx.clone(),
            Some(cached_human_msg.clone()),
            Some(cached_human_msg),
            event_embed_only,
        )
        .await;

    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_after_d,
        "Embed-only message_update without content change must NOT trigger AutoMod actions (Issue #266)"
    );

    let audits_e = state.storage.list_audit_events("100", 50).await.unwrap();
    assert_eq!(
        audits_e.len(),
        audits_count_after_d,
        "No additional AutoMod audit entries should be recorded for embed-only update"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_action_policy_denial_blocks_executor_without_discord_http_call() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state().await;
    let guild_id = GuildId::new(100);
    let target_user = UserId::new(333333333333333333);

    // Upsert Action Policy Rule: deny "ban_member" guild-wide
    let rule = rusty_discord_bot::policy::CommandPolicyRule {
        id: uuid::Uuid::new_v4(),
        guild_id: "100".to_string(),
        subject_kind: "guild".to_string(),
        subject_id: None,
        resource_kind: "action".to_string(),
        module_id: None,
        command_name: None,
        action_key: Some("ban_member".to_string()),
        effect: "deny".to_string(),
        priority: 0,
        enabled: true,
        version: 1,
        created_by: "test_admin".to_string(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    state
        .storage
        .upsert_policy_rule(rule)
        .await
        .expect("upsert action policy rule");

    let initial_modifications = mock.modifying_requests_count.load(Ordering::SeqCst);

    let actions = vec![LuaAction::BanMember {
        user_id: target_user.get().to_string(),
        delete_message_days: 0,
        reason: Some("Test ban blocked by policy".to_string()),
    }];

    let result = execute_event_actions(
        &ctx,
        &state,
        "moderation",
        Some(guild_id),
        Some(ChannelId::new(101)),
        None,
        &actions,
    )
    .await;

    assert!(
        result.is_ok(),
        "execute_event_actions handles failure gracefully"
    );

    // Zero modifying requests to Discord API! The action was blocked by Action Policy before Discord HTTP call.
    let after_modifications = mock.modifying_requests_count.load(Ordering::SeqCst);
    assert_eq!(
        after_modifications, initial_modifications,
        "Blocked action must NOT make any modifying HTTP requests to Discord"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_voice_companion_disabled_mapping_never_grants_and_revokes_active_lease() {
    use rusty_discord_bot::discord::companion::{
        grant_companion_access, reconcile_companion_channel_access,
        reconcile_guild_companion_channels,
    };
    use rusty_discord_bot::storage::{CompanionMappingItem, RecordCompanionLeaseParams};

    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state().await;

    let guild_id = GuildId::new(100);
    let voice_ch = ChannelId::new(105);
    let _text_ch = ChannelId::new(101); // text channel 101 exists in mock
    let user_id = UserId::new(222222222222222222);

    let guild_str = "100";
    let voice_str = "105";
    let text_str = "101";
    let user_str = "222222222222222222";

    // 1. Create a companion mapping with enabled = false (#262)
    let item = CompanionMappingItem::new(voice_str, text_str, true, true).with_enabled(false);
    state
        .storage
        .batch_upsert_companion_mappings(guild_str, &[item])
        .await
        .expect("batch upsert disabled companion mapping");

    let mapping = state
        .storage
        .get_companion_mapping_by_voice(guild_str, voice_str)
        .await
        .expect("lookup mapping")
        .expect("mapping exists");
    assert!(!mapping.enabled, "mapping must have enabled=false in model");

    // 2. Direct grant_companion_access must refuse disabled mapping (defense-in-depth)
    let initial_modifications = mock.modifying_requests_count.load(Ordering::SeqCst);
    grant_companion_access(&ctx, &state, &mapping, user_id)
        .await
        .expect("grant_companion_access on disabled mapping returns Ok safely");

    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        initial_modifications,
        "grant_companion_access must NOT make HTTP requests for disabled mapping"
    );
    let no_lease = state
        .storage
        .get_companion_lease(guild_str, text_str, user_str)
        .await
        .expect("lease lookup");
    assert!(
        no_lease.is_none(),
        "No lease should be created for disabled mapping"
    );

    // 3. User enters voice channel: reconcile_companion_channel_access without lease must NOT grant access
    reconcile_companion_channel_access(&ctx, &state, guild_id, user_id, None, Some(voice_ch))
        .await
        .expect("reconcile voice access");

    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        initial_modifications,
        "Voice event with disabled mapping must NOT make HTTP requests to Discord"
    );
    let still_no_lease = state
        .storage
        .get_companion_lease(guild_str, text_str, user_str)
        .await
        .expect("lease lookup");
    assert!(still_no_lease.is_none());

    // 4. User has an active lease (e.g. from before the mapping was disabled):
    // Voice event must NOT keep the lease; it must revoke it.
    let generation_id = uuid::Uuid::new_v4();
    state
        .storage
        .record_companion_lease(RecordCompanionLeaseParams {
            guild_id: guild_str,
            voice_channel_id: voice_str,
            text_channel_id: text_str,
            user_id: user_str,
            generation: generation_id,
            allow_bits: 1024,
            deny_bits: 0,
        })
        .await
        .expect("seed active lease");

    let seeded_lease = state
        .storage
        .get_companion_lease(guild_str, text_str, user_str)
        .await
        .expect("lease lookup")
        .expect("seeded lease exists");
    assert_eq!(seeded_lease.status, "active");

    // Reconcile user voice event on the same disabled voice channel:
    // Should trigger revoke instead of keeping the lease!
    reconcile_companion_channel_access(
        &ctx,
        &state,
        guild_id,
        user_id,
        Some(voice_ch),
        Some(voice_ch),
    )
    .await
    .expect("reconcile voice access with existing lease");

    let lease_after_voice = state
        .storage
        .get_companion_lease(guild_str, text_str, user_str)
        .await
        .expect("lease lookup");
    // Since mock Discord handler returns 200 OK for delete_permission, lease is deleted from storage
    assert!(
        lease_after_voice.is_none(),
        "Active lease for disabled mapping must be revoked and removed on voice event"
    );

    // 5. Guild reconcile without cache must defer (#263); with reliable voice states it revokes Phase 2 and NOT re-grant in Phase 3
    state
        .storage
        .record_companion_lease(RecordCompanionLeaseParams {
            guild_id: guild_str,
            voice_channel_id: voice_str,
            text_channel_id: text_str,
            user_id: user_str,
            generation: uuid::Uuid::new_v4(),
            allow_bits: 1024,
            deny_bits: 0,
        })
        .await
        .expect("seed active lease for guild reconcile");

    // Reconcile without guild in cache must fail fast and defer (#263)
    let defer_res = reconcile_guild_companion_channels(&ctx, &state, guild_id).await;
    assert!(defer_res.is_err(), "must defer when cache unavailable");
    assert!(
        defer_res
            .unwrap_err()
            .to_string()
            .contains("cache is unavailable")
    );

    // Reconcile with explicit reliable voice states (empty) revokes disabled lease
    let summary = rusty_discord_bot::discord::companion::reconcile_guild_companion_channels_with_voice_states(
        &ctx,
        &state,
        guild_id,
        std::collections::HashMap::new(),
    )
    .await
    .expect("reconcile guild companion channels with voice states");

    assert_eq!(
        summary.revoked, 1,
        "Phase 2 must revoke active lease of disabled mapping"
    );
    assert_eq!(
        summary.granted, 0,
        "Phase 3 must NOT re-grant access for disabled mapping"
    );
    assert_eq!(
        summary.active_leases, 0,
        "No active leases should remain for disabled mapping"
    );

    let final_lease = state
        .storage
        .get_companion_lease(guild_str, text_str, user_str)
        .await
        .expect("lease lookup");
    assert!(final_lease.is_none());

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_voice_companion_ready_empty_cache_defers_reconciliation_without_revocation() {
    use rusty_discord_bot::discord::companion::{
        reconcile_guild_companion_channels, reconcile_guild_companion_channels_with_voice_states,
    };
    use rusty_discord_bot::storage::{CompanionMappingItem, RecordCompanionLeaseParams};
    use serenity::all::VoiceState;

    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state().await;

    let guild_id = GuildId::new(100);
    let voice_str = "105";
    let text_str = "101"; // text channel 101 exists in mock
    let user_str = "222222222222222222";
    let guild_str = "100";

    // 1. Setup enabled companion mapping
    let item = CompanionMappingItem::new(voice_str, text_str, true, true);
    state
        .storage
        .batch_upsert_companion_mappings(guild_str, &[item])
        .await
        .expect("upsert companion mapping");

    // 2. Setup active companion lease for user in this voice channel
    let generation_id = uuid::Uuid::new_v4();
    state
        .storage
        .record_companion_lease(RecordCompanionLeaseParams {
            guild_id: guild_str,
            voice_channel_id: voice_str,
            text_channel_id: text_str,
            user_id: user_str,
            generation: generation_id,
            allow_bits: 1024,
            deny_bits: 0,
        })
        .await
        .expect("seed active lease");

    let initial_modifications = mock.modifying_requests_count.load(Ordering::SeqCst);

    // 3. Scenario: READY before GUILD_CREATE (cache does NOT have guild 100 yet).
    // Reconcile must fail fast with deferral error and NOT revoke active leases (#263).
    let reconcile_res = reconcile_guild_companion_channels(&ctx, &state, guild_id).await;
    assert!(
        reconcile_res.is_err(),
        "Must return Err when guild cache is unavailable"
    );
    let err_msg = reconcile_res.unwrap_err().to_string();
    assert!(
        err_msg.contains("cache is unavailable; deferring companion reconciliation"),
        "Error message must specify deferral due to unavailable cache, got: {}",
        err_msg
    );

    // Zero modifying requests to Discord (no DELETE /permissions)!
    let after_defer_modifications = mock.modifying_requests_count.load(Ordering::SeqCst);
    assert_eq!(
        after_defer_modifications, initial_modifications,
        "Deferral must NOT issue any modifying HTTP calls (like delete_permission) to Discord"
    );

    // Lease in storage must remain untouched with status 'active'
    let lease_during_deferral = state
        .storage
        .get_companion_lease(guild_str, text_str, user_str)
        .await
        .expect("lease query")
        .expect("lease must still exist");
    assert_eq!(
        lease_during_deferral.status, "active",
        "Active lease must NOT be marked revoke_pending or deleted when cache is unavailable"
    );

    // 4. Scenario: GUILD_CREATE arrives and user is still in voice channel 105.
    // Active lease must be preserved!
    let mut current_voice_states = std::collections::HashMap::new();
    let voice_state: VoiceState = serde_json::from_value(serde_json::json!({
        "guild_id": "100",
        "channel_id": "105",
        "user_id": user_str,
        "session_id": "session_active_voice",
        "deaf": false,
        "mute": false,
        "self_deaf": false,
        "self_mute": false,
        "self_video": false,
        "suppress": false,
        "request_to_speak_timestamp": null
    }))
    .expect("deserialize VoiceState");
    current_voice_states.insert(UserId::new(222222222222222222), voice_state);

    let sync_summary = reconcile_guild_companion_channels_with_voice_states(
        &ctx,
        &state,
        guild_id,
        current_voice_states,
    )
    .await
    .expect("reconciliation with active voice states succeeds");

    assert_eq!(
        sync_summary.revoked, 0,
        "No active leases should be revoked when member is in voice"
    );
    assert_eq!(
        sync_summary.granted, 0,
        "No duplicate grant should be issued for already active lease"
    );
    assert_eq!(sync_summary.active_leases, 1, "Active lease remains intact");

    let preserved_lease = state
        .storage
        .get_companion_lease(guild_str, text_str, user_str)
        .await
        .expect("lease query")
        .expect("lease exists");
    assert_eq!(preserved_lease.status, "active");

    // 5. Scenario: User left voice channel while bot was offline.
    // Reconcile with empty voice states correctly revokes the stale lease.
    let empty_states = std::collections::HashMap::new();
    let cleanup_summary =
        reconcile_guild_companion_channels_with_voice_states(&ctx, &state, guild_id, empty_states)
            .await
            .expect("cleanup reconciliation succeeds");

    assert_eq!(
        cleanup_summary.revoked, 1,
        "Stale lease must be revoked when user left voice"
    );
    assert_eq!(cleanup_summary.active_leases, 0);

    let final_lease = state
        .storage
        .get_companion_lease(guild_str, text_str, user_str)
        .await
        .expect("lease query");
    assert!(
        final_lease.is_none(),
        "Stale lease must be deleted after successful revocation"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_voice_companion_delayed_guild_create_reconciliation_race_safe() {
    use rusty_discord_bot::discord::companion::reconcile_guild_companion_channels_with_voice_states;
    use rusty_discord_bot::storage::{CompanionMappingItem, RecordCompanionLeaseParams};
    use serenity::all::{GuildCreateEvent, VoiceState, VoiceStateUpdateEvent};

    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state().await;

    let guild_id = GuildId::new(100);
    let voice_str = "105";
    let text_str = "101";
    let user_str = "222222222222222222";
    let guild_str = "100";
    let user_id = UserId::new(222222222222222222);

    // 1. Setup enabled companion mapping
    let item = CompanionMappingItem::new(voice_str, text_str, true, true);
    state
        .storage
        .batch_upsert_companion_mappings(guild_str, &[item])
        .await
        .expect("upsert companion mapping");

    // 2. Setup active lease
    let generation_id = uuid::Uuid::new_v4();
    state
        .storage
        .record_companion_lease(RecordCompanionLeaseParams {
            guild_id: guild_str,
            voice_channel_id: voice_str,
            text_channel_id: text_str,
            user_id: user_str,
            generation: generation_id,
            allow_bits: 1024,
            deny_bits: 0,
        })
        .await
        .expect("seed active lease");

    // 3. Populate live guild cache with user present in voice channel 105
    let guild_raw_str = format!(
        r#"{{
        "id": "100",
        "name": "Test Guild",
        "owner_id": "111111111111111111",
        "roles": [],
        "emojis": [],
        "stickers": [],
        "features": [],
        "icon": null,
        "splash": null,
        "banner": null,
        "description": null,
        "verification_level": 0,
        "default_message_notifications": 0,
        "explicit_content_filter": 0,
        "afk_channel_id": null,
        "afk_timeout": 300,
        "system_channel_id": null,
        "system_channel_flags": 0,
        "rules_channel_id": null,
        "public_updates_channel_id": null,
        "preferred_locale": "en-US",
        "mfa_level": 0,
        "nsfw_level": 0,
        "premium_tier": 0,
        "premium_subscription_count": 0,
        "premium_progress_bar_enabled": false,
        "large": false,
        "unavailable": false,
        "member_count": 1,
        "joined_at": "2026-01-01T00:00:00Z",
        "threads": [],
        "presences": [],
        "stage_instances": [],
        "guild_scheduled_events": [],
        "channels": [{{
            "id": "101",
            "guild_id": "100",
            "name": "companion-text",
            "type": 0,
            "permission_overwrites": []
        }}],
        "members": [],
        "voice_states": [{{
            "guild_id": "100",
            "channel_id": "105",
            "user_id": "{user_str}",
            "session_id": "live_session_1",
            "deaf": false,
            "mute": false,
            "self_deaf": false,
            "self_mute": false,
            "self_video": false,
            "suppress": false,
            "request_to_speak_timestamp": null
        }}]
    }}"#
    );

    let mut guild_create_event: GuildCreateEvent =
        serde_json::from_str(&guild_raw_str).expect("deserialize guild_create_event");
    ctx.cache.update(&mut guild_create_event);

    let initial_modifications = mock.modifying_requests_count.load(Ordering::SeqCst);

    // Scenario A: GUILD_CREATE carried a stale EMPTY snapshot (taken before user joined voice).
    // Reconcile must check live cache and NOT revoke the active lease (Issue #268).
    let stale_empty_snapshot = std::collections::HashMap::new();
    let summary_a = reconcile_guild_companion_channels_with_voice_states(
        &ctx,
        &state,
        guild_id,
        stale_empty_snapshot,
    )
    .await
    .expect("reconcile with stale empty snapshot");

    assert_eq!(
        summary_a.revoked, 0,
        "Delayed GUILD_CREATE reconcile with empty snapshot must NOT revoke lease when user is present in live cache"
    );
    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        initial_modifications,
        "Zero HTTP delete requests should be sent to Discord"
    );

    let lease_a = state
        .storage
        .get_companion_lease(guild_str, text_str, user_str)
        .await
        .unwrap()
        .expect("lease must remain active");
    assert_eq!(lease_a.status, "active");

    // Scenario B: User now disconnected in live cache (VOICE_STATE_UPDATE arrived).
    // Update live cache with channel_id: null
    let mut disconnect_event: VoiceStateUpdateEvent = serde_json::from_value(serde_json::json!({
        "guild_id": "100",
        "channel_id": null,
        "user_id": user_str,
        "session_id": "live_session_1",
        "deaf": false,
        "mute": false,
        "self_deaf": false,
        "self_mute": false,
        "self_video": false,
        "suppress": false,
        "request_to_speak_timestamp": null
    }))
    .unwrap();
    ctx.cache.update(&mut disconnect_event);

    // Now a delayed GUILD_CREATE carries a stale snapshot with user in voice.
    // Reconcile must check live cache, realize user disconnected, and NOT re-grant (Issue #268).
    let mut stale_populated_snapshot = std::collections::HashMap::new();
    let voice_state: VoiceState = serde_json::from_value(serde_json::json!({
        "guild_id": "100",
        "channel_id": "105",
        "user_id": user_str,
        "session_id": "stale_session",
        "deaf": false,
        "mute": false,
        "self_deaf": false,
        "self_mute": false,
        "self_video": false,
        "suppress": false,
        "request_to_speak_timestamp": null
    }))
    .unwrap();
    stale_populated_snapshot.insert(user_id, voice_state);

    // Also remove the lease from storage to test if phase 3 re-grants it inappropriately
    state
        .storage
        .delete_companion_lease(guild_str, text_str, user_str)
        .await
        .unwrap();

    let summary_b = reconcile_guild_companion_channels_with_voice_states(
        &ctx,
        &state,
        guild_id,
        stale_populated_snapshot,
    )
    .await
    .expect("reconcile with stale populated snapshot");

    assert_eq!(
        summary_b.granted, 0,
        "Delayed GUILD_CREATE reconcile must NOT re-grant access when user disconnected in live cache"
    );

    let lease_b = state
        .storage
        .get_companion_lease(guild_str, text_str, user_str)
        .await
        .unwrap();
    assert!(
        lease_b.is_none(),
        "Lease must NOT be created for disconnected member"
    );

    shard_manager.shutdown_all().await;
}
