use std::collections::HashSet;
use std::sync::Arc;

use axum::http::header::COOKIE;
use dashmap::DashMap;
use rusty_discord_bot::{
    config::Config,
    privacy::{
        PRIVACY_DATASETS, PrivacyAction, build_default_retained_notices, get_dataset_by_key,
        list_exportable_datasets, list_retained_datasets,
    },
    session::{DashboardGuild, DashboardUser},
    state::AppState,
    storage::{AfkRecord, BirthdayRecord, QuoteRecord},
    web::{
        WebState,
        auth::{CachedGuildPermission, LoginRateLimiter},
        build_router,
    },
};
use serde_json::json;
use uuid::Uuid;

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
        ticket_transcripts_disabled: false,
        integration_credential_key: None,
        integration_credential_active_key_id: "v1".to_string(),
        integration_credential_keyring: None,
        integration_credentials_disabled: false,
        metrics_token: None,
        metrics_allowed_ips: HashSet::new(),
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
async fn test_privacy_dataset_registry_completeness() {
    // 1. All registered datasets must have valid metadata
    let mut seen_keys = std::collections::HashSet::new();
    let mut seen_export_keys = std::collections::HashSet::new();

    for ds in PRIVACY_DATASETS {
        assert!(
            seen_keys.insert(ds.key),
            "Duplicate dataset key: {}",
            ds.key
        );
        assert!(!ds.display_name.is_empty());
        assert!(!ds.owner_module.is_empty());
        assert!(!ds.storage_table.is_empty());
        assert!(!ds.subject_fields.is_empty());

        if ds.is_exportable {
            let exp_key = ds
                .export_key
                .expect("Exportable dataset must have export_key");
            assert!(
                seen_export_keys.insert(exp_key),
                "Duplicate export key: {exp_key}"
            );
        }

        match ds.action {
            PrivacyAction::Delete => {}
            PrivacyAction::Pseudonymize => {}
            PrivacyAction::Retain { days, reason } => {
                assert!(days > 0, "Retention period must be positive for {}", ds.key);
                assert!(
                    !reason.is_empty(),
                    "Retention reason must be specified for {}",
                    ds.key
                );
            }
        }
    }

    // 2. Specific completeness requirements from Issue #307
    // Quotes must cover both added_by and author_id
    let quotes_ds = get_dataset_by_key("guild_quotes").expect("guild_quotes dataset");
    assert!(quotes_ds.subject_fields.contains(&"added_by"));
    assert!(quotes_ds.subject_fields.contains(&"author_id"));
    assert_eq!(quotes_ds.action, PrivacyAction::Delete);
    assert_eq!(quotes_ds.export_key, Some("quotes"));

    // AFK must have export and erase
    let afk_ds = get_dataset_by_key("member_afk").expect("member_afk dataset");
    assert!(afk_ds.subject_fields.contains(&"user_id"));
    assert_eq!(afk_ds.action, PrivacyAction::Delete);
    assert_eq!(afk_ds.export_key, Some("afk"));

    // Reaction role grants must have export and erase
    let rr_ds = get_dataset_by_key("reaction_role_grants").expect("reaction_role_grants dataset");
    assert!(rr_ds.subject_fields.contains(&"user_id"));
    assert_eq!(rr_ds.action, PrivacyAction::Delete);
    assert_eq!(rr_ds.export_key, Some("reaction_roles"));

    // 3. Retained notices match retained datasets
    let retained_notices = build_default_retained_notices();
    let retained_ds_count = list_retained_datasets().count();
    assert_eq!(
        retained_notices.len(),
        retained_ds_count,
        "Retained notices count must match retained datasets count"
    );
}

#[tokio::test]
async fn test_privacy_dataset_registry_http_workflow() {
    let (state, perms_cache) = test_web_state().await;
    let app = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::builder().build().unwrap();

    let guild_id = "555000000000000001";
    let user_id = "666000000000000001";
    let session_id = "test_privacy_registry_session_123";
    let csrf_token = "test_privacy_registry_csrf_456";

    state
        .app
        .sessions
        .save_session(
            session_id,
            DashboardUser {
                id: user_id.to_string(),
                username: "PrivacyAdmin".to_string(),
                global_name: Some("Privacy Admin".to_string()),
                avatar: None,
            },
            vec![DashboardGuild {
                id: guild_id.to_string(),
                name: "Privacy Guild".to_string(),
                icon: None,
                owner: true,
                permissions: "8".to_string(), // Administrator
            }],
            csrf_token.to_string(),
            chrono::Utc::now() + chrono::Duration::hours(2),
            None,
        )
        .await
        .unwrap();

    perms_cache.insert(
        (user_id.to_string(), guild_id.to_string()),
        CachedGuildPermission {
            can_manage: true,
            checked_at: chrono::Utc::now(),
        },
    );

    let session_cookie = format!("rusty_discord_bot_session={session_id}");

    // 1. GET /api/guilds/{id}/privacy returns dataset registry
    let summary_res = client
        .get(format!("{base_url}/api/guilds/{guild_id}/privacy"))
        .header(COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(summary_res.status(), 200);
    let summary_json: serde_json::Value = summary_res.json().await.unwrap();
    let datasets = summary_json["datasets"].as_array().expect("datasets array");
    assert!(
        datasets.len() >= 7,
        "Summary must contain all registered datasets"
    );

    // 2. Setup user data across multiple datasets
    state
        .app
        .storage
        .set_afk(AfkRecord {
            guild_id: guild_id.to_string(),
            user_id: user_id.to_string(),
            reason: "AFK for dataset registry test".to_string(),
            set_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let rr_binding = Uuid::new_v4();
    state
        .app
        .storage
        .record_reaction_role_grant(guild_id, user_id, "1002", rr_binding, true)
        .await
        .unwrap();

    state
        .app
        .storage
        .set_birthday(BirthdayRecord {
            guild_id: guild_id.to_string(),
            user_id: user_id.to_string(),
            month: 7,
            day: 15,
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    state
        .app
        .storage
        .create_quote(QuoteRecord {
            id: Uuid::new_v4(),
            guild_id: guild_id.to_string(),
            added_by: user_id.to_string(),
            author_id: user_id.to_string(),
            body: "Registry test quote".to_string(),
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    // 3. POST /privacy/export must return all registered exportable dataset keys
    let export_res = client
        .post(format!("{base_url}/api/guilds/{guild_id}/privacy/export"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&json!({ "user_id": user_id }))
        .send()
        .await
        .unwrap();
    assert_eq!(export_res.status(), 200);
    let export_json: serde_json::Value = export_res.json().await.unwrap();

    for ds in list_exportable_datasets() {
        let key = ds.export_key.unwrap();
        assert!(
            export_json.get(key).is_some(),
            "Export JSON must contain field '{key}' registered for dataset '{}'",
            ds.key
        );
    }

    assert_eq!(
        export_json["afk"]["reason"],
        "AFK for dataset registry test"
    );
    assert_eq!(export_json["birthday"]["month"], 7);
    assert_eq!(export_json["reaction_roles"].as_array().unwrap().len(), 1);
    assert_eq!(export_json["quotes"].as_array().unwrap().len(), 1);

    // 4. POST /privacy/delete must erase deletable datasets and retain notices
    let delete_res = client
        .post(format!("{base_url}/api/guilds/{guild_id}/privacy/delete"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&json!({ "user_id": user_id }))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_res.status(), 200);
    let delete_json: serde_json::Value = delete_res.json().await.unwrap();
    assert_eq!(delete_json["status"], "data_erased");

    let retained = delete_json["report"]["retained_records"]
        .as_array()
        .unwrap();
    assert_eq!(retained.len(), list_retained_datasets().count());

    // 5. Subsequent export must show all deleted datasets as null/empty
    let export_after = client
        .post(format!("{base_url}/api/guilds/{guild_id}/privacy/export"))
        .header(COOKIE, &session_cookie)
        .header("X-CSRF-Token", csrf_token)
        .json(&json!({ "user_id": user_id }))
        .send()
        .await
        .unwrap();
    assert_eq!(export_after.status(), 200);
    let export_after_json: serde_json::Value = export_after.json().await.unwrap();

    assert!(export_after_json["afk"].is_null());
    assert!(export_after_json["birthday"].is_null());
    assert!(export_after_json["progression"].is_null());
    assert!(export_after_json["quotes"].as_array().unwrap().is_empty());
    assert!(
        export_after_json["reaction_roles"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
