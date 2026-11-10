# Security design

## Trust boundaries

The deployment has four distinct trust levels:

1. infrastructure secrets and the Rust process
2. authenticated dashboard administrators
3. reviewed Lua modules
4. Discord users and event payloads

Lua modules are not infrastructure code. Even first-party modules run through the same constrained action boundary.

## Secret handling

Secrets are read from environment variables:

- `DISCORD_TOKEN`
- `DISCORD_CLIENT_SECRET`
- `DATABASE_URL`

They must never be committed, rendered in the dashboard, passed to Lua or included in structured logs. Production deployments should use a secret manager or container secret facility rather than a shared `.env` file.

## Lua isolation

Current controls:

- fresh VM for every invocation
- vendored Lua 5.4
- memory ceiling
- global instruction-count hook covering the main thread and every new Lua thread
- disabled filesystem, operating-system, module-loader and debug libraries
- disabled dynamic loading (`load`, `loadfile`, `dofile`)
- disabled coroutines and direct garbage-collector controls
- disabled `pcall` and `xpcall`, preventing scripts from swallowing the instruction-limit error
- no native network API
- no direct database API
- no Discord client objects
- typed context serialization
- typed action deserialization
- maximum 25 actions per invocation
- validation before action execution
- bounded manifest parser (`install.xml`): 512 KiB source ceiling, max 64 DOM depth, max 4096 elements, max 64 KiB element text

These controls reduce risk from accidental loops, large allocations and direct capability abuse. They do not make arbitrary third-party Lua automatically trustworthy. Module review remains required.

Because error-catching primitives are intentionally unavailable, module code should validate inputs and return safe fallback actions instead of using exception-driven control flow. Recoverable host operations are executed only after the Lua handler returns and are reported by the Rust action executor.

Future hardening candidates:

- process-level sandbox workers
- seccomp/AppArmor profile
- signed module packages
- per-module capability declarations
- module provenance and hash audit
- fuzzing of manifest and action decoding
- bounded VM pools with state reset proofs

## Discord authorization

Privileged command actions perform two checks:

1. invoking member permissions
2. bot/application permissions supplied by the interaction

Moderation actions additionally fetch current guild/member information and verify:

- the target is not the guild owner
- the bot's highest role is above the target
- the invoking moderator's highest role is above the target, unless the moderator is the guild owner
- a moderator cannot target themselves

Role actions verify that the managed role is below the bot and invoking moderator.

Discord remains the final authority and can reject an operation because of changed permissions, hierarchy or rate limits.

## Dashboard security

Implemented:

- Discord authorization-code OAuth2 flow
- one-time random state values
- ten-minute OAuth2 state lifetime
- Discord OAuth2 access token retained in server session memory only for live permission revalidation (never serialized to client)
- opaque HttpOnly session cookie
- SameSite=Lax
- optional Secure flag
- session expiry
- live guild authorization revalidated against Discord with short-lived caching (60s TTL) for sensitive/write routes (fail-closed)
- CSRF token required for writes
- 64 KiB configuration limit
- audit event for configuration changes

### Reverse Proxy and Client IP Resolution (Issue #246)

The dashboard enforces both a global login attempt limiter (60/min) and an individual per-IP limiter (10 attempts/min) to prevent brute-force and credential-stuffing attacks.

To protect against `X-Forwarded-For` spoofing behind trusted reverse proxies:
- **Right-to-Left Recursive Traversal**: `resolve_client_ip` inspects hops from right to left (closest proxy to furthest). While the intermediate hop belongs to `TRUSTED_PROXIES`, traversal continues leftward until the first untrusted IP is encountered (analogous to Nginx `real_ip_recursive on`). Any arbitrary IP injected by the client at the far-left is discarded.
- **Direct Connection Defense**: When the peer socket address does not match `TRUSTED_PROXIES`, all incoming `X-Forwarded-For` and `X-Real-IP` headers are ignored, and the peer socket IP is used directly.
- **Fail-Safe Fallbacks**: Malformed hop IPs or requests without peer address fail safe to `127.0.0.1` and never trust unverified header payloads.

#### Proxy Configuration Guidelines

1. **Nginx (Edge Proxy)**:
   - *Overwrite mode* (recommended when edge terminates public traffic):
     ```nginx
     proxy_set_header X-Forwarded-For $remote_addr;
     proxy_set_header X-Real-IP $remote_addr;
     ```
   - *Append mode* (multi-hop reverse proxy with upstream CDN/LB):
     ```nginx
     proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
     set_real_ip_from 10.0.0.0/8;
     real_ip_header X-Forwarded-For;
     real_ip_recursive on;
     ```
2. **Traefik**:
   - Configure trusted entrypoints to sanitize forwarded headers:
     ```yaml
     entryPoints:
       websecure:
         forwardedHeaders:
           trustedIPs:
             - "127.0.0.1/32"
             - "10.0.0.0/8"
     ```

Production requirements:

- terminate HTTPS before the application
- set `SESSION_COOKIE_SECURE=true`
- restrict dashboard origin and reverse-proxy headers
- configure request-rate limits at the proxy
- use a persistent/distributed session store before horizontal scaling
- rotate Discord credentials after suspected exposure
- retain audit logs according to a documented privacy policy

## Media source security and egress network hardening

Music input invokes external media resolution and streaming via `yt-dlp` / Songbird. Protection against Server-Side Request Forgery (SSRF) combines three defense-in-depth layers:

### 1. Application-level input validation
- User-provided URLs must use HTTPS (`https://`).
- The initial hostname is strictly validated against `MUSIC_ALLOWED_HOSTS` (`youtube.com`, `soundcloud.com`, `bandcamp.com`, etc.). Subdomains must terminate with a registered root domain (`host == candidate || host.ends_with(".{candidate}")`).
- IP literals in URLs are checked against private, loopback, link-local, and reserved ranges (`127.0.0.0/8`, `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `169.254.0.0/16`, `::1`, `fc00::/7`, `fe80::/10`) and rejected immediately.
- Plain text is treated strictly as a search query, never as a raw network endpoint.
- Lua modules cannot modify or bypass the host allowlist.

### 2. HTTP client egress redirect policy
- The media HTTP client enforces a strict redirect policy:
  - Follow-up redirects must remain on HTTPS (`https://`).
  - Redirect targets to private, loopback, carrier-grade NAT (`100.64.0.0/10`), link-local, or cloud metadata endpoints (`169.254.169.254`) are intercepted and rejected.
  - Redirect chains are bounded to at most 10 hops.

### 3. Container and network-level isolation
Production deployments must isolate media workers at the host/container firewall layer:
- Run media containers in an isolated Docker network bridge without host network access.
- Block egress to cloud metadata services (`169.254.169.254/32`).
- Block outbound traffic from media workers to internal VPC ranges (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`) and local daemon ports (PostgreSQL `5432`, Dashboard `8080`).
- Ensure DNS resolvers drop or filter responses resolving to RFC 1918 addresses (DNS rebinding prevention).

## Message safety

Bot-generated messages use an empty Discord allowed-mentions policy. User-provided text should also pass through `zuckerbot.escape_mentions`.

Future rich-message actions must define explicit mention, attachment and embed limits rather than inheriting broad Discord defaults.

## Database

- application queries use SQLx parameter binding
- module configuration is stored as JSONB
- configuration payloads are size-limited
- audit events are append-only through the current application API
- database users should receive only the privileges needed by migrations and runtime

Production should separate migration credentials from runtime credentials.

## Application-Layer Encryption At Rest and Deployment Upgrades

The bot enforces strict application-layer AES-256-GCM encryption at rest with separate cryptographic keyrings across independent domains:
- **Ticket Transcripts**: Controlled by `TRANSCRIPT_ENCRYPTION_KEY` / `TRANSCRIPT_KEYRING` with AAD bound to `guild_id` and `ticket_id`.
- **Integration Credentials**: Controlled by `INTEGRATION_CREDENTIAL_KEY` / `INTEGRATION_CREDENTIAL_KEYRING` with AAD bound to `guild_id`, `entity_type`, `record_id`, and `field_name`.

### Startup Preflight & Upgrade Contract
When `DATABASE_URL` is set, `AppState::bootstrap` executes a strict fail-fast preflight:
1. Validates presence and 64-hex format of `TRANSCRIPT_ENCRYPTION_KEY` (or `TRANSCRIPT_KEYRING`), unless `TICKET_TRANSCRIPTS_DISABLED=true`.
2. Validates presence and 64-hex format of `INTEGRATION_CREDENTIAL_KEY` (or `INTEGRATION_CREDENTIAL_KEYRING`), unless `INTEGRATION_CREDENTIALS_DISABLED=true`.
3. If an encryption secret is missing or malformed, startup immediately aborts prior to opening database connections or executing migrations.

**Upgrade Requirement**: Operators upgrading an existing deployment to version `0.1.0+` (or migrations `>= 0056`) MUST generate a dedicated 32-byte key via `openssl rand -hex 32` and set `INTEGRATION_CREDENTIAL_KEY` in their `.env` file before pulling the new container image or restarting the service. Transparent migration of existing unencrypted rows is performed automatically during backfill. Plaintext storage fallbacks are rejected fail-closed.


## Logging and privacy

Do not log:

- bot or OAuth2 tokens
- database passwords
- complete private message contents
- unnecessary personal data
- raw authorization headers or cookies

New event logs require retention and redaction decisions. Voice recording is explicitly outside the accepted scope until consent, jurisdiction and deletion behavior are designed.

## Member Identity History Retention and Janitor

- **Logical Hiding vs. Physical Deletion**: At query time (`get_member_identity_history`), entries with `expires_at <= NOW()` are logically filtered out and hidden from API responses. Physical storage reclamation is handled by the automated janitor (`Storage::cleanup_expired_identity_history`).
- **Periodic Janitor**: An automated background supervisor task (`run_identity_history_janitor`) runs hourly, batch-deleting expired records in chunks of 1000 in PostgreSQL to avoid table locks and autovacuum spikes, and purging expired records from `MemoryStore`.
- **Isolation from Moderation Cases**: Moderation case records follow disciplinary audit log retention rules and are never pruned by the identity janitor.
- **GDPR / Privacy Erasure**: Explicit erasure requests (`erase_guild_user_data`) immediately wipe all identity history, snapshots, and related data for a user without waiting for TTL expiration.

## Dependency and release security

Before production release:

- pin and commit `Cargo.lock`
- run `cargo audit` or an equivalent advisory scanner
- generate an SBOM
- scan container images
- sign release images
- enable protected branches and required CI
- run dependency updates through pull requests
- maintain a rollback image
