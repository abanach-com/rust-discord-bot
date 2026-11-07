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
| Automated giveaway expiration and winner resolution | Implemented |
| Custom tags persistence and management (`/tag`) | Implemented |
| Community submissions delivery assurance (`/report`, `/suggest`, etc.) | Implemented |
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

### 7.1 Community, Giveaways & Platform Reliability Hardening (Issues #266, #267, #268, #269, #270, #271, #272, #273, #274)
- **AutoMod Embed-Only Update Guard (Issue #266)**: Calculation of `content_changed` and `attachments_changed` flags during `MESSAGE_UPDATE` gateway handling prevents spurious re-moderation and repeat actions on link unfurls and embed-only updates.
- **Custom Tags Persistence & ACL (Issue #267)**: End-to-end `/tag create/get/delete` execution pipeline backed by PostgreSQL `module_kv`, strongly-typed `TagOperation` and `LuaAction::Tag`, `MANAGE_MESSAGES`/`ADMINISTRATOR` privilege validation, sanitized mentions on read, and audit logging.
- **Voice Companion Delayed Reconcile Race Safety (Issue #268)**: Reconciliation on delayed `GUILD_CREATE` checks live guild cache under per-user mutex locks, preventing stale gateway snapshots from revoking or granting permissions contrary to newer `VOICE_STATE_UPDATE` events.
- **Community Submissions Delivery Assurance (Issue #269)**: Replaced premature success replies in `/report`, `/suggest`, `/feedback`, `/apply`, and `/bugreport` with typed `LuaAction::PublishSubmission`, verifying Discord HTTP delivery before confirming to the user and auditing.
- **Automated Giveaway Expiration & Winner Resolution (Issue #270)**: Background worker `run_giveaway_worker` periodically reconciles expired giveaways via transactional `finish_expired_giveaways`, randomly picks winners, updates persistence, announces winners to channel, and audits results.
- **Giveaway Announcement Reliability & Idempotency (Issue #271)**: Ensures announcement publication succeeds prior to persistent record commit in `/giveaway start`, preventing orphan records on HTTP failures, and reports delivery status explicitly in `/end` and `/reroll`.
- **Giveaway Channel View Authorization (Issue #272)**: Replaced naive guild membership-only verification in `get_giveaway` and `join_giveaway` endpoints with `ensure_channel_view_access`, querying Discord channel permission bits (`VIEW_CHANNEL` or `ADMINISTRATOR`) to prevent unauthorized access or drawing entries into private/staff-only giveaway channels.
- **Giveaway Module Configuration Enforcement & Self-Entry Policies (Issue #273)**: Enforced server module configuration across Lua and privileged Rust execution pipelines: respected `max_winners` limits (clamped 1..=20), `default_duration_minutes`, and configured `announcement_channel_id`. Added `created_by_user_id` tracking in PostgreSQL (`migrations/0039_giveaways_created_by.sql`) and MemoryStore, enforcing atomic `allow_self_entry` guards with backward-compatible grandfathering for legacy giveaways.
- **Giveaway Privileged Service Boundary & Capability Guards (Issue #274)**: Hardened `giveaway::execute` with fail-closed capability verification: restricted execution exclusively to trusted module identity (`module_id == "giveaway"`), mandatory interactive user command origin (`enforce_actor_permissions && actor_id.is_some()`), and verified `MANAGE_GUILD` or `ADMINISTRATOR` privileges before any storage mutation or Discord HTTP dispatch.

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

## 15. Storage Parity & Lua Installer Sandboxing (Issues #207, #208, #209, #221, #224, #231)

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
- **Game Account Links PostgreSQL Schema & Challenge TTL Reset Parity (#231)**:
  - Corrected `PostgresStore::verify_game_link` to evaluate TTL against real table schema (`created_at > NOW() - INTERVAL '15 minutes'`) instead of non-existent `expires_at` column, eliminating SQL `undefined_column` errors (HTTP 500) on `/games/verify`.
  - Updated `create_game_link` conflict resolution (`ON CONFLICT (guild_id, user_id, platform) DO UPDATE`) to refresh `created_at = $8`, ensuring newly issued challenges reset the 15-minute verification window across both PostgreSQL and MemoryStore.

## 16. Duty Policy & Companion Mapping Batch Atomicity (Issues #210, #211, #215)

- **Duty Override Time Limit & Overflow Protection (#210)**:
  - `POST /duty/override` authoritatively fetches guild duty configuration (`get_duty_settings`) and validates `duration_mins` against `1..=max_override_duration_mins` (defaulting to 1440m / 24h if unset). Rejects requests with non-positive duration or durations exceeding the configured guild policy with HTTP 400 Bad Request.
  - Replaced unsafe duration constructors with `chrono::Duration::try_minutes(minutes)` and `checked_add_signed(delta)` across web handlers and Discord command dispatchers, preventing service crashes / panics on extreme numeric values (`i64::MAX`).
- **Voice Companion Batch Quota & Transactional Atomicity (#211, #215)**:
  - Enforced global limit of 50 companion mappings per guild (`MAX_COMPANION_MAPPINGS_PER_GUILD = 50`) at both the REST API preflight layer and persistent storage layer.
  - Hardened batch request payload validation against duplicate entries and Discord API request storms (#215): strictly validates `items.len() <= MAX_COMPANION_MAPPINGS_PER_GUILD` upfront, rejects duplicate `voice_channel_id` entries with HTTP 400 Bad Request prior to any external network calls or database writes, and caches resolved channels across the validation pipeline.
  - Implemented transactional batch persistence (`batch_upsert_companion_mappings`) for PostgreSQL using `pg_advisory_xact_lock` and single-transaction rollbacks on constraint violations, and thread-safe atomic batch persistence for `MemoryStore` under synchronization locks.
  - Segmented `POST /companion-channels/batch` into a three-phase pipeline: (1) upfront preflight validation of all mappings, channel snowflakes, voice!=text invariants, tenant isolation, and live Discord channel types; (2) atomic storage batch upsert guaranteeing all-or-nothing execution without partial persistence; (3) post-commit Discord permission revocation and audit logging.

## 17. Lua Legacy Loader Hardening, Community RBAC & SSE Lifecycle (Issues #212, #213, #214, #216, #217, #220, #222, #223, #225, #226, #227, #228, #229, #230, #232, #233, #234, #235)

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
- **Community Poll Member RBAC & Web Voting Access Boundary (#230)**:
  - `GET /api/guilds/{guild_id}/polls/{poll_id}` and `POST .../vote` relaxed permission checks from `ensure_guild_access` (`MANAGE_GUILD`) to `ensure_guild_member_access`, allowing regular guild members without server management rights to view community polls and cast their votes via the web interface.
  - Administrative creation (`create_poll`) strictly remains protected by `ensure_guild_access` (`MANAGE_GUILD`), preventing unauthorized poll creation.
  - Mutating vote handler strictly enforces CSRF token validation (`verify_csrf`), voter attribution bound to authenticated `session.user.id`, cross-guild tenant boundary matching (`poll.guild_id == guild_id`), and idempotency/expiration checks.
- **Community Features Self-Service Member RBAC Boundary (#232)**:
  - `POST /api/guilds/{guild_id}/counting/{channel_id}`, `GET /api/guilds/{guild_id}/cards`, `POST .../cards/trade`, `POST .../cards/trade/{trade_id}/accept`, `GET /api/guilds/{guild_id}/tuppers`, and `POST .../tuppers` relaxed permission checks from `ensure_guild_access` (`MANAGE_GUILD`) to `ensure_guild_member_access`.
  - Enables regular guild members to participate in counting games, inspect and trade their own card collections, and create/list Tupper RP personas via the web dashboard.
  - Retained strict CSRF validation on all mutating endpoints (`verify_csrf`), actor identity bound to authenticated `session.user.id`, cross-guild tenant boundary matching, and atomic trade ownership transitions.
- **Dashboard Guild Selector Manageable Scope Filtering (#233)**:
  - Frontend `web/static/app.js` filters `state.session.guilds` using 64-bit BigInt bitmask checking for `owner === true`, `ADMINISTRATOR` (`1n << 3n`), or `MANAGE_GUILD` (`1n << 5n`).
  - Precludes member-only guilds from populating the administrative server selector or triggering unauthorized `loadModules()` and administrative SSE stream connection attempts.
  - Displays a clean informative state when no manageable servers exist (`guildSelect.disabled = true`, `state.guildId = null`), while preserving member-only guilds in the session for community self-service endpoints.
- **Read-Only GET Safe Methods CSRF Guard Removal (#234)**:
  - Removed erroneous `verify_csrf(&headers, &session)?` calls from non-mutating HTTP GET handlers: `GET /api/guilds/{guild_id}/store/purchases`, `GET /api/guilds/{guild_id}/members/search`, and `GET /api/guilds/{guild_id}/members/{user_id}/identity-history`.
  - Restores compliance with HTTP safe/idempotent method semantics and frontend helper behavior (`request()` injects `X-CSRF-Token` solely on non-GET methods), eliminating 403 Forbidden errors on administrative and self-service reads while maintaining mandatory CSRF enforcement across all mutating POST/PUT/PATCH/DELETE endpoints.
- **GitHub Webhook Cryptographic HMAC-SHA256 & Deduplication Unification (#235)**:
  - Unified `/api/webhooks/github` route with `handle_inbound_webhook`, executing authentic raw-body HMAC-SHA256 verification via `verify_hmac_sha256` against configured secrets instead of inspecting prefix strings or reporting unverified dispatch success.
  - Rejects missing, malformed, or invalid signatures with HTTP 401 Unauthorized (`Invalid GitHub signature`).
  - Enforces persistent event deduplication using `X-GitHub-Delivery` via `check_and_record_event_dedupe`, ensuring strict idempotency and eliminating fake `discord_embed_dispatched: true` affirmations.
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

## 19. Concurrency, Steam LFG Retention & Action Policy Enforcement (Issues #236, #237, #238, #239, #240, #241)

- **PostgreSQL Event Roster Leave Concurrency & Reserve Promotion Lock (#236)**:
  - Synchronized `PostgresStore::leave_event_roster` with `join_event_roster` by acquiring parent event row lock upfront: `SELECT max_participants FROM scheduled_events WHERE id = $1 FOR UPDATE`.
  - Serialized reserve candidate selection via `FOR UPDATE` and hardened reserve-to-main promotion with atomic condition: `UPDATE event_rosters SET role_type = 'main' WHERE event_id = $1 AND user_id = $2 AND role_type = 'reserve'`.
  - Eliminates concurrent resignation races and duplicate reserve promotions under parallel departures, guaranteeing accurate main counts and preventing vacant spots.
- **MemoryStore Atomic Game Account Links UPSERT Parity (#237)**:
  - Re-keyed `MemoryStore.game_links` from random `Uuid` to the business identity tuple `(String, String, String)` (`guild_id, user_id, platform`).
  - Replaced non-atomic `retain` + `insert` sequence with single atomic `DashMap::insert(key, link)` and keyed lookup `get_mut(&key)`, eliminating race windows during concurrent challenge reissue (64 parallel reissue requests produce exactly one active link).
  - Guarantees strict parity with PostgreSQL's `INSERT ... ON CONFLICT (guild_id, user_id, platform) DO UPDATE`.
- **Steam LFG Quota Limits & Retention Janitor (#238)**:
  - Implemented admission control in `create_steam_lfg`: limits active LFG listings to max 5 per member (`count_active_steam_lfg_posts`) and max 100 per guild (`count_active_steam_lfg_posts_guild`), returning HTTP 429 Too Many Requests upon limit exhaustion.
  - Introduced periodic retention janitor `prune_steam_lfg_posts` running every 60 seconds (and on startup) purging posts expired for more than 7 days across both `MemoryStore` and `PostgresStore`.
  - Added PostgreSQL migration `0034_steam_lfg_retention_index.sql` creating `idx_steam_lfg_posts_expires_at` for high-performance batch cleanup without table scans.
- **Fail-Closed Permission Revalidation on Server Ownership Transfer (#239)**:
  - Hardened `revalidate_guild_permission` in `src/web/auth.rs`: when `GET /guilds/{guild_id}/permissions/member/{user_id}` returns 404 and member fallback lacks computed permissions bitfield, the handler fails closed (`Ok(false)` -> 403 Forbidden).
  - Eliminated dangerous fallback that elevated users based on stale `session.guilds[].owner` snapshot, preventing former guild owners from retaining administrative privileges following ownership transfers.
- **Command Policy Engine Tie-Breaker Fail-Closed Precedence (#240)**:
  - Enhanced candidate comparator in `CommandPolicyEngine::evaluate` to prioritize explicit `Deny` rules over conflicting `Allow` rules on identical precedence matches (`tier`, `role_position`, `priority`, `resource_specificity`).
  - Added secondary deterministic tie-breaker sorting by rule UUID (`a.rule.id.cmp(&b.rule.id)`), ensuring evaluation results never depend on database query row ordering or `DashMap` iteration order.
- **Lua Action Policy ACL Runtime Enforcement (#241)**:
  - Added canonical `LuaAction::policy_action_key(&self) -> &'static str` matching Serde action types (`ban_member`, `kick_member`, `timeout_member`, `send_message`, `purge`, etc.).
  - Embedded host policy check in `src/discord/executor.rs:execute_action` prior to action execution and side effects, evaluating `PolicyResource::Action { action_key }` against authoritative guild policy rules.
  - Rejects actions denied by guild policy with descriptive error without executing Discord HTTP calls, ensuring parity between `/policy/explain` dry-runs and actual Lua runtime execution.

## 20. Lua Concurrency, Recruitment Performance, AI Knowledge Hardening & Logout Reliability (Issues #242, #243, #244, #245, #246, #247)

- **Lua Engine Semaphore Eviction & Limiter Integrity (#242)**:
  - Extended semaphore eviction predicate in `src/lua/engine.rs:get_guild_semaphore`: retained semaphores if `Arc::strong_count(sem) > 1 || sem.available_permits() < self.max_per_guild`.
  - Prevents active guild semaphores from being evicted when semaphore cache exceeds threshold (>500 entries) during burst traffic across multiple guilds.
  - Eliminates duplicate semaphore instantiation that bypassed `max_per_guild` parallelism limits and caused admission control starvation.
- **Staff Recruitment N+1 Elimination, Batch Fetching & Fail-Safe Error Handling (#243)**:
  - Eliminated N+1 query pattern (1+2N queries) in `features::list_recruitment_applications`: implemented batch loaders `list_recruitment_reviews_for_applications` and `get_role_fulfillments_for_applications` across `MemoryStore`, `PostgresStore` and `Storage` using SQL `WHERE application_id = ANY($1)`.
  - Added query pagination with `limit` (default 20, clamp 1..=100) and `offset` in `ListRecruitmentApplicationsQuery`, sorting deterministically by `submitted_at DESC, id DESC`.
  - Replaced silent error masking (`unwrap_or_default()`) with strict error propagation via `map_err(ApiError::internal)?` on review and fulfillment fetch failures.
- **AI Knowledge Search LIKE Sanitization, Database ACL Filtering & Pagination (#244)**:
  - Hardened `search_ai_documents` against wildcard amplification: escaped special LIKE characters (`\`, `%`, `_`) to ensure literal search matching across both `MemoryStore` and `PostgresStore`.
  - Relocated channel ACL filtering directly into PostgreSQL query (`CASE WHEN jsonb_typeof(channel_acl) = 'array' THEN jsonb_array_length(channel_acl) = 0 OR channel_acl ?| $3::text[] ELSE FALSE END`), preventing full document text payloads from loading into memory before authorization checks.
  - Added strict fail-closed handling for malformed or non-array `channel_acl` values across both storage engines.
  - Enforced result limit clamping (1..=50, default 20) with deterministic ordering (`ORDER BY created_at DESC, id DESC`), empty query immediate zero-result fast path, and query length validation (max 500 characters) in `POST /api/guilds/{guild_id}/ai/search`.
- **Fail-Closed Session Deletion on Logout (#245)**:
  - Updated `logout` in `src/web/auth.rs`: propagated session deletion failure with `map_err(ApiError::internal)?` instead of ignoring it (`let _ = ...`).
  - Updated `web/static/app.js`: UI only transitions to logged-out state (`showLogin()`) upon successful HTTP response; on failure, displays an error notification and preserves state, preventing client-side false sense of logout when server session remains active.
- **Reverse Proxy X-Forwarded-For Right-to-Left Traversal & Rate-Limit Hardening (#246)**:
  - Implemented recursive right-to-left resolution in `src/web/auth.rs:resolve_client_ip`: walks backwards across `TRUSTED_PROXIES` hops to extract the actual untrusted client IP, ignoring attacker-spoofed leftmost addresses.
  - Guaranteed enforcement of per-IP login rate limiting (10 attempts/min) behind reverse proxies operating in append mode (`$proxy_add_x_forwarded_for`), preventing quota splitting and client impersonation.
  - Documented proxy configuration guides for Nginx and Traefik in `docs/SECURITY.md`.
- **Dynamic Voice Cleanup CSRF Hardening, Method Safety & Atomic TOCTOU Prevention (#247)**:
  - Migrated dynamic voice cleanup from state-changing `GET` to `POST /api/guilds/{guild_id}/voice/dynamic/cleanup` in `src/web/mod.rs`, rejecting top-level `GET` with HTTP 405 Method Not Allowed to eliminate cross-site request forgery risks under `SameSite=Lax`.
  - Added strict `verify_csrf(&headers, &session)?` check in `src/web/features.rs:cleanup_dynamic_voice`.
  - Implemented atomic guild-scoped cleanup in `Storage::cleanup_stale_dynamic_voice_rooms`: in PostgreSQL, executes `DELETE FROM dynamic_voice_rooms WHERE guild_id = $1 AND empty_since IS NOT NULL AND empty_since <= $2 RETURNING channel_id`, and in MemoryStore atomically prunes matching entries under lock, preventing TOCTOU races when empty rooms are reoccupied during cleanup.
  - Aligned response payload semantics with `removed_records_count` and `removed_channel_ids` alongside backwards-compatible aliases.

## 21. FAQ Query Pagination, Data Retention Janitors, Command Budget & Host Policy Protection (Issues #248, #249, #250, #251, #252)

- **FAQ Search SQL Sanitization, Literal Matching & Pagination (#248)**:
  - Hardened `list_faq_entries` across `MemoryStore`, `PostgresStore`, and `Storage`: escaped SQL wildcard characters (`\`, `%`, `_`) using `ESCAPE '\'` to guarantee exact literal substring matching parity between memory and PostgreSQL backends.
  - Added query pagination with `limit` (default 20, clamped 1..=50) and `offset` (default 0), sorted deterministically by `created_at DESC, id DESC`.
  - Added query string validation in `src/web/features.rs:list_faq` rejecting search strings longer than 200 characters with HTTP 400 Bad Request.
- **Audit Log & Ticket Transcript Retention Janitor & TTL Enforcement (#249)**:
  - Enforced declared data lifecycle (45-day audit log retention, 90-day ticket transcript retention) at query and storage levels.
  - Implemented `prune_audit_events(cutoff, limit)` and `prune_transcripts(cutoff, limit)` across `PostgresStore`, `MemoryStore`, and `Storage`/`TicketStore` executing batched pruning (`LIMIT $2`) to avoid long table locks.
  - Added 45-day retention filtering (`created_at >= NOW() - INTERVAL '45 days'`) to `list_audit_events`, `list_audit_events_page`, and `count_audit_events`.
  - Integrated periodic background janitor in `src/web/mod.rs` running every 60 seconds with migration `0035_audit_transcripts_retention_indexes.sql` adding indexes on `created_at` and `(guild_id, created_at DESC, id DESC)`.
- **Slash Commands Per-Scope Limit Accounting for Host Commands (#250)**:
  - Accounted for 5 reserved native host commands (`policy`, `duty`, `help`, `emoji`, `sticker`) within Discord's 100 CHAT_INPUT slash command limit per scope.
  - Enforced maximum limit of 95 Lua commands (`MAX_LUA_COMMANDS = 95`) at Lua manifest load time (`src/lua/engine.rs`), failing fast on engine startup with actionable error messages when exceeded.
  - Added compile-time and runtime validation in `src/discord/commands.rs:register` verifying that total commands (`definitions.len() + 5`) do not exceed `MAX_CHAT_INPUT_COMMANDS_PER_SCOPE = 100`.
  - Updated developer documentation in `docs/lua/manifests.md`.
- **Audit Log Database-Level Pagination & Accurate Counter (#251)**:
  - Refactored `GET /api/guilds/{guild_id}/audit` to execute pagination directly at the database level instead of loading 10,000 records into process memory.
  - Added `Storage::list_audit_events_page(guild_id, limit, offset)` and `Storage::count_audit_events(guild_id)` across `MemoryStore`, `PostgresStore`, and `Storage`.
  - Preserved existing JSON response contract (`items`, `total`, `next_offset`) while returning accurate unexpired audit event counts and bounded page results.
- **Host Commands Policy Protection & Rate Limiting with Recovery Exception (#252)**:
  - Unified command dispatch guard via `evaluate_command_guard` in `src/discord/handler.rs` covering `/duty`, `/help`, `/emoji`, `/sticker` alongside Lua commands.
  - Enforced per-user and per-guild rate limiting (`RateLimiter::check_command`) and Command Policy evaluation (`CommandPolicyEngine::evaluate`) for built-in host commands.
  - Preserved administrative/owner recovery exception for `/policy` (`handle_policy_builtin_command`), preventing accidental policy lockouts.
  - Fixed `/policy explain` target command resolution in Discord slash command handler, mapping host commands (`duty`, `help`, `emoji`, `sticker`, `policy`) to their respective module IDs for consistent policy explain trace output.
- **Ticket Close/Reopen Compensating Rollback & Reconciliation (#253)**:
  - Implemented transactional compensation in `src/discord/tickets.rs` across both `TicketOperation::Reopen` and `close_ticket`: tracks all applied permission overwrites and channel modifications, immediately rolling back changes (`set_member_read_only` / `restore_member_access` and channel name/category restore) if any intermediate Discord API call fails, or if the database mutation fails or returns `InvalidState`.
  - Added `reconcile_ticket_channel(ctx, state, guild_id, channel_id)` helper guaranteeing convergence between authoritative DB ticket status and Discord channel write permissions.
  - Added migration `0036_ticket_channel_transitions.sql` defining `ticket_channel_transitions` for durable tracking of channel state transitions.
- **Ticket Transcript Encryption at Rest (AEAD AES-256-GCM) with Key Rotation (#254)**:
  - Implemented application-layer encryption for archived ticket transcripts at rest in PostgreSQL (`ticket_transcripts`) using authenticated encryption (AES-256-GCM via `aes-gcm = "=0.10.3"`).
  - Designed per-write 96-bit random nonce generation using CSPRNG (`OsRng`) and authenticated additional data (AAD: `transcript:v1:guild={guild_id}:ticket={ticket_id}`) preventing cross-guild and cross-ticket ciphertext tampering or swap attacks.
  - Implemented versioned keyring support (`TranscriptCrypto`) with `TRANSCRIPT_ACTIVE_KEY_ID`, `TRANSCRIPT_ENCRYPTION_KEY`, and `TRANSCRIPT_KEYRING` allowing seamless zero-downtime key rotation while retaining read access for historical transcripts.
  - Enforced fail-closed security: missing active key prevents plaintext fallback, refusing to persist unencrypted records. Debug representation redacts raw key bytes to avoid secret leakage in telemetry, audit trails, and logs.
  - Added migration `0037_encrypted_ticket_transcripts.sql` introducing `ciphertext BYTEA`, `nonce BYTEA`, `key_id TEXT`, `crypto_version SMALLINT`, updating check constraints (`char_length(content) > 0 OR content IS NULL`) and payload integrity constraints.
  - Provided controlled batch-limited backfill mechanism (`PostgresTicketStore::backfill_encrypt_transcripts`) for migration of pre-existing unencrypted rows.
- **PostgreSQL Transcript Encryption Startup Preflight & Operator Contracts (#255)**:
  - Enforced fail-fast startup preflight in `AppState::bootstrap`: when `DATABASE_URL` is set, `TRANSCRIPT_ENCRYPTION_KEY` or `TRANSCRIPT_KEYRING` is required unless `TICKET_TRANSCRIPTS_DISABLED=true`. Prevents "seemingly healthy" server start with broken `/ticket close` or `/ticket transcript` workflows.
  - Added `TICKET_TRANSCRIPTS_DISABLED` configuration flag: allows operators to explicitly opt out of transcript storage in PostgreSQL without breaking ticket closures.
  - Updated `close_ticket` and `TicketOperation::Transcript` in `src/discord/tickets.rs`: when transcripts are disabled, `/ticket close` proceeds safely without saving transcripts, and `/ticket transcript` informs callers with a clear configuration message.
  - Added ticket transcript health check diagnostics in `/health/ready` reporting `configured / disabled / unready` status without secret exposure.
  - Updated `.env.example`, `compose.yaml`, and `docs/TICKETS.md` documenting key generation (`openssl rand -hex 32`), keyring rotation, and deployment requirements.
- **Config Debug Redaction for Secrets and Credentials (#256)**:
  - Replaced derived `#[derive(Debug)]` on `Config` with a custom `fmt::Debug` implementation in `src/config.rs`.
  - Redacted all sensitive secrets (`discord_token`, `discord_client_secret`, database connection credentials in `database_url`, and AEAD transcript encryption keys in `transcript_encryption_key` / `transcript_keyring`).
  - Emitted safe non-sensitive booleans (`postgres_configured`, `transcript_crypto_configured`) preserving operational and diagnostic insight without exposing credentials in logs, traces, crash dumps, or telemetry.
  - Added unit test `test_config_debug_redacts_all_secrets` asserting zero leakage across all secret fields.
- **Readiness Error Sanitization & Metrics Endpoint Protection (#257)**:
  - Sanitized `/health/ready` endpoint: replaced raw SQLx database error strings in public JSON with fixed `"unavailable"` status and server-side `tracing::warn!` logging, preventing information disclosure of internal infrastructure, hostnames, and credentials during outages.
  - Hardened `/metrics` Prometheus endpoint with multi-factor authorization (`is_metrics_authorized`): supports Bearer token (`METRICS_TOKEN`), active authenticated dashboard user session, and network-level scraper filtering (`METRICS_ALLOWED_IPS` or default loopback connections when token is unset).
  - Maintained complete zero-secret compatibility with Docker Compose / Kubernetes container health checks (`GET /health/ready` returns 200/503 without requiring authentication).
  - Redacted `metrics_token` in `Config` `Debug` formatting (emitted as `metrics_token_configured: bool` and safe `metrics_allowed_ips`).
- **Moderator Duty Schedule Validation & Non-Destructive Diagnostics (#258)**:
  - Enforced strict input validation for moderator duty recurring schedules in `POST /api/guilds/{guild_id}/duty/profiles/{user_id}` and `ModeratorDutyProfile::new`: all time strings must match canonical `HH:MM` format via `DutyTimeRange::new` and `RecurringSchedule::validate`. Out-of-range hours (`25:00`), invalid minutes (`18:60`), malformed text (`tomorrow`), and empty strings are rejected with HTTP 400 Bad Request with granular day and index error details.
  - Implemented atomic rejection: invalid payloads are rejected before storage mutation, ensuring existing valid schedules are not corrupted or overwritten.
  - Enhanced on-call evaluation (`compute_is_on_call`): replaced silent ignoring of invalid ranges with structured operator warning logs (`tracing::warn!`) including guild ID, user ID, weekday, and raw range values.
  - Added non-destructive diagnostics for legacy records: `GET /api/guilds/{guild_id}/duty/profiles/{user_id}` and `GET /api/guilds/{guild_id}/duty/profiles` expose `invalid_schedule` containing validation errors without deleting or mutating stored profiles, allowing administrators to inspect and fix legacy schedules safely.
- **PostgreSQL Scheduler Expired Lease Terminalization Resiliency & Telemetry (#259)**:
  - Fixed silent error discarding in `PostgresSchedulerStore::claim_due`: replaced `let _ = sqlx::query(...).execute(...).await;` with observable janitor method `finalize_expired_leases`.
  - Added structured operator error logging via `tracing::error!(error = %error, "scheduler: unable to finalize expired processing leases")` upon database failure during cleanup, preventing invisible stuck `processing` jobs.
  - Implemented Prometheus telemetry counters: `scheduler_expired_lease_cleanup_errors_total` (tracking database errors during lease terminalization) and `scheduler_expired_lease_terminalized_total` (tracking expired processing leases transitioned to terminal `failed` status after exhausting `max_attempts`).
  - Preserved guild-isolation and non-blocking worker polling (#60): failures during stale lease terminalization log errors and increment metrics while allowing `claim_due` to continue claiming active due jobs without halting cross-guild work.
  - Symmetrical parity between `MemorySchedulerStore` and `PostgresSchedulerStore`: both stores expose `finalize_expired_leases` and propagate telemetry metrics, unblocking user quotas once expired attempts are terminalized.
- **Voice Companion Bounded Lock Lifecycle & Weak Reference Janitor (#260)**:
  - Replaced permanent strong mutex retention in `USER_LOCKS` (`DashMap<(String, String), Arc<Mutex<()>>>`) with ephemeral weak references (`Weak<Mutex<()>>`).
  - Implemented safe reference upgrade in `get_user_lock`: concurrent operations for the same `(guild, user)` continue to share the exact same active `Mutex` instance without race conditions, while dropping locks when tasks complete.
  - Added background lock janitor `run_companion_lock_janitor` running every 5 minutes (and opportunistic sweep on guild reconciliation) using `prune_unused_user_locks` (`locks.retain(|_, weak| weak.strong_count() != 0)`), preventing unbounded RAM growth from inactive voice participants.
- **Voice Companion Targeted Per-Member Lease Resolution & Index (#261)**:
  - Eliminated O(N) full-guild lease scanning in `reconcile_companion_channel_access`: replaced duplicate guild-wide fetches (`list_active_companion_leases` + `list_pending_revoke_companion_leases`) with targeted per-member method `list_member_companion_leases(guild_id, user_id)`.
  - Added additive migration `0038_companion_leases_user_status_index.sql` creating compound index `idx_companion_leases_guild_user_status` on `companion_channel_leases (guild_id, user_id, status)`.
  - Implemented symmetrical support across both `MemoryStore` and `PostgresStore` with strict tenant isolation and snowflake validation, ensuring voice state updates fetch only relevant records without deserializing or iterating over unrelated guild members' leases.
- **Voice Companion Runtime Enforcement of Mapping Enabled Semantics (#262)**:
  - Fixed security loophole in `reconcile_companion_channel_access`: resolved target mapping is explicitly filtered with `.filter(|m| m.enabled)`, preventing unauthorized permission grants for disabled mappings and properly revoking (`keep = false`) existing leases if a mapping was operational-disabled.
  - Implemented defense-in-depth in `grant_companion_access`: immediately short-circuits with structured warning if `!mapping.enabled`, preventing accidental permission overwrite creation on Discord and lease persistence.
  - Resolved phase 3 discrepancy in `reconcile_guild_companion_channels`: target mapping lookup now strictly enforces `m.enabled && m.voice_channel_id == voice_str`, preventing reconciliation from immediately re-granting leases revoked in phase 2.
  - Enhanced `CompanionMappingItem` and `UpsertCompanionMappingPayload` with `enabled: bool` across `PostgresStore` and `MemoryStore`: deactivating a mapping (`enabled = false`) atomically transitions all associated active leases to `revoke_pending` with immediate retry schedule (`next_attempt_at = NOW()`).
  - Added comprehensive test coverage: unit tests in `src/storage/mod.rs`, PostgreSQL integration tests in `tests/postgres_integration.rs`, and mock Discord HTTP event tests in `tests/discord_isolation_and_hierarchy.rs`.
- **Voice Companion Safe Startup Reconciliation & Cache Availability Deferral (#263)**:
  - Disambiguated `guild cache unavailable` from `guild has zero voice states`: `reconcile_guild_companion_channels` fails fast and defers reconciliation (`Guild {guild_id} cache is unavailable; deferring companion reconciliation`) when guild cache is absent, eliminating mass unjustified permission overwrite deletion and lease purging during Gateway startup.
  - Extracted `reconcile_guild_companion_channels_with_voice_states` allowing reconciliation against an authoritative, reliable `HashMap<UserId, VoiceState>` snapshot.
  - Shifted startup companion reconciliation from premature `READY` event handler (where guilds are initially unavailable in Serenity cache) to `guild_create`, running only when full guild payload and voice states are reliably populated.
  - Added comprehensive verification in `tests/discord_isolation_and_hierarchy.rs` (`test_voice_companion_ready_empty_cache_defers_reconciliation_without_revocation`) verifying zero HTTP mutations, active lease preservation upon cache absence, and proper cleanup once an authoritative empty voice state arrives.
- **Giveaway Lua -> Rust Typed Action Pipeline (#264)**:
  - Replaced mock text replies in `scripts/giveaway/module.lua` with typed declarative actions `LuaAction::Giveaway { operation: GiveawayOperation }` covering `/giveaway start`, `/giveaway end`, and `/giveaway reroll`.
  - Added robust validation in `GiveawayOperation::validate()` enforcing bounded prizes (1..=256 chars), duration limits (1..=10080 minutes), winner limits (1..=20), snowflake validation, and strict UUID parsing for target giveaways.
  - Implemented privileged host executor in `src/discord/giveaway.rs` with guild context enforcement, `MANAGE_GUILD` / `ADMINISTRATOR` RBAC checks, durable database mutation via storage layer, channel announcement posting, and structured audit trail generation.
- **Giveaway Lifecycle Separation & Premature Reroll Guard (#265)**:
  - Separated first conclusion / winner selection (`finish_giveaway`, `POST /api/guilds/{guild_id}/giveaways/{giveaway_id}/end`) from rerolling (`reroll_giveaway`, `POST .../reroll`).
  - Added strict lifecycle state guard in `reroll_giveaway`: rejects premature reroll attempts on active (`ended = false`) giveaways across storage (`ensure!(ga.ended)`) and web API (HTTP 409 Conflict) without mutating `ended` or winners state, preserving ongoing participant entry (`join_giveaway`).
  - Ensured idempotent conclusion: `finish_giveaway` returns existing winners on repeated invocations without re-drawing.
  - Enforced winner replacement semantics in `reroll_giveaway`: prefers non-winning participants to replace prior winners and strictly rejects drawings without participants with HTTP 400 Bad Request, preventing false-positive `rerolled: true` reports.

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
