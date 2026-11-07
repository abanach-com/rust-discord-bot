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
        economy::{self, EconomyExecutionContext},
        executor::{
            ActionOrigin, ChannelPurpose, ensure_member_hierarchy, execute_event_actions,
            resolve_and_validate_channel,
        },
    },
    lua::{EconomyOperation, LuaAction},
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

    let ctx = tokio::time::timeout(std::time::Duration::from_secs(15), ready_rx)
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

async fn create_test_state_with_port(port: u16) -> AppState {
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
        discord_api_base_url: format!("http://127.0.0.1:{port}"),
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

        interaction_id: None,
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
        interaction_id: None,
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
    let state = create_test_state_with_port(mock.port).await;
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
    let state = create_test_state_with_port(mock.port).await;
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
    let state = create_test_state_with_port(mock.port).await;

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
    let state = create_test_state_with_port(mock.port).await;

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
    let state = create_test_state_with_port(mock.port).await;

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

#[tokio::test]
async fn test_tag_mutation_from_event_and_untrusted_module_strictly_denied() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state_with_port(mock.port).await;
    let guild_id = GuildId::new(100);
    let guild_id_str = "100";
    let initial_modifications = mock.modifying_requests_count.load(Ordering::SeqCst);

    // 1. Tag Create from event (even with module_id = "tags") -> strictly denied, 0 storage mutations
    let create_action = vec![LuaAction::Tag {
        operation: rusty_discord_bot::lua::TagOperation::Create {
            name: "event_exploit".to_string(),
            content: "malicious tag".to_string(),
        },
    }];
    let res = execute_event_actions(
        &ctx,
        &state,
        "tags",
        Some(guild_id),
        Some(ChannelId::new(101)),
        Some(UserId::new(555)),
        &create_action,
    )
    .await;
    assert!(
        res.is_ok(),
        "execute_event_actions handles failures gracefully"
    );

    let tag_check = state
        .storage
        .get_tag(guild_id_str, "event_exploit")
        .await
        .unwrap();
    assert!(
        tag_check.is_none(),
        "Tag must NOT be created from an event callback"
    );

    // 2. Tag Create from untrusted module (e.g. "community") -> strictly denied
    let res_untrusted = execute_event_actions(
        &ctx,
        &state,
        "community",
        Some(guild_id),
        Some(ChannelId::new(101)),
        Some(UserId::new(555)),
        &create_action,
    )
    .await;
    assert!(res_untrusted.is_ok());
    assert!(
        state
            .storage
            .get_tag(guild_id_str, "event_exploit")
            .await
            .unwrap()
            .is_none()
    );

    // 3. Pre-seed a tag directly in storage, then attempt Tag Delete from event
    state
        .storage
        .set_tag(
            guild_id_str,
            rusty_discord_bot::storage::TagRecord {
                name: "legit_tag".to_string(),
                content: "official content".to_string(),
                author_id: "admin_1".to_string(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
        )
        .await
        .unwrap();

    let delete_action = vec![LuaAction::Tag {
        operation: rusty_discord_bot::lua::TagOperation::Delete {
            name: "legit_tag".to_string(),
        },
    }];
    let res_del = execute_event_actions(
        &ctx,
        &state,
        "tags",
        Some(guild_id),
        Some(ChannelId::new(101)),
        Some(UserId::new(555)),
        &delete_action,
    )
    .await;
    assert!(res_del.is_ok());

    // Verify tag still exists in storage (not deleted)
    let tag_still_there = state
        .storage
        .get_tag(guild_id_str, "legit_tag")
        .await
        .unwrap();
    assert!(
        tag_still_there.is_some(),
        "Legitimate tag must NOT be deleted from event callback"
    );

    // Verify zero modifying HTTP requests were made to Discord
    let after_modifications = mock.modifying_requests_count.load(Ordering::SeqCst);
    assert_eq!(after_modifications, initial_modifications);

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_giveaway_worker_cross_guild_channel_blocked_and_quarantined() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state_with_port(mock.port).await;

    let origin_guild = "100";
    let foreign_channel = "201"; // belongs to guild 200 in mock
    let category_channel = "401"; // category channel in mock
    let legitimate_channel = "101"; // legitimate channel in guild 100

    // 1. Seed historical expired giveaway targeting foreign channel 201 (guild 200)
    let cross_ga_id = uuid::Uuid::new_v4();
    state
        .storage
        .create_giveaway_with_id(
            cross_ga_id,
            origin_guild,
            foreign_channel,
            "Cross-Guild Nitro",
            1,
            chrono::Utc::now() - chrono::Duration::minutes(5),
            Some("111111111111111111"),
        )
        .await
        .unwrap();

    // Add a participant to be drawn as winner
    state
        .storage
        .join_giveaway(cross_ga_id, "555555555555555555")
        .await
        .unwrap();

    // 2. Seed expired giveaway targeting category channel 401
    let category_ga_id = uuid::Uuid::new_v4();
    state
        .storage
        .create_giveaway_with_id(
            category_ga_id,
            origin_guild,
            category_channel,
            "Category Gift",
            1,
            chrono::Utc::now() - chrono::Duration::minutes(5),
            Some("111111111111111111"),
        )
        .await
        .unwrap();

    state
        .storage
        .join_giveaway(category_ga_id, "555555555555555555")
        .await
        .unwrap();

    // 3. Seed expired giveaway targeting legitimate channel 101
    let legit_ga_id = uuid::Uuid::new_v4();
    state
        .storage
        .create_giveaway_with_id(
            legit_ga_id,
            origin_guild,
            legitimate_channel,
            "Legit Server Prize",
            1,
            chrono::Utc::now() - chrono::Duration::minutes(5),
            Some("111111111111111111"),
        )
        .await
        .unwrap();

    state
        .storage
        .join_giveaway(legit_ga_id, "555555555555555555")
        .await
        .unwrap();

    // Process worker cycle
    let finished_count =
        rusty_discord_bot::discord::giveaway::process_expired_giveaways_cycle(&ctx.http, &state)
            .await
            .expect("worker cycle");

    assert_eq!(
        finished_count, 3,
        "All 3 expired giveaways should be transitioned"
    );

    // Verify recorded calls to mock Discord HTTP
    let recorded = mock.recorded_calls.lock().await.clone();

    // Channel 201 (foreign guild) MUST NEVER have received a POST to /messages
    let cross_guild_posts: Vec<_> = recorded
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/201/messages"))
        .collect();
    assert!(
        cross_guild_posts.is_empty(),
        "Tenant isolation violation: worker must NEVER post to foreign channel 201"
    );

    // Channel 401 (category) MUST NEVER have received a POST to /messages
    let category_posts: Vec<_> = recorded
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/401/messages"))
        .collect();
    assert!(
        category_posts.is_empty(),
        "Worker must NEVER post to category channel 401"
    );

    // Channel 101 (legitimate) MUST have received exactly 1 POST to /messages
    let legit_posts: Vec<_> = recorded
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(
        legit_posts.len(),
        1,
        "Worker must announce finish in legitimate channel 101"
    );

    // Verify audit logs
    let audit_logs = state
        .storage
        .list_audit_events(origin_guild, 20)
        .await
        .unwrap();

    let cross_blocked = audit_logs
        .iter()
        .find(|a| a.event == "giveaway_cross_guild_blocked");
    assert!(
        cross_blocked.is_some(),
        "Audit log giveaway_cross_guild_blocked must be recorded"
    );
    let cross_payload = &cross_blocked.unwrap().data;
    assert_eq!(
        cross_payload.get("giveaway_id").and_then(|v| v.as_str()),
        Some(cross_ga_id.to_string().as_str())
    );
    assert_eq!(
        cross_payload
            .get("detected_guild_id")
            .and_then(|v| v.as_str()),
        Some("200")
    );

    let cat_failed = audit_logs.iter().find(|a| {
        a.event == "giveaway_delivery_failed"
            && a.data.get("giveaway_id").and_then(|v| v.as_str())
                == Some(category_ga_id.to_string().as_str())
    });
    assert!(
        cat_failed.is_some(),
        "Audit log giveaway_delivery_failed must be recorded for category channel"
    );

    // Simulate restart / subsequent worker cycle:
    // Calling process_expired_giveaways_cycle again must not re-process or send additional messages
    let cycle2_count =
        rusty_discord_bot::discord::giveaway::process_expired_giveaways_cycle(&ctx.http, &state)
            .await
            .expect("worker cycle restart");
    assert_eq!(
        cycle2_count, 0,
        "Restarting worker must find 0 pending expired giveaways"
    );

    let recorded2 = mock.recorded_calls.lock().await.clone();
    let cross_guild_posts2: Vec<_> = recorded2
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/201/messages"))
        .collect();
    assert!(
        cross_guild_posts2.is_empty(),
        "Subsequent cycles must not produce messages"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_reaction_roles_storage_crud_and_guild_isolation() {
    let state = create_test_state().await;
    let guild_a = "100";
    let guild_b = "200";
    let msg_id = "900000000000000001";
    let role_a = "1001";
    let role_b = "1002";
    let emoji = "🎉";

    // 1. Create binding in Guild A
    let original_id_a = uuid::Uuid::new_v4();
    let original_created_at_a = chrono::Utc::now();
    let binding_a = rusty_discord_bot::storage::ReactionRoleBinding {
        id: original_id_a,
        guild_id: guild_a.to_string(),
        channel_id: Some("101".to_string()),
        message_id: msg_id.to_string(),
        emoji_key: emoji.to_string(),
        role_id: role_a.to_string(),
        created_by: Some("admin_a".to_string()),
        created_at: original_created_at_a,
    };
    state
        .storage
        .upsert_reaction_role_binding(binding_a)
        .await
        .expect("upsert guild a binding");

    // Verify Guild A can read it
    let read_a = state
        .storage
        .get_reaction_role_binding(guild_a, msg_id, emoji)
        .await
        .unwrap();
    assert!(read_a.is_some());
    assert_eq!(read_a.unwrap().role_id, role_a);

    // Verify Guild B cannot read Guild A's binding (tenant isolation)
    let read_b_initial = state
        .storage
        .get_reaction_role_binding(guild_b, msg_id, emoji)
        .await
        .unwrap();
    assert!(read_b_initial.is_none());

    // 2. Create binding in Guild B with same msg_id and emoji, but different role
    let binding_b = rusty_discord_bot::storage::ReactionRoleBinding {
        id: uuid::Uuid::new_v4(),
        guild_id: guild_b.to_string(),
        channel_id: Some("201".to_string()),
        message_id: msg_id.to_string(),
        emoji_key: emoji.to_string(),
        role_id: role_b.to_string(),
        created_by: Some("admin_b".to_string()),
        created_at: chrono::Utc::now(),
    };
    state
        .storage
        .upsert_reaction_role_binding(binding_b.clone())
        .await
        .expect("upsert guild b binding");

    // Verify both guilds are strictly isolated
    let read_a2 = state
        .storage
        .get_reaction_role_binding(guild_a, msg_id, emoji)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read_a2.role_id, role_a);

    let read_b2 = state
        .storage
        .get_reaction_role_binding(guild_b, msg_id, emoji)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read_b2.role_id, role_b);

    // 3. Upsert update (idempotency): change Guild A's role to 1005
    let binding_a_updated = rusty_discord_bot::storage::ReactionRoleBinding {
        id: uuid::Uuid::new_v4(),
        guild_id: guild_a.to_string(),
        channel_id: None, // channel_id: None preserves existing channel_id "101"
        message_id: msg_id.to_string(),
        emoji_key: emoji.to_string(),
        role_id: "1005".to_string(),
        created_by: Some("admin_a".to_string()),
        created_at: chrono::Utc::now() + chrono::Duration::hours(1),
    };
    state
        .storage
        .upsert_reaction_role_binding(binding_a_updated)
        .await
        .expect("upsert update");

    let read_a_updated = state
        .storage
        .get_reaction_role_binding(guild_a, msg_id, emoji)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read_a_updated.role_id, "1005");
    assert_eq!(
        read_a_updated.id, original_id_a,
        "UPSERT must preserve original binding UUID (Issue #287)"
    );
    assert_eq!(
        read_a_updated.created_at, original_created_at_a,
        "UPSERT must preserve original created_at"
    );
    assert_eq!(
        read_a_updated.channel_id,
        Some("101".to_string()),
        "UPSERT with None channel_id preserves existing channel_id"
    );

    // 4. Custom emoji canonicalization
    let custom_emoji_raw = "<:my_emoji:123456789012345678>";
    let canonical =
        rusty_discord_bot::discord::reaction_roles::canonicalize_emoji(custom_emoji_raw);
    assert_eq!(canonical, "123456789012345678");

    let binding_custom = rusty_discord_bot::storage::ReactionRoleBinding {
        id: uuid::Uuid::new_v4(),
        guild_id: guild_a.to_string(),
        channel_id: Some("101".to_string()),
        message_id: msg_id.to_string(),
        emoji_key: canonical.clone(),
        role_id: "1002".to_string(),
        created_by: Some("admin_a".to_string()),
        created_at: chrono::Utc::now(),
    };
    state
        .storage
        .upsert_reaction_role_binding(binding_custom)
        .await
        .unwrap();

    let read_custom = state
        .storage
        .get_reaction_role_binding(guild_a, msg_id, &canonical)
        .await
        .unwrap();
    assert!(read_custom.is_some());

    // 5. Listing bindings
    let list_a = state
        .storage
        .list_reaction_role_bindings(guild_a)
        .await
        .unwrap();
    assert_eq!(list_a.len(), 2);

    let list_b = state
        .storage
        .list_reaction_role_bindings(guild_b)
        .await
        .unwrap();
    assert_eq!(list_b.len(), 1);

    // 6. Delete from Guild A
    let del_a = state
        .storage
        .delete_reaction_role_binding(guild_a, msg_id, emoji)
        .await
        .unwrap();
    assert!(del_a);

    // Guild A binding is gone, Guild B binding is still intact
    assert!(
        state
            .storage
            .get_reaction_role_binding(guild_a, msg_id, emoji)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .storage
            .get_reaction_role_binding(guild_b, msg_id, emoji)
            .await
            .unwrap()
            .is_some()
    );

    // Delete by id on Guild B
    let del_b_id = state
        .storage
        .delete_reaction_role_binding_by_id(guild_b, binding_b.id)
        .await
        .unwrap();
    assert!(del_b_id);
    assert!(
        state
            .storage
            .list_reaction_role_bindings(guild_b)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn test_reaction_roles_executor_permissions_and_hierarchy() {
    use serenity::all::Permissions;

    let (ctx, _mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state().await;
    let guild_id = GuildId::new(100);

    // 1. Untrusted module executing ReactionRole -> Err
    let op_create = rusty_discord_bot::lua::ReactionRoleOperation::Create {
        message_id: "900000000000000001".to_string(),
        emoji: "🎉".to_string(),
        role_id: "1002".to_string(),
    };
    let execution_valid =
        rusty_discord_bot::discord::reaction_roles::ReactionRoleExecutionContext::new(
            Some(guild_id),
            Some(ChannelId::new(101)),
            Some(UserId::new(111111111111111111)), // Owner / Admin
            Permissions::MANAGE_ROLES,
            Permissions::MANAGE_ROLES,
            true,
        );

    let untrusted_res = rusty_discord_bot::discord::reaction_roles::execute_reaction_role_action(
        &ctx,
        &state,
        "untrusted_module",
        execution_valid,
        &op_create,
    )
    .await;
    assert!(untrusted_res.is_err());
    assert!(
        untrusted_res
            .unwrap_err()
            .to_string()
            .contains("reaction_role actions require the trusted reaction_roles module")
    );

    // 2. Non-interactive actor / event execution -> Err
    let execution_non_interactive =
        rusty_discord_bot::discord::reaction_roles::ReactionRoleExecutionContext::new(
            Some(guild_id),
            Some(ChannelId::new(101)),
            Some(UserId::new(111111111111111111)),
            Permissions::MANAGE_ROLES,
            Permissions::MANAGE_ROLES,
            false, // not interactive
        );
    let non_interactive_res =
        rusty_discord_bot::discord::reaction_roles::execute_reaction_role_action(
            &ctx,
            &state,
            "reaction_roles",
            execution_non_interactive,
            &op_create,
        )
        .await;
    assert!(non_interactive_res.is_err());
    assert!(
        non_interactive_res
            .unwrap_err()
            .to_string()
            .contains("reaction_role mutations require an interactive authorized actor")
    );

    // 3. Actor lacking MANAGE_ROLES -> Err
    let execution_no_perms =
        rusty_discord_bot::discord::reaction_roles::ReactionRoleExecutionContext::new(
            Some(guild_id),
            Some(ChannelId::new(101)),
            Some(UserId::new(222222222222222222)),
            Permissions::SEND_MESSAGES, // Missing MANAGE_ROLES
            Permissions::MANAGE_ROLES,
            true,
        );
    let no_perms_res = rusty_discord_bot::discord::reaction_roles::execute_reaction_role_action(
        &ctx,
        &state,
        "reaction_roles",
        execution_no_perms,
        &op_create,
    )
    .await;
    assert!(no_perms_res.is_err());
    assert!(
        no_perms_res
            .unwrap_err()
            .to_string()
            .contains("wymaga uprawnienia Zarządzanie rolami lub Administrator")
    );

    // 4. Role hierarchy check: non-existent or higher role than bot
    let op_create_invalid_role = rusty_discord_bot::lua::ReactionRoleOperation::Create {
        message_id: "900000000000000001".to_string(),
        emoji: "🎉".to_string(),
        role_id: "999999999999999999".to_string(), // non-existent role
    };
    let invalid_role_res =
        rusty_discord_bot::discord::reaction_roles::execute_reaction_role_action(
            &ctx,
            &state,
            "reaction_roles",
            execution_valid,
            &op_create_invalid_role,
        )
        .await;
    assert!(invalid_role_res.is_err());
    assert!(
        invalid_role_res
            .unwrap_err()
            .to_string()
            .contains("role does not exist on this server")
    );

    // 5. Valid create with proper actor permissions and role hierarchy (Role 1002 is rank 10, bot is rank 50)
    let valid_res = rusty_discord_bot::discord::reaction_roles::execute_reaction_role_action(
        &ctx,
        &state,
        "reaction_roles",
        execution_valid,
        &op_create,
    )
    .await;
    assert!(valid_res.is_ok());
    let reply = valid_res.unwrap().unwrap();
    assert!(reply.contains("Skonfigurowano rolę za reakcję"));

    // Verify stored
    let stored = state
        .storage
        .get_reaction_role_binding("100", "900000000000000001", "🎉")
        .await
        .unwrap();
    assert!(stored.is_some());
    assert_eq!(stored.unwrap().role_id, "1002");

    // Verify audit log emitted
    let audit_events = state.storage.list_audit_events("100", 10).await.unwrap();
    let has_audit = audit_events
        .iter()
        .any(|e| e.event == "reaction_role_create");
    assert!(has_audit, "reaction_role_create audit log must be recorded");

    // 6. Valid remove
    let op_remove = rusty_discord_bot::lua::ReactionRoleOperation::Remove {
        message_id: "900000000000000001".to_string(),
        emoji: "🎉".to_string(),
    };
    let remove_res = rusty_discord_bot::discord::reaction_roles::execute_reaction_role_action(
        &ctx,
        &state,
        "reaction_roles",
        execution_valid,
        &op_remove,
    )
    .await;
    assert!(remove_res.is_ok());
    let remove_reply = remove_res.unwrap().unwrap();
    assert!(remove_reply.contains("Usunięto powiązanie roli"));

    // Verify deleted
    assert!(
        state
            .storage
            .get_reaction_role_binding("100", "900000000000000001", "🎉")
            .await
            .unwrap()
            .is_none()
    );

    // Verify remove audit
    let audit_events2 = state.storage.list_audit_events("100", 10).await.unwrap();
    let has_remove_audit = audit_events2
        .iter()
        .any(|e| e.event == "reaction_role_remove");
    assert!(
        has_remove_audit,
        "reaction_role_remove audit log must be recorded"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_reaction_roles_gateway_reaction_events() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state_with_port(mock.port).await;

    // Seed binding in storage: Message 900000000000000001 + emoji "🎉" -> Role 1002
    let binding = rusty_discord_bot::storage::ReactionRoleBinding {
        id: uuid::Uuid::new_v4(),
        guild_id: "100".to_string(),
        channel_id: Some("101".to_string()),
        message_id: "900000000000000001".to_string(),
        emoji_key: "🎉".to_string(),
        role_id: "1002".to_string(),
        created_by: Some("admin".to_string()),
        created_at: chrono::Utc::now(),
    };
    state
        .storage
        .upsert_reaction_role_binding(binding)
        .await
        .unwrap();

    let baseline_modifications = mock.modifying_requests_count.load(Ordering::SeqCst);

    // 1. User reacts with "🎉" -> role 1002 added!
    let reaction_user: serenity::all::Reaction = serde_json::from_value(serde_json::json!({
        "channel_id": "101",
        "message_id": "900000000000000001",
        "guild_id": "100",
        "user_id": "222222222222222222", // non-bot user
        "emoji": {
            "name": "🎉",
            "id": null
        },
        "burst": false,
        "type": 0
    }))
    .unwrap();

    rusty_discord_bot::discord::reaction_roles::handle_reaction_role_event(
        &ctx,
        &state,
        &reaction_user,
        true,
    )
    .await
    .unwrap();

    let mods_after_add = mock.modifying_requests_count.load(Ordering::SeqCst);
    assert!(
        mods_after_add > baseline_modifications,
        "Role add HTTP request must be sent to Discord"
    );

    let calls = mock.recorded_calls.lock().await.clone();
    let role_add_call = calls.iter().find(|(m, p)| {
        *m == Method::PUT && p.contains("/guilds/100/members/222222222222222222/roles/1002")
    });
    assert!(
        role_add_call.is_some(),
        "Expected PUT /guilds/100/members/222222222222222222/roles/1002 call"
    );

    // Verify audit log
    let audits = state.storage.list_audit_events("100", 10).await.unwrap();
    assert!(
        audits.iter().any(|a| a.event == "reaction_role_granted"),
        "reaction_role_granted audit log must be recorded"
    );

    // 2. User removes reaction "🎉" -> role 1002 removed!
    let baseline_before_remove = mock.modifying_requests_count.load(Ordering::SeqCst);
    rusty_discord_bot::discord::reaction_roles::handle_reaction_role_event(
        &ctx,
        &state,
        &reaction_user,
        false,
    )
    .await
    .unwrap();

    let mods_after_remove = mock.modifying_requests_count.load(Ordering::SeqCst);
    assert!(
        mods_after_remove > baseline_before_remove,
        "Role remove HTTP request must be sent to Discord"
    );

    let calls_rem = mock.recorded_calls.lock().await.clone();
    let role_rem_call = calls_rem.iter().find(|(m, p)| {
        *m == Method::DELETE && p.contains("/guilds/100/members/222222222222222222/roles/1002")
    });
    assert!(
        role_rem_call.is_some(),
        "Expected DELETE /guilds/100/members/222222222222222222/roles/1002 call"
    );

    // Verify audit log
    let audits_rem = state.storage.list_audit_events("100", 10).await.unwrap();
    assert!(
        audits_rem
            .iter()
            .any(|a| a.event == "reaction_role_revoked"),
        "reaction_role_revoked audit log must be recorded"
    );

    // 3. Bot user reacts -> ignored (prevent bot loops)
    let baseline_bot = mock.modifying_requests_count.load(Ordering::SeqCst);
    let reaction_bot: serenity::all::Reaction = serde_json::from_value(serde_json::json!({
        "channel_id": "101",
        "message_id": "900000000000000001",
        "guild_id": "100",
        "user_id": "999999999999999999", // Bot's own ID
        "emoji": {
            "name": "🎉",
            "id": null
        },
        "burst": false,
        "type": 0
    }))
    .unwrap();

    rusty_discord_bot::discord::reaction_roles::handle_reaction_role_event(
        &ctx,
        &state,
        &reaction_bot,
        true,
    )
    .await
    .unwrap();
    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_bot,
        "Bot reaction must be ignored"
    );

    // 4. Unmapped emoji -> ignored
    let baseline_unmapped = mock.modifying_requests_count.load(Ordering::SeqCst);
    let reaction_unmapped: serenity::all::Reaction = serde_json::from_value(serde_json::json!({
        "channel_id": "101",
        "message_id": "900000000000000001",
        "guild_id": "100",
        "user_id": "222222222222222222",
        "emoji": {
            "name": "⭐", // unmapped emoji
            "id": null
        },
        "burst": false,
        "type": 0
    }))
    .unwrap();

    rusty_discord_bot::discord::reaction_roles::handle_reaction_role_event(
        &ctx,
        &state,
        &reaction_unmapped,
        true,
    )
    .await
    .unwrap();
    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_unmapped,
        "Unmapped emoji reaction must be ignored"
    );

    // 5. Disabled module -> ignored
    state
        .storage
        .update_module_settings_with_audit(rusty_discord_bot::storage::UpdateModuleSettingsParams {
            guild_id: "100",
            module_id: "reaction_roles",
            enabled: false,
            config: serde_json::json!({}),
            actor_id: Some("admin"),
            audit_event: "module_disable",
            audit_data: serde_json::json!({}),
        })
        .await
        .unwrap();

    let baseline_disabled = mock.modifying_requests_count.load(Ordering::SeqCst);
    rusty_discord_bot::discord::reaction_roles::handle_reaction_role_event(
        &ctx,
        &state,
        &reaction_user,
        true,
    )
    .await
    .unwrap();
    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_disabled,
        "Reaction role event must be ignored when module is disabled"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_economy_work_and_crime_payout_cooldown_idempotency_suite() {
    let (ctx, _mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state().await;

    let guild_id = GuildId::new(100);
    let user_1 = UserId::new(111111111111111111);
    let user_2 = UserId::new(222222222222222222);

    // Initial state: 0 coins for user_1
    let initial_progress = state
        .storage
        .get_member_progress("100", "111111111111111111")
        .await
        .unwrap();
    assert_eq!(initial_progress.coins, 0);

    // 1. /economy work with roll 0.5 (deterministic halfway between min 50 and max 250 -> 150 coins)
    let execution = EconomyExecutionContext {
        guild_id: Some(guild_id),
        channel_id: Some(ChannelId::new(101)),
        actor_id: Some(user_1),
        actor_permissions: serenity::all::Permissions::empty(),
        enforce_actor_permissions: true,
        interaction_id: Some("work_int_1".to_string()),
    };

    let reply = economy::execute(
        &ctx,
        &state,
        "economy",
        execution.clone(),
        &EconomyOperation::Work { roll: Some(0.5) },
    )
    .await
    .unwrap()
    .expect("work reply expected");
    assert!(reply.contains("**150** monet") || reply.contains("**150** coins"));

    let progress_after_work = state
        .storage
        .get_member_progress("100", "111111111111111111")
        .await
        .unwrap();
    assert_eq!(progress_after_work.coins, 150);

    // 2. Retry identical interaction_id -> Idempotent! Returns AlreadyProcessed and balance unchanged
    let reply_retry = economy::execute(
        &ctx,
        &state,
        "economy",
        execution.clone(),
        &EconomyOperation::Work { roll: Some(0.5) },
    )
    .await
    .unwrap()
    .expect("retry reply expected");
    assert!(reply_retry.contains("zostało już zrealizowane"));
    let progress_after_retry = state
        .storage
        .get_member_progress("100", "111111111111111111")
        .await
        .unwrap();
    assert_eq!(
        progress_after_retry.coins, 150,
        "balance must not change on duplicate interaction retry"
    );

    // 3. New interaction_id before cooldown expires -> Cooldown rejection!
    let mut execution_cd = execution.clone();
    execution_cd.interaction_id = Some("work_int_2".to_string());
    let reply_cd = economy::execute(
        &ctx,
        &state,
        "economy",
        execution_cd,
        &EconomyOperation::Work { roll: Some(0.5) },
    )
    .await
    .unwrap()
    .expect("cooldown reply expected");
    assert!(reply_cd.contains("odpocząć"));
    let progress_after_cd = state
        .storage
        .get_member_progress("100", "111111111111111111")
        .await
        .unwrap();
    assert_eq!(
        progress_after_cd.coins, 150,
        "balance must not change on cooldown rejection"
    );

    // 4. /economy crime with roll 0.1 (success! roll <= 0.6 success rate)
    let execution_crime = EconomyExecutionContext {
        guild_id: Some(guild_id),
        channel_id: Some(ChannelId::new(101)),
        actor_id: Some(user_1),
        actor_permissions: serenity::all::Permissions::empty(),
        enforce_actor_permissions: true,
        interaction_id: Some("crime_int_1".to_string()),
    };

    let reply_crime = economy::execute(
        &ctx,
        &state,
        "economy",
        execution_crime.clone(),
        &EconomyOperation::Crime { roll: Some(0.1) },
    )
    .await
    .unwrap()
    .expect("crime reply expected");
    assert!(reply_crime.contains("Udany skok"));
    let progress_after_crime = state
        .storage
        .get_member_progress("100", "111111111111111111")
        .await
        .unwrap();
    assert!(
        progress_after_crime.coins > 150,
        "crime success must credit reward"
    );

    // 5. Retry identical crime interaction_id -> Idempotent!
    let reply_crime_retry = economy::execute(
        &ctx,
        &state,
        "economy",
        execution_crime.clone(),
        &EconomyOperation::Crime { roll: Some(0.1) },
    )
    .await
    .unwrap()
    .expect("crime retry reply expected");
    assert!(reply_crime_retry.contains("zostało już zrealizowane"));
    assert_eq!(
        state
            .storage
            .get_member_progress("100", "111111111111111111")
            .await
            .unwrap()
            .coins,
        progress_after_crime.coins
    );

    // 6. User 2 runs /economy crime with roll 0.95 (failure! roll > 0.6)
    let execution_crime_fail = EconomyExecutionContext {
        guild_id: Some(guild_id),
        channel_id: Some(ChannelId::new(101)),
        actor_id: Some(user_2),
        actor_permissions: serenity::all::Permissions::empty(),
        enforce_actor_permissions: true,
        interaction_id: Some("crime_fail_1".to_string()),
    };

    let reply_fail = economy::execute(
        &ctx,
        &state,
        "economy",
        execution_crime_fail,
        &EconomyOperation::Crime { roll: Some(0.95) },
    )
    .await
    .unwrap()
    .expect("crime fail reply expected");
    assert!(reply_fail.contains("przyłapany") || reply_fail.contains("nie powiódł"));
    assert_eq!(
        state
            .storage
            .get_member_progress("100", "222222222222222222")
            .await
            .unwrap()
            .coins,
        0,
        "failed crime must not award coins"
    );

    // 7. Untrusted module identity -> rejected
    let untrusted_err = economy::execute(
        &ctx,
        &state,
        "untrusted_module",
        execution.clone(),
        &EconomyOperation::Work { roll: Some(0.5) },
    )
    .await;
    assert!(
        untrusted_err.is_err(),
        "untrusted module identity must be rejected"
    );

    // 8. Disabled module -> rejected
    state
        .storage
        .update_module_settings_with_audit(UpdateModuleSettingsParams {
            guild_id: "100",
            module_id: "economy",
            enabled: false,
            config: serde_json::json!({}),
            actor_id: Some("admin"),
            audit_event: "module_disable",
            audit_data: serde_json::json!({}),
        })
        .await
        .unwrap();

    let disabled_err = economy::execute(
        &ctx,
        &state,
        "economy",
        execution,
        &EconomyOperation::Work { roll: Some(0.5) },
    )
    .await;
    assert!(
        disabled_err.is_err(),
        "disabled economy module must be rejected"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_economy_shop_buy_inventory_role_hierarchy_and_guild_isolation() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state_with_port(mock.port).await;

    let guild_id = GuildId::new(100);
    let user_1 = UserId::new(111111111111111111);
    let user_2 = UserId::new(222222222222222222);

    // Create store items on guild 100
    // Role 301 is lower than MockBot's role (300) in mock setup
    let item_vip = state
        .storage
        .create_store_item(
            "100",
            "VIP Pass",
            "Access to VIP zone",
            200,
            Some("1002"),
            Some(2),
        )
        .await
        .unwrap();

    let _item_booster = state
        .storage
        .create_store_item("100", "XP Booster", "Increases XP gain", 100, None, None)
        .await
        .unwrap();

    // Create store item on guild 200 (for isolation check)
    let item_foreign = state
        .storage
        .create_store_item("200", "Foreign Trophy", "Cross-guild test", 50, None, None)
        .await
        .unwrap();

    // Give user_1 500 coins on guild 100
    state
        .storage
        .adjust_member_progress("100", "111111111111111111", 0, 500, 0)
        .await
        .unwrap();

    let execution = EconomyExecutionContext {
        guild_id: Some(guild_id),
        channel_id: Some(ChannelId::new(101)),
        actor_id: Some(user_1),
        actor_permissions: serenity::all::Permissions::empty(),
        enforce_actor_permissions: true,
        interaction_id: Some("buy_int_1".to_string()),
    };

    // 1. /economy shop -> displays items for guild 100, excludes guild 200
    let shop_reply = economy::execute(
        &ctx,
        &state,
        "economy",
        execution.clone(),
        &EconomyOperation::Shop,
    )
    .await
    .unwrap()
    .expect("shop reply");
    assert!(shop_reply.contains("VIP Pass"));
    assert!(shop_reply.contains("XP Booster"));
    assert!(
        !shop_reply.contains("Foreign Trophy"),
        "cross-guild item must not be listed in shop"
    );

    // 2. /economy inventory before purchase -> empty
    let inv_empty = economy::execute(
        &ctx,
        &state,
        "economy",
        execution.clone(),
        &EconomyOperation::Inventory,
    )
    .await
    .unwrap()
    .expect("inv reply");
    assert!(inv_empty.contains("Twój ekwipunek na tym serwerze jest pusty"));

    // 3. /economy buy item:xp_booster (by slug!)
    let buy_reply = economy::execute(
        &ctx,
        &state,
        "economy",
        execution.clone(),
        &EconomyOperation::Buy {
            item: "xp_booster".to_string(),
        },
    )
    .await
    .unwrap()
    .expect("buy reply");
    assert!(buy_reply.contains("Pomyślnie zakupiono"));
    assert!(buy_reply.contains("XP Booster"));
    assert!(buy_reply.contains("**400** monet")); // 500 - 100

    let progress_after_buy1 = state
        .storage
        .get_member_progress("100", "111111111111111111")
        .await
        .unwrap();
    assert_eq!(progress_after_buy1.coins, 400);

    // 3b. Retry exact same interaction_id -> returns same result without re-debiting
    let buy_reply_retry = economy::execute(
        &ctx,
        &state,
        "economy",
        execution.clone(),
        &EconomyOperation::Buy {
            item: "xp_booster".to_string(),
        },
    )
    .await
    .unwrap()
    .expect("buy reply retry");
    assert!(buy_reply_retry.contains("Pomyślnie zakupiono"));
    assert!(buy_reply_retry.contains("XP Booster"));
    assert_eq!(
        state
            .storage
            .get_member_progress("100", "111111111111111111")
            .await
            .unwrap()
            .coins,
        400,
        "retry with same interaction_id must not debit coins again"
    );

    // 3c. Idempotency conflict: reusing interaction_id for another item must fail
    let conflict_buy = economy::execute(
        &ctx,
        &state,
        "economy",
        execution.clone(),
        &EconomyOperation::Buy {
            item: "VIP Pass".to_string(),
        },
    )
    .await;
    assert!(
        conflict_buy.is_err(),
        "reusing interaction_id for different item must fail"
    );
    assert!(
        conflict_buy
            .unwrap_err()
            .to_string()
            .contains("idempotency conflict")
    );

    // 4. /economy inventory after first buy -> contains XP Booster
    let inv_reply = economy::execute(
        &ctx,
        &state,
        "economy",
        execution.clone(),
        &EconomyOperation::Inventory,
    )
    .await
    .unwrap()
    .expect("inv reply");
    assert!(inv_reply.contains("XP Booster"));
    assert!(inv_reply.contains("completed"));

    // 5. /economy buy item:vip_pass (by name, has role_id 301 and stock 2)
    let mut execution_vip_1 = execution.clone();
    execution_vip_1.interaction_id = Some("buy_int_vip_1".to_string());

    let baseline_modifications = mock.modifying_requests_count.load(Ordering::SeqCst);
    let buy_vip = economy::execute(
        &ctx,
        &state,
        "economy",
        execution_vip_1.clone(),
        &EconomyOperation::Buy {
            item: "VIP Pass".to_string(),
        },
    )
    .await
    .unwrap()
    .expect("buy vip reply");
    assert!(buy_vip.contains("Pomyślnie zakupiono"));
    assert!(buy_vip.contains("**200** monet")); // 400 - 200

    // Role assignment request dispatched to mock
    assert!(
        mock.modifying_requests_count.load(Ordering::SeqCst) > baseline_modifications,
        "Role delivery PUT request must be dispatched"
    );

    // 5b. Retry VIP pass buy with same interaction_id -> no second HTTP PUT to Discord API
    let modifications_after_first_vip = mock.modifying_requests_count.load(Ordering::SeqCst);
    let buy_vip_retry = economy::execute(
        &ctx,
        &state,
        "economy",
        execution_vip_1.clone(),
        &EconomyOperation::Buy {
            item: "VIP Pass".to_string(),
        },
    )
    .await
    .unwrap()
    .expect("buy vip retry");
    assert!(buy_vip_retry.contains("Pomyślnie zakupiono"));
    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        modifications_after_first_vip,
        "Role delivery PUT request must not be dispatched again on retry"
    );
    assert_eq!(
        state
            .storage
            .get_member_progress("100", "111111111111111111")
            .await
            .unwrap()
            .coins,
        200,
        "Coins must not be debited on retry"
    );

    // Stock decremented to 1
    let items_now = state.storage.list_store_items("100").await.unwrap();
    let vip_now = items_now.into_iter().find(|i| i.id == item_vip.id).unwrap();
    assert_eq!(vip_now.stock, Some(1));

    // 6. Cross-guild buy attempt -> rejected!
    let mut execution_foreign = execution.clone();
    execution_foreign.interaction_id = Some("buy_foreign_1".to_string());
    let cross_guild_buy = economy::execute(
        &ctx,
        &state,
        "economy",
        execution_foreign,
        &EconomyOperation::Buy {
            item: item_foreign.id.to_string(),
        },
    )
    .await;
    assert!(
        cross_guild_buy.is_err(),
        "cross-guild item purchase must fail"
    );
    assert_eq!(
        state
            .storage
            .get_member_progress("100", "111111111111111111")
            .await
            .unwrap()
            .coins,
        200,
        "failed purchase must not debit balance"
    );

    // 7. Insufficient coins -> rejected!
    let execution_u2 = EconomyExecutionContext {
        guild_id: Some(guild_id),
        channel_id: Some(ChannelId::new(101)),
        actor_id: Some(user_2),
        actor_permissions: serenity::all::Permissions::empty(),
        enforce_actor_permissions: true,
        interaction_id: Some("u2_buy_1".to_string()),
    };
    let u2_buy = economy::execute(
        &ctx,
        &state,
        "economy",
        execution_u2,
        &EconomyOperation::Buy {
            item: "XP Booster".to_string(),
        },
    )
    .await;
    assert!(u2_buy.is_err(), "insufficient funds must be rejected");

    // 8. Stock depletion: buy remaining VIP Pass
    let mut execution_vip_2 = execution.clone();
    execution_vip_2.interaction_id = Some("buy_int_vip_2".to_string());
    let buy_vip_2 = economy::execute(
        &ctx,
        &state,
        "economy",
        execution_vip_2,
        &EconomyOperation::Buy {
            item: "vip_pass".to_string(),
        },
    )
    .await
    .unwrap()
    .expect("buy vip 2");
    assert!(buy_vip_2.contains("Pomyślnie zakupiono"));
    assert_eq!(
        state
            .storage
            .get_member_progress("100", "111111111111111111")
            .await
            .unwrap()
            .coins,
        0 // 200 - 200
    );

    // Stock is now 0. Give coins again to attempt buying out of stock item
    state
        .storage
        .adjust_member_progress("100", "111111111111111111", 0, 500, 0)
        .await
        .unwrap();

    let buy_out_of_stock = economy::execute(
        &ctx,
        &state,
        "economy",
        execution.clone(),
        &EconomyOperation::Buy {
            item: "vip_pass".to_string(),
        },
    )
    .await;
    assert!(
        buy_out_of_stock.is_err(),
        "out of stock item purchase must be rejected"
    );
    assert_eq!(
        state
            .storage
            .get_member_progress("100", "111111111111111111")
            .await
            .unwrap()
            .coins,
        500,
        "rejected out of stock purchase must not debit balance"
    );

    // 10. Issue #290 Defense-in-depth: Item with privileged role (AdminRole 1001) cannot deliver role
    let item_admin_priv = state
        .storage
        .create_store_item(
            "100",
            "Admin Badge",
            "Bypassed admin role item",
            100,
            Some("1001"),
            Some(1),
        )
        .await
        .unwrap();

    let baseline_before_admin_buy = mock.modifying_requests_count.load(Ordering::SeqCst);
    let mut execution_admin_buy = execution.clone();
    execution_admin_buy.interaction_id = Some("buy_admin_exploit_1".to_string());
    let buy_admin = economy::execute(
        &ctx,
        &state,
        "economy",
        execution_admin_buy,
        &EconomyOperation::Buy {
            item: item_admin_priv.id.to_string(),
        },
    )
    .await
    .unwrap()
    .expect("buy admin reply");

    assert!(buy_admin.contains("Nie udało się nadać roli"));

    let purchases_user = state
        .storage
        .list_store_purchases("100", Some("111111111111111111"))
        .await
        .unwrap();
    let admin_purchase = purchases_user
        .iter()
        .find(|p| p.item_id == item_admin_priv.id)
        .expect("purchase record must exist");
    assert_eq!(admin_purchase.fulfillment_status, "failed");
    assert!(
        admin_purchase
            .failure_reason
            .as_deref()
            .unwrap_or("")
            .contains("uprawnienia administracyjne")
    );
    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_before_admin_buy,
        "Privileged role must not be dispatched via PUT"
    );

    shard_manager.shutdown_all().await;
}
