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

    // 9a. Reconcile guild duty with dry_run=true
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

    // 10. Delete profile
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
