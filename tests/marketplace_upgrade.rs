use std::{future::Future, time::Duration};

use rusty_discord_bot::storage::run_safe_migrations;
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, migrate::MigrateDatabase};
use url::Url;
use uuid::Uuid;

// SHA-384 of the released 0067 SQL, as recorded by SQLx.
const RELEASED_0067_CHECKSUM: &str = "dfc7e107980f8819bf905bf0a96f61ff78bfb1ad2cbe66d261727108f7488db8669f0154806b88be893f578ace8b9d26";

const LIVE_INSTALLATIONS: &str =
    "SELECT to_jsonb(i) FROM plugin_guild_installations i ORDER BY guild_id";
const QUARANTINED_INSTALLATIONS: &str =
    "SELECT to_jsonb(i) FROM plugin_guild_installation_quarantine i ORDER BY guild_id";

async fn with_upgrade_database<F>(exercise: impl FnOnce(PgPool) -> F)
where
    F: Future<Output = ()> + Send + 'static,
{
    let Some(database_url) = std::env::var("DATABASE_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())
    else {
        eprintln!("SKIPPING PostgreSQL Marketplace upgrade test: DATABASE_URL not set");
        return;
    };
    // A separate database keeps historical migrations and fixtures isolated
    // from the shared DATABASE_URL database used by the other integration tests.
    let mut url = Url::parse(&database_url).expect("parse PostgreSQL test URL");
    url.set_path(&format!("/marketplace_upgrade_{}", Uuid::new_v4().simple()));
    Postgres::create_database(url.as_str())
        .await
        .expect("create isolated PostgreSQL test database");
    let pool = PgPool::connect(url.as_str())
        .await
        .expect("connect to isolated PostgreSQL test database");

    let test_result = tokio::spawn(exercise(pool.clone())).await;
    pool.close().await;
    Postgres::drop_database(url.as_str())
        .await
        .expect("drop isolated PostgreSQL test database");
    test_result.expect("Marketplace upgrade test failed");
}

async fn seed_legacy_installations(pool: &PgPool) -> Vec<Value> {
    sqlx::migrate!("./migrations")
        .run_to(66, pool)
        .await
        .expect("migrate to the last version before immutable releases");
    sqlx::raw_sql(
        r#"
        INSERT INTO plugin_marketplace_packages
            (id, plugin_id, name, version, author, sha256_hash, manifest, is_verified)
        VALUES ('00000000-0000-0000-0000-000000000001', 'upgrade-fixture', 'Upgrade fixture',
            '1.0.0', 'fixture-author', repeat('1', 64), '{"release":1}', true);

        INSERT INTO plugin_guild_installations
            (id, guild_id, plugin_id, version, enabled, config, installed_at)
        VALUES ('00000000-0000-0000-0000-000000000011', '100000000000000001', 'upgrade-fixture',
            '1.0.0', true, '{"legacy": {"channels": ["11", "12"]}}', '2026-10-01T01:01:01.123456Z');

        UPDATE plugin_marketplace_packages
        SET version = '2.0.0', sha256_hash = repeat('2', 64), manifest = '{"release":2}'
        WHERE plugin_id = 'upgrade-fixture';

        INSERT INTO plugin_guild_installations
            (id, guild_id, plugin_id, version, enabled, config, installed_at)
        VALUES ('00000000-0000-0000-0000-000000000022', '100000000000000002', 'upgrade-fixture',
            '2.0.0', false, '{"legacy": ["disabled", null]}', '2026-10-02T02:02:02.234567Z');

        UPDATE plugin_marketplace_packages
        SET version = '3.0.0', sha256_hash = repeat('3', 64), manifest = '{"release":3}'
        WHERE plugin_id = 'upgrade-fixture';

        INSERT INTO plugin_guild_installations
            (id, guild_id, plugin_id, version, enabled, config, installed_at)
        VALUES ('00000000-0000-0000-0000-000000000033', '100000000000000003', 'upgrade-fixture',
            '3.0.0', true, '{"current": true}', '2026-10-03T03:03:03.345678Z');
        "#,
    )
    .execute(pool)
    .await
    .expect("seed historical and current Marketplace installations");
    live_installations(pool).await
}

async fn live_installations(pool: &PgPool) -> Vec<Value> {
    sqlx::query_scalar(LIVE_INSTALLATIONS)
        .fetch_all(pool)
        .await
        .expect("read active Marketplace installations")
}

async fn assert_installations_preserved(pool: &PgPool, original: &[Value]) {
    let archive_exists: bool = sqlx::query_scalar(
        "SELECT to_regclass('plugin_guild_installation_quarantine') IS NOT NULL",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(
        archive_exists,
        "migration 0067 discarded older installations without preserving their configuration"
    );
    let quarantined: Vec<Value> = sqlx::query_scalar(QUARANTINED_INSTALLATIONS)
        .fetch_all(pool)
        .await
        .expect("read quarantined Marketplace installations");
    assert_eq!(quarantined, original[..2]);
    assert_eq!(live_installations(pool).await, original[2..]);
}

#[tokio::test]
async fn upgrades_legacy_marketplace_installations_without_data_loss() {
    with_upgrade_database(|pool| async move {
        let original = seed_legacy_installations(&pool).await;
        assert_eq!(original.len(), 3);

        run_safe_migrations(&pool)
            .await
            .expect("upgrade pre-0067 database");
        assert_installations_preserved(&pool, &original).await;

        let checksum: String = sqlx::query_scalar(
            "SELECT encode(checksum, 'hex') FROM _sqlx_migrations WHERE version = 67 AND success",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(checksum, RELEASED_0067_CHECKSUM);

        let releases: Vec<(String, String, Value, bool)> = sqlx::query_as(
            "SELECT version, sha256_hash, manifest, is_verified FROM plugin_marketplace_releases
             WHERE plugin_id = 'upgrade-fixture'",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            releases,
            [("3.0.0".into(), "3".repeat(64), json!({"release": 3}), true)],
            "the catalog must keep the real release without fabricating historical artifacts"
        );

        let rejected = sqlx::query(
            "INSERT INTO plugin_guild_installations (id, guild_id, plugin_id, version)
             VALUES (gen_random_uuid(), '100000000000000004', 'upgrade-fixture', '1.0.0')",
        )
        .execute(&pool)
        .await
        .expect_err("a version with no release cannot become an active installation");
        assert!(
            rejected
                .as_database_error()
                .is_some_and(|error| error.is_foreign_key_violation())
        );

        run_safe_migrations(&pool).await.expect("repeated startup");
        assert_installations_preserved(&pool, &original).await;
    })
    .await;
}

#[tokio::test]
async fn failed_marketplace_upgrade_preserves_original_rows_and_can_retry() {
    with_upgrade_database(|pool| async move {
        let original = seed_legacy_installations(&pool).await;
        // An AFTER STATEMENT trigger aborts after the row deletes and their
        // row-level triggers, verifying that the migration can roll them back.
        sqlx::raw_sql(
            r#"
            CREATE FUNCTION test_fail_marketplace_delete() RETURNS trigger LANGUAGE plpgsql AS $$
            BEGIN
                RAISE EXCEPTION 'injected Marketplace upgrade failure';
            END;
            $$;
            CREATE TRIGGER test_fail_marketplace_delete AFTER DELETE ON plugin_guild_installations
            FOR EACH STATEMENT EXECUTE FUNCTION test_fail_marketplace_delete();
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        let error = run_safe_migrations(&pool)
            .await
            .expect_err("0067 must fail with the injected PostgreSQL error");
        assert!(
            error
                .to_string()
                .contains("injected Marketplace upgrade failure")
        );
        assert_eq!(live_installations(&pool).await, original);

        let migration_committed: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM _sqlx_migrations WHERE version = 67)")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!migration_committed);
        let archive_exists: bool = sqlx::query_scalar(
            "SELECT to_regclass('plugin_guild_installation_quarantine') IS NOT NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        if archive_exists {
            let archived: Vec<Value> = sqlx::query_scalar(QUARANTINED_INSTALLATIONS)
                .fetch_all(&pool)
                .await
                .unwrap();
            assert!(
                archived.is_empty(),
                "failed upgrade must roll back the archival"
            );
        }

        sqlx::query("DROP TRIGGER test_fail_marketplace_delete ON plugin_guild_installations")
            .execute(&pool)
            .await
            .unwrap();
        let (first, second) = tokio::time::timeout(Duration::from_secs(60), async {
            tokio::join!(run_safe_migrations(&pool), run_safe_migrations(&pool))
        })
        .await
        .expect("concurrent migration retries must finish without holding a stale lock");
        first.expect("first concurrent retry of 0067");
        second.expect("second concurrent retry of 0067");
        assert_installations_preserved(&pool, &original).await;
    })
    .await;
}

#[tokio::test]
async fn upgrades_already_migrated_databases_and_rejects_a_modified_checksum() {
    with_upgrade_database(|pool| async move {
        let original = seed_legacy_installations(&pool).await;
        // Simulate a deployment that crossed 0067 with the original migrator.
        sqlx::migrate!("./migrations")
            .run_to(68, &pool)
            .await
            .expect("apply the released upgrade without the capture step");
        assert_eq!(live_installations(&pool).await, original[2..]);

        run_safe_migrations(&pool)
            .await
            .expect("upgrade an existing post-0067 database");
        assert_eq!(live_installations(&pool).await, original[2..]);
        let archived: Vec<Value> = sqlx::query_scalar(QUARANTINED_INSTALLATIONS)
            .fetch_all(&pool)
            .await
            .unwrap();
        assert!(
            archived.is_empty(),
            "missing legacy rows cannot be recovered"
        );

        // The original 0067 checksum is an integrity check on this upgrade.
        sqlx::query(
            "UPDATE _sqlx_migrations SET checksum = decode('deadbeef', 'hex') WHERE version = 67",
        )
        .execute(&pool)
        .await
        .unwrap();
        let error = run_safe_migrations(&pool)
            .await
            .expect_err("a changed 0067 checksum must not be accepted");
        assert!(matches!(
            error,
            sqlx::migrate::MigrateError::VersionMismatch(67)
        ));
        let checksum: String = sqlx::query_scalar(
            "SELECT encode(checksum, 'hex') FROM _sqlx_migrations WHERE version = 67",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(checksum, "deadbeef");
    })
    .await;
}
