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
| Command enable/disable by channel | Planned |
| Command enable/disable by role | Planned |
| Command cooldowns | Planned |
| Per-user and per-role rate limits | Planned |
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
| Server counters in channel names | Planned |

## 3. Moderation and safety

| Capability | Status |
| --- | --- |
| Timeout action | Implemented |
| Kick action | Implemented |
| Ban action | Implemented |
| Message delete action | Implemented |
| Purge up to 100 messages | Implemented |
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
| Moderator duty/availability status | Planned |
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
| Import from other bots | Implemented |
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
