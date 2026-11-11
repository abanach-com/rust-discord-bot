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
            ensure_manage_roles_access,
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
        integration_credential_key: None,
        integration_credential_active_key_id: "v1".to_string(),
        integration_credential_keyring: None,
        integration_credentials_disabled: false,
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
    let state = WebState::new(
        app,
        Arc::new(LoginRateLimiter::new()),
        permissions_cache.clone(),
    );
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
    assert!(
        parsed_url
            .query_pairs()
            .find(|(k, _)| k == "prompt")
            .is_none(),
        "OAuth authorize URL must not force prompt=none (#337)"
    );
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

    // 4. Discord denial / error callback (?error=access_denied&state=...) with matching browser cookie (#337)
    let denial_resp = client
        .get(format!(
            "{base_url}/auth/discord/callback?error=access_denied&state={state_param}"
        ))
        .header(COOKIE, format!("rusty_oauth_state={state_param}"))
        .send()
        .await
        .expect("denial callback request");

    assert_eq!(denial_resp.status(), reqwest::StatusCode::BAD_REQUEST);
    let denial_body: serde_json::Value = denial_resp.json().await.expect("json denial body");
    assert_eq!(
        denial_body["error"],
        "Discord authorization was denied or requires new consent. Please log in again."
    );

    // State must have been consumed atomically by the denial callback
    assert!(
        !state
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
    let web_state = WebState::new(
        app,
        std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    );
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
async fn test_moderation_appeal_duplicate_prevention_issue_345() {
    let (state, _) = test_web_state().await;

    let guild_id = "111000000000000001";
    let user_id = "888000000000000001";
    let case_number = 42;

    // 1. Initial appeal creation succeeds
    let appeal1 = rusty_discord_bot::storage::ModerationAppeal {
        id: uuid::Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        case_number,
        user_id: user_id.to_string(),
        status: "pending".to_string(),
        reason: "Initial appeal".to_string(),
        evidence_url: None,
        staff_notes: None,
        created_at: Utc::now() - Duration::hours(1),
        updated_at: Utc::now() - Duration::hours(1),
    };

    let outcome1 = state.app.storage.create_appeal(&appeal1).await.unwrap();
    assert_eq!(
        outcome1,
        rusty_discord_bot::storage::CreateAppealOutcome::Created(appeal1.id)
    );

    // 2. Duplicate appeal creation for same (guild_id, case_number, user_id) with pending status fails
    let appeal2 = rusty_discord_bot::storage::ModerationAppeal {
        id: uuid::Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        case_number,
        user_id: user_id.to_string(),
        status: "pending".to_string(),
        reason: "Spam duplicate appeal".to_string(),
        evidence_url: None,
        staff_notes: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let outcome2 = state.app.storage.create_appeal(&appeal2).await.unwrap();
    assert_eq!(
        outcome2,
        rusty_discord_bot::storage::CreateAppealOutcome::AlreadyPending(appeal1.id)
    );

    // List appeals verifies only 1 pending appeal is present
    let pending_list = state
        .app
        .storage
        .list_appeals(guild_id, Some("pending"), Some(10))
        .await
        .unwrap();
    assert_eq!(pending_list.len(), 1);
    assert_eq!(pending_list[0].id, appeal1.id);

    // 3. Different user or different case number can create appeal without being blocked
    let appeal_other_case = rusty_discord_bot::storage::ModerationAppeal {
        id: uuid::Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        case_number: 43,
        user_id: user_id.to_string(),
        status: "pending".to_string(),
        reason: "Appeal for another case".to_string(),
        evidence_url: None,
        staff_notes: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let outcome_other = state
        .app
        .storage
        .create_appeal(&appeal_other_case)
        .await
        .unwrap();
    assert_eq!(
        outcome_other,
        rusty_discord_bot::storage::CreateAppealOutcome::Created(appeal_other_case.id)
    );

    // 4. Once first appeal is decided (e.g. rejected), submitting a new appeal succeeds
    let updated = state
        .app
        .storage
        .update_appeal_status(guild_id, appeal1.id, "rejected", Some("Denied by staff"))
        .await
        .unwrap();
    assert!(updated);

    let appeal3 = rusty_discord_bot::storage::ModerationAppeal {
        id: uuid::Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        case_number,
        user_id: user_id.to_string(),
        status: "pending".to_string(),
        reason: "Second appeal after denial".to_string(),
        evidence_url: None,
        staff_notes: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let outcome3 = state.app.storage.create_appeal(&appeal3).await.unwrap();
    assert_eq!(
        outcome3,
        rusty_discord_bot::storage::CreateAppealOutcome::Created(appeal3.id)
    );

    // 5. Test pagination limit
    let appeals_page = state
        .app
        .storage
        .list_appeals(guild_id, None, Some(2))
        .await
        .unwrap();
    assert_eq!(appeals_page.len(), 2);
}

#[tokio::test]
async fn test_moderation_appeal_concurrent_create_and_decide_deadlock_free_issue_366() {
    let (state, _) = test_web_state().await;
    let guild_id = "111222333444555666";
    let case_number = 42;
    let user_id = "999888777666555444";

    // Initial appeal so there is an active appeal to decide concurrently
    let initial_appeal = rusty_discord_bot::storage::ModerationAppeal {
        id: uuid::Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        case_number,
        user_id: user_id.to_string(),
        status: "pending".to_string(),
        reason: "Initial appeal for concurrent testing".to_string(),
        evidence_url: None,
        staff_notes: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    state
        .app
        .storage
        .create_appeal(&initial_appeal)
        .await
        .unwrap();

    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(40));
    let mut handles = Vec::new();

    // 20 concurrent tasks creating appeals for the exact same case and user
    for i in 0..20 {
        let storage = state.app.storage.clone();
        let b = barrier.clone();
        handles.push(tokio::spawn(async move {
            b.wait().await;
            let appeal = rusty_discord_bot::storage::ModerationAppeal {
                id: uuid::Uuid::new_v4(),
                guild_id: guild_id.to_string(),
                case_number,
                user_id: user_id.to_string(),
                status: "pending".to_string(),
                reason: format!("Concurrent appeal {i}"),
                evidence_url: None,
                staff_notes: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            storage.create_appeal(&appeal).await.unwrap()
        }));
    }

    // 20 concurrent tasks deciding appeals (triggering update_appeal_status concurrently with create_appeal)
    for i in 0..20 {
        let storage = state.app.storage.clone();
        let b = barrier.clone();
        let target_id = initial_appeal.id;
        handles.push(tokio::spawn(async move {
            b.wait().await;
            let _ = storage
                .update_appeal_status(
                    guild_id,
                    target_id,
                    if i % 2 == 0 { "rejected" } else { "approved" },
                    Some("Concurrent decision"),
                )
                .await;
            rusty_discord_bot::storage::CreateAppealOutcome::Created(target_id)
        }));
    }

    // Must finish within 5 seconds without deadlock
    let timeout_res = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        for h in handles {
            h.await.unwrap();
        }
    })
    .await;

    assert!(
        timeout_res.is_ok(),
        "Concurrent create and decide must complete without deadlock"
    );

    // Verify storage consistency: at most one pending appeal ever exists
    let pending_appeals = state
        .app
        .storage
        .list_appeals(guild_id, Some("pending"), Some(100))
        .await
        .unwrap();
    assert!(
        pending_appeals.len() <= 1,
        "At most 1 pending appeal can exist for the case, found: {}",
        pending_appeals.len()
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
    let web_state = WebState::new(
        app,
        std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    );
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
    let state = WebState::new(
        app,
        std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    );

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
async fn test_ensure_manage_roles_access_revocation_and_live_permissions_issue_342() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let scenario_flag = std::sync::Arc::new(AtomicUsize::new(1));
    let flag_clone = scenario_flag.clone();

    let user_id = "123456789012345678";
    let guild_id = "987654321098765432";

    let discord_mock_app = axum::Router::new()
        .route(
            "/guilds/{guild_id}/permissions/member/{user_id}",
            axum::routing::get(move || {
                let flag = flag_clone.clone();
                async move {
                    match flag.load(Ordering::SeqCst) {
                        1 => (
                            // Scenario 1: MANAGE_ROLES revoked, only MANAGE_GUILD (32) remains
                            axum::http::StatusCode::OK,
                            axum::Json(serde_json::json!({ "permissions": "32" })),
                        ),
                        2 => (
                            // Scenario 2: Endpoint returns 404, fallback to member
                            axum::http::StatusCode::NOT_FOUND,
                            axum::Json(serde_json::json!({ "message": "Not Found" })),
                        ),
                        3 => (
                            // Scenario 3: Newly granted MANAGE_ROLES (268435488)
                            axum::http::StatusCode::OK,
                            axum::Json(serde_json::json!({ "permissions": "268435488" })),
                        ),
                        _ => (
                            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                            axum::Json(serde_json::json!({ "message": "Upstream error" })),
                        ),
                    }
                }
            }),
        )
        .route(
            "/guilds/{guild_id}/members/{user_id}",
            axum::routing::get(|| async {
                (
                    axum::http::StatusCode::OK,
                    axum::Json(serde_json::json!({
                        "user": {
                            "id": "123456789012345678",
                            "username": "role_guard_user"
                        },
                        "roles": [] // Standard Discord member response without permissions field
                    })),
                )
            }),
        )
        .route(
            "/guilds/{guild_id}",
            axum::routing::get(|| async {
                (
                    axum::http::StatusCode::OK,
                    axum::Json(serde_json::json!({
                        "id": "987654321098765432",
                        "owner_id": "555555555555555555" // Not the user
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
    let state = WebState::new(
        app,
        std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    );

    // Session 1: Stale snapshot has MANAGE_ROLES (268435488)
    let session_stale_role = Session {
        user: DashboardUser {
            id: user_id.to_owned(),
            username: "role_guard_user".to_owned(),
            global_name: Some("Role Guard User".to_owned()),
            avatar: None,
        },
        guilds: vec![DashboardGuild {
            id: guild_id.to_owned(),
            name: "Role Guard Guild".to_owned(),
            icon: None,
            owner: false,
            permissions: "268435488".to_owned(), // MANAGE_GUILD | MANAGE_ROLES
        }],
        csrf_token: "csrf_role_guard".to_owned(),
        expires_at: Utc::now() + Duration::days(1),
        access_token: None,
    };

    // Scenario 1: MANAGE_ROLES revoked live in Discord (returns 32)
    scenario_flag.store(1, Ordering::SeqCst);
    let result = ensure_manage_roles_access(&state, &session_stale_role, guild_id).await;
    assert!(
        result.is_err(),
        "Revoked MANAGE_ROLES must be rejected even if snapshot has MANAGE_ROLES"
    );
    assert_eq!(
        result.unwrap_err().status,
        axum::http::StatusCode::FORBIDDEN
    );

    // Scenario 2: Endpoint returns 404, fallback member lacks permissions, owner is different -> Fail-closed
    scenario_flag.store(2, Ordering::SeqCst);
    let result = ensure_manage_roles_access(&state, &session_stale_role, guild_id).await;
    assert!(
        result.is_err(),
        "Fallback member without permissions must fail closed and deny access"
    );
    assert_eq!(
        result.unwrap_err().status,
        axum::http::StatusCode::FORBIDDEN
    );

    // Scenario 3: Newly granted MANAGE_ROLES in Discord (user snapshot only had MANAGE_GUILD = 32)
    scenario_flag.store(3, Ordering::SeqCst);
    let session_promoted = Session {
        user: DashboardUser {
            id: user_id.to_owned(),
            username: "role_guard_user".to_owned(),
            global_name: Some("Role Guard User".to_owned()),
            avatar: None,
        },
        guilds: vec![DashboardGuild {
            id: guild_id.to_owned(),
            name: "Role Guard Guild".to_owned(),
            icon: None,
            owner: false,
            permissions: "32".to_owned(), // Snapshot only had MANAGE_GUILD!
        }],
        csrf_token: "csrf_role_guard".to_owned(),
        expires_at: Utc::now() + Duration::days(1),
        access_token: None,
    };
    let result = ensure_manage_roles_access(&state, &session_promoted, guild_id).await;
    assert!(
        result.is_ok(),
        "Newly granted MANAGE_ROLES live in Discord must be permitted: {:?}",
        result.err()
    );

    // Scenario 4: Offline mode check
    let mut offline_config = test_config();
    offline_config.offline_mode = true;
    let offline_app = AppState::bootstrap(offline_config)
        .await
        .expect("bootstrap offline");
    let offline_state = WebState::new(
        offline_app,
        std::sync::Arc::new(LoginRateLimiter::new()),
        std::sync::Arc::new(dashmap::DashMap::new()),
    );

    assert!(
        ensure_manage_roles_access(&offline_state, &session_stale_role, guild_id)
            .await
            .is_ok(),
        "Offline mode with MANAGE_ROLES in snapshot should pass"
    );
    assert!(
        ensure_manage_roles_access(&offline_state, &session_promoted, guild_id)
            .await
            .is_err(),
        "Offline mode without MANAGE_ROLES in snapshot should fail"
    );
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
    let state = WebState::new(app, Arc::new(LoginRateLimiter::new()), permissions_cache);

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
    let web_state = WebState::new(
        app,
        std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    );
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
    let web_state = WebState::new(
        app,
        std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    );
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
            .contains("Kanał musi należeć do tej gildii")
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

    // 1. Mock Discord API for channel view, messages, guild, and role hierarchy verification
    let discord_mock_app = axum::Router::new()
        .route(
            "/users/@me",
            get(|| async move {
                axum::Json(serde_json::json!({
                    "id": "999999999999999999"
                }))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}",
            get(
                |axum::extract::Path(gid): axum::extract::Path<String>| async move {
                    axum::Json(serde_json::json!({
                        "id": gid,
                        "owner_id": "999000000000000000"
                    }))
                    .into_response()
                },
            ),
        )
        .route(
            "/guilds/{guild_id}/roles",
            get(|| async move {
                axum::Json(serde_json::json!([
                    {
                        "id": "1001",
                        "name": "MemberRole",
                        "permissions": "0",
                        "position": 10,
                        "managed": false
                    },
                    {
                        "id": "1002",
                        "name": "MemberRoleA",
                        "permissions": "0",
                        "position": 10,
                        "managed": false
                    },
                    {
                        "id": "1003",
                        "name": "ModRole",
                        "permissions": "268435456", // MANAGE_ROLES
                        "position": 40,
                        "managed": false
                    },
                    {
                        "id": "1004",
                        "name": "BotRole",
                        "permissions": "268435456",
                        "position": 60,
                        "managed": false
                    },
                    {
                        "id": "1005",
                        "name": "AdminRole",
                        "permissions": "8", // ADMINISTRATOR
                        "position": 30,
                        "managed": false
                    },
                    {
                        "id": "1006",
                        "name": "TooHighRole",
                        "permissions": "0",
                        "position": 50, // higher than actor's position 40
                        "managed": false
                    },
                    {
                        "id": "1007",
                        "name": "ManagedIntegrationRole",
                        "permissions": "0",
                        "position": 15,
                        "managed": true
                    }
                ]))
                .into_response()
            }),
        )
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
            "/channels/{channel_id}/messages/{message_id}",
            get(
                |axum::extract::Path((_cid, mid)): axum::extract::Path<(String, String)>| async move {
                    if mid == "900000000000000001" || mid == "900000000000000002" {
                        axum::Json(serde_json::json!({
                            "id": mid
                        }))
                        .into_response()
                    } else {
                        (
                            axum::http::StatusCode::NOT_FOUND,
                            axum::Json(serde_json::json!({"message": "Unknown Message"})),
                        )
                            .into_response()
                    }
                },
            ),
        )
        .route(
            "/guilds/{guild_id}/members/{user_id}",
            get(
                |axum::extract::Path((gid, user_id)): axum::extract::Path<(
                    String,
                    String,
                )>| async move {
                    if user_id == "999999999999999999" {
                        // Bot member
                        axum::Json(serde_json::json!({
                            "user": { "id": user_id },
                            "roles": ["1004"], // position 60
                            "permissions": "268435456"
                        }))
                        .into_response()
                    } else if (user_id == "111111111111111111" && gid == "777888999000111")
                        || (user_id == "222222222222222222" && gid == "888999000111222")
                    {
                        // Admin A in Guild A or Admin B in Guild B: position 40
                        axum::Json(serde_json::json!({
                            "user": { "id": user_id },
                            "roles": ["1003"],
                            "permissions": "268435488" // MANAGE_GUILD | MANAGE_ROLES
                        }))
                        .into_response()
                    } else if user_id == "333333333333333333" && gid == "777888999000111" {
                        // Regular user in Guild A: has MANAGE_GUILD only (32)
                        axum::Json(serde_json::json!({
                            "user": { "id": user_id },
                            "roles": [],
                            "permissions": "32"
                        }))
                        .into_response()
                    } else {
                        // Non-admin or outsider: 0 permissions
                        axum::Json(serde_json::json!({
                            "user": { "id": user_id },
                            "roles": [],
                            "permissions": "0"
                        }))
                        .into_response()
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
    let web_state = WebState::new(
        app,
        std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    );
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

    // Sessions for Admin A (has MANAGE_ROLES | MANAGE_GUILD = 268435488), Admin B, and Regular User (only MANAGE_GUILD = 32)
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
                    permissions: "268435488".to_owned(), // MANAGE_GUILD | MANAGE_ROLES
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
                    permissions: "268435488".to_owned(), // MANAGE_GUILD | MANAGE_ROLES
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

    // Regular User: has MANAGE_GUILD (32) but NOT MANAGE_ROLES (Issue #285 security regression test)
    let no_role_mgmt_sess = "no_role_mgmt_sess_rr";
    let no_role_mgmt_csrf = "no_role_mgmt_csrf_rr";
    let no_role_mgmt_id = "333333333333333333";

    web_state
        .app
        .sessions
        .save_session(
            no_role_mgmt_sess,
            DashboardUser {
                id: no_role_mgmt_id.to_owned(),
                username: "no_role_mgmt".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_a.to_owned(),
                name: "Guild A".to_owned(),
                icon: None,
                owner: false,
                permissions: "32".to_owned(), // MANAGE_GUILD without MANAGE_ROLES
            }],
            no_role_mgmt_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    web_state.permissions_cache.insert(
        (no_role_mgmt_id.to_owned(), guild_a.to_owned()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let cookie_a = format!("rusty_discord_bot_session={admin_a_sess}");
    let cookie_b = format!("rusty_discord_bot_session={admin_b_sess}");
    let cookie_no_roles = format!("rusty_discord_bot_session={no_role_mgmt_sess}");

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

    // Issue #285 Check 1: User with MANAGE_GUILD but without MANAGE_ROLES -> 403 Forbidden!
    let denied_no_role_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/reaction-roles"))
        .header(COOKIE, &cookie_no_roles)
        .header("x-csrf-token", no_role_mgmt_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_a,
            "message_id": "900000000000000001",
            "emoji": "🎉",
            "role_id": "1002"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(denied_no_role_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Issue #285 Check 2: Attempting to configure role with ADMINISTRATOR permissions (role 1005) -> 403 Forbidden!
    let admin_role_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/reaction-roles"))
        .header(COOKIE, &cookie_a)
        .header("x-csrf-token", admin_a_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_a,
            "message_id": "900000000000000001",
            "emoji": "🎉",
            "role_id": "1005"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(admin_role_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Issue #285 Check 3: Attempting to configure role higher than actor's hierarchy (role 1006) -> 403 Forbidden!
    let hierarchy_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/reaction-roles"))
        .header(COOKIE, &cookie_a)
        .header("x-csrf-token", admin_a_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_a,
            "message_id": "900000000000000001",
            "emoji": "🎉",
            "role_id": "1006"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(hierarchy_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Issue #285 Check 4: Attempting to configure managed role (role 1007) -> 400 Bad Request!
    let managed_role_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/reaction-roles"))
        .header(COOKIE, &cookie_a)
        .header("x-csrf-token", admin_a_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_a,
            "message_id": "900000000000000001",
            "emoji": "🎉",
            "role_id": "1007"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(managed_role_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Issue #285 Check 5: Non-existent message -> 400 Bad Request!
    let missing_msg_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/reaction-roles"))
        .header(COOKIE, &cookie_a)
        .header("x-csrf-token", admin_a_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_a,
            "message_id": "900000000000000099",
            "emoji": "🎉",
            "role_id": "1002"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_msg_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // 2. Admin A creates legitimate reaction role in Guild A -> 200 OK
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

    // Issue #287: Re-POSTing the same natural key (guild_a, message_id, emoji) with updated role_id
    // must perform an UPSERT that preserves the original binding ID
    let update_a_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/reaction-roles"))
        .header(COOKIE, &cookie_a)
        .header("x-csrf-token", admin_a_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_a,
            "message_id": "900000000000000001",
            "emoji": "🎉",
            "role_id": "1001"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(update_a_resp.status(), reqwest::StatusCode::OK);
    let updated_a: serde_json::Value = update_a_resp.json().await.unwrap();
    assert_eq!(
        updated_a["reaction_role"]["id"].as_str().unwrap(),
        binding_a_id,
        "UPSERT via Web API must preserve original binding UUID (Issue #287)"
    );
    assert_eq!(updated_a["reaction_role"]["role_id"], "1001");

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
            "message_id": "900000000000000002",
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

    // Issue #285 Check 6: User without MANAGE_ROLES cannot delete binding -> 403 Forbidden!
    let del_no_role_resp = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_a}/reaction-roles/{binding_a_id}"
        ))
        .header(COOKIE, &cookie_no_roles)
        .header("x-csrf-token", no_role_mgmt_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(del_no_role_resp.status(), reqwest::StatusCode::FORBIDDEN);

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

#[tokio::test]
async fn test_store_items_privilege_escalation_guard_suite() {
    use axum::Json;
    use axum::extract::Path;
    use axum::response::IntoResponse;
    use axum::routing::{get, put};
    use reqwest::header::COOKIE;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let guild_a = "777888999000111";

    let role_delivery_count = Arc::new(AtomicUsize::new(0));
    let role_delivery_count_clone = role_delivery_count.clone();

    // 1. Mock Discord API for store role verification and delivery
    let discord_mock_app = axum::Router::new()
        .route(
            "/users/@me",
            get(|| async move {
                Json(serde_json::json!({
                    "id": "999999999999999999"
                }))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}",
            get(|Path(gid): Path<String>| async move {
                Json(serde_json::json!({
                    "id": gid,
                    "owner_id": "999000000000000000"
                }))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}/roles",
            get(|| async move {
                Json(serde_json::json!([
                    {
                        "id": "1002",
                        "name": "MemberRoleA",
                        "permissions": "0",
                        "position": 10,
                        "managed": false
                    },
                    {
                        "id": "1003",
                        "name": "ModRole",
                        "permissions": "268435456", // MANAGE_ROLES
                        "position": 40,
                        "managed": false
                    },
                    {
                        "id": "1004",
                        "name": "BotRole",
                        "permissions": "268435456",
                        "position": 60,
                        "managed": false
                    },
                    {
                        "id": "1005",
                        "name": "AdminRole",
                        "permissions": "8", // ADMINISTRATOR
                        "position": 30,
                        "managed": false
                    },
                    {
                        "id": "1006",
                        "name": "TooHighRole",
                        "permissions": "0",
                        "position": 50, // higher than actor's position 40
                        "managed": false
                    },
                    {
                        "id": "1007",
                        "name": "ManagedIntegrationRole",
                        "permissions": "0",
                        "position": 15,
                        "managed": true
                    }
                ]))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}/members/{user_id}",
            get(|Path((_gid, user_id)): Path<(String, String)>| async move {
                if user_id == "999999999999999999" {
                    // Bot member: position 60
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": ["1004"],
                        "permissions": "268435456"
                    }))
                    .into_response()
                } else if user_id == "111111111111111111" {
                    // Admin A: position 40, MANAGE_GUILD | MANAGE_ROLES
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": ["1003"],
                        "permissions": "268435488"
                    }))
                    .into_response()
                } else if user_id == "333333333333333333" {
                    // Manager without MANAGE_ROLES: MANAGE_GUILD only (32)
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": [],
                        "permissions": "32"
                    }))
                    .into_response()
                } else {
                    // Ordinary member / buyer: 0 permissions
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": [],
                        "permissions": "0"
                    }))
                    .into_response()
                }
            }),
        )
        .route(
            "/guilds/{guild_id}/members/{user_id}/roles/{role_id}",
            put(move || {
                let count = role_delivery_count_clone.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    axum::http::StatusCode::NO_CONTENT.into_response()
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

    // 2. Configure app state in live mode
    let mut config = test_config();
    config.discord_token = "live_bot_token_production_secret".to_string();
    config.offline_mode = false;
    config.discord_api_base_url = format!("http://{discord_addr}");

    let app = AppState::bootstrap(config).await.expect("bootstrap");
    let permissions_cache = std::sync::Arc::new(dashmap::DashMap::new());
    let web_state = WebState::new(
        app,
        std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    );
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
        axum::serve(web_listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{web_addr}");

    // 3. Setup sessions
    // Admin A: has MANAGE_GUILD and MANAGE_ROLES
    let admin_a_sess = "admin_a_sess_store";
    let admin_a_csrf = "admin_a_csrf_store";
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
            vec![DashboardGuild {
                id: guild_a.to_owned(),
                name: "Guild A".to_owned(),
                icon: None,
                owner: false,
                permissions: "268435488".to_owned(),
            }],
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

    // Manager user: has MANAGE_GUILD (32) but NOT MANAGE_ROLES (Issue #290 test)
    let no_role_mgmt_sess = "no_role_mgmt_sess_store";
    let no_role_mgmt_csrf = "no_role_mgmt_csrf_store";
    let no_role_mgmt_id = "333333333333333333";

    web_state
        .app
        .sessions
        .save_session(
            no_role_mgmt_sess,
            DashboardUser {
                id: no_role_mgmt_id.to_owned(),
                username: "no_role_mgmt".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_a.to_owned(),
                name: "Guild A".to_owned(),
                icon: None,
                owner: false,
                permissions: "32".to_owned(),
            }],
            no_role_mgmt_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    web_state.permissions_cache.insert(
        (no_role_mgmt_id.to_owned(), guild_a.to_owned()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    // Ordinary buyer user
    let buyer_sess = "buyer_sess_store";
    let buyer_csrf = "buyer_csrf_store";
    let buyer_id = "444444444444444444";

    web_state
        .app
        .sessions
        .save_session(
            buyer_sess,
            DashboardUser {
                id: buyer_id.to_owned(),
                username: "buyer".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_a.to_owned(),
                name: "Guild A".to_owned(),
                icon: None,
                owner: false,
                permissions: "0".to_owned(),
            }],
            buyer_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    // Give buyer 1000 coins
    web_state
        .app
        .storage
        .adjust_member_progress(guild_a, buyer_id, 0, 1000, 0)
        .await
        .unwrap();

    let cookie_admin = format!("rusty_discord_bot_session={admin_a_sess}");
    let cookie_no_roles = format!("rusty_discord_bot_session={no_role_mgmt_sess}");
    let cookie_buyer = format!("rusty_discord_bot_session={buyer_sess}");

    // Check 1: User with MANAGE_GUILD but without MANAGE_ROLES tries to create role item -> 403 Forbidden!
    let denied_no_roles = client
        .post(format!("{base_url}/api/guilds/{guild_a}/store/items"))
        .header(COOKIE, &cookie_no_roles)
        .header("x-csrf-token", no_role_mgmt_csrf)
        .json(&serde_json::json!({
            "name": "VIP Pass",
            "description": "Exploit attempt",
            "price": 100,
            "role_id": "1002",
            "stock": 5
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(denied_no_roles.status(), reqwest::StatusCode::FORBIDDEN);

    // Check 2: User with MANAGE_GUILD can create items WITHOUT roles -> 200 OK!
    let allowed_no_role_item = client
        .post(format!("{base_url}/api/guilds/{guild_a}/store/items"))
        .header(COOKIE, &cookie_no_roles)
        .header("x-csrf-token", no_role_mgmt_csrf)
        .json(&serde_json::json!({
            "name": "XP Booster",
            "description": "Safe digital good",
            "price": 50,
            "role_id": null,
            "stock": 10
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(allowed_no_role_item.status(), reqwest::StatusCode::OK);

    // Check 3: Admin tries to create item with ADMINISTRATOR role (1005) -> 403 Forbidden!
    let admin_role_item = client
        .post(format!("{base_url}/api/guilds/{guild_a}/store/items"))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_a_csrf)
        .json(&serde_json::json!({
            "name": "Admin Rights",
            "description": "Privilege escalation attempt",
            "price": 0,
            "role_id": "1005",
            "stock": 1
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(admin_role_item.status(), reqwest::StatusCode::FORBIDDEN);

    // Check 4: Admin tries to create item with role higher than actor hierarchy (1006) -> 403 Forbidden!
    let high_role_item = client
        .post(format!("{base_url}/api/guilds/{guild_a}/store/items"))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_a_csrf)
        .json(&serde_json::json!({
            "name": "Too High Role",
            "description": "Hierarchy violation",
            "price": 100,
            "role_id": "1006",
            "stock": 1
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(high_role_item.status(), reqwest::StatusCode::FORBIDDEN);

    // Check 5: Admin tries to create item with managed role (1007) -> 400 Bad Request!
    let managed_role_item = client
        .post(format!("{base_url}/api/guilds/{guild_a}/store/items"))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_a_csrf)
        .json(&serde_json::json!({
            "name": "Integration Role",
            "description": "Managed role attempt",
            "price": 100,
            "role_id": "1007",
            "stock": 1
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(managed_role_item.status(), reqwest::StatusCode::BAD_REQUEST);

    // Check 6: Admin creates legitimate role item (role 1002) -> 200 OK!
    let legitimate_item_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/store/items"))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_a_csrf)
        .json(&serde_json::json!({
            "name": "Member Color",
            "description": "Legitimate vanity role",
            "price": 100,
            "role_id": "1002",
            "stock": 5
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(legitimate_item_resp.status(), reqwest::StatusCode::OK);
    let legitimate_body: serde_json::Value = legitimate_item_resp.json().await.unwrap();
    let legitimate_item_id = legitimate_body["id"].as_str().unwrap();

    // Check 7: Buyer purchases legitimate role item -> completed, 1 HTTP PUT dispatched!
    let initial_deliveries = role_delivery_count.load(Ordering::SeqCst);
    let purchase_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/store/purchase"))
        .header(COOKIE, &cookie_buyer)
        .header("x-csrf-token", buyer_csrf)
        .header("idempotency-key", "buyer_buy_1")
        .json(&serde_json::json!({
            "item_id": legitimate_item_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(purchase_resp.status(), reqwest::StatusCode::OK);
    let purchase_body: serde_json::Value = purchase_resp.json().await.unwrap();
    assert_eq!(purchase_body["fulfillment_status"], "completed");
    assert!(purchase_body["failure_reason"].is_null());
    assert_eq!(
        role_delivery_count.load(Ordering::SeqCst),
        initial_deliveries + 1
    );

    // Check 8: Defense-in-depth: Manually seed a store item with privileged role 1005 into storage
    let privileged_item = web_state
        .app
        .storage
        .create_store_item(
            guild_a,
            "Legacy Corrupt Admin Item",
            "Bypassed validation row",
            100,
            Some("1005"),
            Some(5),
        )
        .await
        .unwrap();

    let deliveries_before_exploit = role_delivery_count.load(Ordering::SeqCst);
    let exploit_purchase_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/store/purchase"))
        .header(COOKIE, &cookie_buyer)
        .header("x-csrf-token", buyer_csrf)
        .header("idempotency-key", "buyer_exploit_1")
        .json(&serde_json::json!({
            "item_id": privileged_item.id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(exploit_purchase_resp.status(), reqwest::StatusCode::OK);
    let exploit_body: serde_json::Value = exploit_purchase_resp.json().await.unwrap();
    assert_eq!(exploit_body["fulfillment_status"], "failed");
    assert!(
        exploit_body["failure_reason"]
            .as_str()
            .unwrap()
            .contains("uprawnienia administracyjne")
    );
    assert_eq!(
        role_delivery_count.load(Ordering::SeqCst),
        deliveries_before_exploit,
        "Privileged role must NEVER be delivered by bot!"
    );
}

#[tokio::test]
async fn test_polls_channel_view_live_mode_and_cross_guild_suite() {
    use axum::response::IntoResponse;
    use axum::routing::get;

    let guild_a = "777888999000111";
    let guild_b = "888999000111222";

    let channel_a = "111111111111111101"; // belongs to guild A (text)
    let channel_b = "222222222222222201"; // belongs to guild B (text)
    let channel_voice = "555555555555555501"; // voice channel (type 2, guild A)
    let channel_fail = "999999999999999999"; // non-existent channel (404)

    let admin_id = "100000000000000001";
    let member_with_view_id = "200000000000000001";
    let member_without_view_id = "300000000000000001";
    let outsider_id = "400000000000000001";

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
                        "555555555555555501" => axum::Json(serde_json::json!({
                            "id": "555555555555555501",
                            "guild_id": "777888999000111",
                            "type": 2
                        }))
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
                |axum::extract::Path((channel_id, user_id)): axum::extract::Path<(
                    String,
                    String,
                )>| async move {
                    if channel_id == "111111111111111101" {
                        if user_id == "100000000000000001" || user_id == "200000000000000001" {
                            // VIEW_CHANNEL is bit 1024 (0x400)
                            axum::Json(serde_json::json!({
                                "permissions": "1024"
                            }))
                            .into_response()
                        } else if user_id == "300000000000000001" {
                            // member without view: permissions = 0
                            axum::Json(serde_json::json!({
                                "permissions": "0"
                            }))
                            .into_response()
                        } else {
                            (
                                reqwest::StatusCode::NOT_FOUND,
                                axum::Json(serde_json::json!({
                                    "code": 10007,
                                    "message": "Unknown Member"
                                })),
                            )
                                .into_response()
                        }
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
                |axum::extract::Path((guild_id, user_id)): axum::extract::Path<(
                    String,
                    String,
                )>| async move {
                    let is_guild_a_member = guild_id == "777888999000111"
                        && (user_id == "100000000000000001"
                            || user_id == "200000000000000001"
                            || user_id == "300000000000000001");
                    let is_guild_b_member =
                        guild_id == "888999000111222" && user_id == "400000000000000001";

                    if is_guild_a_member || is_guild_b_member {
                        axum::Json(serde_json::json!({
                            "user": { "id": user_id },
                            "roles": []
                        }))
                        .into_response()
                    } else {
                        (
                            reqwest::StatusCode::NOT_FOUND,
                            axum::Json(
                                serde_json::json!({"code": 10007, "message": "Unknown Member"}),
                            ),
                        )
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
    let web_state = WebState::new(
        app,
        std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    );
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

    // Set up sessions
    let admin_session_id = "admin_poll_live_sess";
    let admin_csrf = "admin_poll_live_csrf";
    web_state
        .app
        .sessions
        .save_session(
            admin_session_id,
            DashboardUser {
                id: admin_id.to_owned(),
                username: "admin".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_a.to_owned(),
                name: "Guild A".to_owned(),
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
        (admin_id.to_owned(), guild_a.to_owned()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let with_view_session_id = "with_view_poll_live_sess";
    let with_view_csrf = "with_view_poll_live_csrf";
    web_state
        .app
        .sessions
        .save_session(
            with_view_session_id,
            DashboardUser {
                id: member_with_view_id.to_owned(),
                username: "member_with_view".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_a.to_owned(),
                name: "Guild A".to_owned(),
                icon: None,
                owner: false,
                permissions: "0".to_owned(),
            }],
            with_view_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    let no_view_session_id = "no_view_poll_live_sess";
    let no_view_csrf = "no_view_poll_live_csrf";
    web_state
        .app
        .sessions
        .save_session(
            no_view_session_id,
            DashboardUser {
                id: member_without_view_id.to_owned(),
                username: "member_no_view".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_a.to_owned(),
                name: "Guild A".to_owned(),
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

    let outsider_session_id = "outsider_poll_live_sess";
    let outsider_csrf = "outsider_poll_live_csrf";
    web_state
        .app
        .sessions
        .save_session(
            outsider_session_id,
            DashboardUser {
                id: outsider_id.to_owned(),
                username: "outsider".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_b.to_owned(),
                name: "Guild B".to_owned(),
                icon: None,
                owner: false,
                permissions: "0".to_owned(),
            }],
            outsider_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    let admin_cookie = format!("rusty_discord_bot_session={admin_session_id}");
    let with_view_cookie = format!("rusty_discord_bot_session={with_view_session_id}");
    let no_view_cookie = format!("rusty_discord_bot_session={no_view_session_id}");
    let outsider_cookie = format!("rusty_discord_bot_session={outsider_session_id}");

    // Test 1: create_poll with cross-guild channel (channel_b belongs to guild_b) -> 403 Forbidden
    let cross_guild_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/polls"))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_b,
            "question": "Cross-guild test?",
            "options": ["Yes", "No"],
            "duration_minutes": 60
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(cross_guild_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 2: create_poll with voice channel -> 400 Bad Request
    let voice_chan_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/polls"))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_voice,
            "question": "Voice channel poll?",
            "options": ["A", "B"],
            "duration_minutes": 60
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(voice_chan_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Test 3: create_poll with non-existent channel -> 403 Forbidden
    let missing_chan_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/polls"))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_fail,
            "question": "Non-existent channel poll?",
            "options": ["A", "B"],
            "duration_minutes": 60
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_chan_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 4: create_poll with valid text channel in guild_a -> 200 OK
    let create_resp = client
        .post(format!("{base_url}/api/guilds/{guild_a}/polls"))
        .header(COOKIE, &admin_cookie)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "channel_id": channel_a,
            "question": "Private channel confidential poll?",
            "options": ["Feature A", "Feature B"],
            "duration_minutes": 60
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), reqwest::StatusCode::OK);
    let poll_data: rusty_discord_bot::storage::CommunityPoll = create_resp.json().await.unwrap();
    let poll_id = poll_data.id;

    // Test 5: member_without_view accesses GET /api/guilds/{guild_a}/polls/{poll_id} -> 403 Forbidden!
    let get_no_view_resp = client
        .get(format!("{base_url}/api/guilds/{guild_a}/polls/{poll_id}"))
        .header(COOKIE, &no_view_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_no_view_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 6: member_without_view attempts POST .../vote -> 403 Forbidden!
    let vote_no_view_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/polls/{poll_id}/vote"
        ))
        .header(COOKIE, &no_view_cookie)
        .header("x-csrf-token", no_view_csrf)
        .json(&serde_json::json!({ "option_idx": 0 }))
        .send()
        .await
        .unwrap();
    assert_eq!(vote_no_view_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Verify tallies remain [0, 0] in storage
    let stored_poll = web_state
        .app
        .storage
        .get_community_poll(poll_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored_poll.tallies_json, serde_json::json!([0, 0]));

    // Test 7: unauthenticated request -> 401 Unauthorized
    let unauth_resp = client
        .get(format!("{base_url}/api/guilds/{guild_a}/polls/{poll_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // Test 8: member of guild_b (outsider) -> 403 Forbidden
    let outsider_resp = client
        .get(format!("{base_url}/api/guilds/{guild_a}/polls/{poll_id}"))
        .header(COOKIE, &outsider_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(outsider_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 9: member_with_view accesses GET -> 200 OK with poll details
    let get_with_view_resp = client
        .get(format!("{base_url}/api/guilds/{guild_a}/polls/{poll_id}"))
        .header(COOKIE, &with_view_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_with_view_resp.status(), reqwest::StatusCode::OK);
    let view_body: serde_json::Value = get_with_view_resp.json().await.unwrap();
    assert_eq!(view_body["question"], "Private channel confidential poll?");

    // Test 10: member_with_view votes without CSRF -> 403 Forbidden
    let no_csrf_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/polls/{poll_id}/vote"
        ))
        .header(COOKIE, &with_view_cookie)
        .json(&serde_json::json!({ "option_idx": 0 }))
        .send()
        .await
        .unwrap();
    assert_eq!(no_csrf_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 11: member_with_view votes with valid CSRF -> 200 OK
    let vote_ok_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/polls/{poll_id}/vote"
        ))
        .header(COOKIE, &with_view_cookie)
        .header("x-csrf-token", with_view_csrf)
        .json(&serde_json::json!({ "option_idx": 0 }))
        .send()
        .await
        .unwrap();
    assert_eq!(vote_ok_resp.status(), reqwest::StatusCode::OK);
    let updated_body: serde_json::Value = vote_ok_resp.json().await.unwrap();
    assert_eq!(updated_body["tallies_json"][0], 1);
    assert_eq!(updated_body["tallies_json"][1], 0);

    // Test 12: idempotent voting: same option keeps tallies unchanged -> 200 OK
    let repeat_vote_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/polls/{poll_id}/vote"
        ))
        .header(COOKIE, &with_view_cookie)
        .header("x-csrf-token", with_view_csrf)
        .json(&serde_json::json!({ "option_idx": 0 }))
        .send()
        .await
        .unwrap();
    assert_eq!(repeat_vote_resp.status(), reqwest::StatusCode::OK);
    let repeat_body: serde_json::Value = repeat_vote_resp.json().await.unwrap();
    assert_eq!(repeat_body["tallies_json"][0], 1);
    assert_eq!(repeat_body["tallies_json"][1], 0);
}

#[tokio::test]
async fn test_giveaway_private_thread_membership_authorization_suite() {
    use axum::response::IntoResponse;
    use axum::routing::get;

    let guild_id = "777888999000111";
    let private_thread_id = "120000000000000001"; // type 12 GUILD_PRIVATE_THREAD
    let public_thread_id = "110000000000000001"; // type 11 GUILD_PUBLIC_THREAD
    let text_channel_id = "100000000000000001"; // type 0 GUILD_TEXT

    let member_in_thread_id = "111111111111111111";
    let member_not_in_thread_id = "222222222222222222";
    let thread_moderator_id = "333333333333333333";
    let admin_id = "444444444444444444";
    let member_error_id = "555555555555555555";

    // 1. Mock Discord API
    let discord_mock_app = axum::Router::new()
        .route(
            "/channels/{channel_id}",
            get(
                |axum::extract::Path(channel_id): axum::extract::Path<String>| async move {
                    if channel_id == "120000000000000001" {
                        axum::Json(serde_json::json!({
                            "id": channel_id,
                            "guild_id": "777888999000111",
                            "type": 12
                        }))
                        .into_response()
                    } else if channel_id == "110000000000000001" {
                        axum::Json(serde_json::json!({
                            "id": channel_id,
                            "guild_id": "777888999000111",
                            "type": 11
                        }))
                        .into_response()
                    } else if channel_id == "100000000000000001" {
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
                    if user_id == "111111111111111111"
                        || user_id == "222222222222222222"
                        || user_id == "555555555555555555"
                    {
                        // VIEW_CHANNEL is bit 1024 (0x400) - inherited from parent overwrites
                        axum::Json(serde_json::json!({
                            "permissions": "1024"
                        }))
                        .into_response()
                    } else if user_id == "333333333333333333" {
                        // MANAGE_THREADS is (1 << 34) = 17179869184 + VIEW_CHANNEL (1024)
                        axum::Json(serde_json::json!({
                            "permissions": "17179870208"
                        }))
                        .into_response()
                    } else if user_id == "444444444444444444" {
                        // ADMINISTRATOR is bit 8
                        axum::Json(serde_json::json!({
                            "permissions": "8"
                        }))
                        .into_response()
                    } else {
                        (
                            reqwest::StatusCode::NOT_FOUND,
                            axum::Json(serde_json::json!({
                                "code": 10003,
                                "message": "Unknown Member"
                            })),
                        )
                            .into_response()
                    }
                },
            ),
        )
        .route(
            "/channels/{channel_id}/thread-members/{user_id}",
            get(
                |axum::extract::Path((channel_id, user_id)): axum::extract::Path<(
                    String,
                    String,
                )>| async move {
                    if user_id == "111111111111111111" {
                        axum::Json(serde_json::json!({
                            "id": channel_id,
                            "user_id": user_id,
                            "join_timestamp": "2026-11-07T12:00:00+00:00",
                            "flags": 0
                        }))
                        .into_response()
                    } else if user_id == "555555555555555555" {
                        (
                            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                            axum::Json(serde_json::json!({
                                "code": 500,
                                "message": "Discord API upstream internal outage"
                            })),
                        )
                            .into_response()
                    } else {
                        (
                            reqwest::StatusCode::NOT_FOUND,
                            axum::Json(serde_json::json!({
                                "code": 10003,
                                "message": "Unknown Thread Member"
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
    config.discord_token = "live_bot_token_thread_test_secret".to_string();
    config.offline_mode = false;
    config.discord_api_base_url = format!("http://{discord_addr}");

    let app = AppState::bootstrap(config).await.expect("bootstrap");
    let permissions_cache = std::sync::Arc::new(dashmap::DashMap::new());
    let web_state = WebState::new(
        app,
        std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    );

    // 3. Register user sessions
    let make_session = |user_id: &str, username: &str, sess_id: &str, csrf: &str| {
        (
            sess_id.to_string(),
            DashboardUser {
                id: user_id.to_string(),
                username: username.to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Test Guild".to_string(),
                icon: None,
                owner: user_id == admin_id,
                permissions: if user_id == admin_id {
                    "8".to_string()
                } else {
                    "0".to_string()
                },
            }],
            csrf.to_string(),
        )
    };

    let users = [
        (
            member_in_thread_id,
            "member_in_thread",
            "sess_in_thread",
            "csrf_in_thread",
        ),
        (
            member_not_in_thread_id,
            "member_not_in_thread",
            "sess_not_in_thread",
            "csrf_not_in_thread",
        ),
        (
            thread_moderator_id,
            "thread_moderator",
            "sess_mod",
            "csrf_mod",
        ),
        (admin_id, "admin", "sess_admin", "csrf_admin"),
        (member_error_id, "member_error", "sess_err", "csrf_err"),
    ];

    for (uid, uname, sid, csrf) in users {
        let (s_id, d_user, d_guilds, s_csrf) = make_session(uid, uname, sid, csrf);
        web_state
            .app
            .sessions
            .save_session(
                &s_id,
                d_user,
                d_guilds,
                s_csrf,
                Utc::now() + Duration::hours(1),
                None,
            )
            .await
            .unwrap();
    }

    // Set can_manage for admin
    web_state.permissions_cache.insert(
        (admin_id.to_string(), guild_id.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    // 4. Create active giveaways in storage for test channels
    let expires = Utc::now() + Duration::hours(2);
    let ga_private = web_state
        .app
        .storage
        .create_giveaway(
            guild_id,
            private_thread_id,
            "Private Thread Secret Prize",
            1,
            expires,
        )
        .await
        .expect("create private thread giveaway");

    let ga_public = web_state
        .app
        .storage
        .create_giveaway(
            guild_id,
            public_thread_id,
            "Public Thread Prize",
            1,
            expires,
        )
        .await
        .expect("create public thread giveaway");

    let ga_text = web_state
        .app
        .storage
        .create_giveaway(guild_id, text_channel_id, "Text Channel Prize", 1, expires)
        .await
        .expect("create text channel giveaway");

    // 5. Start web server
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind app");
    let server_addr = listener.local_addr().expect("local addr");
    let app_router = build_router(web_state.clone());

    tokio::spawn(async move {
        axum::serve(
            listener,
            app_router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{server_addr}");

    let cookie_in_thread = "rusty_discord_bot_session=sess_in_thread".to_string();
    let cookie_not_in_thread = "rusty_discord_bot_session=sess_not_in_thread".to_string();
    let cookie_mod = "rusty_discord_bot_session=sess_mod".to_string();
    let cookie_admin = "rusty_discord_bot_session=sess_admin".to_string();
    let cookie_err = "rusty_discord_bot_session=sess_err".to_string();

    // --- TEST SCENARIO 1: member_not_in_thread has VIEW_CHANNEL from parent, but is NOT a thread member ---
    // 1a. GET giveaway in private thread -> 403 Forbidden
    let get_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}",
            ga_private.id
        ))
        .header(COOKIE, &cookie_not_in_thread)
        .send()
        .await
        .unwrap();
    assert_eq!(get_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 1b. POST /join giveaway in private thread -> 403 Forbidden
    let join_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}/join",
            ga_private.id
        ))
        .header(COOKIE, &cookie_not_in_thread)
        .header("x-csrf-token", "csrf_not_in_thread")
        .send()
        .await
        .unwrap();
    assert_eq!(join_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Verify 0 participants added in storage
    let ga_check = web_state
        .app
        .storage
        .get_giveaway(ga_private.id)
        .await
        .unwrap()
        .unwrap();
    let participants = ga_check
        .participants_json
        .as_array()
        .expect("participants array");
    assert!(
        participants.is_empty(),
        "Participants must remain empty after forbidden join"
    );

    // 1c. POST /giveaways creating in private thread -> 403 Forbidden (not manager & not member)
    let create_forbidden_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/giveaways"))
        .header(COOKIE, &cookie_not_in_thread)
        .header("x-csrf-token", "csrf_not_in_thread")
        .json(&serde_json::json!({
            "channel_id": private_thread_id,
            "prize": "Hacked Prize",
            "winners_count": 1,
            "duration_minutes": 60
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        create_forbidden_resp.status(),
        reqwest::StatusCode::FORBIDDEN
    );

    // 1d. Accessing public thread (type 11) and text channel (type 0) works with VIEW_CHANNEL alone
    let get_pub_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}",
            ga_public.id
        ))
        .header(COOKIE, &cookie_not_in_thread)
        .send()
        .await
        .unwrap();
    assert_eq!(get_pub_resp.status(), reqwest::StatusCode::OK);

    let get_text_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}",
            ga_text.id
        ))
        .header(COOKIE, &cookie_not_in_thread)
        .send()
        .await
        .unwrap();
    assert_eq!(get_text_resp.status(), reqwest::StatusCode::OK);

    // --- TEST SCENARIO 2: member_in_thread is a verified thread member ---
    // 2a. GET giveaway in private thread -> 200 OK
    let get_ok_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}",
            ga_private.id
        ))
        .header(COOKIE, &cookie_in_thread)
        .send()
        .await
        .unwrap();
    assert_eq!(get_ok_resp.status(), reqwest::StatusCode::OK);

    // 2b. POST /join giveaway in private thread -> 200 OK (joined: true)
    let join_ok_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}/join",
            ga_private.id
        ))
        .header(COOKIE, &cookie_in_thread)
        .header("x-csrf-token", "csrf_in_thread")
        .send()
        .await
        .unwrap();
    assert_eq!(join_ok_resp.status(), reqwest::StatusCode::OK);
    let join_body: serde_json::Value = join_ok_resp.json().await.unwrap();
    assert_eq!(join_body["joined"], true);

    // Verify member_in_thread is registered in storage
    let ga_check2 = web_state
        .app
        .storage
        .get_giveaway(ga_private.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        ga_check2.participants_json,
        serde_json::json!([member_in_thread_id])
    );

    // --- TEST SCENARIO 3: thread_moderator has MANAGE_THREADS permission (bypasses thread-member requirement) ---
    // 3a. GET giveaway in private thread -> 200 OK
    let mod_get_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}",
            ga_private.id
        ))
        .header(COOKIE, &cookie_mod)
        .send()
        .await
        .unwrap();
    assert_eq!(mod_get_resp.status(), reqwest::StatusCode::OK);

    // 3b. POST /join giveaway in private thread -> 200 OK
    let mod_join_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}/join",
            ga_private.id
        ))
        .header(COOKIE, &cookie_mod)
        .header("x-csrf-token", "csrf_mod")
        .send()
        .await
        .unwrap();
    assert_eq!(mod_join_resp.status(), reqwest::StatusCode::OK);

    // --- TEST SCENARIO 4: admin has ADMINISTRATOR permission and can manage guild ---
    // 4a. GET giveaway in private thread -> 200 OK
    let admin_get_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}",
            ga_private.id
        ))
        .header(COOKIE, &cookie_admin)
        .send()
        .await
        .unwrap();
    assert_eq!(admin_get_resp.status(), reqwest::StatusCode::OK);

    // 4b. Admin creates a giveaway in private thread -> 200 OK
    let admin_create_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/giveaways"))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", "csrf_admin")
        .json(&serde_json::json!({
            "channel_id": private_thread_id,
            "prize": "Admin Created Thread Prize",
            "winners_count": 1,
            "duration_minutes": 60
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(admin_create_resp.status(), reqwest::StatusCode::OK);

    // --- TEST SCENARIO 5: Fail-closed on Discord API error (500 from thread-members) ---
    let err_get_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}",
            ga_private.id
        ))
        .header(COOKIE, &cookie_err)
        .send()
        .await
        .unwrap();
    assert_eq!(err_get_resp.status(), reqwest::StatusCode::FORBIDDEN);

    let err_join_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/giveaways/{}/join",
            ga_private.id
        ))
        .header(COOKIE, &cookie_err)
        .header("x-csrf-token", "csrf_err")
        .send()
        .await
        .unwrap();
    assert_eq!(err_join_resp.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_module_autorole_privilege_escalation_guard_suite() {
    use axum::Json;
    use axum::extract::Path;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use reqwest::header::COOKIE;

    let guild_a = "777888999000111";

    // 1. Mock Discord API for module autorole verification
    let discord_mock_app = axum::Router::new()
        .route(
            "/users/@me",
            get(|| async move {
                Json(serde_json::json!({
                    "id": "999999999999999999"
                }))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}",
            get(|Path(gid): Path<String>| async move {
                Json(serde_json::json!({
                    "id": gid,
                    "owner_id": "999000000000000000"
                }))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}/roles",
            get(|| async move {
                Json(serde_json::json!([
                    {
                        "id": "1001",
                        "name": "QuarantineRole",
                        "permissions": "0",
                        "position": 5,
                        "managed": false
                    },
                    {
                        "id": "1002",
                        "name": "MemberRoleA",
                        "permissions": "0",
                        "position": 10,
                        "managed": false
                    },
                    {
                        "id": "1003",
                        "name": "ModRole",
                        "permissions": "268435456", // MANAGE_ROLES
                        "position": 40,
                        "managed": false
                    },
                    {
                        "id": "1004",
                        "name": "BotRole",
                        "permissions": "268435456",
                        "position": 60,
                        "managed": false
                    },
                    {
                        "id": "1005",
                        "name": "AdminRole",
                        "permissions": "8", // ADMINISTRATOR
                        "position": 30,
                        "managed": false
                    },
                    {
                        "id": "1006",
                        "name": "TooHighRole",
                        "permissions": "0",
                        "position": 50, // higher than actor's position 40
                        "managed": false
                    },
                    {
                        "id": "1007",
                        "name": "ManagedIntegrationRole",
                        "permissions": "0",
                        "position": 15,
                        "managed": true
                    },
                    {
                        "id": "1008",
                        "name": "BanRole",
                        "permissions": "4", // BAN_MEMBERS
                        "position": 20,
                        "managed": false
                    }
                ]))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}/members/{user_id}",
            get(|Path((_gid, user_id)): Path<(String, String)>| async move {
                if user_id == "999999999999999999" {
                    // Bot member: position 60
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": ["1004"],
                        "permissions": "268435456"
                    }))
                    .into_response()
                } else if user_id == "111111111111111111" {
                    // Admin A: position 40, MANAGE_GUILD | MANAGE_ROLES
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": ["1003"],
                        "permissions": "268435488"
                    }))
                    .into_response()
                } else if user_id == "333333333333333333" {
                    // Manager without MANAGE_ROLES: MANAGE_GUILD only (32)
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": [],
                        "permissions": "32"
                    }))
                    .into_response()
                } else {
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": [],
                        "permissions": "0"
                    }))
                    .into_response()
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

    let mut config = test_config();
    config.discord_token = "live_bot_token_autorole_test_secret".to_string();
    config.offline_mode = false;
    config.discord_api_base_url = format!("http://{discord_addr}");

    let app = AppState::bootstrap(config).await.expect("bootstrap");
    let permissions_cache = std::sync::Arc::new(dashmap::DashMap::new());
    let web_state = WebState::new(
        app,
        std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    );

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

    let admin_session = "admin_autorole_sess";
    let admin_csrf = "admin_autorole_csrf";
    let admin_id = "111111111111111111";

    web_state
        .app
        .sessions
        .save_session(
            admin_session,
            DashboardUser {
                id: admin_id.to_owned(),
                username: "admin_a".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_a.to_owned(),
                name: "Guild A".to_owned(),
                icon: None,
                owner: false,
                permissions: "268435488".to_owned(), // MANAGE_GUILD | MANAGE_ROLES
            }],
            admin_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    web_state.permissions_cache.insert(
        (admin_id.to_owned(), guild_a.to_owned()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let no_roles_session = "no_roles_autorole_sess";
    let no_roles_csrf = "no_roles_autorole_csrf";
    let no_roles_id = "333333333333333333";

    web_state
        .app
        .sessions
        .save_session(
            no_roles_session,
            DashboardUser {
                id: no_roles_id.to_owned(),
                username: "manager_without_roles".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_a.to_owned(),
                name: "Guild A".to_owned(),
                icon: None,
                owner: false,
                permissions: "32".to_owned(), // MANAGE_GUILD only
            }],
            no_roles_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    web_state.permissions_cache.insert(
        (no_roles_id.to_owned(), guild_a.to_owned()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let cookie_admin = format!("rusty_discord_bot_session={admin_session}");
    let cookie_no_roles = format!("rusty_discord_bot_session={no_roles_session}");

    // Test 1: User with MANAGE_GUILD but without MANAGE_ROLES attempts to configure auto_roles in welcome -> 403 Forbidden
    let no_role_welcome_resp = client
        .put(format!("{base_url}/api/guilds/{guild_a}/modules/welcome"))
        .header(COOKIE, &cookie_no_roles)
        .header("x-csrf-token", no_roles_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "config": {
                "channel_id": null,
                "message": "Welcome {user}!",
                "dm_welcome": false,
                "auto_roles": ["1002"]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        no_role_welcome_resp.status(),
        reqwest::StatusCode::FORBIDDEN
    );

    // Test 2: User with MANAGE_GUILD but without MANAGE_ROLES attempts to configure quarantine_role_id in join_guard -> 403 Forbidden
    let no_role_jg_resp = client
        .put(format!(
            "{base_url}/api/guilds/{guild_a}/modules/join_guard"
        ))
        .header(COOKIE, &cookie_no_roles)
        .header("x-csrf-token", no_roles_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "config": {
                "alert_channel_id": null,
                "join_log_channel_id": null,
                "minimum_account_age_hours": 24,
                "action_on_raid": "alert",
                "quarantine_role_id": "1001"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(no_role_jg_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 3: User with MANAGE_GUILD without MANAGE_ROLES can safely configure welcome module with empty auto_roles -> 200 OK
    let safe_welcome_resp = client
        .put(format!("{base_url}/api/guilds/{guild_a}/modules/welcome"))
        .header(COOKIE, &cookie_no_roles)
        .header("x-csrf-token", no_roles_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "config": {
                "channel_id": null,
                "message": "Welcome {user} safe!",
                "dm_welcome": false,
                "auto_roles": []
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(safe_welcome_resp.status(), reqwest::StatusCode::OK);

    // Test 4: User with MANAGE_GUILD without MANAGE_ROLES can safely configure join_guard without quarantine_role_id -> 200 OK
    let safe_jg_resp = client
        .put(format!(
            "{base_url}/api/guilds/{guild_a}/modules/join_guard"
        ))
        .header(COOKIE, &cookie_no_roles)
        .header("x-csrf-token", no_roles_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "config": {
                "alert_channel_id": null,
                "join_log_channel_id": null,
                "minimum_account_age_hours": 24,
                "action_on_raid": "alert"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(safe_jg_resp.status(), reqwest::StatusCode::OK);

    // Test 5: Admin attempts to configure auto_roles with privileged ADMINISTRATOR role (1005) -> 403 Forbidden
    let admin_role_resp = client
        .put(format!("{base_url}/api/guilds/{guild_a}/modules/welcome"))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "config": {
                "channel_id": null,
                "message": "Welcome {user}!",
                "dm_welcome": false,
                "auto_roles": ["1005"]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(admin_role_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 6: Admin attempts to configure join_guard quarantine_role_id with MANAGE_ROLES role (1003) -> 403 Forbidden
    let jg_mod_role_resp = client
        .put(format!(
            "{base_url}/api/guilds/{guild_a}/modules/join_guard"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "config": {
                "alert_channel_id": null,
                "join_log_channel_id": null,
                "minimum_account_age_hours": 24,
                "action_on_raid": "alert",
                "quarantine_role_id": "1003"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(jg_mod_role_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 7: Admin attempts to configure auto_roles with BAN_MEMBERS role (1008) -> 403 Forbidden
    let ban_role_resp = client
        .put(format!("{base_url}/api/guilds/{guild_a}/modules/welcome"))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "config": {
                "channel_id": null,
                "message": "Welcome {user}!",
                "dm_welcome": false,
                "auto_roles": ["1008"]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(ban_role_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 8: Admin attempts to configure auto_roles with managed integration role (1007) -> 400 Bad Request
    let managed_role_resp = client
        .put(format!("{base_url}/api/guilds/{guild_a}/modules/welcome"))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "config": {
                "channel_id": null,
                "message": "Welcome {user}!",
                "dm_welcome": false,
                "auto_roles": ["1007"]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(managed_role_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Test 9: Admin attempts to configure auto_roles with role higher than actor (1006) -> 403 Forbidden
    let too_high_role_resp = client
        .put(format!("{base_url}/api/guilds/{guild_a}/modules/welcome"))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "config": {
                "channel_id": null,
                "message": "Welcome {user}!",
                "dm_welcome": false,
                "auto_roles": ["1006"]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(too_high_role_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 10: Admin attempts to configure auto_roles with non-existent role (9999) -> 400 Bad Request
    let non_existent_resp = client
        .put(format!("{base_url}/api/guilds/{guild_a}/modules/welcome"))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "config": {
                "channel_id": null,
                "message": "Welcome {user}!",
                "dm_welcome": false,
                "auto_roles": ["9999"]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(non_existent_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Test 11: Admin configures legitimate unprivileged role (1002) in auto_roles -> 200 OK
    let legit_welcome_resp = client
        .put(format!("{base_url}/api/guilds/{guild_a}/modules/welcome"))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "config": {
                "channel_id": null,
                "message": "Welcome {user}!",
                "dm_welcome": false,
                "auto_roles": ["1002"]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(legit_welcome_resp.status(), reqwest::StatusCode::OK);

    // Test 12: Admin configures legitimate unprivileged role (1001) in quarantine_role_id -> 200 OK
    let legit_jg_resp = client
        .put(format!(
            "{base_url}/api/guilds/{guild_a}/modules/join_guard"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "config": {
                "alert_channel_id": null,
                "join_log_channel_id": null,
                "minimum_account_age_hours": 24,
                "action_on_raid": "alert",
                "quarantine_role_id": "1001"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(legit_jg_resp.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn test_progression_role_rewards_privilege_escalation_guard_suite() {
    use axum::Json;
    use axum::extract::Path;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use reqwest::header::COOKIE;

    let guild_a = "777888999000111";

    let discord_mock_app = axum::Router::new()
        .route(
            "/users/@me",
            get(|| async move {
                Json(serde_json::json!({
                    "id": "999999999999999999"
                }))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}",
            get(|Path(gid): Path<String>| async move {
                Json(serde_json::json!({
                    "id": gid,
                    "owner_id": "999000000000000000"
                }))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}/roles",
            get(|| async move {
                Json(serde_json::json!([
                    {
                        "id": "1001",
                        "name": "MemberRoleA",
                        "permissions": "0",
                        "position": 10,
                        "managed": false
                    },
                    {
                        "id": "1002",
                        "name": "MemberRoleB",
                        "permissions": "0",
                        "position": 15,
                        "managed": false
                    },
                    {
                        "id": "1003",
                        "name": "ModRole",
                        "permissions": "268435456", // MANAGE_ROLES
                        "position": 40,
                        "managed": false
                    },
                    {
                        "id": "1004",
                        "name": "BotRole",
                        "permissions": "268435456",
                        "position": 60,
                        "managed": false
                    },
                    {
                        "id": "1005",
                        "name": "AdminRole",
                        "permissions": "8", // ADMINISTRATOR
                        "position": 30,
                        "managed": false
                    },
                    {
                        "id": "1006",
                        "name": "TooHighRole",
                        "permissions": "0",
                        "position": 50,
                        "managed": false
                    },
                    {
                        "id": "1007",
                        "name": "ManagedIntegrationRole",
                        "permissions": "0",
                        "position": 15,
                        "managed": true
                    }
                ]))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}/members/{user_id}",
            get(|Path((_gid, user_id)): Path<(String, String)>| async move {
                if user_id == "999999999999999999" {
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": ["1004"],
                        "permissions": "268435456"
                    }))
                    .into_response()
                } else if user_id == "111111111111111111" {
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": ["1003"],
                        "permissions": "268435488"
                    }))
                    .into_response()
                } else if user_id == "333333333333333333" {
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": [],
                        "permissions": "32"
                    }))
                    .into_response()
                } else {
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": [],
                        "permissions": "0"
                    }))
                    .into_response()
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

    let mut config = test_config();
    config.discord_token = "live_bot_token_autorole_test_secret".to_string();
    config.offline_mode = false;
    config.discord_api_base_url = format!("http://{discord_addr}");

    let app = AppState::bootstrap(config).await.expect("bootstrap");
    let permissions_cache = std::sync::Arc::new(dashmap::DashMap::new());
    let web_state = WebState::new(
        app,
        std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    );

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

    let admin_session = "admin_autorole_sess";
    let admin_csrf = "admin_autorole_csrf";
    let admin_id = "111111111111111111";

    web_state
        .app
        .sessions
        .save_session(
            admin_session,
            DashboardUser {
                id: admin_id.to_owned(),
                username: "admin_a".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_a.to_owned(),
                name: "Guild A".to_owned(),
                icon: None,
                owner: false,
                permissions: "268435488".to_owned(),
            }],
            admin_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    web_state.permissions_cache.insert(
        (admin_id.to_owned(), guild_a.to_owned()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let no_roles_session = "no_roles_autorole_sess";
    let no_roles_csrf = "no_roles_autorole_csrf";
    let no_roles_id = "333333333333333333";

    web_state
        .app
        .sessions
        .save_session(
            no_roles_session,
            DashboardUser {
                id: no_roles_id.to_owned(),
                username: "manager_without_roles".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_a.to_owned(),
                name: "Guild A".to_owned(),
                icon: None,
                owner: false,
                permissions: "32".to_owned(),
            }],
            no_roles_csrf.to_owned(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    web_state.permissions_cache.insert(
        (no_roles_id.to_owned(), guild_a.to_owned()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let cookie_admin = format!("rusty_discord_bot_session={admin_session}");
    let cookie_no_roles = format!("rusty_discord_bot_session={no_roles_session}");

    // Test 1: User with MANAGE_GUILD but without MANAGE_ROLES attempts to configure progression role_rewards -> 403 Forbidden
    let no_role_prog_resp = client
        .put(format!(
            "{base_url}/api/guilds/{guild_a}/modules/progression"
        ))
        .header(COOKIE, &cookie_no_roles)
        .header("x-csrf-token", no_roles_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "config": {
                "role_rewards": [
                    { "level": 1, "role_id": "1002" }
                ]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(no_role_prog_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 2: User without MANAGE_ROLES can safely configure progression with empty role_rewards -> 200 OK
    let safe_prog_resp = client
        .put(format!(
            "{base_url}/api/guilds/{guild_a}/modules/progression"
        ))
        .header(COOKIE, &cookie_no_roles)
        .header("x-csrf-token", no_roles_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "config": {
                "role_rewards": []
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(safe_prog_resp.status(), reqwest::StatusCode::OK);

    // Test 3: Admin attempts to configure progression role_rewards with ADMINISTRATOR role (1005) -> 403 Forbidden
    let admin_prog_admin_role = client
        .put(format!(
            "{base_url}/api/guilds/{guild_a}/modules/progression"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "config": {
                "role_rewards": [
                    { "level": 5, "role_id": "1005" }
                ]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        admin_prog_admin_role.status(),
        reqwest::StatusCode::FORBIDDEN
    );

    // Test 4: Admin attempts to configure progression role_rewards with MANAGE_ROLES role (1003) -> 403 Forbidden
    let admin_prog_mod_role = client
        .put(format!(
            "{base_url}/api/guilds/{guild_a}/modules/progression"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "config": {
                "role_rewards": [
                    { "level": 5, "role_id": "1003" }
                ]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(admin_prog_mod_role.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 5: Admin attempts to configure progression role_rewards with managed integration role (1007) -> 400 Bad Request
    let admin_prog_managed_role = client
        .put(format!(
            "{base_url}/api/guilds/{guild_a}/modules/progression"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "config": {
                "role_rewards": [
                    { "level": 5, "role_id": "1007" }
                ]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        admin_prog_managed_role.status(),
        reqwest::StatusCode::BAD_REQUEST
    );

    // Test 6: Admin attempts to configure progression role_rewards with role higher than actor (1006) -> 403 Forbidden
    let admin_prog_too_high = client
        .put(format!(
            "{base_url}/api/guilds/{guild_a}/modules/progression"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "config": {
                "role_rewards": [
                    { "level": 5, "role_id": "1006" }
                ]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(admin_prog_too_high.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 7: Admin configures legitimate unprivileged role (1002) in progression role_rewards -> 200 OK
    let admin_prog_legit = client
        .put(format!(
            "{base_url}/api/guilds/{guild_a}/modules/progression"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "config": {
                "role_rewards": [
                    { "level": 2, "role_id": "1002" }
                ]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(admin_prog_legit.status(), reqwest::StatusCode::OK);

    // Test 8: Draft publish privilege escalation guard
    // 8a. Save draft with privileged ADMINISTRATOR role (1005)
    let save_draft_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/modules/progression/draft"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "config": {
                "role_rewards": [
                    { "level": 10, "role_id": "1005" }
                ]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(save_draft_resp.status(), reqwest::StatusCode::OK);

    // 8b. User without MANAGE_ROLES attempting to publish draft with role_rewards -> 403 Forbidden
    let publish_no_role_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/modules/progression/publish"
        ))
        .header(COOKIE, &cookie_no_roles)
        .header("x-csrf-token", no_roles_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(
        publish_no_role_resp.status(),
        reqwest::StatusCode::FORBIDDEN
    );

    // 8c. Admin attempting to publish draft containing privileged ADMINISTRATOR role -> 403 Forbidden
    let publish_admin_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/modules/progression/publish"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(publish_admin_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 8d. Save draft with legitimate unprivileged role (1002) and publish -> 200 OK
    let save_legit_draft = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/modules/progression/draft"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "config": {
                "role_rewards": [
                    { "level": 10, "role_id": "1002" }
                ]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(save_legit_draft.status(), reqwest::StatusCode::OK);

    let publish_legit_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/modules/progression/publish"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(publish_legit_resp.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn test_recruitment_privilege_escalation_and_self_accept_guard_suite() {
    use axum::Json;
    use axum::extract::Path;
    use axum::response::IntoResponse;
    use axum::routing::{get, put};
    use reqwest::header::COOKIE;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let guild_a = "777888999000111";
    let role_delivery_count = Arc::new(AtomicUsize::new(0));
    let role_delivery_count_clone = role_delivery_count.clone();

    // 1. Mock Discord API
    let discord_mock_app = axum::Router::new()
        .route(
            "/users/@me",
            get(|| async move {
                Json(serde_json::json!({
                    "id": "999999999999999999"
                }))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}",
            get(|Path(gid): Path<String>| async move {
                Json(serde_json::json!({
                    "id": gid,
                    "owner_id": "999000000000000000",
                    "roles": [
                        {
                            "id": "1001",
                            "name": "MemberRoleA",
                            "permissions": "0",
                            "position": 10,
                            "managed": false
                        },
                        {
                            "id": "1003",
                            "name": "ModRole",
                            "permissions": "268435456", // MANAGE_ROLES
                            "position": 40,
                            "managed": false
                        },
                        {
                            "id": "1004",
                            "name": "BotRole",
                            "permissions": "268435456",
                            "position": 60,
                            "managed": false
                        },
                        {
                            "id": "1005",
                            "name": "AdminRole",
                            "permissions": "8", // ADMINISTRATOR
                            "position": 30,
                            "managed": false
                        },
                        {
                            "id": "1006",
                            "name": "TooHighRole",
                            "permissions": "0",
                            "position": 50,
                            "managed": false
                        },
                        {
                            "id": "1007",
                            "name": "ManagedIntegrationRole",
                            "permissions": "0",
                            "position": 15,
                            "managed": true
                        },
                        {
                            "id": "1008",
                            "name": "BanRole",
                            "permissions": "4", // BAN_MEMBERS
                            "position": 20,
                            "managed": false
                        }
                    ]
                }))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}/roles",
            get(|| async move {
                Json(serde_json::json!([
                    {
                        "id": "1001",
                        "name": "MemberRoleA",
                        "permissions": "0",
                        "position": 10,
                        "managed": false,
                        "color": 0,
                        "colors": { "primary_color": 0 },
                        "hoist": false,
                        "mentionable": false,
                        "flags": 0
                    },
                    {
                        "id": "1003",
                        "name": "ModRole",
                        "permissions": "268435456",
                        "position": 40,
                        "managed": false,
                        "color": 0,
                        "colors": { "primary_color": 0 },
                        "hoist": false,
                        "mentionable": false,
                        "flags": 0
                    },
                    {
                        "id": "1004",
                        "name": "BotRole",
                        "permissions": "268435456",
                        "position": 60,
                        "managed": false,
                        "color": 0,
                        "colors": { "primary_color": 0 },
                        "hoist": false,
                        "mentionable": false,
                        "flags": 0
                    },
                    {
                        "id": "1005",
                        "name": "AdminRole",
                        "permissions": "8",
                        "position": 30,
                        "managed": false,
                        "color": 0,
                        "colors": { "primary_color": 0 },
                        "hoist": false,
                        "mentionable": false,
                        "flags": 0
                    },
                    {
                        "id": "1006",
                        "name": "TooHighRole",
                        "permissions": "0",
                        "position": 50,
                        "managed": false,
                        "color": 0,
                        "colors": { "primary_color": 0 },
                        "hoist": false,
                        "mentionable": false,
                        "flags": 0
                    },
                    {
                        "id": "1007",
                        "name": "ManagedIntegrationRole",
                        "permissions": "0",
                        "position": 15,
                        "managed": true,
                        "color": 0,
                        "colors": { "primary_color": 0 },
                        "hoist": false,
                        "mentionable": false,
                        "flags": 0
                    },
                    {
                        "id": "1008",
                        "name": "BanRole",
                        "permissions": "4",
                        "position": 20,
                        "managed": false,
                        "color": 0,
                        "colors": { "primary_color": 0 },
                        "hoist": false,
                        "mentionable": false,
                        "flags": 0
                    }
                ]))
                .into_response()
            }),
        )
        .route(
            "/api/v10/guilds/{guild_id}/roles",
            get(|| async move {
                Json(serde_json::json!([
                    {
                        "id": "1001",
                        "name": "MemberRoleA",
                        "permissions": "0",
                        "position": 10,
                        "managed": false,
                        "color": 0,
                        "colors": { "primary_color": 0 },
                        "hoist": false,
                        "mentionable": false,
                        "flags": 0
                    },
                    {
                        "id": "1003",
                        "name": "ModRole",
                        "permissions": "268435456",
                        "position": 40,
                        "managed": false,
                        "color": 0,
                        "colors": { "primary_color": 0 },
                        "hoist": false,
                        "mentionable": false,
                        "flags": 0
                    },
                    {
                        "id": "1004",
                        "name": "BotRole",
                        "permissions": "268435456",
                        "position": 60,
                        "managed": false,
                        "color": 0,
                        "colors": { "primary_color": 0 },
                        "hoist": false,
                        "mentionable": false,
                        "flags": 0
                    },
                    {
                        "id": "1005",
                        "name": "AdminRole",
                        "permissions": "8",
                        "position": 30,
                        "managed": false,
                        "color": 0,
                        "colors": { "primary_color": 0 },
                        "hoist": false,
                        "mentionable": false,
                        "flags": 0
                    },
                    {
                        "id": "1006",
                        "name": "TooHighRole",
                        "permissions": "0",
                        "position": 50,
                        "managed": false,
                        "color": 0,
                        "colors": { "primary_color": 0 },
                        "hoist": false,
                        "mentionable": false,
                        "flags": 0
                    },
                    {
                        "id": "1007",
                        "name": "ManagedIntegrationRole",
                        "permissions": "0",
                        "position": 15,
                        "managed": true,
                        "color": 0,
                        "colors": { "primary_color": 0 },
                        "hoist": false,
                        "mentionable": false,
                        "flags": 0
                    },
                    {
                        "id": "1008",
                        "name": "BanRole",
                        "permissions": "4",
                        "position": 20,
                        "managed": false,
                        "color": 0,
                        "colors": { "primary_color": 0 },
                        "hoist": false,
                        "mentionable": false,
                        "flags": 0
                    }
                ]))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}/members/{user_id}",
            get(|Path((_gid, user_id)): Path<(String, String)>| async move {
                if user_id == "999999999999999999" {
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": ["1004"],
                        "permissions": "268435456"
                    }))
                    .into_response()
                } else if user_id == "111111111111111111" || user_id == "555555555555555555" {
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": ["1003"],
                        "permissions": "268435488"
                    }))
                    .into_response()
                } else if user_id == "333333333333333333" {
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": [],
                        "permissions": "32"
                    }))
                    .into_response()
                } else {
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": [],
                        "permissions": "0"
                    }))
                    .into_response()
                }
            }),
        )
        .route(
            "/guilds/{guild_id}/members/{user_id}/roles/{role_id}",
            put({
                let count = role_delivery_count_clone.clone();
                move || {
                    let count = count.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        reqwest::StatusCode::NO_CONTENT.into_response()
                    }
                }
            }),
        )
        .route(
            "/api/v10/guilds/{guild_id}/members/{user_id}/roles/{role_id}",
            put({
                let count = role_delivery_count_clone.clone();
                move || {
                    let count = count.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        reqwest::StatusCode::NO_CONTENT.into_response()
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

    let mut config = test_config();
    config.discord_token = "live_bot_token_recruitment_sec_secret".to_string();
    config.offline_mode = false;
    config.discord_api_base_url = format!("http://{discord_addr}");

    let app = AppState::bootstrap(config).await.expect("bootstrap");
    let permissions_cache = std::sync::Arc::new(dashmap::DashMap::new());
    let web_state = WebState::new(
        app,
        std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    );

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

    let admin_session = "admin_rec_sec_sess";
    let admin_csrf = "admin_rec_sec_csrf";
    let admin_id = "111111111111111111";

    let reviewer2_session = "reviewer2_rec_sec_sess";
    let reviewer2_csrf = "reviewer2_rec_sec_csrf";
    let reviewer2_id = "555555555555555555";

    let no_roles_session = "no_roles_rec_sec_sess";
    let no_roles_csrf = "no_roles_rec_sec_csrf";
    let no_roles_id = "333333333333333333";

    let candidate_session = "candidate_rec_sec_sess";
    let candidate_csrf = "candidate_rec_sec_csrf";
    let candidate_id = "444444444444444444";

    let users = [
        (
            admin_id,
            "admin",
            admin_session,
            admin_csrf,
            "268435488",
            true,
        ),
        (
            reviewer2_id,
            "reviewer2",
            reviewer2_session,
            reviewer2_csrf,
            "268435488",
            true,
        ),
        (
            no_roles_id,
            "manager_no_roles",
            no_roles_session,
            no_roles_csrf,
            "32",
            true,
        ),
        (
            candidate_id,
            "candidate",
            candidate_session,
            candidate_csrf,
            "0",
            false,
        ),
    ];

    for (uid, uname, sid, csrf, perms, can_manage) in users {
        web_state
            .app
            .sessions
            .save_session(
                sid,
                DashboardUser {
                    id: uid.to_owned(),
                    username: uname.to_owned(),
                    global_name: None,
                    avatar: None,
                },
                vec![DashboardGuild {
                    id: guild_a.to_owned(),
                    name: "Guild A".to_owned(),
                    icon: None,
                    owner: false,
                    permissions: perms.to_owned(),
                }],
                csrf.to_owned(),
                Utc::now() + Duration::hours(1),
                None,
            )
            .await
            .unwrap();

        if can_manage {
            web_state.permissions_cache.insert(
                (uid.to_owned(), guild_a.to_owned()),
                CachedGuildPermission {
                    can_manage: true,
                    checked_at: Utc::now(),
                },
            );
        }
    }

    let cookie_admin = format!("rusty_discord_bot_session={admin_session}");
    let cookie_rev2 = format!("rusty_discord_bot_session={reviewer2_session}");
    let cookie_no_roles = format!("rusty_discord_bot_session={no_roles_session}");
    let cookie_candidate = format!("rusty_discord_bot_session={candidate_session}");

    // Test 1: User with MANAGE_GUILD without MANAGE_ROLES attempts to create track with target_role_id -> 403 Forbidden
    let no_role_track_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/tracks"
        ))
        .header(COOKIE, &cookie_no_roles)
        .header("x-csrf-token", no_roles_csrf)
        .json(&serde_json::json!({
            "slug": "staff-mod",
            "name": "Staff Moderator",
            "enabled": true,
            "target_role_id": "1001",
            "config": {
                "auto_role_on_accept": true,
                "final_approvals": 1,
                "questions": []
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(no_role_track_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 2: User with MANAGE_GUILD without MANAGE_ROLES can create track without target_role_id -> 200 OK
    let safe_track_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/tracks"
        ))
        .header(COOKIE, &cookie_no_roles)
        .header("x-csrf-token", no_roles_csrf)
        .json(&serde_json::json!({
            "slug": "safe-track",
            "name": "Safe Track No Roles",
            "enabled": true,
            "config": {
                "auto_role_on_accept": false,
                "final_approvals": 1,
                "questions": []
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(safe_track_resp.status(), reqwest::StatusCode::OK);

    // Test 3: Admin attempts to create track with auto_role_on_accept=true and final_approvals=0 -> 400 Bad Request
    let zero_approvals_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/tracks"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "slug": "exploit-zero-approvals",
            "name": "Zero Approvals Exploit",
            "enabled": true,
            "target_role_id": "1001",
            "config": {
                "auto_role_on_accept": true,
                "final_approvals": 0,
                "questions": []
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        zero_approvals_resp.status(),
        reqwest::StatusCode::BAD_REQUEST
    );

    // Test 4: Admin attempts to configure privileged or invalid target roles:
    // 4a. ADMINISTRATOR role (1005) -> 403 Forbidden
    let admin_priv_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/tracks"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "slug": "admin-role-track",
            "name": "Admin Track",
            "enabled": true,
            "target_role_id": "1005",
            "config": {
                "auto_role_on_accept": true,
                "final_approvals": 1,
                "questions": []
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(admin_priv_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 4b. BAN_MEMBERS role (1008) -> 403 Forbidden
    let ban_priv_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/tracks"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "slug": "ban-role-track",
            "name": "Ban Track",
            "enabled": true,
            "target_role_id": "1008",
            "config": {
                "auto_role_on_accept": true,
                "final_approvals": 1,
                "questions": []
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(ban_priv_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 4c. Managed integration role (1007) -> 400 Bad Request
    let managed_role_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/tracks"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "slug": "managed-role-track",
            "name": "Managed Track",
            "enabled": true,
            "target_role_id": "1007",
            "config": {
                "auto_role_on_accept": true,
                "final_approvals": 1,
                "questions": []
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(managed_role_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // 4d. Role higher than actor (1006) -> 403 Forbidden
    let too_high_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/tracks"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "slug": "too-high-role-track",
            "name": "Too High Track",
            "enabled": true,
            "target_role_id": "1006",
            "config": {
                "auto_role_on_accept": true,
                "final_approvals": 1,
                "questions": []
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(too_high_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 4e. Non-existent role (9999) -> 400 Bad Request
    let non_existent_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/tracks"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "slug": "missing-role-track",
            "name": "Missing Role Track",
            "enabled": true,
            "target_role_id": "9999",
            "config": {
                "auto_role_on_accept": true,
                "final_approvals": 1,
                "questions": []
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(non_existent_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Test 5: Admin creates legitimate track with target_role_id="1001", auto_role_on_accept=true, final_approvals=1 -> 200 OK
    let legit_track_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/tracks"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "slug": "staff-mod",
            "name": "Staff Moderator Track",
            "enabled": true,
            "target_role_id": "1001",
            "config": {
                "auto_role_on_accept": true,
                "final_approvals": 1,
                "questions": []
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(legit_track_resp.status(), reqwest::StatusCode::OK);

    // Test 6: Self-acceptance prevention
    // 6a. Admin submits application to staff-mod
    let admin_apply_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/tracks/staff-mod/apply"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({ "answers": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(admin_apply_resp.status(), reqwest::StatusCode::OK);
    let admin_app_body: serde_json::Value = admin_apply_resp.json().await.unwrap();
    let admin_app_id = admin_app_body["application"]["id"].as_str().unwrap();

    // 6b. Reviewer 2 approves Admin's application
    let rev2_review_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/applications/{admin_app_id}/review"
        ))
        .header(COOKIE, &cookie_rev2)
        .header("x-csrf-token", reviewer2_csrf)
        .json(&serde_json::json!({
            "recommendation": "approve",
            "score": 5,
            "notes": "Approved by peer"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(rev2_review_resp.status(), reqwest::StatusCode::OK);

    // 6c. Admin attempts to transition their OWN application to "accepted" -> 403 Forbidden!
    let self_accept_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/applications/{admin_app_id}/transition"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "expected_version": 1,
            "new_status": "accepted",
            "new_stage": "completed"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(self_accept_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 7: Legitimate candidate application and fulfillment
    // 7a. Candidate submits application
    let cand_apply_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/tracks/staff-mod/apply"
        ))
        .header(COOKIE, &cookie_candidate)
        .header("x-csrf-token", candidate_csrf)
        .json(&serde_json::json!({ "answers": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(cand_apply_resp.status(), reqwest::StatusCode::OK);
    let cand_app_body: serde_json::Value = cand_apply_resp.json().await.unwrap();
    let cand_app_id = cand_app_body["application"]["id"].as_str().unwrap();

    // 7b. Admin reviews Candidate's application
    let admin_review_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/applications/{cand_app_id}/review"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "recommendation": "approve",
            "score": 5,
            "notes": "Great candidate"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(admin_review_resp.status(), reqwest::StatusCode::OK);

    // 7c. Reviewer 2 transitions Candidate's application to accepted -> 200 OK
    let accept_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/applications/{cand_app_id}/transition"
        ))
        .header(COOKIE, &cookie_rev2)
        .header("x-csrf-token", reviewer2_csrf)
        .json(&serde_json::json!({
            "expected_version": 1,
            "new_status": "accepted",
            "new_stage": "completed"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(accept_resp.status(), reqwest::StatusCode::OK);

    // 7d. Process role fulfillment queue -> 1 processed, role dispatched via PUT
    let baseline_delivery = role_delivery_count.load(Ordering::SeqCst);
    let proc_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/fulfillment/process"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(proc_resp.status(), reqwest::StatusCode::OK);

    let _proc_resp_body: serde_json::Value = proc_resp.json().await.unwrap();

    let parsed_cand_id: uuid::Uuid = cand_app_id.parse().unwrap();
    let fulfillment_record = web_state
        .app
        .storage
        .get_role_fulfillment(parsed_cand_id)
        .await
        .unwrap()
        .expect("fulfillment record must exist");
    assert_eq!(fulfillment_record.status, "completed");
    assert_eq!(
        role_delivery_count.load(Ordering::SeqCst),
        baseline_delivery + 1
    );

    // Test 8: Worker defense-in-depth on privileged role
    // Inject a pending fulfillment record with role 1005 (ADMINISTRATOR)
    let bad_app_id = uuid::Uuid::new_v4();
    web_state
        .app
        .storage
        .upsert_role_fulfillment(rusty_discord_bot::storage::RecruitmentRoleFulfillment {
            application_id: bad_app_id,
            guild_id: guild_a.to_string(),
            user_id: candidate_id.to_string(),
            role_id: "1005".to_string(), // ADMINISTRATOR
            status: "pending".to_string(),
            attempts: 0,
            last_error: None,
            next_attempt_at: Utc::now() - Duration::minutes(1),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();

    // Create corresponding accepted application so application status check passes
    web_state
        .app
        .storage
        .create_recruitment_application(rusty_discord_bot::recruitment::RecruitmentApplication {
            id: bad_app_id,
            guild_id: guild_a.to_string(),
            track_id: uuid::Uuid::new_v4(),
            applicant_user_id: candidate_id.to_string(),
            track_version: 1,
            status: "accepted".to_string(),
            submitted_answers: sqlx::types::Json(vec![]),
            current_stage: "completed".to_string(),
            version: 1,
            submitted_at: Utc::now(),
            updated_at: Utc::now(),
            decided_at: Some(Utc::now()),
            decision_reason: None,
            decided_by: Some(admin_id.to_string()),
        })
        .await
        .unwrap();

    let delivery_before_bad = role_delivery_count.load(Ordering::SeqCst);
    let proc_bad_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/fulfillment/process"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(proc_bad_resp.status(), reqwest::StatusCode::OK);

    let bad_record = web_state
        .app
        .storage
        .get_role_fulfillment(bad_app_id)
        .await
        .unwrap()
        .expect("bad fulfillment record must exist");
    assert_eq!(bad_record.status, "failed");
    assert!(
        bad_record
            .last_error
            .as_deref()
            .unwrap_or("")
            .contains("administrative/privileged permissions"),
        "Unexpected error: {:?}",
        bad_record.last_error
    );
    assert_eq!(
        role_delivery_count.load(Ordering::SeqCst),
        delivery_before_bad,
        "No PUT request must be dispatched for privileged role"
    );
}

#[tokio::test]
async fn test_recruitment_review_role_authorization_and_approval_policy_suite() {
    use axum::Json;
    use axum::extract::Path;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use reqwest::header::COOKIE;
    use rusty_discord_bot::storage::RecruitmentReview;

    let guild_a = "100000000000000001";
    let owner_id = "999000000000000000";
    let admin_id = "111111111111111111";
    let role_reviewer_id = "222222222222222222";
    let candidate_id = "444444444444444444";
    let lost_role_user_id = "555555555555555555";
    let other_member_id = "777777777777777777";

    let discord_mock_app = axum::Router::new()
        .route(
            "/users/@me",
            get(|| async move {
                Json(serde_json::json!({
                    "id": "999999999999999999"
                }))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}",
            get(|Path(gid): Path<String>| async move {
                Json(serde_json::json!({
                    "id": gid,
                    "owner_id": "999000000000000000",
                    "roles": [
                        {
                            "id": "2001",
                            "name": "ReviewerRole",
                            "permissions": "0",
                            "position": 10,
                            "managed": false,
                            "color": 0,
                            "colors": { "primary_color": 0 },
                            "hoist": false,
                            "mentionable": false,
                            "flags": 0
                        },
                        {
                            "id": "2002",
                            "name": "AdminRole",
                            "permissions": "8",
                            "position": 20,
                            "managed": false,
                            "color": 0,
                            "colors": { "primary_color": 0 },
                            "hoist": false,
                            "mentionable": false,
                            "flags": 0
                        }
                    ]
                }))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}/roles",
            get(|| async move {
                Json(serde_json::json!([
                    {
                        "id": "2001",
                        "name": "ReviewerRole",
                        "permissions": "0",
                        "position": 10,
                        "managed": false,
                        "color": 0,
                        "colors": { "primary_color": 0 },
                        "hoist": false,
                        "mentionable": false,
                        "flags": 0
                    },
                    {
                        "id": "2002",
                        "name": "AdminRole",
                        "permissions": "8",
                        "position": 20,
                        "managed": false,
                        "color": 0,
                        "colors": { "primary_color": 0 },
                        "hoist": false,
                        "mentionable": false,
                        "flags": 0
                    }
                ]))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}/permissions/member/{user_id}",
            get(|Path((_gid, user_id)): Path<(String, String)>| async move {
                if user_id == "111111111111111111" {
                    Json(serde_json::json!({ "permissions": "32" })).into_response()
                } else if user_id == "999000000000000000" {
                    Json(serde_json::json!({ "permissions": "8" })).into_response()
                } else {
                    Json(serde_json::json!({ "permissions": "0" })).into_response()
                }
            }),
        )
        .route(
            "/guilds/{guild_id}/members/{user_id}",
            get(|Path((_gid, user_id)): Path<(String, String)>| async move {
                if user_id == "222222222222222222" {
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": ["2001"],
                        "permissions": "0"
                    }))
                    .into_response()
                } else if user_id == "111111111111111111" {
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": [],
                        "permissions": "32"
                    }))
                    .into_response()
                } else if user_id == "999000000000000000" {
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": [],
                        "permissions": "8"
                    }))
                    .into_response()
                } else {
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": [],
                        "permissions": "0"
                    }))
                    .into_response()
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

    let mut config = test_config();
    config.discord_token = "live_bot_token_review_role_secret".to_string();
    config.offline_mode = false;
    config.discord_api_base_url = format!("http://{discord_addr}");

    let app = AppState::bootstrap(config).await.expect("bootstrap");
    let permissions_cache = std::sync::Arc::new(dashmap::DashMap::new());
    let web_state = WebState::new(
        app,
        std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    );

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

    let admin_session = "admin_rr_sess";
    let admin_csrf = "admin_rr_csrf";

    let role_reviewer_session = "role_rev_sess";
    let role_reviewer_csrf = "role_rev_csrf";

    let owner_session = "owner_rr_sess";
    let owner_csrf = "owner_rr_csrf";

    let candidate_session = "cand_rr_sess";
    let candidate_csrf = "cand_rr_csrf";

    let other_session = "other_rr_sess";
    let other_csrf = "other_rr_csrf";

    let users = [
        (admin_id, "admin", admin_session, admin_csrf, "32", false),
        (
            role_reviewer_id,
            "role_reviewer",
            role_reviewer_session,
            role_reviewer_csrf,
            "0",
            false,
        ),
        (owner_id, "owner", owner_session, owner_csrf, "8", true),
        (
            candidate_id,
            "candidate",
            candidate_session,
            candidate_csrf,
            "0",
            false,
        ),
        (
            other_member_id,
            "other_member",
            other_session,
            other_csrf,
            "0",
            false,
        ),
    ];

    for (uid, uname, sid, csrf, perms, is_owner) in users {
        web_state
            .app
            .sessions
            .save_session(
                sid,
                DashboardUser {
                    id: uid.to_owned(),
                    username: uname.to_owned(),
                    global_name: None,
                    avatar: None,
                },
                vec![DashboardGuild {
                    id: guild_a.to_owned(),
                    name: "Guild A".to_owned(),
                    icon: None,
                    owner: is_owner,
                    permissions: perms.to_owned(),
                }],
                csrf.to_owned(),
                Utc::now() + Duration::hours(2),
                None,
            )
            .await
            .unwrap();

        if perms != "0" {
            web_state.permissions_cache.insert(
                (uid.to_owned(), guild_a.to_owned()),
                CachedGuildPermission {
                    can_manage: true,
                    checked_at: Utc::now(),
                },
            );
        }
    }

    let cookie_admin = format!("rusty_discord_bot_session={admin_session}");
    let cookie_rev = format!("rusty_discord_bot_session={role_reviewer_session}");
    let cookie_owner = format!("rusty_discord_bot_session={owner_session}");
    let cookie_candidate = format!("rusty_discord_bot_session={candidate_session}");
    let cookie_other = format!("rusty_discord_bot_session={other_session}");

    // 1. Walidacja tracku: próba dodania nieistniejącej roli w review_role_ids zwraca 400
    let bad_track_payload = serde_json::json!({
        "slug": "bad-track",
        "name": "Bad Track",
        "enabled": true,
        "config": {
            "questions": [],
            "review_role_ids": ["999999999999999999"],
            "final_approvals": 1
        }
    });
    let bad_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/tracks"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&bad_track_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(bad_resp.status(), reqwest::StatusCode::BAD_REQUEST);
    let bad_body: serde_json::Value = bad_resp.json().await.unwrap();
    assert!(
        bad_body["error"]
            .as_str()
            .unwrap()
            .contains("does not exist on this server"),
        "Unexpected error: {:?}",
        bad_body
    );

    // 2. Utworzenie poprawnego tracku z review_role_ids=["2001"], final_approvals=2
    let good_track_payload = serde_json::json!({
        "slug": "moderator",
        "name": "Moderator Track",
        "enabled": true,
        "config": {
            "questions": [],
            "review_role_ids": ["2001"],
            "final_approvals": 2
        }
    });
    let track_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/tracks"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&good_track_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(track_resp.status(), reqwest::StatusCode::OK);
    let track_body: serde_json::Value = track_resp.json().await.unwrap();
    let track_id_str = track_body["track"]["id"].as_str().unwrap();
    let _track_id = uuid::Uuid::parse_str(track_id_str).unwrap();

    // 3. Kandydat składa aplikację
    let apply_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/tracks/moderator/apply"
        ))
        .header(COOKIE, &cookie_candidate)
        .header("x-csrf-token", candidate_csrf)
        .json(&serde_json::json!({ "answers": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(apply_resp.status(), reqwest::StatusCode::OK);
    let apply_body: serde_json::Value = apply_resp.json().await.unwrap();
    let app_id_str = apply_body["application"]["id"].as_str().unwrap();
    let app_id = uuid::Uuid::parse_str(app_id_str).unwrap();

    // 4. Inny zwykły członek bez roli nie może odczytać aplikacji
    let other_view_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/applications/{app_id}"
        ))
        .header(COOKIE, &cookie_other)
        .send()
        .await
        .unwrap();
    assert_eq!(other_view_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 5. Kandydat widzi swoją aplikację bez staff notes
    let cand_view_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/applications/{app_id}"
        ))
        .header(COOKIE, &cookie_candidate)
        .send()
        .await
        .unwrap();
    assert_eq!(cand_view_resp.status(), reqwest::StatusCode::OK);
    let cand_view_body: serde_json::Value = cand_view_resp.json().await.unwrap();
    assert!(cand_view_body["application"]["reviews"].is_null());

    // 6. Użytkownik z MANAGE_GUILD bez roli recenzenta (admin_id) próbuje zrobić review -> 403 Forbidden!
    let admin_review_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/applications/{app_id}/review"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "recommendation": "approve",
            "note": "admin unauthorized review"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(admin_review_resp.status(), reqwest::StatusCode::FORBIDDEN);
    let admin_rev_body: serde_json::Value = admin_review_resp.json().await.unwrap();
    assert!(
        admin_rev_body["error"]
            .as_str()
            .unwrap()
            .contains("not assigned to review"),
        "Unexpected error: {:?}",
        admin_rev_body
    );

    // Upewnij się, że w bazie brak zapisanego review od admina
    let reviews_in_db = web_state
        .app
        .storage
        .list_recruitment_reviews(app_id)
        .await
        .unwrap();
    assert_eq!(reviews_in_db.len(), 0);

    // 7. Recenzent z przypisaną rolą (2001) pomyślnie dodaje review approve
    let rev_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/applications/{app_id}/review"
        ))
        .header(COOKIE, &cookie_rev)
        .header("x-csrf-token", role_reviewer_csrf)
        .json(&serde_json::json!({
            "recommendation": "approve",
            "note": "reviewer approved"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(rev_resp.status(), reqwest::StatusCode::OK);

    // 8. Próba akceptacji przy 1 approvalu (wymagane 2) -> 400 Bad Request
    let trans_fail_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/applications/{app_id}/transition"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "new_status": "accepted",
            "new_stage": "accepted",
            "expected_version": 1
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(trans_fail_resp.status(), reqwest::StatusCode::BAD_REQUEST);
    let trans_fail_body: serde_json::Value = trans_fail_resp.json().await.unwrap();
    assert!(
        trans_fail_body["error"]
            .as_str()
            .unwrap()
            .contains("requires at least 2 distinct approvals"),
        "Unexpected error: {:?}",
        trans_fail_body
    );

    // 9. Zapisanie do bazy głosu od użytkownika, który nie ma roli w Discord API (lost_role_user_id)
    web_state
        .app
        .storage
        .upsert_recruitment_review(RecruitmentReview {
            id: uuid::Uuid::new_v4(),
            application_id: app_id,
            guild_id: guild_a.to_string(),
            reviewer_user_id: lost_role_user_id.to_string(),
            recommendation: "approve".to_string(),
            note: Some("stale review".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();

    // Próba akceptacji ponownie: nieuprawniony głos nie może się liczyć!
    let trans_fail_again = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/applications/{app_id}/transition"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "new_status": "accepted",
            "new_stage": "accepted",
            "expected_version": 1
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(trans_fail_again.status(), reqwest::StatusCode::BAD_REQUEST);

    // 10. Właściciel gildii (owner_id) dodaje approval (posiada owner override)
    let owner_rev_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/applications/{app_id}/review"
        ))
        .header(COOKIE, &cookie_owner)
        .header("x-csrf-token", owner_csrf)
        .json(&serde_json::json!({
            "recommendation": "approve",
            "note": "owner approved"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(owner_rev_resp.status(), reqwest::StatusCode::OK);

    // 11. Teraz mamy 2 uprawnione approve (role_reviewer + owner). Akceptacja musi się powieść!
    let trans_success = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/applications/{app_id}/transition"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "new_status": "accepted",
            "new_stage": "accepted",
            "expected_version": 1
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(trans_success.status(), reqwest::StatusCode::OK);
    let app_accepted = web_state
        .app
        .storage
        .get_recruitment_application(guild_a, app_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(app_accepted.status, "accepted");

    // 12. Recenzent z rolą widzi staff view aplikacji z recenzjami
    let staff_view_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/applications/{app_id}"
        ))
        .header(COOKIE, &cookie_rev)
        .send()
        .await
        .unwrap();
    assert_eq!(staff_view_resp.status(), reqwest::StatusCode::OK);
    let staff_view_body: serde_json::Value = staff_view_resp.json().await.unwrap();
    let reviews_arr = staff_view_body["application"]["reviews"]
        .as_array()
        .unwrap();
    assert!(reviews_arr.len() >= 2);

    // 13. Kompatybilność wsteczna: track z pustym review_role_ids=[] dopuszcza review i akceptację od admina
    let legacy_track_payload = serde_json::json!({
        "slug": "legacy",
        "name": "Legacy Track",
        "enabled": true,
        "config": {
            "questions": [],
            "review_role_ids": [],
            "final_approvals": 1
        }
    });
    let leg_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/tracks"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&legacy_track_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(leg_resp.status(), reqwest::StatusCode::OK);

    let leg_apply = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/tracks/legacy/apply"
        ))
        .header(COOKIE, &cookie_candidate)
        .header("x-csrf-token", candidate_csrf)
        .json(&serde_json::json!({ "answers": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(leg_apply.status(), reqwest::StatusCode::OK);
    let leg_app_id = uuid::Uuid::parse_str(
        leg_apply.json::<serde_json::Value>().await.unwrap()["application"]["id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();

    let leg_review = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/applications/{leg_app_id}/review"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "recommendation": "approve",
            "note": "legacy admin approve"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(leg_review.status(), reqwest::StatusCode::OK);

    let leg_trans = client
        .post(format!(
            "{base_url}/api/guilds/{guild_a}/recruitment/applications/{leg_app_id}/transition"
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "new_status": "accepted",
            "new_stage": "accepted",
            "expected_version": 1
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(leg_trans.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn test_store_purchase_concurrent_fulfillment_race_suite() {
    use axum::Json;
    use axum::extract::Path;
    use axum::response::IntoResponse;
    use axum::routing::{get, put};
    use reqwest::header::COOKIE;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let guild_id = "888000000000000001";
    let buyer_id = "888000000000000002";
    let role_id = "888000000000000003";

    let role_delivery_count = Arc::new(AtomicUsize::new(0));
    let role_delivery_count_clone = role_delivery_count.clone();

    let discord_mock_app = axum::Router::new()
        .route(
            "/users/@me",
            get(|| async move {
                Json(serde_json::json!({
                    "id": "999999999999999999"
                }))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}",
            get(|Path(gid): Path<String>| async move {
                Json(serde_json::json!({
                    "id": gid,
                    "owner_id": "999000000000000000",
                    "roles": [
                        {
                            "id": "888000000000000003",
                            "name": "StoreRole",
                            "permissions": "0",
                            "position": 10,
                            "managed": false,
                            "color": 0,
                            "colors": { "primary_color": 0 },
                            "hoist": false,
                            "mentionable": false,
                            "flags": 0
                        }
                    ]
                }))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}/roles",
            get(|| async move {
                Json(serde_json::json!([
                    {
                        "id": "888000000000000003",
                        "name": "StoreRole",
                        "permissions": "0",
                        "position": 10,
                        "managed": false,
                        "color": 0,
                        "colors": { "primary_color": 0 },
                        "hoist": false,
                        "mentionable": false,
                        "flags": 0
                    },
                    {
                        "id": "888000000000000004",
                        "name": "BotRole",
                        "permissions": "268435456",
                        "position": 60,
                        "managed": false,
                        "color": 0,
                        "colors": { "primary_color": 0 },
                        "hoist": false,
                        "mentionable": false,
                        "flags": 0
                    }
                ]))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}/members/{user_id}",
            get(|Path((_gid, user_id)): Path<(String, String)>| async move {
                if user_id == "999999999999999999" {
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": ["888000000000000004"],
                        "permissions": "268435456"
                    }))
                    .into_response()
                } else {
                    Json(serde_json::json!({
                        "user": { "id": user_id },
                        "roles": [],
                        "permissions": "0"
                    }))
                    .into_response()
                }
            }),
        )
        .route(
            "/guilds/{guild_id}/permissions/member/{user_id}",
            get(|| async move { Json(serde_json::json!({ "permissions": "0" })).into_response() }),
        )
        .route(
            "/guilds/{guild_id}/members/{user_id}/roles/{role_id}",
            put({
                let count = role_delivery_count_clone.clone();
                move || {
                    let count = count.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        // Add artificial delay to simulate realistic network latency
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        reqwest::StatusCode::NO_CONTENT.into_response()
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

    let mut config = test_config();
    config.discord_token = "live_bot_token_concurrent_delivery_secret".to_string();
    config.offline_mode = false;
    config.discord_api_base_url = format!("http://{discord_addr}");

    let app = AppState::bootstrap(config).await.expect("bootstrap");
    let permissions_cache = std::sync::Arc::new(dashmap::DashMap::new());
    let web_state = WebState::new(
        app,
        std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    );

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

    let buyer_session = "buyer_concurrent_sess";
    let buyer_csrf = "buyer_concurrent_csrf";

    web_state
        .app
        .sessions
        .save_session(
            buyer_session,
            DashboardUser {
                id: buyer_id.to_owned(),
                username: "buyer".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Guild Concurrent".to_owned(),
                icon: None,
                owner: false,
                permissions: "0".to_owned(),
            }],
            buyer_csrf.to_owned(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .unwrap();

    let cookie_buyer = format!("rusty_discord_bot_session={buyer_session}");

    // Give buyer 1000 coins
    web_state
        .app
        .storage
        .adjust_member_progress(guild_id, buyer_id, 0, 1000, 0)
        .await
        .unwrap();

    // Create store item with role_id
    let item = web_state
        .app
        .storage
        .create_store_item(
            guild_id,
            "VIP Role",
            "Grants VIP status",
            100,
            Some(role_id),
            Some(5),
        )
        .await
        .unwrap();

    // 1. Test unitarny: 20 równoległych tasków próbuje wykonać claim_store_role_delivery
    let (_test_item, test_purchase, _) = web_state
        .app
        .storage
        .purchase_store_item(guild_id, buyer_id, item.id, Some("claim_test_int_1"))
        .await
        .unwrap();
    assert_eq!(test_purchase.fulfillment_status, "pending");

    let mut handles = Vec::new();
    for _ in 0..20 {
        let storage = web_state.app.storage.clone();
        let pid = test_purchase.id;
        handles.push(tokio::spawn(async move {
            storage.claim_store_role_delivery(pid).await.unwrap()
        }));
    }

    let mut true_count = 0;
    let mut false_count = 0;
    for h in handles {
        if h.await.unwrap() {
            true_count += 1;
        } else {
            false_count += 1;
        }
    }
    assert_eq!(true_count, 1, "Exactly one task must claim the delivery");
    assert_eq!(false_count, 19, "All other 19 tasks must receive false");

    // 2. Test statusu: update na completed, a następnie próba nadpisania przez failed
    web_state
        .app
        .storage
        .update_store_purchase_fulfillment(test_purchase.id, "completed", None, Some(Utc::now()))
        .await
        .unwrap();

    // Próba nadpisania na failed musi zostać zignorowana (status pozostaje completed)
    web_state
        .app
        .storage
        .update_store_purchase_fulfillment(
            test_purchase.id,
            "failed",
            Some("late failure override attempt"),
            None,
        )
        .await
        .unwrap();

    let check_purchase = web_state
        .app
        .storage
        .get_store_purchase(test_purchase.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(check_purchase.fulfillment_status, "completed");
    assert!(check_purchase.failure_reason.is_none());

    // 3. Test wyścigu HTTP: Dwa jednoczesne żądania POST z tym samym Idempotency-Key
    let idempotency_key = "concurrent_retry_abc";
    let req1 = client
        .post(format!("{base_url}/api/guilds/{guild_id}/store/purchase"))
        .header(COOKIE, &cookie_buyer)
        .header("x-csrf-token", buyer_csrf)
        .header("idempotency-key", idempotency_key)
        .json(&serde_json::json!({ "item_id": item.id }));

    let req2 = client
        .post(format!("{base_url}/api/guilds/{guild_id}/store/purchase"))
        .header(COOKIE, &cookie_buyer)
        .header("x-csrf-token", buyer_csrf)
        .header("idempotency-key", idempotency_key)
        .json(&serde_json::json!({ "item_id": item.id }));

    let count_before = role_delivery_count.load(Ordering::SeqCst);

    let (res1, res2) = tokio::join!(req1.send(), req2.send());
    let (resp1, resp2) = (res1.unwrap(), res2.unwrap());

    assert_eq!(resp1.status(), reqwest::StatusCode::OK);
    assert_eq!(resp2.status(), reqwest::StatusCode::OK);

    let json1: serde_json::Value = resp1.json().await.unwrap();
    let json2: serde_json::Value = resp2.json().await.unwrap();

    // Identyczny purchase_id
    assert_eq!(json1["purchase_id"], json2["purchase_id"]);
    let final_pid = uuid::Uuid::parse_str(json1["purchase_id"].as_str().unwrap()).unwrap();

    // Dokładnie jedno wywołanie Discord API PUT role!
    let count_after = role_delivery_count.load(Ordering::SeqCst);
    assert_eq!(
        count_after,
        count_before + 1,
        "Discord API PUT must be dispatched exactly once despite concurrent requests"
    );

    // Saldo i stan magazynowy zmniejszone dokładnie jeden raz
    let final_profile = web_state
        .app
        .storage
        .get_member_progress(guild_id, buyer_id)
        .await
        .unwrap();
    // 1000 - 100 (test 1) - 100 (test 2) = 800
    assert_eq!(final_profile.coins, 800);

    let final_items = web_state
        .app
        .storage
        .list_store_items(guild_id)
        .await
        .unwrap();
    let final_item = final_items.iter().find(|i| i.id == item.id).unwrap();
    // 5 - 1 - 1 = 3
    assert_eq!(final_item.stock, Some(3));

    // Zakup ostatecznie ma status completed
    let final_purchase = web_state
        .app
        .storage
        .get_store_purchase(final_pid)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(final_purchase.fulfillment_status, "completed");

    // 4. Test ponowionego żądania po zakończeniu - zwraca completed bez kolejnego PUT do Discorda
    let retry_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/store/purchase"))
        .header(COOKIE, &cookie_buyer)
        .header("x-csrf-token", buyer_csrf)
        .header("idempotency-key", idempotency_key)
        .json(&serde_json::json!({ "item_id": item.id }))
        .send()
        .await
        .unwrap();
    assert_eq!(retry_resp.status(), reqwest::StatusCode::OK);
    let retry_json: serde_json::Value = retry_resp.json().await.unwrap();
    assert_eq!(retry_json["fulfillment_status"], "completed");
    assert_eq!(
        role_delivery_count.load(Ordering::SeqCst),
        count_after,
        "No additional Discord PUT should occur on completed purchase retry"
    );
}

#[tokio::test]
async fn test_economy_store_purchase_idempotency_validation_and_rate_limit_suite() {
    let guild_id = "888000888000888000";
    let buyer_id = "777000777000777000";

    let mut config = test_config();
    config.offline_mode = true;

    let app = AppState::bootstrap(config).await.expect("bootstrap");
    let permissions_cache = std::sync::Arc::new(dashmap::DashMap::new());
    let web_state = WebState::new(
        app,
        std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    );

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

    let buyer_session = "buyer_val_sess";
    let buyer_csrf = "buyer_val_csrf";

    web_state
        .app
        .sessions
        .save_session(
            buyer_session,
            DashboardUser {
                id: buyer_id.to_owned(),
                username: "val_buyer".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Guild Val".to_owned(),
                icon: None,
                owner: false,
                permissions: "0".to_owned(),
            }],
            buyer_csrf.to_owned(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .unwrap();

    let cookie_buyer = format!("rusty_discord_bot_session={buyer_session}");

    // Give buyer 100,000 coins so balance is never exhausted during rate limit tests
    web_state
        .app
        .storage
        .adjust_member_progress(guild_id, buyer_id, 0, 100_000, 0)
        .await
        .unwrap();

    let item = web_state
        .app
        .storage
        .create_store_item(
            guild_id,
            "Validation Candy",
            "Sweet candy",
            10,
            None,
            Some(100),
        )
        .await
        .unwrap();

    // 1. Empty idempotency key -> 400 Bad Request
    let resp_empty = client
        .post(format!("{base_url}/api/guilds/{guild_id}/store/purchase"))
        .header(COOKIE, &cookie_buyer)
        .header("x-csrf-token", buyer_csrf)
        .header("idempotency-key", "")
        .json(&serde_json::json!({ "item_id": item.id }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_empty.status(), reqwest::StatusCode::BAD_REQUEST);
    let body_empty: serde_json::Value = resp_empty.json().await.unwrap();
    assert!(
        body_empty["error"]
            .as_str()
            .unwrap()
            .contains("idempotency-key header cannot be empty")
    );

    // 2. Whitespace idempotency key -> 400 Bad Request
    let resp_ws = client
        .post(format!("{base_url}/api/guilds/{guild_id}/store/purchase"))
        .header(COOKIE, &cookie_buyer)
        .header("x-csrf-token", buyer_csrf)
        .header("idempotency-key", "   ")
        .json(&serde_json::json!({ "item_id": item.id }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_ws.status(), reqwest::StatusCode::BAD_REQUEST);
    let body_ws: serde_json::Value = resp_ws.json().await.unwrap();
    assert!(
        body_ws["error"]
            .as_str()
            .unwrap()
            .contains("idempotency-key header cannot be empty")
    );

    // 3. Overly long key (> 128 chars) -> 400 Bad Request
    let long_key = "k".repeat(129);
    let resp_long = client
        .post(format!("{base_url}/api/guilds/{guild_id}/store/purchase"))
        .header(COOKIE, &cookie_buyer)
        .header("x-csrf-token", buyer_csrf)
        .header("idempotency-key", &long_key)
        .json(&serde_json::json!({ "item_id": item.id }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_long.status(), reqwest::StatusCode::BAD_REQUEST);
    let body_long: serde_json::Value = resp_long.json().await.unwrap();
    assert!(
        body_long["error"]
            .as_str()
            .unwrap()
            .contains("exceeds maximum length of 128 characters")
    );

    // 4. Invalid characters (spaces / control characters) -> 400 Bad Request
    let resp_invalid_chars = client
        .post(format!("{base_url}/api/guilds/{guild_id}/store/purchase"))
        .header(COOKIE, &cookie_buyer)
        .header("x-csrf-token", buyer_csrf)
        .header("idempotency-key", "my key with spaces")
        .json(&serde_json::json!({ "item_id": item.id }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp_invalid_chars.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let body_invalid: serde_json::Value = resp_invalid_chars.json().await.unwrap();
    assert!(
        body_invalid["error"]
            .as_str()
            .unwrap()
            .contains("contains invalid characters")
    );

    // 5. Valid key -> 200 OK and idempotent retry
    let valid_key = "valid_client_order_001";
    let resp_valid = client
        .post(format!("{base_url}/api/guilds/{guild_id}/store/purchase"))
        .header(COOKIE, &cookie_buyer)
        .header("x-csrf-token", buyer_csrf)
        .header("idempotency-key", valid_key)
        .json(&serde_json::json!({ "item_id": item.id }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_valid.status(), reqwest::StatusCode::OK);
    let body_valid: serde_json::Value = resp_valid.json().await.unwrap();
    let purchase_id = body_valid["purchase_id"].as_str().unwrap().to_string();
    assert_eq!(body_valid["remaining_coins"], 99_990);

    // Retry with exact same valid key returns cached purchase without re-debiting
    let resp_retry = client
        .post(format!("{base_url}/api/guilds/{guild_id}/store/purchase"))
        .header(COOKIE, &cookie_buyer)
        .header("x-csrf-token", buyer_csrf)
        .header("idempotency-key", valid_key)
        .json(&serde_json::json!({ "item_id": item.id }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_retry.status(), reqwest::StatusCode::OK);
    let body_retry: serde_json::Value = resp_retry.json().await.unwrap();
    assert_eq!(body_retry["purchase_id"], purchase_id);
    assert_eq!(body_retry["remaining_coins"], 99_990);

    // 6. Test Rate Limiting on store purchase (quota is 30 per 60s)
    let mut rate_limited = false;
    for i in 0..35 {
        let key = format!("rate_test_key_{i}");
        let r = client
            .post(format!("{base_url}/api/guilds/{guild_id}/store/purchase"))
            .header(COOKIE, &cookie_buyer)
            .header("x-csrf-token", buyer_csrf)
            .header("idempotency-key", &key)
            .json(&serde_json::json!({ "item_id": item.id }))
            .send()
            .await
            .unwrap();
        if r.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            rate_limited = true;
            let b: serde_json::Value = r.json().await.unwrap();
            assert!(
                b["error"]
                    .as_str()
                    .unwrap()
                    .contains("Store purchases are too frequent")
            );
            break;
        }
    }
    assert!(
        rate_limited,
        "Rapid purchase flooding must trigger 429 Too Many Requests"
    );
}

#[tokio::test]
async fn test_recruitment_n_plus_one_avoidance_and_approval_threshold_suite() {
    use axum::Json;
    use axum::extract::Path;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use reqwest::header::COOKIE;
    use rusty_discord_bot::recruitment::{
        RecruitmentApplication, RecruitmentReview, RecruitmentTrack, RecruitmentTrackConfig,
    };

    let guild_id = "555000555000555001";
    let owner_id = "999000000000000000";
    let reviewer_id = "222000222000222001";
    let candidate_id = "444000444000444001";

    let guild_get_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let member_get_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let g_cnt = guild_get_count.clone();
    let m_cnt = member_get_count.clone();

    let discord_mock_app = axum::Router::new()
        .route(
            "/users/@me",
            get(|| async move {
                Json(serde_json::json!({
                    "id": "999999999999999999"
                }))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}/permissions/member/{user_id}",
            get(|| async move {
                Json(serde_json::json!({
                    "permissions": "0"
                }))
                .into_response()
            }),
        )
        .route(
            "/guilds/{guild_id}",
            get(move |Path(gid): Path<String>| {
                let g = g_cnt.clone();
                async move {
                    g.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Json(serde_json::json!({
                        "id": gid,
                        "owner_id": "999000000000000000",
                        "roles": [
                            {
                                "id": "2001",
                                "name": "ReviewerRole",
                                "permissions": "0",
                                "position": 10,
                                "managed": false,
                                "color": 0,
                                "colors": { "primary_color": 0 },
                                "hoist": false,
                                "mentionable": false,
                                "flags": 0
                            },
                            {
                                "id": "9999",
                                "name": "OtherRole",
                                "permissions": "0",
                                "position": 12,
                                "managed": false,
                                "color": 0,
                                "colors": { "primary_color": 0 },
                                "hoist": false,
                                "mentionable": false,
                                "flags": 0
                            }
                        ]
                    }))
                    .into_response()
                }
            }),
        )
        .route(
            "/guilds/{guild_id}/members/{user_id}",
            get(move |Path((_gid, uid)): Path<(String, String)>| {
                let m = m_cnt.clone();
                async move {
                    m.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let roles = if uid == "222000222000222001"
                        || uid == "333000333000333001"
                        || uid == "333000333000333002"
                        || uid == "333000333000333003"
                        || uid == "333000333000333004"
                        || uid == "333000333000333005"
                    {
                        vec!["2001"]
                    } else {
                        vec![]
                    };
                    Json(serde_json::json!({
                        "user": { "id": uid, "username": format!("user_{uid}") },
                        "roles": roles
                    }))
                    .into_response()
                }
            }),
        );

    let discord_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind discord listener");
    let discord_addr = discord_listener.local_addr().expect("discord addr");
    tokio::spawn(async move {
        axum::serve(
            discord_listener,
            discord_mock_app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });

    let mut config = test_config();
    config.discord_token = "live_bot_token_n_plus_one_secret".to_string();
    config.offline_mode = false;
    config.discord_api_base_url = format!("http://{discord_addr}");

    let app = AppState::bootstrap(config).await.expect("bootstrap");
    let permissions_cache = std::sync::Arc::new(dashmap::DashMap::new());
    let web_state = WebState::new(
        app,
        std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache,
    );
    let router = build_router(web_state.clone());

    let server_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind web test listener");
    let server_addr = server_listener.local_addr().expect("server addr");
    tokio::spawn(async move {
        axum::serve(
            server_listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });

    let base_url = format!("http://{server_addr}");
    let client = reqwest::Client::new();

    // Setup sessions
    let reviewer_session = "reviewer_sess_n1";
    let reviewer_csrf = "reviewer_csrf_n1";
    web_state
        .app
        .sessions
        .save_session(
            reviewer_session,
            DashboardUser {
                id: reviewer_id.to_owned(),
                username: "reviewer_user".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Guild N1".to_owned(),
                icon: None,
                owner: false,
                permissions: "0".to_owned(),
            }],
            reviewer_csrf.to_owned(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .unwrap();

    let cookie_reviewer = format!("rusty_discord_bot_session={reviewer_session}");

    // Create Track 1 with review_role_ids = ["2001"]
    let track_1 = web_state
        .app
        .storage
        .upsert_recruitment_track(RecruitmentTrack {
            id: uuid::Uuid::new_v4(),
            guild_id: guild_id.to_string(),
            slug: "track-moderator".to_string(),
            name: "Moderator Track".to_string(),
            enabled: true,
            target_role_id: None,
            version: 1,
            config: sqlx::types::Json(RecruitmentTrackConfig {
                review_role_ids: vec!["2001".to_string()],
                final_approvals: 2,
                ..Default::default()
            }),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();

    // Create 100 applications for Track 1
    for i in 0..100 {
        let app_row = RecruitmentApplication {
            id: uuid::Uuid::new_v4(),
            guild_id: guild_id.to_string(),
            track_id: track_1.id,
            applicant_user_id: format!("444000000000000{:03}", i),
            track_version: 1,
            status: "submitted".to_string(),
            submitted_answers: sqlx::types::Json(vec![]),
            current_stage: "screening".to_string(),
            version: 1,
            submitted_at: Utc::now(),
            updated_at: Utc::now(),
            decided_at: None,
            decision_reason: None,
            decided_by: None,
        };
        web_state
            .app
            .storage
            .create_recruitment_application(app_row)
            .await
            .unwrap();
    }

    // Reset REST counters before the test
    guild_get_count.store(0, std::sync::atomic::Ordering::SeqCst);
    member_get_count.store(0, std::sync::atomic::Ordering::SeqCst);
    web_state
        .app
        .metrics
        .discord_rest_requests_total
        .store(0, std::sync::atomic::Ordering::SeqCst);

    // Fetch page of 100 applications
    let list_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/applications?track_id={}&limit=100",
            track_1.id
        ))
        .header(COOKIE, &cookie_reviewer)
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp.status(), reqwest::StatusCode::OK);
    let list_body: serde_json::Value = list_resp.json().await.unwrap();
    let apps_array = list_body["applications"].as_array().unwrap();
    assert_eq!(apps_array.len(), 100);

    // CRITICAL ASSERTION for Issue #297:
    // Without N+1 optimization, 100 applications would trigger 100 GET /guilds and 100 GET /members.
    // With ActorReviewerAccessContext, there is at most 1 GET /guilds (owner check) and 1 GET /members (roles check)
    // for the entire request!
    let g_calls = guild_get_count.load(std::sync::atomic::Ordering::SeqCst);
    let m_calls = member_get_count.load(std::sync::atomic::Ordering::SeqCst);
    let rest_metrics = web_state
        .app
        .metrics
        .discord_rest_requests_total
        .load(std::sync::atomic::Ordering::SeqCst);

    assert!(
        g_calls <= 1,
        "GET /guilds must be executed at most once per request, but got {g_calls}"
    );
    assert!(
        m_calls <= 2,
        "GET /members must be executed at most twice per request (1 auth + 1 roles), but got {m_calls}"
    );
    assert!(
        rest_metrics <= 2,
        "Staff reviewer authorization must perform at most 2 REST calls (1 owner, 1 roles), but performed {rest_metrics}"
    );

    // Verify all 100 views are staff views (contain staff-level reviews array)
    for app_view in apps_array {
        assert!(
            app_view.get("reviews").is_some(),
            "Staff view must contain reviews"
        );
    }

    // Check Multi-track optimization: Create Track 2 with role "9999" (reviewer does NOT have it)
    let track_2 = web_state
        .app
        .storage
        .upsert_recruitment_track(RecruitmentTrack {
            id: uuid::Uuid::new_v4(),
            guild_id: guild_id.to_string(),
            slug: "track-admin".to_string(),
            name: "Admin Track".to_string(),
            enabled: true,
            target_role_id: None,
            version: 1,
            config: sqlx::types::Json(RecruitmentTrackConfig {
                review_role_ids: vec!["9999".to_string()],
                final_approvals: 2,
                ..Default::default()
            }),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();

    let app_track_2 = RecruitmentApplication {
        id: uuid::Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        track_id: track_2.id,
        applicant_user_id: "444999999999999999".to_string(),
        track_version: 1,
        status: "submitted".to_string(),
        submitted_answers: sqlx::types::Json(vec![]),
        current_stage: "screening".to_string(),
        version: 1,
        submitted_at: Utc::now(),
        updated_at: Utc::now(),
        decided_at: None,
        decision_reason: None,
        decided_by: None,
    };
    web_state
        .app
        .storage
        .create_recruitment_application(app_track_2.clone())
        .await
        .unwrap();

    // Verify staff notes/reviews of track 2 do NOT leak to reviewer of track 1
    let app2_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/applications/{}",
            app_track_2.id
        ))
        .header(COOKIE, &cookie_reviewer)
        .send()
        .await
        .unwrap();
    // Reviewer is not assigned to track 2 and is not the applicant -> 403 Forbidden
    assert_eq!(app2_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Test Early Exit / Threshold-Oriented approval counting on acceptance
    // Application under track 1 requires final_approvals = 2
    let test_app = RecruitmentApplication {
        id: uuid::Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        track_id: track_1.id,
        applicant_user_id: candidate_id.to_string(),
        track_version: 1,
        status: "screening".to_string(),
        submitted_answers: sqlx::types::Json(vec![]),
        current_stage: "screening".to_string(),
        version: 1,
        submitted_at: Utc::now(),
        updated_at: Utc::now(),
        decided_at: None,
        decision_reason: None,
        decided_by: None,
    };
    web_state
        .app
        .storage
        .create_recruitment_application(test_app.clone())
        .await
        .unwrap();

    // Add 5 distinct reviews recommending 'approve'
    for i in 1..=5 {
        web_state
            .app
            .storage
            .upsert_recruitment_review(RecruitmentReview {
                id: uuid::Uuid::new_v4(),
                application_id: test_app.id,
                guild_id: guild_id.to_string(),
                reviewer_user_id: format!("33300033300033300{i}"),
                recommendation: "approve".to_string(),
                note: Some(format!("Approval note {i}")),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .unwrap();
    }

    // Reset member get count
    member_get_count.store(0, std::sync::atomic::Ordering::SeqCst);

    // Accept application: final_approvals is 2
    let admin_session = "admin_trans_sess";
    let admin_csrf = "admin_trans_csrf";
    web_state
        .app
        .sessions
        .save_session(
            admin_session,
            DashboardUser {
                id: owner_id.to_owned(),
                username: "owner".to_owned(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_owned(),
                name: "Guild N1".to_owned(),
                icon: None,
                owner: true,
                permissions: "8".to_owned(),
            }],
            admin_csrf.to_owned(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .unwrap();
    web_state.permissions_cache.insert(
        (owner_id.to_owned(), guild_id.to_owned()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );
    let cookie_admin = format!("rusty_discord_bot_session={admin_session}");

    let trans_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/applications/{}/transition",
            test_app.id
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "expected_version": 1,
            "new_status": "accepted",
            "new_stage": "accepted",
            "decision_reason": "Passed"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(trans_resp.status(), reqwest::StatusCode::OK);

    // CRITICAL ASSERTION: Since target is 2 approvals, count_eligible_approvals_with_target stops
    // after verifying 2 reviewers. It must NOT fetch all 5 reviewers!
    let m_trans_calls = member_get_count.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        m_trans_calls <= 2,
        "Acceptance must stop after satisfying final_approvals (<= 2 fetches), but performed {m_trans_calls}"
    );

    // Short-circuit test: When approved count < final_approvals, 0 Discord REST calls should occur
    let short_app = RecruitmentApplication {
        id: uuid::Uuid::new_v4(),
        guild_id: guild_id.to_string(),
        track_id: track_1.id,
        applicant_user_id: candidate_id.to_string(),
        track_version: 1,
        status: "screening".to_string(),
        submitted_answers: sqlx::types::Json(vec![]),
        current_stage: "screening".to_string(),
        version: 1,
        submitted_at: Utc::now(),
        updated_at: Utc::now(),
        decided_at: None,
        decision_reason: None,
        decided_by: None,
    };
    web_state
        .app
        .storage
        .create_recruitment_application(short_app.clone())
        .await
        .unwrap();
    // Only 1 approval for a track requiring 2
    web_state
        .app
        .storage
        .upsert_recruitment_review(RecruitmentReview {
            id: uuid::Uuid::new_v4(),
            application_id: short_app.id,
            guild_id: guild_id.to_string(),
            reviewer_user_id: "333000333000333001".to_string(),
            recommendation: "approve".to_string(),
            note: Some("Single approval".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();

    guild_get_count.store(0, std::sync::atomic::Ordering::SeqCst);
    member_get_count.store(0, std::sync::atomic::Ordering::SeqCst);

    let short_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/recruitment/applications/{}/transition",
            short_app.id
        ))
        .header(COOKIE, &cookie_admin)
        .header("x-csrf-token", admin_csrf)
        .json(&serde_json::json!({
            "expected_version": 1,
            "new_status": "accepted",
            "new_stage": "accepted",
            "decision_reason": "Premature accept"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(short_resp.status(), reqwest::StatusCode::BAD_REQUEST);
    // Short-circuit prevented ANY REST calls!
    assert_eq!(guild_get_count.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(
        member_get_count.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn test_recruitment_track_concurrency_and_slug_uniqueness_suite() {
    use rusty_discord_bot::recruitment::{RecruitmentTrack, RecruitmentTrackConfig};
    use rusty_discord_bot::storage::MemoryStore;
    use std::sync::Arc;
    use tokio::sync::Barrier;

    let store = Arc::new(MemoryStore::default());
    let guild_id = "123456789012345678";
    let slug = "helper-track";

    // 1. Concurrency: 20 parallel tasks trying to create a new track with the same (guild_id, slug) but different UUIDs
    let barrier = Arc::new(Barrier::new(20));
    let mut tasks = Vec::new();

    for _ in 0..20 {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        let track = RecruitmentTrack {
            id: uuid::Uuid::new_v4(),
            guild_id: guild_id.to_string(),
            slug: slug.to_string(),
            name: "Helper Track".to_string(),
            enabled: true,
            target_role_id: None,
            version: 1,
            config: sqlx::types::Json(RecruitmentTrackConfig::default()),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            store.upsert_recruitment_track(track)
        }));
    }

    let mut successes = 0;
    let mut failures = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(_) => successes += 1,
            Err(e) => {
                assert!(e.to_string().contains("already exists in this server"));
                failures += 1;
            }
        }
    }

    assert_eq!(
        successes, 1,
        "Exactly one parallel insert of the same slug must succeed"
    );
    assert_eq!(
        failures, 19,
        "Remaining 19 parallel attempts must be rejected with conflict"
    );

    let tracks = store.list_recruitment_tracks(guild_id).unwrap();
    assert_eq!(tracks.len(), 1, "Exactly 1 track must be stored");
    let single_track = tracks[0].clone();
    assert_eq!(single_track.slug, slug);

    // 2. Concurrency: 20 parallel updates of the SAME track ID -> version must increment monotonically
    let base_version = single_track.version;
    let update_barrier = Arc::new(Barrier::new(20));
    let mut update_tasks = Vec::new();

    for i in 0..20 {
        let store = Arc::clone(&store);
        let update_barrier = Arc::clone(&update_barrier);
        let mut track_to_update = single_track.clone();
        track_to_update.name = format!("Helper Track Update {i}");

        update_tasks.push(tokio::spawn(async move {
            update_barrier.wait().await;
            store.upsert_recruitment_track(track_to_update)
        }));
    }

    for task in update_tasks {
        assert!(
            task.await.unwrap().is_ok(),
            "All upserts of the same track ID must succeed"
        );
    }

    let final_track = store
        .get_recruitment_track(guild_id, single_track.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        final_track.version,
        base_version + 20,
        "Track version must be exactly incremented by 20 without lost updates"
    );

    // 3. Different guild can legally use the exact same slug
    let other_guild_id = "987654321098765432";
    let other_guild_track = RecruitmentTrack {
        id: uuid::Uuid::new_v4(),
        guild_id: other_guild_id.to_string(),
        slug: slug.to_string(),
        name: "Other Guild Helper".to_string(),
        enabled: true,
        target_role_id: None,
        version: 1,
        config: sqlx::types::Json(RecruitmentTrackConfig::default()),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let other_res = store.upsert_recruitment_track(other_guild_track);
    assert!(
        other_res.is_ok(),
        "Different guild with same slug must be allowed"
    );

    // 4. Cross-guild hijacking of an existing track ID is rejected
    let hijack_track = RecruitmentTrack {
        id: single_track.id,
        guild_id: other_guild_id.to_string(),
        slug: "hijack-slug".to_string(),
        name: "Hijack Track".to_string(),
        enabled: true,
        target_role_id: None,
        version: 1,
        config: sqlx::types::Json(RecruitmentTrackConfig::default()),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let hijack_res = store.upsert_recruitment_track(hijack_track);
    assert!(
        hijack_res.is_err(),
        "Cross-guild update with existing track ID must be rejected"
    );
    assert!(
        hijack_res
            .unwrap_err()
            .to_string()
            .contains("belongs to another guild")
    );
}

#[tokio::test]
async fn test_sticky_messages_validation_and_crud() {
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

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");

    let guild_id = "100000000000000001";
    let user_id = "200000000000000001";
    let session_id = "test_sticky_web_session";
    let csrf_token = "csrf_sticky_token";

    state
        .app
        .sessions
        .save_session(
            session_id,
            DashboardUser {
                id: user_id.to_string(),
                username: "StickyTester".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Sticky Guild".to_string(),
                icon: None,
                owner: true,
                permissions: "8".to_string(),
            }],
            csrf_token.to_string(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    perms_cache.insert(
        (user_id.to_string(), guild_id.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    // 1. Invalid snowflake channel_id -> 400 Bad Request
    let bad_chan_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/sticky"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "channel_id": "non_snowflake_chan",
            "content": "Valid content",
            "cooldown_seconds": 5
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_chan_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // 2. Cooldown <= 0 -> 400 Bad Request
    let zero_cd_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/sticky"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "channel_id": "111000000000000101",
            "content": "Valid content",
            "cooldown_seconds": 0
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(zero_cd_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    let neg_cd_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/sticky"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "channel_id": "111000000000000101",
            "content": "Valid content",
            "cooldown_seconds": -10
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(neg_cd_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // 3. Cooldown > 86400 -> 400 Bad Request
    let huge_cd_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/sticky"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "channel_id": "111000000000000101",
            "content": "Valid content",
            "cooldown_seconds": 100_000
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(huge_cd_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // 4. Empty content -> 400 Bad Request
    let empty_content_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/sticky"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "channel_id": "111000000000000101",
            "content": "   ",
            "cooldown_seconds": 5
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        empty_content_resp.status(),
        reqwest::StatusCode::BAD_REQUEST
    );

    // 5. Content > 2000 chars -> 400 Bad Request
    let long_content = "a".repeat(2001);
    let long_content_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/sticky"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "channel_id": "111000000000000101",
            "content": long_content,
            "cooldown_seconds": 5
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(long_content_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // 6. Valid sticky configuration -> 200 OK
    let valid_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/sticky"))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "channel_id": "111000000000000101",
            "content": "Proper Sticky Message",
            "cooldown_seconds": 15
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(valid_resp.status(), reqwest::StatusCode::OK);
    let valid_json: serde_json::Value = valid_resp.json().await.unwrap();
    assert_eq!(valid_json["channel_id"], "111000000000000101");
    assert_eq!(valid_json["content"], "Proper Sticky Message");
    assert_eq!(valid_json["cooldown_seconds"], 15);

    // 7. GET sticky -> 200 OK
    let get_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/sticky/111000000000000101"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_resp.status(), reqwest::StatusCode::OK);
    let get_json: serde_json::Value = get_resp.json().await.unwrap();
    assert_eq!(get_json["channel_id"], "111000000000000101");
    assert_eq!(get_json["content"], "Proper Sticky Message");

    // 8. DELETE sticky -> 200 OK
    let del_resp = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/sticky/111000000000000101"
        ))
        .header(COOKIE, &session_cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(del_resp.status(), reqwest::StatusCode::OK);
    let del_json: serde_json::Value = del_resp.json().await.unwrap();
    assert_eq!(del_json["deleted"], true);

    // 9. GET after DELETE returns null
    let get_after_del = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/sticky/111000000000000101"
        ))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_after_del.status(), reqwest::StatusCode::OK);
    let get_after_json: serde_json::Value = get_after_del.json().await.unwrap();
    assert!(get_after_json.is_null());
}

#[tokio::test]
async fn test_sticky_messages_view_authorization_and_isolation_issue_320() {
    use axum::{response::IntoResponse, routing::get};

    // 1. Build Discord mock router for online permission checks
    let discord_mock_app = axum::Router::new()
        .route(
            "/channels/{channel_id}",
            get(
                |axum::extract::Path(cid): axum::extract::Path<String>| async move {
                    match cid.as_str() {
                        "111000000000000101" => axum::Json(serde_json::json!({
                            "id": "111000000000000101",
                            "guild_id": "100000000000000001",
                            "type": 0
                        }))
                        .into_response(),
                        "111000000000000102" => axum::Json(serde_json::json!({
                            "id": "111000000000000102",
                            "guild_id": "100000000000000001",
                            "type": 0
                        }))
                        .into_response(),
                        "111000000000000103" => axum::Json(serde_json::json!({
                            "id": "111000000000000103",
                            "guild_id": "100000000000000001",
                            "type": 12
                        }))
                        .into_response(),
                        "222000000000000201" => axum::Json(serde_json::json!({
                            "id": "222000000000000201",
                            "guild_id": "888000000000000888",
                            "type": 0
                        }))
                        .into_response(),
                        "777000000000000777" => (
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
                |axum::extract::Path((channel_id, user_id)): axum::extract::Path<(
                    String,
                    String,
                )>| async move {
                    if channel_id == "111000000000000102" && user_id == "200000000000000001" {
                        // User 200000000000000001 is denied VIEW_CHANNEL in private channel 102
                        axum::Json(serde_json::json!({
                            "permissions": "0"
                        }))
                        .into_response()
                    } else if user_id == "200000000000000002" || user_id == "200000000000000001" {
                        // User has VIEW_CHANNEL (bit 1024)
                        axum::Json(serde_json::json!({
                            "permissions": "1024"
                        }))
                        .into_response()
                    } else {
                        (
                            reqwest::StatusCode::NOT_FOUND,
                            axum::Json(serde_json::json!({
                                "code": 10003,
                                "message": "Unknown Channel or Member"
                            })),
                        )
                            .into_response()
                    }
                },
            ),
        )
        .route(
            "/channels/{channel_id}/thread-members/{user_id}",
            get(
                |axum::extract::Path((_channel_id, user_id)): axum::extract::Path<(
                    String,
                    String,
                )>| async move {
                    if user_id == "200000000000000002" {
                        axum::Json(serde_json::json!({
                            "user_id": user_id,
                            "join_timestamp": "2026-11-08T07:00:00Z"
                        }))
                        .into_response()
                    } else {
                        (
                            reqwest::StatusCode::NOT_FOUND,
                            axum::Json(serde_json::json!({
                                "code": 10003,
                                "message": "Not a thread member"
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

    // 2. Configure app state in live mode pointing to discord mock
    let mut config = test_config();
    config.discord_token = "live_bot_token_production_secret".to_string();
    config.offline_mode = false;
    config.discord_api_base_url = format!("http://{discord_addr}");

    let app = AppState::bootstrap(config).await.expect("bootstrap");
    let permissions_cache = std::sync::Arc::new(dashmap::DashMap::new());
    let web_state = WebState::new(
        app,
        std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache.clone(),
    );
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

    let guild_id = "100000000000000001";
    let u_forbidden_id = "200000000000000001";
    let u_allowed_id = "200000000000000002";

    let session_forbidden = "session_u_forbidden";
    let csrf_forbidden = "csrf_forbidden";

    let session_allowed = "session_u_allowed";
    let csrf_allowed = "csrf_allowed";

    // Setup sessions with MANAGE_GUILD
    web_state
        .app
        .sessions
        .save_session(
            session_forbidden,
            DashboardUser {
                id: u_forbidden_id.to_string(),
                username: "UserWithoutViewChannel".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Guild Test".to_string(),
                icon: None,
                owner: false,
                permissions: "32".to_string(), // MANAGE_GUILD
            }],
            csrf_forbidden.to_string(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    web_state
        .app
        .sessions
        .save_session(
            session_allowed,
            DashboardUser {
                id: u_allowed_id.to_string(),
                username: "UserWithViewChannel".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Guild Test".to_string(),
                icon: None,
                owner: false,
                permissions: "32".to_string(), // MANAGE_GUILD
            }],
            csrf_allowed.to_string(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    permissions_cache.insert(
        (u_forbidden_id.to_string(), guild_id.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    permissions_cache.insert(
        (u_allowed_id.to_string(), guild_id.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    // Pre-populate sticky messages in storage
    web_state
        .app
        .storage
        .set_sticky_message(
            guild_id,
            "111000000000000102",
            "",
            "Confidential Staff Note",
            5,
        )
        .await
        .unwrap();

    web_state
        .app
        .storage
        .set_sticky_message(
            guild_id,
            "111000000000000103",
            "",
            "Confidential Thread Note",
            5,
        )
        .await
        .unwrap();

    let cookie_forbidden = format!("rusty_discord_bot_session={session_forbidden}");
    let cookie_allowed = format!("rusty_discord_bot_session={session_allowed}");

    // Test 1: U with MANAGE_GUILD but lacking VIEW_CHANNEL -> GET returns 403 Forbidden without content
    let get_forbidden = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/sticky/111000000000000102"
        ))
        .header(COOKIE, &cookie_forbidden)
        .send()
        .await
        .unwrap();
    assert_eq!(get_forbidden.status(), reqwest::StatusCode::FORBIDDEN);
    let body_text = get_forbidden.text().await.unwrap();
    assert!(!body_text.contains("Confidential Staff Note"));

    // Test 2: U with MANAGE_GUILD but lacking VIEW_CHANNEL -> DELETE returns 403 Forbidden and preserves DB record
    let del_forbidden = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/sticky/111000000000000102"
        ))
        .header(COOKIE, &cookie_forbidden)
        .header("x-csrf-token", csrf_forbidden)
        .send()
        .await
        .unwrap();
    assert_eq!(del_forbidden.status(), reqwest::StatusCode::FORBIDDEN);

    let preserved = web_state
        .app
        .storage
        .get_sticky_message(guild_id, "111000000000000102")
        .await
        .unwrap();
    assert!(
        preserved.is_some(),
        "Sticky record must not be deleted by unauthorized user"
    );

    // Test 3: U with VIEW_CHANNEL -> GET returns 200 OK with content
    let get_allowed = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/sticky/111000000000000102"
        ))
        .header(COOKIE, &cookie_allowed)
        .send()
        .await
        .unwrap();
    assert_eq!(get_allowed.status(), reqwest::StatusCode::OK);
    let allowed_json: serde_json::Value = get_allowed.json().await.unwrap();
    assert_eq!(allowed_json["content"], "Confidential Staff Note");

    // Test 4: U with VIEW_CHANNEL -> DELETE returns 200 OK and deletes record
    let del_allowed = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/sticky/111000000000000102"
        ))
        .header(COOKIE, &cookie_allowed)
        .header("x-csrf-token", csrf_allowed)
        .send()
        .await
        .unwrap();
    assert_eq!(del_allowed.status(), reqwest::StatusCode::OK);

    let after_delete = web_state
        .app
        .storage
        .get_sticky_message(guild_id, "111000000000000102")
        .await
        .unwrap();
    assert!(
        after_delete.is_none(),
        "Sticky record must be deleted by authorized user"
    );

    // Test 5: Private thread without membership -> both GET and DELETE return 403 Forbidden
    let thread_get_forbidden = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/sticky/111000000000000103"
        ))
        .header(COOKIE, &cookie_forbidden)
        .send()
        .await
        .unwrap();
    assert_eq!(
        thread_get_forbidden.status(),
        reqwest::StatusCode::FORBIDDEN
    );

    let thread_del_forbidden = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/sticky/111000000000000103"
        ))
        .header(COOKIE, &cookie_forbidden)
        .header("x-csrf-token", csrf_forbidden)
        .send()
        .await
        .unwrap();
    assert_eq!(
        thread_del_forbidden.status(),
        reqwest::StatusCode::FORBIDDEN
    );

    // Test 6: Channel belonging to another guild -> returns 403 Forbidden without mutation
    let foreign_get = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/sticky/222000000000000201"
        ))
        .header(COOKIE, &cookie_allowed)
        .send()
        .await
        .unwrap();
    assert_eq!(foreign_get.status(), reqwest::StatusCode::FORBIDDEN);

    let foreign_del = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/sticky/222000000000000201"
        ))
        .header(COOKIE, &cookie_allowed)
        .header("x-csrf-token", csrf_allowed)
        .send()
        .await
        .unwrap();
    assert_eq!(foreign_del.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 7: Nonexistent channel (404 from Discord API) -> fails closed with 403 Forbidden
    let nonexist_get = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/sticky/999000000000000999"
        ))
        .header(COOKIE, &cookie_allowed)
        .send()
        .await
        .unwrap();
    assert_eq!(nonexist_get.status(), reqwest::StatusCode::FORBIDDEN);

    // Test 8: Discord API 5xx error -> fails closed with 403 Forbidden
    let err_get = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/sticky/777000000000000777"
        ))
        .header(COOKIE, &cookie_allowed)
        .send()
        .await
        .unwrap();
    assert_eq!(err_get.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_scrub_snapshot_pii_comprehensive_issue_328() {
    use rusty_discord_bot::web::features::{is_sensitive_key, scrub_snapshot_pii};

    // 1. Verify key sensitivity classifier
    assert!(is_sensitive_key("token"));
    assert!(is_sensitive_key("Token"));
    assert!(is_sensitive_key("access_token"));
    assert!(is_sensitive_key("accessToken"));
    assert!(is_sensitive_key("refresh_token"));
    assert!(is_sensitive_key("refreshToken"));
    assert!(is_sensitive_key("client_secret"));
    assert!(is_sensitive_key("clientSecret"));
    assert!(is_sensitive_key("api_key"));
    assert!(is_sensitive_key("apiKey"));
    assert!(is_sensitive_key("webhookUrl"));
    assert!(is_sensitive_key("webhook_url"));
    assert!(is_sensitive_key("authorization"));
    assert!(is_sensitive_key("password"));
    assert!(is_sensitive_key("private_key"));
    assert!(is_sensitive_key("bot_secret"));
    assert!(is_sensitive_key("user_id"));
    assert!(is_sensitive_key("stripe_api_key"));
    assert!(is_sensitive_key("custom_token"));

    // Safe structure keys must not be classified as sensitive
    assert!(!is_sensitive_key("channels"));
    assert!(!is_sensitive_key("roles"));
    assert!(!is_sensitive_key("categories"));
    assert!(!is_sensitive_key("name"));
    assert!(!is_sensitive_key("type"));
    assert!(!is_sensitive_key("permissions"));
    assert!(!is_sensitive_key("color"));
    assert!(!is_sensitive_key("modules"));
    assert!(!is_sensitive_key("enabled"));

    // 2. Multi-level nested object/array scrubbing
    let mut payload = serde_json::json!({
        "name": "configuration-import",
        "channels": [
            {
                "name": "general",
                "type": 0,
                "token": "LEAKED_CHANNEL_TOKEN",
                "webhook_url": "https://discord.invalid/webhook/1"
            }
        ],
        "roles": [
            {
                "name": "Admin",
                "permissions": "8",
                "user_id": "123456789"
            }
        ],
        "modules": {
            "integrations": {
                "token": "REDACTED_BY_CURRENT_CODE",
                "access_token": "DEMO_ACCESS_NOT_REAL",
                "refresh_token": "DEMO_REFRESH_NOT_REAL",
                "client_secret": "DEMO_CLIENT_SECRET_NOT_REAL",
                "api_key": "DEMO_API_KEY_NOT_REAL",
                "webhookUrl": "https://example.invalid/demo",
                "Authorization": "Bearer secret-val",
                "nested_array": [
                    {
                        "Token": "UPPERCASE_TOKEN",
                        "safe_field": "keep_this"
                    }
                ]
            }
        }
    });

    scrub_snapshot_pii(&mut payload);

    let serialized = serde_json::to_string(&payload).unwrap();
    assert!(!serialized.contains("LEAKED_CHANNEL_TOKEN"));
    assert!(!serialized.contains("https://discord.invalid/webhook/1"));
    assert!(!serialized.contains("123456789"));
    assert!(!serialized.contains("REDACTED_BY_CURRENT_CODE"));
    assert!(!serialized.contains("DEMO_ACCESS_NOT_REAL"));
    assert!(!serialized.contains("DEMO_REFRESH_NOT_REAL"));
    assert!(!serialized.contains("DEMO_CLIENT_SECRET_NOT_REAL"));
    assert!(!serialized.contains("DEMO_API_KEY_NOT_REAL"));
    assert!(!serialized.contains("https://example.invalid/demo"));
    assert!(!serialized.contains("Bearer secret-val"));
    assert!(!serialized.contains("UPPERCASE_TOKEN"));

    // Safe fields must remain
    assert!(payload["channels"][0]["name"] == "general");
    assert!(payload["roles"][0]["name"] == "Admin");
    assert!(payload["modules"]["integrations"]["nested_array"][0]["safe_field"] == "keep_this");
}

#[tokio::test]
async fn test_snapshot_api_scrubbing_and_historic_record_isolation_issue_328() {
    let mut config = test_config();
    config.offline_mode = true;
    let app = AppState::bootstrap(config).await.expect("bootstrap");
    let permissions_cache = std::sync::Arc::new(dashmap::DashMap::new());
    let web_state = WebState::new(
        app,
        std::sync::Arc::new(LoginRateLimiter::new()),
        permissions_cache.clone(),
    );
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

    let guild_id = "555000000000000001";
    let foreign_guild_id = "666000000000000001";
    let user_a_id = "555000000000000002";
    let user_b_id = "555000000000000003";

    let session_a = "session_user_a";
    let csrf_a = "csrf_user_a";

    let session_b = "session_user_b";
    let csrf_b = "csrf_user_b";

    // Setup session A with MANAGE_GUILD on guild_id
    web_state
        .app
        .sessions
        .save_session(
            session_a,
            DashboardUser {
                id: user_a_id.to_string(),
                username: "UserA".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Guild 555".to_string(),
                icon: None,
                owner: false,
                permissions: "32".to_string(), // MANAGE_GUILD
            }],
            csrf_a.to_string(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    // Setup session B with MANAGE_GUILD on guild_id
    web_state
        .app
        .sessions
        .save_session(
            session_b,
            DashboardUser {
                id: user_b_id.to_string(),
                username: "UserB".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Guild 555".to_string(),
                icon: None,
                owner: false,
                permissions: "32".to_string(), // MANAGE_GUILD
            }],
            csrf_b.to_string(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    permissions_cache.insert(
        (user_a_id.to_string(), guild_id.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    permissions_cache.insert(
        (user_b_id.to_string(), guild_id.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let cookie_a = format!("rusty_discord_bot_session={session_a}");
    let cookie_b = format!("rusty_discord_bot_session={session_b}");

    // Test 1: User A creates snapshot containing sensitive tokens in snapshot_data
    let create_payload = serde_json::json!({
        "name": "Integration Snapshot With Secrets",
        "snapshot_data": {
            "channels": [{"name": "chat", "type": 0}],
            "roles": [{"name": "Member", "permissions": "1"}],
            "modules": {
                "integrations": {
                    "token": "BOT_SECRET_VAL",
                    "access_token": "ACCESS_SECRET_VAL",
                    "refresh_token": "REFRESH_SECRET_VAL",
                    "client_secret": "CLIENT_SECRET_VAL",
                    "api_key": "API_KEY_VAL",
                    "webhookUrl": "https://discord.invalid/webhook/secret",
                    "Authorization": "Bearer secret_header"
                }
            }
        }
    });

    let create_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/snapshots"))
        .header(COOKIE, &cookie_a)
        .header("x-csrf-token", csrf_a)
        .json(&create_payload)
        .send()
        .await
        .unwrap();

    assert_eq!(create_resp.status(), reqwest::StatusCode::OK);
    let created_json: serde_json::Value = create_resp.json().await.unwrap();
    let snapshot_id_str = created_json["id"].as_str().unwrap();
    let snapshot_id = uuid::Uuid::parse_str(snapshot_id_str).unwrap();

    // Verify response does not leak any secret
    let response_str = serde_json::to_string(&created_json).unwrap();
    assert!(!response_str.contains("ACCESS_SECRET_VAL"));
    assert!(!response_str.contains("CLIENT_SECRET_VAL"));
    assert!(!response_str.contains("API_KEY_VAL"));
    assert!(!response_str.contains("https://discord.invalid/webhook/secret"));
    assert!(!response_str.contains("Bearer secret_header"));

    // Verify storage does not store the secrets
    let stored = web_state
        .app
        .storage
        .get_server_snapshot(snapshot_id)
        .await
        .unwrap()
        .expect("stored snapshot");
    let stored_str = serde_json::to_string(&stored.snapshot_json).unwrap();
    assert!(!stored_str.contains("ACCESS_SECRET_VAL"));
    assert!(!stored_str.contains("CLIENT_SECRET_VAL"));
    assert!(!stored_str.contains("API_KEY_VAL"));

    // Test 2: User B reads snapshot detail via GET -> secrets are absent
    let get_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/snapshots/{snapshot_id}"
        ))
        .header(COOKIE, &cookie_b)
        .send()
        .await
        .unwrap();
    assert_eq!(get_resp.status(), reqwest::StatusCode::OK);
    let get_json: serde_json::Value = get_resp.json().await.unwrap();
    let get_str = serde_json::to_string(&get_json).unwrap();
    assert!(!get_str.contains("ACCESS_SECRET_VAL"));
    assert!(!get_str.contains("CLIENT_SECRET_VAL"));

    // Test 3: Historic record with unscrubbed secrets directly placed in storage
    // is scrubbed on read (GET /snapshots and GET /snapshots/{id})
    let historic_raw = serde_json::json!({
        "channels": [{"name": "archive", "type": 0}],
        "legacy_creds": {
            "access_token": "HISTORIC_ACCESS_TOKEN",
            "client_secret": "HISTORIC_CLIENT_SECRET",
            "apiKey": "HISTORIC_API_KEY"
        }
    });
    let historic_snap = web_state
        .app
        .storage
        .create_server_snapshot(
            guild_id,
            "Historic Unscrubbed Snapshot",
            historic_raw,
            user_a_id,
        )
        .await
        .unwrap();

    // Verify GET detail cleans historic record
    let historic_get = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/snapshots/{}",
            historic_snap.id
        ))
        .header(COOKIE, &cookie_a)
        .send()
        .await
        .unwrap();
    assert_eq!(historic_get.status(), reqwest::StatusCode::OK);
    let historic_json: serde_json::Value = historic_get.json().await.unwrap();
    let historic_str = serde_json::to_string(&historic_json).unwrap();
    assert!(!historic_str.contains("HISTORIC_ACCESS_TOKEN"));
    assert!(!historic_str.contains("HISTORIC_CLIENT_SECRET"));
    assert!(!historic_str.contains("HISTORIC_API_KEY"));
    assert!(historic_json["snapshot_json"]["channels"][0]["name"] == "archive");

    // Verify GET list cleans historic record
    let list_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/snapshots"))
        .header(COOKIE, &cookie_b)
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp.status(), reqwest::StatusCode::OK);
    let list_json: serde_json::Value = list_resp.json().await.unwrap();
    let list_str = serde_json::to_string(&list_json).unwrap();
    assert!(!list_str.contains("HISTORIC_ACCESS_TOKEN"));
    assert!(!list_str.contains("HISTORIC_CLIENT_SECRET"));

    // Test 4: Validation: empty name or non-object snapshot_data rejected with 400 Bad Request
    let empty_name_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/snapshots"))
        .header(COOKIE, &cookie_a)
        .header("x-csrf-token", csrf_a)
        .json(&serde_json::json!({
            "name": "   ",
            "snapshot_data": {}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(empty_name_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    let invalid_data_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/snapshots"))
        .header(COOKIE, &cookie_a)
        .header("x-csrf-token", csrf_a)
        .json(&serde_json::json!({
            "name": "Valid Name",
            "snapshot_data": "not a json object"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_data_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Test 5: Authorization: foreign guild access forbidden
    let foreign_get = client
        .get(format!(
            "{base_url}/api/guilds/{foreign_guild_id}/snapshots/{snapshot_id}"
        ))
        .header(COOKIE, &cookie_a)
        .send()
        .await
        .unwrap();
    assert_eq!(foreign_get.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_scheduled_event_chronological_window_and_rsvp_cutoff_issue_331() {
    let mut config = test_config();
    config.offline_mode = true;
    let app = AppState::bootstrap(config).await.expect("bootstrap");
    let permissions_cache = Arc::new(DashMap::new());
    let web_state = WebState::new(
        app,
        Arc::new(LoginRateLimiter::new()),
        permissions_cache.clone(),
    );
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

    let guild_id = "555000000000000001";
    let admin_user_id = "555000000000000002";
    let member_user_id = "555000000000000003";

    let admin_session = "admin_session_331";
    let admin_csrf = "admin_csrf_331";
    let member_session = "member_session_331";
    let member_csrf = "member_csrf_331";

    web_state
        .app
        .sessions
        .save_session(
            admin_session,
            DashboardUser {
                id: admin_user_id.to_string(),
                username: "EventAdmin".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Events Guild".to_string(),
                icon: None,
                owner: false,
                permissions: "32".to_string(), // MANAGE_GUILD
            }],
            admin_csrf.to_string(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .unwrap();

    web_state
        .app
        .sessions
        .save_session(
            member_session,
            DashboardUser {
                id: member_user_id.to_string(),
                username: "EventMember".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Events Guild".to_string(),
                icon: None,
                owner: false,
                permissions: "0".to_string(), // Regular member
            }],
            member_csrf.to_string(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .unwrap();

    let admin_cookie = format!("rusty_discord_bot_session={admin_session}");
    let member_cookie = format!("rusty_discord_bot_session={member_session}");

    // 1. Rejection of end_time < start_time with HTTP 400
    let start = Utc::now() + Duration::days(5);
    let before_start = start - Duration::hours(2);

    let resp_earlier = client
        .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "title": "Invalid End Earlier",
            "description": "Invalid time window",
            "start_time": start,
            "end_time": before_start,
            "timezone": "UTC"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_earlier.status(), reqwest::StatusCode::BAD_REQUEST);
    let err_earlier: serde_json::Value = resp_earlier.json().await.unwrap();
    assert!(
        err_earlier["error"]
            .as_str()
            .unwrap()
            .contains("end_time must be later than start_time"),
        "Must return explicit error message when end_time < start_time"
    );

    // 2. Rejection of end_time == start_time with HTTP 400
    let resp_equal = client
        .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "title": "Invalid End Equal",
            "description": "Zero duration",
            "start_time": start,
            "end_time": start,
            "timezone": "UTC"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_equal.status(), reqwest::StatusCode::BAD_REQUEST);

    // Storage level rejection for end_time <= start_time
    let mem_invalid = web_state
        .app
        .storage
        .create_scheduled_event(
            guild_id,
            "Direct Storage Invalid",
            "desc",
            start,
            Some(before_start),
            "UTC",
            None,
        )
        .await;
    assert!(
        mem_invalid.is_err(),
        "Storage::create_scheduled_event must reject end_time <= start_time"
    );

    // 3. Valid creation succeeds and produces compliant iCalendar export (DTEND > DTSTART)
    let valid_end = start + Duration::hours(3);
    let resp_valid = client
        .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "title": "Valid Event",
            "description": "Compliant window",
            "start_time": start,
            "end_time": valid_end,
            "timezone": "UTC"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_valid.status(), reqwest::StatusCode::OK);
    let valid_event: serde_json::Value = resp_valid.json().await.unwrap();
    let valid_event_id: uuid::Uuid = serde_json::from_value(valid_event["id"].clone()).unwrap();

    let ics_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{valid_event_id}/ics"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(ics_resp.status(), reqwest::StatusCode::OK);
    let ics_text = ics_resp.text().await.unwrap();
    assert!(ics_text.contains("DTSTART:"));
    assert!(ics_text.contains("DTEND:"));

    // 4. RSVP Cutoff: Event that has ended rejects RSVP with HTTP 409 Conflict
    let expired_start = Utc::now() - Duration::days(2);
    let expired_end = Utc::now() - Duration::days(1);
    let expired_event = web_state
        .app
        .storage
        .create_scheduled_event(
            guild_id,
            "Past Event",
            "Already finished",
            expired_start,
            Some(expired_end),
            "UTC",
            Some(10),
        )
        .await
        .unwrap();

    let join_expired_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{}/roster/join",
            expired_event.id
        ))
        .header(COOKIE, &member_cookie)
        .header("X-CSRF-Token", member_csrf)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        join_expired_resp.status(),
        reqwest::StatusCode::CONFLICT,
        "Expired event join must return 409 Conflict"
    );
    let join_err: serde_json::Value = join_expired_resp.json().await.unwrap();
    assert_eq!(join_err["error"], "Event registration is closed");

    // Zero roster entries created
    let roster_after = web_state
        .app
        .storage
        .get_event_rosters(expired_event.id)
        .await
        .unwrap();
    assert!(
        roster_after.is_empty(),
        "No roster entries allowed after cutoff"
    );

    // 5. 20 concurrent joins on expired event: ALL fail with 409 Conflict, 0 roster rows created
    let mut join_handles = Vec::new();
    for i in 0..20 {
        let client_clone = client.clone();
        let target_url = format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{}/roster/join",
            expired_event.id
        );
        let user_session = format!("concurrent_member_session_{i}");
        let user_csrf = format!("concurrent_member_csrf_{i}");
        let user_id = format!("concurrent_user_555_{i:02}");

        web_state
            .app
            .sessions
            .save_session(
                &user_session,
                DashboardUser {
                    id: user_id,
                    username: format!("Member{i}"),
                    global_name: None,
                    avatar: None,
                },
                vec![DashboardGuild {
                    id: guild_id.to_string(),
                    name: "Events Guild".to_string(),
                    icon: None,
                    owner: false,
                    permissions: "0".to_string(),
                }],
                user_csrf.clone(),
                Utc::now() + Duration::hours(2),
                None,
            )
            .await
            .unwrap();

        let cookie = format!("rusty_discord_bot_session={user_session}");
        join_handles.push(tokio::spawn(async move {
            client_clone
                .post(target_url)
                .header(COOKIE, cookie)
                .header("X-CSRF-Token", user_csrf)
                .json(&serde_json::json!({}))
                .send()
                .await
                .unwrap()
        }));
    }

    for h in join_handles {
        let res = h.await.unwrap();
        assert_eq!(res.status(), reqwest::StatusCode::CONFLICT);
    }

    let roster_after_concurrent = web_state
        .app
        .storage
        .get_event_rosters(expired_event.id)
        .await
        .unwrap();
    assert!(
        roster_after_concurrent.is_empty(),
        "All 20 concurrent joins must be rejected; 0 roster entries created"
    );

    // 6. Ongoing event allows late RSVP
    let ongoing_start = Utc::now() - Duration::minutes(15);
    let ongoing_end = Utc::now() + Duration::minutes(45);
    let ongoing_event = web_state
        .app
        .storage
        .create_scheduled_event(
            guild_id,
            "Ongoing Event",
            "Happening right now",
            ongoing_start,
            Some(ongoing_end),
            "UTC",
            Some(10),
        )
        .await
        .unwrap();

    let join_ongoing_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{}/roster/join",
            ongoing_event.id
        ))
        .header(COOKIE, &member_cookie)
        .header("X-CSRF-Token", member_csrf)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        join_ongoing_resp.status(),
        reqwest::StatusCode::OK,
        "Ongoing event allows join prior to end_time cutoff"
    );
}

#[tokio::test]
async fn test_lua_event_routing_auth_and_permissions() {
    let (web_state, _) = test_web_state().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let base_url = format!("http://{}", addr);
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

    let target_guild = "guild_allow";
    let admin_session_id = "admin_routing_session";
    let admin_csrf = "admin_routing_csrf";
    web_state
        .app
        .sessions
        .save_session(
            admin_session_id,
            DashboardUser {
                id: "1001".to_string(),
                username: "admin".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: target_guild.to_string(),
                name: "Test Guild".to_string(),
                icon: None,
                owner: false,
                permissions: "32".to_string(), // MANAGE_GUILD
            }],
            admin_csrf.to_string(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    let unauthorized_session_id = "unauth_routing_session";
    let unauth_csrf = "unauth_routing_csrf";
    web_state
        .app
        .sessions
        .save_session(
            unauthorized_session_id,
            DashboardUser {
                id: "1002".to_string(),
                username: "regular_user".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: "other_guild".to_string(),
                name: "Other Guild".to_string(),
                icon: None,
                owner: false,
                permissions: "0".to_string(),
            }],
            unauth_csrf.to_string(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    // 1. Missing session cookie -> 401 Unauthorized
    let res = client
        .get(format!(
            "{base_url}/api/guilds/{target_guild}/lua/event-routing?event=message_create"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 2. Session without access to target guild -> 403 Forbidden
    let res = client
        .get(format!(
            "{base_url}/api/guilds/{target_guild}/lua/event-routing?event=message_create"
        ))
        .header(
            COOKIE,
            format!("rusty_discord_bot_session={unauthorized_session_id}"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::FORBIDDEN);

    // 3. Invalid snowflake format on guild_id -> 400 Bad Request
    let res = client
        .get(format!(
            "{base_url}/api/guilds/invalid_snowflake!/lua/event-routing?event=message_create"
        ))
        .header(
            COOKIE,
            format!("rusty_discord_bot_session={admin_session_id}"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);

    // 4. Authorized admin with valid guild -> 200 OK
    let res = client
        .get(format!(
            "{base_url}/api/guilds/{target_guild}/lua/event-routing?event=message_create"
        ))
        .header(
            COOKIE,
            format!("rusty_discord_bot_session={admin_session_id}"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn test_lua_event_routing_validation_and_disclaimer() {
    let (web_state, _) = test_web_state().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let base_url = format!("http://{}", addr);
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

    let target_guild = "guild_allow";
    let admin_session_id = "admin_validation_session";
    web_state
        .app
        .sessions
        .save_session(
            admin_session_id,
            DashboardUser {
                id: "1001".to_string(),
                username: "admin".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: target_guild.to_string(),
                name: "Test Guild".to_string(),
                icon: None,
                owner: false,
                permissions: "32".to_string(),
            }],
            "csrf".to_string(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    let cookie = format!("rusty_discord_bot_session={admin_session_id}");

    // 1. Missing event query parameter -> 400 Bad Request
    let res = client
        .get(format!(
            "{base_url}/api/guilds/{target_guild}/lua/event-routing"
        ))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);

    // 2. Empty event query parameter -> 400 Bad Request
    let res = client
        .get(format!(
            "{base_url}/api/guilds/{target_guild}/lua/event-routing?event="
        ))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);

    // 3. Whitespace-only event -> 400 Bad Request
    let res = client
        .get(format!(
            "{base_url}/api/guilds/{target_guild}/lua/event-routing?event=%20%20"
        ))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);

    // 4. Invalid characters in event -> 400 Bad Request
    let res = client
        .get(format!(
            "{base_url}/api/guilds/{target_guild}/lua/event-routing?event=message_create;drop_table"
        ))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);

    // 5. Exceeding 64 characters in event name -> 400 Bad Request
    let long_event = "a".repeat(65);
    let res = client
        .get(format!(
            "{base_url}/api/guilds/{target_guild}/lua/event-routing?event={long_event}"
        ))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);

    // 6. Unknown well-formed event name -> 200 OK with empty list []
    let res = client
        .get(format!("{base_url}/api/guilds/{target_guild}/lua/event-routing?event=non_existent_gateway_event"))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let routes: Vec<rusty_discord_bot::lua::LuaEventRouteDecision> = res.json().await.unwrap();
    assert!(routes.is_empty(), "Unknown event must return empty list");

    // 7. Verify Disclaimer header on valid response
    let res = client
        .get(format!(
            "{base_url}/api/guilds/{target_guild}/lua/event-routing?event=message_create"
        ))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let disclaimer = res.headers().get("x-routing-disclaimer");
    assert_eq!(
        disclaimer.and_then(|v| v.to_str().ok()),
        Some("Eligible for dispatch - not a guarantee of execution")
    );
}

#[tokio::test]
async fn test_lua_event_routing_guild_isolation_and_manifest_defaults() {
    let (web_state, _) = test_web_state().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let base_url = format!("http://{}", addr);
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

    let guild_a = "guild_allow";
    let guild_b = "guild_web_1";

    let admin_session = "admin_isolation_session";
    web_state
        .app
        .sessions
        .save_session(
            admin_session,
            DashboardUser {
                id: "1001".to_string(),
                username: "admin".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![
                DashboardGuild {
                    id: guild_a.to_string(),
                    name: "Guild A".to_string(),
                    icon: None,
                    owner: false,
                    permissions: "32".to_string(),
                },
                DashboardGuild {
                    id: guild_b.to_string(),
                    name: "Guild B".to_string(),
                    icon: None,
                    owner: false,
                    permissions: "32".to_string(),
                },
            ],
            "csrf".to_string(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    let cookie = format!("rusty_discord_bot_session={admin_session}");

    // Set Guild A override: afk enabled = true
    web_state
        .app
        .storage
        .set_module_settings(
            guild_a,
            "afk",
            true,
            serde_json::json!({"auto_clear_on_message": true, "max_reason_length": 256, "afk_prefix": "[AFK] "}),
        )
        .await
        .unwrap();

    // Set Guild B override: afk enabled = false
    web_state
        .app
        .storage
        .set_module_settings(
            guild_b,
            "afk",
            false,
            serde_json::json!({"auto_clear_on_message": false, "max_reason_length": 100, "afk_prefix": "[BUSY] "}),
        )
        .await
        .unwrap();

    // 1. Query Guild A: afk must be enabled with guild_override
    let res_a = client
        .get(format!(
            "{base_url}/api/guilds/{guild_a}/lua/event-routing?event=message_create"
        ))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res_a.status(), reqwest::StatusCode::OK);
    let routes_a: Vec<rusty_discord_bot::lua::LuaEventRouteDecision> = res_a.json().await.unwrap();
    let afk_a = routes_a
        .iter()
        .find(|r| r.module_id == "afk")
        .expect("afk module route");
    assert!(afk_a.subscribed);
    assert!(afk_a.configured_enabled);
    assert_eq!(afk_a.config_source, "guild_override");
    assert_eq!(afk_a.decision_reason, "enabled");

    // 2. Query Guild B: afk must be disabled with guild_override
    let res_b = client
        .get(format!(
            "{base_url}/api/guilds/{guild_b}/lua/event-routing?event=message_create"
        ))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res_b.status(), reqwest::StatusCode::OK);
    let routes_b: Vec<rusty_discord_bot::lua::LuaEventRouteDecision> = res_b.json().await.unwrap();
    let afk_b = routes_b
        .iter()
        .find(|r| r.module_id == "afk")
        .expect("afk module route");
    assert!(afk_b.subscribed);
    assert!(!afk_b.configured_enabled);
    assert_eq!(afk_b.config_source, "guild_override");
    assert_eq!(afk_b.decision_reason, "guild_disabled");

    // 3. Check unconfigured module in Guild A: automod has manifest default_enabled = false
    if let Some(automod_a) = routes_a.iter().find(|r| r.module_id == "automod") {
        assert!(automod_a.subscribed);
        assert!(!automod_a.configured_enabled);
        assert_eq!(automod_a.config_source, "manifest_default");
        assert_eq!(automod_a.decision_reason, "default_disabled");
    }

    // 4. Update Guild A's automod to enabled -> verify immediate reflection
    web_state
        .app
        .storage
        .set_module_settings(guild_a, "automod", true, serde_json::json!({}))
        .await
        .unwrap();

    let res_a_updated = client
        .get(format!(
            "{base_url}/api/guilds/{guild_a}/lua/event-routing?event=message_create"
        ))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    let routes_a_updated: Vec<rusty_discord_bot::lua::LuaEventRouteDecision> =
        res_a_updated.json().await.unwrap();
    let automod_a_updated = routes_a_updated
        .iter()
        .find(|r| r.module_id == "automod")
        .expect("automod route");
    assert!(automod_a_updated.configured_enabled);
    assert_eq!(automod_a_updated.config_source, "guild_override");
    assert_eq!(automod_a_updated.decision_reason, "enabled");

    // 5. Verify no unsubscribed modules: e.g. economy or tags
    assert!(!routes_a.iter().any(|r| r.module_id == "economy"));

    // 6. Verify zero secrets/config returned in JSON payload
    let raw_val: serde_json::Value = client
        .get(format!(
            "{base_url}/api/guilds/{guild_a}/lua/event-routing?event=message_create"
        ))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    for entry in raw_val.as_array().unwrap() {
        assert!(entry.get("config").is_none(), "Must never return config");
        assert!(entry.get("token").is_none(), "Must never return token");
        assert!(entry.get("secret").is_none(), "Must never return secret");
        assert_eq!(
            entry.as_object().unwrap().len(),
            5,
            "Exclusively 5 fields in decision"
        );
    }
}

#[tokio::test]
async fn test_lua_event_routing_pagination_and_rate_limiting() {
    let (web_state, _) = test_web_state().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let base_url = format!("http://{}", addr);
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

    let target_guild = "guild_allow";
    let admin_session = "admin_pagination_session";
    web_state
        .app
        .sessions
        .save_session(
            admin_session,
            DashboardUser {
                id: "1001".to_string(),
                username: "admin".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: target_guild.to_string(),
                name: "Test Guild".to_string(),
                icon: None,
                owner: false,
                permissions: "32".to_string(),
            }],
            "csrf".to_string(),
            Utc::now() + Duration::hours(1),
            None,
        )
        .await
        .unwrap();

    let cookie = format!("rusty_discord_bot_session={admin_session}");

    // All routes for message_create
    let res_all = client
        .get(format!(
            "{base_url}/api/guilds/{target_guild}/lua/event-routing?event=message_create"
        ))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res_all.status(), reqwest::StatusCode::OK);
    let all_routes: Vec<rusty_discord_bot::lua::LuaEventRouteDecision> =
        res_all.json().await.unwrap();
    assert!(
        all_routes.len() >= 2,
        "message_create has at least afk, automod, progression"
    );

    // Pagination: limit=1, offset=0
    let res_p1 = client
        .get(format!("{base_url}/api/guilds/{target_guild}/lua/event-routing?event=message_create&limit=1&offset=0"))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res_p1.status(), reqwest::StatusCode::OK);
    let p1_routes: Vec<rusty_discord_bot::lua::LuaEventRouteDecision> =
        res_p1.json().await.unwrap();
    assert_eq!(p1_routes.len(), 1);
    assert_eq!(p1_routes[0].module_id, all_routes[0].module_id);

    // Pagination: limit=1, offset=1
    let res_p2 = client
        .get(format!("{base_url}/api/guilds/{target_guild}/lua/event-routing?event=message_create&limit=1&offset=1"))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res_p2.status(), reqwest::StatusCode::OK);
    let p2_routes: Vec<rusty_discord_bot::lua::LuaEventRouteDecision> =
        res_p2.json().await.unwrap();
    assert_eq!(p2_routes.len(), 1);
    assert_eq!(p2_routes[0].module_id, all_routes[1].module_id);

    // Rate limiting: configure quota of 3 requests per 60s for admin:lua_event_routing
    web_state.app.rate_limiter.set_quota(
        "admin:lua_event_routing",
        rusty_discord_bot::rate_limit::RateLimitQuota::new(3, 60),
    );

    // 3 rapid requests consume quota
    for _ in 0..3 {
        let _ = client
            .get(format!(
                "{base_url}/api/guilds/{target_guild}/lua/event-routing?event=message_create"
            ))
            .header(COOKIE, &cookie)
            .send()
            .await
            .unwrap();
    }

    // 4th request must be 429 Too Many Requests
    let res_throttled = client
        .get(format!(
            "{base_url}/api/guilds/{target_guild}/lua/event-routing?event=message_create"
        ))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(
        res_throttled.status(),
        reqwest::StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn test_lua_event_routing_parity_and_side_effects() {
    let (web_state, _) = test_web_state().await;

    // Unit parity test across plan_event_routing states
    let guild_id = "guild_parity_test";

    // 1. Configured enabled
    web_state
        .app
        .storage
        .set_module_settings(guild_id, "afk", true, serde_json::json!({}))
        .await
        .unwrap();

    // 2. Configured disabled
    web_state
        .app
        .storage
        .set_module_settings(guild_id, "automod", false, serde_json::json!({}))
        .await
        .unwrap();

    let plans = rusty_discord_bot::lua::plan_event_routing(
        &web_state.app.scripts,
        &web_state.app.storage,
        Some(guild_id),
        "message_create",
    )
    .await;

    let afk_plan = plans.iter().find(|p| p.module_id == "afk").unwrap();
    assert!(afk_plan.decision.configured_enabled);
    assert_eq!(afk_plan.decision.config_source, "guild_override");
    assert_eq!(afk_plan.decision.decision_reason, "enabled");
    assert!(afk_plan.config.is_some());

    let automod_plan = plans.iter().find(|p| p.module_id == "automod").unwrap();
    assert!(!automod_plan.decision.configured_enabled);
    assert_eq!(automod_plan.decision.config_source, "guild_override");
    assert_eq!(automod_plan.decision.decision_reason, "guild_disabled");
    assert!(automod_plan.config.is_none());

    // 3. Unconfigured module uses manifest default
    let prog_plan = plans.iter().find(|p| p.module_id == "progression").unwrap();
    assert!(!prog_plan.decision.configured_enabled);
    assert_eq!(prog_plan.decision.config_source, "manifest_default");
    assert_eq!(prog_plan.decision.decision_reason, "default_disabled");

    // 3b. Unconfigured guild for afk: manifest default_enabled = true
    let default_plans = rusty_discord_bot::lua::plan_event_routing(
        &web_state.app.scripts,
        &web_state.app.storage,
        Some("unconfigured_guild_999"),
        "message_create",
    )
    .await;
    let default_afk = default_plans.iter().find(|p| p.module_id == "afk").unwrap();
    assert!(default_afk.decision.configured_enabled);
    assert_eq!(default_afk.decision.config_source, "manifest_default");
    assert_eq!(default_afk.decision.decision_reason, "enabled");

    // 4. Verify zero side-effects: calling plan_event_routing produces no metrics increment on executions
    let initial_lua_cmds = web_state
        .app
        .metrics
        .lua_commands_total
        .load(std::sync::atomic::Ordering::Relaxed);
    let initial_events = web_state
        .app
        .metrics
        .discord_events_total
        .load(std::sync::atomic::Ordering::Relaxed);

    let _ = rusty_discord_bot::lua::plan_event_routing(
        &web_state.app.scripts,
        &web_state.app.storage,
        Some(guild_id),
        "message_create",
    )
    .await;

    assert_eq!(
        web_state
            .app
            .metrics
            .lua_commands_total
            .load(std::sync::atomic::Ordering::Relaxed),
        initial_lua_cmds
    );
    assert_eq!(
        web_state
            .app
            .metrics
            .discord_events_total
            .load(std::sync::atomic::Ordering::Relaxed),
        initial_events
    );
}

#[tokio::test]
async fn test_guild_cases_pagination_and_no_500_issue_367() {
    let (web_state, _) = test_web_state().await;
    let router = build_router(web_state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let guild_id = "123456789012345678";
    let other_guild_id = "987654321098765432";
    let guild_u64 = guild_id.parse::<u64>().unwrap();
    let other_guild_u64 = other_guild_id.parse::<u64>().unwrap();

    let session_id = "session_issue_367";
    let session = rusty_discord_bot::session::Session {
        user: DashboardUser {
            id: "100200300".to_string(),
            username: "mod_tester".to_string(),
            avatar: None,
            global_name: None,
        },
        guilds: vec![DashboardGuild {
            id: guild_id.to_string(),
            name: "Mod Guild".to_string(),
            icon: None,
            owner: true,
            permissions: "8".to_string(), // ADMINISTRATOR
        }],
        expires_at: Utc::now() + Duration::hours(2),
        access_token: None,
        csrf_token: "csrf_367".to_string(),
    };
    web_state
        .app
        .sessions
        .save_session(
            session_id,
            session.user,
            session.guilds,
            session.csrf_token,
            session.expires_at,
            None,
        )
        .await
        .unwrap();
    let cookie = format!("rusty_discord_bot_session={session_id}");

    // 1. Empty guild test: MUST return 200 OK (previously returned 500 Internal Server Error)
    let res_empty = client
        .get(format!("{base_url}/api/guilds/{guild_id}/cases"))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res_empty.status(), reqwest::StatusCode::OK);
    let body_empty: serde_json::Value = res_empty.json().await.unwrap();
    assert_eq!(body_empty["total"], 0);
    assert_eq!(body_empty["items"].as_array().unwrap().len(), 0);
    assert!(body_empty["next_offset"].is_null());

    // 2. Populate cases in guild_id (105 cases) and 5 cases in other_guild_id
    let now = Utc::now();
    for i in 1..=105 {
        web_state
            .app
            .moderation_cases
            .create_case(
                rusty_discord_bot::moderation_cases::CreateCase {
                    guild_id: guild_u64,
                    subject_user_id: 2000 + (i as u64 % 10),
                    actor_user_id: 100200300,
                    kind: rusty_discord_bot::moderation_cases::CaseKind::Warning,
                    reason: format!("Test infraction #{i}"),
                    severity: rusty_discord_bot::moderation_cases::CaseSeverity::Low,
                    points: 10,
                    source_module: "automod".to_string(),
                    visible_to_subject: true,
                    expires_at: None,
                },
                now,
            )
            .await
            .expect("create case");
    }

    for i in 1..=5 {
        web_state
            .app
            .moderation_cases
            .create_case(
                rusty_discord_bot::moderation_cases::CreateCase {
                    guild_id: other_guild_u64,
                    subject_user_id: 3000 + (i as u64),
                    actor_user_id: 100200300,
                    kind: rusty_discord_bot::moderation_cases::CaseKind::Timeout,
                    reason: format!("Other guild infraction #{i}"),
                    severity: rusty_discord_bot::moderation_cases::CaseSeverity::Medium,
                    points: 20,
                    source_module: "manual".to_string(),
                    visible_to_subject: true,
                    expires_at: None,
                },
                now,
            )
            .await
            .expect("create other guild case");
    }

    // 3. Test page 1: default limit (50), offset=0
    let res_page1 = client
        .get(format!("{base_url}/api/guilds/{guild_id}/cases"))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res_page1.status(), reqwest::StatusCode::OK);
    let body_page1: serde_json::Value = res_page1.json().await.unwrap();
    assert_eq!(body_page1["total"], 105);
    let items_p1 = body_page1["items"].as_array().unwrap();
    assert_eq!(items_p1.len(), 50);
    assert_eq!(body_page1["next_offset"], 50);
    // Cases must be ordered DESC by case_number
    assert_eq!(items_p1[0]["case_number"], 105);
    assert_eq!(items_p1[49]["case_number"], 56);

    // 4. Test page 2: limit=50, offset=50
    let res_page2 = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/cases?limit=50&offset=50"
        ))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res_page2.status(), reqwest::StatusCode::OK);
    let body_page2: serde_json::Value = res_page2.json().await.unwrap();
    assert_eq!(body_page2["total"], 105);
    let items_p2 = body_page2["items"].as_array().unwrap();
    assert_eq!(items_p2.len(), 50);
    assert_eq!(body_page2["next_offset"], 100);
    assert_eq!(items_p2[0]["case_number"], 55);
    assert_eq!(items_p2[49]["case_number"], 6);

    // 5. Test page 3 (last page): limit=50, offset=100
    let res_page3 = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/cases?limit=50&offset=100"
        ))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res_page3.status(), reqwest::StatusCode::OK);
    let body_page3: serde_json::Value = res_page3.json().await.unwrap();
    assert_eq!(body_page3["total"], 105);
    let items_p3 = body_page3["items"].as_array().unwrap();
    assert_eq!(items_p3.len(), 5);
    assert!(
        body_page3["next_offset"].is_null(),
        "Last page must have next_offset = null"
    );
    assert_eq!(items_p3[0]["case_number"], 5);
    assert_eq!(items_p3[4]["case_number"], 1);

    // 6. Test offset out of bounds: offset=200
    let res_oob = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/cases?limit=50&offset=200"
        ))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res_oob.status(), reqwest::StatusCode::OK);
    let body_oob: serde_json::Value = res_oob.json().await.unwrap();
    assert_eq!(body_oob["total"], 105);
    assert_eq!(body_oob["items"].as_array().unwrap().len(), 0);
    assert!(body_oob["next_offset"].is_null());

    // 7. Tenant isolation: unauthorized guild returns 403 Forbidden
    let res_forbidden = client
        .get(format!("{base_url}/api/guilds/{other_guild_id}/cases"))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(res_forbidden.status(), reqwest::StatusCode::FORBIDDEN);

    // 8. Missing auth returns 401 Unauthorized
    let res_unauth = client
        .get(format!("{base_url}/api/guilds/{guild_id}/cases"))
        .send()
        .await
        .unwrap();
    assert_eq!(res_unauth.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_module_draft_resource_limits_and_validation_issue_375() {
    let (web_state, _) = test_web_state().await;
    let router = build_router(web_state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let guild_id = "111222333444555666";
    let other_guild_id = "999888777666555444";

    let session_id = "session_issue_375";
    let csrf_token = "csrf_issue_375";
    let session = rusty_discord_bot::session::Session {
        user: DashboardUser {
            id: "user_issue_375".to_string(),
            username: "admin_tester".to_string(),
            avatar: None,
            global_name: None,
        },
        guilds: vec![DashboardGuild {
            id: guild_id.to_string(),
            name: "Draft Admin Guild".to_string(),
            icon: None,
            owner: true,
            permissions: "8".to_string(), // ADMINISTRATOR
        }],
        expires_at: Utc::now() + Duration::hours(2),
        access_token: None,
        csrf_token: csrf_token.to_string(),
    };
    web_state
        .app
        .sessions
        .save_session(
            session_id,
            session.user,
            session.guilds,
            session.csrf_token,
            session.expires_at,
            None,
        )
        .await
        .unwrap();
    let cookie = format!("rusty_discord_bot_session={session_id}");

    // Session for other user in other guild
    let other_session_id = "session_issue_375_other";
    let other_session = rusty_discord_bot::session::Session {
        user: DashboardUser {
            id: "user_issue_375_other".to_string(),
            username: "other_tester".to_string(),
            avatar: None,
            global_name: None,
        },
        guilds: vec![DashboardGuild {
            id: other_guild_id.to_string(),
            name: "Other Guild".to_string(),
            icon: None,
            owner: true,
            permissions: "8".to_string(),
        }],
        expires_at: Utc::now() + Duration::hours(2),
        access_token: None,
        csrf_token: "csrf_other".to_string(),
    };
    web_state
        .app
        .sessions
        .save_session(
            other_session_id,
            other_session.user,
            other_session.guilds,
            other_session.csrf_token,
            other_session.expires_at,
            None,
        )
        .await
        .unwrap();
    let other_cookie = format!("rusty_discord_bot_session={other_session_id}");

    // AC1: POST for non-existent module_id returns 404 and does not create draft in storage
    let non_existent_payload = serde_json::json!({
        "config": { "foo": "bar" }
    });
    let post_non_existent = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/modules/nonexistent_mod_xyz/draft"
        ))
        .header(COOKIE, &cookie)
        .header("x-csrf-token", csrf_token)
        .json(&non_existent_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(post_non_existent.status(), reqwest::StatusCode::NOT_FOUND);

    let draft_in_store = web_state
        .app
        .storage
        .get_module_draft(guild_id, "nonexistent_mod_xyz")
        .await
        .unwrap();
    assert!(
        draft_in_store.is_none(),
        "No draft must be created in storage for non-existent module"
    );

    // GET / DELETE / PUBLISH for non-existent module also return 404
    let get_non_existent = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/modules/nonexistent_mod_xyz/draft"
        ))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_non_existent.status(), reqwest::StatusCode::NOT_FOUND);

    let delete_non_existent = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/modules/nonexistent_mod_xyz/draft"
        ))
        .header(COOKIE, &cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(delete_non_existent.status(), reqwest::StatusCode::NOT_FOUND);

    let publish_non_existent = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/modules/nonexistent_mod_xyz/publish"
        ))
        .header(COOKIE, &cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        publish_non_existent.status(),
        reqwest::StatusCode::NOT_FOUND
    );

    // AC2: Non-object config is rejected with 400 Bad Request
    let non_obj_payload = serde_json::json!({
        "config": ["array", "not", "object"]
    });
    let post_non_obj = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/modules/automod/draft"
        ))
        .header(COOKIE, &cookie)
        .header("x-csrf-token", csrf_token)
        .json(&non_obj_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(post_non_obj.status(), reqwest::StatusCode::BAD_REQUEST);

    // AC2: Config > 64 KiB is rejected with 400 Bad Request
    let huge_str = "x".repeat(65 * 1024);
    let oversized_payload = serde_json::json!({
        "config": {
            "large_field": huge_str
        }
    });
    let post_oversized = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/modules/automod/draft"
        ))
        .header(COOKIE, &cookie)
        .header("x-csrf-token", csrf_token)
        .json(&oversized_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(post_oversized.status(), reqwest::StatusCode::BAD_REQUEST);

    // AC2: Valid draft <= 64 KiB is accepted (200 OK) and readable
    let valid_payload = serde_json::json!({
        "config": {
            "max_mentions": 5,
            "anti_spam": true
        }
    });
    let post_valid = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/modules/automod/draft"
        ))
        .header(COOKIE, &cookie)
        .header("x-csrf-token", csrf_token)
        .json(&valid_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(post_valid.status(), reqwest::StatusCode::OK);

    let get_valid = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/modules/automod/draft"
        ))
        .header(COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_valid.status(), reqwest::StatusCode::OK);
    let draft_body: serde_json::Value = get_valid.json().await.unwrap();
    assert_eq!(draft_body["has_draft"], true);
    assert_eq!(draft_body["config"]["max_mentions"], 5);

    // AC4: Tenant isolation - user from other_guild cannot access guild_id draft
    let get_cross_tenant = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/modules/automod/draft"
        ))
        .header(COOKIE, &other_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_cross_tenant.status(), reqwest::StatusCode::FORBIDDEN);

    let post_cross_tenant = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/modules/automod/draft"
        ))
        .header(COOKIE, &other_cookie)
        .header("x-csrf-token", "csrf_other")
        .json(&valid_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(post_cross_tenant.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_sse_eventsource_connection_limits_and_isolation_issue_376() {
    let (mut web_state, _) = test_web_state().await;
    // Set max 2 per user/guild for testing
    let sse_quota = Arc::new(rusty_discord_bot::web::SseQuota::new(2));
    web_state.sse_quota = sse_quota.clone();
    let global_slots = Arc::new(tokio::sync::Semaphore::new(3));
    web_state.sse_global_slots = global_slots.clone();

    let router = build_router(web_state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let guild_a = "123000000000000001";
    let guild_b = "123000000000000002";
    let user_1 = "user_sse_1";
    let user_2 = "user_sse_2";

    // Setup session for user_1 with access to guild_a and guild_b
    let session_1_id = "session_sse_user_1";
    let session_1 = rusty_discord_bot::session::Session {
        user: DashboardUser {
            id: user_1.to_string(),
            username: "sse_user_1".to_string(),
            avatar: None,
            global_name: None,
        },
        guilds: vec![
            DashboardGuild {
                id: guild_a.to_string(),
                name: "Guild A".to_string(),
                icon: None,
                owner: true,
                permissions: "8".to_string(),
            },
            DashboardGuild {
                id: guild_b.to_string(),
                name: "Guild B".to_string(),
                icon: None,
                owner: true,
                permissions: "8".to_string(),
            },
        ],
        expires_at: Utc::now() + Duration::hours(2),
        access_token: None,
        csrf_token: "csrf_sse_1".to_string(),
    };
    web_state
        .app
        .sessions
        .save_session(
            session_1_id,
            session_1.user,
            session_1.guilds,
            session_1.csrf_token,
            session_1.expires_at,
            None,
        )
        .await
        .unwrap();
    let cookie_1 = format!("rusty_discord_bot_session={session_1_id}");

    // Setup session for user_2 with access to guild_a
    let session_2_id = "session_sse_user_2";
    let session_2 = rusty_discord_bot::session::Session {
        user: DashboardUser {
            id: user_2.to_string(),
            username: "sse_user_2".to_string(),
            avatar: None,
            global_name: None,
        },
        guilds: vec![DashboardGuild {
            id: guild_a.to_string(),
            name: "Guild A".to_string(),
            icon: None,
            owner: true,
            permissions: "8".to_string(),
        }],
        expires_at: Utc::now() + Duration::hours(2),
        access_token: None,
        csrf_token: "csrf_sse_2".to_string(),
    };
    web_state
        .app
        .sessions
        .save_session(
            session_2_id,
            session_2.user,
            session_2.guilds,
            session_2.csrf_token,
            session_2.expires_at,
            None,
        )
        .await
        .unwrap();
    let cookie_2 = format!("rusty_discord_bot_session={session_2_id}");

    // 1. User 1 connects stream 1 on guild_a -> 200 OK
    let resp_1 = client
        .get(format!("{base_url}/api/guilds/{guild_a}/events"))
        .header(COOKIE, &cookie_1)
        .send()
        .await
        .unwrap();
    assert_eq!(resp_1.status(), reqwest::StatusCode::OK);
    assert_eq!(sse_quota.active_connections(user_1, guild_a), 1);

    // 2. User 1 connects stream 2 on guild_a -> 200 OK (reaches max limit 2)
    let resp_2 = client
        .get(format!("{base_url}/api/guilds/{guild_a}/events"))
        .header(COOKIE, &cookie_1)
        .send()
        .await
        .unwrap();
    assert_eq!(resp_2.status(), reqwest::StatusCode::OK);
    assert_eq!(sse_quota.active_connections(user_1, guild_a), 2);

    let initial_receivers = web_state.app.guild_events.receiver_count();

    // 3. User 1 attempts 3rd stream on guild_a -> 429 Too Many Requests
    let resp_excess = client
        .get(format!("{base_url}/api/guilds/{guild_a}/events"))
        .header(COOKIE, &cookie_1)
        .send()
        .await
        .unwrap();
    assert_eq!(resp_excess.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
    // Crucial: receiver count must NOT have increased!
    assert_eq!(
        web_state.app.guild_events.receiver_count(),
        initial_receivers
    );
    assert_eq!(sse_quota.active_connections(user_1, guild_a), 2);

    // 4. User 2 connects to guild_a -> independent quota, succeeds with 200 OK!
    let resp_user_2 = client
        .get(format!("{base_url}/api/guilds/{guild_a}/events"))
        .header(COOKIE, &cookie_2)
        .send()
        .await
        .unwrap();
    assert_eq!(resp_user_2.status(), reqwest::StatusCode::OK);
    assert_eq!(sse_quota.active_connections(user_2, guild_a), 1);

    // 5. Global slot limit reached: 3 streams active (resp_1, resp_2, resp_user_2) out of 3 total.
    // Any new stream (even for User 1 on guild_b) is rejected by global slots -> 429 Too Many Requests
    let resp_global_full = client
        .get(format!("{base_url}/api/guilds/{guild_b}/events"))
        .header(COOKIE, &cookie_1)
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp_global_full.status(),
        reqwest::StatusCode::TOO_MANY_REQUESTS
    );

    // 6. Dropping a stream frees the slot promptly
    drop(resp_1);
    // Give async task a brief moment to detect disconnect and drop RAII permit
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(sse_quota.active_connections(user_1, guild_a), 1);

    // Now User 1 can connect another stream on guild_a
    let resp_new = client
        .get(format!("{base_url}/api/guilds/{guild_a}/events"))
        .header(COOKIE, &cookie_1)
        .send()
        .await
        .unwrap();
    assert_eq!(resp_new.status(), reqwest::StatusCode::OK);
    assert_eq!(sse_quota.active_connections(user_1, guild_a), 2);

    // Drop all remaining streams and verify zero active entries left in map
    drop(resp_2);
    drop(resp_user_2);
    drop(resp_new);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(sse_quota.active_connections(user_1, guild_a), 0);
    assert_eq!(sse_quota.active_connections(user_2, guild_a), 0);
    assert_eq!(
        sse_quota.total_tracked_entries(),
        0,
        "Zero-count entries must be cleaned from memory"
    );
}

#[tokio::test]
async fn test_ai_knowledge_doc_validation_quota_and_deletion_issue_377() {
    let (web_state, _) = test_web_state().await;
    let router = build_router(web_state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let guild_id = "555111222333444555";
    let other_guild_id = "555999888777666555";

    let session_id = "session_issue_377";
    let csrf_token = "csrf_issue_377";
    let session = rusty_discord_bot::session::Session {
        user: DashboardUser {
            id: "user_issue_377".to_string(),
            username: "ai_admin".to_string(),
            avatar: None,
            global_name: None,
        },
        guilds: vec![DashboardGuild {
            id: guild_id.to_string(),
            name: "AI Guild".to_string(),
            icon: None,
            owner: true,
            permissions: "8".to_string(),
        }],
        expires_at: Utc::now() + Duration::hours(2),
        access_token: None,
        csrf_token: csrf_token.to_string(),
    };
    web_state
        .app
        .sessions
        .save_session(
            session_id,
            session.user,
            session.guilds,
            session.csrf_token,
            session.expires_at,
            None,
        )
        .await
        .unwrap();
    let cookie = format!("rusty_discord_bot_session={session_id}");

    // Session for other user in other guild
    let other_session_id = "session_issue_377_other";
    let other_session = rusty_discord_bot::session::Session {
        user: DashboardUser {
            id: "user_issue_377_other".to_string(),
            username: "other_admin".to_string(),
            avatar: None,
            global_name: None,
        },
        guilds: vec![DashboardGuild {
            id: other_guild_id.to_string(),
            name: "Other AI Guild".to_string(),
            icon: None,
            owner: true,
            permissions: "8".to_string(),
        }],
        expires_at: Utc::now() + Duration::hours(2),
        access_token: None,
        csrf_token: "csrf_377_other".to_string(),
    };
    web_state
        .app
        .sessions
        .save_session(
            other_session_id,
            other_session.user,
            other_session.guilds,
            other_session.csrf_token,
            other_session.expires_at,
            None,
        )
        .await
        .unwrap();
    let other_cookie = format!("rusty_discord_bot_session={other_session_id}");

    // 1. Validation: empty title -> 400 Bad Request
    let bad_title_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/documents"))
        .header(COOKIE, &cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "title": "   ",
            "content": "Valid content",
            "source_type": "manual"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_title_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // 2. Validation: oversized title (>200 chars) -> 400 Bad Request
    let long_title = "a".repeat(201);
    let long_title_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/documents"))
        .header(COOKIE, &cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "title": long_title,
            "content": "Valid content",
            "source_type": "manual"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(long_title_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // 3. Validation: empty content -> 400 Bad Request
    let empty_content_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/documents"))
        .header(COOKIE, &cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "title": "Valid title",
            "content": "\n\t   \n",
            "source_type": "manual"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        empty_content_resp.status(),
        reqwest::StatusCode::BAD_REQUEST
    );

    // 4. Validation: oversized content (>64 KiB) -> 400 Bad Request
    let huge_content = "x".repeat(65 * 1024);
    let huge_content_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/documents"))
        .header(COOKIE, &cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "title": "Valid title",
            "content": huge_content,
            "source_type": "manual"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(huge_content_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // 5. Validation: invalid source_type -> 400 Bad Request
    let bad_source_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/documents"))
        .header(COOKIE, &cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "title": "Valid title",
            "content": "Valid content",
            "source_type": "invalid source<script>"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_source_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // 6. Validation: invalid channel_acl (non-numeric channel id) -> 400 Bad Request
    let bad_acl_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/documents"))
        .header(COOKIE, &cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "title": "Valid title",
            "content": "Valid content",
            "source_type": "manual",
            "channel_acl": ["not_a_snowflake_123!"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_acl_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // 7. Success: create valid document -> 200 OK
    let create_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/documents"))
        .header(COOKIE, &cookie)
        .header("x-csrf-token", csrf_token)
        .json(&serde_json::json!({
            "title": "Server Policy Guide",
            "content": "Be respectful and adhere to community guidelines at all times.",
            "source_type": "handbook",
            "channel_acl": ["100200300400500"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), reqwest::StatusCode::OK);
    let created_doc: serde_json::Value = create_resp.json().await.unwrap();
    let doc_id = created_doc["id"].as_str().unwrap();

    // Verify document is searchable
    let search_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/search"))
        .header(COOKIE, &cookie)
        .json(&serde_json::json!({
            "query": "respectful",
            "user_channels": ["100200300400500"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(search_resp.status(), reqwest::StatusCode::OK);
    let search_results: Vec<serde_json::Value> = search_resp.json().await.unwrap();
    assert_eq!(search_results.len(), 1);
    assert_eq!(search_results[0]["id"], doc_id);

    // 8. Cross-tenant isolation on DELETE: other guild user cannot delete this doc -> 403 Forbidden
    let cross_del_resp = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/ai/documents/{doc_id}"
        ))
        .header(COOKIE, &other_cookie)
        .header("x-csrf-token", "csrf_377_other")
        .send()
        .await
        .unwrap();
    assert_eq!(cross_del_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 9. Deleting non-existent doc on authorized guild -> 404 Not Found
    let random_uuid = uuid::Uuid::new_v4();
    let not_found_del = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/ai/documents/{random_uuid}"
        ))
        .header(COOKIE, &cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(not_found_del.status(), reqwest::StatusCode::NOT_FOUND);

    // 10. Legitimate DELETE: owner deletes document -> 200 OK
    let del_resp = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/ai/documents/{doc_id}"
        ))
        .header(COOKIE, &cookie)
        .header("x-csrf-token", csrf_token)
        .send()
        .await
        .unwrap();
    assert_eq!(del_resp.status(), reqwest::StatusCode::OK);
    let del_body: serde_json::Value = del_resp.json().await.unwrap();
    assert_eq!(del_body["status"], "deleted");
    assert_eq!(del_body["id"], doc_id);

    // Verify document is no longer searchable
    let search_after = client
        .post(format!("{base_url}/api/guilds/{guild_id}/ai/search"))
        .header(COOKIE, &cookie)
        .json(&serde_json::json!({
            "query": "respectful",
            "user_channels": ["100200300400500"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(search_after.status(), reqwest::StatusCode::OK);
    let after_results: Vec<serde_json::Value> = search_after.json().await.unwrap();
    assert_eq!(after_results.len(), 0);

    // 11. Storage Quota enforcement: filling up to limit returns 409 Conflict
    let quota_guild = "666111222333444555";
    for i in 1..=rusty_discord_bot::storage::MAX_AI_DOCUMENTS_PER_GUILD {
        web_state
            .app
            .storage
            .create_ai_document(rusty_discord_bot::storage::AiKnowledgeDocument {
                id: uuid::Uuid::new_v4(),
                guild_id: quota_guild.to_string(),
                channel_acl: serde_json::json!([]),
                title: format!("Doc #{i}"),
                content: format!("Content #{i}"),
                source_type: "bulk".to_string(),
                created_at: chrono::Utc::now(),
            })
            .await
            .expect("fill doc under quota");
    }

    // Next insert must exceed quota and return Err
    let excess_res = web_state
        .app
        .storage
        .create_ai_document(rusty_discord_bot::storage::AiKnowledgeDocument {
            id: uuid::Uuid::new_v4(),
            guild_id: quota_guild.to_string(),
            channel_acl: serde_json::json!([]),
            title: "Excess Doc".to_string(),
            content: "Excess content".to_string(),
            source_type: "bulk".to_string(),
            created_at: chrono::Utc::now(),
        })
        .await;
    assert!(
        excess_res.is_err(),
        "Must reject insertion beyond MAX_AI_DOCUMENTS_PER_GUILD"
    );
}

#[tokio::test]
async fn test_scheduled_events_validation_quota_and_pagination_issue_393() {
    let (web_state, permissions_cache) = test_web_state().await;
    let guild_id = "393000000000000099";
    let foreign_guild_id = "393000000000000088";

    let admin_id = "user_admin_393";
    let admin_session = "session_admin_393";
    let admin_csrf = "csrf_admin_393";

    let member_id = "user_member_393";
    let member_session = "session_member_393";
    let member_csrf = "csrf_member_393";

    let foreign_admin_id = "user_foreign_393";
    let foreign_admin_session = "session_foreign_393";
    let foreign_admin_csrf = "csrf_foreign_393";

    // Seed sessions
    web_state
        .app
        .sessions
        .save_session(
            admin_session,
            DashboardUser {
                id: admin_id.to_string(),
                username: "Admin393".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Guild 393".to_string(),
                icon: None,
                owner: true,
                permissions: "8".to_string(),
            }],
            admin_csrf.to_string(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .unwrap();

    web_state
        .app
        .sessions
        .save_session(
            member_session,
            DashboardUser {
                id: member_id.to_string(),
                username: "Member393".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Guild 393".to_string(),
                icon: None,
                owner: false,
                permissions: "0".to_string(),
            }],
            member_csrf.to_string(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .unwrap();

    web_state
        .app
        .sessions
        .save_session(
            foreign_admin_session,
            DashboardUser {
                id: foreign_admin_id.to_string(),
                username: "Foreign393".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: foreign_guild_id.to_string(),
                name: "Foreign Guild 393".to_string(),
                icon: None,
                owner: true,
                permissions: "8".to_string(),
            }],
            foreign_admin_csrf.to_string(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .unwrap();

    permissions_cache.insert(
        (admin_id.to_string(), guild_id.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );
    permissions_cache.insert(
        (member_id.to_string(), guild_id.to_string()),
        CachedGuildPermission {
            can_manage: false,
            checked_at: Utc::now(),
        },
    );
    permissions_cache.insert(
        (foreign_admin_id.to_string(), foreign_guild_id.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = build_router(web_state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let admin_cookie = format!("rusty_discord_bot_session={admin_session}");
    let member_cookie = format!("rusty_discord_bot_session={member_session}");
    let foreign_cookie = format!("rusty_discord_bot_session={foreign_admin_session}");

    let now = Utc::now();

    // 1. Validation tests on POST /api/guilds/{guild_id}/scheduled-events
    // 1a. Unauthenticated -> 401
    let unauth_create = client
        .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .json(&serde_json::json!({
            "title": "Unauth event",
            "description": "desc",
            "start_time": (now + Duration::hours(1)).to_rfc3339(),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth_create.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 1b. Member without manage permission -> 403
    let member_create = client
        .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &member_cookie)
        .header("X-CSRF-Token", member_csrf)
        .json(&serde_json::json!({
            "title": "Member event",
            "description": "desc",
            "start_time": (now + Duration::hours(1)).to_rfc3339(),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(member_create.status(), reqwest::StatusCode::FORBIDDEN);

    // 1c. Missing CSRF -> 403
    let no_csrf_create = client
        .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &admin_cookie)
        .json(&serde_json::json!({
            "title": "No CSRF",
            "description": "desc",
            "start_time": (now + Duration::hours(1)).to_rfc3339(),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(no_csrf_create.status(), reqwest::StatusCode::FORBIDDEN);

    // 1d. Empty or whitespace-only title -> 400
    let empty_title_create = client
        .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "title": "   ",
            "description": "desc",
            "start_time": (now + Duration::hours(1)).to_rfc3339(),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        empty_title_create.status(),
        reqwest::StatusCode::BAD_REQUEST
    );

    // 1e. Title > 200 chars -> 400
    let long_title = "t".repeat(201);
    let long_title_create = client
        .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "title": long_title,
            "description": "desc",
            "start_time": (now + Duration::hours(1)).to_rfc3339(),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(long_title_create.status(), reqwest::StatusCode::BAD_REQUEST);

    // 1f. Description > 4000 chars -> 400
    let long_desc = "d".repeat(4001);
    let long_desc_create = client
        .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "title": "Valid title",
            "description": long_desc,
            "start_time": (now + Duration::hours(1)).to_rfc3339(),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(long_desc_create.status(), reqwest::StatusCode::BAD_REQUEST);

    // 2. Quota enforcement on POST /scheduled-events
    web_state
        .app
        .storage
        .set_scheduled_events_quota_for_test(Some(2));

    // 2a. Event 1 with whitespace padding -> 200 OK & verify trimmed fields
    let ev1_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "title": "  Sprint Planning  ",
            "description": "  Discuss Q4 roadmap  ",
            "start_time": (now + Duration::hours(1)).to_rfc3339(),
            "end_time": (now + Duration::hours(2)).to_rfc3339(),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(ev1_resp.status(), reqwest::StatusCode::OK);
    let ev1: serde_json::Value = ev1_resp.json().await.unwrap();
    assert_eq!(ev1["title"], "Sprint Planning");
    assert_eq!(ev1["description"], "Discuss Q4 roadmap");
    let ev1_id = ev1["id"].as_str().unwrap().to_string();

    // 2b. Event 2 -> 200 OK
    let ev2_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "title": "Design Review",
            "description": "Review UI mocks",
            "start_time": (now + Duration::hours(3)).to_rfc3339(),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(ev2_resp.status(), reqwest::StatusCode::OK);
    let ev2: serde_json::Value = ev2_resp.json().await.unwrap();
    let ev2_id = ev2["id"].as_str().unwrap().to_string();

    // 2c. Event 3 -> 409 Conflict (quota exceeded)
    let quota_blocked = client
        .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "title": "Event 3 blocked",
            "description": "Blocked",
            "start_time": (now + Duration::hours(5)).to_rfc3339(),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(quota_blocked.status(), reqwest::StatusCode::CONFLICT);

    // 3. Deletion and slot reclamation
    // 3a. Unauthenticated DELETE -> 401
    let unauth_delete = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{ev1_id}"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth_delete.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 3b. Member without manage permission -> 403
    let member_delete = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{ev1_id}"
        ))
        .header(COOKIE, &member_cookie)
        .header("X-CSRF-Token", member_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(member_delete.status(), reqwest::StatusCode::FORBIDDEN);

    // 3c. Foreign admin on guild_id -> 403
    let foreign_delete = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{ev1_id}"
        ))
        .header(COOKIE, &foreign_cookie)
        .header("X-CSRF-Token", foreign_admin_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(foreign_delete.status(), reqwest::StatusCode::FORBIDDEN);

    // 3d. Non-existent event -> 404
    let not_found_delete = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{}",
            uuid::Uuid::new_v4()
        ))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(not_found_delete.status(), reqwest::StatusCode::NOT_FOUND);

    // 3e. Admin deletes ev1 -> 200 OK
    let delete_ok = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{ev1_id}"
        ))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(delete_ok.status(), reqwest::StatusCode::OK);

    // 3f. Slot reclaimed: creating Event 3 now succeeds
    let ev3_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "title": "Event 3 created",
            "description": "After slot release",
            "start_time": (now + Duration::hours(5)).to_rfc3339(),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(ev3_resp.status(), reqwest::StatusCode::OK);

    // 3g. Delete via alternate route /api/guilds/{guild_id}/events/{event_id} -> 200 OK
    let alt_delete_ok = client
        .delete(format!("{base_url}/api/guilds/{guild_id}/events/{ev2_id}"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(alt_delete_ok.status(), reqwest::StatusCode::OK);

    // 4. Keyset pagination tests
    web_state
        .app
        .storage
        .set_scheduled_events_quota_for_test(None);

    // Create 3 additional events
    for i in 1..=3 {
        let resp = client
            .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
            .header(COOKIE, &admin_cookie)
            .header("X-CSRF-Token", admin_csrf)
            .json(&serde_json::json!({
                "title": format!("Batch Event {i}"),
                "description": format!("Desc {i}"),
                "start_time": (now + Duration::hours(10 + i as i64)).to_rfc3339(),
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
    }

    // 4a. Unauthenticated GET /events/page -> 401
    let unauth_page = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/page"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth_page.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 4b. Foreign member GET /events/page -> 403
    let foreign_page = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/page"
        ))
        .header(COOKIE, &foreign_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(foreign_page.status(), reqwest::StatusCode::FORBIDDEN);

    // 4c. Member on guild_id with invalid limit (0 or 101) -> 400
    let limit_zero = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/page?limit=0"
        ))
        .header(COOKIE, &member_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(limit_zero.status(), reqwest::StatusCode::BAD_REQUEST);

    let limit_overflow = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/page?limit=101"
        ))
        .header(COOKIE, &member_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(limit_overflow.status(), reqwest::StatusCode::BAD_REQUEST);

    // 4d. Member fetches Page 1 (limit=2)
    let page1_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/page?limit=2"
        ))
        .header(COOKIE, &member_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(page1_resp.status(), reqwest::StatusCode::OK);
    let page1: serde_json::Value = page1_resp.json().await.unwrap();
    let p1_items = page1["items"].as_array().unwrap();
    assert_eq!(p1_items.len(), 2);
    assert_eq!(page1["has_more"], true);
    let next_cur = page1["next_cursor"].as_str().unwrap();

    // 4e. Cross-guild cursor tampering -> 400
    let foreign_cursor = rusty_discord_bot::storage::encode_scheduled_event_cursor(
        foreign_guild_id,
        &now,
        &uuid::Uuid::new_v4(),
    );
    let foreign_cur_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/page?cursor={foreign_cursor}"
        ))
        .header(COOKIE, &member_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(foreign_cur_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // 4f. Page 2 via alternate route /api/guilds/{guild_id}/events/page
    let page2_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/events/page?limit=2&cursor={next_cur}"
        ))
        .header(COOKIE, &member_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(page2_resp.status(), reqwest::StatusCode::OK);
    let page2: serde_json::Value = page2_resp.json().await.unwrap();
    let p2_items = page2["items"].as_array().unwrap();
    assert_eq!(p2_items.len(), 2);

    // 5. Legacy GET /api/guilds/{guild_id}/scheduled-events returns 200 OK with Vec
    let legacy_resp = client
        .get(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &member_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(legacy_resp.status(), reqwest::StatusCode::OK);
    let legacy_events: serde_json::Value = legacy_resp.json().await.unwrap();
    assert!(legacy_events.is_array());
    assert!(!legacy_events.as_array().unwrap().is_empty());

    // Reset quota override
    web_state
        .app
        .storage
        .set_scheduled_events_quota_for_test(None);
}

#[tokio::test]
async fn test_event_voice_queue_config_ready_and_outbox_issue_394() {
    let (web_state, permissions_cache) = test_web_state().await;
    let guild_id = "394000000000000099";

    let admin_id = "user_admin_394";
    let admin_session = "session_admin_394";
    let admin_csrf = "csrf_admin_394";

    let member_id = "user_member_394";
    let member_session = "session_member_394";
    let member_csrf = "csrf_member_394";

    // Seed sessions
    web_state
        .app
        .sessions
        .save_session(
            admin_session,
            DashboardUser {
                id: admin_id.to_string(),
                username: "Admin394".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Guild 394".to_string(),
                icon: None,
                owner: true,
                permissions: "8".to_string(),
            }],
            admin_csrf.to_string(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .unwrap();

    web_state
        .app
        .sessions
        .save_session(
            member_session,
            DashboardUser {
                id: member_id.to_string(),
                username: "Member394".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Guild 394".to_string(),
                icon: None,
                owner: false,
                permissions: "0".to_string(),
            }],
            member_csrf.to_string(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .unwrap();

    permissions_cache.insert(
        (admin_id.to_string(), guild_id.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );
    permissions_cache.insert(
        (member_id.to_string(), guild_id.to_string()),
        CachedGuildPermission {
            can_manage: false,
            checked_at: Utc::now(),
        },
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = build_router(web_state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let admin_cookie = format!("rusty_discord_bot_session={admin_session}");
    let member_cookie = format!("rusty_discord_bot_session={member_session}");

    let now = Utc::now();

    // 1. Create a scheduled event
    let create_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "title": "Voice Queue Event",
            "description": "Raid with voice queue and lobby pull",
            "start_time": (now + Duration::hours(1)).to_rfc3339(),
            "max_participants": 1,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), reqwest::StatusCode::OK);
    let created_event: serde_json::Value = create_resp.json().await.unwrap();
    let event_id = created_event["id"].as_str().unwrap().to_string();

    // 2. GET default voice queue config -> 200 OK (enabled: false, mode: notify_only)
    let get_vq_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{event_id}/voice-queue"
        ))
        .header(COOKIE, &member_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_vq_resp.status(), reqwest::StatusCode::OK);
    let vq_json: serde_json::Value = get_vq_resp.json().await.unwrap();
    assert_eq!(vq_json["enabled"], false);
    assert_eq!(vq_json["mode"], "notify_only");

    // 3. PUT voice queue config without manage permissions (member) -> 403 Forbidden
    let put_forbidden = client
        .put(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{event_id}/voice-queue"
        ))
        .header(COOKIE, &member_cookie)
        .header("X-CSRF-Token", member_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "lobby_channel_id": "111222333",
            "target_channel_id": "444555666",
            "mode": "auto_move"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(put_forbidden.status(), reqwest::StatusCode::FORBIDDEN);

    // 4. PUT voice queue config with invalid mode -> 400 Bad Request
    let put_invalid_mode = client
        .put(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{event_id}/voice-queue"
        ))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "lobby_channel_id": "111222333",
            "target_channel_id": "444555666",
            "mode": "invalid_teleport"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(put_invalid_mode.status(), reqwest::StatusCode::BAD_REQUEST);

    // 4b. Issue #398: Injected audit failure causes PUT to fail with 500 and rollback
    web_state.app.storage.inject_audit_failure(true);
    let put_audit_fail = client
        .put(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{event_id}/voice-queue"
        ))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "lobby_channel_id": "111222333444",
            "target_channel_id": "555666777888",
            "mode": "auto_move"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        put_audit_fail.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );
    web_state.app.storage.inject_audit_failure(false);

    // Config must still be default (enabled: false)
    let get_after_fail = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{event_id}/voice-queue"
        ))
        .header(COOKIE, &member_cookie)
        .send()
        .await
        .unwrap();
    let vq_after_fail: serde_json::Value = get_after_fail.json().await.unwrap();
    assert_eq!(vq_after_fail["enabled"], false);

    // 5. PUT voice queue config as admin with auto_move -> 200 OK
    let lobby_ch = "111222333444";
    let target_ch = "555666777888";
    let put_ok = client
        .put(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{event_id}/voice-queue"
        ))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "lobby_channel_id": lobby_ch,
            "target_channel_id": target_ch,
            "mode": "auto_move"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(put_ok.status(), reqwest::StatusCode::OK);
    let updated_vq: serde_json::Value = put_ok.json().await.unwrap();
    assert_eq!(updated_vq["enabled"], true);
    assert_eq!(updated_vq["lobby_channel_id"], lobby_ch);
    assert_eq!(updated_vq["target_channel_id"], target_ch);
    assert_eq!(updated_vq["mode"], "auto_move");

    // Verify config_updated audit event was atomically written
    let audits = web_state
        .app
        .storage
        .list_audit_events(guild_id, 10)
        .await
        .unwrap();
    let vq_audits: Vec<_> = audits
        .iter()
        .filter(|a| a.event == "config_updated" && a.module_id == "event_voice_queue")
        .collect();
    assert_eq!(vq_audits.len(), 1);
    assert_eq!(vq_audits[0].actor_id.as_deref(), Some(admin_id));
    assert_eq!(vq_audits[0].data["event_id"], event_id);
    assert_eq!(vq_audits[0].data["enabled"], true);
    assert_eq!(vq_audits[0].data["mode"], "auto_move");

    // 6. Add admin as main roster member, member as reserve roster member
    let ev_uuid = uuid::Uuid::parse_str(&event_id).unwrap();
    let r_admin = web_state
        .app
        .storage
        .join_event_roster(ev_uuid, admin_id)
        .await
        .unwrap();
    assert_eq!(r_admin.role_type, "main");
    let r_member = web_state
        .app
        .storage
        .join_event_roster(ev_uuid, member_id)
        .await
        .unwrap();
    assert_eq!(r_member.role_type, "reserve");

    // 7. Member marks ready via POST /voice-queue/ready
    // Since member is reserve, ready status updates but NO auto_move outbox job is created
    let member_ready_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{event_id}/voice-queue/ready"
        ))
        .header(COOKIE, &member_cookie)
        .header("X-CSRF-Token", member_csrf)
        .json(&serde_json::json!({
            "consent_auto_move": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(member_ready_resp.status(), reqwest::StatusCode::OK);
    let member_ready: serde_json::Value = member_ready_resp.json().await.unwrap();
    assert_eq!(member_ready["ready_status"], "ready");
    assert_eq!(member_ready["consent_auto_move"], true);
    assert_eq!(member_ready["generation"], 1);

    // Verify outbox has NO job for reserve member
    let no_job = web_state
        .app
        .storage
        .claim_event_voice_outbox_job("worker_1", Duration::seconds(30))
        .await
        .unwrap();
    assert!(no_job.is_none());

    // 8. Admin marks ready via POST /voice-queue/ready with consent_auto_move
    // Since admin is main and mode is auto_move, an auto_move outbox job is generated!
    let admin_ready_resp = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/events/{event_id}/voice-queue/ready"
        ))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "consent_auto_move": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(admin_ready_resp.status(), reqwest::StatusCode::OK);
    let admin_ready: serde_json::Value = admin_ready_resp.json().await.unwrap();
    assert_eq!(admin_ready["ready_status"], "ready");
    assert_eq!(admin_ready["consent_auto_move"], true);

    // Verify outbox NOW has the auto_move job for admin
    let claimed_job = web_state
        .app
        .storage
        .claim_event_voice_outbox_job("worker_1", Duration::seconds(30))
        .await
        .unwrap();
    assert!(claimed_job.is_some());
    let job = claimed_job.unwrap();
    assert_eq!(job.event_id, ev_uuid);
    assert_eq!(job.user_id, admin_id);
    assert_eq!(
        job.kind,
        rusty_discord_bot::storage::EVENT_VOICE_JOB_KIND_MOVE_VOICE
    );

    // Complete job
    web_state
        .app
        .storage
        .complete_event_voice_outbox_job(job.id, "worker_1")
        .await
        .unwrap();

    // 9. GET /voice-queue/roster returns merged entries with ready states
    let roster_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{event_id}/voice-queue/roster"
        ))
        .header(COOKIE, &member_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(roster_resp.status(), reqwest::StatusCode::OK);
    let roster: Vec<serde_json::Value> = roster_resp.json().await.unwrap();
    assert_eq!(roster.len(), 2);
    let admin_entry = roster.iter().find(|e| e["user_id"] == admin_id).unwrap();
    assert_eq!(admin_entry["role_type"], "main");
    assert_eq!(admin_entry["ready_status"], "ready");
    assert_eq!(admin_entry["consent_auto_move"], true);

    let member_entry = roster.iter().find(|e| e["user_id"] == member_id).unwrap();
    assert_eq!(member_entry["role_type"], "reserve");
    assert_eq!(member_entry["ready_status"], "ready");

    // 10. Member cancels ready via DELETE /voice-queue/ready -> ready_status reverts to waiting, generation increments
    let cancel_resp = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/events/{event_id}/voice-queue/ready"
        ))
        .header(COOKIE, &member_cookie)
        .header("X-CSRF-Token", member_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(cancel_resp.status(), reqwest::StatusCode::OK);
    let cancelled_state: serde_json::Value = cancel_resp.json().await.unwrap();
    assert_eq!(cancelled_state["ready_status"], "waiting");
    assert_eq!(cancelled_state["generation"], 2);

    // 11. Delete event via DELETE /scheduled-events/{event_id} -> verifies cascade
    let del_resp = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{event_id}"
        ))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(del_resp.status(), reqwest::StatusCode::OK);

    // Voice queue endpoints now return 404
    let after_del_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{event_id}/voice-queue"
        ))
        .header(COOKIE, &member_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(after_del_resp.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_event_voice_outbox_requeue_and_backoff_issue_396() {
    let (web_state, permissions_cache) = test_web_state().await;
    let guild_id = "396000000000000099";

    let admin_id = "user_admin_396";
    let admin_session = "session_admin_396";
    let admin_csrf = "csrf_admin_396";

    // Seed admin session
    web_state
        .app
        .sessions
        .save_session(
            admin_session,
            DashboardUser {
                id: admin_id.to_string(),
                username: "Admin396".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Guild 396".to_string(),
                icon: None,
                owner: true,
                permissions: "8".to_string(),
            }],
            admin_csrf.to_string(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .unwrap();

    permissions_cache.insert(
        (admin_id.to_string(), guild_id.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = build_router(web_state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let admin_cookie = format!("rusty_discord_bot_session={admin_session}");
    let now = Utc::now();

    // 1. Create a scheduled event
    let create_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/scheduled-events"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "title": "Voice Outbox Test Event",
            "description": "Testing outbox requeue and backoff policies",
            "start_time": (now + Duration::hours(1)).to_rfc3339(),
            "max_participants": 1,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), reqwest::StatusCode::OK);
    let created_event: serde_json::Value = create_resp.json().await.unwrap();
    let event_id = created_event["id"].as_str().unwrap().to_string();
    let ev_uuid = uuid::Uuid::parse_str(&event_id).unwrap();

    // 2. Configure voice queue
    let put_resp = client
        .put(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{event_id}/voice-queue"
        ))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "enabled": true,
            "lobby_channel_id": "111222333",
            "target_channel_id": "444555666",
            "mode": "notify_only"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(put_resp.status(), reqwest::StatusCode::OK);

    // 3. Admin joins roster as main
    let r_admin = web_state
        .app
        .storage
        .join_event_roster(ev_uuid, admin_id)
        .await
        .unwrap();
    assert_eq!(r_admin.role_type, "main");

    // 4. Admin marks ready -> queued: true
    let ready_resp1 = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{event_id}/voice-queue/ready"
        ))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(ready_resp1.status(), reqwest::StatusCode::OK);
    let ready_val1: serde_json::Value = ready_resp1.json().await.unwrap();
    assert_eq!(ready_val1["queued"], true);
    let job_id_1 = ready_val1["job_id"].as_str().unwrap();

    // 5. Worker claims job -> leased
    let claimed = web_state
        .app
        .storage
        .claim_event_voice_outbox_job("worker_1", Duration::seconds(30))
        .await
        .unwrap();
    assert!(claimed.is_some());
    let cl = claimed.unwrap();
    assert_eq!(cl.id.to_string(), job_id_1);

    // 6. Worker fails job with backoff (e.g. 429 rate limit with 60s delay)
    let failed = web_state
        .app
        .storage
        .fail_event_voice_outbox_job(
            cl.id,
            "worker_1",
            "Rate limited 429",
            Some(chrono::Duration::seconds(60)),
        )
        .await
        .unwrap();
    assert!(failed);

    // 7. Repeated Ready calls during delayed pending still return queued: true, but do not shorten backoff
    let ready_resp2 = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{event_id}/voice-queue/ready"
        ))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(ready_resp2.status(), reqwest::StatusCode::OK);
    let ready_val2: serde_json::Value = ready_resp2.json().await.unwrap();
    assert_eq!(ready_val2["queued"], true);

    // Worker cannot claim yet because backoff has not expired
    assert!(
        web_state
            .app
            .storage
            .claim_event_voice_outbox_job("worker_2", Duration::seconds(30))
            .await
            .unwrap()
            .is_none()
    );

    // 8. Cancel ready -> increments generation to 2
    let cancel_resp = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{event_id}/voice-queue/ready"
        ))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(cancel_resp.status(), reqwest::StatusCode::OK);
    let cancel_val: serde_json::Value = cancel_resp.json().await.unwrap();
    assert_eq!(cancel_val["generation"], 2);

    // 9. Ready on generation 2 creates new job and returns queued: true
    let ready_resp3 = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{event_id}/voice-queue/ready"
        ))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(ready_resp3.status(), reqwest::StatusCode::OK);
    let ready_val3: serde_json::Value = ready_resp3.json().await.unwrap();
    assert_eq!(ready_val3["queued"], true);
    assert_eq!(ready_val3["generation"], 2);

    // Claim on gen 2
    let claimed2 = web_state
        .app
        .storage
        .claim_event_voice_outbox_job("worker_3", Duration::seconds(30))
        .await
        .unwrap()
        .unwrap();

    // Fail permanently (retry_delay = None) -> state = 'failed'
    assert!(
        web_state
            .app
            .storage
            .fail_event_voice_outbox_job(claimed2.id, "worker_3", "Permanent error", None)
            .await
            .unwrap()
    );

    // 10. Calling Ready after failure successfully requeues to pending -> queued: true
    let ready_requeue = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{event_id}/voice-queue/ready"
        ))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(ready_requeue.status(), reqwest::StatusCode::OK);
    let requeue_val: serde_json::Value = ready_requeue.json().await.unwrap();
    assert_eq!(requeue_val["queued"], true);
    assert_eq!(
        requeue_val["job_id"].as_str().unwrap(),
        claimed2.id.to_string()
    );

    // Worker can now claim the requeued job!
    let claimed_after_requeue = web_state
        .app
        .storage
        .claim_event_voice_outbox_job("worker_3", Duration::seconds(30))
        .await
        .unwrap();
    assert!(claimed_after_requeue.is_some());
    let cr = claimed_after_requeue.unwrap();
    assert_eq!(cr.id, claimed2.id);

    // Worker completes it -> 'done'
    assert!(
        web_state
            .app
            .storage
            .complete_event_voice_outbox_job(cr.id, "worker_3")
            .await
            .unwrap()
    );

    // 11. Repeated Ready after 'done' in the same generation returns queued: false (never duplicates job)
    let ready_after_done = client
        .post(format!(
            "{base_url}/api/guilds/{guild_id}/scheduled-events/{event_id}/voice-queue/ready"
        ))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(ready_after_done.status(), reqwest::StatusCode::OK);
    let done_val: serde_json::Value = ready_after_done.json().await.unwrap();
    assert_eq!(done_val["queued"], false);
    assert_eq!(done_val["job_id"].as_str().unwrap(), cr.id.to_string());
}

#[tokio::test]
async fn test_workflow_builder_crud_quota_and_pagination_issue_395() {
    let (web_state, permissions_cache) = test_web_state().await;
    let guild_id = "395000000000000099";
    let foreign_guild_id = "395000000000000088";

    let admin_id = "user_admin_395";
    let admin_session = "session_admin_395";
    let admin_csrf = "csrf_admin_395";

    let member_id = "user_member_395";
    let member_session = "session_member_395";
    let member_csrf = "csrf_member_395";

    // Seed sessions
    web_state
        .app
        .sessions
        .save_session(
            admin_session,
            DashboardUser {
                id: admin_id.to_string(),
                username: "Admin395".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Guild 395".to_string(),
                icon: None,
                owner: true,
                permissions: "8".to_string(),
            }],
            admin_csrf.to_string(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .unwrap();

    web_state
        .app
        .sessions
        .save_session(
            member_session,
            DashboardUser {
                id: member_id.to_string(),
                username: "Member395".to_string(),
                global_name: None,
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Guild 395".to_string(),
                icon: None,
                owner: false,
                permissions: "0".to_string(),
            }],
            member_csrf.to_string(),
            Utc::now() + Duration::hours(2),
            None,
        )
        .await
        .unwrap();

    permissions_cache.insert(
        (admin_id.to_string(), guild_id.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: Utc::now(),
        },
    );
    permissions_cache.insert(
        (member_id.to_string(), guild_id.to_string()),
        CachedGuildPermission {
            can_manage: false,
            checked_at: Utc::now(),
        },
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = build_router(web_state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{addr}");
    let admin_cookie = format!("rusty_discord_bot_session={admin_session}");
    let member_cookie = format!("rusty_discord_bot_session={member_session}");

    // 1. Validation tests on POST /api/guilds/{guild_id}/workflows
    // 1a. Empty name -> 400
    let empty_name_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/workflows"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "name": "   ",
            "dag_nodes": [{ "id": "t1", "node_type": "trigger", "config": {} }],
            "dag_edges": []
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(empty_name_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // 1b. Name > 120 chars -> 400
    let long_name_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/workflows"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "name": "w".repeat(121),
            "dag_nodes": [{ "id": "t1", "node_type": "trigger", "config": {} }],
            "dag_edges": []
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(long_name_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // 1c. Invalid node ID characters -> 400
    let invalid_node_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/workflows"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "name": "Valid Name",
            "dag_nodes": [{ "id": "t1 bad id!", "node_type": "trigger", "config": {} }],
            "dag_edges": []
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_node_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // 1d. Node config > 8 KiB -> 400
    let huge_config_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/workflows"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "name": "Valid Name",
            "dag_nodes": [{ "id": "t1", "node_type": "trigger", "config": { "payload": "x".repeat(8193) } }],
            "dag_edges": []
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(huge_config_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // 1e. Unauthorized member without manage permissions -> 403 Forbidden
    let forbidden_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/workflows"))
        .header(COOKIE, &member_cookie)
        .header("X-CSRF-Token", member_csrf)
        .json(&serde_json::json!({
            "name": "Unauthorized WF",
            "dag_nodes": [{ "id": "t1", "node_type": "trigger", "config": {} }],
            "dag_edges": []
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(forbidden_resp.status(), reqwest::StatusCode::FORBIDDEN);

    // 2. Quota enforcement on POST /workflows
    web_state
        .app
        .storage
        .set_workflows_quota_for_test(Some((2, 100_000)));

    // 2a. Workflow 1 created with whitespace padding -> 200 OK & verify trimmed name
    let wf1_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/workflows"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "name": "  Discord Alert WF  ",
            "dag_nodes": [{ "id": "start", "node_type": "trigger", "config": {} }],
            "dag_edges": []
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(wf1_resp.status(), reqwest::StatusCode::OK);
    let wf1: serde_json::Value = wf1_resp.json().await.unwrap();
    assert_eq!(wf1["name"], "Discord Alert WF");
    let wf1_id = wf1["id"].as_str().unwrap().to_string();

    // 2b. Workflow 2 -> 200 OK
    let wf2_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/workflows"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "name": "Role Assign WF",
            "dag_nodes": [{ "id": "t1", "node_type": "trigger", "config": {} }],
            "dag_edges": []
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(wf2_resp.status(), reqwest::StatusCode::OK);

    // 2c. Workflow 3 -> 409 Conflict (quota exceeded)
    let wf3_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/workflows"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "name": "Quota Over WF",
            "dag_nodes": [{ "id": "t1", "node_type": "trigger", "config": {} }],
            "dag_edges": []
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(wf3_resp.status(), reqwest::StatusCode::CONFLICT);

    // 3. Keyset Pagination on GET /api/guilds/{guild_id}/workflows
    // 3a. Member without manage permissions cannot list workflows -> 403 Forbidden
    let forbidden_get = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/workflows?limit=2"
        ))
        .header(COOKIE, &member_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(forbidden_get.status(), reqwest::StatusCode::FORBIDDEN);

    web_state
        .app
        .storage
        .set_workflows_quota_for_test(Some((10, 500_000)));

    // Create 2 more workflows (total 4)
    for i in 3..=4 {
        let resp = client
            .post(format!("{base_url}/api/guilds/{guild_id}/workflows"))
            .header(COOKIE, &admin_cookie)
            .header("X-CSRF-Token", admin_csrf)
            .json(&serde_json::json!({
                "name": format!("Batch WF {i}"),
                "dag_nodes": [{ "id": "t1", "node_type": "trigger", "config": {} }],
                "dag_edges": []
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
    }

    // Page 1: limit=2 as admin -> 200 OK
    let page1_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/workflows?limit=2"
        ))
        .header(COOKIE, &admin_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(page1_resp.status(), reqwest::StatusCode::OK);
    let page1: serde_json::Value = page1_resp.json().await.unwrap();
    assert_eq!(page1["items"].as_array().unwrap().len(), 2);
    assert_eq!(page1["has_more"], true);
    let next_cursor = page1["next_cursor"].as_str().unwrap().to_string();

    // Cross-guild cursor tampering -> 400 Bad Request
    let foreign_cursor = rusty_discord_bot::storage::encode_workflow_definition_cursor(
        foreign_guild_id,
        &Utc::now(),
        &uuid::Uuid::new_v4(),
    );
    let tamper_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/workflows?cursor={foreign_cursor}"
        ))
        .header(COOKIE, &admin_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(tamper_resp.status(), reqwest::StatusCode::BAD_REQUEST);

    // Page 2: limit=2 with valid next_cursor as admin -> 200 OK
    let page2_resp = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/workflows?limit=2&cursor={next_cursor}"
        ))
        .header(COOKIE, &admin_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(page2_resp.status(), reqwest::StatusCode::OK);
    let page2: serde_json::Value = page2_resp.json().await.unwrap();
    assert_eq!(page2["items"].as_array().unwrap().len(), 2);

    // 4. GET /api/guilds/{guild_id}/workflows/{wf1_id} -> 200 OK
    let get_single = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/workflows/{wf1_id}"
        ))
        .header(COOKIE, &admin_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_single.status(), reqwest::StatusCode::OK);

    // 5. DELETE tests
    // 5a. Member without manage permissions -> 403 Forbidden
    let del_forbidden = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/workflows/{wf1_id}"
        ))
        .header(COOKIE, &member_cookie)
        .header("X-CSRF-Token", member_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(del_forbidden.status(), reqwest::StatusCode::FORBIDDEN);

    // 5b. Non-existent workflow -> 404 Not Found
    let random_id = uuid::Uuid::new_v4();
    let del_not_found = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/workflows/{random_id}"
        ))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(del_not_found.status(), reqwest::StatusCode::NOT_FOUND);

    // 5c. Admin successfully deletes wf1 -> 200 OK
    let del_ok = client
        .delete(format!(
            "{base_url}/api/guilds/{guild_id}/workflows/{wf1_id}"
        ))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(del_ok.status(), reqwest::StatusCode::OK);

    // 5d. Subsequent GET for wf1 -> 404 Not Found
    let get_deleted = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/workflows/{wf1_id}"
        ))
        .header(COOKIE, &admin_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(get_deleted.status(), reqwest::StatusCode::NOT_FOUND);

    // 6. Issue #397: Failure injection on audit_events causes POST /workflows to fail and roll back
    web_state.app.storage.inject_audit_failure(true);
    let audit_fail_resp = client
        .post(format!("{base_url}/api/guilds/{guild_id}/workflows"))
        .header(COOKIE, &admin_cookie)
        .header("X-CSRF-Token", admin_csrf)
        .json(&serde_json::json!({
            "name": "Audit Fail Rollback WF",
            "dag_nodes": [{ "id": "t1", "node_type": "trigger", "config": {} }],
            "dag_edges": []
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        audit_fail_resp.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );
    web_state.app.storage.inject_audit_failure(false);

    // Verify the rolled-back workflow was NOT saved
    let list_after_rollback = client
        .get(format!(
            "{base_url}/api/guilds/{guild_id}/workflows?limit=100"
        ))
        .header(COOKIE, &admin_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(list_after_rollback.status(), reqwest::StatusCode::OK);
    let page_val: serde_json::Value = list_after_rollback.json().await.unwrap();
    let items = page_val["items"].as_array().unwrap();
    assert!(!items.iter().any(|w| w["name"] == "Audit Fail Rollback WF"));

    // Verify successful creation produced workflow_created audit event with correct actor_id
    let audits = web_state
        .app
        .storage
        .list_audit_events(guild_id, 50)
        .await
        .unwrap();
    let created_audits: Vec<_> = audits
        .iter()
        .filter(|a| a.event == "workflow_created")
        .collect();
    assert!(!created_audits.is_empty());
    for a in &created_audits {
        assert_eq!(a.module_id, "workflow_builder");
        assert_eq!(a.actor_id.as_deref(), Some(admin_id));
        assert!(a.data.get("workflow_id").is_some());
        assert!(a.data.get("workflow_name").is_some());
    }

    // Reset quota override
    web_state.app.storage.set_workflows_quota_for_test(None);
}
