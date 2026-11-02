# Rusty Discord Bot (`rusty-discord-bot`)

A secure, modular, Lua-scriptable Discord automation platform with a high-performance Rust core, PostgreSQL storage, voice support, XML plugin installer and a first-party web dashboard.

Rusty Discord Bot is designed as a modern, extensible alternative to closed all-in-one bots. Server behavior lives in self-contained, versioned Lua modules (each in its own folder with `module.lua`, `config.json` and `README.md`); Discord connectivity, authorization, storage, rate-sensitive operations and privileged actions remain behind a validated Rust API.

> **Current status:** advanced release. The repository contains the modular runtime, `install.xml` plugin registry, dashboard, OAuth2 login, PostgreSQL migrations, Discord command registration, voice queue integration and 26 modular Lua feature packages. The complete product scope is tracked in [`docs/FEATURE_MATRIX.md`](docs/FEATURE_MATRIX.md), while the audited Discord API surface is recorded in [`docs/DISCORD_PLATFORM_COVERAGE.md`](docs/DISCORD_PLATFORM_COVERAGE.md). [`docs/VERSION_POLICY.md`](docs/VERSION_POLICY.md) defines the latest-stable dependency policy.

## Why Rust plus Lua?

Rust owns the trust boundary:

- Discord Gateway and HTTP clients
- XML plugin manifest parser and module supervisor (`install.xml`)
- permission and role-hierarchy checks
- OAuth2 and browser sessions
- database access and audit records
- voice connections and media source validation
- Lua memory, instruction and action limits

Lua owns server behavior:

- slash-command manifests
- command installation and interaction contexts
- command options and choices
- reactions to Discord events (`message_create`, `message_update`, `message_delete`, `guild_member_add`, `guild_member_remove`)
- per-guild configuration schemas
- declarative actions returned to the Rust host (messages, `send_direct_message`, moderation, roles, music, scheduler, tickets, audit)

A Lua script never receives the Discord bot token, raw database credentials, unrestricted network access or direct filesystem access.

## Modular Folder Architecture & `install.xml`

Each Lua feature package lives in its own dedicated folder under `scripts/<module_name>/`:
- `module.lua`: declarative manifest, command handlers and event subscribers
- `config.json`: default / baseline configuration for the module
- `README.md`: feature documentation, options, permissions and examples

Right before the folders sits `scripts/install.xml`, the master plugin registry:
```xml
<?xml version="1.0" encoding="UTF-8"?>
<installer version="1.0.0">
    <bot name="rusty-discord-bot" min_engine_version="0.2.0" author="Adrian Banach" />
    <modules>
        <module id="core" enabled="true" version="1.0.0">
            <name>Core System</name>
            <description>Podstawowe komendy diagnostyczne, ping, informacja o platformie i spis modułów.</description>
            <category>platform</category>
            <folder>core</folder>
            <entry>module.lua</entry>
            <config>config.json</config>
            <author>Adrian Banach</author>
        </module>
        <!-- ... additional modules ... -->
    </modules>
</installer>
```
Modules can be toggled on/off globally by setting `enabled="true"` or `enabled="false"` in `install.xml`. The Rust engine respects this descriptor before loading code into memory.

## Included 26 Modular Feature Packages

| Category | Module Folder | Feature Summary |
| --- | --- | --- |
| **Platform** | `scripts/core/` | `/ping`, `/about` — diagnostyka i stan silnika |
| **Safety** | `scripts/moderation/` | `/warn`, `/moderate`, `/case` — sankcje, unieważnienia i trwała ewidencja spraw |
| **Safety** | `scripts/automod/` | Filtry słów, anty-spam, anty-caps, link guard i blokada zaproszeń |
| **Support** | `scripts/tickets/` | `/ticket` — prywatne kanały pomocy, kolejki wsparcia i transkrypcje |
| **Utility** | `scripts/scheduler/` | `/remind`, `/schedule` — terminowe przypomnienia i zadania cykliczne |
| **Progression**| `scripts/progression/`| `/rank`, `/leaderboard`, `/daily`, `/rep` — poziomy XP i role za level |
| **Economy** | `scripts/economy/` | `/economy` — portfel, praca, zlecenia crime, sklep serwerowy, ekwipunek i przelewy |
| **Giveaway** | `scripts/giveaway/` | `/giveaway` — konkursy, losowania nagród i wybór zwycięzców |
| **Social** | `scripts/starboard/` | `/starboard` — tablica chwały dla wiadomości wyróżnionych gwiazdkami |
| **Utility** | `scripts/afk/` | `/afk` — statusy nieobecności z automatycznym powiadomieniem wzmiankujących |
| **Utility** | `scripts/tags/` | `/tag` — własne komendy tekstowe, szablony i skróty |
| **Community** | `scripts/birthdays/` | `/birthday` — kalendarz urodzin serwera i życzenia |
| **Security** | `scripts/logging/` | `/logging` — rejestr zdarzeń serwerowych i usuniętych wiadomości |
| **Management**| `scripts/reaction_roles/`| `/reactionrole` — samodzielny wybór ról poprzez reakcje pod wiadomościami |
| **Fun** | `scripts/quotes/` | `/quote` — baza złotych myśli i pamiętnych cytatów serwera |
| **Progression**| `scripts/achievements/`| `/achievements` — system odznak i osiągnięć serwerowych z nagrodami |
| **Multimedia** | `scripts/music/` | `/music` — odtwarzacz audio Songbird ze wsparciem kolejki |
| **Community** | `scripts/community/` | `/poll`, `/suggest`, `/report` — ankiety, sugestie i zgłoszenia |
| **Management** | `scripts/welcome/` | Powitania nowych użytkowników i automatyczne nadawanie roli |
| **Security** | `scripts/join_guard/`| `/joinguard` — ochrona anty-raid i weryfikacja wieku kont |
| **Management** | `scripts/roles/` | `/role` — komendy przydzielania i odbierania ról |
| **Fun** | `scripts/fun/` | `/roll`, `/8ball`, `/choose`, `/reverse` — rozrywka tekstowa |
| **Fun** | `scripts/games/` | `/coinflip`, `/trivia`, `/rps`, `/guess` — quizy i mini-gry |
| **Utility** | `scripts/utility/` | `/userinfo`, `/avatar`, `/serverinfo`, `/calculate` |
| **Management** | `scripts/staff_tools/`| `/slowmode`, `/lockdown`, `/unlock`, `/announce` |
| **Information**| `scripts/server_info/`| `/server`, `/channel`, `/rules` |

## Architecture

```text
Discord Gateway / Interactions
             |
             v
      Rust Discord host
             |
     +-------+--------+
     |                |
     v                v
Lua sandbox      Action validator
     |                |
     +-------> allowed actions
                      |
          +-----------+-----------+
          |           |           |
          v           v           v
      Discord API  PostgreSQL   Songbird
          ^
          |
Discord OAuth2 web dashboard
```

Read [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for trust boundaries and request flows. The scheduler contract is documented in [`docs/SCHEDULER.md`](docs/SCHEDULER.md), and the private support workflow in [`docs/TICKETS.md`](docs/TICKETS.md).

## Local development

### Requirements

- Rust 1.99.1; CI verifies that the pinned toolchain matches Rust's current stable channel
- PostgreSQL 18.6 through the supplied Compose stack, unless using the volatile in-memory fallback
- a Discord application with a bot user
- `ffmpeg`, `yt-dlp` and Opus runtime support for music

### Discord application

1. Create a Discord application and bot.
2. Enable the **Server Members Intent** and **Message Content Intent** while the bundled event modules are in use.
3. Add the OAuth2 redirect URI used by `DISCORD_OAUTH_REDIRECT_URL`.
4. Invite the bot with the `bot` and `applications.commands` scopes.
5. Grant only the permissions needed by enabled modules. Do not use Administrator in production unless there is a documented reason.

### Configure

```bash
cp .env.example .env
```

Fill in at least:

```dotenv
DISCORD_TOKEN=...
DISCORD_APPLICATION_ID=...
DISCORD_CLIENT_ID=...
DISCORD_CLIENT_SECRET=...
DISCORD_OAUTH_REDIRECT_URL=http://127.0.0.1:8080/auth/discord/callback
DISCORD_DEV_GUILD_ID=...
```

Using `DISCORD_DEV_GUILD_ID` registers commands on one development server, where updates appear immediately. Remove it before production to register global commands with their declared installation and interaction contexts.

### Run with Docker Compose

```bash
docker compose up --build
```

Open the dashboard at `http://127.0.0.1:8080`.

### Run directly

```bash
cargo run
```

Without `DATABASE_URL`, configuration is kept only in memory and is lost when the process exits.

## A minimal Lua module

```lua
return {
    manifest = {
        id = "hello",
        name = "Hello",
        version = "1.0.0",
        description = "A minimal Rusty Discord Bot module.",
        category = "example",
        default_enabled = true,
        commands = {
            {
                name = "hello",
                description = "Greets the invoking user.",
                integration_types = { "guild", "user" },
                contexts = { "guild", "bot_dm", "private_channel" },
                nsfw = false,
            },
        },
        config_schema = {
            type = "object",
            additionalProperties = false,
            properties = {},
        },
    },

    on_command = function(command, ctx)
        if command == "hello" then
            return {
                {
                    type = "reply",
                    content = "Hello, " .. rusty-discord-bot.escape_mentions(ctx.user_name) .. ".",
                    ephemeral = false,
                },
            }
        end
        return {}
    end,
}
```

The full manifest, context and action contract is documented in [`docs/LUA_API.md`](docs/LUA_API.md).

## Repository layout

```text
src/
  discord/       Discord events, command registration and action execution
  lua/           sandbox, manifests, contexts and action contract
  storage/       guild configuration, audit and module data stores
  scheduler/     persistent jobs, leasing, retries and recurrence
  tickets/       persistent support cases, participants and transcripts
  web/           OAuth2 dashboard and API
scripts/         first-party Lua modules
web/static/      dependency-free dashboard frontend
migrations/      PostgreSQL schema
docs/            architecture, security, API and product scope
tests/           Lua runtime and safety tests
```

## Development commands

```bash
bash scripts/check-latest-rust.sh
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release
docker build --tag rusty-discord-bot:local .
```

## Music and external services

The music module is a host integration, not a license to copy protected content or bypass a platform's access rules. Deployments must use sources they are permitted to access and must comply with the source platform's terms, applicable copyright law and Discord's policies. URL input is restricted to HTTPS hosts configured through `MUSIC_ALLOWED_HOSTS`.

## Security

Read [`SECURITY.md`](SECURITY.md) and [`docs/SECURITY.md`](docs/SECURITY.md) before exposing a deployment to untrusted users. The Lua sandbox is a defense layer, not a substitute for reviewing third-party modules.

## Contributing

Review [`AGENTS.md`](AGENTS.md) and [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md). Changes to the Lua action contract must include validation, authorization, audit behavior and tests in the same pull request.
