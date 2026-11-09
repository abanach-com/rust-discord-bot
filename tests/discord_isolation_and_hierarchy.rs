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
    all::{
        ChannelId, GatewayIntents, GuildId, Message, MessageId, MessageUpdateEvent, Ready, UserId,
    },
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

    // Thread members queries: GET /api/v10/channels/{channel_id}/thread-members
    if method == Method::GET && path.contains("/thread-members") {
        return Json(json!([
            {
                "id": "105",
                "user_id": "999999999999999999", // Bot only
                "join_timestamp": "2026-01-01T00:00:00Z",
                "flags": 0
            }
        ]))
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

            "103" => Json(json!({
                "id": "103",
                "guild_id": "100",
                "type": 0, // GuildText
                "name": "hidden-private",
                "position": 2,
                "permission_overwrites": [
                    {
                        "id": "100", // @everyone role
                        "type": 0,
                        "allow": "0",
                        "deny": "1024" // VIEW_CHANNEL
                    }
                ]
            }))
            .into_response(),

            "104" => Json(json!({
                "id": "104",
                "guild_id": "100",
                "type": 0, // GuildText
                "name": "visible-no-send",
                "position": 3,
                "permission_overwrites": [
                    {
                        "id": "333333333333333333", // Member
                        "type": 1,
                        "allow": "1024", // VIEW_CHANNEL
                        "deny": "2048" // SEND_MESSAGES
                    }
                ]
            }))
            .into_response(),

            "105" => Json(json!({
                "id": "105",
                "guild_id": "100",
                "type": 12, // PrivateThread
                "name": "private-thread",
                "position": 4,
                "parent_id": "101",
                "permission_overwrites": []
            }))
            .into_response(),

            "106" => Json(json!({
                "id": "106",
                "guild_id": "100",
                "type": 11, // PublicThread
                "name": "public-thread-no-thread-send",
                "position": 5,
                "parent_id": "107",
                "permission_overwrites": []
            }))
            .into_response(),

            "107" => Json(json!({
                "id": "107",
                "guild_id": "100",
                "type": 0, // GuildText
                "name": "thread-parent",
                "position": 6,
                "permission_overwrites": [
                    {
                        "id": "333333333333333333", // Member
                        "type": 1,
                        "allow": "1024", // VIEW_CHANNEL
                        "deny": "274877906944" // SEND_MESSAGES_IN_THREADS = 1 << 38
                    }
                ]
            }))
            .into_response(),

            "108" => Json(json!({
                "id": "108",
                "guild_id": "100",
                "type": 0, // GuildText
                "name": "visible-no-embed",
                "position": 7,
                "permission_overwrites": [
                    {
                        "id": "333333333333333333", // Member
                        "type": 1,
                        "allow": "3072", // VIEW_CHANNEL | SEND_MESSAGES
                        "deny": "16384" // EMBED_LINKS
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

    // Modify Channel: PATCH /api/v10/channels/{channel_id}
    if method == Method::PATCH
        && path.starts_with("/api/v10/channels/")
        && !path.contains("/permissions/")
    {
        let channel_id = path
            .trim_start_matches("/api/v10/channels/")
            .split('/')
            .next()
            .unwrap_or("101");
        let name = serde_json::from_str::<serde_json::Value>(&body_str)
            .ok()
            .and_then(|v| {
                v.get("name")
                    .and_then(|n| n.as_str().map(|s| s.to_string()))
            })
            .unwrap_or_else(|| "general".to_string());
        return (
            StatusCode::OK,
            Json(json!({
                "id": channel_id,
                "guild_id": "100",
                "type": 0,
                "name": name,
                "position": 0
            })),
        )
            .into_response();
    }

    // Permission Overwrites: PUT/DELETE /api/v10/channels/{channel_id}/permissions/{overwrite_id}
    if (method == Method::PUT || method == Method::DELETE)
        && path.starts_with("/api/v10/channels/")
        && path.contains("/permissions/")
    {
        return StatusCode::NO_CONTENT.into_response();
    }

    // Send Message: POST /api/v10/channels/{channel_id}/messages

    if method == Method::POST && path.contains("/messages") {
        let count = mock.modifying_requests_count.load(Ordering::SeqCst);
        let msg_id = format!("900000000000000{:03}", count);
        return (
            StatusCode::OK,
            Json(json!({

                "id": msg_id,

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
                    "id": "100",
                    "name": "@everyone",
                    "position": 0,
                    "permissions": "274877926400", // VIEW_CHANNEL | SEND_MESSAGES | EMBED_LINKS | SEND_MESSAGES_IN_THREADS
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
                },
                {
                    "id": "1003",
                    "name": "SubordinateAdminRole",
                    "position": 25,
                    "permissions": "8", // Administrator, but position 25 < bot position 50
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
                    "id": "1004",
                    "name": "ManagerRole",
                    "position": 30,
                    "permissions": "32", // MANAGE_GUILD
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
                    "id": "1005",
                    "name": "MemberRole2",
                    "position": 11,
                    "permissions": "0",
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
                    "id": "1006",
                    "name": "MemberRole3",
                    "position": 12,
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

    // Member computed permissions query: GET /guilds/100/permissions/member/{user_id}
    if method == Method::GET && path.contains("/permissions/member/") {
        let user_id = path.split("/permissions/member/").nth(1).unwrap_or("");
        let perms = match user_id {
            "111111111111111111" => "8",  // ADMINISTRATOR
            "333333333333333333" => "32", // MANAGE_GUILD
            _ => "0",
        };
        return Json(json!({
            "permissions": perms
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
                "roles": [],
                "joined_at": "2026-10-31T06:59:00Z",
                "deaf": false,
                "mute": false,
                "flags": 0,
                "pending": false
            })).into_response(),

            // Member with pre-existing role (Issue #286)
            "555555555555555555" => Json(json!({
                "user": { "id": "555555555555555555", "username": "PreExistingMember", "avatar": null, "bot": false },
                "roles": ["1002"],
                "joined_at": "2026-01-01T00:00:00Z",
                "deaf": false,
                "mute": false,
                "flags": 0,
                "pending": false
            })).into_response(),

            // Member for shared multi-binding testing (Issue #286)
            "777777777777777777" => Json(json!({
                "user": { "id": "777777777777777777", "username": "MultiBindingMember", "avatar": null, "bot": false },
                "roles": [],
                "joined_at": "2026-01-01T00:00:00Z",
                "deaf": false,
                "mute": false,
                "flags": 0,
                "pending": false
            })).into_response(),

            // Guild Manager member with MANAGE_GUILD
            "333333333333333333" => Json(json!({
                "user": { "id": "333333333333333333", "username": "GuildManager", "avatar": null, "bot": false },
                "roles": ["1004"],
                "joined_at": "2026-01-01T00:00:00Z",
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
    let canary_goodbye = "SENSITIVE_CANARY_GOODBYE_SECRET_123";
    let goodbye_actions = vec![
        LuaAction::SendMessage {
            channel_id: Some("999".to_string()),
            content: canary_goodbye.to_string(),
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

    // Issue #357: action_denied must record action_type without leaking sensitive content into audit_events
    let denied_events: Vec<_> = audit_events_after
        .iter()
        .filter(|e| e.event == "action_denied")
        .collect();
    assert!(!denied_events.is_empty(), "action_denied must be logged");
    for entry in &denied_events {
        let entry_str = serde_json::to_string(&entry.data).unwrap();
        assert!(
            !entry_str.contains(canary_goodbye),
            "Sensitive message content must NEVER be saved in audit_events: {entry_str}"
        );
        assert_eq!(
            entry.data.get("action_type").and_then(|v| v.as_str()),
            Some("send_message")
        );
    }

    // 2b. Direct message failure test: canary content must NOT be recorded in audit_events (Issue #357)
    let canary_dm = "SENSITIVE_CANARY_DM_SECRET_456";
    let dm_actions = vec![LuaAction::SendDirectMessage {
        user_id: "000000000000000000".to_string(), // Invalid user / DM fails
        content: canary_dm.to_string(),
    }];
    let res_dm = execute_event_actions(
        &ctx,
        &state,
        "welcome",
        Some(guild_id),
        None,
        Some(UserId::new(123456)),
        &dm_actions,
    )
    .await;
    assert!(res_dm.is_ok());

    let audit_events_dm = state.storage.list_audit_events("100", 20).await.unwrap();
    for entry in audit_events_dm
        .iter()
        .filter(|e| e.event == "action_denied")
    {
        let entry_str = serde_json::to_string(&entry.data).unwrap();
        assert!(
            !entry_str.contains(canary_dm),
            "Sensitive DM content must NEVER be saved in audit_events: {entry_str}"
        );
    }

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
async fn test_reaction_roles_provenance_and_shared_grants_lifecycle_suite() {
    use chrono::Utc;
    use rusty_discord_bot::storage::ReactionRoleBinding;
    use uuid::Uuid;

    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state().await;

    // Enable reaction_roles module
    state
        .storage
        .update_module_settings_with_audit(rusty_discord_bot::storage::UpdateModuleSettingsParams {
            guild_id: "100",
            module_id: "reaction_roles",
            enabled: true,
            config: serde_json::json!({}),
            actor_id: Some("admin"),
            audit_event: "module_enable",
            audit_data: serde_json::json!({}),
        })
        .await
        .unwrap();

    // Create Binding 1: (msg 900000000000000001, 🎉) -> Role 1002
    let _b1 = state
        .storage
        .upsert_reaction_role_binding(ReactionRoleBinding {
            id: Uuid::new_v4(),
            guild_id: "100".to_string(),
            channel_id: Some("101".to_string()),
            message_id: "900000000000000001".to_string(),
            emoji_key: "🎉".to_string(),
            role_id: "1002".to_string(),
            created_by: Some("admin".to_string()),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    // Create Binding 2: (msg 900000000000000002, ✅) -> Same Role 1002
    let _b2 = state
        .storage
        .upsert_reaction_role_binding(ReactionRoleBinding {
            id: Uuid::new_v4(),
            guild_id: "100".to_string(),
            channel_id: Some("101".to_string()),
            message_id: "900000000000000002".to_string(),
            emoji_key: "✅".to_string(),
            role_id: "1002".to_string(),
            created_by: Some("admin".to_string()),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    // =========================================================================
    // CASE A: User with pre-existing role granted by moderator (Issue #286)
    // User 555555555555555555 already holds role 1002 in mock guild member state
    // =========================================================================
    let reaction_preexisting_add: serenity::all::Reaction =
        serde_json::from_value(serde_json::json!({
            "channel_id": "101",
            "message_id": "900000000000000001",
            "guild_id": "100",
            "user_id": "555555555555555555",
            "emoji": { "name": "🎉", "id": null },
            "burst": false,
            "type": 0
        }))
        .unwrap();

    let baseline_calls = mock.recorded_calls.lock().await.len();
    rusty_discord_bot::discord::reaction_roles::handle_reaction_role_event(
        &ctx,
        &state,
        &reaction_preexisting_add,
        true,
    )
    .await
    .unwrap();

    // Verify: No PUT sent because user already has role 1002
    let calls_after_add = mock.recorded_calls.lock().await.clone();
    let put_preexisting = calls_after_add[baseline_calls..].iter().any(|(m, p)| {
        *m == Method::PUT && p.contains("/guilds/100/members/555555555555555555/roles/1002")
    });
    assert!(
        !put_preexisting,
        "Redundant PUT must not be sent for member with pre-existing role"
    );

    // Ledger must record the grant with granted_by_bot = false
    let grants = state
        .storage
        .list_active_reaction_role_grants_for_user_role("100", "555555555555555555", "1002")
        .await
        .unwrap();
    assert_eq!(grants.len(), 1);
    assert!(
        !grants[0].granted_by_bot,
        "Pre-existing role grant must be marked granted_by_bot = false"
    );

    // Now user removes the reaction:
    let reaction_preexisting_remove: serenity::all::Reaction =
        serde_json::from_value(serde_json::json!({
            "channel_id": "101",
            "message_id": "900000000000000001",
            "guild_id": "100",
            "user_id": "555555555555555555",
            "emoji": { "name": "🎉", "id": null },
            "burst": false,
            "type": 0
        }))
        .unwrap();

    let baseline_before_remove = mock.recorded_calls.lock().await.len();
    rusty_discord_bot::discord::reaction_roles::handle_reaction_role_event(
        &ctx,
        &state,
        &reaction_preexisting_remove,
        false,
    )
    .await
    .unwrap();

    // CRITICAL ASSERTION: DELETE must NOT be sent to Discord!
    let calls_after_remove = mock.recorded_calls.lock().await.clone();
    let delete_sent = calls_after_remove[baseline_before_remove..]
        .iter()
        .any(|(m, p)| {
            *m == Method::DELETE && p.contains("/guilds/100/members/555555555555555555/roles/1002")
        });
    assert!(
        !delete_sent,
        "CRITICAL: remove_member_role must NOT be sent for pre-existing role granted by moderator"
    );

    // Verify audit log records preservation
    let audits = state.storage.list_audit_events("100", 10).await.unwrap();
    let preserved_audit = audits.iter().find(|a| {
        a.event == "reaction_role_preserved" && a.actor_id.as_deref() == Some("555555555555555555")
    });
    assert!(
        preserved_audit.is_some(),
        "Audit log must record reaction_role_preserved for pre-existing role"
    );
    assert_eq!(
        preserved_audit.unwrap().data["reason"],
        "pre_existing_role_preserved"
    );

    // =========================================================================
    // CASE B: Shared Multi-Binding Preservation (Issue #286)
    // User 777777777777777777 reacts to B1 (msg 1), then B2 (msg 2)
    // =========================================================================
    let r_user_b1_add: serenity::all::Reaction = serde_json::from_value(serde_json::json!({
        "channel_id": "101",
        "message_id": "900000000000000001",
        "guild_id": "100",
        "user_id": "777777777777777777",
        "emoji": { "name": "🎉", "id": null },
        "burst": false,
        "type": 0
    }))
    .unwrap();

    let r_user_b2_add: serenity::all::Reaction = serde_json::from_value(serde_json::json!({
        "channel_id": "101",
        "message_id": "900000000000000002",
        "guild_id": "100",
        "user_id": "777777777777777777",
        "emoji": { "name": "✅", "id": null },
        "burst": false,
        "type": 0
    }))
    .unwrap();

    let baseline_multi = mock.recorded_calls.lock().await.len();
    rusty_discord_bot::discord::reaction_roles::handle_reaction_role_event(
        &ctx,
        &state,
        &r_user_b1_add,
        true,
    )
    .await
    .unwrap();

    // Verify PUT sent for B1
    let calls_multi_add = mock.recorded_calls.lock().await.clone();
    let put_b1 = calls_multi_add[baseline_multi..].iter().any(|(m, p)| {
        *m == Method::PUT && p.contains("/guilds/100/members/777777777777777777/roles/1002")
    });
    assert!(put_b1, "PUT must be sent for initial bot grant under B1");

    // React under B2
    rusty_discord_bot::discord::reaction_roles::handle_reaction_role_event(
        &ctx,
        &state,
        &r_user_b2_add,
        true,
    )
    .await
    .unwrap();

    // Both B1 and B2 must be recorded in ledger as active bot grants
    let grants_multi = state
        .storage
        .list_active_reaction_role_grants_for_user_role("100", "777777777777777777", "1002")
        .await
        .unwrap();
    assert_eq!(
        grants_multi.len(),
        2,
        "Must hold two distinct active grants for role 1002"
    );
    assert!(grants_multi.iter().all(|g| g.granted_by_bot));

    // Now user UNREACTS under B1 (🎉 on msg 1):
    let r_user_b1_rem: serenity::all::Reaction = serde_json::from_value(serde_json::json!({
        "channel_id": "101",
        "message_id": "900000000000000001",
        "guild_id": "100",
        "user_id": "777777777777777777",
        "emoji": { "name": "🎉", "id": null },
        "burst": false,
        "type": 0
    }))
    .unwrap();

    let baseline_b1_rem = mock.recorded_calls.lock().await.len();
    rusty_discord_bot::discord::reaction_roles::handle_reaction_role_event(
        &ctx,
        &state,
        &r_user_b1_rem,
        false,
    )
    .await
    .unwrap();

    // CRITICAL ASSERTION: B2 is still active, so DELETE must NOT be sent!
    let calls_b1_rem = mock.recorded_calls.lock().await.clone();
    let delete_b1_rem = calls_b1_rem[baseline_b1_rem..].iter().any(|(m, p)| {
        *m == Method::DELETE && p.contains("/guilds/100/members/777777777777777777/roles/1002")
    });
    assert!(
        !delete_b1_rem,
        "CRITICAL: Unreacting B1 must NOT remove role because B2 is still active"
    );

    // Audit log confirms preservation due to other active binding
    let audits_b1 = state.storage.list_audit_events("100", 10).await.unwrap();
    let preserved_b1 = audits_b1.iter().find(|a| {
        a.event == "reaction_role_preserved" && a.actor_id.as_deref() == Some("777777777777777777")
    });
    assert!(preserved_b1.is_some());
    assert_eq!(
        preserved_b1.unwrap().data["reason"],
        "other_active_binding_preserved"
    );

    // Finally, user UNREACTS under B2 (✅ on msg 2):
    let r_user_b2_rem: serenity::all::Reaction = serde_json::from_value(serde_json::json!({
        "channel_id": "101",
        "message_id": "900000000000000002",
        "guild_id": "100",
        "user_id": "777777777777777777",
        "emoji": { "name": "✅", "id": null },
        "burst": false,
        "type": 0
    }))
    .unwrap();

    let baseline_b2_rem = mock.recorded_calls.lock().await.len();
    rusty_discord_bot::discord::reaction_roles::handle_reaction_role_event(
        &ctx,
        &state,
        &r_user_b2_rem,
        false,
    )
    .await
    .unwrap();

    // Now all sources are exhausted: DELETE must be sent!
    let calls_b2_rem = mock.recorded_calls.lock().await.clone();
    let delete_b2_rem = calls_b2_rem[baseline_b2_rem..].iter().any(|(m, p)| {
        *m == Method::DELETE && p.contains("/guilds/100/members/777777777777777777/roles/1002")
    });
    assert!(
        delete_b2_rem,
        "DELETE role must be sent once the final reaction role binding is removed"
    );

    // =========================================================================
    // CASE C: Fail-closed on member resolution error (Issue #301)
    // Unknown member 888888888888888888 returns error from get_member
    // Must NOT record grant as granted_by_bot=true and must NOT mutate roles
    // =========================================================================
    let reaction_unknown_member: serenity::all::Reaction =
        serde_json::from_value(serde_json::json!({
            "channel_id": "101",
            "message_id": "900000000000000001",
            "guild_id": "100",
            "user_id": "888888888888888888",
            "emoji": { "name": "🎉", "id": null },
            "burst": false,
            "type": 0
        }))
        .unwrap();

    let baseline_unknown = mock.recorded_calls.lock().await.len();
    let res_unknown = rusty_discord_bot::discord::reaction_roles::handle_reaction_role_event(
        &ctx,
        &state,
        &reaction_unknown_member,
        true,
    )
    .await;

    assert!(
        res_unknown.is_err(),
        "Must fail closed when member state cannot be determined"
    );
    assert!(
        res_unknown
            .unwrap_err()
            .to_string()
            .contains("cannot determine reaction role member state")
    );

    // Verify: No grants recorded for this unknown member
    let grants_unknown = state
        .storage
        .list_active_reaction_role_grants_for_user_role("100", "888888888888888888", "1002")
        .await
        .unwrap();
    assert!(
        grants_unknown.is_empty(),
        "No grant must be recorded when member resolution fails"
    );

    // Verify: No PUT sent to Discord for role 1002
    let calls_unknown = mock.recorded_calls.lock().await.clone();
    let put_unknown = calls_unknown[baseline_unknown..].iter().any(|(m, p)| {
        *m == Method::PUT && p.contains("/guilds/100/members/888888888888888888/roles/1002")
    });
    assert!(
        !put_unknown,
        "No PUT role must be sent when member state resolution fails"
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

#[tokio::test]
async fn test_event_autorole_privileged_defense_in_depth() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state_with_port(mock.port).await;

    let guild_id = GuildId::new(100);
    let target_user = "222222222222222222";

    // 1. Attempt to execute event-based AddRole with privileged role (1003 has ADMINISTRATOR, position 25 < bot position 50)
    // Even if configured in database, runtime defense-in-depth must reject granting privileged role
    let privileged_action = vec![LuaAction::AddRole {
        user_id: target_user.to_string(),
        role_id: "1003".to_string(),
        reason: Some("Compromised welcome autorole".to_string()),
    }];

    let baseline_requests = mock
        .modifying_requests_count
        .load(std::sync::atomic::Ordering::SeqCst);

    let res_priv = rusty_discord_bot::discord::executor::execute_event_actions(
        &ctx,
        &state,
        "welcome",
        Some(guild_id),
        None,
        None,
        &privileged_action,
    )
    .await;
    assert!(
        res_priv.is_ok(),
        "execute_event_actions handles action denial gracefully"
    );

    // Verify modifying requests were blocked (0 PUT requests sent to Discord)
    assert_eq!(
        mock.modifying_requests_count
            .load(std::sync::atomic::Ordering::SeqCst),
        baseline_requests,
        "No PUT request must be dispatched for privileged role"
    );

    // Verify structured audit log entry 'action_denied' recorded the refusal
    let audit_logs = state.storage.list_audit_events("100", 10).await.unwrap();
    let denied_entry = audit_logs
        .iter()
        .find(|a| a.event == "action_denied")
        .expect("action_denied audit event must be logged");
    let audit_data_str = serde_json::to_string(&denied_entry.data).unwrap();
    assert!(
        audit_data_str.contains("event-based automation cannot grant privileged role"),
        "Audit log must describe privileged role refusal: {audit_data_str}"
    );

    // 2. Event-based AddRole with normal unprivileged role (1002 has 0 permissions) -> succeeds
    let unprivileged_action = vec![LuaAction::AddRole {
        user_id: target_user.to_string(),
        role_id: "1002".to_string(),
        reason: Some("Legitimate welcome autorole".to_string()),
    }];

    let res_unpriv = rusty_discord_bot::discord::executor::execute_event_actions(
        &ctx,
        &state,
        "welcome",
        Some(guild_id),
        None,
        None,
        &unprivileged_action,
    )
    .await;

    assert!(
        res_unpriv.is_ok(),
        "Event-driven AddRole must succeed on unprivileged role: {:?}",
        res_unpriv.err()
    );
    assert!(
        mock.modifying_requests_count
            .load(std::sync::atomic::Ordering::SeqCst)
            > baseline_requests,
        "Role delivery PUT request must be dispatched for unprivileged role"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_scheduler_destination_channel_authorization_suite() {
    use serenity::all::Permissions;

    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state_with_port(mock.port).await;

    let guild_id = GuildId::new(100);
    let manager_id = UserId::new(333333333333333333); // Member with MANAGE_GUILD

    let make_exec_ctx = |actor_perms: Permissions| {
        rusty_discord_bot::discord::scheduler::SchedulerExecutionContext::new(
            Some(guild_id),
            Some(ChannelId::new(101)),
            Some(manager_id),
            actor_perms,
            true,
        )
    };

    let baseline_requests = mock.modifying_requests_count.load(Ordering::SeqCst);

    // 1. Manager with MANAGE_GUILD attempts to schedule message in channel 103 (hidden-private with deny VIEW_CHANNEL)
    let op_hidden_channel = rusty_discord_bot::lua::SchedulerOperation::Create {
        when: "15m".to_string(),
        content: "Exploit attempt in hidden channel".to_string(),
        channel_id: Some("103".to_string()),
        repeat: Some("1h".to_string()),
        repeat_count: None,
        mention_creator: false,
        max_jobs: 10,
        minimum_delay_seconds: 60,
        maximum_delay_seconds: 86400,
    };

    let res_hidden = rusty_discord_bot::discord::scheduler::execute(
        &ctx,
        &state,
        "scheduler",
        make_exec_ctx(Permissions::MANAGE_GUILD),
        &op_hidden_channel,
    )
    .await;

    assert!(
        res_hidden.is_err(),
        "Must reject scheduling to channel without VIEW_CHANNEL"
    );
    let err_str = res_hidden.unwrap_err().to_string();
    assert!(
        err_str.contains("VIEW_CHANNEL"),
        "Error must mention VIEW_CHANNEL: {err_str}"
    );

    // Verify 0 jobs stored in scheduler
    let jobs_guild = state
        .scheduler
        .list_jobs("100", "333333333333333333", true, 50)
        .await
        .unwrap();
    assert!(
        jobs_guild.is_empty(),
        "0 jobs must be created on authorization denial"
    );

    // 2. Manager attempts to schedule in channel 104 (visible, but deny SEND_MESSAGES)
    let op_no_send = rusty_discord_bot::lua::SchedulerOperation::Create {
        when: "15m".to_string(),
        content: "Exploit attempt in read-only channel".to_string(),
        channel_id: Some("104".to_string()),
        repeat: Some("1h".to_string()),
        repeat_count: None,
        mention_creator: false,
        max_jobs: 10,
        minimum_delay_seconds: 60,
        maximum_delay_seconds: 86400,
    };

    let res_no_send = rusty_discord_bot::discord::scheduler::execute(
        &ctx,
        &state,
        "scheduler",
        make_exec_ctx(Permissions::MANAGE_GUILD),
        &op_no_send,
    )
    .await;

    assert!(
        res_no_send.is_err(),
        "Must reject scheduling to channel without SEND_MESSAGES"
    );
    let err_no_send_str = res_no_send.unwrap_err().to_string();
    assert!(
        err_no_send_str.contains("SEND_MESSAGES"),
        "Error must mention SEND_MESSAGES: {err_no_send_str}"
    );
    assert!(
        state
            .scheduler
            .list_jobs("100", "333333333333333333", true, 50)
            .await
            .unwrap()
            .is_empty()
    );

    // 3. Manager attempts to schedule in channel 201 (cross-guild channel)
    let op_cross_guild = rusty_discord_bot::lua::SchedulerOperation::Create {
        when: "15m".to_string(),
        content: "Exploit attempt cross-guild".to_string(),
        channel_id: Some("201".to_string()),
        repeat: None,
        repeat_count: None,
        mention_creator: false,
        max_jobs: 10,
        minimum_delay_seconds: 60,
        maximum_delay_seconds: 86400,
    };

    let res_cross = rusty_discord_bot::discord::scheduler::execute(
        &ctx,
        &state,
        "scheduler",
        make_exec_ctx(Permissions::MANAGE_GUILD),
        &op_cross_guild,
    )
    .await;

    assert!(
        res_cross.is_err(),
        "Must reject scheduling to cross-guild channel"
    );
    assert!(
        res_cross
            .unwrap_err()
            .to_string()
            .contains("different server")
    );
    assert!(
        state
            .scheduler
            .list_jobs("100", "333333333333333333", true, 50)
            .await
            .unwrap()
            .is_empty()
    );

    // 4. Manager attempts to schedule in channel 105 (private thread where manager is not a member)
    let op_private_thread = rusty_discord_bot::lua::SchedulerOperation::Create {
        when: "15m".to_string(),
        content: "Exploit attempt in private thread".to_string(),
        channel_id: Some("105".to_string()),
        repeat: None,
        repeat_count: None,
        mention_creator: false,
        max_jobs: 10,
        minimum_delay_seconds: 60,
        maximum_delay_seconds: 86400,
    };

    let res_thread = rusty_discord_bot::discord::scheduler::execute(
        &ctx,
        &state,
        "scheduler",
        make_exec_ctx(Permissions::MANAGE_GUILD),
        &op_private_thread,
    )
    .await;

    assert!(
        res_thread.is_err(),
        "Must reject scheduling in private thread without membership"
    );
    assert!(
        res_thread
            .unwrap_err()
            .to_string()
            .contains("private thread")
    );
    assert!(
        state
            .scheduler
            .list_jobs("100", "333333333333333333", true, 50)
            .await
            .unwrap()
            .is_empty()
    );

    // 5. Manager attempts to schedule in channel 106 (thread where parent denies SEND_MESSAGES_IN_THREADS)
    let op_thread_deny = rusty_discord_bot::lua::SchedulerOperation::Create {
        when: "15m".to_string(),
        content: "Exploit attempt in thread without thread send perm".to_string(),
        channel_id: Some("106".to_string()),
        repeat: None,
        repeat_count: None,
        mention_creator: false,
        max_jobs: 10,
        minimum_delay_seconds: 60,
        maximum_delay_seconds: 86400,
    };

    let res_thread_deny = rusty_discord_bot::discord::scheduler::execute(
        &ctx,
        &state,
        "scheduler",
        make_exec_ctx(Permissions::MANAGE_GUILD),
        &op_thread_deny,
    )
    .await;

    assert!(
        res_thread_deny.is_err(),
        "Must reject scheduling in thread when parent denies thread messages"
    );
    assert!(
        res_thread_deny
            .unwrap_err()
            .to_string()
            .contains("SEND_MESSAGES_IN_THREADS")
    );
    assert!(
        state
            .scheduler
            .list_jobs("100", "333333333333333333", true, 50)
            .await
            .unwrap()
            .is_empty()
    );

    // 6. Legitimate schedule in visible channel 101 with all permissions -> succeeds!
    let op_legit = rusty_discord_bot::lua::SchedulerOperation::Create {
        when: "15m".to_string(),
        content: "Legitimate scheduled message".to_string(),
        channel_id: Some("101".to_string()),
        repeat: Some("1h".to_string()),
        repeat_count: Some(5),
        mention_creator: true,
        max_jobs: 10,
        minimum_delay_seconds: 60,
        maximum_delay_seconds: 86400,
    };

    let res_legit = rusty_discord_bot::discord::scheduler::execute(
        &ctx,
        &state,
        "scheduler",
        make_exec_ctx(Permissions::MANAGE_GUILD),
        &op_legit,
    )
    .await;

    assert!(
        res_legit.is_ok(),
        "Legitimate schedule must succeed: {:?}",
        res_legit.err()
    );
    let jobs_after_legit = state
        .scheduler
        .list_jobs("100", "333333333333333333", true, 50)
        .await
        .unwrap();
    assert_eq!(jobs_after_legit.len(), 1, "Exactly 1 job must be created");

    // 7. Normal /remind in current channel without explicit channel_id -> succeeds!
    let op_remind = rusty_discord_bot::lua::SchedulerOperation::Create {
        when: "20m".to_string(),
        content: "Personal reminder in current channel".to_string(),
        channel_id: None,
        repeat: None,
        repeat_count: None,
        mention_creator: true,
        max_jobs: 10,
        minimum_delay_seconds: 60,
        maximum_delay_seconds: 86400,
    };

    let res_remind = rusty_discord_bot::discord::scheduler::execute(
        &ctx,
        &state,
        "scheduler",
        make_exec_ctx(Permissions::empty()), // Regular user command without MANAGE_GUILD
        &op_remind,
    )
    .await;

    assert!(
        res_remind.is_ok(),
        "Remind in current channel must succeed: {:?}",
        res_remind.err()
    );
    let jobs_after_remind = state
        .scheduler
        .list_jobs("100", "333333333333333333", true, 50)
        .await
        .unwrap();
    assert_eq!(jobs_after_remind.len(), 2, "2 jobs now in scheduler");

    // 8. Runtime delivery live-check (ensure_scheduled_delivery_access) fails closed if permission is revoked
    let check_revoked = rusty_discord_bot::discord::scheduler::ensure_scheduled_delivery_access(
        &ctx.http,
        guild_id,
        manager_id,
        ChannelId::new(104), // Channel where user has no SEND_MESSAGES
    )
    .await;
    assert!(
        check_revoked.is_err(),
        "Runtime delivery check must fail closed when permissions revoked"
    );

    // Verify no modifying requests were dispatched to Discord
    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_requests,
        "No PUT/POST message requests dispatched during denied scheduler creations"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_starboard_config_execution_and_persistence_suite() {
    use rusty_discord_bot::discord::starboard::{
        StarboardExecutionContext, execute_starboard_action,
    };
    use rusty_discord_bot::lua::StarboardOperation;
    use serenity::all::{ChannelId, GuildId, Permissions, UserId};

    let (ctx, _mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state().await;

    let guild_id = GuildId::new(100);
    let owner_id = UserId::new(111111111111111111);
    let member_id = UserId::new(333333333333333333);
    let valid_channel_id = "101".to_string();

    // 1. Valid configuration by Owner / Administrator
    let res = execute_starboard_action(
        &ctx,
        &state,
        "starboard",
        StarboardExecutionContext::new(
            Some(guild_id),
            Some(ChannelId::new(101)),
            Some(owner_id),
            Permissions::ADMINISTRATOR,
            true,
        ),
        &StarboardOperation::Configure {
            channel_id: valid_channel_id.clone(),
            threshold: 5,
        },
    )
    .await;

    assert!(
        res.is_ok(),
        "Starboard configure by owner should succeed: {:?}",
        res.err()
    );
    let confirmation = res.unwrap().expect("Should have confirmation message");
    assert!(confirmation.contains("<#101>") && confirmation.contains("5"));

    // Verify persistence in storage
    let settings = state
        .storage
        .get_module_settings("100", "starboard")
        .await
        .unwrap()
        .expect("Settings should exist");
    assert_eq!(settings.config["channel_id"], "101");
    assert_eq!(settings.config["threshold"], 5);

    // Verify audit log
    let audits = state.storage.list_audit_events("100", 10).await.unwrap();
    assert!(audits.iter().any(|a| a.event == "starboard_config_updated"));

    // 2. Verify Lua engine executes /starboard stats with the new ctx.config
    let scripts = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts");
    let eng = rusty_discord_bot::lua::LuaEngine::load(
        scripts,
        rusty_discord_bot::lua::LuaLimits {
            memory_bytes: 4 * 1024 * 1024,
            instruction_limit: 200_000,
            hook_granularity: 100,
        },
    )
    .expect("LuaEngine load");

    let stats_ctx = rusty_discord_bot::lua::LuaExecutionContext {
        guild_id: Some("100".to_string()),
        channel_id: "101".to_string(),
        user_id: "111111111111111111".to_string(),
        user_name: "Owner".to_string(),
        member_roles: Vec::new(),
        member_permissions: "8".to_string(),
        locale: "en-US".to_string(),
        config: settings.config.clone(),
        options: serde_json::json!({ "stats": {} }),
    };
    let actions = eng
        .execute_command("starboard", "starboard", stats_ctx)
        .await
        .expect("stats command should succeed");
    assert!(actions.iter().any(|a| matches!(
        a,
        rusty_discord_bot::lua::LuaAction::Reply { content, .. }
        if content.contains("101") && content.contains("5")
    )));

    // 3. Rejection when actor lacks MANAGE_GUILD / ADMINISTRATOR
    let res_no_perm = execute_starboard_action(
        &ctx,
        &state,
        "starboard",
        StarboardExecutionContext::new(
            Some(guild_id),
            Some(ChannelId::new(101)),
            Some(member_id),
            Permissions::empty(),
            true,
        ),
        &StarboardOperation::Configure {
            channel_id: valid_channel_id.clone(),
            threshold: 5,
        },
    )
    .await;
    assert!(
        res_no_perm.is_err(),
        "Non-admin actor without MANAGE_GUILD must be rejected"
    );
    assert!(
        res_no_perm
            .unwrap_err()
            .to_string()
            .contains("MANAGE_GUILD")
    );

    // 4. Rejection when channel belongs to foreign guild (channel 201 on guild 200)
    let res_foreign = execute_starboard_action(
        &ctx,
        &state,
        "starboard",
        StarboardExecutionContext::new(
            Some(guild_id),
            Some(ChannelId::new(101)),
            Some(owner_id),
            Permissions::ADMINISTRATOR,
            true,
        ),
        &StarboardOperation::Configure {
            channel_id: "201".to_string(),
            threshold: 5,
        },
    )
    .await;
    assert!(
        res_foreign.is_err(),
        "Foreign guild channel must be rejected"
    );
    assert!(
        res_foreign
            .unwrap_err()
            .to_string()
            .contains("innego serwera")
    );

    // 5. Rejection when channel is category (channel 401)
    let res_cat = execute_starboard_action(
        &ctx,
        &state,
        "starboard",
        StarboardExecutionContext::new(
            Some(guild_id),
            Some(ChannelId::new(101)),
            Some(owner_id),
            Permissions::ADMINISTRATOR,
            true,
        ),
        &StarboardOperation::Configure {
            channel_id: "401".to_string(),
            threshold: 5,
        },
    )
    .await;
    assert!(res_cat.is_err(), "Category channel must be rejected");
    assert!(res_cat.unwrap_err().to_string().contains("Kategoria"));

    // 6. Validation of threshold out of bounds
    let op_bad_threshold = StarboardOperation::Configure {
        channel_id: "101".to_string(),
        threshold: 0,
    };
    assert!(op_bad_threshold.validate().is_err());

    let op_bad_threshold2 = StarboardOperation::Configure {
        channel_id: "101".to_string(),
        threshold: 26,
    };
    assert!(op_bad_threshold2.validate().is_err());

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_quotes_storage_and_execution_lifecycle_suite() {
    use rusty_discord_bot::discord::quotes::{QuoteExecutionContext, execute_quote_action};
    use rusty_discord_bot::lua::QuoteOperation;
    use serenity::all::{ChannelId, GuildId, Permissions, UserId};

    let (ctx, _mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state().await;

    let guild_a = "100";
    let actor_id = UserId::new(111111111111111111);
    let author_a = "222222222222222222";
    let author_b = "333333333333333333";

    // 1. Initial random / list on empty storage returns safe empty message
    let res_empty_random = execute_quote_action(
        &ctx,
        &state,
        "quotes",
        QuoteExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(actor_id),
            Permissions::empty(),
            false,
        ),
        &QuoteOperation::Random,
    )
    .await
    .unwrap();
    assert_eq!(
        res_empty_random.as_deref(),
        Some("Brak zapisanych cytatów na tym serwerze.")
    );

    let res_empty_list = execute_quote_action(
        &ctx,
        &state,
        "quotes",
        QuoteExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(actor_id),
            Permissions::empty(),
            false,
        ),
        &QuoteOperation::List { limit: 10 },
    )
    .await
    .unwrap();
    assert_eq!(
        res_empty_list.as_deref(),
        Some("Brak zapisanych cytatów na tym serwerze.")
    );

    // 2. Add quotes to Guild A
    let res_add_1 = execute_quote_action(
        &ctx,
        &state,
        "quotes",
        QuoteExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(actor_id),
            Permissions::empty(),
            false,
        ),
        &QuoteOperation::Add {
            author_id: author_a.to_string(),
            text: "Cytat numer jeden".to_string(),
        },
    )
    .await
    .unwrap();
    let reply_1 = res_add_1.unwrap();
    assert!(reply_1.contains("Dodano cytat"));
    assert!(reply_1.contains(author_a));
    assert!(reply_1.contains("Cytat numer jeden"));

    let res_add_2 = execute_quote_action(
        &ctx,
        &state,
        "quotes",
        QuoteExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(actor_id),
            Permissions::empty(),
            false,
        ),
        &QuoteOperation::Add {
            author_id: author_b.to_string(),
            text: "Drugi cytat z gildii A".to_string(),
        },
    )
    .await
    .unwrap();
    assert!(res_add_2.unwrap().contains("Drugi cytat"));

    // Add quote to Guild B (tenant isolation test)
    let _ = execute_quote_action(
        &ctx,
        &state,
        "quotes",
        QuoteExecutionContext::new(
            Some(GuildId::new(200)),
            Some(ChannelId::new(201)),
            Some(actor_id),
            Permissions::empty(),
            false,
        ),
        &QuoteOperation::Add {
            author_id: author_b.to_string(),
            text: "Cytat z gildii B - sekretny".to_string(),
        },
    )
    .await
    .unwrap();

    // 3. Verify List in Guild A returns only Guild A quotes
    let list_a = execute_quote_action(
        &ctx,
        &state,
        "quotes",
        QuoteExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(actor_id),
            Permissions::empty(),
            false,
        ),
        &QuoteOperation::List { limit: 10 },
    )
    .await
    .unwrap()
    .unwrap();

    assert!(list_a.contains("Cytat numer jeden"));
    assert!(list_a.contains("Drugi cytat z gildii A"));
    assert!(
        !list_a.contains("Cytat z gildii B"),
        "Tenant isolation: Guild B quotes must not leak to Guild A"
    );

    // 4. Verify Random in Guild A returns one of Guild A quotes, never Guild B
    for _ in 0..10 {
        let rand_a = execute_quote_action(
            &ctx,
            &state,
            "quotes",
            QuoteExecutionContext::new(
                Some(GuildId::new(100)),
                Some(ChannelId::new(101)),
                Some(actor_id),
                Permissions::empty(),
                false,
            ),
            &QuoteOperation::Random,
        )
        .await
        .unwrap()
        .unwrap();

        assert!(
            !rand_a.contains("Cytat z gildii B"),
            "Random quote must respect guild boundary"
        );
    }

    // 5. Validation: length checks
    let op_empty = QuoteOperation::Add {
        author_id: author_a.to_string(),
        text: "   ".to_string(),
    };
    assert!(op_empty.validate().is_err());

    let op_too_long = QuoteOperation::Add {
        author_id: author_a.to_string(),
        text: "a".repeat(501),
    };
    assert!(op_too_long.validate().is_err());

    // 6. Delete quote: requires interactive actor with MANAGE_MESSAGES or ADMINISTRATOR (Issue #356)
    let quotes_in_a = state.storage.list_quotes(guild_a, 10).await.unwrap();
    assert_eq!(quotes_in_a.len(), 2);
    let to_delete_id = quotes_in_a[0].id;

    // 6a. Non-interactive event path (enforce_actor_permissions = false, no actor) must be REJECTED (Issue #356)
    let res_del_event_no_actor = execute_quote_action(
        &ctx,
        &state,
        "quotes",
        QuoteExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            None,
            Permissions::empty(),
            false, // Event path
        ),
        &QuoteOperation::Delete {
            quote_id: to_delete_id,
        },
    )
    .await;
    assert!(
        res_del_event_no_actor.is_err(),
        "Event path without actor must be rejected for Quote Delete"
    );

    // 6b. Non-interactive event path with actor present but enforce_actor_permissions = false must also be REJECTED (Issue #356)
    let res_del_event_with_actor = execute_quote_action(
        &ctx,
        &state,
        "quotes",
        QuoteExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(actor_id),
            Permissions::all(),
            false, // Event path
        ),
        &QuoteOperation::Delete {
            quote_id: to_delete_id,
        },
    )
    .await;
    assert!(
        res_del_event_with_actor.is_err(),
        "Non-interactive event path must be rejected for Quote Delete"
    );

    // Ensure quote still exists in storage after rejected attempts
    let quotes_check = state.storage.list_quotes(guild_a, 10).await.unwrap();
    assert!(quotes_check.iter().any(|q| q.id == to_delete_id));

    // 6c. Interactive path without MANAGE_MESSAGES must be rejected
    let res_del_unauthorized = execute_quote_action(
        &ctx,
        &state,
        "quotes",
        QuoteExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(actor_id),
            Permissions::empty(),
            true, // Enforce actor permissions
        ),
        &QuoteOperation::Delete {
            quote_id: to_delete_id,
        },
    )
    .await;
    assert!(
        res_del_unauthorized.is_err(),
        "Unauthorized deletion must be rejected"
    );

    // 6d. Interactive path with MANAGE_MESSAGES succeeds
    let res_del_authorized = execute_quote_action(
        &ctx,
        &state,
        "quotes",
        QuoteExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(actor_id),
            Permissions::MANAGE_MESSAGES,
            true,
        ),
        &QuoteOperation::Delete {
            quote_id: to_delete_id,
        },
    )
    .await;
    assert!(
        res_del_authorized.is_ok(),
        "Authorized deletion must succeed"
    );

    // After deletion, remaining count is 1
    let remaining_in_a = state.storage.list_quotes(guild_a, 10).await.unwrap();
    assert_eq!(remaining_in_a.len(), 1);
    assert_eq!(remaining_in_a[0].id, quotes_in_a[1].id);

    // Verify audit events recorded
    let audits = state.storage.list_audit_events(guild_a, 10).await.unwrap();
    assert!(audits.iter().any(|a| a.event == "quote_add"));
    assert!(audits.iter().any(|a| a.event == "quote_delete"));

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_birthdays_storage_and_execution_lifecycle_suite() {
    use rusty_discord_bot::discord::birthdays::{
        BirthdayExecutionContext, execute_birthday_action,
    };
    use rusty_discord_bot::lua::BirthdayOperation;
    use serenity::all::{ChannelId, GuildId, Permissions, UserId};

    let (ctx, _mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state().await;

    let guild_a = "100";
    let actor_1 = UserId::new(111111111111111111);
    let actor_2 = UserId::new(222222222222222222);
    let actor_b = UserId::new(333333333333333333);

    // 1. Initial check & upcoming on empty storage
    let res_empty_check = execute_birthday_action(
        &ctx,
        &state,
        "birthdays",
        BirthdayExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(actor_1),
            Permissions::empty(),
            false,
        ),
        &BirthdayOperation::Check { user_id: None },
    )
    .await
    .unwrap();
    assert!(res_empty_check.unwrap().contains("Nie znaleziono"));

    let res_empty_upcoming = execute_birthday_action(
        &ctx,
        &state,
        "birthdays",
        BirthdayExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(actor_1),
            Permissions::empty(),
            false,
        ),
        &BirthdayOperation::Upcoming { limit: 10 },
    )
    .await
    .unwrap();
    assert_eq!(
        res_empty_upcoming.as_deref(),
        Some("Brak nadchodzących urodzin na tym serwerze.")
    );

    // 2. Reject impossible dates
    let err_invalid_date = execute_birthday_action(
        &ctx,
        &state,
        "birthdays",
        BirthdayExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(actor_1),
            Permissions::empty(),
            false,
        ),
        &BirthdayOperation::Set { day: 31, month: 4 },
    )
    .await;
    assert!(err_invalid_date.is_err(), "April 31 must be rejected");

    let err_feb_30 = execute_birthday_action(
        &ctx,
        &state,
        "birthdays",
        BirthdayExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(actor_1),
            Permissions::empty(),
            false,
        ),
        &BirthdayOperation::Set { day: 30, month: 2 },
    )
    .await;
    assert!(err_feb_30.is_err(), "February 30 must be rejected");

    // 3. Set valid birthdays (including Feb 29)
    let res_set_1 = execute_birthday_action(
        &ctx,
        &state,
        "birthdays",
        BirthdayExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(actor_1),
            Permissions::empty(),
            false,
        ),
        &BirthdayOperation::Set { day: 15, month: 8 },
    )
    .await
    .unwrap();
    assert!(res_set_1.unwrap().contains("15.08"));

    let res_set_2 = execute_birthday_action(
        &ctx,
        &state,
        "birthdays",
        BirthdayExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(actor_2),
            Permissions::empty(),
            false,
        ),
        &BirthdayOperation::Set { day: 29, month: 2 },
    )
    .await
    .unwrap();
    assert!(res_set_2.unwrap().contains("29.02"));

    // Set birthday in Guild B for tenant isolation
    let _ = execute_birthday_action(
        &ctx,
        &state,
        "birthdays",
        BirthdayExecutionContext::new(
            Some(GuildId::new(200)),
            Some(ChannelId::new(201)),
            Some(actor_b),
            Permissions::empty(),
            false,
        ),
        &BirthdayOperation::Set { day: 25, month: 12 },
    )
    .await
    .unwrap();

    // 4. Check birthday
    let check_self = execute_birthday_action(
        &ctx,
        &state,
        "birthdays",
        BirthdayExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(actor_1),
            Permissions::empty(),
            false,
        ),
        &BirthdayOperation::Check { user_id: None },
    )
    .await
    .unwrap();
    assert!(check_self.unwrap().contains("15.08"));

    let check_other = execute_birthday_action(
        &ctx,
        &state,
        "birthdays",
        BirthdayExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(actor_1),
            Permissions::empty(),
            false,
        ),
        &BirthdayOperation::Check {
            user_id: Some(actor_2.get().to_string()),
        },
    )
    .await
    .unwrap();
    assert!(check_other.unwrap().contains("29.02"));

    // Check user from Guild B in Guild A -> not found
    let check_cross_guild = execute_birthday_action(
        &ctx,
        &state,
        "birthdays",
        BirthdayExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(actor_1),
            Permissions::empty(),
            false,
        ),
        &BirthdayOperation::Check {
            user_id: Some(actor_b.get().to_string()),
        },
    )
    .await
    .unwrap();
    assert!(check_cross_guild.unwrap().contains("Nie znaleziono"));

    // 5. Upcoming birthdays in Guild A does not include Guild B
    let upcoming_a = execute_birthday_action(
        &ctx,
        &state,
        "birthdays",
        BirthdayExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(actor_1),
            Permissions::empty(),
            false,
        ),
        &BirthdayOperation::Upcoming { limit: 10 },
    )
    .await
    .unwrap()
    .unwrap();

    assert!(upcoming_a.contains(&actor_1.get().to_string()));
    assert!(upcoming_a.contains(&actor_2.get().to_string()));
    assert!(!upcoming_a.contains(&actor_b.get().to_string()));

    // 6. Delete birthday
    let res_del = execute_birthday_action(
        &ctx,
        &state,
        "birthdays",
        BirthdayExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(actor_1),
            Permissions::empty(),
            false,
        ),
        &BirthdayOperation::Delete,
    )
    .await
    .unwrap();
    assert!(res_del.unwrap().contains("zostały usunięte"));

    let check_after_del = execute_birthday_action(
        &ctx,
        &state,
        "birthdays",
        BirthdayExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(actor_1),
            Permissions::empty(),
            false,
        ),
        &BirthdayOperation::Check { user_id: None },
    )
    .await
    .unwrap();
    assert!(check_after_del.unwrap().contains("Nie znaleziono"));

    // 7. Verify audits
    let audits = state.storage.list_audit_events(guild_a, 10).await.unwrap();
    assert!(audits.iter().any(|a| a.event == "birthday_set"));
    assert!(audits.iter().any(|a| a.event == "birthday_delete"));

    // 8. Issue #333: Leap year upcoming chronology (29.02 vs 01.03 in non-leap year)
    let leap_guild = "777000000000000099";
    let user_leap = "888000000000000001";
    let user_march = "888000000000000002";
    let user_feb28 = "888000000000000003";

    state
        .storage
        .set_birthday(rusty_discord_bot::storage::BirthdayRecord {
            guild_id: leap_guild.to_string(),
            user_id: user_leap.to_string(),
            month: 2,
            day: 29,
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    state
        .storage
        .set_birthday(rusty_discord_bot::storage::BirthdayRecord {
            guild_id: leap_guild.to_string(),
            user_id: user_march.to_string(),
            month: 3,
            day: 1,
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    state
        .storage
        .set_birthday(rusty_discord_bot::storage::BirthdayRecord {
            guild_id: leap_guild.to_string(),
            user_id: user_feb28.to_string(),
            month: 2,
            day: 28,
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    // In a non-leap year (e.g. 2026-11-08):
    // Next 28.02 is 2027-02-28
    // Next 01.03 is 2027-03-01
    // Next 29.02 is 2028-02-29
    // Ordering MUST be: 28.02, then 01.03, then 29.02.
    let frozen_2026 = chrono::NaiveDate::from_ymd_opt(2026, 11, 8).unwrap();
    let up_at_2026 = state
        .storage
        .list_upcoming_birthdays_at(leap_guild, 3, frozen_2026)
        .await
        .unwrap();
    assert_eq!(up_at_2026.len(), 3);
    assert_eq!(up_at_2026[0].user_id, user_feb28);
    assert_eq!(up_at_2026[1].user_id, user_march);
    assert_eq!(up_at_2026[2].user_id, user_leap);

    // Limit=1 must return user_feb28, limit=2 must return user_feb28 and user_march
    let up_limit1 = state
        .storage
        .list_upcoming_birthdays_at(leap_guild, 1, frozen_2026)
        .await
        .unwrap();
    assert_eq!(up_limit1.len(), 1);
    assert_eq!(up_limit1[0].user_id, user_feb28);

    // On 2028-02-28 (a leap year):
    // Next 28.02 is 2028-02-28
    // Next 29.02 is 2028-02-29
    // Next 01.03 is 2028-03-01
    // Ordering MUST be: 28.02, then 29.02, then 01.03.
    let frozen_2028_feb28 = chrono::NaiveDate::from_ymd_opt(2028, 2, 28).unwrap();
    let up_at_2028 = state
        .storage
        .list_upcoming_birthdays_at(leap_guild, 3, frozen_2028_feb28)
        .await
        .unwrap();
    assert_eq!(up_at_2028[0].user_id, user_feb28);
    assert_eq!(up_at_2028[1].user_id, user_leap);
    assert_eq!(up_at_2028[2].user_id, user_march);

    // On 2028-03-01 (after leap day passed):
    // Next 01.03 is 2028-03-01
    // Next 28.02 is 2029-02-28
    // Next 29.02 is 2032-02-29
    // Ordering MUST be: 01.03, then 28.02, then 29.02.
    let frozen_2028_mar01 = chrono::NaiveDate::from_ymd_opt(2028, 3, 1).unwrap();
    let up_after_leap = state
        .storage
        .list_upcoming_birthdays_at(leap_guild, 3, frozen_2028_mar01)
        .await
        .unwrap();
    assert_eq!(up_after_leap[0].user_id, user_march);
    assert_eq!(up_after_leap[1].user_id, user_feb28);
    assert_eq!(up_after_leap[2].user_id, user_leap);

    // Century boundary check: 2097-01-01 -> 2100 is NOT a leap year, next is 2104-02-29
    let frozen_2097 = chrono::NaiveDate::from_ymd_opt(2097, 1, 1).unwrap();
    let up_century = state
        .storage
        .list_upcoming_birthdays_at(leap_guild, 3, frozen_2097)
        .await
        .unwrap();
    assert_eq!(up_century[0].user_id, user_feb28);
    assert_eq!(up_century[1].user_id, user_march);
    assert_eq!(up_century[2].user_id, user_leap);

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_quotes_and_birthdays_privacy_erasure_and_export_suite() {
    use chrono::Utc;
    use rusty_discord_bot::storage::{BirthdayRecord, QuoteRecord};
    use uuid::Uuid;

    let (_ctx, _mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state().await;

    let guild_a = "100";
    let guild_b = "200";
    let user_u = "111111111111111111";
    let user_v = "222222222222222222";

    // 1. Create quotes in Guild A
    // Q1: added by U, quoting V
    let _q1 = state
        .storage
        .create_quote(QuoteRecord {
            id: Uuid::new_v4(),
            guild_id: guild_a.to_string(),
            added_by: user_u.to_string(),
            author_id: user_v.to_string(),
            body: "Quote 1 by V, added by U".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    // Q2: added by V, quoting U
    let _q2 = state
        .storage
        .create_quote(QuoteRecord {
            id: Uuid::new_v4(),
            guild_id: guild_a.to_string(),
            added_by: user_v.to_string(),
            author_id: user_u.to_string(),
            body: "Quote 2 by U, added by V".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    // Q3: added by V, quoting V (unrelated to U)
    let q3 = state
        .storage
        .create_quote(QuoteRecord {
            id: Uuid::new_v4(),
            guild_id: guild_a.to_string(),
            added_by: user_v.to_string(),
            author_id: user_v.to_string(),
            body: "Quote 3 unrelated to U".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    // Q4: Guild B quote by U (tenant isolation)
    let q4 = state
        .storage
        .create_quote(QuoteRecord {
            id: Uuid::new_v4(),
            guild_id: guild_b.to_string(),
            added_by: user_u.to_string(),
            author_id: user_u.to_string(),
            body: "Quote 4 in Guild B".to_string(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();

    // 2. Set birthdays
    let _b1 = state
        .storage
        .set_birthday(BirthdayRecord {
            guild_id: guild_a.to_string(),
            user_id: user_u.to_string(),
            month: 7,
            day: 10,
            updated_at: Utc::now(),
        })
        .await
        .unwrap();

    let _b2 = state
        .storage
        .set_birthday(BirthdayRecord {
            guild_id: guild_a.to_string(),
            user_id: user_v.to_string(),
            month: 8,
            day: 20,
            updated_at: Utc::now(),
        })
        .await
        .unwrap();

    let _b3 = state
        .storage
        .set_birthday(BirthdayRecord {
            guild_id: guild_b.to_string(),
            user_id: user_u.to_string(),
            month: 7,
            day: 10,
            updated_at: Utc::now(),
        })
        .await
        .unwrap();

    // 3. Verify user quotes and birthdays exist before erasure
    let u_quotes_before = state
        .storage
        .list_user_quotes(guild_a, user_u)
        .await
        .unwrap();
    assert_eq!(u_quotes_before.len(), 2);
    let u_birthday_before = state.storage.get_birthday(guild_a, user_u).await.unwrap();
    assert!(u_birthday_before.is_some());

    // 4. Perform erasure for user U in Guild A
    let erasure_report = state
        .storage
        .erase_guild_user_data(guild_a, user_u)
        .await
        .expect("erase should succeed");

    assert!(
        erasure_report.related_records_erased >= 3,
        "Erasure must erase at least 2 quotes and 1 birthday for user U"
    );

    // 5. Verify Guild A state after erasure
    let u_quotes_after = state
        .storage
        .list_user_quotes(guild_a, user_u)
        .await
        .unwrap();
    assert!(
        u_quotes_after.is_empty(),
        "User U quotes in Guild A must be completely erased"
    );

    let all_quotes_a = state.storage.list_quotes(guild_a, 10).await.unwrap();
    assert_eq!(
        all_quotes_a.len(),
        1,
        "Only unrelated quote Q3 must remain in Guild A"
    );
    assert_eq!(all_quotes_a[0].id, q3.id);

    let u_b_after = state.storage.get_birthday(guild_a, user_u).await.unwrap();
    assert!(
        u_b_after.is_none(),
        "User U birthday in Guild A must be erased"
    );

    let v_b_after = state.storage.get_birthday(guild_a, user_v).await.unwrap();
    assert!(
        v_b_after.is_some(),
        "User V birthday in Guild A must remain intact"
    );

    // 6. Verify Guild B isolation (Q4 and B3 remain)
    let b_quotes_after = state.storage.list_quotes(guild_b, 10).await.unwrap();
    assert_eq!(b_quotes_after.len(), 1);
    assert_eq!(b_quotes_after[0].id, q4.id);

    let b_birthday_after = state.storage.get_birthday(guild_b, user_u).await.unwrap();
    assert!(b_birthday_after.is_some());

    // 7. Verify idempotency of erasure
    let second_erasure = state
        .storage
        .erase_guild_user_data(guild_a, user_u)
        .await
        .expect("second erasure must succeed");
    assert_eq!(
        second_erasure.related_records_erased, 0,
        "Idempotent erasure must report 0 additional records erased"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_afk_storage_and_lifecycle_suite() {
    use rusty_discord_bot::discord::afk::{AfkExecutionContext, execute_afk_action};
    use rusty_discord_bot::lua::AfkOperation;
    use serenity::all::{ChannelId, GuildId, Permissions, UserId};

    let (ctx, _mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state().await;

    let guild_a = "100";
    let guild_b = "200";
    let user_1 = UserId::new(111111111111111111);
    let user_2 = UserId::new(222222222222222222);

    // 1. Initial status check when not AFK
    let res_status_none = execute_afk_action(
        &ctx,
        &state,
        "afk",
        AfkExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(user_1),
            Permissions::empty(),
            false,
        ),
        &AfkOperation::Status { user_id: None },
    )
    .await
    .unwrap();
    assert!(
        res_status_none
            .unwrap()
            .contains("nie ma obecnie aktywnego statusu AFK")
    );

    // 2. Set AFK for user_1 in Guild A
    let res_set = execute_afk_action(
        &ctx,
        &state,
        "afk",
        AfkExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(user_1),
            Permissions::empty(),
            false,
        ),
        &AfkOperation::Set {
            reason: Some("Piszę kod w Rust".to_string()),
        },
    )
    .await
    .unwrap();
    assert!(res_set.unwrap().contains("Piszę kod w Rust"));

    // Set AFK for user_1 in Guild B with different reason (multi-tenant isolation)
    let _ = execute_afk_action(
        &ctx,
        &state,
        "afk",
        AfkExecutionContext::new(
            Some(GuildId::new(200)),
            Some(ChannelId::new(201)),
            Some(user_1),
            Permissions::empty(),
            false,
        ),
        &AfkOperation::Set {
            reason: Some("Inny serwer AFK".to_string()),
        },
    )
    .await
    .unwrap();

    // 3. Status check in Guild A returns Guild A's status
    let res_status_a = execute_afk_action(
        &ctx,
        &state,
        "afk",
        AfkExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(user_1),
            Permissions::empty(),
            false,
        ),
        &AfkOperation::Status { user_id: None },
    )
    .await
    .unwrap();
    assert!(res_status_a.unwrap().contains("Piszę kod w Rust"));

    // Status check in Guild B returns Guild B's status
    let res_status_b = execute_afk_action(
        &ctx,
        &state,
        "afk",
        AfkExecutionContext::new(
            Some(GuildId::new(200)),
            Some(ChannelId::new(201)),
            Some(user_1),
            Permissions::empty(),
            false,
        ),
        &AfkOperation::Status { user_id: None },
    )
    .await
    .unwrap();
    assert!(res_status_b.unwrap().contains("Inny serwer AFK"));

    // 4. OnMessage mention: user_2 mentions user_1 in Guild A
    let _ = execute_afk_action(
        &ctx,
        &state,
        "afk",
        AfkExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(user_2),
            Permissions::empty(),
            false,
        ),
        &AfkOperation::OnMessage {
            author_id: user_2.get().to_string(),
            mentioned_user_ids: vec![user_1.get().to_string()],
            auto_clear: false,
        },
    )
    .await
    .unwrap();
    // user_1 is still AFK
    assert!(
        state
            .storage
            .get_afk(guild_a, &user_1.get().to_string())
            .await
            .unwrap()
            .is_some()
    );

    // 5. OnMessage auto_clear: user_1 sends a message with auto_clear = true
    let _ = execute_afk_action(
        &ctx,
        &state,
        "afk",
        AfkExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(user_1),
            Permissions::empty(),
            false,
        ),
        &AfkOperation::OnMessage {
            author_id: user_1.get().to_string(),
            mentioned_user_ids: vec![],
            auto_clear: true,
        },
    )
    .await
    .unwrap();

    // user_1's AFK in Guild A is now cleared!
    assert!(
        state
            .storage
            .get_afk(guild_a, &user_1.get().to_string())
            .await
            .unwrap()
            .is_none()
    );
    // user_1's AFK in Guild B is STILL intact (tenant isolation)!
    assert!(
        state
            .storage
            .get_afk(guild_b, &user_1.get().to_string())
            .await
            .unwrap()
            .is_some()
    );

    // 6. Explicit clear
    let _ = execute_afk_action(
        &ctx,
        &state,
        "afk",
        AfkExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(user_1),
            Permissions::empty(),
            false,
        ),
        &AfkOperation::Set { reason: None },
    )
    .await
    .unwrap();

    let res_clear = execute_afk_action(
        &ctx,
        &state,
        "afk",
        AfkExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(user_1),
            Permissions::empty(),
            false,
        ),
        &AfkOperation::Clear,
    )
    .await
    .unwrap();
    assert!(res_clear.unwrap().contains("został wyłączony"));

    // 7. Erasure clears AFK
    let _ = execute_afk_action(
        &ctx,
        &state,
        "afk",
        AfkExecutionContext::new(
            Some(GuildId::new(100)),
            Some(ChannelId::new(101)),
            Some(user_1),
            Permissions::empty(),
            false,
        ),
        &AfkOperation::Set { reason: None },
    )
    .await
    .unwrap();
    let rep = state
        .storage
        .erase_guild_user_data(guild_a, &user_1.get().to_string())
        .await
        .unwrap();
    assert!(rep.related_records_erased >= 1);
    assert!(
        state
            .storage
            .get_afk(guild_a, &user_1.get().to_string())
            .await
            .unwrap()
            .is_none()
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_reaction_roles_binding_lifecycle_edit_and_deletion_suite() {
    use axum::http::Method;
    use rusty_discord_bot::discord::reaction_roles::{
        ReactionRoleExecutionContext, execute_reaction_role_action, handle_reaction_role_event,
    };
    use rusty_discord_bot::lua::ReactionRoleOperation;
    use serenity::all::Permissions;

    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state().await;

    let guild_id = GuildId::new(100);
    let channel_id = ChannelId::new(101);
    let admin_id = UserId::new(999999999999999999);
    let user_id = UserId::new(777777777777777777);

    // Enable reaction_roles module
    state
        .storage
        .update_module_settings_with_audit(rusty_discord_bot::storage::UpdateModuleSettingsParams {
            guild_id: "100",
            module_id: "reaction_roles",
            enabled: true,
            config: serde_json::json!({}),
            actor_id: Some("admin"),
            audit_event: "module_enable",
            audit_data: serde_json::json!({}),
        })
        .await
        .unwrap();

    // 1. Create binding: msg 900000000000000010, ⭐ -> role 1002
    let _ = execute_reaction_role_action(
        &ctx,
        &state,
        "reaction_roles",
        ReactionRoleExecutionContext::new(
            Some(guild_id),
            Some(channel_id),
            Some(admin_id),
            Permissions::ADMINISTRATOR,
            Permissions::ADMINISTRATOR,
            true,
        ),
        &ReactionRoleOperation::Create {
            message_id: "900000000000000010".to_string(),
            emoji: "⭐".to_string(),
            role_id: "1002".to_string(),
        },
    )
    .await
    .unwrap();

    // 2. User reacts ⭐ -> receives role 1002
    let reaction_add: serenity::all::Reaction = serde_json::from_value(serde_json::json!({
        "channel_id": "101",
        "message_id": "900000000000000010",
        "guild_id": "100",
        "user_id": user_id.get().to_string(),
        "emoji": { "name": "⭐", "id": null },
        "burst": false,
        "type": 0
    }))
    .unwrap();

    handle_reaction_role_event(&ctx, &state, &reaction_add, true)
        .await
        .unwrap();

    // Verify grant recorded
    let grants_1002 = state
        .storage
        .list_active_reaction_role_grants_for_user_role("100", &user_id.get().to_string(), "1002")
        .await
        .unwrap();
    assert_eq!(grants_1002.len(), 1);
    assert!(grants_1002[0].granted_by_bot);

    // 3. Admin edits binding: msg 900000000000000010, ⭐ -> role 1005 (role change!)
    let _ = execute_reaction_role_action(
        &ctx,
        &state,
        "reaction_roles",
        ReactionRoleExecutionContext::new(
            Some(guild_id),
            Some(channel_id),
            Some(admin_id),
            Permissions::ADMINISTRATOR,
            Permissions::ADMINISTRATOR,
            true,
        ),
        &ReactionRoleOperation::Create {
            message_id: "900000000000000010".to_string(),
            emoji: "⭐".to_string(),
            role_id: "1005".to_string(),
        },
    )
    .await
    .unwrap();

    // Verify old role 1002 grant was released upon role mapping change
    let grants_1002_after_edit = state
        .storage
        .list_active_reaction_role_grants_for_user_role("100", &user_id.get().to_string(), "1002")
        .await
        .unwrap();
    assert!(
        grants_1002_after_edit.is_empty(),
        "Grant for old role 1002 must be released"
    );

    // Verify remove_member_role was called for role 1002
    let calls = mock.recorded_calls.lock().await.clone();
    assert!(
        calls.iter().any(|(m, p)| *m == Method::DELETE
            && p.contains("/guilds/100/members/777777777777777777/roles/1002")),
        "remove_member_role must be called for role 1002 upon mapping update"
    );

    // 4. User reacts ⭐ again -> receives new role 1005
    handle_reaction_role_event(&ctx, &state, &reaction_add, true)
        .await
        .unwrap();

    let grants_1005 = state
        .storage
        .list_active_reaction_role_grants_for_user_role("100", &user_id.get().to_string(), "1005")
        .await
        .unwrap();
    assert_eq!(grants_1005.len(), 1);

    // 5. Admin removes binding: msg 900000000000000010, ⭐
    let _ = execute_reaction_role_action(
        &ctx,
        &state,
        "reaction_roles",
        ReactionRoleExecutionContext::new(
            Some(guild_id),
            Some(channel_id),
            Some(admin_id),
            Permissions::ADMINISTRATOR,
            Permissions::ADMINISTRATOR,
            true,
        ),
        &ReactionRoleOperation::Remove {
            message_id: "900000000000000010".to_string(),
            emoji: "⭐".to_string(),
        },
    )
    .await
    .unwrap();

    // Verify binding is deleted from storage
    let binding_after_del = state
        .storage
        .get_reaction_role_binding("100", "900000000000000010", "⭐")
        .await
        .unwrap();
    assert!(binding_after_del.is_none());

    // Verify role 1005 grant is gone
    let grants_1005_after_del = state
        .storage
        .list_active_reaction_role_grants_for_user_role("100", &user_id.get().to_string(), "1005")
        .await
        .unwrap();
    assert!(grants_1005_after_del.is_empty());

    // Verify remove_member_role was called for role 1005
    let calls_after = mock.recorded_calls.lock().await.clone();
    assert!(
        calls_after.iter().any(|(m, p)| *m == Method::DELETE
            && p.contains("/guilds/100/members/777777777777777777/roles/1005")),
        "remove_member_role must be called for role 1005 upon binding deletion"
    );

    // 6. Shared grant test: 2 bindings pointing to role 1006
    let _ = execute_reaction_role_action(
        &ctx,
        &state,
        "reaction_roles",
        ReactionRoleExecutionContext::new(
            Some(guild_id),
            Some(channel_id),
            Some(admin_id),
            Permissions::ADMINISTRATOR,
            Permissions::ADMINISTRATOR,
            true,
        ),
        &ReactionRoleOperation::Create {
            message_id: "900000000000000021".to_string(),
            emoji: "🅰️".to_string(),
            role_id: "1006".to_string(),
        },
    )
    .await
    .unwrap();

    let _ = execute_reaction_role_action(
        &ctx,
        &state,
        "reaction_roles",
        ReactionRoleExecutionContext::new(
            Some(guild_id),
            Some(channel_id),
            Some(admin_id),
            Permissions::ADMINISTRATOR,
            Permissions::ADMINISTRATOR,
            true,
        ),
        &ReactionRoleOperation::Create {
            message_id: "900000000000000022".to_string(),
            emoji: "🅱️".to_string(),
            role_id: "1006".to_string(),
        },
    )
    .await
    .unwrap();

    // User reacts to both
    let reaction_a: serenity::all::Reaction = serde_json::from_value(serde_json::json!({
        "channel_id": "101",
        "message_id": "900000000000000021",
        "guild_id": "100",
        "user_id": user_id.get().to_string(),
        "emoji": { "name": "🅰️", "id": null },
        "burst": false,
        "type": 0
    }))
    .unwrap();
    let reaction_b: serenity::all::Reaction = serde_json::from_value(serde_json::json!({
        "channel_id": "101",
        "message_id": "900000000000000022",
        "guild_id": "100",
        "user_id": user_id.get().to_string(),
        "emoji": { "name": "🅱️", "id": null },
        "burst": false,
        "type": 0
    }))
    .unwrap();

    handle_reaction_role_event(&ctx, &state, &reaction_a, true)
        .await
        .unwrap();
    handle_reaction_role_event(&ctx, &state, &reaction_b, true)
        .await
        .unwrap();

    let grants_1006 = state
        .storage
        .list_active_reaction_role_grants_for_user_role("100", &user_id.get().to_string(), "1006")
        .await
        .unwrap();
    assert_eq!(grants_1006.len(), 2);

    // Remove binding A: role 1006 must NOT be revoked because binding B is still active
    mock.recorded_calls.lock().await.clear();
    let _ = execute_reaction_role_action(
        &ctx,
        &state,
        "reaction_roles",
        ReactionRoleExecutionContext::new(
            Some(guild_id),
            Some(channel_id),
            Some(admin_id),
            Permissions::ADMINISTRATOR,
            Permissions::ADMINISTRATOR,
            true,
        ),
        &ReactionRoleOperation::Remove {
            message_id: "900000000000000021".to_string(),
            emoji: "🅰️".to_string(),
        },
    )
    .await
    .unwrap();

    let calls_after_a = mock.recorded_calls.lock().await.clone();
    assert!(
        !calls_after_a.iter().any(|(m, p)| *m == Method::DELETE
            && p.contains("/guilds/100/members/777777777777777777/roles/1006")),
        "Role 1006 must NOT be revoked while binding B still grants it"
    );

    // Remove binding B: now role 1006 MUST be revoked because remaining_active_sources == 0
    let _ = execute_reaction_role_action(
        &ctx,
        &state,
        "reaction_roles",
        ReactionRoleExecutionContext::new(
            Some(guild_id),
            Some(channel_id),
            Some(admin_id),
            Permissions::ADMINISTRATOR,
            Permissions::ADMINISTRATOR,
            true,
        ),
        &ReactionRoleOperation::Remove {
            message_id: "900000000000000022".to_string(),
            emoji: "🅱️".to_string(),
        },
    )
    .await
    .unwrap();

    let calls_after_b = mock.recorded_calls.lock().await.clone();
    assert!(
        calls_after_b.iter().any(|(m, p)| *m == Method::DELETE
            && p.contains("/guilds/100/members/777777777777777777/roles/1006")),
        "Role 1006 must be revoked when the last binding is removed"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_afk_mention_aggregation_and_rate_limiting_suite() {
    use axum::http::Method;
    use rusty_discord_bot::discord::afk::{AfkExecutionContext, execute_afk_action};
    use rusty_discord_bot::lua::AfkOperation;
    use serenity::all::{ChannelId, GuildId, Permissions, UserId};
    use std::sync::atomic::Ordering;

    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state().await;

    let guild_id = GuildId::new(100);
    let channel_id = ChannelId::new(101);
    let channel_2 = ChannelId::new(102);

    // 1. Setup 10 AFK users in guild 100
    for i in 1..=10 {
        let user = UserId::new(1000 + i);
        let _ = execute_afk_action(
            &ctx,
            &state,
            "afk",
            AfkExecutionContext::new(
                Some(guild_id),
                Some(channel_id),
                Some(user),
                Permissions::empty(),
                false,
            ),
            &AfkOperation::Set {
                reason: Some(format!("AFK task #{i}")),
            },
        )
        .await
        .unwrap();
    }

    mock.recorded_calls.lock().await.clear();

    // 2. User 222222222222222222 mentions 10 users + self + duplicate mentions
    let author_id = UserId::new(222222222222222222);
    let mut mentions: Vec<String> = (1..=10).map(|i| (1000 + i).to_string()).collect();
    mentions.push(author_id.get().to_string()); // Self-mention (must be ignored)
    mentions.push("1001".to_string()); // Duplicate mention (must be deduplicated)

    let _ = execute_afk_action(
        &ctx,
        &state,
        "afk",
        AfkExecutionContext::new(
            Some(guild_id),
            Some(channel_id),
            Some(author_id),
            Permissions::empty(),
            false,
        ),
        &AfkOperation::OnMessage {
            author_id: author_id.get().to_string(),
            mentioned_user_ids: mentions,
            auto_clear: false,
        },
    )
    .await
    .unwrap();

    // Exactly 1 outbound Discord POST /messages must be dispatched (fan-out prevention & aggregation)
    let calls = mock.recorded_calls.lock().await.clone();
    let msg_posts: Vec<_> = calls
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(
        msg_posts.len(),
        1,
        "Exactly 1 aggregate message must be sent to Discord instead of N individual messages"
    );

    // 3. Mentioning recently announced user (1001) in the same channel must be suppressed by per-target cooldown
    mock.recorded_calls.lock().await.clear();
    let author_2 = UserId::new(333333333333333333);
    let _ = execute_afk_action(
        &ctx,
        &state,
        "afk",
        AfkExecutionContext::new(
            Some(guild_id),
            Some(channel_id),
            Some(author_2),
            Permissions::empty(),
            false,
        ),
        &AfkOperation::OnMessage {
            author_id: author_2.get().to_string(),
            mentioned_user_ids: vec!["1001".to_string()],
            auto_clear: false,
        },
    )
    .await
    .unwrap();

    let calls_cooldown = mock.recorded_calls.lock().await.clone();
    assert_eq!(
        calls_cooldown
            .iter()
            .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
            .count(),
        0,
        "Notice must be suppressed while target user 1001 is on cooldown in channel 101"
    );

    // 4. Mentioning 1001 (on cooldown) along with 1006 (not yet announced) sends notice ONLY for 1006
    let _ = execute_afk_action(
        &ctx,
        &state,
        "afk",
        AfkExecutionContext::new(
            Some(guild_id),
            Some(channel_id),
            Some(author_2),
            Permissions::empty(),
            false,
        ),
        &AfkOperation::OnMessage {
            author_id: author_2.get().to_string(),
            mentioned_user_ids: vec!["1001".to_string(), "1006".to_string()],
            auto_clear: false,
        },
    )
    .await
    .unwrap();

    let calls_mixed = mock.recorded_calls.lock().await.clone();
    assert_eq!(
        calls_mixed
            .iter()
            .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
            .count(),
        1,
        "Notice for unannounced target 1006 must be sent"
    );

    // 5. Channel rate limit: 2 notices have been sent in channel 101 (limit is 2 per 30s)
    // A 3rd message mentioning 1007 in channel 101 must be throttled
    let author_3 = UserId::new(444444444444444444);
    let baseline_suppressed = state
        .metrics
        .afk_notifications_suppressed_total
        .load(Ordering::Relaxed);
    mock.recorded_calls.lock().await.clear();

    let _ = execute_afk_action(
        &ctx,
        &state,
        "afk",
        AfkExecutionContext::new(
            Some(guild_id),
            Some(channel_id),
            Some(author_3),
            Permissions::empty(),
            false,
        ),
        &AfkOperation::OnMessage {
            author_id: author_3.get().to_string(),
            mentioned_user_ids: vec!["1007".to_string()],
            auto_clear: false,
        },
    )
    .await
    .unwrap();

    let calls_throttled = mock.recorded_calls.lock().await.clone();
    assert_eq!(
        calls_throttled
            .iter()
            .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
            .count(),
        0,
        "Channel notice #3 in 30s must be throttled by channel rate limit"
    );
    assert!(
        state
            .metrics
            .afk_notifications_suppressed_total
            .load(Ordering::Relaxed)
            > baseline_suppressed,
        "afk_notifications_suppressed_total metric must be incremented"
    );

    let audits = state.storage.list_audit_events("100", 10).await.unwrap();
    assert!(
        audits.iter().any(|a| a.event == "afk_notice_throttled"),
        "Audit event afk_notice_throttled must be recorded upon throttling"
    );

    // 6. Channel isolation: channel 102 has independent rate limiter and target cooldowns
    let _ = execute_afk_action(
        &ctx,
        &state,
        "afk",
        AfkExecutionContext::new(
            Some(guild_id),
            Some(channel_2),
            Some(author_3),
            Permissions::empty(),
            false,
        ),
        &AfkOperation::OnMessage {
            author_id: author_3.get().to_string(),
            mentioned_user_ids: vec!["1007".to_string()],
            auto_clear: false,
        },
    )
    .await
    .unwrap();

    let calls_chan2 = mock.recorded_calls.lock().await.clone();
    assert_eq!(
        calls_chan2
            .iter()
            .filter(|(m, p)| *m == Method::POST && p.contains("/channels/102/messages"))
            .count(),
        1,
        "Channel 102 must deliver notice without being affected by channel 101's limits"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_afk_multibyte_emoji_truncation_utf8_safety_suite() {
    use axum::http::Method;
    use rusty_discord_bot::discord::afk::{AfkExecutionContext, execute_afk_action};
    use rusty_discord_bot::lua::AfkOperation;
    use serenity::all::{ChannelId, GuildId, Permissions, UserId};

    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state().await;

    let guild_id = GuildId::new(200);
    let channel_id = ChannelId::new(201);

    // Two users with 19-digit IDs and 256-character 4-byte emoji reasons ("🫠".repeat(256))
    let user1 = UserId::new(1111111111111111111);
    let user2 = UserId::new(2222222222222222222);
    let emoji_reason = "🫠".repeat(256);

    let _ = execute_afk_action(
        &ctx,
        &state,
        "afk",
        AfkExecutionContext::new(
            Some(guild_id),
            Some(channel_id),
            Some(user1),
            Permissions::empty(),
            false,
        ),
        &AfkOperation::Set {
            reason: Some(emoji_reason.clone()),
        },
    )
    .await
    .unwrap();

    let _ = execute_afk_action(
        &ctx,
        &state,
        "afk",
        AfkExecutionContext::new(
            Some(guild_id),
            Some(channel_id),
            Some(user2),
            Permissions::empty(),
            false,
        ),
        &AfkOperation::Set {
            reason: Some(emoji_reason),
        },
    )
    .await
    .unwrap();

    mock.recorded_calls.lock().await.clear();

    // Author mentions both user1 and user2 in channel 201
    let author = UserId::new(333333333333333333);
    let res = execute_afk_action(
        &ctx,
        &state,
        "afk",
        AfkExecutionContext::new(
            Some(guild_id),
            Some(channel_id),
            Some(author),
            Permissions::empty(),
            false,
        ),
        &AfkOperation::OnMessage {
            author_id: author.get().to_string(),
            mentioned_user_ids: vec![user1.get().to_string(), user2.get().to_string()],
            auto_clear: false,
        },
    )
    .await;

    assert!(
        res.is_ok(),
        "OnMessage must not panic on UTF-8 emoji boundary"
    );

    let calls = mock.recorded_calls.lock().await.clone();
    let msg_posts: Vec<_> = calls
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/201/messages"))
        .collect();
    assert_eq!(msg_posts.len(), 1, "Exactly 1 message must be dispatched");

    let (_, payload) = &msg_posts[0];
    assert!(payload.contains(&user1.get().to_string()));
    // Verify valid UTF-8 and bounded length
    let parts: Vec<&str> = payload.split('|').collect();
    let body_json: serde_json::Value = serde_json::from_str(parts[1]).expect("valid json body");
    let content = body_json["content"].as_str().expect("content string");
    assert!(content.len() <= 2000, "Content bytes must not exceed 2000");
    assert!(
        content.chars().count() <= 2000,
        "Content chars must not exceed 2000"
    );

    // Because user2's notice did not fit, user2 must NOT have consumed their per-target cooldown.
    // In a new message, mentioning user2 alone in channel 201 should succeed!
    mock.recorded_calls.lock().await.clear();
    let author2 = UserId::new(444444444444444444);
    let res2 = execute_afk_action(
        &ctx,
        &state,
        "afk",
        AfkExecutionContext::new(
            Some(guild_id),
            Some(channel_id),
            Some(author2),
            Permissions::empty(),
            false,
        ),
        &AfkOperation::OnMessage {
            author_id: author2.get().to_string(),
            mentioned_user_ids: vec![user2.get().to_string()],
            auto_clear: false,
        },
    )
    .await;

    assert!(res2.is_ok());
    let calls2 = mock.recorded_calls.lock().await.clone();
    let msg_posts2: Vec<_> = calls2
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/201/messages"))
        .collect();
    assert_eq!(
        msg_posts2.len(),
        1,
        "User 2 notice must be delivered in subsequent message since their cooldown was not consumed"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_web_broadcast_destination_channel_authorization_suite() {
    use dashmap::DashMap;
    use reqwest::header::COOKIE;
    use rusty_discord_bot::web::{
        WebState,
        auth::{DashboardGuild, DashboardUser, LoginRateLimiter},
        build_router,
    };

    let (_ctx, mock, shard_manager) = start_mock_discord_environment().await;

    let bind: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    let config = Config {
        discord_token: "test_bot_token".to_string(),
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
        discord_api_base_url: format!("http://127.0.0.1:{}", mock.port),
        transcript_encryption_key: None,
        transcript_active_key_id: "v1".to_string(),
        transcript_keyring: None,
        ticket_transcripts_disabled: true,
        metrics_token: None,
        metrics_allowed_ips: std::collections::HashSet::new(),
    };

    let app = AppState::bootstrap(config)
        .await
        .expect("bootstrap app state");
    let permissions_cache = Arc::new(DashMap::new());
    let web_state = WebState {
        app: app.clone(),
        login_rate_limiter: Arc::new(LoginRateLimiter::new()),
        permissions_cache: permissions_cache.clone(),
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let base_url = format!("http://{addr}");
    let router = build_router(web_state.clone());

    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve test web");
    });

    let http_client = reqwest::Client::new();

    // 1. Setup session for manager (User 333333333333333333 with MANAGE_GUILD)
    let session_id = "test_broadcast_manager_session";
    let user = DashboardUser {
        id: "333333333333333333".to_owned(),
        username: "manager_user".to_owned(),
        global_name: None,
        avatar: None,
    };
    let guilds = vec![DashboardGuild {
        id: "100".to_owned(),
        name: "Test Guild".to_owned(),
        icon: None,
        owner: false,
        permissions: "32".to_owned(), // MANAGE_GUILD
    }];
    let csrf_token = "valid_csrf_token_broadcast";
    let expires_at = chrono::Utc::now() + chrono::Duration::hours(24);
    web_state
        .app
        .sessions
        .save_session(
            session_id,
            user,
            guilds,
            csrf_token.to_owned(),
            expires_at,
            None,
        )
        .await
        .unwrap();

    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    let baseline_requests = mock.modifying_requests_count.load(Ordering::SeqCst);

    // Test A: Target hidden-private channel 103 (deny VIEW_CHANNEL)
    let resp_103 = http_client
        .post(format!("{base_url}/api/guilds/100/broadcast"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "target_channels": ["103"],
            "title": "Confidential Staff Update",
            "description": "Unauthorized announcement attempt"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp_103.status(), reqwest::StatusCode::OK);
    let body_103: serde_json::Value = resp_103.json().await.unwrap();
    assert_eq!(body_103["success_count"], 0);
    assert_eq!(body_103["failure_count"], 1);
    let err_103 = body_103["failures"][0]["error"].as_str().unwrap();
    assert!(
        err_103.contains("VIEW_CHANNEL"),
        "Error must mention VIEW_CHANNEL: {err_103}"
    );
    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_requests,
        "Zero POST requests dispatched for hidden channel"
    );

    // Test B: Target visible-no-send channel 104 (deny SEND_MESSAGES)
    let resp_104 = http_client
        .post(format!("{base_url}/api/guilds/100/broadcast"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "target_channels": ["104"],
            "title": "Read-Only Announcement",
            "description": "Unauthorized announcement attempt"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp_104.status(), reqwest::StatusCode::OK);
    let body_104: serde_json::Value = resp_104.json().await.unwrap();
    assert_eq!(body_104["success_count"], 0);
    assert_eq!(body_104["failure_count"], 1);
    let err_104 = body_104["failures"][0]["error"].as_str().unwrap();
    assert!(
        err_104.contains("SEND_MESSAGES"),
        "Error must mention SEND_MESSAGES: {err_104}"
    );
    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_requests,
        "Zero POST requests dispatched for read-only channel"
    );

    // Test C: Target visible-no-embed channel 108 (deny EMBED_LINKS)
    let resp_108 = http_client
        .post(format!("{base_url}/api/guilds/100/broadcast"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "target_channels": ["108"],
            "title": "No-Embed Announcement",
            "description": "Unauthorized announcement attempt"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp_108.status(), reqwest::StatusCode::OK);
    let body_108: serde_json::Value = resp_108.json().await.unwrap();
    assert_eq!(body_108["success_count"], 0);
    assert_eq!(body_108["failure_count"], 1);
    let err_108 = body_108["failures"][0]["error"].as_str().unwrap();
    assert!(
        err_108.contains("EMBED_LINKS"),
        "Error must mention EMBED_LINKS: {err_108}"
    );
    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_requests,
        "Zero POST requests dispatched for channel denying EMBED_LINKS"
    );

    // Test D: Target private thread 105 where manager is not a member
    let resp_105 = http_client
        .post(format!("{base_url}/api/guilds/100/broadcast"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "target_channels": ["105"],
            "title": "Private Thread Announcement",
            "description": "Unauthorized announcement attempt"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp_105.status(), reqwest::StatusCode::OK);
    let body_105: serde_json::Value = resp_105.json().await.unwrap();
    assert_eq!(body_105["success_count"], 0);
    assert_eq!(body_105["failure_count"], 1);
    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_requests,
        "Zero POST requests dispatched for non-member private thread"
    );

    // Test E: Target channel 201 from another guild (foreign guild 200)
    let resp_201 = http_client
        .post(format!("{base_url}/api/guilds/100/broadcast"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "target_channels": ["201"],
            "title": "Cross-Guild Announcement",
            "description": "Unauthorized announcement attempt"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp_201.status(), reqwest::StatusCode::OK);
    let body_201: serde_json::Value = resp_201.json().await.unwrap();
    assert_eq!(body_201["success_count"], 0);
    assert_eq!(body_201["failure_count"], 1);
    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_requests,
        "Zero POST requests dispatched for cross-guild channel"
    );

    // Test F: Target valid public channel 101 with full permissions -> succeeds!
    let resp_101 = http_client
        .post(format!("{base_url}/api/guilds/100/broadcast"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "target_channels": ["101"],
            "title": "Legitimate Server Announcement",
            "description": "Welcome everyone!"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp_101.status(), reqwest::StatusCode::OK);
    let body_101: serde_json::Value = resp_101.json().await.unwrap();
    assert_eq!(body_101["success_count"], 1);
    assert_eq!(body_101["failure_count"], 0);
    assert_eq!(body_101["successes"][0], "101");
    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_requests + 1,
        "Exactly 1 message POST request dispatched for legitimate channel"
    );

    // Test G: Partial batch with 1 valid and 2 invalid channels: ["103", "101", "104"]
    let resp_batch = http_client
        .post(format!("{base_url}/api/guilds/100/broadcast"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "target_channels": ["103", "101", "104"],
            "title": "Batch Announcement",
            "description": "Testing partial delivery authorization"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp_batch.status(), reqwest::StatusCode::OK);
    let body_batch: serde_json::Value = resp_batch.json().await.unwrap();
    assert_eq!(body_batch["success_count"], 1);
    assert_eq!(body_batch["failure_count"], 2);
    assert_eq!(body_batch["successes"][0], "101");
    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_requests + 2,
        "Only the authorized channel received a POST dispatch"
    );

    // Test H: Server Owner / Administrator (User 111111111111111111) bypasses overwrites
    let admin_session_id = "test_broadcast_admin_session";
    let admin_user = DashboardUser {
        id: "111111111111111111".to_owned(),
        username: "owner_user".to_owned(),
        global_name: None,
        avatar: None,
    };
    let admin_guilds = vec![DashboardGuild {
        id: "100".to_owned(),
        name: "Test Guild".to_owned(),
        icon: None,
        owner: true,
        permissions: "8".to_owned(), // ADMINISTRATOR
    }];
    let expires_at_admin = chrono::Utc::now() + chrono::Duration::hours(24);
    web_state
        .app
        .sessions
        .save_session(
            admin_session_id,
            admin_user,
            admin_guilds,
            csrf_token.to_owned(),
            expires_at_admin,
            None,
        )
        .await
        .unwrap();
    let admin_cookie = format!("rusty_discord_bot_session={admin_session_id}");

    let resp_admin = http_client
        .post(format!("{base_url}/api/guilds/100/broadcast"))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "target_channels": ["103", "104"],
            "title": "Admin Announcement",
            "description": "Administrator bypasses channel overwrites"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp_admin.status(), reqwest::StatusCode::OK);
    let body_admin: serde_json::Value = resp_admin.json().await.unwrap();
    assert_eq!(body_admin["success_count"], 2);
    assert_eq!(body_admin["failure_count"], 0);

    // Test F: Duplicate channel IDs in target_channels [101, 101] -> 400 Bad Request (#317)
    let baseline_before_dups = mock.modifying_requests_count.load(Ordering::SeqCst);
    let resp_dup = http_client
        .post(format!("{base_url}/api/guilds/100/broadcast"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "target_channels": ["101", "101"],
            "title": "Duplicate Channels Announcement",
            "description": "Attempting broadcast with duplicate channels"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp_dup.status(), reqwest::StatusCode::BAD_REQUEST);
    let body_dup: serde_json::Value = resp_dup.json().await.unwrap();
    assert!(
        body_dup["error"]
            .as_str()
            .unwrap()
            .contains("target_channels must be unique"),
        "Error message should mention unique target_channels: {body_dup:?}"
    );
    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_before_dups,
        "No Discord POST /messages requests should be sent when duplicate channels are submitted"
    );

    // Test G: 50 identical duplicate channel IDs [101 x 50] -> 400 Bad Request (#317)
    let fifty_dups: Vec<String> = (0..50).map(|_| "101".to_string()).collect();
    let resp_50_dups = http_client
        .post(format!("{base_url}/api/guilds/100/broadcast"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "target_channels": fifty_dups,
            "title": "50 Duplicate Channels Announcement",
            "description": "Attempting broadcast with 50 duplicate channels"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp_50_dups.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(
        mock.modifying_requests_count.load(Ordering::SeqCst),
        baseline_before_dups,
        "No Discord modifying requests should be dispatched for 50 duplicates"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_sticky_messages_end_to_end_delivery_and_anti_recursion() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state_with_port(mock.port).await;
    let handler = Handler::new(state.clone());

    // 1. Configure sticky message on channel 101 in guild 100 with cooldown 5s
    state
        .storage
        .set_sticky_message(
            "100",
            "101",
            "",
            "📌 Important Server Rules: Be respectful to all members!",
            5,
        )
        .await
        .expect("set sticky message");

    let initial = state
        .storage
        .get_sticky_message("100", "101")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(initial.last_posted_message_id, None);
    assert_eq!(initial.last_bumped_at, None);
    assert_eq!(initial.generation, 0);

    // Step A: Member sends a normal message in channel 101
    let member_msg: Message = serde_json::from_value(json!({
        "id": "111000000000000001",
        "channel_id": "101",
        "guild_id": "100",
        "author": {
            "id": "222222222222222222",
            "username": "NormalUser",
            "discriminator": "0000",
            "avatar": null,
            "bot": false,
        },
        "content": "Hello world from member!",
        "timestamp": "2026-11-08T07:00:00Z",
        "tts": false,
        "mention_everyone": false,
        "mentions": [],
        "mention_roles": [],
        "attachments": [],
        "embeds": [],
        "pinned": false,
        "type": 0,
    }))
    .expect("deserialize member_msg");

    handler.message(ctx.clone(), member_msg).await;

    // Wait for channel worker to deliver
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Verify: Exactly 1 POST /channels/101/messages was dispatched
    let recorded = mock.recorded_calls.lock().await.clone();
    let channel_101_posts: Vec<_> = recorded
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(
        channel_101_posts.len(),
        1,
        "Exactly 1 sticky message should be posted to channel 101"
    );

    // Verify content and safe mentions (no @everyone or user pings)
    let post_body = &channel_101_posts[0].1;
    assert!(post_body.contains("Important Server Rules"));
    assert!(post_body.contains("\"allowed_mentions\""));

    // Verify storage was committed with live message ID from mock
    let updated = state
        .storage
        .get_sticky_message("100", "101")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        updated.last_posted_message_id.as_deref(),
        Some("900000000000000001")
    );
    assert_eq!(updated.message_id, "900000000000000001");
    assert!(updated.last_bumped_at.is_some());
    assert!(updated.generation >= 1);

    // Step B: Bot message in channel 101 MUST NOT trigger recursion
    let bot_msg: Message = serde_json::from_value(json!({
        "id": "900000000000000001",
        "channel_id": "101",
        "guild_id": "100",
        "author": {
            "id": "999999999999999999",
            "username": "MockBot",
            "discriminator": "0000",
            "avatar": null,
            "bot": true,
        },
        "content": "📌 Important Server Rules: Be respectful to all members!",
        "timestamp": "2026-11-08T07:00:01Z",
        "tts": false,
        "mention_everyone": false,
        "mentions": [],
        "mention_roles": [],
        "attachments": [],
        "embeds": [],
        "pinned": false,
        "type": 0,
    }))
    .expect("deserialize bot_msg");

    handler.message(ctx.clone(), bot_msg).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Check that no additional message was posted
    let recorded_after_bot = mock.recorded_calls.lock().await.clone();
    let posts_after_bot: Vec<_> = recorded_after_bot
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(
        posts_after_bot.len(),
        1,
        "Bot messages must NOT trigger recursive sticky refresh"
    );

    // Step C: Burst of 30 member messages within cooldown window
    for i in 2..=31 {
        let burst_msg: Message = serde_json::from_value(json!({
            "id": format!("1110000000000000{:02}", i),
            "channel_id": "101",
            "guild_id": "100",
            "author": {
                "id": "222222222222222222",
                "username": "NormalUser",
                "discriminator": "0000",
                "avatar": null,
                "bot": false,
            },
            "content": format!("Chatter message {i}"),
            "timestamp": "2026-11-08T07:00:02Z",
            "tts": false,
            "mention_everyone": false,
            "mentions": [],
            "mention_roles": [],
            "attachments": [],
            "embeds": [],
            "pinned": false,
            "type": 0,
        }))
        .expect("deserialize burst_msg");

        handler.message(ctx.clone(), burst_msg).await;
    }

    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    // Verify: across all 30 messages in burst, total posts is STILL exactly 1!
    let recorded_after_burst = mock.recorded_calls.lock().await.clone();
    let posts_after_burst: Vec<_> = recorded_after_burst
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(
        posts_after_burst.len(),
        1,
        "Burst of 30 messages within cooldown must result in AT MOST 1 delivery"
    );

    // Step D: Message after cooldown expires triggers refresh and deletes old message
    let past_time = chrono::Utc::now() - chrono::Duration::seconds(15);
    let cur_sticky = state
        .storage
        .get_sticky_message("100", "101")
        .await
        .unwrap()
        .unwrap();
    state
        .storage
        .update_sticky_posted_message(
            "100",
            "101",
            "900000000000000001",
            past_time,
            cur_sticky.generation,
            cur_sticky.config_revision,
        )
        .await
        .unwrap();

    state.sticky_coordinator.invalidate_channel("100", "101");

    let post_cooldown_msg: Message = serde_json::from_value(json!({
        "id": "111000000000000032",
        "channel_id": "101",
        "guild_id": "100",
        "author": {
            "id": "222222222222222222",
            "username": "NormalUser",
            "discriminator": "0000",
            "avatar": null,
            "bot": false,
        },
        "content": "Message after cooldown elapsed!",
        "timestamp": "2026-11-08T07:00:20Z",
        "tts": false,
        "mention_everyone": false,
        "mentions": [],
        "mention_roles": [],
        "attachments": [],
        "embeds": [],
        "pinned": false,
        "type": 0,
    }))
    .expect("deserialize post_cooldown_msg");

    handler.message(ctx.clone(), post_cooldown_msg).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let recorded_after_refresh = mock.recorded_calls.lock().await.clone();
    let posts_after_refresh: Vec<_> = recorded_after_refresh
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(
        posts_after_refresh.len(),
        2,
        "New user message after cooldown must refresh the sticky message"
    );

    // Verify old sticky message was deleted
    let deletes_recorded: Vec<_> = recorded_after_refresh
        .iter()
        .filter(|(m, p)| {
            *m == Method::DELETE && p.contains("/channels/101/messages/900000000000000001")
        })
        .collect();
    assert_eq!(
        deletes_recorded.len(),
        1,
        "Old sticky message must be deleted when sticky message is refreshed"
    );

    // Step E: After DELETE /sticky/101, subsequent messages produce 0 deliveries
    state
        .storage
        .delete_sticky_message("100", "101")
        .await
        .unwrap();
    state.sticky_coordinator.remove_channel("100", "101");

    let post_delete_msg: Message = serde_json::from_value(json!({
        "id": "111000000000000033",
        "channel_id": "101",
        "guild_id": "100",
        "author": {
            "id": "222222222222222222",
            "username": "NormalUser",
            "discriminator": "0000",
            "avatar": null,
            "bot": false,
        },
        "content": "Message after sticky deleted!",
        "timestamp": "2026-11-08T07:00:25Z",
        "tts": false,
        "mention_everyone": false,
        "mentions": [],
        "mention_roles": [],
        "attachments": [],
        "embeds": [],
        "pinned": false,
        "type": 0,
    }))
    .expect("deserialize post_delete_msg");

    handler.message(ctx.clone(), post_delete_msg).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let recorded_final = mock.recorded_calls.lock().await.clone();
    let posts_final: Vec<_> = recorded_final
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(
        posts_final.len(),
        2,
        "No new sticky deliveries after configuration was deleted"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_sticky_messages_channel_guards_and_permissions() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state_with_port(mock.port).await;
    let handler = Handler::new(state.clone());

    // 1. Channel 401: Category channel guard
    state
        .storage
        .set_sticky_message("100", "401", "", "Sticky in category", 5)
        .await
        .unwrap();

    let cat_msg: Message = serde_json::from_value(json!({
        "id": "111000000000000401",
        "channel_id": "401",
        "guild_id": "100",
        "author": {
            "id": "222222222222222222",
            "username": "NormalUser",
            "discriminator": "0000",
            "avatar": null,
            "bot": false,
        },
        "content": "Message in category channel",
        "timestamp": "2026-11-08T07:00:00Z",
        "tts": false,
        "mention_everyone": false,
        "mentions": [],
        "mention_roles": [],
        "attachments": [],
        "embeds": [],
        "pinned": false,
        "type": 0,
    }))
    .unwrap();
    handler.message(ctx.clone(), cat_msg).await;

    // 2. Channel 201: Cross-guild isolation (belongs to guild 200, but configured in 100)
    state
        .storage
        .set_sticky_message("100", "201", "", "Sticky cross guild", 5)
        .await
        .unwrap();

    let cross_msg: Message = serde_json::from_value(json!({
        "id": "111000000000000201",
        "channel_id": "201",
        "guild_id": "100",
        "author": {
            "id": "222222222222222222",
            "username": "NormalUser",
            "discriminator": "0000",
            "avatar": null,
            "bot": false,
        },
        "content": "Message targeting foreign channel",
        "timestamp": "2026-11-08T07:00:00Z",
        "tts": false,
        "mention_everyone": false,
        "mentions": [],
        "mention_roles": [],
        "attachments": [],
        "embeds": [],
        "pinned": false,
        "type": 0,
    }))
    .unwrap();
    handler.message(ctx.clone(), cross_msg).await;

    // 3. Channel 999: Non-existent channel (returns 404 from Discord)
    state
        .storage
        .set_sticky_message("100", "999", "", "Sticky missing channel", 5)
        .await
        .unwrap();

    let missing_msg: Message = serde_json::from_value(json!({
        "id": "111000000000000999",
        "channel_id": "999",
        "guild_id": "100",
        "author": {
            "id": "222222222222222222",
            "username": "NormalUser",
            "discriminator": "0000",
            "avatar": null,
            "bot": false,
        },
        "content": "Message in 404 channel",
        "timestamp": "2026-11-08T07:00:00Z",
        "tts": false,
        "mention_everyone": false,
        "mentions": [],
        "mention_roles": [],
        "attachments": [],
        "embeds": [],
        "pinned": false,
        "type": 0,
    }))
    .unwrap();
    handler.message(ctx.clone(), missing_msg).await;

    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    let recorded = mock.recorded_calls.lock().await.clone();

    // Verify 0 deliveries to category 401
    assert!(
        recorded
            .iter()
            .all(|(m, p)| !(*m == Method::POST && p.contains("/channels/401/messages"))),
        "Category channel must NOT receive sticky deliveries"
    );

    // Verify 0 deliveries to foreign channel 201
    assert!(
        recorded
            .iter()
            .all(|(m, p)| !(*m == Method::POST && p.contains("/channels/201/messages"))),
        "Cross-guild channel must NOT receive sticky deliveries"
    );

    // Verify 0 deliveries to missing channel 999
    assert!(
        recorded
            .iter()
            .all(|(m, p)| !(*m == Method::POST && p.contains("/channels/999/messages"))),
        "404 channel must NOT receive sticky deliveries"
    );

    // Verify none of the rejected channels have confirmed delivery state
    let cat_state = state
        .storage
        .get_sticky_message("100", "401")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cat_state.last_posted_message_id, None);

    let cross_state = state
        .storage
        .get_sticky_message("100", "201")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cross_state.last_posted_message_id, None);

    let missing_state = state
        .storage
        .get_sticky_message("100", "999")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(missing_state.last_posted_message_id, None);

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_sticky_coordinator_cold_burst_concurrency_issue_318() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state_with_port(mock.port).await;

    // 1. Configure sticky message on channel 101 in guild 100 with cooldown 5s
    state
        .storage
        .set_sticky_message(
            "100",
            "101",
            "",
            "📌 Important Server Rules: Cold Burst Test!",
            5,
        )
        .await
        .expect("set sticky message");

    // Cold burst: 50 calls to enqueue_refresh without ANY await between them
    for i in 1..=50 {
        state.sticky_coordinator.enqueue_refresh(
            ctx.http.clone(),
            GuildId::new(100),
            ChannelId::new(101),
            MessageId::new(1000 + i),
        );
    }

    // Wait for the single channel worker to complete delivery
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // Exactly 1 worker handle should be registered for channel 101
    assert_eq!(
        state.sticky_coordinator.active_worker_count(),
        1,
        "Exactly 1 active worker must exist for channel 101 despite 50 cold burst calls"
    );

    // Verify exactly 1 POST /channels/101/messages was dispatched
    let recorded = mock.recorded_calls.lock().await.clone();
    let channel_101_posts: Vec<_> = recorded
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(
        channel_101_posts.len(),
        1,
        "Exactly 1 sticky message should be posted to channel 101 for the cold burst"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_sticky_coordinator_barrier_multitask_spawn_concurrency_issue_318() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state_with_port(mock.port).await;

    state
        .storage
        .set_sticky_message(
            "100",
            "101",
            "",
            "📌 Important Server Rules: Multitask Barrier Test!",
            5,
        )
        .await
        .expect("set sticky message");

    let num_tasks = 40;
    let barrier = Arc::new(tokio::sync::Barrier::new(num_tasks));
    let mut join_set = tokio::task::JoinSet::new();

    for i in 0..num_tasks {
        let b = barrier.clone();
        let coordinator = state.sticky_coordinator.clone();
        let http = ctx.http.clone();
        join_set.spawn(async move {
            b.wait().await;
            coordinator.enqueue_refresh(
                http,
                GuildId::new(100),
                ChannelId::new(101),
                MessageId::new(5000 + i as u64),
            );
        });
    }

    while let Some(res) = join_set.join_next().await {
        res.expect("task join");
    }

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    assert_eq!(
        state.sticky_coordinator.active_worker_count(),
        1,
        "Barrier cold-start burst must produce exactly 1 active worker"
    );

    let recorded = mock.recorded_calls.lock().await.clone();
    let channel_101_posts: Vec<_> = recorded
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(
        channel_101_posts.len(),
        1,
        "Exactly 1 sticky message must be dispatched from barrier burst"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_sticky_coordinator_unconfigured_channel_cleans_up_without_posts_issue_318() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state_with_port(mock.port).await;

    // Do NOT set sticky message for channel 102.
    // Dispatch burst of 20 refreshes.
    for i in 1..=20 {
        state.sticky_coordinator.enqueue_refresh(
            ctx.http.clone(),
            GuildId::new(100),
            ChannelId::new(102),
            MessageId::new(7000 + i),
        );
    }

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // Worker should have detected no sticky message configured and removed itself
    assert_eq!(
        state.sticky_coordinator.active_worker_count(),
        0,
        "Unconfigured channel worker should clean itself up via remove_if"
    );

    let recorded = mock.recorded_calls.lock().await.clone();
    let modifying_calls: Vec<_> = recorded
        .iter()
        .filter(|(m, _)| *m == Method::POST || *m == Method::DELETE || *m == Method::PATCH)
        .collect();
    assert_eq!(
        modifying_calls.len(),
        0,
        "Unconfigured channel refresh must make 0 modifying calls"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_sticky_optimistic_concurrency_cas_and_aba_memory_store_issue_319() {
    let state = create_test_state().await;

    // 1. Configure Sticky A
    let cfg_a = state
        .storage
        .set_sticky_message("100", "101", "", "Sticky Content A", 10)
        .await
        .unwrap();
    assert_eq!(cfg_a.generation, 0);

    // 2. In-flight simulation: worker fetched cfg_a. Meanwhile admin updates to cfg_b
    let cfg_b = state
        .storage
        .set_sticky_message("100", "101", "", "Sticky Content B", 30)
        .await
        .unwrap();
    assert_eq!(cfg_b.generation, 1);
    assert_ne!(cfg_a.config_revision, cfg_b.config_revision);

    // 3. Worker tries to commit delivery of A with stale generation and stale revision
    let stale_commit = state
        .storage
        .update_sticky_posted_message(
            "100",
            "101",
            "stale_msg_a",
            chrono::Utc::now(),
            cfg_a.generation,
            cfg_a.config_revision,
        )
        .await
        .unwrap();
    assert!(
        stale_commit.is_none(),
        "Stale delivery of A must be rejected by CAS"
    );

    // Verify B's state was not altered
    let cur_b = state
        .storage
        .get_sticky_message("100", "101")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cur_b.content, "Sticky Content B");
    assert_eq!(cur_b.generation, 1);
    assert_eq!(cur_b.last_posted_message_id, None);

    // 4. Worker delivers B with correct generation and revision
    let b_commit = state
        .storage
        .update_sticky_posted_message(
            "100",
            "101",
            "msg_b_101",
            chrono::Utc::now(),
            cfg_b.generation,
            cfg_b.config_revision,
        )
        .await
        .unwrap();
    assert!(b_commit.is_some(), "Valid delivery of B must succeed");
    let b_updated = b_commit.unwrap();
    assert_eq!(
        b_updated.last_posted_message_id.as_deref(),
        Some("msg_b_101")
    );
    assert_eq!(b_updated.generation, 2);

    // 5. ABA scenario: Delete B, then recreate C on the same channel
    let deleted = state
        .storage
        .delete_sticky_message("100", "101")
        .await
        .unwrap();
    assert!(deleted);

    let cfg_c = state
        .storage
        .set_sticky_message("100", "101", "", "Sticky Content C", 15)
        .await
        .unwrap();
    assert_eq!(cfg_c.generation, 0);
    assert_ne!(cfg_c.config_revision, cfg_b.config_revision);

    // Old delivery of B attempt must be rejected by ABA protection (mismatched config_revision)
    let aba_attempt = state
        .storage
        .update_sticky_posted_message(
            "100",
            "101",
            "ghost_b_msg",
            chrono::Utc::now(),
            cfg_b.generation,
            cfg_b.config_revision,
        )
        .await
        .unwrap();
    assert!(
        aba_attempt.is_none(),
        "Old incarnation of B must fail ABA protection check"
    );
}

#[tokio::test]
async fn test_sticky_superseded_in_flight_delivery_compensating_delete_and_refresh_issue_319() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state_with_port(mock.port).await;

    // 1. Initial config A
    state
        .storage
        .set_sticky_message("100", "101", "", "Config A rules", 5)
        .await
        .unwrap();

    // 2. Deliver message 1 for config A
    state.sticky_coordinator.enqueue_refresh(
        ctx.http.clone(),
        GuildId::new(100),
        ChannelId::new(101),
        MessageId::new(1001),
    );
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    let sticky_after_a = state
        .storage
        .get_sticky_message("100", "101")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        sticky_after_a.last_posted_message_id.as_deref(),
        Some("900000000000000001")
    );

    // 3. Admin updates configuration to B (generation increments, config_revision changes, last_bumped_at resets to None)
    let config_b = state
        .storage
        .set_sticky_message(
            "100",
            "101",
            "900000000000000001",
            "Config B updated rules",
            5,
        )
        .await
        .unwrap();
    assert_eq!(config_b.generation, sticky_after_a.generation + 1);
    assert_ne!(config_b.config_revision, sticky_after_a.config_revision);

    // 4. Member sends new message triggering refresh of config B
    state.sticky_coordinator.enqueue_refresh(
        ctx.http.clone(),
        GuildId::new(100),
        ChannelId::new(101),
        MessageId::new(1002),
    );
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    let sticky_after_b = state
        .storage
        .get_sticky_message("100", "101")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        sticky_after_b.last_posted_message_id.as_deref(),
        Some("900000000000000002")
    );
    assert_eq!(sticky_after_b.content, "Config B updated rules");
    assert_eq!(sticky_after_b.config_revision, config_b.config_revision);

    // Verify Discord mock calls:
    // Message A posted ("900000000000000001"), then Message B posted ("900000000000000002"),
    // and Message A deleted via DELETE /channels/101/messages/900000000000000001
    let recorded = mock.recorded_calls.lock().await.clone();
    let posts: Vec<_> = recorded
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(posts.len(), 2, "Both A and B were posted");
    assert!(posts[0].1.contains("Config A rules"));
    assert!(posts[1].1.contains("Config B updated rules"));

    let deletes: Vec<_> = recorded
        .iter()
        .filter(|(m, p)| {
            *m == Method::DELETE && p.contains("/channels/101/messages/900000000000000001")
        })
        .collect();
    assert_eq!(
        deletes.len(),
        1,
        "Old sticky A was deleted after B delivery"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_sticky_cooldown_deferred_bump_after_silence_issue_321() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state_with_port(mock.port).await;

    // 1. Configure sticky message on channel 101 with 1 second cooldown
    state
        .storage
        .set_sticky_message(
            "100",
            "101",
            "",
            "📌 Rules: Deferred Bump Cooldown Test!",
            1,
        )
        .await
        .expect("set sticky message");

    // 2. Initial message triggers first delivery
    state.sticky_coordinator.enqueue_refresh(
        ctx.http.clone(),
        GuildId::new(100),
        ChannelId::new(101),
        MessageId::new(1001),
    );

    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    // Verify first post delivered
    let recorded = mock.recorded_calls.lock().await.clone();
    let posts_1: Vec<_> = recorded
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(posts_1.len(), 1, "First sticky message posted");

    // 3. Burst of 10 messages sent during the 1-second cooldown window
    for i in 2..=11 {
        state.sticky_coordinator.enqueue_refresh(
            ctx.http.clone(),
            GuildId::new(100),
            ChannelId::new(101),
            MessageId::new(1000 + i),
        );
    }

    // Wait 200ms - still inside the 1-second cooldown window
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Verify: still exactly 1 post during cooldown
    let recorded_cooldown = mock.recorded_calls.lock().await.clone();
    let posts_cd: Vec<_> = recorded_cooldown
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(
        posts_cd.len(),
        1,
        "Zero new deliveries while inside cooldown window"
    );

    // 4. Channel goes completely SILENT! No new messages.
    // Wait for the 1-second cooldown deadline to expire (sleep 1.1s total)
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    // Verify: deferred bump automatically posted after cooldown without any new message!
    let recorded_after_deadline = mock.recorded_calls.lock().await.clone();
    let posts_deferred: Vec<_> = recorded_after_deadline
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(
        posts_deferred.len(),
        2,
        "Deferred bump must execute automatically after cooldown without new messages"
    );

    // Verify old sticky was deleted
    let deletes: Vec<_> = recorded_after_deadline
        .iter()
        .filter(|(m, p)| {
            *m == Method::DELETE && p.contains("/channels/101/messages/900000000000000001")
        })
        .collect();
    assert_eq!(deletes.len(), 1, "Old sticky was deleted by deferred bump");

    // 5. Subsequent silence causes 0 additional posts
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let recorded_final = mock.recorded_calls.lock().await.clone();
    let posts_final: Vec<_> = recorded_final
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(
        posts_final.len(),
        2,
        "No additional posts without new user messages"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_sticky_negative_caching_and_invalidation_issue_322() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state_with_port(mock.port).await;

    // 1. Channel 101 on Guild 100 has NO sticky message configured initially
    assert_eq!(state.sticky_coordinator.negative_cache_len(), 0);
    assert_eq!(state.sticky_coordinator.active_worker_count(), 0);

    // 2. First message triggers worker spawn, checks storage, finds None, records negative cache
    state.sticky_coordinator.enqueue_refresh(
        ctx.http.clone(),
        GuildId::new(100),
        ChannelId::new(101),
        MessageId::new(5001),
    );

    // Give worker time to query storage and exit
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    assert_eq!(
        state.sticky_coordinator.negative_cache_len(),
        1,
        "Channel 101 should now be in negative cache"
    );
    assert_eq!(
        state.sticky_coordinator.active_worker_count(),
        0,
        "Worker should have terminated after noticing unconfigured channel"
    );

    // 3. Flood of 100 subsequent messages should be dropped on fast-path without spawning workers
    for i in 2..=101 {
        state.sticky_coordinator.enqueue_refresh(
            ctx.http.clone(),
            GuildId::new(100),
            ChannelId::new(101),
            MessageId::new(5000 + i),
        );
    }

    assert_eq!(
        state.sticky_coordinator.active_worker_count(),
        0,
        "No workers spawned for negative cache hits"
    );
    assert_eq!(
        state.sticky_coordinator.negative_cache_len(),
        1,
        "Negative cache entry remains active"
    );

    let recorded = mock.recorded_calls.lock().await.clone();
    let posts: Vec<_> = recorded
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(posts.len(), 0, "No messages posted in unconfigured channel");

    // 4. Configure sticky message and invalidate cache
    state
        .storage
        .set_sticky_message(
            "100",
            "101",
            "",
            "📌 Rules: Activated After Negative Cache Invalidation!",
            1,
        )
        .await
        .expect("set sticky message");

    state.sticky_coordinator.invalidate_channel("100", "101");
    assert_eq!(
        state.sticky_coordinator.negative_cache_len(),
        0,
        "Negative cache entry removed after invalidation"
    );

    // 5. Next message spawns worker and delivers sticky message immediately
    state.sticky_coordinator.enqueue_refresh(
        ctx.http.clone(),
        GuildId::new(100),
        ChannelId::new(101),
        MessageId::new(6001),
    );

    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_sticky_race_worker_stale_ok_none_does_not_clobber_invalidation_issue_323() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state_with_port(mock.port).await;

    // 1. Channel 101 on Guild 100 has NO sticky message configured initially
    assert_eq!(state.sticky_coordinator.negative_cache_len(), 0);

    // 2. Setup deterministic pause hook: freeze worker right after receiving Ok(None)
    let pause = rusty_discord_bot::discord::sticky::StickyTestPause {
        worker_reached_ok_none: Arc::new(tokio::sync::Notify::new()),
        allow_worker_to_proceed: Arc::new(tokio::sync::Notify::new()),
        ..Default::default()
    };
    state.sticky_coordinator.set_test_pause(pause.clone());

    // 3. Trigger worker via user message in unconfigured channel
    state.sticky_coordinator.enqueue_refresh(
        ctx.http.clone(),
        GuildId::new(100),
        ChannelId::new(101),
        MessageId::new(7001),
    );

    // 4. Wait deterministically for worker to query DB, receive Ok(None), and pause
    pause.worker_reached_ok_none.notified().await;

    // 5. While worker is paused, admin configures sticky message and invalidates cache!
    state
        .storage
        .set_sticky_message(
            "100",
            "101",
            "",
            "📌 Rules: Configured Concurrently With Stale Worker!",
            1,
        )
        .await
        .expect("set sticky message concurrently");

    state.sticky_coordinator.invalidate_channel("100", "101");

    // 6. Release the stale worker; it must NOT clobber the invalidation with MissingUntil!
    pause.allow_worker_to_proceed.notify_one();

    // Give worker time to complete exit
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Negative cache length must be 0 because the channel was invalidated, not marked missing
    assert_eq!(
        state.sticky_coordinator.negative_cache_len(),
        0,
        "Channel 101 must not be marked as missing after invalidation"
    );

    // 7. Next user message arrives: coordinator MUST spawn worker and deliver sticky message immediately!
    state.sticky_coordinator.clear_test_pause();
    state.sticky_coordinator.enqueue_refresh(
        ctx.http.clone(),
        GuildId::new(100),
        ChannelId::new(101),
        MessageId::new(7002),
    );

    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    // 8. Verify sticky message was posted with zero delay / no 60s lockout
    let recorded = mock.recorded_calls.lock().await.clone();
    let posts: Vec<_> = recorded
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(
        posts.len(),
        1,
        "Sticky message posted immediately without 60-second negative cache lockout"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_sticky_reverse_order_invalidation_overwrites_existing_missing_entry_issue_323() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state_with_port(mock.port).await;

    // 1. Channel 101 on Guild 100 has NO sticky message initially
    state.sticky_coordinator.enqueue_refresh(
        ctx.http.clone(),
        GuildId::new(100),
        ChannelId::new(101),
        MessageId::new(8001),
    );

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Verify channel is in negative cache
    assert_eq!(
        state.sticky_coordinator.negative_cache_len(),
        1,
        "Channel 101 is cached as missing"
    );

    // 2. Admin configures sticky message and invalidates cache
    state
        .storage
        .set_sticky_message(
            "100",
            "101",
            "",
            "📌 Rules: Configured After MissingUntil!",
            1,
        )
        .await
        .expect("set sticky message");

    state.sticky_coordinator.invalidate_channel("100", "101");

    assert_eq!(
        state.sticky_coordinator.negative_cache_len(),
        0,
        "MissingUntil entry overwritten by invalidation"
    );

    // 3. Next message immediately delivers sticky
    state.sticky_coordinator.enqueue_refresh(
        ctx.http.clone(),
        GuildId::new(100),
        ChannelId::new(101),
        MessageId::new(8002),
    );

    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    let recorded = mock.recorded_calls.lock().await.clone();
    let posts: Vec<_> = recorded
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(
        posts.len(),
        1,
        "Sticky message delivered immediately on next message after invalidation"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_sticky_cooldown_avoids_repeated_storage_selects_issue_324() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state_with_port(mock.port).await;

    // 1. Configure sticky message on channel 101 with 1 second cooldown
    state
        .storage
        .set_sticky_message(
            "100",
            "101",
            "",
            "📌 Rules: O(1) Storage Reads During Cooldown!",
            1,
        )
        .await
        .expect("set sticky message");

    // Initial message triggers first delivery
    state.sticky_coordinator.enqueue_refresh(
        ctx.http.clone(),
        GuildId::new(100),
        ChannelId::new(101),
        MessageId::new(9001),
    );

    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    // Verify first post delivered
    let recorded = mock.recorded_calls.lock().await.clone();
    let posts_1: Vec<_> = recorded
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(posts_1.len(), 1, "First sticky message posted");

    let initial_fetch_count = state.sticky_coordinator.storage_fetch_count();
    assert!(
        initial_fetch_count >= 1,
        "Initial delivery fetched config from storage"
    );

    // 2. Send 1 message into cooldown: this triggers exactly 1 storage fetch to verify cooldown
    state.sticky_coordinator.enqueue_refresh(
        ctx.http.clone(),
        GuildId::new(100),
        ChannelId::new(101),
        MessageId::new(9002),
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let cooldown_fetch_count = state.sticky_coordinator.storage_fetch_count();
    assert_eq!(
        cooldown_fetch_count,
        initial_fetch_count + 1,
        "Exactly 1 storage fetch to evaluate cooldown and start sleep"
    );

    // Now flood 100 messages into the channel while worker is sleeping in cooldown
    for i in 3..=102 {
        state.sticky_coordinator.enqueue_refresh(
            ctx.http.clone(),
            GuildId::new(100),
            ChannelId::new(101),
            MessageId::new(9000 + i),
        );
    }

    // Wait 200ms - still inside the 1-second cooldown window
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Verify: exactly 1 post during cooldown (no premature deliveries)
    let recorded_cooldown = mock.recorded_calls.lock().await.clone();
    let posts_cd: Vec<_> = recorded_cooldown
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(
        posts_cd.len(),
        1,
        "Zero new deliveries while inside cooldown window"
    );

    // CRITICAL ASSERTION FOR ISSUE #324:
    // Storage fetches MUST NOT grow with incoming messages in cooldown!
    // The worker coalesces 100 triggers in-memory without querying storage on each message.
    let burst_fetch_count = state.sticky_coordinator.storage_fetch_count();
    assert_eq!(
        burst_fetch_count, cooldown_fetch_count,
        "Zero additional storage fetches for 100 messages while worker sleeps in cooldown"
    );

    // 3. Silence: wait for cooldown deadline to expire (sleep 1.1s)
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    // Verify: deferred bump delivered after cooldown
    let recorded_after = mock.recorded_calls.lock().await.clone();
    let posts_after: Vec<_> = recorded_after
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(
        posts_after.len(),
        2,
        "Deferred bump delivered after cooldown expiry"
    );

    // Exactly 1 additional storage fetch at deadline expiry to check CAS & deliver
    let final_fetch_count = state.sticky_coordinator.storage_fetch_count();
    assert_eq!(
        final_fetch_count,
        cooldown_fetch_count + 1,
        "Exactly 1 storage fetch at cooldown deadline (O(1) total reads, not 100)"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_sticky_cooldown_interrupted_by_web_config_mutation_or_deletion_issue_324() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state_with_port(mock.port).await;

    // 1. Configure sticky message on channel 101 with long cooldown (3600 seconds)
    state
        .storage
        .set_sticky_message("100", "101", "", "📌 Rules: Long 1-Hour Cooldown!", 3600)
        .await
        .expect("set sticky message");

    // Initial message triggers first delivery
    state.sticky_coordinator.enqueue_refresh(
        ctx.http.clone(),
        GuildId::new(100),
        ChannelId::new(101),
        MessageId::new(9501),
    );

    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    // Next message puts worker into 1-hour sleep
    state.sticky_coordinator.enqueue_refresh(
        ctx.http.clone(),
        GuildId::new(100),
        ChannelId::new(101),
        MessageId::new(9502),
    );

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(state.sticky_coordinator.active_worker_count(), 1);

    // 2. While worker is sleeping in 1-hour cooldown, admin updates config via web:
    // changes cooldown to 0 (immediate) and new content!
    state
        .storage
        .set_sticky_message(
            "100",
            "101",
            "",
            "📌 Rules: Updated Web Config Should Wake Worker Immediately!",
            0,
        )
        .await
        .expect("set sticky message");

    // invalidate_channel sends ConfigChanged signal to sleeping worker
    state.sticky_coordinator.invalidate_channel("100", "101");

    // Worker must wake up immediately without waiting 3600s and deliver updated config!
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let recorded = mock.recorded_calls.lock().await.clone();
    let posts: Vec<_> = recorded
        .iter()
        .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
        .collect();
    assert_eq!(
        posts.len(),
        2,
        "Worker woke up immediately on ConfigChanged and delivered updated sticky"
    );
    assert!(posts[1].1.contains("Updated Web Config"));

    // 3. Now send another message and then DELETE sticky while worker is sleeping
    state.sticky_coordinator.enqueue_refresh(
        ctx.http.clone(),
        GuildId::new(100),
        ChannelId::new(101),
        MessageId::new(9503),
    );

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    state
        .storage
        .delete_sticky_message("100", "101")
        .await
        .expect("delete sticky");

    state.sticky_coordinator.remove_channel("100", "101");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    assert_eq!(
        state.sticky_coordinator.active_worker_count(),
        0,
        "Worker terminated cleanly on deletion signal without leaks"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_sticky_config_change_and_deletion_not_lost_when_mpsc_queue_full_issue_325() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state_with_port(mock.port).await;

    // 1. Configure sticky message on channel 101 with long cooldown (3600 seconds)
    state
        .storage
        .set_sticky_message(
            "100",
            "101",
            "",
            "📌 Rules: Long 1-Hour Cooldown Before Queue Saturation!",
            3600,
        )
        .await
        .expect("set sticky message");

    // 2. Setup deterministic pause hook to reliably catch worker entering cooldown
    let pause = rusty_discord_bot::discord::sticky::StickyTestPause {
        worker_entered_cooldown: Arc::new(tokio::sync::Notify::new()),
        ..Default::default()
    };
    state.sticky_coordinator.set_test_pause(pause.clone());

    // 3. Trigger initial sticky message delivery
    state.sticky_coordinator.enqueue_refresh(
        ctx.http.clone(),
        GuildId::new(100),
        ChannelId::new(101),
        MessageId::new(8201),
    );

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    {
        let recorded = mock.recorded_calls.lock().await;
        let posts: Vec<_> = recorded
            .iter()
            .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
            .collect();
        assert_eq!(posts.len(), 1, "Initial sticky message delivered");
    }

    // 4. Send second message to enter cooldown
    let cooldown_notified = pause.worker_entered_cooldown.notified();
    state.sticky_coordinator.enqueue_refresh(
        ctx.http.clone(),
        GuildId::new(100),
        ChannelId::new(101),
        MessageId::new(8202),
    );

    // Wait until worker is deterministically inside cooldown sleep
    cooldown_notified.await;
    assert_eq!(state.sticky_coordinator.active_worker_count(), 1);

    // 5. Flood the channel with 70 triggers to completely saturate mpsc buffer (capacity 64)
    for i in 0..70 {
        state.sticky_coordinator.enqueue_refresh(
            ctx.http.clone(),
            GuildId::new(100),
            ChannelId::new(101),
            MessageId::new(8210 + i),
        );
    }

    // 6. While mpsc queue is 100% full, admin updates config via web:
    // updates content and sets new config revision while retaining cooldown.
    let re_entered_cooldown = pause.worker_entered_cooldown.notified();
    state
        .storage
        .set_sticky_message(
            "100",
            "101",
            "",
            "📌 Rules: Updated Immediately Despite Full MPSC Queue!",
            3600,
        )
        .await
        .expect("set sticky message");

    // Invalidate channel: watch channel must deliver notification even if mpsc is full
    state.sticky_coordinator.invalidate_channel("100", "101");

    // Worker must wake up immediately without waiting 3600s and re-enter cooldown after delivery
    re_entered_cooldown.await;

    {
        let recorded = mock.recorded_calls.lock().await;
        let posts: Vec<_> = recorded
            .iter()
            .filter(|(m, p)| *m == Method::POST && p.contains("/channels/101/messages"))
            .collect();
        assert_eq!(
            posts.len(),
            2,
            "Worker woke up immediately despite full mpsc queue and delivered updated sticky"
        );
        assert!(
            posts[1]
                .1
                .contains("Updated Immediately Despite Full MPSC Queue")
        );
    }

    // 7. Verify deletion while worker is in cooldown with a fully saturated mpsc queue
    // Flood mpsc queue again with 70 triggers
    for i in 0..70 {
        state.sticky_coordinator.enqueue_refresh(
            ctx.http.clone(),
            GuildId::new(100),
            ChannelId::new(101),
            MessageId::new(8310 + i),
        );
    }

    // Admin deletes sticky while mpsc is completely full
    state
        .storage
        .delete_sticky_message("100", "101")
        .await
        .expect("delete sticky");

    state.sticky_coordinator.remove_channel("100", "101");

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    assert_eq!(
        state.sticky_coordinator.active_worker_count(),
        0,
        "Worker terminated cleanly on deletion signal via watch channel even with full mpsc queue"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_sticky_negative_cache_sweep_throttling_and_hard_cap_issue_327() {
    let (ctx, mock, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state_with_port(mock.port).await;

    // 1. Configure a valid sticky message on channel 101 so triggers proceed past fast-path
    state
        .storage
        .set_sticky_message(
            "100",
            "101",
            "",
            "📌 Rules: Test Sticky Message for Cache Sweep Throttling",
            3600,
        )
        .await
        .expect("set sticky message");

    // 2. Pre-populate negative cache with 4200 fresh entries (> 4096 threshold)
    let now = std::time::Instant::now();
    for i in 1..=4200 {
        state.sticky_coordinator.insert_negative_cache_for_test(
            GuildId::new(200),
            ChannelId::new(20000 + i),
            rusty_discord_bot::discord::sticky::StickyCacheEntry::MissingUntil(
                now + std::time::Duration::from_secs(60),
            ),
        );
    }
    assert_eq!(state.sticky_coordinator.negative_cache_raw_len(), 4200);
    assert_eq!(state.sticky_coordinator.negative_cache_sweeps_total(), 0);

    // 3. First trigger should run a sweep because last_sweep was initialized > 5s ago
    state.sticky_coordinator.enqueue_refresh(
        ctx.http.clone(),
        GuildId::new(100),
        ChannelId::new(101),
        MessageId::new(9901),
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(
        state.sticky_coordinator.negative_cache_sweeps_total(),
        1,
        "First trigger ran initial sweep"
    );

    // 4. Send 150 consecutive triggers in the same 5-second window
    for i in 0..150 {
        state.sticky_coordinator.enqueue_refresh(
            ctx.http.clone(),
            GuildId::new(100),
            ChannelId::new(101),
            MessageId::new(9902 + i),
        );
    }

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(
        state.sticky_coordinator.negative_cache_sweeps_total(),
        1,
        "Sweeps total must remain 1: all 150 triggers in 5s window are rate-limited in O(1)"
    );

    // 5. Test hard cap eviction: populate to 9000 entries (exceeding 8192 hard cap)
    // Include an InvalidatedUntil entry to ensure it is NEVER evicted by hard cap
    let protected_key = (GuildId::new(999), ChannelId::new(888));
    state.sticky_coordinator.insert_negative_cache_for_test(
        protected_key.0,
        protected_key.1,
        rusty_discord_bot::discord::sticky::StickyCacheEntry::InvalidatedUntil(
            now + std::time::Duration::from_secs(60),
        ),
    );

    for i in 4201..=9000 {
        state.sticky_coordinator.insert_negative_cache_for_test(
            GuildId::new(300),
            ChannelId::new(30000 + i),
            rusty_discord_bot::discord::sticky::StickyCacheEntry::MissingUntil(
                now + std::time::Duration::from_secs(60),
            ),
        );
    }
    assert!(state.sticky_coordinator.negative_cache_raw_len() >= 9000);

    // Trigger after sweeping interval expires or when rate limit allows
    // Wait for 5 seconds rate limit window to pass
    tokio::time::sleep(std::time::Duration::from_millis(5100)).await;

    state.sticky_coordinator.enqueue_refresh(
        ctx.http.clone(),
        GuildId::new(100),
        ChannelId::new(101),
        MessageId::new(10001),
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    assert_eq!(
        state.sticky_coordinator.negative_cache_sweeps_total(),
        2,
        "Sweep executed after 5s interval elapsed"
    );
    assert!(
        state.sticky_coordinator.negative_cache_raw_len() < 9000,
        "Hard cap evicted excess entries"
    );

    // Verify InvalidatedUntil entry was not evicted by hard cap
    assert_eq!(
        state.sticky_coordinator.negative_cache_len(),
        state.sticky_coordinator.negative_cache_raw_len() - 1,
        "Protected InvalidatedUntil entry is preserved"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_progression_award_message_xp_defense_in_depth_issue_351() {
    let (ctx, _mock_state, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state().await;

    let guild_id = GuildId::new(100);
    let channel_id = ChannelId::new(101);
    let actor_id = UserId::new(222222222222222222);

    // 1. AwardMessageXp with privileged role reward 1003 (SubordinateAdminRole with ADMINISTRATOR)
    let execution = rusty_discord_bot::discord::progression::ProgressionExecutionContext::new(
        Some(guild_id),
        Some(channel_id),
        Some(actor_id),
        serenity::all::Permissions::empty(),
        false, // event context
    );

    let op_privileged = rusty_discord_bot::lua::ProgressionOperation::AwardMessageXp {
        amount: 50,
        cooldown_seconds: 0,
        announce_level_up: false,
        level_up_message: None,
        role_rewards: vec![rusty_discord_bot::lua::LevelRoleReward {
            level: 1,
            role_id: "1003".to_string(), // ADMINISTRATOR role
        }],
    };

    let res = rusty_discord_bot::discord::progression::execute(
        &ctx,
        &state,
        "progression",
        execution,
        &op_privileged,
    )
    .await;
    assert!(res.is_ok(), "AwardMessageXp should execute and award XP");

    // Privileged role must NOT be recorded as granted in storage
    let granted_privileged = state
        .storage
        .is_role_reward_granted("100", "222222222222222222", "1003")
        .await
        .unwrap_or(false);
    assert!(
        !granted_privileged,
        "Privileged role 1003 must be denied by defense-in-depth and not recorded as granted"
    );

    // XP must still be granted
    let profile = state
        .storage
        .get_member_progress("100", "222222222222222222")
        .await
        .unwrap();
    assert_eq!(profile.xp, 50, "XP should still be awarded to user");

    // 2. AwardMessageXp with legitimate unprivileged role reward 1002 (MemberRole)
    let op_legit = rusty_discord_bot::lua::ProgressionOperation::AwardMessageXp {
        amount: 50,
        cooldown_seconds: 0,
        announce_level_up: false,
        level_up_message: None,
        role_rewards: vec![rusty_discord_bot::lua::LevelRoleReward {
            level: 1,
            role_id: "1002".to_string(), // Unprivileged MemberRole
        }],
    };

    let res_legit = rusty_discord_bot::discord::progression::execute(
        &ctx,
        &state,
        "progression",
        execution,
        &op_legit,
    )
    .await;
    assert!(res_legit.is_ok());

    let granted_legit = state
        .storage
        .is_role_reward_granted("100", "222222222222222222", "1002")
        .await
        .unwrap_or(false);
    assert!(
        granted_legit,
        "Legitimate role 1002 must be safely granted and recorded"
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_progression_role_rewards_o1_amplification_and_cooldown_issue_365() {
    let (ctx, mock_state, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state().await;

    let guild_id = GuildId::new(100);
    let channel_id = ChannelId::new(101);
    let actor_id = UserId::new(333333333333333333);

    let execution = rusty_discord_bot::discord::progression::ProgressionExecutionContext::new(
        Some(guild_id),
        Some(channel_id),
        Some(actor_id),
        serenity::all::Permissions::empty(),
        false, // event context
    );

    // Create 50 role rewards for level 1: 1 legitimate role (1002), 1 privileged role (1003),
    // and 48 fictitious roles (2000..2048)
    let mut role_rewards = Vec::with_capacity(50);
    role_rewards.push(rusty_discord_bot::lua::LevelRoleReward {
        level: 1,
        role_id: "1002".to_string(), // Unprivileged MemberRole
    });
    role_rewards.push(rusty_discord_bot::lua::LevelRoleReward {
        level: 1,
        role_id: "1003".to_string(), // ADMINISTRATOR privileged role
    });
    for i in 2000..2048 {
        role_rewards.push(rusty_discord_bot::lua::LevelRoleReward {
            level: 1,
            role_id: i.to_string(),
        });
    }

    // Clear recorded HTTP calls
    mock_state.recorded_calls.lock().await.clear();

    // 1. First event: XP is awarded (cooldown_seconds = 60), leveling up to level 1
    let op_award = rusty_discord_bot::lua::ProgressionOperation::AwardMessageXp {
        amount: 100,
        cooldown_seconds: 60,
        announce_level_up: false,
        level_up_message: None,
        role_rewards: role_rewards.clone(),
    };

    let res1 = rusty_discord_bot::discord::progression::execute(
        &ctx,
        &state,
        "progression",
        execution,
        &op_award,
    )
    .await;
    assert!(res1.is_ok(), "First AwardMessageXp should succeed");

    // Count GET calls to guild and bot member
    let calls = mock_state.recorded_calls.lock().await.clone();
    let guild_get_calls = calls
        .iter()
        .filter(|(m, p)| *m == axum::http::Method::GET && p.starts_with("/api/v10/guilds/100|"))
        .count();
    let member_get_calls = calls
        .iter()
        .filter(|(m, p)| {
            *m == axum::http::Method::GET && p.starts_with("/api/v10/guilds/100/members/")
        })
        .count();

    // MUST be O(1), at most 1 guild GET and 1 bot member GET across all 50 roles (NOT 50 or 100!)
    assert!(
        guild_get_calls <= 1,
        "Guild hierarchy snapshot must be fetched at most once (was {guild_get_calls})"
    );
    assert!(
        member_get_calls <= 1,
        "Bot member snapshot must be fetched at most once (was {member_get_calls})"
    );

    // Legitimate role 1002 was granted, privileged role 1003 was denied
    let granted_1002 = state
        .storage
        .is_role_reward_granted("100", "333333333333333333", "1002")
        .await
        .unwrap();
    assert!(granted_1002, "Role 1002 must be granted");

    let granted_1003 = state
        .storage
        .is_role_reward_granted("100", "333333333333333333", "1003")
        .await
        .unwrap();
    assert!(!granted_1003, "Privileged role 1003 must be denied");

    // 2. Second event immediately after: user is on cooldown (award.awarded == 0)
    mock_state.recorded_calls.lock().await.clear();

    let res2 = rusty_discord_bot::discord::progression::execute(
        &ctx,
        &state,
        "progression",
        execution,
        &op_award,
    )
    .await;
    assert!(
        res2.is_ok(),
        "Second AwardMessageXp should return Ok during cooldown"
    );

    // During cooldown, ZERO Discord HTTP calls must be made!
    let calls2 = mock_state.recorded_calls.lock().await.clone();
    assert_eq!(
        calls2.len(),
        0,
        "During cooldown (awarded == 0), no Discord HTTP calls should be made (was {:?})",
        calls2
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_ticket_claim_unclaim_rename_and_participant_resilience_issues_361_360_358() {
    let (ctx, _mock_state, shard_manager) = start_mock_discord_environment().await;
    let state = create_test_state().await;

    let guild_id = GuildId::new(100);
    let channel_id = ChannelId::new(101);
    let owner_id = UserId::new(111111111111111111); // Server owner (has ADMINISTRATOR)

    // Reserve and activate a test ticket in channel 101
    let reservation = rusty_discord_bot::tickets::NewTicketReservation {
        guild_id: "100".to_string(),
        creator_user_id: "222222222222222222".to_string(),
        subject: "Pomoc techniczna".to_string(),
        description: "Opis problemu".to_string(),
        queue: "support".to_string(),
    };
    let reserved = state.tickets.reserve_ticket(reservation, 5).await.unwrap();
    let ticket_id = match reserved {
        rusty_discord_bot::tickets::ReserveTicketOutcome::Reserved(t) => t.id,
        _ => panic!("reservation failed"),
    };
    let activated = state
        .tickets
        .activate_ticket(&ticket_id, "101", "ticket-000001-pomoc-techniczna")
        .await
        .unwrap();
    assert!(matches!(
        activated,
        rusty_discord_bot::tickets::TicketMutationOutcome::Updated(_)
    ));

    let execution = rusty_discord_bot::discord::tickets::TicketExecutionContext::new(
        Some(guild_id),
        Some(channel_id),
        Some(owner_id),
        serenity::all::Permissions::all(),
        serenity::all::Permissions::all(),
        true, // command context
    );

    // 1. Claim ticket (Issue #361): even if channel notification or audit fails, claim succeeds in DB
    let claim_op = rusty_discord_bot::lua::TicketOperation::Claim {
        support_role_ids: vec![],
    };
    let claim_res =
        rusty_discord_bot::discord::tickets::execute(&ctx, &state, "tickets", execution, &claim_op)
            .await;
    assert!(claim_res.is_ok(), "Claim execution should succeed");
    let claim_msg = claim_res.unwrap().expect("claim should return message");
    assert!(claim_msg.contains("Claimed ticket"));

    // DB state must be claimed
    let ticket_after_claim = state
        .tickets
        .get_by_channel("100", "101")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        ticket_after_claim.status,
        rusty_discord_bot::tickets::STATUS_CLAIMED
    );
    assert_eq!(
        ticket_after_claim.claimed_by_user_id.as_deref(),
        Some("111111111111111111")
    );

    // 2. Unclaim ticket (Issue #361)
    let unclaim_op = rusty_discord_bot::lua::TicketOperation::Unclaim {
        support_role_ids: vec![],
    };
    let unclaim_res = rusty_discord_bot::discord::tickets::execute(
        &ctx,
        &state,
        "tickets",
        execution,
        &unclaim_op,
    )
    .await;
    assert!(unclaim_res.is_ok(), "Unclaim execution should succeed");
    let unclaim_msg = unclaim_res.unwrap().expect("unclaim should return message");
    assert!(unclaim_msg.contains("Zwolniono ticket"));

    // DB state must be open again
    let ticket_after_unclaim = state
        .tickets
        .get_by_channel("100", "101")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        ticket_after_unclaim.status,
        rusty_discord_bot::tickets::STATUS_OPEN
    );
    assert!(ticket_after_unclaim.claimed_by_user_id.is_none());

    // 3. Rename ticket (Issue #358)
    let rename_op = rusty_discord_bot::lua::TicketOperation::Rename {
        name: "ticket-new-name".to_string(),
        support_role_ids: vec![],
        creator_can_rename: false,
    };
    let rename_res = rusty_discord_bot::discord::tickets::execute(
        &ctx, &state, "tickets", execution, &rename_op,
    )
    .await;
    assert!(
        rename_res.is_ok(),
        "Rename execution failed with: {:?}",
        rename_res.as_ref().err()
    );
    let rename_msg = rename_res.unwrap().expect("rename should return message");
    assert!(rename_msg.contains("Ticket name changed"));

    let ticket_after_rename = state
        .tickets
        .get_by_channel("100", "101")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        ticket_after_rename.channel_name.as_deref(),
        Some("ticket-new-name")
    );

    // 4. Add participant (Issue #360)
    let add_op = rusty_discord_bot::lua::TicketOperation::AddMember {
        user_id: "555555555555555555".to_string(),
        support_role_ids: vec![],
        creator_can_manage_participants: false,
    };
    let add_res =
        rusty_discord_bot::discord::tickets::execute(&ctx, &state, "tickets", execution, &add_op)
            .await;
    assert!(add_res.is_ok(), "Add member should succeed");
    let add_msg = add_res.unwrap().expect("add member message");
    assert!(add_msg.contains("Dodano <@555555555555555555> do ticketu"));

    let participants = state.tickets.list_participants(&ticket_id).await.unwrap();
    assert!(
        participants
            .iter()
            .any(|p| p.user_id == "555555555555555555")
    );

    // 5. Remove participant (Issue #360)
    let rem_op = rusty_discord_bot::lua::TicketOperation::RemoveMember {
        user_id: "555555555555555555".to_string(),
        support_role_ids: vec![],
        creator_can_manage_participants: false,
    };
    let rem_res =
        rusty_discord_bot::discord::tickets::execute(&ctx, &state, "tickets", execution, &rem_op)
            .await;
    assert!(rem_res.is_ok(), "Remove member should succeed");
    let rem_msg = rem_res.unwrap().expect("remove member message");
    assert!(rem_msg.contains("Removed <@555555555555555555> from ticket"));

    let participants_after = state.tickets.list_participants(&ticket_id).await.unwrap();
    assert!(
        !participants_after
            .iter()
            .any(|p| p.user_id == "555555555555555555")
    );

    shard_manager.shutdown_all().await;
}

#[tokio::test]
async fn test_giveaway_web_end_and_reroll_discord_delivery_issue_359() {
    use dashmap::DashMap;
    use reqwest::header::COOKIE;
    use rusty_discord_bot::web::{
        WebState,
        auth::{DashboardGuild, DashboardUser, LoginRateLimiter},
        build_router,
    };

    let (_ctx, mock, shard_manager) = start_mock_discord_environment().await;

    let bind: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    let config = Config {
        discord_token: "test_bot_token".to_string(),
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
        discord_api_base_url: format!("http://127.0.0.1:{}", mock.port),
        transcript_encryption_key: None,
        transcript_active_key_id: "v1".to_string(),
        transcript_keyring: None,
        ticket_transcripts_disabled: true,
        metrics_token: None,
        metrics_allowed_ips: std::collections::HashSet::new(),
    };

    let app = AppState::bootstrap(config)
        .await
        .expect("bootstrap app state");
    let permissions_cache = Arc::new(DashMap::new());
    let web_state = WebState {
        app: app.clone(),
        login_rate_limiter: Arc::new(LoginRateLimiter::new()),
        permissions_cache: permissions_cache.clone(),
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let base_url = format!("http://{addr}");
    let router = build_router(web_state.clone());

    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });

    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    let admin_session = "admin_giveaway_session";
    let csrf_token = "csrf_giveaway_test";
    app.sessions
        .save_session(
            admin_session,
            DashboardUser {
                id: "111111111111111111".to_string(),
                username: "ServerOwner".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: "100".to_string(),
                name: "Test Guild".to_string(),
                icon: None,
                owner: true,
                permissions: "8".to_string(),
            }],
            csrf_token.to_string(),
            chrono::Utc::now() + chrono::Duration::hours(1),
            None,
        )
        .await
        .unwrap();
    let cookie = format!("rusty_discord_bot_session={admin_session}");

    // 1. Create a giveaway with 2 participants in valid channel 101
    let ga = app
        .storage
        .create_giveaway(
            "100",
            "101",
            "Discord Nitro 1 Month",
            1,
            chrono::Utc::now() + chrono::Duration::hours(2),
        )
        .await
        .unwrap();
    app.storage
        .join_giveaway(ga.id, "222222222222222222")
        .await
        .unwrap();
    app.storage
        .join_giveaway(ga.id, "333333333333333333")
        .await
        .unwrap();

    mock.recorded_calls.lock().await.clear();

    // 2. POST /end via Web REST API
    let end_res = client
        .post(format!("{base_url}/api/guilds/100/giveaways/{}/end", ga.id))
        .header(COOKIE, &cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(end_res.status(), reqwest::StatusCode::OK);
    let end_json: serde_json::Value = end_res.json().await.unwrap();
    assert_eq!(end_json["ended"], true);
    assert_eq!(end_json["announcement_status"], "delivered");
    assert!(end_json["message_id"].as_str().is_some());
    let winners = end_json["winners"].as_array().expect("winners array");
    assert_eq!(winners.len(), 1);

    // Verify exactly 1 message POST was sent to channel 101 with announcement
    let calls = mock.recorded_calls.lock().await.clone();
    let msg_posts: Vec<_> = calls
        .iter()
        .filter(|(m, p)| {
            *m == axum::http::Method::POST && p.starts_with("/api/v10/channels/101/messages")
        })
        .collect();
    assert_eq!(
        msg_posts.len(),
        1,
        "Exactly 1 Discord announcement must be sent on /end"
    );
    let post_body = &msg_posts[0].1;
    assert!(
        post_body.contains("KONKURS ZAKO\\u0143CZONY") || post_body.contains("KONKURS ZAKOŃCZONY")
    );
    assert!(post_body.contains("Discord Nitro 1 Month"));

    // 3. Idempotent second POST /end: returns already_ended and sends 0 additional Discord messages
    mock.recorded_calls.lock().await.clear();
    let end_dup_res = client
        .post(format!("{base_url}/api/guilds/100/giveaways/{}/end", ga.id))
        .header(COOKIE, &cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(end_dup_res.status(), reqwest::StatusCode::OK);
    let end_dup_json: serde_json::Value = end_dup_res.json().await.unwrap();
    assert_eq!(end_dup_json["ended"], true);
    assert_eq!(end_dup_json["announcement_status"], "already_ended");
    let calls_after_dup = mock.recorded_calls.lock().await.clone();
    assert_eq!(
        calls_after_dup.len(),
        0,
        "Second /end must NOT send any Discord messages"
    );

    // 4. POST /reroll on active giveaway -> 409 Conflict & 0 messages
    let ga_active = app
        .storage
        .create_giveaway(
            "100",
            "101",
            "Active Prize",
            1,
            chrono::Utc::now() + chrono::Duration::hours(5),
        )
        .await
        .unwrap();
    app.storage
        .join_giveaway(ga_active.id, "222222222222222222")
        .await
        .unwrap();

    let reroll_conflict_res = client
        .post(format!(
            "{base_url}/api/guilds/100/giveaways/{}/reroll",
            ga_active.id
        ))
        .header(COOKIE, &cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(reroll_conflict_res.status(), reqwest::StatusCode::CONFLICT);

    // 5. POST /reroll on concluded giveaway ga.id -> 200 OK & exactly 1 reroll message
    mock.recorded_calls.lock().await.clear();
    let reroll_res = client
        .post(format!(
            "{base_url}/api/guilds/100/giveaways/{}/reroll",
            ga.id
        ))
        .header(COOKIE, &cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(reroll_res.status(), reqwest::StatusCode::OK);
    let reroll_json: serde_json::Value = reroll_res.json().await.unwrap();
    assert_eq!(reroll_json["rerolled"], true);
    assert_eq!(reroll_json["announcement_status"], "delivered");
    assert!(reroll_json["message_id"].as_str().is_some());

    let calls_reroll = mock.recorded_calls.lock().await.clone();
    let reroll_posts: Vec<_> = calls_reroll
        .iter()
        .filter(|(m, p)| {
            *m == axum::http::Method::POST && p.starts_with("/api/v10/channels/101/messages")
        })
        .collect();
    assert_eq!(
        reroll_posts.len(),
        1,
        "Exactly 1 Discord reroll announcement must be sent on /reroll"
    );
    let reroll_body = &reroll_posts[0].1;
    assert!(reroll_body.contains("PONOWNE LOSOWANIE") || reroll_body.contains("REROLL"));

    // 6. Cross-guild channel attempt: channel 201 belongs to foreign guild 200
    let ga_cross = app
        .storage
        .create_giveaway(
            "100",
            "201", // Channel 201 is in guild 200
            "Cross Guild Hack",
            1,
            chrono::Utc::now() + chrono::Duration::hours(2),
        )
        .await
        .unwrap();
    app.storage
        .join_giveaway(ga_cross.id, "222222222222222222")
        .await
        .unwrap();

    mock.recorded_calls.lock().await.clear();
    let cross_end_res = client
        .post(format!(
            "{base_url}/api/guilds/100/giveaways/{}/end",
            ga_cross.id
        ))
        .header(COOKIE, &cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(cross_end_res.status(), reqwest::StatusCode::OK);
    let cross_json: serde_json::Value = cross_end_res.json().await.unwrap();
    assert_eq!(cross_json["ended"], true);
    assert_eq!(cross_json["announcement_status"], "cross_guild_blocked");

    // ZERO messages must be posted to channel 201 or any foreign channel!
    let cross_calls = mock.recorded_calls.lock().await.clone();
    let cross_posts = cross_calls
        .iter()
        .filter(|(m, p)| {
            *m == axum::http::Method::POST && p.starts_with("/api/v10/channels/201/messages")
        })
        .count();
    assert_eq!(
        cross_posts, 0,
        "No message should be posted to cross-guild channel 201"
    );

    shard_manager.shutdown_all().await;
}
