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
