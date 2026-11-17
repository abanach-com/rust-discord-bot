use std::{future::Future, sync::Arc, time::Duration};

use rusty_discord_bot::{
    datetime::Timestamp,
    integrations::CredentialCrypto,
    storage::{
        GitLabSubscription, IntegrationConnection, IntegrationWebhookSubscription, Storage,
        TelegramBridge, VkSubscription, run_safe_migrations,
    },
};
use serde_json::json;
use sqlx::{PgPool, Postgres, migrate::MigrateDatabase};
use url::Url;
use uuid::Uuid;

const GUILD: &str = "100000000000000001";
const CHANNEL: &str = "200000000000000001";
const FIRST_SECRET: &str = "synthetic-first-credential";
const SECOND_SECRET: &str = "synthetic-second-credential";

async fn with_credential_database<F>(exercise: impl FnOnce(PgPool, Storage) -> F + Send + 'static)
where
    F: Future<Output = ()> + Send + 'static,
{
    let Some(database_url) = std::env::var("DATABASE_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())
    else {
        eprintln!("SKIPPING PostgreSQL integration credential test: DATABASE_URL not set");
        return;
    };
    let mut url = Url::parse(&database_url).expect("parse PostgreSQL test URL");
    url.set_path(&format!("/credential_storage_{}", Uuid::new_v4().simple()));
    Postgres::create_database(url.as_str())
        .await
        .expect("create isolated credential test database");
    let pool = PgPool::connect(url.as_str())
        .await
        .expect("connect to isolated credential test database");
    let storage = Storage::postgres_with_crypto(
        pool.clone(),
        Some(Arc::new(CredentialCrypto::generate_ephemeral())),
    );
    let test_pool = pool.clone();
    let test = tokio::spawn(async move {
        run_safe_migrations(&test_pool)
            .await
            .expect("migrate credential test database");
        exercise(test_pool, storage).await;
    })
    .await;
    pool.close().await;
    Postgres::drop_database(url.as_str())
        .await
        .expect("drop isolated credential test database");
    test.expect("credential test failed");
}

fn gitlab(secret: &str) -> GitLabSubscription {
    GitLabSubscription {
        id: Uuid::new_v4(),
        guild_id: GUILD.into(),
        host: "gitlab.example.test".into(),
        project_id: "123".into(),
        project_path: "example/project".into(),
        secret_token: secret.into(),
        discord_channel_id: CHANNEL.into(),
        created_at: Timestamp::now(),
    }
}

fn vk(secret: &str) -> VkSubscription {
    VkSubscription {
        id: Uuid::new_v4(),
        guild_id: GUILD.into(),
        group_id: "123".into(),
        confirmation_code: "synthetic-confirmation".into(),
        secret_key: secret.into(),
        discord_channel_id: CHANNEL.into(),
        created_at: Timestamp::now(),
    }
}

fn webhook(secret: Option<&str>) -> IntegrationWebhookSubscription {
    IntegrationWebhookSubscription {
        id: Uuid::new_v4(),
        guild_id: GUILD.into(),
        provider: "fixture-provider".into(),
        external_id: "fixture-external-id".into(),
        secret_token: secret.map(str::to_owned),
        target_channel_id: CHANNEL.into(),
        config: json!({}),
        active: true,
        created_at: Timestamp::now(),
    }
}

async fn assert_encrypted(pool: &PgPool, query: &'static str, id: Uuid, field: &str) {
    let is_encrypted: bool = sqlx::query_scalar(query)
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("query credential column");
    assert!(
        is_encrypted,
        "{field} was stored without an encrypted envelope"
    );
}

#[tokio::test]
async fn postgres_gitlab_and_vk_credentials_are_encrypted() {
    with_credential_database(|pool, storage| async move {
        let (first_gitlab, second_gitlab) = race_first_upserts(
            &pool,
            "gitlab_subscriptions",
            storage.upsert_gitlab_subscription(gitlab(FIRST_SECRET)),
            storage.upsert_gitlab_subscription(gitlab(SECOND_SECRET)),
        )
        .await;
        let first_gitlab = first_gitlab.expect("first GitLab upsert");
        let second_gitlab = second_gitlab.expect("second GitLab upsert");
        assert_eq!(first_gitlab.id, second_gitlab.id);
        assert_eq!(first_gitlab.created_at, second_gitlab.created_at);
        assert!(first_gitlab.secret_token == FIRST_SECRET);
        assert!(second_gitlab.secret_token == SECOND_SECRET);
        assert_encrypted(
            &pool,
            "SELECT secret_token LIKE 'enc:v1:%' FROM gitlab_subscriptions WHERE id = $1",
            first_gitlab.id,
            "GitLab secret token",
        )
        .await;

        let (first_vk, second_vk) = race_first_upserts(
            &pool,
            "vk_subscriptions",
            storage.upsert_vk_subscription(vk(FIRST_SECRET)),
            storage.upsert_vk_subscription(vk(SECOND_SECRET)),
        )
        .await;
        let first_vk = first_vk.expect("first VK upsert");
        let second_vk = second_vk.expect("second VK upsert");
        assert_eq!(first_vk.id, second_vk.id);
        assert_eq!(first_vk.created_at, second_vk.created_at);
        assert!(first_vk.secret_key == FIRST_SECRET);
        assert!(second_vk.secret_key == SECOND_SECRET);
        assert_encrypted(
            &pool,
            "SELECT secret_key LIKE 'enc:v1:%' FROM vk_subscriptions WHERE id = $1",
            first_vk.id,
            "VK secret key",
        )
        .await;

        let mut replacement_gitlab = gitlab(FIRST_SECRET);
        replacement_gitlab.project_path = "example/updated-project".into();
        let mut replacement_vk = vk(FIRST_SECRET);
        replacement_vk.confirmation_code = "synthetic-updated-confirmation".into();
        let updated_gitlab = storage
            .upsert_gitlab_subscription(replacement_gitlab.clone())
            .await
            .expect("update GitLab subscription");
        let updated_vk = storage
            .upsert_vk_subscription(replacement_vk.clone())
            .await
            .expect("update VK subscription");
        assert_eq!(updated_gitlab.id, first_gitlab.id);
        assert_eq!(updated_vk.id, first_vk.id);
        let stored_gitlab = storage
            .get_gitlab_subscription(GUILD, &updated_gitlab.host, &updated_gitlab.project_id)
            .await
            .expect("decrypt GitLab subscription")
            .expect("find GitLab subscription");
        let stored_vk = storage
            .get_vk_subscription(GUILD, &updated_vk.group_id)
            .await
            .expect("decrypt VK subscription")
            .expect("find VK subscription");
        assert!(stored_gitlab.secret_token == FIRST_SECRET);
        assert!(stored_vk.secret_key == FIRST_SECRET);
        assert_eq!(stored_gitlab.project_path, replacement_gitlab.project_path);
        assert!(stored_vk.confirmation_code == replacement_vk.confirmation_code);

        let unkeyed = Storage::postgres(pool.clone());
        assert!(
            unkeyed
                .get_gitlab_subscription(GUILD, &stored_gitlab.host, &stored_gitlab.project_id)
                .await
                .is_err()
        );
        assert!(
            unkeyed
                .get_vk_subscription(GUILD, &stored_vk.group_id)
                .await
                .is_err()
        );
        assert!(
            unkeyed
                .upsert_gitlab_subscription(gitlab(SECOND_SECRET))
                .await
                .is_err()
        );
        assert!(
            unkeyed
                .upsert_vk_subscription(vk(SECOND_SECRET))
                .await
                .is_err()
        );
    })
    .await;
}

// SHARE permits SELECT FOR UPDATE to see an empty result, and blocks both
// INSERTs. A writer serializing the natural key can also wait on its
// advisory lock while the first INSERT is blocked.
async fn race_first_upserts<A, B>(
    pool: &PgPool,
    table: &str,
    first: impl Future<Output = A>,
    second: impl Future<Output = B>,
) -> (A, B) {
    let mut blocker = pool.begin().await.expect("open lock transaction");
    let lock_sql = match table {
        "integration_connections" => "LOCK TABLE integration_connections IN SHARE MODE",
        "integration_webhook_subscriptions" => {
            "LOCK TABLE integration_webhook_subscriptions IN SHARE MODE"
        }
        "telegram_bridges" => "LOCK TABLE telegram_bridges IN SHARE MODE",
        "gitlab_subscriptions" => "LOCK TABLE gitlab_subscriptions IN SHARE MODE",
        "vk_subscriptions" => "LOCK TABLE vk_subscriptions IN SHARE MODE",
        _ => unreachable!("unsupported credential test relation"),
    };
    sqlx::query(lock_sql)
        .execute(&mut *blocker)
        .await
        .expect("lock table for the credential upsert race");
    let release = async {
        let blocked = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let waiting: i64 = sqlx::query_scalar(
                    "SELECT count(DISTINCT pid) FROM pg_locks
                     WHERE NOT granted AND database = (
                         SELECT oid FROM pg_database WHERE datname = current_database()
                     ) AND (locktype = 'advisory' OR relation = $1::text::regclass)",
                )
                .bind(table)
                .fetch_one(pool)
                .await
                .expect("observe waiting PostgreSQL upserts");
                if waiting == 2 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        blocker.commit().await.expect("release the write barrier");
        blocked.expect("both upserts must reach the write barrier");
    };
    let (first_result, second_result, ()) = tokio::join!(first, second, release);
    (first_result, second_result)
}

#[tokio::test]
async fn postgres_webhook_first_upserts_share_persisted_aad() {
    with_credential_database(|pool, storage| async move {
        let first = webhook(Some(FIRST_SECRET));
        let second = webhook(Some(SECOND_SECRET));

        let (first_result, second_result) = race_first_upserts(
            &pool,
            "integration_webhook_subscriptions",
            storage.create_webhook_subscription(first.clone()),
            storage.create_webhook_subscription(second.clone()),
        )
        .await;

        let first_saved = first_result.expect("first upsert must return a readable credential");
        let second_saved = second_result.expect("second upsert must return a readable credential");
        assert_eq!(first_saved.id, second_saved.id);
        assert!(first_saved.id == first.id || first_saved.id == second.id);
        assert!(first_saved.secret_token.as_deref() == Some(FIRST_SECRET));
        assert!(second_saved.secret_token.as_deref() == Some(SECOND_SECRET));

        let rows = storage
            .list_webhook_subscriptions(GUILD, Some(&first.provider))
            .await
            .expect("decrypt the final webhook subscription");
        assert_eq!(rows.len(), 1);
        let saved = &rows[0];
        assert_eq!(saved.id, first_saved.id);
        let final_secret = if saved.id == first.id {
            SECOND_SECRET
        } else {
            FIRST_SECRET
        };
        assert!(saved.secret_token.as_deref() == Some(final_secret));
        assert_encrypted(
            &pool,
            "SELECT secret_token LIKE 'enc:v1:%' FROM integration_webhook_subscriptions WHERE id = $1",
            saved.id,
            "webhook secret token",
        )
        .await;
        let unkeyed = Storage::postgres(pool.clone());
        assert!(unkeyed.list_webhook_subscriptions(GUILD, None).await.is_err());
        assert!(
            unkeyed
                .create_webhook_subscription(webhook(Some(SECOND_SECRET)))
                .await
                .is_err()
        );

        let cleared = storage
            .create_webhook_subscription(webhook(None))
            .await
            .expect("clear the optional webhook credential");
        assert_eq!(cleared.id, saved.id);
        assert!(cleared.secret_token.is_none());
        let raw_null: bool = sqlx::query_scalar(
            "SELECT secret_token IS NULL FROM integration_webhook_subscriptions WHERE id = $1",
        )
        .bind(cleared.id)
        .fetch_one(&pool)
        .await
        .expect("read nullable webhook credential column");
        assert!(raw_null);
    })
    .await;
}

fn telegram(secret: &str) -> TelegramBridge {
    TelegramBridge {
        id: Uuid::new_v4(),
        guild_id: GUILD.into(),
        telegram_chat_id: "-100123456789".into(),
        discord_channel_id: CHANNEL.into(),
        secret_token: secret.into(),
        paused: false,
        created_at: Timestamp::now(),
    }
}

fn connection(access_token: Option<&str>) -> IntegrationConnection {
    IntegrationConnection {
        id: Uuid::new_v4(),
        guild_id: GUILD.into(),
        provider: "fixture-provider".into(),
        provider_account_id: None,
        access_token: access_token.map(str::to_owned),
        refresh_token: Some("synthetic-refresh-credential".into()),
        token_expires_at: None,
        scopes: json!([]),
        metadata: json!({}),
        enabled: true,
        created_at: Timestamp::now(),
        updated_at: Timestamp::now(),
    }
}

#[tokio::test]
async fn postgres_telegram_upserts_encrypt_and_share_persisted_aad() {
    with_credential_database(|pool, storage| async move {
        let first = telegram(FIRST_SECRET);
        let second = telegram(SECOND_SECRET);
        let (first_result, second_result) = race_first_upserts(
            &pool,
            "telegram_bridges",
            storage.create_telegram_bridge(first.clone()),
            storage.create_telegram_bridge(second.clone()),
        )
        .await;
        let first_saved = first_result.expect("first Telegram upsert");
        let second_saved = second_result.expect("second Telegram upsert");
        assert_eq!(first_saved.id, second_saved.id);
        assert_eq!(first_saved.created_at, second_saved.created_at);
        assert!(first_saved.secret_token == FIRST_SECRET);
        assert!(second_saved.secret_token == SECOND_SECRET);

        let rows = storage
            .list_telegram_bridges(GUILD)
            .await
            .expect("list decrypted Telegram bridges");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, first_saved.id);

        let mut replacement = telegram(FIRST_SECRET);
        replacement.telegram_chat_id = "-100123456790".into();
        replacement.paused = true;
        let saved = storage
            .create_telegram_bridge(replacement)
            .await
            .expect("update Telegram bridge");
        assert_eq!(saved.id, first_saved.id);
        assert_eq!(saved.created_at, first_saved.created_at);
        assert_encrypted(
            &pool,
            "SELECT secret_token LIKE 'enc:v1:%' FROM telegram_bridges WHERE id = $1",
            saved.id,
            "Telegram secret token",
        )
        .await;
        let stored = storage
            .get_telegram_bridge(GUILD, CHANNEL)
            .await
            .expect("decrypt Telegram bridge")
            .expect("find Telegram bridge");
        assert!(stored.secret_token == FIRST_SECRET);
        assert!(stored.paused);
        assert_eq!(stored.telegram_chat_id, "-100123456790");

        let unkeyed = Storage::postgres(pool.clone());
        assert!(
            unkeyed
                .create_telegram_bridge(telegram(SECOND_SECRET))
                .await
                .is_err()
        );
        assert!(unkeyed.get_telegram_bridge(GUILD, CHANNEL).await.is_err());
        assert!(unkeyed.list_telegram_bridges(GUILD).await.is_err());
        let stored = storage
            .get_telegram_bridge(GUILD, CHANNEL)
            .await
            .expect("read after rejecting no-key overwrite")
            .expect("find Telegram bridge after rejecting no-key overwrite");
        assert!(stored.secret_token == FIRST_SECRET);
    })
    .await;
}

#[tokio::test]
async fn postgres_connection_upserts_encrypt_and_share_persisted_aad() {
    with_credential_database(|pool, storage| async move {
        let first = connection(Some(FIRST_SECRET));
        let second = connection(Some(SECOND_SECRET));
        let (first_result, second_result) = race_first_upserts(
            &pool,
            "integration_connections",
            storage.upsert_integration_connection(first.clone()),
            storage.upsert_integration_connection(second.clone()),
        )
        .await;
        let first_saved = first_result.expect("first connection upsert");
        let second_saved = second_result.expect("second connection upsert");
        assert_eq!(first_saved.id, second_saved.id);
        assert_eq!(first_saved.created_at, second_saved.created_at);
        assert!(first_saved.access_token.as_deref() == Some(FIRST_SECRET));
        assert!(second_saved.access_token.as_deref() == Some(SECOND_SECRET));
        assert_encrypted(
            &pool,
            "SELECT access_token LIKE 'enc:v1:%' AND refresh_token LIKE 'enc:v1:%'
             FROM integration_connections WHERE id = $1",
            first_saved.id,
            "connection access and refresh tokens",
        )
        .await;
        let rows = storage
            .list_integration_connections(GUILD)
            .await
            .expect("list decrypted connections");
        assert_eq!(rows.len(), 1);
        let final_secret = if rows[0].id == first.id {
            SECOND_SECRET
        } else {
            FIRST_SECRET
        };
        assert!(rows[0].access_token.as_deref() == Some(final_secret));
        assert!(rows[0].refresh_token == first.refresh_token);

        let unkeyed = Storage::postgres(pool.clone());
        assert!(
            unkeyed
                .get_integration_connection(GUILD, &first.provider)
                .await
                .is_err()
        );
        assert!(unkeyed.list_integration_connections(GUILD).await.is_err());
        assert!(
            unkeyed
                .upsert_integration_connection(connection(Some(SECOND_SECRET)))
                .await
                .is_err()
        );

        let mut cleared = connection(None);
        cleared.refresh_token = None;
        let stored = storage
            .upsert_integration_connection(cleared)
            .await
            .expect("clear both connection tokens");
        assert_eq!(stored.id, first_saved.id);
        assert!(stored.access_token.is_none());
        assert!(stored.refresh_token.is_none());
        let raw_nulls: bool = sqlx::query_scalar(
            "SELECT access_token IS NULL AND refresh_token IS NULL
             FROM integration_connections WHERE id = $1",
        )
        .bind(stored.id)
        .fetch_one(&pool)
        .await
        .expect("read nullable credential columns");
        assert!(raw_nulls);
    })
    .await;
}

#[tokio::test]
async fn postgres_connection_uuid_collision_cannot_overwrite_another_tenant() {
    with_credential_database(|_, storage| async move {
        let stored = storage
            .upsert_integration_connection(connection(Some(FIRST_SECRET)))
            .await
            .expect("create owner connection");
        let mut collision = connection(Some(SECOND_SECRET));
        collision.id = stored.id;
        collision.guild_id = "100000000000000002".into();
        assert!(
            storage
                .upsert_integration_connection(collision)
                .await
                .is_err(),
            "must reject a caller UUID owned by another natural key"
        );
        let original = storage
            .get_integration_connection(GUILD, &stored.provider)
            .await
            .expect("owner credential still decrypts after rejected UUID collision")
            .expect("owner record remains present");
        assert_eq!(original.id, stored.id);
        assert!(original.access_token.as_deref() == Some(FIRST_SECRET));
    })
    .await;
}

#[tokio::test]
async fn postgres_credential_reads_fail_closed_on_identity_and_field_mismatch() {
    with_credential_database(|pool, storage| async move {
        let first = storage
            .upsert_integration_connection(connection(Some(FIRST_SECRET)))
            .await
            .expect("create original connection");
        let mut other = connection(Some(SECOND_SECRET));
        other.provider = "other-fixture-provider".into();
        let other = storage
            .upsert_integration_connection(other)
            .await
            .expect("create second connection");
        // Copy between records in the same tenant, with identical entity and field.
        sqlx::query(
            "UPDATE integration_connections AS target SET access_token = source.access_token
             FROM integration_connections AS source WHERE source.id = $1 AND target.id = $2",
        )
        .bind(first.id)
        .bind(other.id)
        .execute(&pool)
        .await
        .expect("copy encrypted credential to another record");
        assert!(
            storage
                .get_integration_connection(GUILD, &other.provider)
                .await
                .is_err()
        );

        // Move the affected row across tenants while keeping its ID, entity and field.
        let other_guild = "100000000000000002";
        sqlx::query("UPDATE integration_connections SET guild_id = $1 WHERE id = $2")
            .bind(other_guild)
            .bind(first.id)
            .execute(&pool)
            .await
            .expect("move connection fixture to another guild");
        assert!(
            storage
                .get_integration_connection(other_guild, &first.provider)
                .await
                .is_err()
        );

        // Restore the tenant but copy the ciphertext across fields in the same row.
        sqlx::query(
            "UPDATE integration_connections SET guild_id = $1, refresh_token = access_token
             WHERE id = $2",
        )
        .bind(GUILD)
        .bind(first.id)
        .execute(&pool)
        .await
        .expect("copy credential between fields");
        assert!(
            storage
                .get_integration_connection(GUILD, &first.provider)
                .await
                .is_err()
        );
        assert!(storage.list_integration_connections(GUILD).await.is_err());
    })
    .await;
}

#[tokio::test]
async fn postgres_telegram_rejects_malformed_unknown_and_tampered_envelopes() {
    with_credential_database(|pool, storage| async move {
        let row = storage
            .create_telegram_bridge(telegram(FIRST_SECRET))
            .await
            .expect("create Telegram bridge");
        let envelope: String =
            sqlx::query_scalar("SELECT secret_token FROM telegram_bridges WHERE id = $1")
                .bind(row.id)
                .fetch_one(&pool)
                .await
                .expect("fetch fixture envelope from PostgreSQL");
        let mut tampered = envelope.clone();
        let last = tampered.pop().expect("nonempty fixture envelope");
        tampered.push(if last == '0' { '1' } else { '0' });
        for invalid in [
            "enc:v1:malformed".to_owned(),
            "enc:v99:unsupported".to_owned(),
            envelope.replacen("enc:v1:ephemeral:", "enc:v1:unknown-key:", 1),
            tampered,
        ] {
            sqlx::query("UPDATE telegram_bridges SET secret_token = $1 WHERE id = $2")
                .bind(invalid)
                .bind(row.id)
                .execute(&pool)
                .await
                .expect("replace fixture envelope");
            assert!(storage.get_telegram_bridge(GUILD, CHANNEL).await.is_err());
            assert!(storage.list_telegram_bridges(GUILD).await.is_err());
        }
    })
    .await;
}
