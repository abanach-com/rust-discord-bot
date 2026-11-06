# Product feature matrix

This document turns the goal “an all-in-one bot with everything” into auditable delivery tracks.

Status:

- **Implemented** — present in the foundation and connected end to end
- **Foundation** — core interface or schema exists, but product workflows are not complete
- **Planned** — accepted scope, not yet implemented
- **Research** — requires policy, cost, quality or scale validation before commitment

The bot should not advertise a feature as complete until its Discord flow, dashboard configuration, persistence, permissions, audit behavior and tests are all present.

## 1. Platform and extensibility

| Capability | Status |
| --- | --- |
| Rust Discord Gateway/HTTP host | Implemented |
| Lua 5.4 module loading | Implemented |
| Command manifests defined in Lua | Implemented |
| Event subscriptions defined in Lua | Implemented |
| Per-execution memory limit | Implemented |
| Per-execution instruction limit | Implemented |
| Allowlisted declarative actions | Implemented |
| Atomic Lua registry reload primitive | Implemented |
| Per-guild module enable/disable | Implemented |
| Per-guild JSON configuration | Implemented |
| PostgreSQL persistence | Implemented |
| In-memory developer fallback | Implemented |
| Discord OAuth2 dashboard login | Implemented |
| CSRF-protected dashboard writes | Implemented |
| Audit events | Implemented |
| JSON Schema form renderer | Planned |
| JSON Schema server-side validation | Planned |
| Live Lua reload from dashboard | Planned |
| Module version pinning per guild | Planned |
| Module dependency graph | Planned |
| Signed first-party modules | Planned |
| Third-party module review pipeline | Planned |
| Module marketplace | Planned |
| Staged rollout and rollback | Planned |
| Feature flags | Planned |
| Localization of commands and dashboard | Planned |
| Multi-process shard supervisor | Planned |
| Distributed session store | Planned |
| Distributed rate-limit coordination | Planned |
| Scheduler workers | Foundation |
| Module key/value persistence API | Foundation |
| Webhook/API trigger system | Planned |
| Public developer SDK and type stubs | Planned |
| Lua language server annotations | Planned |

## 2. Administration

| Capability | Status |
| --- | --- |
| Slash command registration | Implemented |
| Development-guild command mode | Implemented |
| Global command mode | Implemented |
| Role add/remove action | Implemented |
| Actor permission validation | Implemented |
| Bot permission validation for interactions | Implemented |
| Bot and moderator hierarchy validation | Implemented |
| Module configuration audit | Implemented |
| Server setup wizard | Implemented |
| Permission diagnostic wizard | Implemented |
| Command enable/disable by channel | Implemented |
| Command enable/disable by role | Implemented |
| Command cooldowns | Implemented |
| Per-user and per-role rate limits | Implemented |
| Unified Command Policy Engine (ACL per user/role/channel + canrun/explain) | Implemented |
| Staff Recruitment Pipeline (custom forms, stages, multi-person reviews, role fulfillment outbox) | Implemented |
| Channel templates | Planned |
| Server backup/export | Implemented |
| Server configuration import | Implemented |
| Clone roles/channels/categories | Planned |
| Temporary roles | Implemented |
| Role persistence after rejoin | Implemented |
| Self-role buttons and select menus | Planned |
| Reaction roles | Planned |
| Scheduled role assignment | Planned |
| Role synchronization across guilds | Planned |
| Nickname policies | Planned |
| Voice channel rename/limit controls | Planned |
| Temporary voice channels | Implemented |
| Voice companion text channels | Implemented |
| Server counters in channel names | Planned |

## 3. Moderation and safety

| Capability | Status |
| --- | --- |
| Timeout action | Implemented |
| Kick action | Implemented |
| Ban action | Implemented |
| Message delete action | Implemented |
| Purge up to 100 messages | Implemented |
| Advanced Cleanup: filtered purge, preview/confirm, 14-day bulk window partition | Implemented |
| Privileged action audit | Implemented |
| Example blocked-word AutoMod module | Implemented |
| Configurable default timeout | Implemented |
| Warnings and moderator notes | Implemented |
| Numbered moderation cases | Implemented |
| Case edit and reason update | Implemented |
| Temporary bans | Planned |
| Soft bans | Implemented |
| Unban workflow | Implemented |
| Mass-ban raid response | Planned |
| Quarantine role | Planned |
| Channel lockdown | Planned |
| Category lockdown | Planned |
| Slowmode controls | Planned |
| Anti-spam | Planned |
| Duplicate-message detection | Planned |
| Mention-spam protection | Planned |
| Invite-link filter | Planned |
| Domain allow/block lists | Planned |
| Attachment and file-type rules | Planned |
| Unicode/Zalgo normalization | Planned |
| Caps and emoji flood rules | Planned |
| New-account rules | Planned |
| Join-velocity raid detection | Planned |
| Bot-add protection | Planned |
| Suspicious role-change alerts | Planned |
| Native Discord AutoMod rule synchronization | Planned |
| Verification gate | Planned |
| CAPTCHA provider integration | Research |
| Moderator dashboard case queue | Implemented |
| Evidence snapshots | Planned |
| Modmail | Planned |
| Appeals portal | Implemented |
| Moderator duty/availability status | Implemented |
| Escalation policies | Planned |
| Cross-guild shared ban lists | Research |
| Privacy-aware toxicity classifier | Research |

## 4. Logs and observability

| Capability | Status |
| --- | --- |
| Structured process logs | Implemented |
| Database audit table | Implemented |
| Dashboard change audit | Implemented |
| Moderation action audit | Implemented |
| Message edit/delete logs | Planned |
| Member join/leave logs | Planned |
| Voice join/leave/move logs | Planned |
| Role and permission logs | Planned |
| Channel and guild setting logs | Planned |
| Invite usage tracking | Implemented |
| Bot command usage logs | Planned |
| Searchable audit dashboard | Implemented |
| Audit retention policies | Planned |
| Privacy redaction policies | Implemented |
| Prometheus metrics | Implemented |
| OpenTelemetry traces | Planned |
| Error tracking integration | Planned |
| Health and readiness endpoints | Implemented |
| Per-shard status dashboard | Planned |

## 5. Welcome, onboarding and automation

| Capability | Status |
| --- | --- |
| `guild_member_add` Lua event | Implemented |
| Configurable welcome message | Implemented |
| Welcome channel selection | Implemented |
| Event-driven message action | Implemented |
| Goodbye messages | Planned |
| Direct-message onboarding | Planned |
| Autoroles | Planned |
| Rules acceptance | Planned |
| Multi-step onboarding forms | Planned |
| Onboarding role paths | Planned |
| Account-age gates | Planned |
| Invite attribution | Planned |
| Custom command builder | Implemented |
| Keyword and regex triggers | Implemented |
| Button and select-menu workflows | Planned |
| Modal/form workflows | Planned |
| Scheduled messages | Foundation |
| Recurring schedules | Planned |
| Reminders | Planned |
| Sticky messages | Implemented |
| Auto-replies | Planned |
| Cross-channel relays | Planned |
| RSS/Atom feeds | Implemented |
| YouTube notifications | Implemented |
| Twitch notifications | Implemented |
| GitHub release/issue notifications | Implemented |
| Reddit notifications | Planned |
| Calendar event reminders | Planned |
| Webhook ingestion | Planned |
| Outgoing webhooks | Planned |
| Conditional workflow branches | Planned |
| Workflow variables and state | Foundation |
| Retry/dead-letter handling | Planned |

## 6. Tickets, support and forms

| Capability | Status |
| --- | --- |
| Ticket panel | Planned |
| Private ticket channels | Planned |
| Ticket categories and routing | Planned |
| Claim/unclaim | Planned |
| Priorities and service levels | Planned |
| Ticket forms | Planned |
| Application/recruitment forms and review pipeline | Implemented |
| Help Forum lifecycle, solved state, and inactivity auto-archive | Implemented |

### 6.1 Help Forum Lifecycle & Hardening (Issues #164, #190, #191, #192, #193, #194, #195)
- **Background Inactivity Reconciler**: Dedicated daemon loop `run_help_forum_reconciler_loop` with configurable interval (`reconciliation_interval_secs`, default 60s), gracefully integrated with Tokio cancellation token (#190).
- **Atomic Concurrency & Discord Error Safety**: `close_help_forum_session_with_discord` verifies generation tokens, prevents DB state corruption on Discord 403/500 errors, and provides automatic repair (unarchive/unlock) if optimistic concurrency fails (#191).
- **Unified Closure Workflow**: Both web dashboard `POST .../close` and slash command `/help solved` share a single robust closure pipeline with audit trails (#192).
- **Tag Enforcement (`require_question_tag`)**: Threads created without tags transition to `needs_tag` status with automated guidance; promoted to `open` upon tag addition via `thread_update` (#193).
- **Mention Security**: Automated welcome messages and tag warnings explicitly suppress `@everyone`, `@here`, and role mentions using `CreateAllowedMentions::new()` to prevent notification spam (#194).
- **Tenant & Channel Type Validation**: Web configuration validates Discord `ChannelType::Forum` and guild ownership to prevent cross-guild channel hijacking (#195).
| Internal notes | Planned |
| User transcript export | Planned |
| HTML/PDF transcripts | Implemented |
| Close reasons | Planned |
| Reopen workflow | Planned |
| Satisfaction surveys | Planned |
| Staff analytics | Planned |
| Modmail-to-ticket bridge | Planned |
| Email integration | Research |
| Knowledge-base suggestions | Research |

## 7. Community engagement

| Capability | Status |
| --- | --- |
| Text memes | Implemented |
| Dice/number roll | Implemented |
| Random choice | Implemented |
| Eight-ball answers | Implemented |
| Polls | Implemented |
| Anonymous polls | Implemented |
| Giveaways | Implemented |
| Reroll and fraud controls | Implemented |
| Suggestions with voting | Planned |
| Starboard | Implemented |
| Counting channels | Implemented |
| Word-chain games | Planned |
| Trivia | Planned |
| Quote database | Planned |
| Birthdays | Planned |
| Server anniversaries | Planned |
| Event sign-ups | Implemented |
| Attendance tracking | Planned |
| LFG/party finder | Planned |
| Reputation/thanks | Planned |
| Confessions with moderation queue | Planned |
| Question of the day | Planned |
| Custom collectible cards | Implemented |
| Achievements | Planned |

## 8. Levels and economy

| Capability | Status |
| --- | --- |
| Text XP | Planned |
| Voice XP | Planned |
| Anti-farming controls | Planned |
| Configurable XP curves | Planned |
| Role rewards | Planned |
| Rank cards | Planned |
| Leaderboards | Planned |
| Seasonal resets | Implemented |
| Import from other bots | Foundation (preview-only) |
| Virtual currency | Planned |
| Daily/weekly rewards | Planned |
| Shop and inventory | Implemented |
| Tradable items | Implemented |
| Gambling-style commands | Excluded pending legal review |
| Work/minigame income | Planned |
| Economy sinks and taxes | Planned |
| Per-guild currency branding | Planned |
| Fraud and alt detection | Planned |
| Economy audit ledger | Implemented |

## 9. Music and voice

| Capability | Status |
| --- | --- |
| Join invoking user's voice channel | Implemented |
| Search query playback | Implemented |
| Allowlisted HTTPS URL playback | Implemented |
| Queue | Implemented |
| Pause/resume | Implemented |
| Skip | Implemented |
| Stop and clear queue | Implemented |
| Leave | Implemented |
| URL host allowlist | Implemented |
| Per-guild queue state | Implemented through Songbird |
| Now-playing metadata | Planned |
| Queue display and pagination | Implemented |
| Track remove/move/shuffle | Planned |
| Repeat track/queue | Planned |
| Volume control | Planned |
| DJ role and vote-skip | Implemented |
| Saved playlists | Planned |
| User favorites and history | Planned |
| 24/7 mode | Planned |
| Radio streams | Implemented |
| Audio filters/equalizer | Planned |
| Sponsor-block style metadata integration | Research |
| Multiple source adapters | Planned |
| Dedicated voice nodes | Planned |
| Voice receive/recording | Excluded until consent and privacy design |
| Soundboard management | Planned |
| Text-to-speech | Research |
| Stage channel controls | Planned |
| Karaoke/lyrics display | Research; licensing required |

Music integrations must comply with source-platform terms, copyright rules and Discord policy.

## 10. Utility

| Capability | Status |
| --- | --- |
| Ping/runtime diagnostic | Implemented |
| Bot information | Implemented |
| User information | Planned |
| Avatar/banner lookup | Planned |
| Server information | Planned |
| Role/channel information | Planned |
| Snowflake/timestamp tools | Planned |
| Reminders | Planned |
| Time zones | Planned |
| Weather | Planned |
| Translation | Implemented |
| Unit and currency conversion | Planned |
| Calculator | Planned |
| URL preview controls | Planned |
| QR codes | Planned |
| Short links | Excluded unless abuse controls exist |
| File conversion | Research |
| Image manipulation | Planned with resource quotas |
| Meme image generator | Planned |
| Screenshot service | Research |
| Status-page checks | Planned |
| Minecraft/game server status | Implemented |
| Steam/game profile lookup | Implemented |
| Path of Exile integrations | Implemented as optional modules |
| Hero Siege integrations | Implemented as optional modules |

## 11. Analytics and dashboard

| Capability | Status |
| --- | --- |
| Discord OAuth2 login | Implemented |
| Manageable-guild filtering | Implemented |
| Module list | Implemented |
| Enable/disable modules | Implemented |
| JSON configuration editor | Implemented |
| Responsive first-party UI | Implemented |
| Module configuration timestamps | Implemented |
| Emoji & Sticker usage analytics | Implemented |
| Command analytics | Planned |
| Member growth | Planned |
| Retention/cohort analytics | Implemented |
| Message and voice activity | Implemented |
| Channel heat maps | Planned |
| Moderation trends | Planned |
| Ticket analytics | Planned |
| Level/economy analytics | Planned |
| Music analytics | Planned |
| Custom dashboards | Planned |
| CSV/JSON export | Planned |
| Scheduled reports | Planned |
| Data retention controls | Planned |
| Consent and privacy center | Planned |
| Multi-user dashboard roles | Planned |
| Organization/multi-guild view | Planned |
| Mobile-optimized administration | Foundation |

### 11.1 Emoji & Sticker Analytics (Issue #162)
- **Batch Processing**: Usage counts and inventory metadata are buffered in memory via concurrent `DashMap` buckets and flushed atomically to PostgreSQL or volatile storage.
- **Privacy by Design**: Telemetry strictly stores only guild ID, date, asset kind, asset ID, usage count, and timestamp. **NEVER** stores `user_id` or `message.content`.
- **Deduplication Policy**: Approximate analytics using a bounded FIFO/LRU queue of recent `message_id` entries to prevent double-counting during gateway reconnects.
- **Failure Recovery**: If a storage error occurs during flush, batch snapshots are safely merged back into the in-memory buffer without data loss.
- **REST Endpoints & Slash Commands**:
  - `GET /api/guilds/{guild_id}/assets/usage?kind=emoji&period=30d`
  - `GET /api/guilds/{guild_id}/assets/inventory`
  - `POST /api/guilds/{guild_id}/assets/flush`
  - `/emoji stats [period]`, `/emoji unused [days]`, `/sticker stats [period]`

### 11.2 Emoji & Sticker Analytics Hardening & Lifecycle (Issues #196, #197, #198, #199, #200, #201)
- **Exact Calendar Windows**: Semantics of `7d` and `30d` windows fixed to exactly 7 and 30 calendar days inclusive (today + 6 / 29 prior days) (#196).
- **Periodic Flush Loop & Graceful Shutdown**: `run_asset_analytics_flush_loop` worker runs every 30s with single-flight mutex serialization and bounded 5s flush on shutdown (#197).
- **Accurate Inactive Asset Detection & Inventory Sync**: `/emoji unused` inspects full inventory with guild ownership filtering, properly identifying never-used and long-unused emojis. Automatic cache synchronization on `guild_create`, `guild_emojis_update`, and `guild_stickers_update` (#198).
- **Strict Ownership Classification**: External stickers and emojis are verified against cached guild asset sets, classifying missing cache as unknown (`None`) rather than falsely owned (`true`) (#199).
- **SQL Aggregation & Paged Inventory**: Server-side aggregation via `GROUP BY` and `FILTER` prevents unbounded daily row transfer. Added composite index `(guild_id, asset_kind, usage_date)` and pagination (`limit`, `offset`, `cursor`) on `/assets/inventory` (#200).
- **Authoritative Inventory Reconciliation & Stale Flush Immunity**: `reconcile_guild_inventory` atomically upserts active assets and deactivates removed ones on `guild_create` (cold-start) and update events. `record_asset_usage_batch` never overwrites `is_active` on conflict, preventing delayed flushes from reactivating deleted emojis or stickers (#201).

## 12. AI-assisted features

AI features are optional modules, not a prerequisite for core moderation.

| Capability | Status |
| --- | --- |
| FAQ assistant grounded in approved documents | Research |
| Support-ticket summarization | Research |
| Moderation case summarization | Research |
| Suggested moderator responses | Research |
| Semantic search across approved knowledge | Research |
| Conversation summaries with consent | Research |
| Translation assistance | Research |
| Spam/scam classifier | Research |
| Image safety classifier | Research |
| Natural-language automation builder | Research |
| Per-guild model/provider choice | Planned if AI track proceeds |
| Cost quotas and hard limits | Required before release |
| Privacy redaction and retention | Required before release |
| Human review for enforcement | Required before release |

No AI model may autonomously ban users without an explicit, reviewable guild policy and an appeal path.

## 13. Integrations, External Ecosystem & Extensibility (Phase 4)

| Capability | Status | Notes |
| --- | --- | --- |
| Provider Adapter & Contract Hierarchy | Implemented | Strict traits `ProviderAdapter`, `ProviderCapability`, `ProviderEvent`, `ProviderAction` (#92) |
| SSRF Sentinel & IP Restrictor | Implemented | RFC 1918, RFC 3927, loopback, and cloud metadata filter with allowlists (#92) |
| OAuth2 Authorization Code + PKCE | Implemented | S256 verifier/challenge pair, state/nonce TTL, single-flight refresh (#92) |
| Webhook Gateway & Broker | Implemented | Raw body HMAC-SHA256, persistent deduplication, exponential backoff, circuit breaker (#92) |
| Unified Identity Service | Implemented | Multi-account links, anti-hijack checks, self-service unlink, role sync (#93) |
| Steam Extended API & LFG | Implemented | OpenID assertion validation, privacy-respecting library/profile, cross-guild LFG (#94) |
| GitHub DevOps Suite | Implemented | Push/PR/Check-run webhooks, Forum thread mapper, RBAC write controls (#95) |
| Twitch Extended EventSub | Implemented | EventSub transport, stream online/offline, live embeds, live roles (#96) |
| Kick Integration | Implemented | Kick Dev API client, livestream alerts, dynamic embed updates (#97) |
| YouTube Extended WebSub | Implemented | WebSub push subscriptions, upload/shorts heuristics, quota monitor (#98) |
| Telegram Bidirectional Bridge | Implemented | Webhook secret token check, bidirectional relay, anti-loop echo protection (#99) |
| Workflow Builder Visual DAG | Implemented | Directed acyclic graph engine, Kahn's cycle detection, dry-run simulation (#100) |
| GitLab Cloud & Self-Hosted | Implemented | Token verification, pipeline failure alerts, self-hosted SSRF allowlist (#101) |
| VKontakte Community Gateway | Implemented | Callback API handshake challenge, post notifications, failure isolation (#102) |
| Reddit Official API Poller | Implemented | Cursor pagination, flair/score filtering, moderator dry-run approvals (#103) |
| Gaming Hub Aggregator | Implemented | Multi-game status registry, strict PoE1 vs PoE2 namespace isolation (#104) |
| Social Publisher Multi-Platform | Implemented | Cross-platform composer, scheduled release, two-person approval gates (#105) |
| AI Gateway & Hard Budget | Implemented | Hard token/USD caps per guild with instant cutoff, prompt injection sanitization (#106) |
| MCP Protocol Client/Server | Implemented | Model Context Protocol tool allowlists, mutating action approval gates (#106) |
| Plugin Marketplace & Verification | Implemented | Semver manifests, SHA-256 cryptographic check, transactional install/rollback (#107) |

## 14. Platform & API Hardening (Issues #202, #203, #204, #205, #206)

- **Authoritative Broadcast Delivery (#202)**: Validates target channel snowflakes, verifies guild ownership, enforces `CreateAllowedMentions` to prevent unauthorized @everyone/role pings, and fails closed in offline mode rather than reporting fake delivery success.
- **Custom Automation Simulation & Tenant Isolation (#203)**: `/automations/{id}/test` enforces guild-scoped lookup (returning 404 for unknown or cross-guild rules), checks active status (400 Bad Request if disabled), enforces cyclic recursion limits (depth > 5), and evaluates planned actions without side effects or unverified executions.
- **Grounded AI Knowledge & Server Budget Enforcement (#204)**: `/ai/query` queries approved guild knowledge documents via `search_ai_documents` with tenant isolation and channel ACLs. Prohibits hallucinated regulations and unverified citations. Authoritatively enforces server-side token budget limits against client tampering, returning `status: "no_sources"` and `escalated_to_staff: false` when unindexed.
- **Unified Offline Mode & Live Member Verification (#205)**: Centralized `Config::offline_mode` and `is_offline_mode()` across Gateway and web dashboard. Prevents `OFFLINE_MODE=false` or empty values from bypassing live Discord membership verification in store and web routes, failing closed with 403 on Discord 404/403.
- **Steam LFG Self-Service Access (#206)**: Relaxed permission checks on `/steam/lfg` endpoints from `MANAGE_GUILD` to `ensure_guild_member_access`, allowing regular community members to view, create, and join teams while maintaining CSRF validation, tenant isolation, and author attribution.

## 15. Storage Parity & Lua Installer Sandboxing (Issues #207, #208, #209, #221, #224)

- **MemoryStore & PostgreSQL UPSERT Semantics Parity (#207)**:
  - Keyed `github_devops_mappings` by business identity tuple `(guild_id, repo_full_name.to_ascii_lowercase(), issue_or_pr_number)` instead of raw UUID in `MemoryStore`, matching PostgreSQL's `UNIQUE` constraint and `ON CONFLICT DO UPDATE`.
  - Applied equivalent business tuple keying to `twitch_subs` (`(guild_id, streamer_login.to_ascii_lowercase())`), `kick_subs` (`(guild_id, slug.to_ascii_lowercase())`), and `youtube_subs` (`(guild_id, channel_id)`).
  - Preserved original record `id` and `created_at` timestamps on conflict updates while updating mutable target fields (`thread_channel_id`, `discord_thread_id`, `last_status`, `live_role_id`, `last_message_id`, `is_live`, etc.), ensuring identical data retention contracts across both storage engines.
- **Lua Installer Manifest Identity Parity & Duplicate Rejection (#208)**:
  - `InstallManifest::parse` strictly enforces unique `<module id>` entries in `install.xml` with fail-closed rejection upon duplicates.
  - `LuaEngine::load` and `build_registry` strictly cross-validate installer entry IDs against Lua manifest identities (`installer_entry.id == manifest.id`). Prevents unauthorized or spoofed packages from declaring privileged capability IDs (`tickets`, `scheduler`, `progression`) under arbitrary package labels.
- **Lua Package Confinement & Path Traversal Prevention (#209)**:
  - `InstallManifest::parse` validates that `folder`, `entry`, and `config` relative paths contain only normal or current-dir components (`Component::Normal(_) | Component::CurDir`), explicitly forbidding parent directory traversal (`..`) and absolute roots.
  - `ensure_confined_path` in `LuaEngine` canonicalizes paths on disk and enforces that resolved files and directories remain strictly within their parent package boundary (`starts_with(&canonical_base)`).
  - Symlinks attempting to escape package directories to read arbitrary system or secret JSON configuration files are rejected at load time, safeguarding runtime configuration and preventing sandbox escapes.
- **Lua Installer XML Single Root & Trailing Fragment Hardening (#221)**:
  - `parse_xml_to_dom` strictly enforces standard XML root-containment invariants: rejects multiple root elements (`stack.is_empty() && root.is_some()`), unclosed elements on EOF (`!stack.is_empty()`), and non-whitespace text/CDATA content outside the root document element.
  - Guarantees fail-closed parsing of malformed manifests such as `<installer/><other/>`, `<installer/><broken>`, `<installer/>unexpected-text`, and unclosed root tags (`<installer>`), returning controlled `ParseError`.
  - Preserves legitimate XML structures including self-closing roots (`<installer/>`), single rooted hierarchies, declarations (`<?xml ...?>`), and comments outside or within root elements without regressions.
- **Atomic Webhook Event Deduplication Parity (#224)**:
  - Replaced non-atomic `contains_key(&key)` + `insert(key, Utc::now())` in `MemoryStore::check_and_record_event_dedupe` with a single atomic `DashMap::entry(key)`, matching PostgreSQL's `INSERT ... ON CONFLICT DO NOTHING` atomicity.
  - Guarantees strict single-winner semantics under concurrent replays (16–64 parallel requests for the same event ID produce exactly one `true`/`processed` and remainder `false`/`deduplicated`).
  - Standardized provider case-insensitivity (`provider.to_ascii_lowercase()`) across both MemoryStore and PostgreSQL backends.

## 16. Duty Policy & Companion Mapping Batch Atomicity (Issues #210, #211, #215)

- **Duty Override Time Limit & Overflow Protection (#210)**:
  - `POST /duty/override` authoritatively fetches guild duty configuration (`get_duty_settings`) and validates `duration_mins` against `1..=max_override_duration_mins` (defaulting to 1440m / 24h if unset). Rejects requests with non-positive duration or durations exceeding the configured guild policy with HTTP 400 Bad Request.
  - Replaced unsafe duration constructors with `chrono::Duration::try_minutes(minutes)` and `checked_add_signed(delta)` across web handlers and Discord command dispatchers, preventing service crashes / panics on extreme numeric values (`i64::MAX`).
- **Voice Companion Batch Quota & Transactional Atomicity (#211, #215)**:
  - Enforced global limit of 50 companion mappings per guild (`MAX_COMPANION_MAPPINGS_PER_GUILD = 50`) at both the REST API preflight layer and persistent storage layer.
  - Hardened batch request payload validation against duplicate entries and Discord API request storms (#215): strictly validates `items.len() <= MAX_COMPANION_MAPPINGS_PER_GUILD` upfront, rejects duplicate `voice_channel_id` entries with HTTP 400 Bad Request prior to any external network calls or database writes, and caches resolved channels across the validation pipeline.
  - Implemented transactional batch persistence (`batch_upsert_companion_mappings`) for PostgreSQL using `pg_advisory_xact_lock` and single-transaction rollbacks on constraint violations, and thread-safe atomic batch persistence for `MemoryStore` under synchronization locks.
  - Segmented `POST /companion-channels/batch` into a three-phase pipeline: (1) upfront preflight validation of all mappings, channel snowflakes, voice!=text invariants, tenant isolation, and live Discord channel types; (2) atomic storage batch upsert guaranteeing all-or-nothing execution without partial persistence; (3) post-commit Discord permission revocation and audit logging.

## 17. Lua Legacy Loader Hardening, Community RBAC & SSE Lifecycle (Issues #212, #213, #214, #216, #217, #220, #222, #223, #225, #226, #227, #228, #229)

- **Lua Legacy Discovery Hardening & Symlink Containment (#212)**:
  - In legacy discovery mode (when `install.xml` is absent), symlinks are strictly forbidden via `DirEntry::file_type().is_symlink()`, preventing unintended loading of arbitrary external code or configuration leakage.
  - Hardened recursive discovery with a maximum depth limit of 16 (`MAX_DISCOVERY_DEPTH = 16`), maximum script count limit of 512 (`MAX_DISCOVERY_FILES = 512`), canonical root containment checks (`canonical_current.starts_with(canonical_root)`), and cycle detection via visited canonical directories.
  - Symlinked or directory loop references (`loop -> .`) fail closed with controlled load errors without recursion hangs, and preserve the active Lua registry upon reload.
- **XML Tag Lexer & Parser Resilience (#213, #221)**:
  - Robust XML attribute lexer in `src/lua/install_xml.rs` preventing infinite loops on corrupted XML attributes containing unescaped slashes, quotes, or trailing fragments.
  - Strict XML root verification (#221): installer manifest parser strictly rejects documents with multiple root elements, non-whitespace/content outside root tag boundaries, and unclosed tags on EOF, preventing bypass of permission manifests and script declarations.
- **Giveaway Community Member Access, Deadline Enforcement & Admin Boundaries (#213, #217)**:
  - `GET /api/guilds/{guild_id}/giveaways/{giveaway_id}` and `POST .../join` employ `ensure_guild_member_access`, allowing regular community members without `MANAGE_GUILD` privileges to view active giveaways and enter drawings with CSRF validation.
  - Enforced strict deadline validation (#217): both REST handler and storage persistence layers (`MemoryStore` and PostgreSQL `SELECT ... FOR UPDATE`) reject joins when `Utc::now() >= ga.expires_at`, preventing participant additions to expired giveaways prior to rerolls.
  - Administrative operations (`POST /giveaways` and `POST .../reroll`) remain strictly protected by `ensure_guild_access`, requiring server management permissions.
- **Scheduled Events Community Member Access & Self-Service RSVP Boundary (#220, #222, #223)**:
  - `GET /api/guilds/{guild_id}/scheduled-events`, `POST .../scheduled-events/{event_id}/roster/join` and `POST .../roster/leave` relaxed permission checks from `ensure_guild_access` (`MANAGE_GUILD`) to `ensure_guild_member_access`, allowing regular community members to view server events and manage their own RSVP roster registrations.
  - Administrative creation (`create_event`) strictly requires `ensure_guild_access` (`MANAGE_GUILD`), preventing unauthorized event creation.
  - Preserved mandatory CSRF token validation on mutating actions and author attribution binding exclusively to authenticated `session.user.id`.
  - Enforced cross-guild tenant isolation and live membership fail-closed behavior verified against live-mode Discord API mock (200 OK -> allowed; 404 Unknown Member & 503 Upstream Error -> 403 Forbidden).
  - Pre-cache session invariant hardening (#222): `ensure_guild_member_access` validates `session.guilds` presence upfront before reading `permissions_cache`, preventing positive manager cache entries from prior sessions from bypassing membership revocation when a user presents a session lacking the guild.
  - RSVP point lookup & zero-scan performance (#223): replaced full guild table scan and chronological sorting (`list_scheduled_events`) in `join_event_roster` and `leave_event_roster` with direct primary-key point lookup by UUID (`get_scheduled_event(event_id)`) and tenant boundary filtering (`ev.guild_id == guild_id`), eliminating $O(N \log N)$ sorting and large multi-column allocations.
- **Staff Recruitment Re-Application Cooldown Precision (#225)**:
  - In `apply_recruitment_track`, cooldown evaluation calculates `latest_rejection_at` as `rejected_apps.iter().filter_map(|app| app.decided_at).max()` instead of blindly inspecting `rejected_apps.first()`.
  - Guarantees that cooldown enforcement is authoritatively anchored to the most recent decision date (`MAX(decided_at)`), independent of the application submission order (`submitted_at DESC`) or interleaved review outcomes across concurrent application tracks.
- **Asset Analytics Manual Flush CSRF Guard & Per-Guild Tenant Isolation (#226)**:
  - `POST /api/guilds/{guild_id}/assets/flush` strictly requires valid CSRF verification via `verify_csrf(&headers, &session)` prior to executing flush mutations, rejecting unauthenticated or cross-origin requests with 403 Forbidden.
  - Introduced `AssetAnalyticsBatcher::flush_guild(guild_id)` using scoped key filtration `flush_inner(Some(guild_id))`. Manual dashboard flushes drain and persist solely the requesting guild's buffered counts and inventory records, preventing cross-tenant side effects, locking interference, and information leakage from other guilds. Global `flush()` (`flush_inner(None)`) remains exclusively reserved for internal background worker loops and graceful process shutdown.
- **Help Forum Session Owner Close Guild Member Lifecycle Verification (#227)**:
  - `POST /api/guilds/{guild_id}/help-forums/{thread_id}/close` verifies active guild membership via `ensure_guild_member_access(&state, &session, &guild_id)` for non-administrative post owners before checking `session.user.id == help_session.owner_user_id` and `config.owner_can_close`.
  - Guarantees that former members who departed or were removed from the server cannot remotely archive, lock, or transition historical help forum posts via the dashboard. Administrative access (`ensure_guild_access`) continues to govern moderator closures.
- **Moderation Appeal Cross-Guild Tenant Isolation (#228)**:
  - `POST /api/guilds/{guild_id}/appeals/{appeal_id}/decide` and storage layer (`update_appeal_status`) enforce strict tenant containment (`guild_id` passed to `MemoryStore` check `entry.value().guild_id == guild_id` and PostgreSQL query `WHERE id = $1 AND guild_id = $2`).
  - Cross-guild modification attempts fail closed with HTTP 404 Not Found without modifying status or staff notes of appeals belonging to other guilds.
- **Computed Discord Permissions & Truthful System Diagnostics (#229)**:
  - `GET /api/guilds/{guild_id}/diagnostics` replaces dummy 100% / healthy responses with real validation against official Discord computed member permissions API (`GET /guilds/{guild_id}/permissions/member/{bot_id}`).
  - Evaluates module-specific permission requirements (e.g. `MANAGE_ROLES`, `MANAGE_CHANNELS`, `BAN_MEMBERS`, `VIEW_AUDIT_LOG`), reports granular check status (`ok`, `missing`, `unknown`), remediation instructions, and cross-guild channel containment.
  - Fail-safe reporting: Discord API errors (403, 429, timeout) report `unknown` and `overall_health_percent: "incomplete"` / `health_status: "incomplete"` instead of falsely declaring 100% health. In-memory 30s caching prevents API rate limiting.
- **SSE Guild Realtime Events Stream Re-Authentication & Disconnect Cleanup (#214, #216)**:
  - `GET /api/guilds/{guild_id}/events` utilizes an asynchronous dispatch pipeline with dual periodic re-authentication (15-second interval) and pre-event verification against storage and `permissions_cache`.
  - Stale, expired, or deleted user sessions, as well as revoked guild management permissions (`can_manage: false`), immediately terminate active Server-Sent Events (SSE) connections, preventing administrative and moderation event leakage to unauthorized parties. Handled broadcast channel lagging with explicit client resync events.
  - Mitigated orphaned background task leaks upon idle SSE disconnects (#216): worker task listens for `tx.closed()` in `tokio::select!`, immediately terminating background loops, timers, and broadcast receiver subscriptions when a client drops the connection without requiring new events.

## 18. Moderation Delete Coordinator & Queue Architecture (Issues #218, #219)

- **Per-Channel Shared Moderation Delete Queue & Backpressure (#218)**:
  - Introduced host-managed `DeleteCoordinator` in `src/discord/delete_queue.rs` coordinating moderation deletions across `LuaAction::DeleteMessage`, `LuaAction::Purge`, and cleanup routines.
  - Implemented per-channel asynchronous workers with configurable micro-batching windows (100ms) and automatic deduplication of concurrent/in-flight message IDs across multiple AutoMod rules or moderation commands.
  - Enforced bounded backpressure (`channel_queue_capacity`, `try_send`) and per-guild channel concurrency limits (`max_channels_per_guild`), preventing queue bloat and memory leaks during high-volume bursts.
  - Automatic partition into Discord Bulk Delete batches (2–100 messages < 14 days old) and single deletes for older messages. Explicitly categorizes `Unknown Message` (10008) errors as `already_gone` while propagating permissions (403) and rate limits (429) without masking.
  - Idle worker cleanup automatically deallocates channels after inactivity periods without lingering background tasks.
- **Atomic Registration & Shutdown Drain Protocol (#219)**:
  - Eliminated race conditions between channel worker registration, concurrency check, and idle worker shutdown using per-guild locking (`guild_locks: Arc<DashMap<GuildId, Arc<Mutex<()>>>>`). Guarantees atomic check-then-spawn and strictly enforces `max_channels_per_guild` under concurrent load.
  - Safe channel worker idle shutdown: workers remove their registration conditionally (`remove_if`) only if the entry still points to the same active channel sender (`same_channel`), preventing inadvertent de-registration of newly spawned workers.
  - Full queue drain on idle timeout: before termination, worker closes receiver (`rx.close()`), drains all buffered requests, and processes final moderation delete batches (`process_delete_batch`), guaranteeing zero lost moderation deletes.
  - Enqueue retry loop with channel refresh: if an enqueue operation encounters a closing worker channel (`TrySendError::Closed`), it recovers the oneshot reply channel, re-acquires or spawns an active worker channel under lock, and retries delivery.

## Delivery order

### Phase 0 — foundation

Current pull request:

- Rust/Lua trust boundary
- commands and first events
- dashboard authentication and module settings
- persistent storage
- initial moderation, role, music, fun and welcome modules
- CI, Docker and documentation

### Phase 1 — production moderation

- cases, warnings, temp actions and full logs
- anti-spam, anti-raid and native AutoMod synchronization
- dashboard permission diagnostics
- test coverage for Discord authorization edge cases
- metrics, backups and operational runbooks

### Phase 2 — automation and community

- workflows, scheduled jobs, role menus and tickets
- levels, reputation, suggestions, polls and giveaways
- external notification adapters
- visual configuration forms

### Phase 3 — advanced voice and analytics

- playlist persistence, DJ controls, voice nodes
- complete analytics and report exports
- multi-guild dashboard roles
- scale testing and shard orchestration

### Phase 4 — ecosystem & integrations (Completed)

- signed modules, review tooling and marketplace (#107)
- integration core, OAuth2 + PKCE, webhook gateway broker (#92)
- unified identity and account linking (#93)
- external bridges: Steam (#94), GitHub (#95), Twitch (#96), Kick (#97), YouTube (#98), Telegram (#99), GitLab (#101), VK (#102), Reddit (#103)
- visual DAG workflow builder (#100)
- gaming hub and PoE1/PoE2 separation (#104)
- social publisher with multi-target composer and approvals (#105)
- AI gateway, hard budget caps, channel ACL RAG, and MCP tools (#106)
