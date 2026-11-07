use std::sync::Arc;

use chrono::{Duration, Utc};
use dashmap::DashMap;
use reqwest::header::COOKIE;
use rusty_discord_bot::{
    config::Config,
    session::{DashboardGuild, DashboardUser, Session},
    state::AppState,
    web::{
        WebState,
        auth::{
            CachedGuildPermission, LoginRateLimiter, cleanup_stale_records, ensure_guild_access,
        },
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
async fn test_ensure_guild_access_lifecycle_cache_and_revocation() {
    let (state, perms_cache) = test_web_state().await;

    let mut session = Session {
        user: DashboardUser {
            id: "user_100".to_owned(),
            username: "admin_user".to_owned(),
            global_name: Some("Admin".to_owned()),
            avatar: None,
        },
        guilds: vec![DashboardGuild {
            id: "guild_allow".to_owned(),
            name: "Allowed Guild".to_owned(),
            icon: None,
            owner: false,
            permissions: "32".to_owned(), // MANAGE_GUILD
        }],
        csrf_token: "csrf_abc".to_owned(),
        expires_at: Utc::now() + Duration::days(1),
        access_token: None,
    };

    // 1. Initial access: guild is allowed and permission is cached
    let result = ensure_guild_access(&state, &session, "guild_allow").await;
    assert!(result.is_ok());

    let cache_entry = perms_cache
        .get(&("user_100".to_owned(), "guild_allow".to_owned()))
        .expect("cache entry should exist");
    assert!(cache_entry.can_manage);
    drop(cache_entry);

    // 2. Fast cache hit: immediately succeeds from cache
    assert!(
        ensure_guild_access(&state, &session, "guild_allow")
            .await
            .is_ok()
    );

    // 3. Unassociated guild access is rejected immediately (403 Forbidden)
    let denied = ensure_guild_access(&state, &session, "guild_unauthorized").await;
    assert!(denied.is_err());
    let denied_msg = denied.unwrap_err().message;
    assert!(
        denied_msg == "Nie masz uprawnień do zarządzania tym serwerem."
            || denied_msg == "You do not have permission to manage this server."
    );

    // 4. Permissions revoked in Discord (simulated by updating permissions bitmask)
    session.guilds[0].permissions = "0".to_owned();

    // 5. While cache is fresh (< 60s), previous grant is served
    assert!(
        ensure_guild_access(&state, &session, "guild_allow")
            .await
            .is_ok()
    );

    // 6. Expire cache entry (> 60s)
    perms_cache.insert(
        ("user_100".to_owned(), "guild_allow".to_owned()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now() - Duration::seconds(70),
        },
    );

    // 7. Revalidation runs against Discord, fails closed, and records revocation in cache
    let revalidated = ensure_guild_access(&state, &session, "guild_allow").await;
    assert!(revalidated.is_err());
    let err = revalidated.unwrap_err();
    assert_eq!(err.status, axum::http::StatusCode::FORBIDDEN);
    assert!(
        err.message == "Uprawnienia do zarządzania tym serwerem zostały cofnięte."
            || err.message == "Permissions to manage this server have been revoked."
    );

    // 8. Verify cache entry now records can_manage = false
    let cached_revocation = perms_cache
        .get(&("user_100".to_owned(), "guild_allow".to_owned()))
        .expect("revocation should be cached");
    assert!(!cached_revocation.can_manage);
    drop(cached_revocation);

    // 9. Subsequent request within cache TTL remains rejected
    let cached_denial = ensure_guild_access(&state, &session, "guild_allow").await;
    assert!(cached_denial.is_err());
    let cached_msg = cached_denial.unwrap_err().message;
    assert!(
        cached_msg == "Uprawnienia do zarządzania tym serwerem zostały cofnięte."
            || cached_msg == "Permissions to manage this server have been revoked."
    );
}

#[tokio::test]
async fn test_ensure_guild_access_owner_and_administrator_bits() {
    let (state, _) = test_web_state().await;

    // Test guild owner (owner = true, permissions = 0)
    let session_owner = Session {
        user: DashboardUser {
            id: "user_owner".to_owned(),
            username: "server_owner".to_owned(),
            global_name: None,
            avatar: None,
        },
        guilds: vec![DashboardGuild {
            id: "guild_owned".to_owned(),
            name: "Owned Guild".to_owned(),
            icon: None,
            owner: true,
            permissions: "0".to_owned(),
        }],
        csrf_token: "csrf_owner".to_owned(),
        expires_at: Utc::now() + Duration::days(1),
        access_token: None,
    };
    assert!(
        ensure_guild_access(&state, &session_owner, "guild_owned")
            .await
            .is_ok()
    );

    // Test administrator bit (1 << 3 = 8)
    let session_admin = Session {
        user: DashboardUser {
            id: "user_admin".to_owned(),
            username: "server_admin".to_owned(),
            global_name: None,
            avatar: None,
        },
        guilds: vec![DashboardGuild {
            id: "guild_admin".to_owned(),
            name: "Admin Guild".to_owned(),
            icon: None,
            owner: false,
            permissions: "8".to_owned(), // ADMINISTRATOR
        }],
        csrf_token: "csrf_admin".to_owned(),
        expires_at: Utc::now() + Duration::days(1),
        access_token: None,
    };
    assert!(
        ensure_guild_access(&state, &session_admin, "guild_admin")
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn test_cleanup_stale_permissions_cache() {
    let (state, perms_cache) = test_web_state().await;

    // Insert active entry (< 120s old)
    perms_cache.insert(
        ("u1".to_owned(), "g1".to_owned()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now() - Duration::seconds(30),
        },
    );

    // Insert stale entry (> 120s old)
    perms_cache.insert(
        ("u2".to_owned(), "g2".to_owned()),
        CachedGuildPermission {
            can_manage: false,
            checked_at: Utc::now() - Duration::seconds(150),
        },
    );

    let (_, _, perms_removed) = cleanup_stale_records(&state).await;
    assert_eq!(perms_removed, 1);
    assert!(perms_cache.contains_key(&("u1".to_owned(), "g1".to_owned())));
    assert!(!perms_cache.contains_key(&("u2".to_owned(), "g2".to_owned())));
}

#[tokio::test]
async fn test_http_api_guild_endpoints_enforce_permissions_and_revalidation() {
    let (state, perms_cache) = test_web_state().await;

    // Start ephemeral local server
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

    let session_id = "test_active_session_token";
    let user = DashboardUser {
        id: "2001".to_owned(),
        username: "web_admin".to_owned(),
        global_name: None,
        avatar: None,
    };
    let guilds = vec![DashboardGuild {
        id: "guild_web_1".to_owned(),
        name: "Web Server 1".to_owned(),
        icon: None,
        owner: false,
        permissions: "32".to_owned(), // MANAGE_GUILD
    }];

    state
        .app
        .sessions
        .save_session(
            session_id,
            user,
            guilds,
            "csrf_token_test".to_owned(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .expect("save session");

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    // 1. Unauthenticated request to /api/guilds/{id}/modules returns 401 Unauthorized
    let unauth_resp = client
        .get(format!("{base_url}/api/guilds/guild_web_1/modules"))
        .send()
        .await
        .expect("send unauthenticated request");
    assert_eq!(unauth_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 2. Request for authorized guild returns 200 OK
    let auth_resp = client
        .get(format!("{base_url}/api/guilds/guild_web_1/modules"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("send authorized request");
    assert_eq!(auth_resp.status(), reqwest::StatusCode::OK);

    // 3. Request for unassociated guild returns 403 Forbidden
    let forbidden_resp = client
        .get(format!("{base_url}/api/guilds/guild_unassociated/modules"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("send unassociated guild request");
    assert_eq!(forbidden_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 4. Permission revoked in cache -> request to previously authorized guild returns 403 Forbidden
    perms_cache.insert(
        ("2001".to_owned(), "guild_web_1".to_owned()),
        CachedGuildPermission {
            can_manage: false,
            checked_at: Utc::now(),
        },
    );

    let revoked_resp = client
        .get(format!("{base_url}/api/guilds/guild_web_1/modules"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("send revoked request");
    assert_eq!(revoked_resp.status(), reqwest::StatusCode::FORBIDDEN);
    let body: serde_json::Value = revoked_resp.json().await.expect("json body");
    let err_str = body["error"].as_str().unwrap_or_default();
    assert!(
        err_str == "Uprawnienia do zarządzania tym serwerem zostały cofnięte."
            || err_str == "Permissions to manage this server have been revoked."
    );
}

#[tokio::test]
async fn test_phase1_governance_and_defense_suite() {
    let (web_state, _) = test_web_state().await;
    let app = build_router(web_state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");

    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve axum");
    });

    let session_id = "test_sess_phase1";
    let user = DashboardUser {
        id: "3001".to_owned(),
        username: "tester_phase1".to_owned(),
        global_name: None,
        avatar: None,
    };
    let guilds = vec![DashboardGuild {
        id: "guild_phase1".to_owned(),
        name: "Phase1 Test Guild".to_owned(),
        icon: None,
        owner: true,
        permissions: (1 << 3).to_string(), // Administrator
    }];

    web_state
        .app
        .sessions
        .save_session(
            session_id,
            user,
            guilds,
            "csrf_phase1_token".to_owned(),
            Utc::now() + Duration::hours(1),
            Some("token_123".to_owned()),
        )
        .await
        .expect("save session");

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    // 1. Wizard profiles check (#73 / Agent 02)
    let wizard_resp = client
        .get(format!("{base_url}/api/guilds/guild_phase1/wizard"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("wizard request");
    assert_eq!(wizard_resp.status(), reqwest::StatusCode::OK);
    let wizard_body: serde_json::Value = wizard_resp.json().await.expect("wizard json");
    let profiles = wizard_body["profiles"].as_array().expect("profiles array");
    assert_eq!(profiles.len(), 3);
    assert_eq!(profiles[0]["id"], "gaming");
    assert_eq!(profiles[1]["id"], "community");
    assert_eq!(profiles[2]["id"], "support");

    // 2. Diagnostics check (#73 / Agent 02)
    let diag_resp = client
        .get(format!("{base_url}/api/guilds/guild_phase1/diagnostics"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("diagnostics request");
    assert_eq!(diag_resp.status(), reqwest::StatusCode::OK);
    let diag_body: serde_json::Value = diag_resp.json().await.expect("diag json");
    assert!(diag_body["active_modules_count"].as_u64().unwrap() > 0);
    assert_eq!(diag_body["overall_health_percent"], 100);

    // 3. Draft vs Published configuration lifecycle (#73 / Agent 02)
    let draft_payload = serde_json::json!({
        "config": {
            "anti_spam": true,
            "max_mentions": 3
        }
    });

    // Save draft
    let save_draft_resp = client
        .post(format!(
            "{base_url}/api/guilds/guild_phase1/modules/automod/draft"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", "csrf_phase1_token")
        .json(&draft_payload)
        .send()
        .await
        .expect("save draft request");
    assert_eq!(save_draft_resp.status(), reqwest::StatusCode::OK);

    // Read draft
    let get_draft_resp = client
        .get(format!(
            "{base_url}/api/guilds/guild_phase1/modules/automod/draft"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("get draft request");
    assert_eq!(get_draft_resp.status(), reqwest::StatusCode::OK);
    let draft_val: serde_json::Value = get_draft_resp.json().await.expect("draft val");
    assert_eq!(draft_val["has_draft"], true);
    assert_eq!(draft_val["config"]["max_mentions"], 3);

    // Publish draft
    let publish_resp = client
        .post(format!(
            "{base_url}/api/guilds/guild_phase1/modules/automod/publish"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", "csrf_phase1_token")
        .send()
        .await
        .expect("publish draft request");
    assert_eq!(publish_resp.status(), reqwest::StatusCode::OK);

    // Verify draft was cleared after publish
    let get_draft_after = client
        .get(format!(
            "{base_url}/api/guilds/guild_phase1/modules/automod/draft"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("get draft after publish");
    let after_val: serde_json::Value = get_draft_after.json().await.expect("after draft val");
    assert_eq!(after_val["has_draft"], false);

    // 4. Privacy Center summary & export (#73 / Agent 02)
    let privacy_resp = client
        .get(format!("{base_url}/api/guilds/guild_phase1/privacy"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("privacy request");
    assert_eq!(privacy_resp.status(), reqwest::StatusCode::OK);
    let privacy_body: serde_json::Value = privacy_resp.json().await.expect("privacy json");
    assert!(privacy_body["stored_entities"].as_array().unwrap().len() >= 3);

    let export_resp = client
        .post(format!("{base_url}/api/guilds/guild_phase1/privacy/export"))
        .header(COOKIE, &session_cookie)
        .json(&serde_json::json!({ "user_id": "3001" }))
        .send()
        .await
        .expect("privacy export request");
    assert_eq!(export_resp.status(), reqwest::StatusCode::OK);
    let export_body: serde_json::Value = export_resp.json().await.expect("export json");
    assert_eq!(export_body["user_id"], "3001");

    // 5. AutoMod dry-run simulator (#75 / Agent 04)
    let sim_invite_resp = client
        .post(format!(
            "{base_url}/api/guilds/guild_phase1/automod/simulate"
        ))
        .header(COOKIE, &session_cookie)
        .json(&serde_json::json!({
            "content": "Dołącz do nas! discord.gg/super-serwer",
            "author_account_age_days": 10,
            "has_avatar": true
        }))
        .send()
        .await
        .expect("simulate invite request");
    assert_eq!(sim_invite_resp.status(), reqwest::StatusCode::OK);
    let sim_invite_body: serde_json::Value = sim_invite_resp.json().await.expect("sim invite json");
    assert_eq!(sim_invite_body["matched"], true);
    assert_eq!(sim_invite_body["winning_rule"], "automod.invite_guard");

    let sim_clean_resp = client
        .post(format!(
            "{base_url}/api/guilds/guild_phase1/automod/simulate"
        ))
        .header(COOKIE, &session_cookie)
        .json(&serde_json::json!({
            "content": "Cześć wszystkim, miłego dnia życzę!",
            "author_account_age_days": 100,
            "has_avatar": true
        }))
        .send()
        .await
        .expect("simulate clean request");
    assert_eq!(sim_clean_resp.status(), reqwest::StatusCode::OK);
    let sim_clean_body: serde_json::Value = sim_clean_resp.json().await.expect("sim clean json");
    assert_eq!(sim_clean_body["matched"], false);
    assert_eq!(sim_clean_body["expected_action"], "allow");

    // 6. Appeals lifecycle (#74 / Agent 03)
    let appeals_list_resp = client
        .get(format!("{base_url}/api/guilds/guild_phase1/appeals"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("list appeals request");
    assert_eq!(appeals_list_resp.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn test_http_oauth_flow_browser_binding() {
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

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build client");

    let base_url = format!("http://{}", addr);

    // 1. Initiate OAuth login -> must get 307 redirect and Set-Cookie with rusty_oauth_state
    let login_resp = client
        .get(format!("{base_url}/auth/discord"))
        .send()
        .await
        .expect("login request");

    assert_eq!(login_resp.status(), reqwest::StatusCode::TEMPORARY_REDIRECT);
    let cookie_header = login_resp
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .expect("set-cookie header")
        .to_str()
        .expect("cookie str");

    assert!(cookie_header.contains("rusty_oauth_state="));
    assert!(cookie_header.contains("Path=/auth/discord"));
    assert!(cookie_header.contains("HttpOnly"));
    assert!(cookie_header.contains("SameSite=Lax"));

    let location = login_resp
        .headers()
        .get(reqwest::header::LOCATION)
        .expect("location header")
        .to_str()
        .expect("location str");
    let parsed_url = url::Url::parse(location).expect("parsed oauth url");
    let state_param = parsed_url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.to_string())
        .expect("state param");

    // 2. Attack attempt: attacker initiates OAuth, sends link to victim without victim having cookie
    let victim_resp_no_cookie = client
        .get(format!(
            "{base_url}/auth/discord/callback?code=mock_code&state={state_param}"
        ))
        .send()
        .await
        .expect("victim callback request");

    assert_eq!(
        victim_resp_no_cookie.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let error_body: serde_json::Value =
        victim_resp_no_cookie.json().await.expect("json error body");
    assert_eq!(
        error_body["error"],
        "OAuth login was not initiated in this browser."
    );

    // 3. Attack attempt: victim has another / mismatched oauth state cookie
    let victim_resp_wrong_cookie = client
        .get(format!(
            "{base_url}/auth/discord/callback?code=mock_code&state={state_param}"
        ))
        .header(COOKIE, "rusty_oauth_state=mismatched_state_uuid")
        .send()
        .await
        .expect("victim callback request with mismatched cookie");

    assert_eq!(
        victim_resp_wrong_cookie.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let error_body2: serde_json::Value = victim_resp_wrong_cookie
        .json()
        .await
        .expect("json error body");
    assert_eq!(
        error_body2["error"],
        "OAuth login was not initiated in this browser."
    );

    // State must still be preserved in server memory (not burned by malicious/mismatched requests)
    assert!(
        state
            .app
            .sessions
            .consume_oauth_state(&state_param)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_member_intelligence_api_endpoints_permissions_and_search() {
    use rusty_discord_bot::storage::UpsertMemberSnapshotParams;

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

    let session_id = "test_member_intel_session";
    let user = DashboardUser {
        id: "100000000000000099".to_owned(),
        username: "intel_admin_user".to_owned(),
        global_name: None,
        avatar: None,
    };
    let guild_id = "777000000000000001";
    let guilds = vec![DashboardGuild {
        id: guild_id.to_owned(),
        name: "Intelligence Guild".to_owned(),
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
            "csrf_token_intel".to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .expect("save session");

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    // Seed snapshots and identity history
    let joined = Utc::now() - Duration::days(10);
    state
        .app
        .storage
        .upsert_member_snapshot(UpsertMemberSnapshotParams {
            guild_id,
            user_id: "100000000000000001",
            username: "alice_crypto",
            global_name: Some("Alice C"),
            nickname: Some("Cool Alice"),
            role_ids: &["role_dev".to_string(), "role_vip".to_string()],
            joined_at: Some(joined),
        })
        .await
        .unwrap();

    state
        .app
        .storage
        .record_member_identity_change(
            guild_id,
            "100000000000000001",
            "nickname",
            Some("Old Nick"),
            Some("Cool Alice"),
            30,
        )
        .await
        .unwrap();

    state
        .app
        .storage
        .record_member_identity_change(
            guild_id,
            "100000000000000001",
            "username",
            Some("alice_old"),
            Some("alice_crypto"),
            30,
        )
        .await
        .unwrap();

    // 1. Unauthenticated request -> 401
    let unauth_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/members/search"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    let csrf_header = "csrf_token_intel";

    // 2. Search query matches username or nickname -> 200 OK
    let search_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/members/search"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_header)
        .send()
        .await
        .unwrap();
    assert_eq!(search_resp.status(), reqwest::StatusCode::OK);
    let search_json: serde_json::Value = search_resp.json().await.unwrap();
    assert!(search_json["members"].is_array());
    let members = search_json["members"].as_array().unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0]["user_id"], "100000000000000001");
    assert_eq!(members[0]["username"], "alice_crypto");
    assert_eq!(members[0]["nickname"], "Cool Alice");
    assert_eq!(members[0]["moderation_case_count"], 0);

    // 3. Search filtering by role
    let role_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/members/search?roles=role_dev"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_header)
        .send()
        .await
        .unwrap();
    assert_eq!(role_resp.status(), reqwest::StatusCode::OK);
    let role_json: serde_json::Value = role_resp.json().await.unwrap();
    assert_eq!(role_json["members"].as_array().unwrap().len(), 1);

    // 4. Identity history endpoint -> 200 OK
    let history_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/members/100000000000000001/identity-history"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_header)
        .send()
        .await
        .unwrap();
    assert_eq!(history_resp.status(), reqwest::StatusCode::OK);
    let history_json: serde_json::Value = history_resp.json().await.unwrap();
    assert!(history_json["history"].is_array());
    let history_items = history_json["history"].as_array().unwrap();
    assert_eq!(history_items.len(), 2);
    assert_eq!(history_items[0]["kind"], "username");
    assert_eq!(history_items[0]["new_value"], "alice_crypto");
    assert_eq!(history_items[1]["kind"], "nickname");
    assert_eq!(history_items[1]["new_value"], "Cool Alice");

    // 5. Forbidden guild -> 403
    let forbidden_resp = client
        .get(format!(
            "{base_url}/api/guilds/999000000000000001/members/search"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_header)
        .send()
        .await
        .unwrap();
    assert_eq!(forbidden_resp.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_cleanup_preview_and_confirm_endpoints_and_isolation() {
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

    let guild_id = "777000000000000050";
    let admin_user_id = "100000000000000050";
    let admin_session_id = "test_cleanup_admin_session";
    let csrf_token = "csrf_token_cleanup";

    state
        .app
        .sessions
        .save_session(
            admin_session_id,
            DashboardUser {
                id: admin_user_id.to_owned(),
                username: "mod_admin".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Cleanup Guild".to_owned(),
                icon: None,
                owner: true,
                permissions: "8".to_owned(),
            }],
            csrf_token.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let session_cookie = format!("rusty_discord_bot_session={admin_session_id}");

    // 1. Preview cleanup generates plan
    let preview_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/cleanup/preview"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "channel_id": "888000000000000050",
            "message_ids": ["999000000000000001", "999000000000000002"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(preview_resp.status(), reqwest::StatusCode::OK);
    let plan_json: serde_json::Value = preview_resp.json().await.unwrap();
    let plan_id = plan_json["id"].as_str().unwrap();
    assert_eq!(plan_json["actor_user_id"], admin_user_id);
    assert_eq!(plan_json["channel_id"], "888000000000000050");
    assert!(plan_json["consumed_at"].is_null());

    // 2. Confirm cleanup succeeds
    let confirm_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/cleanup/confirm"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "plan_id": plan_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(confirm_resp.status(), reqwest::StatusCode::OK);
    let confirmed_json: serde_json::Value = confirm_resp.json().await.unwrap();
    assert!(confirmed_json["consumed_at"].is_string());

    // 3. Second confirm fails (idempotent atomic consumption)
    let second_confirm = client
        .post(format!("{base_url}/api/guilds/{guild_id}/cleanup/confirm"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "plan_id": plan_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(second_confirm.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_voice_companion_channels_api() {
    let (state, _perms) = test_web_state().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let app = build_router(state.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve axum");
    });

    let admin_user_id = "555000000000000001";
    let guild_id = "777000000000000001";
    let csrf_token = "valid_csrf_token_companion";
    let admin_session_id = "test_companion_admin_session";

    state
        .app
        .sessions
        .save_session(
            admin_session_id,
            DashboardUser {
                id: admin_user_id.to_owned(),
                username: "Admin".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Test Guild".to_owned(),
                icon: None,
                owner: true,
                permissions: "8".to_owned(),
            }],
            csrf_token.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let session_cookie = format!("rusty_discord_bot_session={admin_session_id}");

    // 1. Initial list of mappings is empty
    let list_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/voice/companion/mappings"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp.status(), reqwest::StatusCode::OK);
    let list_json: serde_json::Value = list_resp.json().await.unwrap();
    assert_eq!(list_json["mappings"].as_array().unwrap().len(), 0);

    // 2. Reject request without CSRF
    let no_csrf = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/voice/companion/mappings"
        ))
        .header(COOKIE, &session_cookie)
        .json(&serde_json::json!({
            "voice_channel_id": "888000000000000001",
            "text_channel_id": "888000000000000002"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), reqwest::StatusCode::FORBIDDEN);

    // 3. Reject same voice and text channel
    let same_ch = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/voice/companion/mappings"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "voice_channel_id": "888000000000000001",
            "text_channel_id": "888000000000000001"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(same_ch.status(), reqwest::StatusCode::BAD_REQUEST);

    // 3b. Reject cross-guild channels (#172)
    let cross_guild = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/voice/companion/mappings"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "voice_channel_id": "888000000000000001",
            "text_channel_id": "888000000000000002_other_guild"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(cross_guild.status(), reqwest::StatusCode::BAD_REQUEST);

    // 4. Reject invalid snowflake
    let bad_snow = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/voice/companion/mappings"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "voice_channel_id": "not_a_snowflake",
            "text_channel_id": "888000000000000002"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_snow.status(), reqwest::StatusCode::BAD_REQUEST);

    // 4b. Reject batch exceeding 50 mappings without inserting any records (Issue #211)
    let mut over_limit_payload = Vec::new();
    for i in 1..=51 {
        over_limit_payload.push(serde_json::json!({
            "voice_channel_id": format!("8880000000000000{:02}", i),
            "text_channel_id": format!("9990000000000000{:02}", i),
        }));
    }
    let over_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/voice/companion/mappings"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({ "mappings": over_limit_payload }))
        .send()
        .await
        .unwrap();
    assert_eq!(over_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Verify 0 records inserted after over_limit failure
    let empty_check = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/voice/companion/mappings"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    let empty_json: serde_json::Value = empty_check.json().await.unwrap();
    assert_eq!(empty_json["mappings"].as_array().unwrap().len(), 0);

    // 4c. Reject mixed batch (1 valid, 1 invalid) without partial inserts (Issue #211)
    let mixed_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/voice/companion/mappings"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "mappings": [
                {
                    "voice_channel_id": "888000000000000050",
                    "text_channel_id": "999000000000000050"
                },
                {
                    "voice_channel_id": "888000000000000051",
                    "text_channel_id": "888000000000000051" // same: invalid
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(mixed_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Verify 0 records inserted after mixed failure
    let empty_check2 = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/voice/companion/mappings"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    let empty_json2: serde_json::Value = empty_check2.json().await.unwrap();
    assert_eq!(empty_json2["mappings"].as_array().unwrap().len(), 0);

    // 4d. Reject batch with duplicate voice_channel_id in 2-element payload (Issue #215)
    let dup_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/voice/companion/mappings"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "mappings": [
                {
                    "voice_channel_id": "888000000000000010",
                    "text_channel_id": "999000000000000010"
                },
                {
                    "voice_channel_id": "888000000000000010", // duplicate voice ID
                    "text_channel_id": "999000000000000011"
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(dup_resp.status(), reqwest::StatusCode::BAD_REQUEST);
    let dup_json: serde_json::Value = dup_resp.json().await.unwrap();
    assert!(
        dup_json["error"]
            .as_str()
            .unwrap()
            .contains("Duplicate voice_channel_id")
    );

    // 4e. Reject batch with >50 items sharing 1 unique voice ID (Issue #215)
    let mut dup_51_payload = Vec::new();
    for i in 1..=100 {
        dup_51_payload.push(serde_json::json!({
            "voice_channel_id": "888000000000000020",
            "text_channel_id": format!("99900000000000{:04}", i),
        }));
    }
    let dup_51_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/voice/companion/mappings"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({ "mappings": dup_51_payload }))
        .send()
        .await
        .unwrap();
    assert_eq!(dup_51_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Verify still 0 records inserted
    let empty_check3 = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/voice/companion/mappings"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    let empty_json3: serde_json::Value = empty_check3.json().await.unwrap();
    assert_eq!(empty_json3["mappings"].as_array().unwrap().len(), 0);

    // 5. Valid create mapping
    let create_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/voice/companion/mappings"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "voice_channel_id": "888000000000000001",
            "text_channel_id": "888000000000000002",
            "allow_send_messages": true,
            "allow_read_history": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), reqwest::StatusCode::OK);
    let created_json: serde_json::Value = create_resp.json().await.unwrap();
    let mappings_arr = created_json["mappings"].as_array().unwrap();
    assert_eq!(mappings_arr.len(), 1);
    let mapping_id = mappings_arr[0]["id"].as_str().unwrap();

    // 6. List returns 1 mapping
    let list_resp2 = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/voice/companion/mappings"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp2.status(), reqwest::StatusCode::OK);
    let list_json2: serde_json::Value = list_resp2.json().await.unwrap();
    assert_eq!(list_json2["mappings"].as_array().unwrap().len(), 1);

    // 7. List leases returns empty
    let leases_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/voice/companion/leases"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(leases_resp.status(), reqwest::StatusCode::OK);
    let leases_json: serde_json::Value = leases_resp.json().await.unwrap();
    assert_eq!(leases_json["leases"].as_array().unwrap().len(), 0);

    // 8. Reconcile endpoint
    let rec_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/voice/companion/reconcile"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(rec_resp.status(), reqwest::StatusCode::OK);
    let rec_json: serde_json::Value = rec_resp.json().await.unwrap();
    assert_eq!(rec_json["reconciled"], true);

    // 9. Delete mapping
    let del_resp = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/voice/companion/mappings/{mapping_id}"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(del_resp.status(), reqwest::StatusCode::OK);

    // 10. List is empty again
    let list_resp3 = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/voice/companion/mappings"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp3.status(), reqwest::StatusCode::OK);
    let list_json3: serde_json::Value = list_resp3.json().await.unwrap();
    assert_eq!(list_json3["mappings"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_command_policies_api() {
    let (state, _perms) = test_web_state().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let app = build_router(state.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve axum");
    });

    let admin_user_id = "555000000000000001";
    let guild_id = "777000000000000001";
    let guild_id_2 = "777000000000000002";
    let user_id = "666000000000000001";
    let channel_id = "888000000000000001";
    let csrf_token = "valid_csrf_token_policies";
    let admin_session_id = "test_policies_admin_session";

    state
        .app
        .sessions
        .save_session(
            admin_session_id,
            DashboardUser {
                id: admin_user_id.to_owned(),
                username: "Admin".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![
                DashboardGuild {
                    id: guild_id.to_owned(),
                    name: "Test Guild".to_owned(),
                    icon: None,
                    owner: true,
                    permissions: "8".to_owned(),
                },
                DashboardGuild {
                    id: guild_id_2.to_owned(),
                    name: "Test Guild 2".to_owned(),
                    icon: None,
                    owner: true,
                    permissions: "8".to_owned(),
                },
            ],
            csrf_token.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let session_cookie = format!("rusty_discord_bot_session={admin_session_id}");

    // 1. Unauthenticated request rejected
    let unauth = client
        .get(format!("{base_url}/api/guilds/{guild_id}/policies"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 2. Initial list of policies is empty
    let list_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/policies"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp.status(), reqwest::StatusCode::OK);
    let list_json: serde_json::Value = list_resp.json().await.unwrap();
    assert_eq!(list_json["rules"].as_array().unwrap().len(), 0);

    // 3. Upsert without CSRF rejected
    let no_csrf = client
        .post(format!("{base_url}/api/guilds/{guild_id}/policies"))
        .header(COOKIE, &session_cookie)
        .json(&serde_json::json!({
            "subject_kind": "user",
            "subject_id": user_id,
            "resource_kind": "command",
            "module_id": "fun",
            "command_name": "meme",
            "effect": "deny"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), reqwest::StatusCode::FORBIDDEN);

    // 4. Upsert with invalid snowflake rejected
    let invalid_id = client
        .post(format!("{base_url}/api/guilds/{guild_id}/policies"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "subject_kind": "user",
            "subject_id": "not_a_snowflake",
            "resource_kind": "command",
            "module_id": "fun",
            "command_name": "meme",
            "effect": "deny"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_id.status(), reqwest::StatusCode::BAD_REQUEST);

    // 5. Valid upsert
    let upsert_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/policies"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "subject_kind": "user",
            "subject_id": user_id,
            "resource_kind": "command",
            "module_id": "fun",
            "command_name": "meme",
            "effect": "deny",
            "priority": 5
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(upsert_resp.status(), reqwest::StatusCode::OK);
    let upsert_json: serde_json::Value = upsert_resp.json().await.unwrap();
    let rule_id = upsert_json["rule"]["id"].as_str().unwrap().to_string();
    assert!(upsert_json["revision"].as_i64().unwrap() >= 2);

    // 6. List now contains the rule
    let list_resp2 = client
        .get(format!("{base_url}/api/guilds/{guild_id}/policies"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp2.status(), reqwest::StatusCode::OK);
    let list_json2: serde_json::Value = list_resp2.json().await.unwrap();
    assert_eq!(list_json2["rules"].as_array().unwrap().len(), 1);

    // 7. Explain policy shows denied
    let explain_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/policies/explain"))
        .header(COOKIE, &session_cookie)
        .json(&serde_json::json!({
            "user_id": user_id,
            "channel_id": channel_id,
            "resource_kind": "command",
            "module_id": "fun",
            "command_name": "meme"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(explain_resp.status(), reqwest::StatusCode::OK);
    let explain_json: serde_json::Value = explain_resp.json().await.unwrap();
    assert_eq!(explain_json["outcome"]["allowed"], false);

    // 8. Export policies
    let export_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/policies/export"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(export_resp.status(), reqwest::StatusCode::OK);
    let export_json: serde_json::Value = export_resp.json().await.unwrap();
    assert_eq!(export_json["version"], 1);
    assert_eq!(export_json["rules"].as_array().unwrap().len(), 1);

    // 9. Delete specific policy rule
    let del_resp = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/policies/{rule_id}"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(del_resp.status(), reqwest::StatusCode::OK);

    // Re-create a rule for reset test
    let _ = client
        .post(format!("{base_url}/api/guilds/{guild_id}/policies"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "subject_kind": "channel",
            "subject_id": channel_id,
            "resource_kind": "module",
            "module_id": "fun",
            "effect": "allow"
        }))
        .send()
        .await
        .unwrap();

    // 10. Reset policies requires confirm
    let reset_fail = client
        .post(format!("{base_url}/api/guilds/{guild_id}/policies/reset"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({ "confirm": false }))
        .send()
        .await
        .unwrap();
    assert_eq!(reset_fail.status(), reqwest::StatusCode::BAD_REQUEST);

    // 10. Reset policies succeeds with confirm
    let reset_ok = client
        .post(format!("{base_url}/api/guilds/{guild_id}/policies/reset"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({ "confirm": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(reset_ok.status(), reqwest::StatusCode::OK);
    let reset_json: serde_json::Value = reset_ok.json().await.unwrap();
    assert_eq!(reset_json["reset"], true);
    assert_eq!(reset_json["removed_count"], 1);

    // 11. List is empty again
    let list_resp3 = client
        .get(format!("{base_url}/api/guilds/{guild_id}/policies"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp3.status(), reqwest::StatusCode::OK);
    let list_json3: serde_json::Value = list_resp3.json().await.unwrap();
    assert_eq!(list_json3["rules"].as_array().unwrap().len(), 0);

    // 12. Issue #176: Cross-guild UUID overwrite rejected with 409 Conflict
    let create_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/policies"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "subject_kind": "channel",
            "subject_id": channel_id,
            "resource_kind": "module",
            "module_id": "music",
            "effect": "deny"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), reqwest::StatusCode::OK);
    let created_json: serde_json::Value = create_resp.json().await.unwrap();
    let original_id = created_json["rule"]["id"].as_str().unwrap();

    let cross_upsert = client
        .post(format!("{base_url}/api/guilds/{guild_id_2}/policies"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "id": original_id,
            "subject_kind": "channel",
            "subject_id": channel_id,
            "resource_kind": "module",
            "module_id": "music",
            "effect": "allow"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(cross_upsert.status(), reqwest::StatusCode::CONFLICT);

    // 13. Issue #178: Policy import validation and atomicity
    // A) dry_run with invalid rule fails validation
    let dry_run_invalid = client
        .post(format!("{base_url}/api/guilds/{guild_id}/policies/import"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "dry_run": true,
            "rules": [{
                "id": uuid::Uuid::new_v4().to_string(),
                "guild_id": guild_id,
                "subject_kind": "channel",
                "subject_id": channel_id,
                "resource_kind": "module",
                "module_id": "music",
                "effect": "invalid_effect",
                "priority": 0,
                "enabled": true,
                "version": 1,
                "created_by": admin_user_id,
                "created_at": Utc::now().to_rfc3339(),
                "updated_at": Utc::now().to_rfc3339(),
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(dry_run_invalid.status(), reqwest::StatusCode::BAD_REQUEST);

    // B) live import with invalid rule fails validation and existing rules are not wiped
    let live_invalid = client
        .post(format!("{base_url}/api/guilds/{guild_id}/policies/import"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "dry_run": false,
            "rules": [{
                "id": uuid::Uuid::new_v4().to_string(),
                "guild_id": guild_id,
                "subject_kind": "channel",
                "subject_id": channel_id,
                "resource_kind": "module",
                "module_id": "music",
                "effect": "invalid_effect",
                "priority": 0,
                "enabled": true,
                "version": 1,
                "created_by": admin_user_id,
                "created_at": Utc::now().to_rfc3339(),
                "updated_at": Utc::now().to_rfc3339(),
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(live_invalid.status(), reqwest::StatusCode::BAD_REQUEST);

    // Verify existing rule still exists in guild_id
    let list_verify = client
        .get(format!("{base_url}/api/guilds/{guild_id}/policies"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    let list_v_json: serde_json::Value = list_verify.json().await.unwrap();
    assert_eq!(list_v_json["rules"].as_array().unwrap().len(), 1);

    // C) import with stale expected_revision fails with 409 Conflict
    let cur_rev = list_v_json["revision"].as_i64().unwrap();
    let stale_import = client
        .post(format!("{base_url}/api/guilds/{guild_id}/policies/import"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "expected_revision": cur_rev + 10,
            "rules": []
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(stale_import.status(), reqwest::StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_moderator_duty_api() {
    let (state, _perms) = test_web_state().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let app = build_router(state.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve axum");
    });

    let admin_user_id = "555000000000000001";
    let guild_id = "777000000000000001";
    let user_id = "666000000000000001";
    let csrf_token = "valid_csrf_token_duty";
    let admin_session_id = "test_duty_admin_session";

    state
        .app
        .sessions
        .save_session(
            admin_session_id,
            DashboardUser {
                id: admin_user_id.to_owned(),
                username: "Admin".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Test Guild".to_owned(),
                icon: None,
                owner: true,
                permissions: "8".to_owned(),
            }],
            csrf_token.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let session_cookie = format!("rusty_discord_bot_session={admin_session_id}");

    // 1. Unauthenticated request rejected
    let unauth = client
        .get(format!("{base_url}/api/guilds/{guild_id}/duty/settings"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 2. Fetch default settings
    let get_set = client
        .get(format!("{base_url}/api/guilds/{guild_id}/duty/settings"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_set.status(), reqwest::StatusCode::OK);

    // 3. Upsert settings without CSRF rejected
    let no_csrf = client
        .post(format!("{base_url}/api/guilds/{guild_id}/duty/settings"))
        .header(COOKIE, &session_cookie)
        .json(&serde_json::json!({
            "staff_role_id": "888000000000000001",
            "on_call_role_id": "888000000000000002"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), reqwest::StatusCode::FORBIDDEN);

    // 4. Upsert settings with CSRF succeeds
    let set_ok = client
        .post(format!("{base_url}/api/guilds/{guild_id}/duty/settings"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "staff_role_id": "888000000000000001",
            "on_call_role_id": "888000000000000002",
            "max_override_duration_mins": 720
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(set_ok.status(), reqwest::StatusCode::OK);

    // 5. List profiles initially empty
    let list_p1 = client
        .get(format!("{base_url}/api/guilds/{guild_id}/duty/profiles"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(list_p1.status(), reqwest::StatusCode::OK);
    let list_p1_json: serde_json::Value = list_p1.json().await.unwrap();
    assert_eq!(list_p1_json["profiles"].as_array().unwrap().len(), 0);

    // 6. Upsert profile
    let upsert_p = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/duty/profiles/{user_id}"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "timezone": "Europe/Berlin",
            "recurring_schedule": {
                "monday": [{"start": "18:00", "end": "22:00"}]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(upsert_p.status(), reqwest::StatusCode::OK);

    // 7. Get specific profile
    let get_p = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/duty/profiles/{user_id}"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_p.status(), reqwest::StatusCode::OK);
    let get_p_json: serde_json::Value = get_p.json().await.unwrap();
    assert_eq!(get_p_json["profile"]["timezone"], "Europe/Berlin");

    // 8. Override off
    let off_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/duty/override"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "user_id": user_id,
            "action": "off",
            "duration_mins": 120
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(off_resp.status(), reqwest::StatusCode::OK);
    let off_json: serde_json::Value = off_resp.json().await.unwrap();
    assert_eq!(off_json["profile"]["manual_override"], "force_off");

    // 8b. Override off with duration exceeding guild limit (max is 720, requesting 1440) -> 400 Bad Request (Issue #210)
    let over_duration_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/duty/override"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "user_id": user_id,
            "action": "off",
            "duration_mins": 1440
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        over_duration_resp.status(),
        reqwest::StatusCode::BAD_REQUEST
    );

    // 8c. Override off with extreme duration (i64::MAX) -> 400 Bad Request without panic (Issue #210)
    let overflow_duration_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/duty/override"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "user_id": user_id,
            "action": "off",
            "duration_mins": i64::MAX
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        overflow_duration_resp.status(),
        reqwest::StatusCode::BAD_REQUEST
    );

    // 8d. Override off with invalid non-positive duration -> 400 Bad Request (Issue #210)
    let zero_duration_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/duty/override"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "user_id": user_id,
            "action": "off",
            "duration_mins": 0
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        zero_duration_resp.status(),
        reqwest::StatusCode::BAD_REQUEST
    );

    // 9a. Reconcile guild duty with dry_run=true (Issue #187: pure preview, no DB mutation)
    let state_before = state
        .app
        .storage
        .get_duty_state(guild_id, user_id)
        .await
        .unwrap();
    assert!(
        state_before.is_none(),
        "No state should exist before live reconcile"
    );

    let rec_dry = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/duty/reconcile?dry_run=true"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(rec_dry.status(), reqwest::StatusCode::OK);
    let rec_dry_json: serde_json::Value = rec_dry.json().await.unwrap();
    assert_eq!(rec_dry_json["reconciled"], true);
    assert_eq!(rec_dry_json["dry_run"], true);
    assert_eq!(rec_dry_json["count"], 1);
    assert_eq!(rec_dry_json["outcomes"][0]["role_granted"], false);
    assert_eq!(rec_dry_json["outcomes"][0]["role_revoked"], false);

    let state_after_dry = state
        .app
        .storage
        .get_duty_state(guild_id, user_id)
        .await
        .unwrap();
    assert!(
        state_after_dry.is_none(),
        "Dry-run reconcile MUST NOT mutate ModeratorDutyState"
    );

    // 9b. Reconcile guild duty live apply
    let rec_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/duty/reconcile"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(rec_resp.status(), reqwest::StatusCode::OK);
    let rec_json: serde_json::Value = rec_resp.json().await.unwrap();
    assert_eq!(rec_json["reconciled"], true);
    assert_eq!(rec_json["dry_run"], false);
    assert_eq!(rec_json["count"], 1);

    // Simulate an active bot-owned role grant to test Issue #188 (deletion creates revoke_pending)
    state
        .app
        .storage
        .record_duty_role_grant(guild_id, user_id, "400000000000000001", "active")
        .await
        .unwrap();

    // 10. Delete profile (Issue #188: must mark grant as revoke_pending instead of orphaning)
    let del_resp = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/duty/profiles/{user_id}"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(del_resp.status(), reqwest::StatusCode::OK);

    let grant_after_del = state
        .app
        .storage
        .get_duty_role_grant(guild_id, user_id, "400000000000000001")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        grant_after_del.status, "revoke_pending",
        "Deleting profile MUST queue orphaned bot-owned role for revocation"
    );

    // 11. List is empty again
    let list_p2 = client
        .get(format!("{base_url}/api/guilds/{guild_id}/duty/profiles"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(list_p2.status(), reqwest::StatusCode::OK);
    let list_p2_json: serde_json::Value = list_p2.json().await.unwrap();
    assert_eq!(list_p2_json["profiles"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_moderator_duty_schedule_validation_and_diagnostics_issue_258() {
    let (state, _perms) = test_web_state().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let app = build_router(state.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve axum");
    });

    let admin_user_id = "555000000000000001";
    let guild_id = "777000000000000001";
    let user_id = "666000000000000001";
    let csrf_token = "valid_csrf_token_duty_validation";
    let admin_session_id = "test_duty_admin_session_val";

    state
        .app
        .sessions
        .save_session(
            admin_session_id,
            DashboardUser {
                id: admin_user_id.to_owned(),
                username: "Admin".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Test Guild".to_owned(),
                icon: None,
                owner: true,
                permissions: "8".to_owned(),
            }],
            csrf_token.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let session_cookie = format!("rusty_discord_bot_session={admin_session_id}");

    // 1. Initial valid profile upsert: 18:00 - 22:00
    let valid_upsert = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/duty/profiles/{user_id}"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "timezone": "Europe/Berlin",
            "recurring_schedule": {
                "monday": [{"start": "18:00", "end": "22:00"}]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(valid_upsert.status(), reqwest::StatusCode::OK);

    // Verify initial profile state
    let get_init = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/duty/profiles/{user_id}"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_init.status(), reqwest::StatusCode::OK);
    let get_init_json: serde_json::Value = get_init.json().await.unwrap();
    assert_eq!(
        get_init_json["profile"]["recurring_schedule"]["monday"][0]["end"],
        "22:00"
    );
    assert!(get_init_json["invalid_schedule"].is_null());

    // 2. Reject out-of-range hour: 25:00
    let resp_25 = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/duty/profiles/{user_id}"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "timezone": "Europe/Berlin",
            "recurring_schedule": {
                "monday": [{"start": "18:00", "end": "25:00"}]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_25.status(), reqwest::StatusCode::BAD_REQUEST);
    let err_25: serde_json::Value = resp_25.json().await.unwrap();
    let err_25_msg = err_25["error"].as_str().unwrap();
    assert!(err_25_msg.contains("Mon[0]"));
    assert!(err_25_msg.contains("25:00"));

    // 3. Reject non-time text: "tomorrow"
    let resp_tomorrow = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/duty/profiles/{user_id}"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "timezone": "Europe/Berlin",
            "recurring_schedule": {
                "tuesday": [{"start": "tomorrow", "end": "22:00"}]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_tomorrow.status(), reqwest::StatusCode::BAD_REQUEST);
    let err_tomorrow: serde_json::Value = resp_tomorrow.json().await.unwrap();
    let err_tomorrow_msg = err_tomorrow["error"].as_str().unwrap();
    assert!(err_tomorrow_msg.contains("Tue[0]"));
    assert!(err_tomorrow_msg.contains("tomorrow"));

    // 4. Reject empty string start time
    let resp_empty = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/duty/profiles/{user_id}"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "timezone": "Europe/Berlin",
            "recurring_schedule": {
                "wednesday": [{"start": "", "end": "22:00"}]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_empty.status(), reqwest::StatusCode::BAD_REQUEST);

    // 5. Reject invalid minute: 18:60
    let resp_minute = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/duty/profiles/{user_id}"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "timezone": "Europe/Berlin",
            "recurring_schedule": {
                "thursday": [{"start": "18:60", "end": "22:00"}]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_minute.status(), reqwest::StatusCode::BAD_REQUEST);

    // 6. Verify ATOMIC REJECTION: existing profile was NOT modified or overwritten
    let get_after_fails = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/duty/profiles/{user_id}"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_after_fails.status(), reqwest::StatusCode::OK);
    let get_after_json: serde_json::Value = get_after_fails.json().await.unwrap();
    assert_eq!(
        get_after_json["profile"]["recurring_schedule"]["monday"][0]["start"],
        "18:00"
    );
    assert_eq!(
        get_after_json["profile"]["recurring_schedule"]["monday"][0]["end"],
        "22:00"
    );

    // 7. Valid overnight shift succeeds: 22:00 - 02:00
    let resp_overnight = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/duty/profiles/{user_id}"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "timezone": "Europe/Berlin",
            "recurring_schedule": {
                "friday": [{"start": "22:00", "end": "02:00"}]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_overnight.status(), reqwest::StatusCode::OK);

    // 8. Non-destructive diagnostics for pre-existing legacy invalid record
    let user_legacy = "666000000000000002";
    let mut legacy_schedule = rusty_discord_bot::duty::RecurringSchedule::default();
    legacy_schedule
        .monday
        .push(rusty_discord_bot::duty::DutyTimeRange {
            start: "18:00".to_string(),
            end: "25:00".to_string(),
        });
    let legacy_profile = rusty_discord_bot::duty::ModeratorDutyProfile {
        guild_id: guild_id.to_string(),
        user_id: user_legacy.to_string(),
        timezone: "UTC".to_string(),
        recurring_schedule: sqlx::types::Json(legacy_schedule),
        manual_override: None,
        override_expires_at: None,
        version: 0,
        updated_at: Utc::now(),
    };
    state
        .app
        .storage
        .upsert_duty_profile(legacy_profile)
        .await
        .unwrap();

    // GET specific legacy profile reports invalid_schedule without deleting the record
    let get_legacy = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/duty/profiles/{user_legacy}"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_legacy.status(), reqwest::StatusCode::OK);
    let get_legacy_json: serde_json::Value = get_legacy.json().await.unwrap();
    assert_eq!(get_legacy_json["profile"]["user_id"], user_legacy);
    let diag_err = get_legacy_json["invalid_schedule"].as_str().unwrap();
    assert!(diag_err.contains("Mon[0]"));
    assert!(diag_err.contains("25:00"));

    // GET list reports invalid_schedule in the profile object
    let list_legacy = client
        .get(format!("{base_url}/api/guilds/{guild_id}/duty/profiles"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(list_legacy.status(), reqwest::StatusCode::OK);
    let list_json: serde_json::Value = list_legacy.json().await.unwrap();
    let profiles_arr = list_json["profiles"].as_array().unwrap();
    let legacy_item = profiles_arr
        .iter()
        .find(|p| p["user_id"] == user_legacy)
        .expect("legacy item found in list");
    assert!(
        legacy_item["invalid_schedule"]
            .as_str()
            .unwrap()
            .contains("Mon[0]")
    );
}

#[tokio::test]
async fn test_staff_recruitment_pipeline_full_lifecycle() {
    let (state, _perms) = test_web_state().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let app = build_router(state.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve axum");
    });
    let guild_id = "777000000000000001";
    let admin_user_id = "999000000000000001";
    let reviewer2_user_id = "999000000000000002";
    let candidate_user_id = "888000000000000001";
    let csrf_token = "valid_csrf_token_recruitment";

    // 1. Setup Admin session
    let admin_session_id = "admin_recruitment_session";
    state
        .app
        .sessions
        .save_session(
            admin_session_id,
            DashboardUser {
                id: admin_user_id.to_owned(),
                username: "AdminUser".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Test Guild".to_owned(),
                icon: None,
                owner: true,
                permissions: "8".to_owned(),
            }],
            csrf_token.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    // Setup Second Reviewer session
    let reviewer2_session_id = "reviewer2_recruitment_session";
    state
        .app
        .sessions
        .save_session(
            reviewer2_session_id,
            DashboardUser {
                id: reviewer2_user_id.to_owned(),
                username: "ReviewerTwo".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Test Guild".to_owned(),
                icon: None,
                owner: false,
                permissions: "8".to_owned(),
            }],
            csrf_token.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    // Setup Candidate session (normal guild member, permissions: "0")
    let candidate_session_id = "candidate_recruitment_session";
    state
        .app
        .sessions
        .save_session(
            candidate_session_id,
            DashboardUser {
                id: candidate_user_id.to_owned(),
                username: "CandidateUser".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Test Guild".to_owned(),
                icon: None,
                owner: false,
                permissions: "0".to_owned(),
            }],
            csrf_token.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let admin_cookie = format!("rusty_discord_bot_session={admin_session_id}");
    let reviewer2_cookie = format!("rusty_discord_bot_session={reviewer2_session_id}");
    let candidate_cookie = format!("rusty_discord_bot_session={candidate_session_id}");

    // 2. Candidate lists tracks - initially empty
    let tracks_res = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/tracks"
        ))
        .header(COOKIE, &candidate_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(tracks_res.status(), reqwest::StatusCode::OK);
    let tracks_json: serde_json::Value = tracks_res.json().await.unwrap();
    assert_eq!(tracks_json["tracks"].as_array().unwrap().len(), 0);

    // 3. Admin creates recruitment track with 2-person approval requirement
    let create_track_res = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/tracks"
        ))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "slug": "moderator",
            "name": "Moderator Application",
            "enabled": true,
            "target_role_id": "555000000000000001",
            "config": {
                "final_approvals": 2,
                "max_active_per_user": 1,
                "cooldown_days": 14,
                "auto_role_on_accept": true,
                "questions": [
                    {
                        "id": "q_exp",
                        "label": "Why do you want to be a moderator?",
                        "field_type": "paragraph",
                        "required": true
                    },
                    {
                        "id": "q_tz",
                        "label": "Timezone",
                        "field_type": "select",
                        "required": true,
                        "options": ["UTC", "CET", "EST"]
                    }
                ]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_track_res.status(), reqwest::StatusCode::OK);
    let track_json: serde_json::Value = create_track_res.json().await.unwrap();
    let track_id = track_json["track"]["id"].as_str().unwrap().to_owned();

    // 4. Candidate submits invalid application (missing required field)
    let invalid_apply = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/tracks/moderator/apply"
        ))
        .header(COOKIE, &candidate_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "answers": [
                { "question_id": "q_tz", "answer": "CET" }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_apply.status(), reqwest::StatusCode::BAD_REQUEST);

    // 5. Candidate submits valid application
    let valid_apply = client
        .post(format!("{base_url}/api/guilds/{guild_id}/recruitment/tracks/moderator/apply"))
        .header(COOKIE, &candidate_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "answers": [
                { "question_id": "q_exp", "answer": "I have 3 years of Discord moderation experience." },
                { "question_id": "q_tz", "answer": "CET" }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(valid_apply.status(), reqwest::StatusCode::OK);
    let app_json: serde_json::Value = valid_apply.json().await.unwrap();
    let app_id = app_json["application"]["id"].as_str().unwrap().to_owned();
    assert_eq!(app_json["application"]["status"], "submitted");

    // 6. Candidate tries to apply again (max_active_per_user limit)
    let duplicate_apply = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/tracks/moderator/apply"
        ))
        .header(COOKIE, &candidate_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "answers": [
                { "question_id": "q_exp", "answer": "Second application" },
                { "question_id": "q_tz", "answer": "UTC" }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(duplicate_apply.status(), reqwest::StatusCode::BAD_REQUEST);

    // 7. Candidate cannot review their own application
    let self_review = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/applications/{app_id}/review"
        ))
        .header(COOKIE, &candidate_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "recommendation": "approve",
            "note": "I approve myself!"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(self_review.status(), reqwest::StatusCode::FORBIDDEN);

    // 8. Admin reviews with 'approve'
    let review1_res = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/applications/{app_id}/review"
        ))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "recommendation": "approve",
            "note": "Strong track record."
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(review1_res.status(), reqwest::StatusCode::OK);

    // 9. Admin attempts to accept with only 1 approval when 2 are required
    let early_accept = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/applications/{app_id}/transition"
        ))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "expected_version": 1,
            "new_status": "accepted",
            "new_stage": "accepted",
            "decision_reason": "Accepted early"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(early_accept.status(), reqwest::StatusCode::BAD_REQUEST);

    // 10. Reviewer 2 reviews with 'approve'
    let review2_res = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/applications/{app_id}/review"
        ))
        .header(COOKIE, &reviewer2_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "recommendation": "approve",
            "note": "Second approval confirmed."
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(review2_res.status(), reqwest::StatusCode::OK);

    // 11. Optimistic concurrency conflict test (stale version)
    let conflict_res = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/applications/{app_id}/transition"
        ))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "expected_version": 999,
            "new_status": "accepted",
            "new_stage": "accepted"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(conflict_res.status(), reqwest::StatusCode::CONFLICT);

    // 12. Successful acceptance with expected_version = 1
    let accept_res = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/applications/{app_id}/transition"
        ))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "expected_version": 1,
            "new_status": "accepted",
            "new_stage": "accepted",
            "decision_reason": "Welcome to the team!"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(accept_res.status(), reqwest::StatusCode::OK);
    let accepted_json: serde_json::Value = accept_res.json().await.unwrap();
    assert_eq!(accepted_json["application"]["status"], "accepted");
    assert_eq!(
        accepted_json["application"]["reviews"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        accepted_json["application"]["fulfillment"]["status"],
        "pending"
    );

    // 13. Process role fulfillment batch
    let fulfill_res = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/fulfillment/process"
        ))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(fulfill_res.status(), reqwest::StatusCode::OK);
    let fulfill_json: serde_json::Value = fulfill_res.json().await.unwrap();
    // Issue #185: Offline mode leaves task pending instead of falsifying 'completed'
    assert_eq!(fulfill_json["processed"], 0);
    assert_eq!(fulfill_json["status"], "offline_no_discord_client");
    let pending_task = state
        .app
        .storage
        .get_role_fulfillment(app_id.parse::<uuid::Uuid>().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending_task.status, "pending");

    // 14. Candidate view check: privacy protection (no reviewer identity or notes)
    let candidate_view_res = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/applications/{app_id}"
        ))
        .header(COOKIE, &candidate_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(candidate_view_res.status(), reqwest::StatusCode::OK);
    let cand_view_json: serde_json::Value = candidate_view_res.json().await.unwrap();
    assert_eq!(cand_view_json["application"]["status"], "accepted");
    assert!(cand_view_json["application"].get("reviews").is_none());
    assert!(cand_view_json["application"].get("decided_by").is_none());

    // 15. Candidate applies to second track and withdraws application
    let track2_res = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/tracks"
        ))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "slug": "helper",
            "name": "Community Helper",
            "enabled": true,
            "config": {
                "questions": []
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(track2_res.status(), reqwest::StatusCode::OK);

    let apply2_res = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/tracks/helper/apply"
        ))
        .header(COOKIE, &candidate_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({ "answers": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(apply2_res.status(), reqwest::StatusCode::OK);
    let app2_id = apply2_res.json::<serde_json::Value>().await.unwrap()["application"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let withdraw_res = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/applications/{app2_id}/withdraw"
        ))
        .header(COOKIE, &candidate_cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(withdraw_res.status(), reqwest::StatusCode::OK);
    let withdrawn_json: serde_json::Value = withdraw_res.json().await.unwrap();
    assert_eq!(withdrawn_json["application"]["status"], "withdrawn");

    // Clean up track
    let del_track_res = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/tracks/{track_id}"
        ))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(del_track_res.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn test_help_forum_lifecycle_and_api() {
    let (state, _perms) = test_web_state().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let app = build_router(state.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve axum");
    });

    let guild_id = "888000000000000001";
    let admin_user_id = "999000000000000001";
    let author_user_id = "111000000000000001";
    let stranger_user_id = "222000000000000001";
    let helper_role_id = "333000000000000001";
    let forum_id = "444000000000000001";
    let thread_id = "555000000000000001";
    let csrf_token = "valid_csrf_token_help_forum";

    // Setup Admin session
    let admin_session_id = "admin_help_session";
    state
        .app
        .sessions
        .save_session(
            admin_session_id,
            DashboardUser {
                id: admin_user_id.to_owned(),
                username: "Admin".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Test Guild".to_owned(),
                icon: None,
                owner: true,
                permissions: "8".to_owned(),
            }],
            csrf_token.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    // Setup Author session
    let author_session_id = "author_help_session";
    state
        .app
        .sessions
        .save_session(
            author_session_id,
            DashboardUser {
                id: author_user_id.to_owned(),
                username: "Author".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Test Guild".to_owned(),
                icon: None,
                owner: false,
                permissions: "0".to_owned(),
            }],
            csrf_token.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    // Setup Stranger session
    let stranger_session_id = "stranger_help_session";
    state
        .app
        .sessions
        .save_session(
            stranger_session_id,
            DashboardUser {
                id: stranger_user_id.to_owned(),
                username: "Stranger".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Test Guild".to_owned(),
                icon: None,
                owner: false,
                permissions: "0".to_owned(),
            }],
            csrf_token.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let admin_cookie = format!("rusty_discord_bot_session={admin_session_id}");
    let author_cookie = format!("rusty_discord_bot_session={author_session_id}");
    let stranger_cookie = format!("rusty_discord_bot_session={stranger_session_id}");

    // 1. List help forums initially empty
    let list_res = client
        .get(format!("{base_url}/api/guilds/{guild_id}/help-forums"))
        .header(COOKIE, &admin_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(list_res.status(), reqwest::StatusCode::OK);
    let list_json: serde_json::Value = list_res.json().await.unwrap();
    assert_eq!(list_json["configs"].as_array().unwrap().len(), 0);

    // 2. Admin creates Help Forum config
    let create_res = client
        .post(format!("{base_url}/api/guilds/{guild_id}/help-forums"))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "forum_channel_id": forum_id,
            "enabled": true,
            "idle_close_minutes": 60,
            "owner_can_close": true,
            "helper_role_ids": [helper_role_id],
            "lock_on_close": true,
            "archive_on_close": true,
            "welcome_message": "Welcome to the help forum! Please be descriptive."
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_res.status(), reqwest::StatusCode::OK);

    // 3. Create active session in storage
    let session = rusty_discord_bot::storage::HelpForumSession {
        guild_id: guild_id.to_string(),
        forum_channel_id: forum_id.to_string(),
        thread_id: thread_id.to_string(),
        owner_user_id: author_user_id.to_string(),
        status: "open".to_string(),
        opened_at: Utc::now() - Duration::minutes(10),
        last_activity_at: Utc::now() - Duration::minutes(10),
        first_helper_response_at: None,
        closed_at: None,
        closing_reason: None,
        generation: 1,
    };
    state
        .app
        .storage
        .create_help_forum_session(session)
        .await
        .unwrap();

    // 4. Initial stats: 1 opened, 1 open_active, 1 unanswered
    let stats_res = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/help-forums/{forum_id}/stats"
        ))
        .header(COOKIE, &admin_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(stats_res.status(), reqwest::StatusCode::OK);
    let stats_json: serde_json::Value = stats_res.json().await.unwrap();
    assert_eq!(stats_json["stats"]["opened_count"], 1);
    assert_eq!(stats_json["stats"]["open_active_count"], 1);
    assert_eq!(stats_json["stats"]["unanswered_count"], 1);

    // 5. Simulate helper activity: record activity as helper
    state
        .app
        .storage
        .record_help_forum_activity(guild_id, thread_id, "888123456789012345", true)
        .await
        .unwrap();

    // 6. Stranger tries to close the post -> 403 Forbidden
    let stranger_close = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/help-forums/{thread_id}/close"
        ))
        .header(COOKIE, &stranger_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "status": "solved",
            "reason": "Closed by stranger"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(stranger_close.status(), reqwest::StatusCode::FORBIDDEN);

    // 7. Author closes their own post -> 200 OK
    let author_close = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/help-forums/{thread_id}/close"
        ))
        .header(COOKIE, &author_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "status": "solved",
            "reason": "Issue solved thanks!"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(author_close.status(), reqwest::StatusCode::OK);

    // 8. Stats updated: 1 opened, 0 active, 1 solved, 1 answered
    let stats_after = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/help-forums/{forum_id}/stats"
        ))
        .header(COOKIE, &admin_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(stats_after.status(), reqwest::StatusCode::OK);
    let stats_after_json: serde_json::Value = stats_after.json().await.unwrap();
    assert_eq!(stats_after_json["stats"]["solved_count"], 1);
    assert_eq!(stats_after_json["stats"]["answered_count"], 1);
    assert_eq!(stats_after_json["stats"]["open_active_count"], 0);

    // 9. Inactivity auto-close test via reconcile endpoint
    let idle_thread_id = "666000000000000001";
    let idle_session = rusty_discord_bot::storage::HelpForumSession {
        guild_id: guild_id.to_string(),
        forum_channel_id: forum_id.to_string(),
        thread_id: idle_thread_id.to_string(),
        owner_user_id: author_user_id.to_string(),
        status: "open".to_string(),
        opened_at: Utc::now() - Duration::hours(5),
        last_activity_at: Utc::now() - Duration::hours(2), // 2 hours idle, threshold is 60 mins
        first_helper_response_at: None,
        closed_at: None,
        closing_reason: None,
        generation: 1,
    };
    state
        .app
        .storage
        .create_help_forum_session(idle_session)
        .await
        .unwrap();

    let rec_res = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/help-forums/reconcile-idle"
        ))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(rec_res.status(), reqwest::StatusCode::OK);
    let rec_json: serde_json::Value = rec_res.json().await.unwrap();
    assert_eq!(rec_json["reconciled"], true);
    assert_eq!(rec_json["closed_count"], 1);

    // Verify session status is now 'inactive'
    let closed_session = state
        .app
        .storage
        .get_help_forum_session(guild_id, idle_thread_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(closed_session.status, "inactive");

    // 10. Delete help forum config
    let del_res = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/help-forums/{forum_id}"
        ))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(del_res.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn test_recruitment_security_and_concurrency_fixes() {
    let (state, _perms) = test_web_state().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let app = build_router(state.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve axum");
    });

    let g1 = "777000000000000001";
    let g2 = "777000000000000002";
    let admin_user = "888000000000000001";
    let candidate_user = "888000000000000002";
    let reviewer_user = "888000000000000003";

    let csrf = "test_csrf_token_recruitment_fixes";
    let session_admin = "session_admin_g1";
    let session_candidate = "session_candidate_g1";
    let session_reviewer = "session_reviewer_g1";

    // Setup sessions
    state
        .app
        .sessions
        .save_session(
            session_admin,
            DashboardUser {
                id: admin_user.to_owned(),
                username: "AdminG1".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: g1.to_owned(),
                name: "Guild 1".to_owned(),
                icon: None,
                owner: true,
                permissions: "8".to_owned(),
            }],
            csrf.to_owned(),
            Utc::now() + Duration::hours(24),
            None,
        )
        .await
        .unwrap();

    state
        .app
        .sessions
        .save_session(
            session_candidate,
            DashboardUser {
                id: candidate_user.to_owned(),
                username: "CandidateG1".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: g1.to_owned(),
                name: "Guild 1".to_owned(),
                icon: None,
                owner: false,
                permissions: "0".to_owned(),
            }],
            csrf.to_owned(),
            Utc::now() + Duration::hours(24),
            None,
        )
        .await
        .unwrap();

    state
        .app
        .sessions
        .save_session(
            session_reviewer,
            DashboardUser {
                id: reviewer_user.to_owned(),
                username: "ReviewerG1".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: g1.to_owned(),
                name: "Guild 1".to_owned(),
                icon: None,
                owner: false,
                permissions: "8".to_owned(),
            }],
            csrf.to_owned(),
            Utc::now() + Duration::hours(24),
            None,
        )
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let admin_cookie = format!("rusty_discord_bot_session={session_admin}");
    let cand_cookie = format!("rusty_discord_bot_session={session_candidate}");
    let rev_cookie = format!("rusty_discord_bot_session={session_reviewer}");

    // 1. Create track on G1 with max_active_per_user = 1 and auto_role_on_accept = true
    let track_res = client
        .post(format!("{base_url}/api/guilds/{g1}/recruitment/tracks"))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", csrf)
        .json(&serde_json::json!({
            "name": "Moderator Track",
            "slug": "mod-track",
            "description": "Recruiting mods",
            "target_role_id": "999000111222333444",
            "enabled": true,
            "config": {
                "max_active_per_user": 1,
                "cooldown_days": 0,
                "final_approvals": 1,
                "auto_role_on_accept": true,
                "questions": [
                    {
                        "id": "q1",
                        "label": "Why do you want to join?",
                        "field_type": "paragraph",
                        "required": true
                    }
                ]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(track_res.status(), reqwest::StatusCode::OK);
    let track_json: serde_json::Value = track_res.json().await.unwrap();
    let _track_id = track_json["track"]["id"].as_str().unwrap();

    // 2. Issue #189: Concurrency test - 10 parallel apply requests from the same user
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(10));
    let mut handles = Vec::new();
    for _ in 0..10 {
        let b = barrier.clone();
        let c = client.clone();
        let url = format!("{base_url}/api/guilds/{g1}/recruitment/tracks/mod-track/apply");
        let ck = cand_cookie.clone();
        let csrf_str = csrf.to_string();

        handles.push(tokio::spawn(async move {
            b.wait().await;
            c.post(url)
                .header(COOKIE, ck)
                .header("x-csrf-token", csrf_str)
                .json(&serde_json::json!({
                    "answers": [
                        { "question_id": "q1", "answer": "I want to help the server stay safe." }
                    ]
                }))
                .send()
                .await
                .unwrap()
        }));
    }

    let mut success_count = 0;
    let mut rejected_limit_count = 0;
    let mut app_id = None;

    for h in handles {
        let res = h.await.unwrap();
        if res.status() == reqwest::StatusCode::OK {
            success_count += 1;
            let j: serde_json::Value = res.json().await.unwrap();
            app_id = Some(j["application"]["id"].as_str().unwrap().to_string());
        } else if res.status() == reqwest::StatusCode::BAD_REQUEST {
            rejected_limit_count += 1;
        }
    }

    assert_eq!(
        success_count, 1,
        "Exactly one concurrent application must succeed when max_active=1"
    );
    assert_eq!(
        rejected_limit_count, 9,
        "Remaining 9 parallel attempts must be rejected with limit error"
    );
    let application_id = app_id.unwrap();

    // 3. Review application (approve)
    let review_res = client
        .post(format!(
            "{base_url}/api/guilds/{g1}/recruitment/applications/{application_id}/review"
        ))
        .header(COOKIE, &rev_cookie)
        .header("x-csrf-token", csrf)
        .json(&serde_json::json!({
            "recommendation": "approve",
            "note": "Looks like a great candidate!"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(review_res.status(), reqwest::StatusCode::OK);

    // 4. Issue #186 & #183: Transition to accepted (atomic transition and role enqueue)
    let accept_res = client
        .post(format!(
            "{base_url}/api/guilds/{g1}/recruitment/applications/{application_id}/transition"
        ))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", csrf)
        .json(&serde_json::json!({
            "expected_version": 1,
            "new_status": "accepted",
            "new_stage": "accepted",
            "decision_reason": "Welcome to staff!"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(accept_res.status(), reqwest::StatusCode::OK);
    let accept_json: serde_json::Value = accept_res.json().await.unwrap();
    assert_eq!(accept_json["application"]["status"], "accepted");
    assert_eq!(
        accept_json["application"]["fulfillment"]["status"],
        "pending"
    );

    // 5. Issue #183: Change accepted -> rejected cancels pending role fulfillment!
    let reject_res = client
        .post(format!(
            "{base_url}/api/guilds/{g1}/recruitment/applications/{application_id}/transition"
        ))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", csrf)
        .json(&serde_json::json!({
            "expected_version": 2,
            "new_status": "rejected",
            "new_stage": "rejected",
            "decision_reason": "Decision revoked before role granted"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(reject_res.status(), reqwest::StatusCode::OK);

    // Verify fulfillment task is cancelled!
    let parsed_aid = application_id.parse::<uuid::Uuid>().unwrap();
    let f_task = state
        .app
        .storage
        .get_role_fulfillment(parsed_aid)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        f_task.status, "cancelled",
        "Pending fulfillment MUST be cancelled when decision is revoked"
    );

    // 6. Issue #184: Tenant isolation in role fulfillment
    // Create a pending fulfillment for G2
    let g2_app_id = uuid::Uuid::new_v4();
    state
        .app
        .storage
        .upsert_role_fulfillment(rusty_discord_bot::storage::RecruitmentRoleFulfillment {
            application_id: g2_app_id,
            guild_id: g2.to_string(),
            user_id: "888000000000000099".to_string(),
            role_id: "999000000000000099".to_string(),
            status: "pending".to_string(),
            attempts: 0,
            last_error: None,
            next_attempt_at: Utc::now(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();

    // Admin of G1 triggers fulfillment endpoint for G1
    let g1_fulfill_res = client
        .post(format!(
            "{base_url}/api/guilds/{g1}/recruitment/fulfillment/process"
        ))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(g1_fulfill_res.status(), reqwest::StatusCode::OK);
    let g1_fulfill_json: serde_json::Value = g1_fulfill_res.json().await.unwrap();
    assert_eq!(
        g1_fulfill_json["processed"], 0,
        "G1 fulfillment must NOT process G2 tasks!"
    );

    // G2 task must remain completely untouched
    let g2_task = state
        .app
        .storage
        .get_role_fulfillment(g2_app_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(g2_task.status, "pending");
    assert_eq!(g2_task.attempts, 0);
}

#[tokio::test]
async fn test_asset_analytics_api_and_isolation() {
    let (state, _perms) = test_web_state().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let app = build_router(state.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve axum");
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");

    let g1 = "100000000000000001";
    let g2 = "200000000000000002";
    let user_id = "300000000000000003";

    let session_token = "asset_analytics_session_token";
    let csrf_token = "asset_csrf_token";

    state
        .app
        .sessions
        .save_session(
            session_token,
            DashboardUser {
                id: user_id.to_owned(),
                username: "AssetAdmin".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: g1.to_owned(),
                name: "Guild 1".to_owned(),
                icon: None,
                owner: false,
                permissions: "8".to_owned(),
            }],
            csrf_token.to_owned(),
            Utc::now() + Duration::hours(24),
            None,
        )
        .await
        .unwrap();

    let auth_cookie = format!("rusty_discord_bot_session={session_token}");

    // 1. Unauthenticated request -> 401
    let unauth_res = client
        .get(format!("{base_url}/api/guilds/{g1}/assets/usage"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth_res.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 2. Access to G2 (not member) -> 403 Forbidden
    let forbid_res = client
        .get(format!("{base_url}/api/guilds/{g2}/assets/usage"))
        .header(COOKIE, &auth_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(forbid_res.status(), reqwest::StatusCode::FORBIDDEN);

    // 3. Record assets in batcher for G1
    let party_emoji = "111222333444555666";
    let spin_emoji = "222333444555666777";
    let sticker_id = "333444555666777888";

    // Message 1: 3x party emoji + 1x spin animated emoji
    let msg_content = format!(
        "<:party:{party_emoji}> wow <:party:{party_emoji}> awesome <:party:{party_emoji}> <a:spin:{spin_emoji}>"
    );
    let stickers = vec![rusty_discord_bot::asset_analytics::ParsedSticker {
        id: sticker_id.to_string(),
        name: "cool_cat".to_string(),
        animated: true,
    }];

    // Record through batcher
    let count = state.app.asset_analytics.record_message(
        g1,
        "555000111222333444",
        &msg_content,
        &stickers,
        Some(&[party_emoji.to_string()]), // spin is external, party is guild-owned
        Some(&[sticker_id.to_string()]),
    );
    assert_eq!(count, 5); // 3 party + 1 spin + 1 sticker

    // Duplicate message from gateway reconnect must be ignored by dedupe
    let dup_count = state.app.asset_analytics.record_message(
        g1,
        "555000111222333444",
        &msg_content,
        &stickers,
        Some(&[party_emoji.to_string()]),
        Some(&[sticker_id.to_string()]),
    );
    assert_eq!(
        dup_count, 0,
        "Duplicate message_id must be ignored by bounded LRU"
    );

    // 4. Flush batch via REST API
    let flush_res = client
        .post(format!("{base_url}/api/guilds/{g1}/assets/flush"))
        .header(COOKIE, &auth_cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(flush_res.status(), reqwest::StatusCode::OK);
    let flush_json: serde_json::Value = flush_res.json().await.unwrap();
    assert_eq!(flush_json["flushed"], true);
    assert_eq!(flush_json["items_flushed"], 3); // 3 daily rows (party, spin, sticker)

    // 5. Query usage for emojis
    let usage_res = client
        .get(format!(
            "{base_url}/api/guilds/{g1}/assets/usage?kind=emoji&period=30d"
        ))
        .header(COOKIE, &auth_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(usage_res.status(), reqwest::StatusCode::OK);
    let usage_json: serde_json::Value = usage_res.json().await.unwrap();

    assert_eq!(usage_json["guild_id"], g1);
    assert_eq!(usage_json["total_usages"], 4); // 3 party + 1 spin
    assert_eq!(usage_json["unique_assets_used"], 2);

    let top = usage_json["top"].as_array().unwrap();
    assert_eq!(top.len(), 2);
    assert_eq!(top[0]["asset_id"], party_emoji);
    assert_eq!(top[0]["period_usage"], 3);
    assert_eq!(top[0]["is_guild_owned"], true);
    assert_eq!(top[0]["name"], "party");
    assert_eq!(top[0]["animated"], false);

    assert_eq!(top[1]["asset_id"], spin_emoji);
    assert_eq!(top[1]["period_usage"], 1);
    assert_eq!(top[1]["is_guild_owned"], false);
    assert_eq!(top[1]["name"], "spin");
    assert_eq!(top[1]["animated"], true);

    // Privacy verification: no user_id or message content anywhere in payload
    let raw_response = serde_json::to_string(&usage_json).unwrap();
    assert!(
        !raw_response.contains("555000111222333444"),
        "Message ID must not be leaked"
    );
    assert!(
        !raw_response.contains("AssetAdmin"),
        "User name must not be stored in analytics"
    );
    assert!(
        !raw_response.contains(user_id),
        "User ID must not be stored in analytics"
    );
    assert!(
        !raw_response.contains("wow"),
        "Message content must not be stored in analytics"
    );

    // 6. Query inventory
    let inv_res = client
        .get(format!("{base_url}/api/guilds/{g1}/assets/inventory"))
        .header(COOKIE, &auth_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(inv_res.status(), reqwest::StatusCode::OK);
    let inv_json: serde_json::Value = inv_res.json().await.unwrap();
    let items = inv_json.as_array().unwrap();
    assert_eq!(items.len(), 3); // party, spin, cool_cat sticker

    // 7. Issue #200: Test inventory pagination (limit and cursor)
    let paged_res = client
        .get(format!(
            "{base_url}/api/guilds/{g1}/assets/inventory?limit=2"
        ))
        .header(COOKIE, &auth_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(paged_res.status(), reqwest::StatusCode::OK);
    let paged_items = paged_res.json::<Vec<serde_json::Value>>().await.unwrap();
    assert_eq!(paged_items.len(), 2);

    let first_id = paged_items[0]["asset_id"].as_str().unwrap();
    let cursor_res = client
        .get(format!(
            "{base_url}/api/guilds/{g1}/assets/inventory?cursor={first_id}&limit=1"
        ))
        .header(COOKIE, &auth_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(cursor_res.status(), reqwest::StatusCode::OK);
    let cursor_items = cursor_res.json::<Vec<serde_json::Value>>().await.unwrap();
    assert_eq!(cursor_items.len(), 1);
    assert_eq!(cursor_items[0]["asset_id"], paged_items[1]["asset_id"]);
}

#[tokio::test]
async fn test_help_forum_hardening_and_lifecycle() {
    let (state, _perms) = test_web_state().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let app = build_router(state.clone());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve axum");
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");

    let g1 = "666000000000000001";
    let g2 = "666000000000000002";
    let user_id = "777000000000000001";
    let forum_id = "888000000000000001";
    let thread_id = "999000000000000001";
    let csrf_token = "csrf_token_help_forum_hardening";

    state
        .app
        .sessions
        .save_session(
            "session_hardening",
            DashboardUser {
                id: user_id.to_owned(),
                username: "HelpAdmin".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: g1.to_owned(),
                name: "Guild 1".to_owned(),
                icon: None,
                owner: true,
                permissions: "8".to_owned(),
            }],
            csrf_token.to_owned(),
            Utc::now() + Duration::hours(24),
            None,
        )
        .await
        .unwrap();

    let auth_cookie = "rusty_discord_bot_session=session_hardening";

    // 1. Issue #195: Malformed forum_channel_id is rejected with 400
    let invalid_config = client
        .post(format!("{base_url}/api/guilds/{g1}/help-forums"))
        .header(COOKIE, auth_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "forum_channel_id": "not-a-valid-snowflake",
            "enabled": true,
            "idle_close_minutes": 60,
            "owner_can_close": true,
            "helper_role_ids": [],
            "lock_on_close": true,
            "archive_on_close": true,
            "require_question_tag": true,
            "welcome_message": "Welcome @everyone to the forum!"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_config.status(), reqwest::StatusCode::BAD_REQUEST);

    // 2. Issue #193: Save config with require_question_tag = true
    let valid_config = client
        .post(format!("{base_url}/api/guilds/{g1}/help-forums"))
        .header(COOKIE, auth_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "forum_channel_id": forum_id,
            "enabled": true,
            "idle_close_minutes": 60,
            "owner_can_close": true,
            "helper_role_ids": [],
            "lock_on_close": true,
            "archive_on_close": true,
            "require_question_tag": true,
            "welcome_message": "Welcome to the forum!"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(valid_config.status(), reqwest::StatusCode::OK);

    // Verify list_enabled_help_forum_guilds returns G1 (Issue #190)
    let enabled_guilds = state
        .app
        .storage
        .list_enabled_help_forum_guilds()
        .await
        .unwrap();
    assert!(enabled_guilds.contains(&g1.to_string()));
    assert!(!enabled_guilds.contains(&g2.to_string()));

    // 3. Issue #193: Create session with 'needs_tag' status
    let session = rusty_discord_bot::storage::HelpForumSession {
        guild_id: g1.to_string(),
        forum_channel_id: forum_id.to_string(),
        thread_id: thread_id.to_string(),
        owner_user_id: user_id.to_string(),
        status: "needs_tag".to_string(),
        opened_at: Utc::now() - Duration::hours(2),
        last_activity_at: Utc::now() - Duration::hours(2),
        first_helper_response_at: None,
        closed_at: None,
        closing_reason: None,
        generation: 1,
    };
    state
        .app
        .storage
        .create_help_forum_session(session)
        .await
        .unwrap();

    let fetched = state
        .app
        .storage
        .get_help_forum_session(g1, thread_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.status, "needs_tag");

    // 4. Issue #192: Dashboard close endpoint closes the post using close_help_forum_session_with_discord
    let close_res = client
        .post(format!(
            "{base_url}/api/guilds/{g1}/help-forums/{thread_id}/close"
        ))
        .header(COOKIE, auth_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "status": "solved",
            "reason": "Solved via dashboard"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(close_res.status(), reqwest::StatusCode::OK);
    let close_json: serde_json::Value = close_res.json().await.unwrap();
    assert_eq!(close_json["closed"], true);

    let session_closed = state
        .app
        .storage
        .get_help_forum_session(g1, thread_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session_closed.status, "solved");
    assert_eq!(
        session_closed.closing_reason.as_deref(),
        Some("Solved via dashboard")
    );

    // 5. Issue #190 & #191: Test background reconciler logic across guilds
    let idle_thread = "999000000000000002";
    let idle_session = rusty_discord_bot::storage::HelpForumSession {
        guild_id: g1.to_string(),
        forum_channel_id: forum_id.to_string(),
        thread_id: idle_thread.to_string(),
        owner_user_id: user_id.to_string(),
        status: "open".to_string(),
        opened_at: Utc::now() - Duration::hours(5),
        last_activity_at: Utc::now() - Duration::hours(3), // idle > 60 mins
        first_helper_response_at: None,
        closed_at: None,
        closing_reason: None,
        generation: 1,
    };
    state
        .app
        .storage
        .create_help_forum_session(idle_session)
        .await
        .unwrap();

    // Reconcile across all enabled guilds
    let mut total_reconciled = 0;
    for gid in state
        .app
        .storage
        .list_enabled_help_forum_guilds()
        .await
        .unwrap()
    {
        total_reconciled += rusty_discord_bot::help_forum::reconcile_idle_help_forums(
            &state.app.storage,
            None,
            &gid,
        )
        .await
        .unwrap();
    }
    assert_eq!(total_reconciled, 1);

    let idle_after = state
        .app
        .storage
        .get_help_forum_session(g1, idle_thread)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(idle_after.status, "inactive");
    assert_eq!(
        idle_after.closing_reason.as_deref(),
        Some("inactivity_auto_close")
    );
}

#[tokio::test]
async fn test_giveaways_member_access_and_admin_boundary() {
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

    let guild_id = "111222333444555666";
    let giveaway = state
        .app
        .storage
        .create_giveaway(
            guild_id,
            "channel_123",
            "Discord Nitro",
            1,
            Utc::now() + Duration::days(7),
        )
        .await
        .expect("create test giveaway");

    // Regular member session: member of guild_id, but permissions = "0" (no MANAGE_GUILD)
    let member_session_id = "regular_member_session_token";
    let member_user = DashboardUser {
        id: "member_999".to_owned(),
        username: "regular_joe".to_owned(),
        global_name: None,
        avatar: None,
    };
    let member_guilds = vec![DashboardGuild {
        id: guild_id.to_owned(),
        name: "Community Guild".to_owned(),
        icon: None,
        owner: false,
        permissions: "0".to_owned(), // regular member, no MANAGE_GUILD
    }];

    state
        .app
        .sessions
        .save_session(
            member_session_id,
            member_user,
            member_guilds,
            "csrf_member_123".to_owned(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .expect("save member session");

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let session_cookie = format!("rusty_discord_bot_session={member_session_id}");

    // 1. Regular member can view the giveaway (GET /api/guilds/{guild_id}/giveaways/{giveaway_id}) -> 200 OK
    let get_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}",
            giveaway.id
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("get giveaway");
    assert_eq!(get_resp.status(), reqwest::StatusCode::OK);
    let ga_json: serde_json::Value = get_resp.json().await.unwrap();
    assert_eq!(ga_json["prize"], "Discord Nitro");

    // 2. Regular member can join the giveaway with CSRF (POST /api/guilds/{guild_id}/giveaways/{giveaway_id}/join) -> 200 OK, joined: true
    let join_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}/join",
            giveaway.id
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", "csrf_member_123")
        .send()
        .await
        .expect("join giveaway");
    assert_eq!(join_resp.status(), reqwest::StatusCode::OK);
    let join_json: serde_json::Value = join_resp.json().await.unwrap();
    assert_eq!(join_json["joined"], true);

    // 3. Second join attempt -> 200 OK, joined: false
    let join_again_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}/join",
            giveaway.id
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", "csrf_member_123")
        .send()
        .await
        .expect("join giveaway again");
    assert_eq!(join_again_resp.status(), reqwest::StatusCode::OK);
    let join_again_json: serde_json::Value = join_again_resp.json().await.unwrap();
    assert_eq!(join_again_json["joined"], false);

    // 4. Regular member cannot create giveaways -> 403 Forbidden
    let create_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/giveaways"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", "csrf_member_123")
        .json(&serde_json::json!({
            "channel_id": "channel_123",
            "prize": "Hacked Nitro",
            "winners_count": 1,
            "duration_minutes": 60
        }))
        .send()
        .await
        .expect("create giveaway as member");
    assert_eq!(create_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 5. Regular member cannot reroll giveaways -> 403 Forbidden
    let reroll_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}/reroll",
            giveaway.id
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", "csrf_member_123")
        .send()
        .await
        .expect("reroll giveaway as member");
    assert_eq!(reroll_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 6. Non-member access to other guild -> 403 Forbidden
    let other_guild_resp = client
        .get(format!(
            "{base_url}/api/guilds/999888777666/giveaways/{}",
            giveaway.id
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("other guild giveaway");
    assert_eq!(other_guild_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 7. Joining an expired giveaway (expires_at in the past, ended=false) must return joined: false (Issue #217)
    let expired_ga = state
        .app
        .storage
        .create_giveaway(
            guild_id,
            "channel_123",
            "Expired Nitro",
            1,
            Utc::now() - Duration::minutes(5),
        )
        .await
        .expect("create expired giveaway");

    let join_expired_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}/join",
            expired_ga.id
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", "csrf_member_123")
        .send()
        .await
        .expect("join expired giveaway");
    assert_eq!(join_expired_resp.status(), reqwest::StatusCode::OK);
    let join_expired_json: serde_json::Value = join_expired_resp.json().await.unwrap();
    assert_eq!(join_expired_json["joined"], false);

    // Verify participant wasn't added in storage
    let ga_after = state
        .app
        .storage
        .get_giveaway(expired_ga.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ga_after.participants_json.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_guild_events_stream_security_and_reauth_on_revoke() {
    let (state, perms_cache) = test_web_state().await;

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

    let guild_id = "stream_guild_100";
    let user_id = "stream_user_200";
    let session_id = "stream_session_token_xyz";

    let user = DashboardUser {
        id: user_id.to_owned(),
        username: "stream_admin".to_owned(),
        global_name: None,
        avatar: None,
    };
    let guilds = vec![DashboardGuild {
        id: guild_id.to_owned(),
        name: "Stream Guild".to_owned(),
        icon: None,
        owner: false,
        permissions: "32".to_owned(), // MANAGE_GUILD
    }];

    state
        .app
        .sessions
        .save_session(
            session_id,
            user,
            guilds,
            "csrf_stream_token".to_owned(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .expect("save stream session");

    // Populate perms_cache
    perms_cache.insert(
        (user_id.to_owned(), guild_id.to_owned()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    // Connect to SSE stream
    let mut resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/events"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("connect to sse");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // Initial connected event chunk
    let first_chunk = resp
        .chunk()
        .await
        .expect("receive chunk")
        .expect("initial chunk");
    let first_text = String::from_utf8_lossy(&first_chunk);
    assert!(first_text.contains("connected"));

    // Emit live event while authorized
    state
        .app
        .guild_events
        .send(rusty_discord_bot::state::GuildRealtimeEvent {
            guild_id: guild_id.to_owned(),
            event_type: "case_created".to_owned(),
            payload: serde_json::json!({ "case_number": 42 }),
        })
        .expect("send live event");

    let event_chunk = resp
        .chunk()
        .await
        .expect("receive chunk")
        .expect("event chunk");
    let event_text = String::from_utf8_lossy(&event_chunk);
    assert!(event_text.contains("case_created"));

    // Revoke session from store (e.g. logout or session invalidation)
    state
        .app
        .sessions
        .delete_session(session_id)
        .await
        .expect("delete session");

    // Also invalidate permissions cache
    perms_cache.insert(
        (user_id.to_owned(), guild_id.to_owned()),
        CachedGuildPermission {
            can_manage: false,
            checked_at: Utc::now(),
        },
    );

    // Emit another event after revocation
    state
        .app
        .guild_events
        .send(rusty_discord_bot::state::GuildRealtimeEvent {
            guild_id: guild_id.to_owned(),
            event_type: "case_created".to_owned(),
            payload: serde_json::json!({ "case_number": 43 }),
        })
        .expect("send event after revoke");

    // The stream must terminate (either next chunk is None, or EOF reached)
    let terminated_chunk = resp.chunk().await.expect("stream chunk read");
    assert!(
        terminated_chunk.is_none(),
        "stream should terminate on session revocation, but got: {:?}",
        terminated_chunk.map(|c| String::from_utf8_lossy(&c).to_string())
    );
}

#[tokio::test]
async fn test_guild_events_stream_task_cancellation_on_client_drop() {
    let (state, perms_cache) = test_web_state().await;

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

    let guild_id = "stream_guild_drop_101";
    let user_id = "stream_user_drop_202";
    let session_id = "stream_session_drop_token";

    let user = DashboardUser {
        id: user_id.to_owned(),
        username: "stream_drop_admin".to_owned(),
        global_name: None,
        avatar: None,
    };
    let guilds = vec![DashboardGuild {
        id: guild_id.to_owned(),
        name: "Stream Drop Guild".to_owned(),
        icon: None,
        owner: false,
        permissions: "32".to_owned(), // MANAGE_GUILD
    }];

    state
        .app
        .sessions
        .save_session(
            session_id,
            user,
            guilds,
            "csrf_stream_drop_token".to_owned(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .expect("save drop stream session");

    perms_cache.insert(
        (user_id.to_owned(), guild_id.to_owned()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    let baseline_subscribers = state.app.guild_events.receiver_count();

    // 1. Connect to SSE stream
    let mut resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/events"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("connect to sse");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // Initial connected chunk
    let first_chunk = resp
        .chunk()
        .await
        .expect("receive chunk")
        .expect("initial chunk");
    let first_text = String::from_utf8_lossy(&first_chunk);
    assert!(first_text.contains("connected"));

    // Subscriptions count should now be baseline + 1
    assert_eq!(
        state.app.guild_events.receiver_count(),
        baseline_subscribers + 1
    );

    // 2. Drop the client response WITHOUT publishing any events and WITHOUT revoking session
    drop(resp);

    // 3. The worker must observe tx.closed() and terminate promptly, returning receiver_count to baseline
    let finished_cleanup = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if state.app.guild_events.receiver_count() == baseline_subscribers {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        }
    })
    .await;
    assert!(
        finished_cleanup.is_ok(),
        "Worker task failed to cancel on client drop within timeout; active receivers: {}",
        state.app.guild_events.receiver_count()
    );
}

#[tokio::test]
async fn test_scheduled_events_member_access_and_admin_boundary() {
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

    let guild_id = "111222333444555666";
    let other_guild_id = "222333444555666777";

    let ev = state
        .app
        .storage
        .create_scheduled_event(
            guild_id,
            "Community Game Night",
            "Board games and fun",
            Utc::now() + Duration::days(1),
            Some(Utc::now() + Duration::days(1) + Duration::hours(2)),
            "UTC",
            Some(10),
        )
        .await
        .expect("create test event");

    let other_ev = state
        .app
        .storage
        .create_scheduled_event(
            other_guild_id,
            "Other Guild Meeting",
            "Secret admin meeting",
            Utc::now() + Duration::days(2),
            None,
            "UTC",
            Some(5),
        )
        .await
        .expect("create other guild event");

    // Regular member session (member of guild_id, permissions = "0", no MANAGE_GUILD)
    let member_session_id = "events_regular_member_session";
    let member_user = DashboardUser {
        id: "member_555".to_owned(),
        username: "event_goer".to_owned(),
        global_name: None,
        avatar: None,
    };
    let member_guilds = vec![DashboardGuild {
        id: guild_id.to_owned(),
        name: "Community Guild".to_owned(),
        icon: None,
        owner: false,
        permissions: "0".to_owned(),
    }];
    let member_csrf = "csrf_events_member_555";

    state
        .app
        .sessions
        .save_session(
            member_session_id,
            member_user,
            member_guilds,
            member_csrf.to_owned(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .expect("save member session");

    // Non-member session (member of other_guild_id only)
    let non_member_session_id = "events_non_member_session";
    let non_member_user = DashboardUser {
        id: "outsider_777".to_owned(),
        username: "outsider".to_owned(),
        global_name: None,
        avatar: None,
    };
    let non_member_guilds = vec![DashboardGuild {
        id: other_guild_id.to_owned(),
        name: "Other Guild".to_owned(),
        icon: None,
        owner: false,
        permissions: "0".to_owned(),
    }];
    let non_member_csrf = "csrf_outsider_777";

    state
        .app
        .sessions
        .save_session(
            non_member_session_id,
            non_member_user,
            non_member_guilds,
            non_member_csrf.to_owned(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .expect("save non member session");

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let member_cookie = format!("rusty_discord_bot_session={member_session_id}");
    let non_member_cookie = format!("rusty_discord_bot_session={non_member_session_id}");

    // 1. Regular member CAN view events list (GET /api/guilds/{guild_id}/scheduled-events) -> 200 OK
    let list_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &member_cookie)
        .send()
        .await
        .expect("list events");
    assert_eq!(list_resp.status(), reqwest::StatusCode::OK);
    let events_json: Vec<serde_json::Value> = list_resp.json().await.unwrap();
    assert_eq!(events_json.len(), 1);
    assert_eq!(events_json[0]["title"], "Community Game Night");

    // 2. Regular member CAN join event roster with CSRF -> 200 OK
    let join_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{}/roster/join",
            ev.id
        ))
        .header(COOKIE, &member_cookie)
        .header("X-CSRF-Token", member_csrf)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("join roster");
    assert_eq!(join_resp.status(), reqwest::StatusCode::OK);
    let roster_json: serde_json::Value = join_resp.json().await.unwrap();
    assert_eq!(roster_json["user_id"], "member_555");
    assert_eq!(roster_json["role_type"], "main");

    // 3. Regular member CAN leave event roster with CSRF -> 200 OK
    let leave_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{}/roster/leave",
            ev.id
        ))
        .header(COOKIE, &member_cookie)
        .header("X-CSRF-Token", member_csrf)
        .send()
        .await
        .expect("leave roster");
    assert_eq!(leave_resp.status(), reqwest::StatusCode::OK);
    let leave_json: serde_json::Value = leave_resp.json().await.unwrap();
    assert_eq!(leave_json["left"], true);

    // 4. Regular member CANNOT create event -> 403 Forbidden (requires MANAGE_GUILD)
    let create_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &member_cookie)
        .header("X-CSRF-Token", member_csrf)
        .json(&serde_json::json!({
            "title": "Unauthorized Event",
            "description": "I have no admin rights",
            "start_time": Utc::now() + Duration::days(3),
            "timezone": "UTC"
        }))
        .send()
        .await
        .expect("create event unauthorized");
    assert_eq!(create_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 5. POST join/leave without CSRF or with invalid CSRF -> 403 Forbidden
    let no_csrf_join = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{}/roster/join",
            ev.id
        ))
        .header(COOKIE, &member_cookie)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("join without csrf");
    assert_eq!(no_csrf_join.status(), reqwest::StatusCode::FORBIDDEN);

    let bad_csrf_join = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{}/roster/join",
            ev.id
        ))
        .header(COOKIE, &member_cookie)
        .header("X-CSRF-Token", "wrong_csrf_token")
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("join bad csrf");
    assert_eq!(bad_csrf_join.status(), reqwest::StatusCode::FORBIDDEN);

    let no_csrf_leave = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{}/roster/leave",
            ev.id
        ))
        .header(COOKIE, &member_cookie)
        .send()
        .await
        .expect("leave without csrf");
    assert_eq!(no_csrf_leave.status(), reqwest::StatusCode::FORBIDDEN);

    // 6. Non-member session accessing guild_id -> 403 Forbidden
    let non_member_list = client
        .get(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &non_member_cookie)
        .send()
        .await
        .expect("non member list");
    assert_eq!(non_member_list.status(), reqwest::StatusCode::FORBIDDEN);

    let non_member_join = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{}/roster/join",
            ev.id
        ))
        .header(COOKIE, &non_member_cookie)
        .header("X-CSRF-Token", non_member_csrf)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("non member join");
    assert_eq!(non_member_join.status(), reqwest::StatusCode::FORBIDDEN);

    // 7. Cross-guild tenant isolation: accessing other guild's event under guild_id -> 404 Not Found
    let cross_guild_join = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{}/roster/join",
            other_ev.id
        ))
        .header(COOKIE, &member_cookie)
        .header("X-CSRF-Token", member_csrf)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("cross guild join");
    assert_eq!(cross_guild_join.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_scheduled_events_live_mode_discord_verification() {
    let mock_status = std::sync::Arc::new(std::sync::atomic::AtomicU16::new(200));
    let mock_status_clone = mock_status.clone();

    // 1. Mock Discord HTTP API server for GET /guilds/{guild_id}/members/{user_id}
    let discord_mock_app = axum::Router::new().route(
        "/guilds/{guild_id}/members/{user_id}",
        axum::routing::get(move || {
            let status_code = mock_status_clone.clone();
            async move {
                match status_code.load(std::sync::atomic::Ordering::SeqCst) {
                    200 => (
                        axum::http::StatusCode::OK,
                        axum::Json(serde_json::json!({
                            "user": {"id": "888000000000000002"},
                            "roles": []
                        })),
                    ),
                    404 => (
                        axum::http::StatusCode::NOT_FOUND,
                        axum::Json(serde_json::json!({
                            "code": 10007,
                            "message": "Unknown Member"
                        })),
                    ),
                    _ => (
                        axum::http::StatusCode::SERVICE_UNAVAILABLE,
                        axum::Json(serde_json::json!({
                            "message": "Discord upstream error"
                        })),
                    ),
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

    // 2. Configure app state in live mode (offline_mode = false, real token format)
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
    web_state
        .app
        .is_gateway_ready
        .store(true, std::sync::atomic::Ordering::SeqCst);

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
    let session_id = "sess_live_events_test";
    let csrf_token = "csrf_live_events_test";

    let ev = web_state
        .app
        .storage
        .create_scheduled_event(
            guild_id,
            "Live Guild Event",
            "Testing live verification",
            Utc::now() + Duration::days(1),
            Some(Utc::now() + Duration::days(1) + Duration::hours(1)),
            "UTC",
            Some(20),
        )
        .await
        .expect("create live event");

    // Session has guild snapshot from login time, but permissions = "0" (no MANAGE_GUILD)
    web_state
        .app
        .sessions
        .save_session(
            session_id,
            DashboardUser {
                id: user_id.to_string(),
                username: "live_member".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Live Guild".to_string(),
                icon: None,
                owner: false,
                permissions: "0".to_string(),
            }],
            csrf_token.to_string(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .expect("save live session");

    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    // Case 1: Member verified by Discord API (200 OK) -> Access granted
    mock_status.store(200, std::sync::atomic::Ordering::SeqCst);
    let list_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("list events live");
    assert_eq!(list_resp.status(), reqwest::StatusCode::OK);

    let join_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{}/roster/join",
            ev.id
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("join live");
    assert_eq!(join_resp.status(), reqwest::StatusCode::OK);

    // Case 2: Member was kicked/left guild -> Discord API returns 404 Not Found -> 403 Forbidden
    mock_status.store(404, std::sync::atomic::Ordering::SeqCst);
    let list_resp_404 = client
        .get(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("list events 404");
    assert_eq!(list_resp_404.status(), reqwest::StatusCode::FORBIDDEN);

    let join_resp_404 = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{}/roster/join",
            ev.id
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("join live 404");
    assert_eq!(join_resp_404.status(), reqwest::StatusCode::FORBIDDEN);

    // Case 3: Discord API is down / errors out (503 Service Unavailable) -> Fails closed with 403 Forbidden
    mock_status.store(503, std::sync::atomic::Ordering::SeqCst);
    let list_resp_503 = client
        .get(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("list events 503");
    assert_eq!(list_resp_503.status(), reqwest::StatusCode::FORBIDDEN);

    let join_resp_503 = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{}/roster/join",
            ev.id
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("join live 503");
    assert_eq!(join_resp_503.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_ensure_guild_member_access_rejects_session_missing_guild_even_with_fresh_manage_cache()
 {
    let (state, perms_cache) = test_web_state().await;

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

    let guild_id = "555000000000000001";
    let user_id = "555000000000000002";

    // 1. Pre-seed permissions_cache with can_manage: true for (user_id, guild_id)
    perms_cache.insert(
        (user_id.to_string(), guild_id.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    // 2. Create session for user_id where guilds does NOT contain guild_id (e.g. member left/removed)
    let session_id = "sess_left_guild_manage_cache";
    let csrf_token = "csrf_left_guild_123";
    state
        .app
        .sessions
        .save_session(
            session_id,
            DashboardUser {
                id: user_id.to_string(),
                username: "former_admin".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![], // guilds list is empty!
            csrf_token.to_string(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .expect("save session");

    // Helper call directly: must return 403 Forbidden
    let session = state
        .app
        .sessions
        .get_session(session_id)
        .await
        .expect("get session")
        .expect("session exists");
    let check_res =
        rusty_discord_bot::web::auth::ensure_guild_member_access(&state, &session, guild_id).await;
    assert!(check_res.is_err());
    assert_eq!(
        check_res.err().unwrap().status,
        axum::http::StatusCode::FORBIDDEN
    );

    // 3. HTTP tests: GET scheduled-events returns 403 Forbidden
    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    let ev = state
        .app
        .storage
        .create_scheduled_event(
            guild_id,
            "Secret Guild Event",
            "Should not be visible without guild in session",
            Utc::now() + Duration::days(1),
            None,
            "UTC",
            Some(10),
        )
        .await
        .expect("create event");

    let list_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("list events");
    assert_eq!(list_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // POST join returns 403 Forbidden and makes zero changes
    let join_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{}/roster/join",
            ev.id
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("join event");
    assert_eq!(join_resp.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_scheduled_events_rsvp_point_lookup_and_tenant_isolation() {
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

    let guild_id = "666000000000000001";
    let other_guild_id = "666000000000000002";

    // Create 20 events in guild_id
    let mut target_event_id = None;
    for idx in 0..20 {
        let ev = state
            .app
            .storage
            .create_scheduled_event(
                guild_id,
                &format!("Event #{idx}"),
                "Description",
                Utc::now() + Duration::days(idx + 1),
                None,
                "UTC",
                Some(15),
            )
            .await
            .expect("create event");
        if idx == 7 {
            target_event_id = Some(ev.id);
        }
    }
    let target_event_id = target_event_id.unwrap();

    // Create 1 event in other_guild_id
    let other_ev = state
        .app
        .storage
        .create_scheduled_event(
            other_guild_id,
            "Other Guild Event",
            "Other",
            Utc::now() + Duration::days(5),
            None,
            "UTC",
            Some(10),
        )
        .await
        .expect("create other event");

    // Session for guild_id member
    let session_id = "sess_point_lookup_test";
    let csrf_token = "csrf_point_lookup_test";
    let user_id = "666000000000000003";

    state
        .app
        .sessions
        .save_session(
            session_id,
            DashboardUser {
                id: user_id.to_string(),
                username: "point_tester".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Guild 666".to_string(),
                icon: None,
                owner: false,
                permissions: "0".to_string(),
            }],
            csrf_token.to_string(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .expect("save session");

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    // 1. Join target event in guild_id -> 200 OK
    let join_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{target_event_id}/roster/join"
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("join target event");
    assert_eq!(join_resp.status(), reqwest::StatusCode::OK);

    // 2. Leave target event in guild_id -> 200 OK
    let leave_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{target_event_id}/roster/leave"
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .send()
        .await
        .expect("leave target event");
    assert_eq!(leave_resp.status(), reqwest::StatusCode::OK);

    // 3. Trying to join other_guild's event under guild_id -> 404 Not Found
    let cross_join = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{}/roster/join",
            other_ev.id
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("cross join");
    assert_eq!(cross_join.status(), reqwest::StatusCode::NOT_FOUND);

    // 4. Trying to join non-existent UUID -> 404 Not Found
    let random_uuid = uuid::Uuid::new_v4();
    let non_existent_join = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{random_uuid}/roster/join"
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("non existent join");
    assert_eq!(non_existent_join.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_recruitment_cooldown_checks_latest_rejection_decision() {
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

    let guild_id = "777000000000000001";
    let candidate_user = "777000000000000002";
    let candidate_session_id = "session_candidate_cd_test";
    let csrf = "test_csrf_token_recruitment_cd";

    state
        .app
        .sessions
        .save_session(
            candidate_session_id,
            DashboardUser {
                id: candidate_user.to_owned(),
                username: "CandidateCD".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Guild 777".to_owned(),
                icon: None,
                owner: false,
                permissions: "0".to_owned(),
            }],
            csrf.to_owned(),
            Utc::now() + Duration::hours(24),
            None,
        )
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let cand_cookie = format!("rusty_discord_bot_session={candidate_session_id}");

    // Create track with cooldown_days = 3 and max_active_per_user = 2
    let track = rusty_discord_bot::recruitment::RecruitmentTrack {
        id: uuid::Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        slug: "cooldown-track".to_string(),
        name: "Cooldown Track".to_string(),
        target_role_id: Some("999000111222333444".to_string()),
        enabled: true,
        config: sqlx::types::Json(rusty_discord_bot::recruitment::RecruitmentTrackConfig {
            max_active_per_user: 2,
            cooldown_days: 3,
            final_approvals: 1,
            auto_role_on_accept: false,
            questions: vec![rusty_discord_bot::recruitment::RecruitmentQuestion {
                id: "q1".to_string(),
                label: "Question 1".to_string(),
                field_type: "text".to_string(),
                required: true,
                options: vec![],
            }],
            ..Default::default()
        }),
        version: 1,
        created_at: Utc::now() - Duration::days(50),
        updated_at: Utc::now() - Duration::days(50),
    };
    state
        .app
        .storage
        .upsert_recruitment_track(track.clone())
        .await
        .unwrap();

    // Application A: submitted 40 days ago, rejected 1 day ago (decided_at = Utc::now() - 1 day -> inside cooldown)
    let app_a = rusty_discord_bot::recruitment::RecruitmentApplication {
        id: uuid::Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        track_id: track.id,
        applicant_user_id: candidate_user.to_string(),
        track_version: 1,
        status: "rejected".to_string(),
        submitted_answers: sqlx::types::Json(vec![
            rusty_discord_bot::recruitment::SubmittedAnswer {
                question_id: "q1".to_string(),
                answer: "Answer A".to_string(),
            },
        ]),
        current_stage: "screening".to_string(),
        version: 2,
        submitted_at: Utc::now() - Duration::days(40),
        updated_at: Utc::now() - Duration::days(1),
        decided_at: Some(Utc::now() - Duration::days(1)),
        decision_reason: Some("Not enough experience".to_string()),
        decided_by: Some("admin_1".to_string()),
    };
    state
        .app
        .storage
        .create_recruitment_application(app_a.clone())
        .await
        .unwrap();

    // Application B: submitted 30 days ago, rejected 10 days ago (decided_at = Utc::now() - 10 days -> outside cooldown)
    let app_b = rusty_discord_bot::recruitment::RecruitmentApplication {
        id: uuid::Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        track_id: track.id,
        applicant_user_id: candidate_user.to_string(),
        track_version: 1,
        status: "rejected".to_string(),
        submitted_answers: sqlx::types::Json(vec![
            rusty_discord_bot::recruitment::SubmittedAnswer {
                question_id: "q1".to_string(),
                answer: "Answer B".to_string(),
            },
        ]),
        current_stage: "screening".to_string(),
        version: 2,
        submitted_at: Utc::now() - Duration::days(30),
        updated_at: Utc::now() - Duration::days(10),
        decided_at: Some(Utc::now() - Duration::days(10)),
        decision_reason: Some("Earlier rejection".to_string()),
        decided_by: Some("admin_1".to_string()),
    };
    state
        .app
        .storage
        .create_recruitment_application(app_b.clone())
        .await
        .unwrap();

    // Application C: rejected with decided_at = None (must be handled gracefully without crashing or ignoring A)
    let app_c = rusty_discord_bot::recruitment::RecruitmentApplication {
        id: uuid::Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        track_id: track.id,
        applicant_user_id: candidate_user.to_string(),
        track_version: 1,
        status: "rejected".to_string(),
        submitted_answers: sqlx::types::Json(vec![
            rusty_discord_bot::recruitment::SubmittedAnswer {
                question_id: "q1".to_string(),
                answer: "Answer C".to_string(),
            },
        ]),
        current_stage: "screening".to_string(),
        version: 1,
        submitted_at: Utc::now() - Duration::days(60),
        updated_at: Utc::now() - Duration::days(60),
        decided_at: None,
        decision_reason: None,
        decided_by: None,
    };
    state
        .app
        .storage
        .create_recruitment_application(app_c)
        .await
        .unwrap();

    // 1. Candidate tries to apply now -> must fail with HTTP 400 Bad Request because Application A was rejected 1 day ago (< 3 days cooldown)
    let apply_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/tracks/cooldown-track/apply"
        ))
        .header(COOKIE, &cand_cookie)
        .header("x-csrf-token", csrf)
        .json(&serde_json::json!({
            "answers": [
                { "question_id": "q1", "answer": "Trying again too soon" }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(apply_resp.status(), reqwest::StatusCode::BAD_REQUEST);
    let err_json: serde_json::Value = apply_resp.json().await.unwrap();
    assert!(
        err_json["error"]
            .as_str()
            .unwrap()
            .contains("You must wait 3 days after a rejected application before reapplying")
    );

    // 2. Now simulate that Application A's rejection happened 5 days ago (> 3 days cooldown)
    let mut app_a_older = app_a;
    app_a_older.decided_at = Some(Utc::now() - Duration::days(5));
    app_a_older.updated_at = Utc::now() - Duration::days(5);
    state
        .app
        .storage
        .create_recruitment_application(app_a_older)
        .await
        .unwrap();

    // Now all rejection decisions are older than 3 days -> Apply must succeed with 200 OK
    let apply_ok_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/tracks/cooldown-track/apply"
        ))
        .header(COOKIE, &cand_cookie)
        .header("x-csrf-token", csrf)
        .json(&serde_json::json!({
            "answers": [
                { "question_id": "q1", "answer": "Applying after cooldown expired" }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(apply_ok_resp.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn test_asset_analytics_manual_flush_csrf_and_guild_isolation() {
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

    let g1 = "666000000000000001";
    let g2 = "666000000000000002";
    let admin_g1 = "666000000000000003";

    let session_token = "sess_asset_flush_isolation";
    let csrf_token = "csrf_asset_flush_isolation";

    state
        .app
        .sessions
        .save_session(
            session_token,
            DashboardUser {
                id: admin_g1.to_owned(),
                username: "AdminG1".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: g1.to_owned(),
                name: "Guild 1".to_owned(),
                icon: None,
                owner: false,
                permissions: "32".to_owned(), // MANAGE_GUILD
            }],
            csrf_token.to_owned(),
            Utc::now() + Duration::hours(24),
            None,
        )
        .await
        .unwrap();

    let auth_cookie = format!("rusty_discord_bot_session={session_token}");

    // Buffer items for G1 and G2
    let emoji_g1 = "111000111000111000";
    let emoji_g2 = "222000222000222000";

    state.app.asset_analytics.record_message(
        g1,
        "999000000000000001",
        &format!("<:emoji1:{emoji_g1}> hello"),
        &[],
        Some(&[emoji_g1.to_string()]),
        None,
    );

    state.app.asset_analytics.record_message(
        g2,
        "999000000000000002",
        &format!("<:emoji2:{emoji_g2}> world"),
        &[],
        Some(&[emoji_g2.to_string()]),
        None,
    );

    assert_eq!(state.app.asset_analytics.pending_counts_len(), 2);

    // 1. Flush G1 WITHOUT CSRF token -> 403 Forbidden and no state changes
    let unauth_flush = client
        .post(format!("{base_url}/api/guilds/{g1}/assets/flush"))
        .header(COOKIE, &auth_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(unauth_flush.status(), reqwest::StatusCode::FORBIDDEN);
    assert_eq!(
        state.app.asset_analytics.pending_counts_len(),
        2,
        "No items should be flushed on CSRF error"
    );

    // 2. Flush G1 WITH invalid CSRF token -> 403 Forbidden
    let bad_csrf_flush = client
        .post(format!("{base_url}/api/guilds/{g1}/assets/flush"))
        .header(COOKIE, &auth_cookie)
        .header("x-csrf-token", "invalid_csrf_token_xyz")
        .send()
        .await
        .unwrap();
    assert_eq!(bad_csrf_flush.status(), reqwest::StatusCode::FORBIDDEN);
    assert_eq!(state.app.asset_analytics.pending_counts_len(), 2);

    // 3. Flush G1 WITH valid CSRF token -> 200 OK, flushes only G1 items
    let flush_res = client
        .post(format!("{base_url}/api/guilds/{g1}/assets/flush"))
        .header(COOKIE, &auth_cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(flush_res.status(), reqwest::StatusCode::OK);
    let flush_json: serde_json::Value = flush_res.json().await.unwrap();
    assert_eq!(flush_json["flushed"], true);
    assert_eq!(flush_json["items_flushed"], 1);

    // Verify G1 item is no longer pending, but G2 is STILL pending
    assert_eq!(state.app.asset_analytics.pending_counts_len(), 1);

    // 4. Verify G1 data is recorded in storage, while G2 has no storage usage yet
    let g1_usage = state
        .app
        .storage
        .get_asset_usage_summary(
            g1,
            Some(rusty_discord_bot::asset_analytics::AssetKind::Emoji),
            "7d",
        )
        .await
        .unwrap();
    assert_eq!(g1_usage.total_usages, 1);

    let g2_usage = state
        .app
        .storage
        .get_asset_usage_summary(
            g2,
            Some(rusty_discord_bot::asset_analytics::AssetKind::Emoji),
            "7d",
        )
        .await
        .unwrap();
    assert_eq!(g2_usage.total_usages, 0);

    // 5. Global flush (worker/shutdown) flushes remaining G2 items
    let worker_flushed = state.app.asset_analytics.flush().await.unwrap();
    assert_eq!(worker_flushed, 1);
    assert_eq!(state.app.asset_analytics.pending_counts_len(), 0);
}

#[tokio::test]
async fn test_help_forum_session_close_requires_active_membership_for_owner() {
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

    let guild_id = "555000000000000001";
    let forum_channel_id = "555000000000000002";
    let thread_id = "555000000000000003";

    let former_member_user_id = "555000000000000010";
    let other_active_member_user_id = "555000000000000020";

    // Create HelpForumConfig with owner_can_close = true
    let config = rusty_discord_bot::help_forum::HelpForumConfig {
        guild_id: guild_id.to_string(),
        forum_channel_id: forum_channel_id.to_string(),
        owner_can_close: true,
        ..Default::default()
    };
    state
        .app
        .storage
        .upsert_help_forum_config(config)
        .await
        .unwrap();

    // Create HelpForumSession owned by former_member_user_id
    let help_session = rusty_discord_bot::storage::HelpForumSession {
        guild_id: guild_id.to_string(),
        forum_channel_id: forum_channel_id.to_string(),
        thread_id: thread_id.to_string(),
        owner_user_id: former_member_user_id.to_string(),
        status: "open".to_string(),
        opened_at: Utc::now() - Duration::hours(3),
        last_activity_at: Utc::now() - Duration::hours(1),
        first_helper_response_at: None,
        closed_at: None,
        closing_reason: None,
        generation: 1,
    };
    state
        .app
        .storage
        .create_help_forum_session(help_session)
        .await
        .unwrap();

    // Case 1: Former member user has a valid session, but NOT member of guild_id (left the guild)
    let former_sess_id = "session_former_member_close_test";
    let former_csrf = "csrf_former_member_close_test";
    state
        .app
        .sessions
        .save_session(
            former_sess_id,
            DashboardUser {
                id: former_member_user_id.to_owned(),
                username: "FormerMember".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![], // session has NO guilds (or different guilds)
            former_csrf.to_owned(),
            Utc::now() + Duration::hours(24),
            None,
        )
        .await
        .unwrap();

    let former_cookie = format!("rusty_discord_bot_session={former_sess_id}");

    let former_close_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/help-forums/{thread_id}/close"
        ))
        .header(COOKIE, &former_cookie)
        .header("x-csrf-token", former_csrf)
        .json(&serde_json::json!({
            "status": "solved",
            "reason": "Trying to close after leaving"
        }))
        .send()
        .await
        .unwrap();
    // Must be 403 Forbidden!
    assert_eq!(former_close_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Verify session in storage remains "open"
    let session_check1 = state
        .app
        .storage
        .get_help_forum_session(guild_id, thread_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session_check1.status, "open");

    // Case 2: Other active member (member of guild_id, but NOT the owner and NOT an admin) tries to close
    let other_sess_id = "session_other_member_close_test";
    let other_csrf = "csrf_other_member_close_test";
    state
        .app
        .sessions
        .save_session(
            other_sess_id,
            DashboardUser {
                id: other_active_member_user_id.to_owned(),
                username: "OtherMember".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Guild 555".to_owned(),
                icon: None,
                owner: false,
                permissions: "0".to_owned(),
            }],
            other_csrf.to_owned(),
            Utc::now() + Duration::hours(24),
            None,
        )
        .await
        .unwrap();

    let other_cookie = format!("rusty_discord_bot_session={other_sess_id}");

    let other_close_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/help-forums/{thread_id}/close"
        ))
        .header(COOKIE, &other_cookie)
        .header("x-csrf-token", other_csrf)
        .json(&serde_json::json!({
            "status": "solved",
            "reason": "Closing someone else's post"
        }))
        .send()
        .await
        .unwrap();
    // Must be 403 Forbidden!
    assert_eq!(other_close_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Case 3: Owner user is an ACTIVE member of guild_id
    let active_owner_sess_id = "session_active_owner_close_test";
    let active_owner_csrf = "csrf_active_owner_close_test";
    state
        .app
        .sessions
        .save_session(
            active_owner_sess_id,
            DashboardUser {
                id: former_member_user_id.to_owned(),
                username: "ActiveOwnerMember".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Guild 555".to_owned(),
                icon: None,
                owner: false,
                permissions: "0".to_owned(), // regular member, not admin
            }],
            active_owner_csrf.to_owned(),
            Utc::now() + Duration::hours(24),
            None,
        )
        .await
        .unwrap();

    let active_owner_cookie = format!("rusty_discord_bot_session={active_owner_sess_id}");

    let active_owner_close_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/help-forums/{thread_id}/close"
        ))
        .header(COOKIE, &active_owner_cookie)
        .header("x-csrf-token", active_owner_csrf)
        .json(&serde_json::json!({
            "status": "solved",
            "reason": "Closed by author who is active member"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(active_owner_close_resp.status(), reqwest::StatusCode::OK);

    // Verify session in storage is now solved
    let session_check2 = state
        .app
        .storage
        .get_help_forum_session(guild_id, thread_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session_check2.status, "solved");
    assert_eq!(
        session_check2.closing_reason.as_deref(),
        Some("Closed by author who is active member")
    );
}

#[tokio::test]
async fn test_moderation_appeal_cross_guild_tenant_isolation() {
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

    let guild_a = "111000000000000001";
    let guild_b = "222000000000000002";

    let mod_a_id = "111000000000000010";
    let mod_b_id = "222000000000000020";

    let sess_a = "sess_mod_a_isolation_test";
    let csrf_a = "csrf_mod_a_isolation_test";

    let sess_b = "sess_mod_b_isolation_test";
    let csrf_b = "csrf_mod_b_isolation_test";

    // Moderator A has permissions ONLY on Guild A
    state
        .app
        .sessions
        .save_session(
            sess_a,
            DashboardUser {
                id: mod_a_id.to_owned(),
                username: "ModA".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_a.to_owned(),
                name: "Guild A".to_owned(),
                icon: None,
                owner: false,
                permissions: "32".to_owned(), // MANAGE_GUILD
            }],
            csrf_a.to_owned(),
            Utc::now() + Duration::hours(24),
            None,
        )
        .await
        .unwrap();

    // Moderator B has permissions on Guild B
    state
        .app
        .sessions
        .save_session(
            sess_b,
            DashboardUser {
                id: mod_b_id.to_owned(),
                username: "ModB".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_b.to_owned(),
                name: "Guild B".to_owned(),
                icon: None,
                owner: false,
                permissions: "32".to_owned(), // MANAGE_GUILD
            }],
            csrf_b.to_owned(),
            Utc::now() + Duration::hours(24),
            None,
        )
        .await
        .unwrap();

    // Create an appeal in Guild B
    let appeal_b = rusty_discord_bot::storage::ModerationAppeal {
        id: uuid::Uuid::new_v4(),
        guild_id: guild_b.to_string(),
        case_number: 101,
        user_id: "888000000000000001".to_string(),
        status: "pending".to_string(),
        reason: "Unfair ban appeal".to_string(),
        evidence_url: None,
        staff_notes: None,
        created_at: Utc::now() - Duration::hours(5),
        updated_at: Utc::now() - Duration::hours(5),
    };
    state.app.storage.create_appeal(&appeal_b).await.unwrap();

    let cookie_a = format!("rusty_discord_bot_session={sess_a}");
    let cookie_b = format!("rusty_discord_bot_session={sess_b}");

    // 1. Moderator A attempts to decide appeal B using Guild A's endpoint WITHOUT CSRF -> 403 Forbidden
    let no_csrf_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/appeals/{}/decide",
            appeal_b.id
        ))
        .header(COOKIE, &cookie_a)
        .json(&serde_json::json!({
            "decision": "approved",
            "staff_notes": "Attempting cross-guild bypass without CSRF"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(no_csrf_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 2. Moderator A attempts to decide appeal B using Guild A's endpoint WITH valid CSRF -> must be 404 Not Found!
    let cross_guild_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/appeals/{}/decide",
            appeal_b.id
        ))
        .header(COOKIE, &cookie_a)
        .header("x-csrf-token", csrf_a)
        .json(&serde_json::json!({
            "decision": "approved",
            "staff_notes": "Attempting cross-guild bypass with CSRF"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(cross_guild_resp.status(), reqwest::StatusCode::NOT_FOUND);

    // Verify appeal B in storage was NOT modified
    let check_b = state
        .app
        .storage
        .get_appeal(appeal_b.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(check_b.status, "pending");
    assert_eq!(check_b.staff_notes, None);

    // 3. Moderator B decides appeal B using Guild B's endpoint -> 200 OK
    let legit_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_b}/appeals/{}/decide",
            appeal_b.id
        ))
        .header(COOKIE, &cookie_b)
        .header("x-csrf-token", csrf_b)
        .json(&serde_json::json!({
            "decision": "approved",
            "staff_notes": "Legitimate moderator decision"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(legit_resp.status(), reqwest::StatusCode::OK);

    // Verify appeal B in storage was properly updated
    let updated_b = state
        .app
        .storage
        .get_appeal(appeal_b.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated_b.status, "approved");
    assert_eq!(
        updated_b.staff_notes.as_deref(),
        Some("Legitimate moderator decision")
    );
}

#[tokio::test]
async fn test_diagnostics_discord_permissions_and_failure_states() {
    // 1. Setup mock Discord API server
    let mock_status = std::sync::Arc::new(std::sync::atomic::AtomicU16::new(200));
    let mock_status_clone = mock_status.clone();

    let mock_perms_str = std::sync::Arc::new(tokio::sync::RwLock::new("8".to_string())); // Default: ADMINISTRATOR
    let mock_perms_clone = mock_perms_str.clone();

    let discord_mock_app = axum::Router::new()
        .route(
            "/users/@me",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({
                    "id": "999000000000000001",
                    "username": "RustyBotMock"
                }))
            }),
        )
        .route(
            "/guilds/{guild_id}/permissions/member/{user_id}",
            axum::routing::get(
                move |axum::extract::Path((_gid, uid)): axum::extract::Path<(String, String)>| {
                    let status_val = mock_status_clone.load(std::sync::atomic::Ordering::SeqCst);
                    let perms_val = mock_perms_clone.clone();
                    async move {
                        // If this is the dashboard user revalidation, grant MANAGE_GUILD (32)
                        if uid != "999000000000000001" {
                            return (
                                axum::http::StatusCode::OK,
                                axum::Json(serde_json::json!({ "permissions": "32" })),
                            );
                        }
                        if status_val == 200 {
                            let p = perms_val.read().await.clone();
                            (
                                axum::http::StatusCode::OK,
                                axum::Json(serde_json::json!({ "permissions": p })),
                            )
                        } else if status_val == 403 {
                            (
                                axum::http::StatusCode::FORBIDDEN,
                                axum::Json(serde_json::json!({ "message": "Missing Access" })),
                            )
                        } else if status_val == 429 {
                            (
                                axum::http::StatusCode::TOO_MANY_REQUESTS,
                                axum::Json(serde_json::json!({ "message": "Rate limited" })),
                            )
                        } else {
                            (
                                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                axum::Json(serde_json::json!({ "message": "Upstream error" })),
                            )
                        }
                    }
                },
            ),
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

    // 2. Setup web app configured in live mode pointing to the mock server
    let mut config = test_config();
    config.discord_token = "live_bot_token_diagnostics_test".to_string();
    config.offline_mode = false;
    config.discord_api_base_url = format!("http://{discord_addr}");

    let app = AppState::bootstrap(config).await.expect("bootstrap");
    let permissions_cache = std::sync::Arc::new(dashmap::DashMap::new());
    let web_state = WebState {
        app,
        login_rate_limiter: std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    };
    web_state
        .app
        .is_gateway_ready
        .store(true, std::sync::atomic::Ordering::SeqCst);

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

    let g_admin = "888000000000000001";
    let g_limited = "888000000000000002";
    let g_rate_limited = "888000000000000003";

    let admin_user = "888000000000000010";
    let session_id = "sess_diag_tests";
    let csrf = "csrf_diag_tests";

    web_state
        .app
        .sessions
        .save_session(
            session_id,
            DashboardUser {
                id: admin_user.to_owned(),
                username: "DiagAdmin".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![
                DashboardGuild {
                    id: g_admin.to_owned(),
                    name: "Admin Guild".to_owned(),
                    icon: None,
                    owner: false,
                    permissions: "32".to_owned(),
                },
                DashboardGuild {
                    id: g_limited.to_owned(),
                    name: "Limited Guild".to_owned(),
                    icon: None,
                    owner: false,
                    permissions: "32".to_owned(),
                },
                DashboardGuild {
                    id: g_rate_limited.to_owned(),
                    name: "Rate Limited Guild".to_owned(),
                    icon: None,
                    owner: false,
                    permissions: "32".to_owned(),
                },
            ],
            csrf.to_owned(),
            Utc::now() + Duration::hours(24),
            None,
        )
        .await
        .unwrap();

    let cookie = format!("rusty_discord_bot_session={session_id}");

    // Test Case 1: Bot has ADMINISTRATOR (perms = "8") -> 100% health, healthy status
    mock_status.store(200, std::sync::atomic::Ordering::SeqCst);
    *mock_perms_str.write().await = "8".to_string(); // ADMINISTRATOR

    let diag_admin_resp = client
        .get(format!("{base_url}/api/guilds/{g_admin}/diagnostics"))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(diag_admin_resp.status(), reqwest::StatusCode::OK);
    let diag_admin_json: serde_json::Value = diag_admin_resp.json().await.unwrap();
    assert_eq!(diag_admin_json["overall_health_percent"], 100);
    assert_eq!(diag_admin_json["overall_health_status"], "healthy");

    // Test Case 2: Bot has only VIEW_CHANNEL (perms = "1024"), missing MANAGE_ROLES / BAN_MEMBERS etc.
    *mock_perms_str.write().await = "1024".to_string(); // VIEW_CHANNEL only

    let diag_limited_resp = client
        .get(format!("{base_url}/api/guilds/{g_limited}/diagnostics"))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(diag_limited_resp.status(), reqwest::StatusCode::OK);
    let diag_limited_json: serde_json::Value = diag_limited_resp.json().await.unwrap();
    // Health percent must be strictly < 100!
    let pct = diag_limited_json["overall_health_percent"]
        .as_u64()
        .expect("number percent");
    assert!(pct < 100, "Limited permissions must not report 100% health");
    assert_eq!(diag_limited_json["overall_health_status"], "degraded");

    // Check that a module requiring MANAGE_ROLES (like "roles") has status "missing"
    let mod_diags = diag_limited_json["module_diagnostics"].as_array().unwrap();
    let roles_check = mod_diags
        .iter()
        .find(|m| m["id"] == "roles")
        .expect("roles module check");
    assert_eq!(roles_check["status"], "missing");
    assert!(
        roles_check["reason"]
            .as_str()
            .unwrap()
            .contains("MANAGE_ROLES")
    );

    // Test Case 3: Discord returns 429 Too Many Requests -> unknown status, overall_health_percent = "incomplete"
    mock_status.store(429, std::sync::atomic::Ordering::SeqCst);

    let diag_429_resp = client
        .get(format!(
            "{base_url}/api/guilds/{g_rate_limited}/diagnostics"
        ))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(diag_429_resp.status(), reqwest::StatusCode::OK);
    let diag_429_json: serde_json::Value = diag_429_resp.json().await.unwrap();
    assert_eq!(diag_429_json["overall_health_status"], "incomplete");
    assert_eq!(diag_429_json["overall_health_percent"], "incomplete");

    let mod_diags_429 = diag_429_json["module_diagnostics"].as_array().unwrap();
    assert_eq!(mod_diags_429[0]["status"], "unknown");
    assert!(mod_diags_429[0]["reason"].as_str().unwrap().contains("429"));
}

#[tokio::test]
async fn test_polls_member_access_and_admin_boundary() {
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

    let guild_a = "111222333444555666";
    let guild_b = "999888777666555444";

    let poll = state
        .app
        .storage
        .create_community_poll(
            guild_a,
            "channel_123",
            "What feature should we build next?",
            &["Music Bot".to_string(), "Economy System".to_string()],
            false,
            Utc::now() + Duration::days(3),
        )
        .await
        .expect("create test poll");

    // Regular member session: member of guild_a, but permissions = "0" (no MANAGE_GUILD)
    let member_session_id = "poll_member_session_token";
    let member_user = DashboardUser {
        id: "member_poll_1".to_owned(),
        username: "voter_dan".to_owned(),
        global_name: None,
        avatar: None,
    };
    let member_guilds = vec![DashboardGuild {
        id: guild_a.to_owned(),
        name: "Guild A".to_owned(),
        icon: None,
        owner: false,
        permissions: "0".to_owned(), // regular member, no MANAGE_GUILD
    }];

    state
        .app
        .sessions
        .save_session(
            member_session_id,
            member_user,
            member_guilds,
            "csrf_poll_123".to_owned(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .expect("save member session");

    // Guild B member session: only member of guild_b
    let guild_b_session_id = "guild_b_member_session_token";
    let b_user = DashboardUser {
        id: "member_b_user".to_owned(),
        username: "outsider_bob".to_owned(),
        global_name: None,
        avatar: None,
    };
    let b_guilds = vec![DashboardGuild {
        id: guild_b.to_owned(),
        name: "Guild B".to_owned(),
        icon: None,
        owner: false,
        permissions: "0".to_owned(),
    }];

    state
        .app
        .sessions
        .save_session(
            guild_b_session_id,
            b_user,
            b_guilds,
            "csrf_b_123".to_owned(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .expect("save guild b session");

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let session_cookie = format!("rusty_discord_bot_session={member_session_id}");
    let b_cookie = format!("rusty_discord_bot_session={guild_b_session_id}");

    // 1. Regular member without MANAGE_GUILD can view poll in guild A -> 200 OK
    let get_resp = client
        .get(format!("{base_url}/api/guilds/{guild_a}/polls/{}", poll.id))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("get poll");
    assert_eq!(get_resp.status(), reqwest::StatusCode::OK);
    let poll_json: serde_json::Value = get_resp.json().await.unwrap();
    assert_eq!(poll_json["question"], "What feature should we build next?");

    // 2. Regular member without MANAGE_GUILD can vote with valid CSRF -> 200 OK
    let vote_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/polls/{}/vote",
            poll.id
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", "csrf_poll_123")
        .json(&serde_json::json!({ "option_idx": 0 }))
        .send()
        .await
        .expect("vote poll");
    assert_eq!(vote_resp.status(), reqwest::StatusCode::OK);
    let vote_json: serde_json::Value = vote_resp.json().await.unwrap();
    assert_eq!(vote_json["tallies_json"][0], 1);
    assert_eq!(vote_json["tallies_json"][1], 0);

    // 3. Repeat identical vote is idempotent -> 200 OK, tallies unchanged
    let repeat_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/polls/{}/vote",
            poll.id
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", "csrf_poll_123")
        .json(&serde_json::json!({ "option_idx": 0 }))
        .send()
        .await
        .expect("repeat vote poll");
    assert_eq!(repeat_resp.status(), reqwest::StatusCode::OK);
    let repeat_json: serde_json::Value = repeat_resp.json().await.unwrap();
    assert_eq!(repeat_json["tallies_json"][0], 1);

    // 4. Voting without CSRF -> 403 Forbidden
    let no_csrf_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/polls/{}/vote",
            poll.id
        ))
        .header(COOKIE, &session_cookie)
        .json(&serde_json::json!({ "option_idx": 1 }))
        .send()
        .await
        .expect("vote without csrf");
    assert_eq!(no_csrf_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 5. Regular member cannot create polls (requires MANAGE_GUILD) -> 403 Forbidden
    let create_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/polls"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", "csrf_poll_123")
        .json(&serde_json::json!({
            "channel_id": "channel_123",
            "question": "Unauthorized poll?",
            "options": ["Yes", "No"],
            "is_anonymous": false,
            "duration_minutes": 60
        }))
        .send()
        .await
        .expect("create poll as member");
    assert_eq!(create_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 6. Member of Guild B cannot access Guild A poll endpoint -> 403 Forbidden
    let outsider_get_resp = client
        .get(format!("{base_url}/api/guilds/{guild_a}/polls/{}", poll.id))
        .header(COOKIE, &b_cookie)
        .send()
        .await
        .expect("outsider get poll");
    assert_eq!(outsider_get_resp.status(), reqwest::StatusCode::FORBIDDEN);

    let outsider_vote_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/polls/{}/vote",
            poll.id
        ))
        .header(COOKIE, &b_cookie)
        .header("X-CSRF-Token", "csrf_b_123")
        .json(&serde_json::json!({ "option_idx": 0 }))
        .send()
        .await
        .expect("outsider vote poll");
    assert_eq!(outsider_vote_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 7. Member of Guild B querying Guild B path with Guild A poll UUID -> 404 Not Found
    let mismatched_guild_resp = client
        .get(format!("{base_url}/api/guilds/{guild_b}/polls/{}", poll.id))
        .header(COOKIE, &b_cookie)
        .send()
        .await
        .expect("mismatched guild poll");
    assert_eq!(
        mismatched_guild_resp.status(),
        reqwest::StatusCode::NOT_FOUND
    );

    // 8. Anonymous unauthenticated request -> 401 Unauthorized
    let anon_resp = client
        .get(format!("{base_url}/api/guilds/{guild_a}/polls/{}", poll.id))
        .send()
        .await
        .expect("anonymous get poll");
    assert_eq!(anon_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 9. Invalid option index -> 400 Bad Request
    let invalid_idx_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/polls/{}/vote",
            poll.id
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", "csrf_poll_123")
        .json(&serde_json::json!({ "option_idx": 99 }))
        .send()
        .await
        .expect("invalid option index vote");
    assert_eq!(invalid_idx_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // 10. Expired poll rejection -> 400 Bad Request
    let expired_poll = state
        .app
        .storage
        .create_community_poll(
            guild_a,
            "channel_123",
            "Expired Question",
            &["A".to_string(), "B".to_string()],
            false,
            Utc::now() - Duration::minutes(10),
        )
        .await
        .expect("create expired poll");

    let expired_vote_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/polls/{}/vote",
            expired_poll.id
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", "csrf_poll_123")
        .json(&serde_json::json!({ "option_idx": 0 }))
        .send()
        .await
        .expect("vote expired poll");
    assert_eq!(expired_vote_resp.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_community_member_self_service_and_admin_boundary() {
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

    let guild_a = "111222333444555666";
    let guild_b = "999888777666555444";

    // User A: regular member of guild_a, permissions = "0"
    let user_a_session_id = "user_a_session_token";
    let user_a_id = "user_a_123";
    let user_a = DashboardUser {
        id: user_a_id.to_owned(),
        username: "member_alice".to_owned(),
        global_name: None,
        avatar: None,
    };
    let a_guilds = vec![DashboardGuild {
        id: guild_a.to_owned(),
        name: "Guild A".to_owned(),
        icon: None,
        owner: false,
        permissions: "0".to_owned(),
    }];

    state
        .app
        .sessions
        .save_session(
            user_a_session_id,
            user_a,
            a_guilds,
            "csrf_a_123".to_owned(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .expect("save user a session");

    // User B: another regular member of guild_a, permissions = "0"
    let user_b_session_id = "user_b_session_token";
    let user_b_id = "user_b_456";
    let user_b = DashboardUser {
        id: user_b_id.to_owned(),
        username: "member_bob".to_owned(),
        global_name: None,
        avatar: None,
    };
    let b_guilds = vec![DashboardGuild {
        id: guild_a.to_owned(),
        name: "Guild A".to_owned(),
        icon: None,
        owner: false,
        permissions: "0".to_owned(),
    }];

    state
        .app
        .sessions
        .save_session(
            user_b_session_id,
            user_b,
            b_guilds,
            "csrf_b_123".to_owned(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .expect("save user b session");

    // Outsider C: member of guild_b only
    let outsider_session_id = "outsider_session_token";
    let outsider_user = DashboardUser {
        id: "outsider_789".to_owned(),
        username: "outsider_charlie".to_owned(),
        global_name: None,
        avatar: None,
    };
    let outsider_guilds = vec![DashboardGuild {
        id: guild_b.to_owned(),
        name: "Guild B".to_owned(),
        icon: None,
        owner: false,
        permissions: "0".to_owned(),
    }];

    state
        .app
        .sessions
        .save_session(
            outsider_session_id,
            outsider_user,
            outsider_guilds,
            "csrf_c_123".to_owned(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .expect("save outsider session");

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let a_cookie = format!("rusty_discord_bot_session={user_a_session_id}");
    let b_cookie = format!("rusty_discord_bot_session={user_b_session_id}");
    let c_cookie = format!("rusty_discord_bot_session={outsider_session_id}");

    // Grant test cards to User A and User B
    let card_a = state
        .app
        .storage
        .grant_user_card(guild_a, user_a_id, "card_dragon", "Red Dragon", "legendary")
        .await
        .expect("grant card to A");
    let card_b = state
        .app
        .storage
        .grant_user_card(guild_a, user_b_id, "card_phoenix", "Solar Phoenix", "rare")
        .await
        .expect("grant card to B");

    // 1. Regular member User A can list their own cards -> 200 OK
    let list_cards_resp = client
        .get(format!("{base_url}/api/guilds/{guild_a}/cards"))
        .header(COOKIE, &a_cookie)
        .send()
        .await
        .expect("list cards");
    assert_eq!(list_cards_resp.status(), reqwest::StatusCode::OK);
    let cards_json: serde_json::Value = list_cards_resp.json().await.unwrap();
    let cards_arr = cards_json.as_array().expect("array of cards");
    assert_eq!(cards_arr.len(), 1);
    assert_eq!(cards_arr[0]["card_name"], "Red Dragon");

    // 2. Regular member User A can participate in counting with valid CSRF -> 200 OK
    let count1_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/counting/chan_count_1"
        ))
        .header(COOKIE, &a_cookie)
        .header("X-CSRF-Token", "csrf_a_123")
        .json(&serde_json::json!({ "count": 1 }))
        .send()
        .await
        .expect("submit count 1");
    assert_eq!(count1_resp.status(), reqwest::StatusCode::OK);
    let count1_json: serde_json::Value = count1_resp.json().await.unwrap();
    assert_eq!(count1_json["valid"], true);
    assert_eq!(count1_json["current_count"], 1);

    // 3. User B can submit next count -> 200 OK
    let count2_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/counting/chan_count_1"
        ))
        .header(COOKIE, &b_cookie)
        .header("X-CSRF-Token", "csrf_b_123")
        .json(&serde_json::json!({ "count": 2 }))
        .send()
        .await
        .expect("submit count 2");
    assert_eq!(count2_resp.status(), reqwest::StatusCode::OK);
    let count2_json: serde_json::Value = count2_resp.json().await.unwrap();
    assert_eq!(count2_json["valid"], true);
    assert_eq!(count2_json["current_count"], 2);

    // 4. Missing CSRF on counting -> 403 Forbidden
    let no_csrf_count = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/counting/chan_count_1"
        ))
        .header(COOKIE, &a_cookie)
        .json(&serde_json::json!({ "count": 3 }))
        .send()
        .await
        .expect("submit count without csrf");
    assert_eq!(no_csrf_count.status(), reqwest::StatusCode::FORBIDDEN);

    // 5. User A proposes trade with User B with valid CSRF -> 200 OK
    let propose_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/cards/trade"))
        .header(COOKIE, &a_cookie)
        .header("X-CSRF-Token", "csrf_a_123")
        .json(&serde_json::json!({
            "initiator_card_id": card_a.id,
            "target_user_id": user_b_id,
            "target_card_id": card_b.id
        }))
        .send()
        .await
        .expect("propose trade");
    assert_eq!(propose_resp.status(), reqwest::StatusCode::OK);
    let trade_json: serde_json::Value = propose_resp.json().await.unwrap();
    let trade_id = trade_json["id"].as_str().expect("trade id");

    // 6. User B accepts the trade with valid CSRF -> 200 OK
    let accept_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/cards/trade/{trade_id}/accept"
        ))
        .header(COOKIE, &b_cookie)
        .header("X-CSRF-Token", "csrf_b_123")
        .send()
        .await
        .expect("accept trade");
    assert_eq!(accept_resp.status(), reqwest::StatusCode::OK);
    let accept_json: serde_json::Value = accept_resp.json().await.unwrap();
    assert_eq!(accept_json["accepted"], true);

    // Verify ownership swapped
    let a_cards_after = state
        .app
        .storage
        .list_user_cards(guild_a, user_a_id)
        .await
        .unwrap();
    assert!(a_cards_after.iter().any(|c| c.card_id == "card_phoenix"));

    // 7. Regular member User A can create Tupper RP persona with CSRF -> 200 OK
    let tupper_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/tuppers"))
        .header(COOKIE, &a_cookie)
        .header("X-CSRF-Token", "csrf_a_123")
        .json(&serde_json::json!({
            "name": "Valeros the Fighter",
            "prefix_or_brackets": "v:"
        }))
        .send()
        .await
        .expect("create tupper");
    assert_eq!(tupper_resp.status(), reqwest::StatusCode::OK);
    let tupper_json: serde_json::Value = tupper_resp.json().await.unwrap();
    assert_eq!(tupper_json["name"], "Valeros the Fighter");

    // 8. Regular member User A can list their Tupper profiles -> 200 OK
    let list_tuppers_resp = client
        .get(format!("{base_url}/api/guilds/{guild_a}/tuppers"))
        .header(COOKIE, &a_cookie)
        .send()
        .await
        .expect("list tuppers");
    assert_eq!(list_tuppers_resp.status(), reqwest::StatusCode::OK);
    let tuppers_json: serde_json::Value = list_tuppers_resp.json().await.unwrap();
    let tuppers_arr = tuppers_json.as_array().expect("array of tuppers");
    assert_eq!(tuppers_arr.len(), 1);
    assert_eq!(tuppers_arr[0]["prefix_or_brackets"], "v:");

    // 9. Outsider C (Guild B only) cannot access Guild A counting, cards, or tuppers -> 403 Forbidden
    let outsider_cards = client
        .get(format!("{base_url}/api/guilds/{guild_a}/cards"))
        .header(COOKIE, &c_cookie)
        .send()
        .await
        .expect("outsider get cards");
    assert_eq!(outsider_cards.status(), reqwest::StatusCode::FORBIDDEN);

    let outsider_tuppers = client
        .get(format!("{base_url}/api/guilds/{guild_a}/tuppers"))
        .header(COOKIE, &c_cookie)
        .send()
        .await
        .expect("outsider get tuppers");
    assert_eq!(outsider_tuppers.status(), reqwest::StatusCode::FORBIDDEN);

    let outsider_count = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/counting/chan_count_1"
        ))
        .header(COOKIE, &c_cookie)
        .header("X-CSRF-Token", "csrf_c_123")
        .json(&serde_json::json!({ "count": 3 }))
        .send()
        .await
        .expect("outsider submit count");
    assert_eq!(outsider_count.status(), reqwest::StatusCode::FORBIDDEN);

    // 10. Regular member cannot manage administrative endpoints (e.g. FAQ creation) -> 403 Forbidden
    let admin_faq_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/faq"))
        .header(COOKIE, &a_cookie)
        .header("X-CSRF-Token", "csrf_a_123")
        .json(&serde_json::json!({
            "question": "Admin question?",
            "answer": "Admin answer."
        }))
        .send()
        .await
        .expect("create faq as member");
    assert_eq!(admin_faq_resp.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_read_only_get_endpoints_do_not_require_csrf() {
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

    let guild_id = "111222333444555666";
    let admin_user_id = "222333444555666777";

    let session_id = "admin_session_token_no_csrf";
    let admin_user = DashboardUser {
        id: admin_user_id.to_owned(),
        username: "admin_tester".to_owned(),
        global_name: None,
        avatar: None,
    };
    let admin_guilds = vec![DashboardGuild {
        id: guild_id.to_owned(),
        name: "Test Guild".to_owned(),
        icon: None,
        owner: true,
        permissions: "8".to_owned(), // Administrator
    }];

    state
        .app
        .sessions
        .save_session(
            session_id,
            admin_user,
            admin_guilds,
            "csrf_secret_token_123".to_owned(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .expect("save admin session");

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    // 1. GET /api/guilds/{guild_id}/store/purchases WITHOUT X-CSRF-Token -> 200 OK
    let purchases_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/store/purchases"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("get purchases without csrf");
    assert_eq!(purchases_resp.status(), reqwest::StatusCode::OK);

    // 2. GET /api/guilds/{guild_id}/members/search WITHOUT X-CSRF-Token -> 200 OK
    let search_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/members/search"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("search members without csrf");
    assert_eq!(search_resp.status(), reqwest::StatusCode::OK);

    // 3. GET /api/guilds/{guild_id}/members/{user_id}/identity-history WITHOUT X-CSRF-Token -> 200 OK
    let identity_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/members/{admin_user_id}/identity-history"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("get identity history without csrf");
    assert_eq!(identity_resp.status(), reqwest::StatusCode::OK);

    // 4. Anonymous request without session -> 401 Unauthorized
    let anon_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/store/purchases"))
        .send()
        .await
        .expect("anon get purchases");
    assert_eq!(anon_resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_github_webhook_route_hmac_verification_and_deduplication() {
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

    let secret = "test-github-secret";
    let body = r#"{"event_id":"gh_web_evt_100","repository":{"full_name":"org/repo"}}"#;
    let hmac_bytes = rusty_discord_bot::integrations::webhook::compute_hmac_sha256(
        secret.as_bytes(),
        body.as_bytes(),
    );
    let valid_sig = format!("sha256={}", hex::encode(hmac_bytes));

    // 1. Missing signature header -> 401 Unauthorized
    let missing_sig_resp = client
        .post(format!("{base_url}/api/webhooks/github"))
        .header("X-GitHub-Delivery", "delivery-web-100")
        .body(body)
        .send()
        .await
        .expect("post missing sig");
    assert_eq!(missing_sig_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 2. Bogus signature header -> 401 Unauthorized
    let bad_sig_resp = client
        .post(format!("{base_url}/api/webhooks/github"))
        .header(
            "X-Hub-Signature-256",
            "sha256=bogus_hmac_00000000000000000000",
        )
        .header("X-GitHub-Delivery", "delivery-web-100")
        .body(body)
        .send()
        .await
        .expect("post bad sig");
    assert_eq!(bad_sig_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 3. Valid HMAC signature -> 200 OK, duplicate: false
    let valid_resp = client
        .post(format!("{base_url}/api/webhooks/github"))
        .header("X-Hub-Signature-256", &valid_sig)
        .header("X-GitHub-Delivery", "delivery-web-100")
        .body(body)
        .send()
        .await
        .expect("post valid sig");
    assert_eq!(valid_resp.status(), reqwest::StatusCode::OK);
    let valid_json: serde_json::Value = valid_resp.json().await.unwrap();
    assert_eq!(valid_json["duplicate"], false);

    // 4. Replay with same delivery ID -> 200 OK, duplicate: true
    let replay_resp = client
        .post(format!("{base_url}/api/webhooks/github"))
        .header("X-Hub-Signature-256", &valid_sig)
        .header("X-GitHub-Delivery", "delivery-web-100")
        .body(body)
        .send()
        .await
        .expect("post replay sig");
    assert_eq!(replay_resp.status(), reqwest::StatusCode::OK);
    let replay_json: serde_json::Value = replay_resp.json().await.unwrap();
    assert_eq!(replay_json["duplicate"], true);
}

#[tokio::test]
async fn test_revalidate_guild_permission_owner_fallback_fails_closed_on_transfer() {
    let discord_mock_app = axum::Router::new()
        // 1. permissions/member returns 404
        .route(
            "/guilds/{guild_id}/permissions/member/{user_id}",
            axum::routing::get(|| async {
                (
                    axum::http::StatusCode::NOT_FOUND,
                    axum::Json(serde_json::json!({ "message": "Unknown Member Permissions" })),
                )
            }),
        )
        // 2. Fallback members/{user_id} returns 200 without permissions field
        .route(
            "/guilds/{guild_id}/members/{user_id}",
            axum::routing::get(|| async {
                (
                    axum::http::StatusCode::OK,
                    axum::Json(serde_json::json!({
                        "user": {
                            "id": "user_former_owner",
                            "username": "former_owner"
                        },
                        "roles": []
                    })),
                )
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

    let mut config = test_config();
    config.discord_token = "live_bot_token_production_secret".to_string();
    config.offline_mode = false;
    config.discord_api_base_url = format!("http://{discord_addr}");

    let app = AppState::bootstrap(config).await.expect("bootstrap");
    let permissions_cache = std::sync::Arc::new(dashmap::DashMap::new());
    let state = WebState {
        app,
        login_rate_limiter: std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    };

    let session = Session {
        user: DashboardUser {
            id: "user_former_owner".to_owned(),
            username: "former_owner".to_owned(),
            global_name: Some("Former Owner".to_owned()),
            avatar: None,
        },
        guilds: vec![DashboardGuild {
            id: "guild_transferred".to_owned(),
            name: "Transferred Guild".to_owned(),
            icon: None,
            owner: true,                 // Historic snapshot from OAuth login
            permissions: "0".to_owned(), // Live permissions are 0 (e.g. ownership was transferred)
        }],
        csrf_token: "csrf_transferred".to_owned(),
        expires_at: Utc::now() + Duration::days(1),
        access_token: None,
    };

    // permissions/member returns 404, fallback members returns 200 without permissions.
    // Must fail closed (403 Forbidden) and NOT elevate permissions based on stale owner=true!
    let access = ensure_guild_access(&state, &session, "guild_transferred").await;
    assert!(
        access.is_err(),
        "Access must fail closed and deny management permissions"
    );
    let err = access.unwrap_err();
    assert_eq!(err.status, axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_logout_revocation_success_and_lifecycle() {
    let (state, perms_cache) = test_web_state().await;
    let user_id = "user_logout_test";
    let guild_id = "guild_logout_test";

    let session = Session {
        user: DashboardUser {
            id: user_id.to_owned(),
            username: "logout_user".to_owned(),
            global_name: Some("Logout User".to_owned()),
            avatar: None,
        },
        guilds: vec![DashboardGuild {
            id: guild_id.to_owned(),
            name: "Logout Guild".to_owned(),
            icon: None,
            owner: true,
            permissions: "8".to_owned(),
        }],
        csrf_token: "csrf_token_logout_123".to_owned(),
        expires_at: Utc::now() + Duration::days(1),
        access_token: None,
    };

    let session_id = "test_session_id_logout_123";
    state
        .app
        .sessions
        .save_session(
            session_id,
            session.user,
            session.guilds,
            session.csrf_token.clone(),
            session.expires_at,
            session.access_token,
        )
        .await
        .expect("save session");

    perms_cache.insert(
        (user_id.to_owned(), guild_id.to_owned()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let web_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind web listener");
    let web_addr = web_listener.local_addr().expect("web addr");
    let router = build_router(state.clone());
    tokio::spawn(async move {
        axum::serve(web_listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{web_addr}");

    // 1. Without CSRF -> 403 Forbidden
    let no_csrf = client
        .post(format!("{base_url}/api/logout"))
        .header(
            reqwest::header::COOKIE,
            format!("rusty_discord_bot_session={session_id}"),
        )
        .send()
        .await
        .expect("send no csrf");
    assert_eq!(no_csrf.status(), reqwest::StatusCode::FORBIDDEN);

    // 2. Successful logout -> 204 No Content, Set-Cookie expired
    let valid_logout = client
        .post(format!("{base_url}/api/logout"))
        .header(
            reqwest::header::COOKIE,
            format!("rusty_discord_bot_session={session_id}"),
        )
        .header("X-CSRF-Token", "csrf_token_logout_123")
        .send()
        .await
        .expect("send valid logout");
    assert_eq!(valid_logout.status(), reqwest::StatusCode::NO_CONTENT);

    let cookie_header = valid_logout
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .expect("set-cookie header")
        .to_str()
        .unwrap();
    assert!(cookie_header.contains("Max-Age=0") || cookie_header.contains("expires="));

    // 3. Session must be deleted from storage
    let stored_session = state
        .app
        .sessions
        .get_session(session_id)
        .await
        .expect("get session");
    assert!(stored_session.is_none());

    // 4. Permissions cache for this user must be cleared
    assert!(
        perms_cache
            .get(&(user_id.to_owned(), guild_id.to_owned()))
            .is_none()
    );

    // 5. Subsequent request with old session -> 401 Unauthorized
    let unauthed = client
        .get(format!("{base_url}/api/session"))
        .header(
            reqwest::header::COOKIE,
            format!("rusty_discord_bot_session={session_id}"),
        )
        .send()
        .await
        .expect("send unauthed");
    assert_eq!(unauthed.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_recruitment_applications_pagination_and_batch_fetching() {
    let (state, perms_cache) = test_web_state().await;
    let admin_id = "111222333444555666";
    let guild_id = "999888777666555444";

    let session = Session {
        user: DashboardUser {
            id: admin_id.to_owned(),
            username: "admin_user".to_owned(),
            global_name: Some("Admin".to_owned()),
            avatar: None,
        },
        guilds: vec![DashboardGuild {
            id: guild_id.to_owned(),
            name: "Recruitment Guild".to_owned(),
            icon: None,
            owner: true,
            permissions: "8".to_owned(),
        }],
        csrf_token: "csrf_token_recruitment".to_owned(),
        expires_at: Utc::now() + Duration::days(1),
        access_token: None,
    };

    let session_id = "session_recruitment_pagination";
    state
        .app
        .sessions
        .save_session(
            session_id,
            session.user,
            session.guilds,
            session.csrf_token.clone(),
            session.expires_at,
            session.access_token,
        )
        .await
        .expect("save session");

    perms_cache.insert(
        (admin_id.to_owned(), guild_id.to_owned()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    // Create 5 applications directly in storage
    let track_id = uuid::Uuid::new_v4();
    for i in 0..5 {
        let app = rusty_discord_bot::recruitment::RecruitmentApplication {
            id: uuid::Uuid::new_v4(),
            guild_id: guild_id.to_owned(),
            track_id,
            applicant_user_id: format!("10000000000000000{i}"),
            track_version: 1,
            status: "submitted".to_owned(),
            submitted_answers: sqlx::types::Json(Vec::new()),
            current_stage: "submitted".to_owned(),
            version: 1,
            submitted_at: Utc::now() - chrono::Duration::minutes(i as i64),
            updated_at: Utc::now() - chrono::Duration::minutes(i as i64),
            decided_at: None,
            decision_reason: None,
            decided_by: None,
        };
        state
            .app
            .storage
            .create_recruitment_application(app)
            .await
            .expect("create app");
    }

    let web_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind web listener");
    let web_addr = web_listener.local_addr().expect("web addr");
    let router = build_router(state.clone());
    tokio::spawn(async move {
        axum::serve(web_listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{web_addr}");

    // Page 1: limit=2, offset=0
    let page1_res = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/applications?limit=2&offset=0"
        ))
        .header(
            reqwest::header::COOKIE,
            format!("rusty_discord_bot_session={session_id}"),
        )
        .send()
        .await
        .expect("get page 1");
    assert_eq!(page1_res.status(), reqwest::StatusCode::OK);
    let page1_json: serde_json::Value = page1_res.json().await.unwrap();
    assert_eq!(page1_json["total"], 5);
    assert_eq!(page1_json["limit"], 2);
    assert_eq!(page1_json["offset"], 0);
    let page1_apps = page1_json["applications"].as_array().unwrap();
    assert_eq!(page1_apps.len(), 2);

    // Page 2: limit=2, offset=2
    let page2_res = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/applications?limit=2&offset=2"
        ))
        .header(
            reqwest::header::COOKIE,
            format!("rusty_discord_bot_session={session_id}"),
        )
        .send()
        .await
        .expect("get page 2");
    assert_eq!(page2_res.status(), reqwest::StatusCode::OK);
    let page2_json: serde_json::Value = page2_res.json().await.unwrap();
    let page2_apps = page2_json["applications"].as_array().unwrap();
    assert_eq!(page2_apps.len(), 2);

    // Page 3: limit=2, offset=4
    let page3_res = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/applications?limit=2&offset=4"
        ))
        .header(
            reqwest::header::COOKIE,
            format!("rusty_discord_bot_session={session_id}"),
        )
        .send()
        .await
        .expect("get page 3");
    assert_eq!(page3_res.status(), reqwest::StatusCode::OK);
    let page3_json: serde_json::Value = page3_res.json().await.unwrap();
    let page3_apps = page3_json["applications"].as_array().unwrap();
    assert_eq!(page3_apps.len(), 1);

    // No overlap between page 1 and page 2
    let id1 = page1_apps[0]["id"].as_str().unwrap();
    let id2 = page1_apps[1]["id"].as_str().unwrap();
    let id3 = page2_apps[0]["id"].as_str().unwrap();
    let id4 = page2_apps[1]["id"].as_str().unwrap();
    let id5 = page3_apps[0]["id"].as_str().unwrap();

    let all_ids = [id1, id2, id3, id4, id5];
    let unique_ids: std::collections::HashSet<_> = all_ids.iter().collect();
    assert_eq!(
        unique_ids.len(),
        5,
        "All pages must yield distinct application IDs"
    );
}

#[tokio::test]
async fn test_login_rate_limiting_with_spoofed_x_forwarded_for_issue_246() {
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

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build client");

    let base_url = format!("http://{}", addr);

    // 10 requests from the same real client IP (203.0.113.45), but with rotating spoofed leftmost IP in X-Forwarded-For
    for i in 1..=10 {
        let spoofed_xff = format!("198.51.100.{i}, 203.0.113.45");
        let resp = client
            .get(format!("{base_url}/auth/discord"))
            .header("X-Forwarded-For", spoofed_xff)
            .send()
            .await
            .expect("send login attempt");
        assert_eq!(
            resp.status(),
            reqwest::StatusCode::TEMPORARY_REDIRECT,
            "Attempt {i} within per-IP quota (10/min) must succeed"
        );
    }

    // 11th request from the same real client IP (with yet another spoofed leftmost IP) MUST be blocked with 429 Too Many Requests
    let resp_blocked = client
        .get(format!("{base_url}/auth/discord"))
        .header("X-Forwarded-For", "198.51.100.99, 203.0.113.45")
        .send()
        .await
        .expect("send 11th login attempt");
    assert_eq!(
        resp_blocked.status(),
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        "11th attempt from real IP 203.0.113.45 must return HTTP 429 despite spoofed leftmost XFF"
    );

    // A different real client IP (203.0.113.46) should NOT be blocked (independent per-IP rate limiting preserved)
    let resp_other_client = client
        .get(format!("{base_url}/auth/discord"))
        .header("X-Forwarded-For", "198.51.100.99, 203.0.113.46")
        .send()
        .await
        .expect("send login attempt for other client");
    assert_eq!(
        resp_other_client.status(),
        reqwest::StatusCode::TEMPORARY_REDIRECT,
        "Independent real client IP 203.0.113.46 must succeed"
    );
}

#[tokio::test]
async fn test_cleanup_dynamic_voice_csrf_and_atomic_isolation_issue_247() {
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

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build client");

    let base_url = format!("http://{}", addr);

    let guild_id_1 = "111000000000000101";
    let guild_id_2 = "111000000000000102";
    let admin_user_id = "user_voice_admin";
    let session_id = "test_session_voice_admin";
    let csrf_token = "valid_csrf_token_voice_101";

    let user = DashboardUser {
        id: admin_user_id.to_owned(),
        username: "VoiceAdmin".to_owned(),
        global_name: None,
        avatar: None,
    };
    let guilds = vec![
        DashboardGuild {
            id: guild_id_1.to_owned(),
            name: "Guild One".to_owned(),
            icon: None,
            owner: true,
            permissions: "8".to_owned(),
        },
        DashboardGuild {
            id: guild_id_2.to_owned(),
            name: "Guild Two".to_owned(),
            icon: None,
            owner: true,
            permissions: "8".to_owned(),
        },
    ];

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

    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    // Setup voice rooms:
    // Guild 1:
    // - room_1_stale: empty 60s ago (stale > 30s threshold)
    // - room_1_recent: empty 10s ago (not stale < 30s threshold)
    // - room_1_active: active (empty_since = None)
    // Guild 2:
    // - room_2_stale: empty 60s ago (stale > 30s threshold)
    state
        .app
        .storage
        .register_dynamic_voice_room("chan_1_stale", guild_id_1, "owner_1")
        .await
        .unwrap();
    state
        .app
        .storage
        .set_dynamic_voice_empty_since("chan_1_stale", Some(Utc::now() - Duration::seconds(60)))
        .await
        .unwrap();

    state
        .app
        .storage
        .register_dynamic_voice_room("chan_1_recent", guild_id_1, "owner_2")
        .await
        .unwrap();
    state
        .app
        .storage
        .set_dynamic_voice_empty_since("chan_1_recent", Some(Utc::now() - Duration::seconds(10)))
        .await
        .unwrap();

    state
        .app
        .storage
        .register_dynamic_voice_room("chan_1_active", guild_id_1, "owner_3")
        .await
        .unwrap();

    state
        .app
        .storage
        .register_dynamic_voice_room("chan_2_stale", guild_id_2, "owner_4")
        .await
        .unwrap();
    state
        .app
        .storage
        .set_dynamic_voice_empty_since("chan_2_stale", Some(Utc::now() - Duration::seconds(60)))
        .await
        .unwrap();

    // 1. GET /voice/dynamic/cleanup MUST be rejected with 405 Method Not Allowed (eliminating top-level GET CSRF)
    let get_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id_1}/voice/dynamic/cleanup"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("send GET cleanup");
    assert_eq!(
        get_resp.status(),
        reqwest::StatusCode::METHOD_NOT_ALLOWED,
        "GET request must return 405 Method Not Allowed"
    );

    // Verify all 4 rooms are still in storage after GET
    assert!(
        state
            .app
            .storage
            .get_dynamic_voice_room("chan_1_stale")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        state
            .app
            .storage
            .get_dynamic_voice_room("chan_1_recent")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        state
            .app
            .storage
            .get_dynamic_voice_room("chan_1_active")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        state
            .app
            .storage
            .get_dynamic_voice_room("chan_2_stale")
            .await
            .unwrap()
            .is_some()
    );

    // 2. POST without session -> 401 Unauthorized
    let post_unauth = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id_1}/voice/dynamic/cleanup"
        ))
        .send()
        .await
        .expect("send unauth POST cleanup");
    assert_eq!(post_unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 3. POST with session but missing CSRF token -> 403 Forbidden
    let post_no_csrf = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id_1}/voice/dynamic/cleanup"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("send POST without csrf");
    assert_eq!(post_no_csrf.status(), reqwest::StatusCode::FORBIDDEN);

    // 4. POST with session and invalid CSRF token -> 403 Forbidden
    let post_bad_csrf = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id_1}/voice/dynamic/cleanup"
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", "invalid_csrf_token_xyz")
        .send()
        .await
        .expect("send POST with invalid csrf");
    assert_eq!(post_bad_csrf.status(), reqwest::StatusCode::FORBIDDEN);

    // Verify all 4 rooms are STILL in storage after failed CSRF attempts
    assert!(
        state
            .app
            .storage
            .get_dynamic_voice_room("chan_1_stale")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        state
            .app
            .storage
            .get_dynamic_voice_room("chan_1_recent")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        state
            .app
            .storage
            .get_dynamic_voice_room("chan_1_active")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        state
            .app
            .storage
            .get_dynamic_voice_room("chan_2_stale")
            .await
            .unwrap()
            .is_some()
    );

    // 5. POST with session and VALID CSRF token for Guild 1 -> 200 OK
    let post_ok = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id_1}/voice/dynamic/cleanup"
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .send()
        .await
        .expect("send valid POST cleanup");
    assert_eq!(post_ok.status(), reqwest::StatusCode::OK);

    let res_json: serde_json::Value = post_ok.json().await.unwrap();
    assert_eq!(res_json["removed_records_count"], 1);
    assert_eq!(
        res_json["removed_channel_ids"],
        serde_json::json!(["chan_1_stale"])
    );
    // Backwards-compatible aliases
    assert_eq!(res_json["deleted_count"], 1);
    assert_eq!(
        res_json["deleted_channels"],
        serde_json::json!(["chan_1_stale"])
    );

    // Verify storage states:
    // - chan_1_stale in Guild 1 was removed
    assert!(
        state
            .app
            .storage
            .get_dynamic_voice_room("chan_1_stale")
            .await
            .unwrap()
            .is_none()
    );
    // - chan_1_recent in Guild 1 was NOT removed (<30s)
    assert!(
        state
            .app
            .storage
            .get_dynamic_voice_room("chan_1_recent")
            .await
            .unwrap()
            .is_some()
    );
    // - chan_1_active in Guild 1 was NOT removed (active)
    assert!(
        state
            .app
            .storage
            .get_dynamic_voice_room("chan_1_active")
            .await
            .unwrap()
            .is_some()
    );
    // - chan_2_stale in Guild 2 was NOT removed (cross-guild isolation)
    assert!(
        state
            .app
            .storage
            .get_dynamic_voice_room("chan_2_stale")
            .await
            .unwrap()
            .is_some()
    );

    // 6. Cleanup Guild 2 -> removes chan_2_stale
    let post_ok_g2 = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id_2}/voice/dynamic/cleanup"
        ))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .send()
        .await
        .expect("send valid POST cleanup g2");
    assert_eq!(post_ok_g2.status(), reqwest::StatusCode::OK);
    let res_g2_json: serde_json::Value = post_ok_g2.json().await.unwrap();
    assert_eq!(res_g2_json["removed_records_count"], 1);
    assert_eq!(
        res_g2_json["removed_channel_ids"],
        serde_json::json!(["chan_2_stale"])
    );
    assert!(
        state
            .app
            .storage
            .get_dynamic_voice_room("chan_2_stale")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn test_readiness_endpoint_sanitizes_internal_database_errors() {
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

    // 1. Healthy state: storage ping passes -> 200 OK
    let resp_ok = client
        .get(format!("{base_url}/health/ready"))
        .send()
        .await
        .expect("send ready check");
    assert_eq!(resp_ok.status(), reqwest::StatusCode::OK);
    let body_ok: serde_json::Value = resp_ok.json().await.unwrap();
    assert_eq!(body_ok["status"], "ready");
    assert_eq!(body_ok["checks"]["storage"], "ok");

    // 2. Failure state: inject database failure
    state.app.storage.inject_ping_failure(true);

    let resp_fail = client
        .get(format!("{base_url}/health/ready"))
        .send()
        .await
        .expect("send ready check during failure");
    assert_eq!(resp_fail.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    let raw_text = resp_fail.text().await.unwrap();
    let body_fail: serde_json::Value = serde_json::from_str(&raw_text).unwrap();

    // Must report "unready" and "storage": "unavailable"
    assert_eq!(body_fail["status"], "unready");
    assert_eq!(body_fail["checks"]["storage"], "unavailable");

    // Crucial security requirement: MUST NOT leak credentials, internal hostnames, or raw error string
    assert!(
        !raw_text.contains("postgres-prod.internal"),
        "must not leak internal DB hostname"
    );
    assert!(!raw_text.contains("10.0.0.5"), "must not leak internal IP");
    assert!(!raw_text.contains("5432"), "must not leak internal port");
    assert!(
        !raw_text.contains("rusty_prod"),
        "must not leak database user"
    );
    assert!(
        !raw_text.contains("password"),
        "must not leak password or auth phrase"
    );
    assert!(
        !raw_text.contains("error:"),
        "must not leak raw error: prefix"
    );
    assert!(!raw_text.contains("SQLSTATE"), "must not leak SQLSTATE");

    // 3. Recovery: storage ping returns to healthy -> 200 OK
    state.app.storage.inject_ping_failure(false);
    let resp_recovered = client
        .get(format!("{base_url}/health/ready"))
        .send()
        .await
        .expect("send recovered check");
    assert_eq!(resp_recovered.status(), reqwest::StatusCode::OK);
    let body_recovered: serde_json::Value = resp_recovered.json().await.unwrap();
    assert_eq!(body_recovered["status"], "ready");
    assert_eq!(body_recovered["checks"]["storage"], "ok");
}

#[tokio::test]
async fn test_metrics_endpoint_bearer_token_session_cookie_and_ip_authorization() {
    let mut config = test_config();
    config.metrics_token = Some("super_secret_metrics_token_xyz987".to_string());
    config.metrics_allowed_ips = ["192.0.2.1".parse().unwrap()].into_iter().collect();

    let app = AppState::bootstrap(config)
        .await
        .expect("bootstrap app state");
    let permissions_cache = Arc::new(DashMap::new());
    let state = WebState {
        app,
        login_rate_limiter: Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    };

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

    // 1. Healthcheck /health/ready remains accessible without any authentication (for Compose)
    let ready_res = client
        .get(format!("{base_url}/health/ready"))
        .send()
        .await
        .expect("send ready check");
    assert_eq!(ready_res.status(), reqwest::StatusCode::OK);

    // 2. Anonymous GET /metrics without token or session -> 401 Unauthorized
    let unauth_res = client
        .get(format!("{base_url}/metrics"))
        .send()
        .await
        .expect("send unauthenticated metrics check");
    assert_eq!(unauth_res.status(), reqwest::StatusCode::UNAUTHORIZED);
    let unauth_body = unauth_res.text().await.unwrap();
    assert!(!unauth_body.contains("rusty_discord_bot_active_sessions"));

    // 3. Invalid Bearer token -> 401 Unauthorized
    let wrong_token_res = client
        .get(format!("{base_url}/metrics"))
        .header("Authorization", "Bearer invalid_secret_123")
        .send()
        .await
        .expect("send wrong token check");
    assert_eq!(wrong_token_res.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 4. Valid Bearer token -> 200 OK with Prometheus telemetry
    let valid_token_res = client
        .get(format!("{base_url}/metrics"))
        .header("Authorization", "Bearer super_secret_metrics_token_xyz987")
        .send()
        .await
        .expect("send valid bearer check");
    assert_eq!(valid_token_res.status(), reqwest::StatusCode::OK);
    let valid_body = valid_token_res.text().await.unwrap();
    assert!(valid_body.contains("rusty_discord_bot_active_sessions"));
    assert!(valid_body.contains("rusty_discord_bot_dashboard_requests_total"));

    // 5. Valid Bearer token with lowercase prefix -> 200 OK
    let lowercase_token_res = client
        .get(format!("{base_url}/metrics"))
        .header("Authorization", "bearer super_secret_metrics_token_xyz987")
        .send()
        .await
        .expect("send valid lowercase bearer check");
    assert_eq!(lowercase_token_res.status(), reqwest::StatusCode::OK);

    // 6. Authenticated dashboard session cookie -> 200 OK
    let session_id = "test_metrics_viewer_session_token";
    let user = DashboardUser {
        id: "777".to_owned(),
        username: "metrics_operator".to_owned(),
        global_name: None,
        avatar: None,
    };
    state
        .app
        .sessions
        .save_session(
            session_id,
            user,
            vec![],
            "csrf_metrics".to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .expect("save session");

    let session_cookie = format!("rusty_discord_bot_session={session_id}");
    let session_res = client
        .get(format!("{base_url}/metrics"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .expect("send session check");
    assert_eq!(session_res.status(), reqwest::StatusCode::OK);
    let session_body = session_res.text().await.unwrap();
    assert!(session_body.contains("rusty_discord_bot_active_sessions"));

    // 7. Expired/invalid session cookie -> 401 Unauthorized
    let bad_session_res = client
        .get(format!("{base_url}/metrics"))
        .header(COOKIE, "rusty_discord_bot_session=non_existent_session")
        .send()
        .await
        .expect("send bad session check");
    assert_eq!(bad_session_res.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_giveaways_end_and_reroll_lifecycle_and_conflict() {
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

    let guild_id = "555666777888999000";
    let base_url = format!("http://{addr}");
    let client = reqwest::Client::new();

    // Setup admin session with MANAGE_GUILD (0x20)
    let admin_session_id = "admin_giveaway_session";
    let admin_csrf = "admin_csrf_token_xyz";
    let admin_user = DashboardUser {
        id: "admin_123".to_owned(),
        username: "admin_user".to_owned(),
        global_name: None,
        avatar: None,
    };
    let admin_guilds = vec![DashboardGuild {
        id: guild_id.to_owned(),
        name: "Test Guild".to_owned(),
        icon: None,
        owner: false,
        permissions: "32".to_owned(), // MANAGE_GUILD
    }];
    state
        .app
        .sessions
        .save_session(
            admin_session_id,
            admin_user,
            admin_guilds,
            admin_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .expect("save admin session");
    let admin_cookie = format!("rusty_discord_bot_session={admin_session_id}");

    // Setup regular member session (permissions = "0")
    let member_session_id = "member_giveaway_session";
    let member_csrf = "member_csrf_token_xyz";
    let member_user = DashboardUser {
        id: "member_456".to_owned(),
        username: "regular_member".to_owned(),
        global_name: None,
        avatar: None,
    };
    let member_guilds = vec![DashboardGuild {
        id: guild_id.to_owned(),
        name: "Test Guild".to_owned(),
        icon: None,
        owner: false,
        permissions: "0".to_owned(),
    }];
    state
        .app
        .sessions
        .save_session(
            member_session_id,
            member_user,
            member_guilds,
            member_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .expect("save member session");
    let member_cookie = format!("rusty_discord_bot_session={member_session_id}");

    // Create an active giveaway with 1 winner and 2 participants
    let giveaway = state
        .app
        .storage
        .create_giveaway(
            guild_id,
            "channel_123",
            "VIP Subscription",
            1,
            Utc::now() + Duration::hours(2),
        )
        .await
        .expect("create giveaway");
    let ga_id = giveaway.id;

    assert!(
        state
            .app
            .storage
            .join_giveaway(ga_id, "participant_A")
            .await
            .unwrap()
    );
    assert!(
        state
            .app
            .storage
            .join_giveaway(ga_id, "participant_B")
            .await
            .unwrap()
    );

    // 1. Calling /reroll before giveaway is ended must return 409 Conflict
    let reroll_conflict_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{ga_id}/reroll"
        ))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", admin_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(reroll_conflict_resp.status(), reqwest::StatusCode::CONFLICT);

    // Verify storage state was NOT mutated to ended
    let ga_check = state
        .app
        .storage
        .get_giveaway(ga_id)
        .await
        .unwrap()
        .unwrap();
    assert!(!ga_check.ended);
    assert_eq!(ga_check.winners_json.as_array().unwrap().len(), 0);

    // 2. Joining active giveaway after rejected reroll must still work
    let join_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{ga_id}/join"
        ))
        .header(COOKIE, &member_cookie)
        .header("x-csrf-token", member_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(join_resp.status(), reqwest::StatusCode::OK);
    let join_json: serde_json::Value = join_resp.json().await.unwrap();
    assert_eq!(join_json["joined"], true);

    // 3. Regular member cannot end giveaway -> 403 Forbidden
    let member_end_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{ga_id}/end"
        ))
        .header(COOKIE, &member_cookie)
        .header("x-csrf-token", member_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(member_end_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 4. Admin ending giveaway explicitly via POST /end -> 200 OK
    let end_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{ga_id}/end"
        ))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", admin_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(end_resp.status(), reqwest::StatusCode::OK);
    let end_json: serde_json::Value = end_resp.json().await.unwrap();
    assert_eq!(end_json["ended"], true);
    let winners = end_json["winners"].as_array().unwrap();
    assert_eq!(winners.len(), 1);
    let first_winner = winners[0].as_str().unwrap().to_owned();

    // 5. Calling /end again is idempotent and returns the same winners
    let retry_end_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{ga_id}/end"
        ))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", admin_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(retry_end_resp.status(), reqwest::StatusCode::OK);
    let retry_json: serde_json::Value = retry_end_resp.json().await.unwrap();
    assert_eq!(retry_json["winners"][0].as_str().unwrap(), first_winner);

    // 6. Calling /reroll on concluded giveaway selects an alternative winner (replacement semantics)
    let reroll_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{ga_id}/reroll"
        ))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", admin_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(reroll_resp.status(), reqwest::StatusCode::OK);
    let reroll_json: serde_json::Value = reroll_resp.json().await.unwrap();
    assert_eq!(reroll_json["rerolled"], true);
    let rerolled_winner = reroll_json["winners"][0].as_str().unwrap().to_owned();
    assert_ne!(rerolled_winner, first_winner);

    // 7. Giveaway without participants: reroll must return 400 Bad Request
    let empty_ga = state
        .app
        .storage
        .create_giveaway(
            guild_id,
            "channel_123",
            "Empty Giveaway",
            1,
            Utc::now() + Duration::hours(1),
        )
        .await
        .unwrap();
    state
        .app
        .storage
        .finish_giveaway(empty_ga.id)
        .await
        .unwrap();

    let reroll_empty_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}/reroll",
            empty_ga.id
        ))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", admin_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(reroll_empty_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // 8. Other guild isolation -> 403 Forbidden
    let other_guild_end = client
        .post(format!(
            "{base_url}/api/guilds/999888777666/giveaways/{ga_id}/end"
        ))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", admin_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(other_guild_end.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_giveaway_self_entry_and_creator_policy() {
    let (state, _perms_cache) = test_web_state().await;

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

    let guild_id = "guild_ga_self_100";
    let creator_user_id = "creator_user_200";
    let regular_user_id = "regular_user_300";

    let creator_session_id = "creator_session_token";
    let creator_csrf = "creator_csrf";
    state
        .app
        .sessions
        .save_session(
            creator_session_id,
            DashboardUser {
                id: creator_user_id.to_owned(),
                username: "creator".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "GA Guild".to_owned(),
                icon: None,
                owner: false,
                permissions: "32".to_owned(),
            }],
            creator_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .expect("save creator session");

    let regular_session_id = "regular_session_token";
    let regular_csrf = "regular_csrf";
    state
        .app
        .sessions
        .save_session(
            regular_session_id,
            DashboardUser {
                id: regular_user_id.to_owned(),
                username: "regular".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "GA Guild".to_owned(),
                icon: None,
                owner: false,
                permissions: "0".to_owned(),
            }],
            regular_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .expect("save regular session");

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let creator_cookie = format!("rusty_discord_bot_session={creator_session_id}");
    let regular_cookie = format!("rusty_discord_bot_session={regular_session_id}");

    // 1. Create giveaway with creator_user_id
    let ga = state
        .app
        .storage
        .create_giveaway_with_id(
            uuid::Uuid::new_v4(),
            guild_id,
            "111222333444555",
            "Self Entry Nitro",
            1,
            Utc::now() + Duration::hours(2),
            Some(creator_user_id),
        )
        .await
        .expect("create giveaway with creator");

    // 2. Default allow_self_entry is false -> creator joining returns joined: false
    let creator_join_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}/join",
            ga.id
        ))
        .header(COOKIE, &creator_cookie)
        .header("x-csrf-token", creator_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(creator_join_resp.status(), reqwest::StatusCode::OK);
    let creator_join_json: serde_json::Value = creator_join_resp.json().await.unwrap();
    assert_eq!(creator_join_json["joined"], false);

    // 3. Regular member joining returns joined: true
    let regular_join_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}/join",
            ga.id
        ))
        .header(COOKIE, &regular_cookie)
        .header("x-csrf-token", regular_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(regular_join_resp.status(), reqwest::StatusCode::OK);
    let regular_join_json: serde_json::Value = regular_join_resp.json().await.unwrap();
    assert_eq!(regular_join_json["joined"], true);

    // 4. Update module settings to allow_self_entry = true
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
        .expect("update module settings");

    // Create another giveaway where creator now can enter
    let ga2 = state
        .app
        .storage
        .create_giveaway_with_id(
            uuid::Uuid::new_v4(),
            guild_id,
            "111222333444555",
            "Self Entry Allowed Nitro",
            1,
            Utc::now() + Duration::hours(2),
            Some(creator_user_id),
        )
        .await
        .expect("create giveaway 2");

    let creator_join2_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}/join",
            ga2.id
        ))
        .header(COOKIE, &creator_cookie)
        .header("x-csrf-token", creator_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(creator_join2_resp.status(), reqwest::StatusCode::OK);
    let creator_join2_json: serde_json::Value = creator_join2_resp.json().await.unwrap();
    assert_eq!(creator_join2_json["joined"], true);

    // 5. Grandfathering: legacy giveaway with created_by_user_id = None allows joining
    let ga_legacy = state
        .app
        .storage
        .create_giveaway_with_id(
            uuid::Uuid::new_v4(),
            guild_id,
            "111222333444555",
            "Legacy Giveaway",
            1,
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .expect("create legacy giveaway");

    let creator_join_legacy = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}/join",
            ga_legacy.id
        ))
        .header(COOKIE, &creator_cookie)
        .header("x-csrf-token", creator_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(creator_join_legacy.status(), reqwest::StatusCode::OK);
    let legacy_json: serde_json::Value = creator_join_legacy.json().await.unwrap();
    assert_eq!(legacy_json["joined"], true);
}

#[tokio::test]
async fn test_giveaway_max_winners_and_channel_view_live_mode() {
    use axum::response::IntoResponse;
    use axum::routing::get;

    // 1. Mock Discord API
    let discord_mock_app = axum::Router::new()
        .route(
            "/channels/{channel_id}",
            get(
                |axum::extract::Path(channel_id): axum::extract::Path<String>| async move {
                    if channel_id == "555666777888999" {
                        axum::Json(serde_json::json!({
                            "id": channel_id,
                            "guild_id": "777888999000111",
                            "type": 0
                        }))
                        .into_response()
                    } else {
                        (
                            reqwest::StatusCode::NOT_FOUND,
                            axum::Json(serde_json::json!({
                                "code": 10003,
                                "message": "Unknown Channel"
                            })),
                        )
                            .into_response()
                    }
                },
            ),
        )
        .route(
            "/channels/{channel_id}/permissions/member/{user_id}",
            get(
                |axum::extract::Path((_channel_id, user_id)): axum::extract::Path<(
                    String,
                    String,
                )>| async move {
                    if user_id == "111111111111111111" {
                        // VIEW_CHANNEL is bit 1024 (0x400)
                        axum::Json(serde_json::json!({
                            "permissions": "1024"
                        }))
                        .into_response()
                    } else if user_id == "222222222222222222" {
                        axum::Json(serde_json::json!({
                            "permissions": "0"
                        }))
                        .into_response()
                    } else {
                        (
                            reqwest::StatusCode::NOT_FOUND,
                            axum::Json(serde_json::json!({
                                "code": 10003,
                                "message": "Unknown Channel"
                            })),
                        )
                            .into_response()
                    }
                },
            ),
        )
        .route(
            "/guilds/{guild_id}/members/{user_id}",
            get(
                |axum::extract::Path((_guild_id, user_id)): axum::extract::Path<(
                    String,
                    String,
                )>| async move {
                    if user_id == "999999999999999999" {
                        (
                            reqwest::StatusCode::NOT_FOUND,
                            axum::Json(
                                serde_json::json!({"code": 10007, "message": "Unknown Member"}),
                            ),
                        )
                            .into_response()
                    } else {
                        axum::Json(serde_json::json!({
                            "user": { "id": user_id },
                            "roles": []
                        }))
                        .into_response()
                    }
                },
            ),
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

    // 2. Configure app state in live mode
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
    web_state
        .app
        .is_gateway_ready
        .store(true, std::sync::atomic::Ordering::SeqCst);

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
    let guild_id = "777888999000111";
    let channel_id = "555666777888999";

    // Set max_winners = 3 in module settings
    web_state
        .app
        .storage
        .set_module_settings(
            guild_id,
            "giveaway",
            true,
            serde_json::json!({
                "max_winners": 3
            }),
        )
        .await
        .unwrap();

    let admin_session_id = "admin_live_sess";
    let admin_csrf = "admin_live_csrf";
    web_state
        .app
        .sessions
        .save_session(
            admin_session_id,
            DashboardUser {
                id: "111111111111111111".to_owned(),
                username: "admin".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Live GA Guild".to_owned(),
                icon: None,
                owner: false,
                permissions: "32".to_owned(),
            }],
            admin_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    web_state.permissions_cache.insert(
        ("111111111111111111".to_owned(), guild_id.to_owned()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let no_view_session_id = "no_view_live_sess";
    let no_view_csrf = "no_view_live_csrf";
    web_state
        .app
        .sessions
        .save_session(
            no_view_session_id,
            DashboardUser {
                id: "222222222222222222".to_owned(),
                username: "noview".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Live GA Guild".to_owned(),
                icon: None,
                owner: false,
                permissions: "0".to_owned(),
            }],
            no_view_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    let admin_cookie = format!("rusty_discord_bot_session={admin_session_id}");
    let no_view_cookie = format!("rusty_discord_bot_session={no_view_session_id}");

    // Test 1: POST /api/guilds/{guild_id}/giveaways with winners_count = 5 exceeds max_winners (3) -> 400 Bad Request
    let over_limit_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/giveaways"))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_id,
            "prize": "Too many winners",
            "winners_count": 5,
            "duration_minutes": 60
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(over_limit_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Test 2: POST /api/guilds/{guild_id}/giveaways with winners_count = 2 succeeds
    let create_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/giveaways"))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_id,
            "prize": "Legit Nitro",
            "winners_count": 2,
            "duration_minutes": 60
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), reqwest::StatusCode::OK);
    let create_ga: rusty_discord_bot::storage::Giveaway = create_resp.json().await.unwrap();
    assert_eq!(
        create_ga.created_by_user_id.as_deref(),
        Some("111111111111111111")
    );

    // Test 3: user_without_view accessing GET /api/guilds/{guild_id}/giveaways/{id} -> 403 Forbidden
    let get_forbidden_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}",
            create_ga.id
        ))
        .header(COOKIE, &no_view_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_forbidden_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 4: user_without_view attempting POST .../join -> 403 Forbidden
    let join_forbidden_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}/join",
            create_ga.id
        ))
        .header(COOKIE, &no_view_cookie)
        .header("x-csrf-token", no_view_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(join_forbidden_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 5: user_with_view accessing GET -> 200 OK
    let get_ok_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}",
            create_ga.id
        ))
        .header(COOKIE, &admin_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_ok_resp.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn test_giveaway_cross_guild_channel_isolation_live_mode() {
    use axum::response::IntoResponse;
    use axum::routing::get;

    let guild_a = "777888999000111";
    let guild_b = "888999000111222";

    let channel_a = "111111111111111101"; // belongs to guild A (text)
    let channel_b = "222222222222222201"; // belongs to guild B (text)
    let channel_dm = "333333333333333301"; // DM channel (type 1, guild_id null)
    let channel_category = "444444444444444401"; // category channel (type 4, guild A)
    let channel_voice = "555555555555555501"; // voice channel (type 2, guild A)
    let channel_forbidden = "666666666666666601"; // bot forbidden (403)
    let channel_upstream_err = "777777777777777701"; // upstream error (500)

    // 1. Mock Discord API
    let discord_mock_app = axum::Router::new()
        .route(
            "/channels/{channel_id}",
            get(
                |axum::extract::Path(cid): axum::extract::Path<String>| async move {
                    match cid.as_str() {
                        "111111111111111101" => axum::Json(serde_json::json!({
                            "id": "111111111111111101",
                            "guild_id": "777888999000111",
                            "type": 0
                        }))
                        .into_response(),
                        "222222222222222201" => axum::Json(serde_json::json!({
                            "id": "222222222222222201",
                            "guild_id": "888999000111222",
                            "type": 0
                        }))
                        .into_response(),
                        "333333333333333301" => axum::Json(serde_json::json!({
                            "id": "333333333333333301",
                            "guild_id": null,
                            "type": 1
                        }))
                        .into_response(),
                        "444444444444444401" => axum::Json(serde_json::json!({
                            "id": "444444444444444401",
                            "guild_id": "777888999000111",
                            "type": 4
                        }))
                        .into_response(),
                        "555555555555555501" => axum::Json(serde_json::json!({
                            "id": "555555555555555501",
                            "guild_id": "777888999000111",
                            "type": 2
                        }))
                        .into_response(),
                        "666666666666666601" => (
                            reqwest::StatusCode::FORBIDDEN,
                            axum::Json(serde_json::json!({
                                "code": 50001,
                                "message": "Missing Access"
                            })),
                        )
                            .into_response(),
                        "777777777777777701" => (
                            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                            axum::Json(serde_json::json!({
                                "code": 0,
                                "message": "Internal Discord Error"
                            })),
                        )
                            .into_response(),
                        _ => (
                            reqwest::StatusCode::NOT_FOUND,
                            axum::Json(serde_json::json!({
                                "code": 10003,
                                "message": "Unknown Channel"
                            })),
                        )
                            .into_response(),
                    }
                },
            ),
        )
        .route(
            "/channels/{channel_id}/permissions/member/{user_id}",
            get(
                |axum::extract::Path((_channel_id, _user_id)): axum::extract::Path<(
                    String,
                    String,
                )>| async move {
                    // Both admin A and admin B have VIEW_CHANNEL (1024) in the tested channels
                    axum::Json(serde_json::json!({
                        "permissions": "1024"
                    }))
                    .into_response()
                },
            ),
        )
        .route(
            "/guilds/{guild_id}/members/{user_id}",
            get(
                |axum::extract::Path((_guild_id, user_id)): axum::extract::Path<(
                    String,
                    String,
                )>| async move {
                    axum::Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": []
                    }))
                    .into_response()
                },
            ),
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

    // 2. Configure app state in live mode
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
    web_state
        .app
        .is_gateway_ready
        .store(true, std::sync::atomic::Ordering::SeqCst);

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

    // Sessions for Admin A (Guild A manager) and Admin B (Guild B manager)
    let admin_a_sess = "admin_a_sess_token";
    let admin_a_csrf = "admin_a_csrf_token";
    let admin_a_id = "111111111111111111";

    web_state
        .app
        .sessions
        .save_session(
            admin_a_sess,
            DashboardUser {
                id: admin_a_id.to_owned(),
                username: "admin_a".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![
                DashboardGuild {
                    id: guild_a.to_owned(),
                    name: "Guild A".to_owned(),
                    icon: None,
                    owner: false,
                    permissions: "32".to_owned(), // MANAGE_GUILD
                },
                DashboardGuild {
                    id: guild_b.to_owned(),
                    name: "Guild B".to_owned(),
                    icon: None,
                    owner: false,
                    permissions: "0".to_owned(), // Non-manager in Guild B
                },
            ],
            admin_a_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    web_state.permissions_cache.insert(
        (admin_a_id.to_owned(), guild_a.to_owned()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let admin_b_sess = "admin_b_sess_token";
    let admin_b_csrf = "admin_b_csrf_token";
    let admin_b_id = "222222222222222222";

    web_state
        .app
        .sessions
        .save_session(
            admin_b_sess,
            DashboardUser {
                id: admin_b_id.to_owned(),
                username: "admin_b".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_b.to_owned(),
                name: "Guild B".to_owned(),
                icon: None,
                owner: false,
                permissions: "32".to_owned(), // MANAGE_GUILD in B
            }],
            admin_b_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    web_state.permissions_cache.insert(
        (admin_b_id.to_owned(), guild_b.to_owned()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let cookie_a = format!("rusty_discord_bot_session={admin_a_sess}");
    let cookie_b = format!("rusty_discord_bot_session={admin_b_sess}");

    // Case 1: Admin A attempts POST /api/guilds/{guild_a}/giveaways with channel_b (Foreign Guild B)
    // -> 403 Forbidden ("Kanał konkursu musi należeć do tej gildii"), 0 database writes
    let cross_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/giveaways"))
        .header(COOKIE, &cookie_a)
        .header("x-csrf-token", admin_a_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_b,
            "prize": "Cross-Guild Attack",
            "winners_count": 1,
            "duration_minutes": 60
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(cross_resp.status(), reqwest::StatusCode::FORBIDDEN);
    let error_body: serde_json::Value = cross_resp.json().await.unwrap();
    assert!(
        error_body["error"]
            .as_str()
            .unwrap_or("")
            .contains("Kanał konkursu musi należeć do tej gildii")
    );

    // Case 2: Admin A attempts POST with channel_dm (DM channel, guild_id is null)
    // -> 403 Forbidden
    let dm_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/giveaways"))
        .header(COOKIE, &cookie_a)
        .header("x-csrf-token", admin_a_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_dm,
            "prize": "DM Attack",
            "winners_count": 1,
            "duration_minutes": 60
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(dm_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Case 3: Admin A attempts POST with channel_category (type 4, Category)
    // -> 400 Bad Request
    let cat_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/giveaways"))
        .header(COOKIE, &cookie_a)
        .header("x-csrf-token", admin_a_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_category,
            "prize": "Category Attack",
            "winners_count": 1,
            "duration_minutes": 60
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(cat_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Case 4: Admin A attempts POST with channel_voice (type 2, Voice)
    // -> 400 Bad Request
    let voice_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/giveaways"))
        .header(COOKIE, &cookie_a)
        .header("x-csrf-token", admin_a_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_voice,
            "prize": "Voice Attack",
            "winners_count": 1,
            "duration_minutes": 60
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(voice_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Case 5: Admin A attempts POST with channel_forbidden (bot has no access / 403)
    // -> 403 Forbidden
    let forb_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/giveaways"))
        .header(COOKIE, &cookie_a)
        .header("x-csrf-token", admin_a_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_forbidden,
            "prize": "Inaccessible Channel Attack",
            "winners_count": 1,
            "duration_minutes": 60
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(forb_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Case 6: Admin A attempts POST with channel_upstream_err (upstream 500 error)
    // -> 403 Forbidden (fail closed)
    let upstr_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/giveaways"))
        .header(COOKIE, &cookie_a)
        .header("x-csrf-token", admin_a_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_upstream_err,
            "prize": "Upstream Error Channel",
            "winners_count": 1,
            "duration_minutes": 60
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(upstr_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Case 7: Admin A attempts POST with legitimate channel_a belonging to Guild A
    // -> 200 OK, persistent giveaway created
    let legit_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/giveaways"))
        .header(COOKIE, &cookie_a)
        .header("x-csrf-token", admin_a_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_a,
            "prize": "Legitimate Guild A Prize",
            "winners_count": 1,
            "duration_minutes": 60
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(legit_resp.status(), reqwest::StatusCode::OK);
    let ga_a: rusty_discord_bot::storage::Giveaway = legit_resp.json().await.unwrap();
    assert_eq!(ga_a.guild_id, guild_a);
    assert_eq!(ga_a.channel_id, channel_a);

    // Case 8: Multi-tenant G1 / G2 isolation:
    // Admin B in Guild B creates a giveaway targeting channel_b in Guild B -> 200 OK
    let legit_b_resp = client
        .post(format!("{base_url}/api/guilds/{guild_b}/giveaways"))
        .header(COOKIE, &cookie_b)
        .header("x-csrf-token", admin_b_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_b,
            "prize": "Legitimate Guild B Prize",
            "winners_count": 1,
            "duration_minutes": 60
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(legit_b_resp.status(), reqwest::StatusCode::OK);
    let ga_b: rusty_discord_bot::storage::Giveaway = legit_b_resp.json().await.unwrap();
    assert_eq!(ga_b.guild_id, guild_b);
    assert_eq!(ga_b.channel_id, channel_b);

    // Admin B attempts to access Guild A's giveaway -> 403 Forbidden (not a member of Guild A)
    let b_access_a_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_a}/giveaways/{}",
            ga_a.id
        ))
        .header(COOKIE, &cookie_b)
        .send()
        .await
        .unwrap();
    assert_eq!(b_access_a_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Admin A attempts to create giveaway in Guild B targeting channel_a -> 403 Forbidden (Admin A cannot manage B)
    let a_target_b_resp = client
        .post(format!("{base_url}/api/guilds/{guild_b}/giveaways"))
        .header(COOKIE, &cookie_a)
        .header("x-csrf-token", admin_a_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_a,
            "prize": "Forbidden Cross Guild",
            "winners_count": 1,
            "duration_minutes": 60
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(a_target_b_resp.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_reaction_roles_web_api_and_isolation() {
    use axum::response::IntoResponse;
    use axum::routing::get;

    let guild_a = "777888999000111";
    let guild_b = "888999000111222";
    let channel_a = "111111111111111101"; // belongs to guild A
    let channel_b = "222222222222222201"; // belongs to guild B

    // 1. Mock Discord API for channel view verification
    let discord_mock_app = axum::Router::new()
        .route(
            "/channels/{channel_id}",
            get(
                |axum::extract::Path(cid): axum::extract::Path<String>| async move {
                    match cid.as_str() {
                        "111111111111111101" => axum::Json(serde_json::json!({
                            "id": "111111111111111101",
                            "guild_id": "777888999000111",
                            "type": 0
                        }))
                        .into_response(),
                        "222222222222222201" => axum::Json(serde_json::json!({
                            "id": "222222222222222201",
                            "guild_id": "888999000111222",
                            "type": 0
                        }))
                        .into_response(),
                        _ => (
                            axum::http::StatusCode::NOT_FOUND,
                            axum::Json(serde_json::json!({"message": "Not Found"})),
                        )
                            .into_response(),
                    }
                },
            ),
        )
        .route(
            "/guilds/{guild_id}/members/{user_id}",
            get(
                |axum::extract::Path((_guild_id, user_id)): axum::extract::Path<(
                    String,
                    String,
                )>| async move {
                    axum::Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": []
                    }))
                    .into_response()
                },
            ),
        )
        .route(
            "/channels/{channel_id}/permissions/member/{user_id}",
            get(
                |axum::extract::Path((_channel_id, _user_id)): axum::extract::Path<(
                    String,
                    String,
                )>| async move {
                    axum::Json(serde_json::json!({
                        "permissions": "1024"
                    }))
                    .into_response()
                },
            ),
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

    // 2. Configure app state in live mode
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
    web_state
        .app
        .is_gateway_ready
        .store(true, std::sync::atomic::Ordering::SeqCst);

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

    // Sessions for Admin A and Admin B
    let admin_a_sess = "admin_a_sess_token_rr";
    let admin_a_csrf = "admin_a_csrf_token_rr";
    let admin_a_id = "111111111111111111";

    web_state
        .app
        .sessions
        .save_session(
            admin_a_sess,
            DashboardUser {
                id: admin_a_id.to_owned(),
                username: "admin_a".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![
                DashboardGuild {
                    id: guild_a.to_owned(),
                    name: "Guild A".to_owned(),
                    icon: None,
                    owner: false,
                    permissions: "32".to_owned(), // MANAGE_GUILD
                },
                DashboardGuild {
                    id: guild_b.to_owned(),
                    name: "Guild B".to_owned(),
                    icon: None,
                    owner: false,
                    permissions: "0".to_owned(),
                },
            ],
            admin_a_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    web_state.permissions_cache.insert(
        (admin_a_id.to_owned(), guild_a.to_owned()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let admin_b_sess = "admin_b_sess_token_rr";
    let admin_b_csrf = "admin_b_csrf_token_rr";
    let admin_b_id = "222222222222222222";

    web_state
        .app
        .sessions
        .save_session(
            admin_b_sess,
            DashboardUser {
                id: admin_b_id.to_owned(),
                username: "admin_b".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![
                DashboardGuild {
                    id: guild_b.to_owned(),
                    name: "Guild B".to_owned(),
                    icon: None,
                    owner: false,
                    permissions: "32".to_owned(), // MANAGE_GUILD
                },
                DashboardGuild {
                    id: guild_a.to_owned(),
                    name: "Guild A".to_owned(),
                    icon: None,
                    owner: false,
                    permissions: "0".to_owned(),
                },
            ],
            admin_b_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    web_state.permissions_cache.insert(
        (admin_b_id.to_owned(), guild_b.to_owned()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let cookie_a = format!("rusty_discord_bot_session={admin_a_sess}");
    let cookie_b = format!("rusty_discord_bot_session={admin_b_sess}");

    // 1. Initial list for Guild A is empty
    let list_a_resp = client
        .get(format!("{base_url}/api/guilds/{guild_a}/reaction-roles"))
        .header(COOKIE, &cookie_a)
        .send()
        .await
        .unwrap();
    assert_eq!(list_a_resp.status(), reqwest::StatusCode::OK);
    let list_a_body: serde_json::Value = list_a_resp.json().await.unwrap();
    assert_eq!(list_a_body["reaction_roles"].as_array().unwrap().len(), 0);

    // 2. Admin A creates reaction role in Guild A -> 200 OK
    let create_a_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/reaction-roles"))
        .header(COOKIE, &cookie_a)
        .header("x-csrf-token", admin_a_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_a,
            "message_id": "900000000000000001",
            "emoji": "🎉",
            "role_id": "1002"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_a_resp.status(), reqwest::StatusCode::OK);
    let created_a: serde_json::Value = create_a_resp.json().await.unwrap();
    assert_eq!(created_a["status"], "created");
    let binding_a_id = created_a["reaction_role"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // 3. Admin A attempts without CSRF -> 403 Forbidden
    let no_csrf_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/reaction-roles"))
        .header(COOKIE, &cookie_a)
        .json(&serde_json::json!({
            "channel_id": channel_a,
            "message_id": "900000000000000002",
            "emoji": "⭐",
            "role_id": "1002"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(no_csrf_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 4. Admin A attempts cross-guild channel (channel_b in Guild A) -> 403 Forbidden
    let cross_ch_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/reaction-roles"))
        .header(COOKIE, &cookie_a)
        .header("x-csrf-token", admin_a_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_b, // belongs to Guild B!
            "message_id": "900000000000000003",
            "emoji": "⭐",
            "role_id": "1002"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(cross_ch_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 5. Admin B creates reaction role in Guild B -> 200 OK
    let create_b_resp = client
        .post(format!("{base_url}/api/guilds/{guild_b}/reaction-roles"))
        .header(COOKIE, &cookie_b)
        .header("x-csrf-token", admin_b_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_b,
            "message_id": "900000000000000001",
            "emoji": "🎉",
            "role_id": "1001"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_b_resp.status(), reqwest::StatusCode::OK);

    // 6. Multi-tenant isolation: Admin B cannot view Guild A's reaction roles -> 403 Forbidden
    let b_view_a_resp = client
        .get(format!("{base_url}/api/guilds/{guild_a}/reaction-roles"))
        .header(COOKIE, &cookie_b)
        .send()
        .await
        .unwrap();
    assert_eq!(b_view_a_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 7. Multi-tenant isolation: Admin B cannot delete Guild A's binding from Guild B -> 404 Not Found
    let b_del_a_resp = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_b}/reaction-roles/{binding_a_id}"
        ))
        .header(COOKIE, &cookie_b)
        .header("x-csrf-token", admin_b_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(b_del_a_resp.status(), reqwest::StatusCode::NOT_FOUND);

    // 8. Admin A deletes binding in Guild A -> 200 OK
    let del_a_resp = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_a}/reaction-roles/{binding_a_id}"
        ))
        .header(COOKIE, &cookie_a)
        .header("x-csrf-token", admin_a_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(del_a_resp.status(), reqwest::StatusCode::OK);

    // 9. Verify Guild A's list is empty again
    let list_a_after = client
        .get(format!("{base_url}/api/guilds/{guild_a}/reaction-roles"))
        .header(COOKIE, &cookie_a)
        .send()
        .await
        .unwrap();
    let list_a_after_body: serde_json::Value = list_a_after.json().await.unwrap();
    assert_eq!(
        list_a_after_body["reaction_roles"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}
