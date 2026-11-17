# Development guide

## Toolchain

The repository pins Rust 1.100.0 through `rust-toolchain.toml`. CI verifies that this exact version matches Rust's official stable channel before compiling the project.

System packages commonly required on Debian/Ubuntu:

```bash
sudo apt-get update
sudo apt-get install --yes build-essential cmake libopus-dev pkg-config ffmpeg yt-dlp
```

The production container does not trust the distribution's potentially older `yt-dlp` package. It downloads the audited upstream release named in `Dockerfile` and verifies its SHA-256 digest before installation.

See [`VERSION_POLICY.md`](VERSION_POLICY.md) before changing Rust, crate, container, database or CI versions.

## Initial setup

You can develop using either full Docker Compose or running the bot locally with `cargo run`.

### Variant 1: Full Docker Compose

Runs both PostgreSQL and the bot inside the isolated Compose network:

```bash
cp .env.example .env
# Fill in your Discord bot credentials in .env
docker compose up -d
```

### Variant 2: Local `cargo run` with Compose PostgreSQL

Runs PostgreSQL in Docker with port `5432` mapped to `127.0.0.1` on the host, while running the bot directly:

```bash
cp .env.example .env
cp compose.override.yaml.example compose.override.yaml
# In .env, set: DATABASE_URL=postgres://zuckerbot:zuckerbot@127.0.0.1:5432/zuckerbot
# And fill in your Discord credentials
docker compose up -d postgres
cargo run
```

In the base deployment (`compose.yaml`), PostgreSQL's port remains strictly internal to the Compose network.

For fast command iteration, set `DISCORD_DEV_GUILD_ID`. Guild commands update immediately; global command propagation is intentionally not used during normal development.

## Quality gate

Run before every pull request:

```bash
bash scripts/check-latest-rust.sh
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release
docker build --tag zuckerbot:local .
```

CI runs the same Rust checks and builds the production container independently.

The Rust job uses GitHub's Ubuntu 24.04 image. A failed package setup preserves
`apt-install.log` in the Rust diagnostics artifact, including the runner image,
APT source hosts and bounded DNS checks. Formatting and dashboard checks run
before package installation. Run `bash tests/ci_apt_setup.sh` on Ubuntu to
rehearse an interrupted index refresh without changing installed packages.

## Adding a Lua module

1. Add a `.lua` file under `scripts/`.
2. Return a valid module table.
3. Use unique module and command IDs.
4. Define a configuration schema, even if it is an empty object.
5. Return only documented actions.
6. Add a test that loads the module and exercises its important branches.
7. Document required Discord intents and permissions.
8. Add the capability to the feature matrix.

The process refuses to start if any module has an invalid manifest or duplicate command.

## Adding a Rust action

A new action is a security-sensitive change.

Required implementation areas:

- `src/lua/model.rs`: serialized contract and local input validation
- `src/discord/executor.rs`: permission, hierarchy and execution logic
- storage: audit record or durable state when relevant
- `docs/LUA_API.md`: public contract
- tests: valid, invalid, unauthorized and boundary cases
- dashboard/schema support when configurable

Never expose a raw Serenity `Context`, HTTP client, SQL pool or filesystem handle to Lua.

## Adding a Gateway event

1. Add the Serenity handler.
2. Convert the event into a stable `LuaEventContext`.
3. Expose Discord IDs as strings.
4. Include only data needed by modules.
5. Document privileged intents.
6. Define whether event actions run as automation or on behalf of an actor.
7. Add redaction and retention decisions for logged data.

Avoid serializing complete Serenity objects. That would couple Lua modules to library internals and expose data unintentionally.

## Database migrations

Add ordered SQL files under `migrations/`.

Rules:

- migrations must be forward-only once released
- use explicit indexes for query paths
- avoid storing JavaScript-unsafe snowflakes as numeric values
- include retention or cleanup strategy for high-volume tables
- separate runtime and migration database roles in production

The supplied Compose stack currently targets PostgreSQL 18.6. PostgreSQL 18 changed the official container's volume layout, so persistent data is mounted at `/var/lib/postgresql`, not the pre-18 `/var/lib/postgresql/data` location.

### Upgrading Marketplace installations past migration 0067

Use the upgraded application's PostgreSQL startup migrator for existing databases. The CI job that executes the SQL files directly validates a fresh, disposable schema. Running those files directly on an existing database bypasses the preservation step.

1. Stop the old application instances and any other Marketplace writers or migration jobs. Take a PostgreSQL backup and verify that it restores to a separate database. Prepare the PostgreSQL credential configuration for the new binary. [Startup preflight and encryption settings](SECURITY.md#application-layer-encryption-at-rest-and-deployment-upgrades)
2. Rehearse startup against the restored copy with `OFFLINE_MODE=true` and external network access blocked. During startup, the migrator holds the SQLx migration lock. If migration `0067` is pending, the migrator applies preceding migrations and then archives the exact rows that the original `0067` would delete. The archive writes, `0067` and trigger cleanup run in a single transaction. A failure in that transaction leaves the original installations available for retry.
3. On the restored copy, check that `plugin_guild_installations` still contains the installations with a matching release and that `plugin_guild_installation_quarantine` contains the original rows for the missing versions. Treat exported configuration as private guild data.
4. After verifying the rehearsal, start the upgraded binary against the target database and check the same results before bringing the remaining instances back online. Guild administrators can review the disabled entries through the read-only `GET /api/guilds/{guild_id}/marketplace/unresolved` endpoint. It requires permission to manage that guild and returns `status: unresolved`, `enabled: false`, `original_enabled`, and the original UUID, pinned version, configuration and installation timestamp. Archived entries remain inactive even if a guild later installs an available release.

#### Recovering installations deleted by an earlier upgrade

A successful `0067` applied by a previous binary can already have deleted the original rows. Check `SELECT version, success FROM _sqlx_migrations WHERE version = 67` on the affected database. An empty quarantine in an already-upgraded database cannot establish whether any rows were lost.

1. Restore a verified backup or recover to the point immediately before `0067` using PostgreSQL point-in-time recovery. Use a separate database. Confirm the restored copy has no `0067` entry in `_sqlx_migrations` and contains the original `plugin_guild_installations` rows. An older export can supply evidence, but an installation missing today could also have been intentionally removed between the export and migration.
2. Apply the fixed startup migration to the isolated copy. Read the affected guild's rows from `plugin_guild_installation_quarantine`. Compare their UUIDs and `(guild_id, plugin_id)` pairs with the current installations and any existing quarantine entries. Preserve `id`, `guild_id`, `plugin_id`, `version`, `enabled`, `config` and `installed_at` in the recovery export. Review any conflicts or newer installations with the guild administrator before using that export.
3. Recover the historical settings for the affected guild from the reviewed export. Leave current installations in place. An administrator can explicitly install a known release through the normal Marketplace flow after reviewing its version and configuration. Exact recovery requires original records. The current release's hash or manifest cannot stand in for the missing historical artifact.

## Dashboard frontend

The foundation frontend uses plain HTML, CSS and JavaScript so the Rust binary has no Node build requirement.

Rules:

- use `textContent`, not `innerHTML`, for untrusted values
- require CSRF on writes
- authorize every guild path on the server
- do not trust a guild ID supplied by the browser
- do not store OAuth access tokens in the browser or application session
- keep controls usable on desktop and mobile
- use accessible labels and focus states

A future framework migration requires a measurable benefit and must preserve the server-side authorization model.

## Voice development

The Songbird input resolver expects `yt-dlp` at runtime for supported web sources.

Do not add source-specific bypasses. Source adapters must:

- comply with the service's terms
- validate schemes and hosts
- use timeouts and size limits
- avoid internal network access
- expose operational metrics
- fail without blocking a shard

## Branch and pull-request conventions

Recommended branch names:

```text
feat/moderation-cases
feat/workflow-engine
fix/oauth-session-expiry
docs/lua-actions
```

Recommended commit messages:

```text
feat(lua): add scheduled-message action
fix(auth): consume OAuth state atomically
test(moderation): cover equal-role hierarchy
docs(security): document media worker isolation
```

A pull request should explain:

- user-facing behavior
- architecture impact
- security impact
- database changes
- test evidence
- deployment or rollback requirements
