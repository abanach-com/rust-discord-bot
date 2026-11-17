use std::{future::Future, path::PathBuf, sync::Arc};

use rusty_discord_bot::{
    AppState,
    config::Config,
    datetime::Timestamp,
    integrations::CredentialCrypto,
    storage::{IntegrationConnection, Storage, run_safe_migrations},
};
use sqlx::{PgPool, Postgres, migrate::MigrateDatabase};
use url::Url;
use uuid::Uuid;

const OLD_KEY: [u8; 32] = [0x21; 32];
const ACTIVE_KEY: [u8; 32] = [0x42; 32];
const GUILD: &str = "100000000000000001";
const OTHER_GUILD: &str = "100000000000000002";
const FIXTURE_SECRET: &str = "fixture-legacy-credential";
const OLD_SECRET: &str = "fixture-credential-from-old-key";
const NEW_SECRET: &str = "fixture-credential-written-after-rotation";
const UNSUPPORTED_ENVELOPE: &str = "enc:v2:reserved-envelope";
const MIXED_CONNECTION_ID: Uuid = Uuid::from_u128(1);
const ACTIVE_PREFIX: &str = "enc:v1:v2:";
const OLD_PREFIX: &str = "enc:v1:v1:";

fn rotated_keyring() -> String {
    format!("v1:{},v2:{}", hex::encode(OLD_KEY), hex::encode(ACTIVE_KEY))
}

async fn with_credential_database<F>(exercise: impl FnOnce(PgPool, String) -> F)
where
    F: Future<Output = ()> + Send + 'static,
{
    let Some(database_url) = std::env::var("DATABASE_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())
    else {
        eprintln!("SKIPPING PostgreSQL credential backfill test: DATABASE_URL not set");
        return;
    };
    let mut url = Url::parse(&database_url).expect("parse PostgreSQL test URL");
    url.set_path(&format!("/credential_backfill_{}", Uuid::new_v4().simple()));
    Postgres::create_database(url.as_str())
        .await
        .expect("create isolated PostgreSQL test database");
    let pool = PgPool::connect(url.as_str())
        .await
        .expect("connect to isolated PostgreSQL test database");

    let test_result = tokio::spawn(exercise(pool.clone(), url.to_string())).await;
    pool.close().await;
    Postgres::drop_database(url.as_str())
        .await
        .expect("drop isolated PostgreSQL test database");
    test_result.expect("credential backfill test failed");
}

fn offline_postgres_config(database_url: String) -> Config {
    Config {
        discord_token: "dummy-token".into(),
        discord_application_id: 1,
        discord_client_id: "1".into(),
        discord_client_secret: "fixture-client-secret".into(),
        discord_oauth_redirect_url: "http://localhost/callback".into(),
        discord_dev_guild_id: None,
        dashboard_bind: "127.0.0.1:0".parse().unwrap(),
        dashboard_public_url: "http://localhost".into(),
        database_url: Some(database_url),
        scripts_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts"),
        session_cookie_secure: false,
        session_ttl_seconds: 3600,
        lua_memory_limit_bytes: 4 * 1024 * 1024,
        lua_instruction_limit: 200_000,
        lua_hook_granularity: 100,
        lua_max_parallel: 8,
        lua_max_per_guild: 2,
        lua_command_timeout_ms: 3000,
        music_allowed_hosts: Default::default(),
        trusted_proxies: Default::default(),
        offline_mode: true,
        discord_api_base_url: "https://discord.com/api/v10".into(),
        transcript_encryption_key: None,
        transcript_active_key_id: "v1".into(),
        transcript_keyring: None,
        ticket_transcripts_disabled: true,
        integration_credential_key: None,
        integration_credential_active_key_id: "v2".into(),
        integration_credential_keyring: Some(rotated_keyring()),
        integration_credentials_disabled: false,
        metrics_token: None,
        metrics_allowed_ips: Default::default(),
    }
}

async fn seed_legacy_credentials(pool: &PgPool) -> String {
    run_safe_migrations(pool).await.unwrap();
    let old_envelope = CredentialCrypto::new("v1", OLD_KEY)
        .encrypt_field(
            GUILD,
            "integration_connection",
            &MIXED_CONNECTION_ID.to_string(),
            "refresh_token",
            OLD_SECRET,
        )
        .unwrap();
    sqlx::query(
        r#"
        WITH seed AS (
            SELECT i, ('2000-01-01T00:00:00Z'::timestamptz + i * interval '1 second') AS created_at
            FROM generate_series(1, 2) AS i
        ),
        telegram AS (
            INSERT INTO telegram_bridges
                (id, guild_id, telegram_chat_id, discord_channel_id, secret_token, created_at)
            SELECT gen_random_uuid(), $1, '41' || i, '51' || i, $2, created_at FROM seed
        ),
        webhook AS (
            INSERT INTO integration_webhook_subscriptions
                (id, guild_id, provider, external_id, secret_token, target_channel_id, created_at)
            SELECT gen_random_uuid(), $1, 'github', 'webhook-' || i, $2, '61' || i, created_at
            FROM seed
            UNION ALL
            SELECT gen_random_uuid(), $1, 'github', 'webhook-no-secret', NULL, '613', now()
        ),
        gitlab AS (
            INSERT INTO gitlab_subscriptions
                (id, guild_id, host, project_id, project_path, secret_token, discord_channel_id, created_at)
            SELECT gen_random_uuid(), $1, 'gitlab.example', 'project-' || i, 'fixture/project',
                   $2, '71' || i, created_at FROM seed
        ),
        vk AS (
            INSERT INTO vk_subscriptions
                (id, guild_id, group_id, confirmation_code, secret_key, discord_channel_id, created_at)
            SELECT gen_random_uuid(), $1, 'group-' || i, 'fixture-confirmation', $2, '81' || i,
                   created_at FROM seed
        )
        INSERT INTO integration_connections
            (id, guild_id, provider, access_token, refresh_token, created_at)
        VALUES ($3, $1, 'mixed', $2, $4, '2000-01-01T00:00:01Z'),
               (gen_random_uuid(), $1, 'access-only', $2, NULL, '2000-01-01T00:00:02Z'),
               (gen_random_uuid(), $1, 'empty', NULL, NULL, '2000-01-01T00:00:03Z')
        "#,
    )
    .bind(GUILD)
    .bind(FIXTURE_SECRET)
    .bind(MIXED_CONNECTION_ID)
    .bind(&old_envelope)
    .execute(pool)
    .await
    .unwrap();
    old_envelope
}

async fn raw_credentials(pool: &PgPool) -> Vec<Option<String>> {
    sqlx::query_scalar(
        r#"
        SELECT credential FROM (
            SELECT 'telegram' AS entity_field, id, guild_id, secret_token AS credential
            FROM telegram_bridges
            UNION ALL SELECT 'webhook', id, guild_id, secret_token FROM integration_webhook_subscriptions
            UNION ALL SELECT 'access', id, guild_id, access_token FROM integration_connections
            UNION ALL SELECT 'refresh', id, guild_id, refresh_token FROM integration_connections
            UNION ALL SELECT 'gitlab', id, guild_id, secret_token FROM gitlab_subscriptions
            UNION ALL SELECT 'vk', id, guild_id, secret_key FROM vk_subscriptions
        ) AS fields WHERE guild_id = $1 ORDER BY entity_field, id
        "#,
    )
    .bind(GUILD)
    .fetch_all(pool)
    .await
    .unwrap()
}

fn assert_stored_credentials(stored: &[Option<String>], migrated: usize, old_envelope: &str) {
    assert_eq!(stored.len(), 15);
    let populated: Vec<&str> = stored.iter().filter_map(Option::as_deref).collect();
    assert_eq!(populated.len(), 11, "absent credentials must remain NULL");
    assert_eq!(
        populated
            .iter()
            .filter(|value| value.starts_with(ACTIVE_PREFIX))
            .count(),
        migrated,
        "each migrated credential must be sealed with the active key"
    );
    assert_eq!(
        populated
            .iter()
            .filter(|value| **value == FIXTURE_SECRET)
            .count(),
        10 - migrated,
        "raw SQL must find only the legacy credentials still awaiting backfill"
    );
    assert_eq!(
        populated
            .iter()
            .filter(|value| value.starts_with(OLD_PREFIX))
            .count(),
        1
    );
    assert!(
        populated.contains(&old_envelope),
        "backfill must preserve the existing old-key envelope byte for byte"
    );
}

async fn assert_legacy_credentials_readable(storage: &Storage) {
    let bridge = storage
        .get_telegram_bridge(GUILD, "511")
        .await
        .unwrap()
        .unwrap();
    assert!(bridge.secret_token == FIXTURE_SECRET, "Telegram round trip");

    let webhook = storage
        .list_webhook_subscriptions(GUILD, Some("github"))
        .await
        .unwrap();
    assert_eq!(webhook.len(), 3);
    assert!(
        webhook.iter().all(|row| {
            if row.external_id == "webhook-no-secret" {
                row.secret_token.is_none()
            } else {
                row.secret_token.as_deref() == Some(FIXTURE_SECRET)
            }
        }),
        "generic webhooks must retain their secrets and NULLs"
    );

    let connection = storage
        .get_integration_connection(GUILD, "mixed")
        .await
        .unwrap()
        .unwrap();
    assert!(
        connection.access_token.as_deref() == Some(FIXTURE_SECRET)
            && connection.refresh_token.as_deref() == Some(OLD_SECRET),
        "both backfilled and old-key fields must be readable in a mixed OAuth row"
    );
    let empty = storage
        .get_integration_connection(GUILD, "empty")
        .await
        .unwrap()
        .unwrap();
    assert!(empty.access_token.is_none() && empty.refresh_token.is_none());

    let gitlab = storage
        .get_gitlab_subscription(GUILD, "gitlab.example", "project-1")
        .await
        .unwrap()
        .unwrap();
    assert!(gitlab.secret_token == FIXTURE_SECRET, "GitLab round trip");

    let vk = storage
        .get_vk_subscription(GUILD, "group-1")
        .await
        .unwrap()
        .unwrap();
    assert!(vk.secret_key == FIXTURE_SECRET, "VK round trip");
}

#[tokio::test]
async fn postgres_bootstrap_encrypts_legacy_credentials_before_returning() {
    with_credential_database(|pool, database_url| async move {
        let old_envelope = seed_legacy_credentials(&pool).await;
        assert_stored_credentials(&raw_credentials(&pool).await, 0, &old_envelope);
        // A second guild spans more than one startup batch in a single table.
        sqlx::query(
            "INSERT INTO telegram_bridges
                (id, guild_id, telegram_chat_id, discord_channel_id, secret_token)
             SELECT gen_random_uuid(), $1, '41' || i, '51' || i, $2
             FROM generate_series(1, 101) AS i",
        )
        .bind(OTHER_GUILD)
        .bind(FIXTURE_SECRET)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO vk_subscriptions
                (id, guild_id, group_id, confirmation_code, secret_key, discord_channel_id)
             VALUES (gen_random_uuid(), $1, 'unsupported', 'fixture-confirmation', $2, '813')",
        )
        .bind(OTHER_GUILD)
        .bind(UNSUPPORTED_ENVELOPE)
        .execute(&pool)
        .await
        .unwrap();

        let state = AppState::bootstrap(offline_postgres_config(database_url))
            .await
            .expect("boot with the rotated credential keyring");

        assert_stored_credentials(&raw_credentials(&pool).await, 10, &old_envelope);
        let sealed_telegram_bridges: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM telegram_bridges
             WHERE guild_id = $1 AND secret_token LIKE 'enc:v1:v2:%'",
        )
        .bind(OTHER_GUILD)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            sealed_telegram_bridges, 101,
            "startup must drain later batches"
        );
        assert_legacy_credentials_readable(&state.storage).await;
        let unsupported_unchanged: bool = sqlx::query_scalar(
            "SELECT secret_key = $1 FROM vk_subscriptions WHERE guild_id = $2 AND group_id = 'unsupported'",
        )
        .bind(UNSUPPORTED_ENVELOPE)
        .bind(OTHER_GUILD)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(unsupported_unchanged, "startup must preserve unsupported envelopes");
        let error = state
            .storage
            .get_vk_subscription(OTHER_GUILD, "unsupported")
            .await
            .err()
            .expect("reading an unsupported envelope must fail");
        assert!(error.to_string().contains("unsupported integration credential envelope"));

        let new_id = Uuid::new_v4();
        let now = Timestamp::now();
        state
            .storage
            .upsert_integration_connection(IntegrationConnection {
                id: new_id,
                guild_id: GUILD.into(),
                provider: "rotated".into(),
                provider_account_id: None,
                access_token: Some(NEW_SECRET.into()),
                refresh_token: None,
                token_expires_at: None,
                scopes: serde_json::json!([]),
                metadata: serde_json::json!({}),
                enabled: true,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        let new_token_sealed: bool = sqlx::query_scalar(
            "SELECT access_token LIKE 'enc:v1:v2:%' AND refresh_token IS NULL
             FROM integration_connections WHERE id = $1",
        )
        .bind(new_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            new_token_sealed,
            "new writes must use the active rotation key"
        );

        let connection = state
            .storage
            .get_integration_connection(GUILD, "rotated")
            .await
            .unwrap()
            .unwrap();
        assert!(
            connection.access_token.as_deref() == Some(NEW_SECRET),
            "trusted storage must read a credential written after key rotation"
        );
    })
    .await;
}

#[tokio::test]
async fn postgres_credential_backfill_is_bounded_idempotent_and_rejects_future_envelopes() {
    with_credential_database(|pool, _| async move {
        let old_envelope = seed_legacy_credentials(&pool).await;
        let crypto = CredentialCrypto::from_keyring_spec("v2", &rotated_keyring()).unwrap();
        let storage = Storage::postgres_with_crypto(pool.clone(), Some(Arc::new(crypto)));

        let update_vk = "UPDATE vk_subscriptions SET secret_key = $1 WHERE group_id = 'group-2'";
        sqlx::query(update_vk)
            .bind(UNSUPPORTED_ENVELOPE)
            .execute(&pool)
            .await
            .unwrap();
        let before = raw_credentials(&pool).await;
        let error = storage
            .backfill_encrypt_integration_credentials(1)
            .await
            .expect_err("backfill must reject a future envelope in the last table");
        assert!(
            error
                .to_string()
                .contains("unsupported integration credential envelope")
        );
        assert!(
            raw_credentials(&pool).await == before,
            "failing backfill must leave all credential columns unchanged"
        );
        sqlx::query(update_vk)
            .bind(FIXTURE_SECRET)
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(
            storage
                .backfill_encrypt_integration_credentials(1)
                .await
                .unwrap(),
            5
        );
        assert_stored_credentials(&raw_credentials(&pool).await, 5, &old_envelope);
        assert_eq!(
            storage
                .backfill_encrypt_integration_credentials(1)
                .await
                .unwrap(),
            5
        );
        let migrated = raw_credentials(&pool).await;
        assert_stored_credentials(&migrated, 10, &old_envelope);
        assert_eq!(
            storage
                .backfill_encrypt_integration_credentials(1)
                .await
                .unwrap(),
            0
        );
        assert!(
            raw_credentials(&pool).await == migrated,
            "repeating backfill must retain all existing envelope bytes"
        );
        assert_legacy_credentials_readable(&storage).await;
    })
    .await;
}
