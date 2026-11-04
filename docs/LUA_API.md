# Lua module API

This is the entry point for ZuckerBot's Lua API **1**, checked against source snapshot [`10a2ba1`](https://github.com/abanach-com/rust-discord-bot/tree/10a2ba1780a2becd6d7ba2bfc5c4f7a2fecc9882) on 2026-11-04. The [Lua author handbook](lua/README.md) contains the complete reference, and [examples/lua](../examples/lua/README.md) contains three installable example packages. For a Polish introduction, start with [Polski szybki start](lua/quickstart-pl.md).

Lua receives a context and returns action requests. Rust owns authorization, persistent state, Discord delivery and service execution. The [support matrix](lua/support-matrix.md) separates implemented paths from helpers, registrations and future features.

## Complete reference

| Chapter | Scope |
| --- | --- |
| [1. Runtime](lua/runtime.md) | Discovery, loading, fresh VMs, configuration and execution phases |
| [2. Quickstart](lua/quickstart.md) | Complete module, JSON defaults and XML installation |
| [3. Manifests](lua/manifests.md) | Every manifest, command, option and installer field |
| [4. Contexts and events](lua/contexts-events.md) | Callback signatures, nested options and all five dispatched events |
| [5. Actions](lua/actions.md) | All 16 action types, fields, bounds, delivery and permissions |
| [6. Host services](lua/services.md) | State ownership, reserved module identities and service results |
| [7. Progression](lua/progression.md) | Profiles, balances, daily rewards, transfers and staff adjustments |
| [8. Scheduler](lua/scheduler.md) | Durable jobs, policies, time syntax, ownership and retries |
| [9. Tickets](lua/tickets.md) | Opening, policies, participants, transcripts and lifecycle |
| [10. Moderation cases](lua/moderation-cases.md) | Cases, sanctions, notes, evidence and visibility |
| [11. Helpers and translations](lua/helpers-i18n.md) | Complete helper table, argument rules and catalogue loading |
| [12. Security and errors](lua/security-errors.md) | Sandbox, resource budgets, authorization and partial execution |
| [13. Testing](lua/testing.md) | Engine fixtures, configuration validation and diagnostic workflow |
| [14. Recipes](lua/recipes.md) | Complete packages and practical callback fragments |
| [15. Support matrix](lua/support-matrix.md) | Current limitations, known defects and bundled module inventory |

## Module contract

A module file must return a table containing its `manifest` and exported callbacks. Returning only a manifest, or defining a global callback without exporting it, is not this contract.

```lua
return {
    manifest = {
        id = "hello", name = "Hello", version = "1.0.0",
        description = "A minimal module", category = "examples",
        commands = {{ name = "hello", description = "Greet the invoking user" }},
        config_schema = { type = "object", additionalProperties = false },
    },
    on_command = function(command, ctx)
        if command ~= "hello" then return {} end
        return {{
            type = "reply",
            content = "Hello, " .. rustybot.escape_mentions(ctx.user_name) .. ".",
            ephemeral = true,
        }}
    end,
}
```

Install packages through an `install.xml` whose root is `installer` or `install`, or use the documented legacy discovery mode. See [quickstart](lua/quickstart.md) for the complete file layout. The host evaluates source in a fresh VM for each invocation; globals and closures do not persist between calls.

The callbacks are `on_command(command, ctx)` and `on_event(event, ctx)`. Both return a dense, one-based array of action tables; `{}` means no actions. Always use the array wrapper and do not rely on malformed keyed tables producing a particular error.

## Manifest

Required fields are `id`, `name`, `version`, `description` and `category`. Optional fields include `default_enabled`, `commands`, `events`, `config_schema`, `author` and `folder`. The [manifest reference](lua/manifests.md) describes defaults and validation; the XML installer has a separate selection/metadata contract.

Declare an object configuration schema when the callback expects an object. Saved guild configuration replaces package defaults; it is not a deep merge. Loading compiles the schema but does not validate the package's actual `config.json` against it. Validate defaults explicitly; [#128](https://github.com/abanach-com/rust-discord-bot/issues/128) records failures in shipped packages at this snapshot.

## Command definition

Commands declare their name, description, nested options, default member permissions and installation/interaction contexts. Required ordinary options precede optional ones. Discord IDs and permission bitsets are decimal strings; avoid converting IDs through numeric types.

### Installation and interaction contexts

The default is guild installation and guild interaction. `integration_types` supports `guild` and `user`; `contexts` supports `guild`, `bot_dm` and `private_channel`. Private-channel interaction requires user installation. Deprecated `dm_permission` cannot be combined with `contexts`. See [the full context rules](lua/manifests.md) for registration differences and validation.

## Command context

`ctx` contains `guild_id` (optional), `channel_id`, `user_id`, `user_name`, `member_roles`, `member_permissions`, `locale`, `options` and `config`. [The field reference](lua/contexts-events.md) describes their types and origin.

Subcommands are nested in `options`; there is no host-supplied `options.subcommand` discriminator. For `/economy transfer recipient:... amount:...`, read:

```lua
local transfer = (ctx.options or {}).transfer
if transfer then
    local recipient = transfer.recipient -- ID string from the option declaration
    local amount = transfer.amount
    -- Route this branch and return supported actions from the callback.
end
```

For a group, the shape is `options.<group>.<subcommand>.<option>`. The [complete router](../examples/lua/nested-router/module.lua) demonstrates both shapes. [#126](https://github.com/abanach-com/rust-discord-bot/issues/126) identifies shipped modules using a different, incompatible pattern.

## Event context

`ctx` contains `name`, optional `guild_id`, `channel_id` and `actor_id`, plus `data` and `config`. Event callbacks do not receive the command context's user name, permission bits, locale or option fields. The actual dispatched events are below; a syntactically accepted subscription name does not create a new host event.

### `message_create`

Eligible guild messages provide message/author/content metadata. Bot authors and messages excluded by the handler's filters are not dispatched. See the [complete payload and filters](lua/contexts-events.md#message_create).

### `message_update`

Only eligible updates with usable content and a verified non-bot author (`event.author` or cached `new.author`) reach Lua. Edits with an unconfirmed/missing author are dropped by the gateway handler (fail-closed) to protect against unverified actor execution and false moderation actions; dropped edits increment `message_updates_unknown_author_total`. See [the update payload](lua/contexts-events.md#message_update).

### `message_delete`

Deletion supplies identifiers, not the deleted message body or author. See [the deletion payload](lua/contexts-events.md#message_delete).

### `guild_member_add`

Join events have a guild and actor but no event channel. See [the member-add payload](lua/contexts-events.md#guild_member_add).

### `guild_member_remove`

Leave events likewise have no event channel. See [the member-remove payload](lua/contexts-events.md#guild_member_remove).

## Actions

The top-level types are `reply`, `send_message`, `send_direct_message`, `delete_message`, `timeout_member`, `kick_member`, `ban_member`, `add_role`, `remove_role`, `purge`, `music`, `progression`, `scheduler`, `ticket`, `moderation_case` and `audit`. The [action reference](lua/actions.md) documents every field and bound.

The engine decodes and validates the entire list before execution, with a maximum of 25 actions. Ordinary message content must contain 1–2,000 Unicode scalar values. Validation does not make the subsequent Discord/storage effects one transaction. For commands, the first `reply` is delivered before non-reply actions, even when it occurs later in the list. See [execution and partial effects](lua/security-errors.md#execution-order-and-partial-effects).

### Reply

[`reply`](lua/actions.md#reply) responds to the interaction; additional replies become follow-ups. In event context it becomes a channel message, so `ephemeral` cannot make it private.

### Send message

[`send_message`](lua/actions.md#send-message) targets a channel, subject to scope and destination checks. Review the known permission-resolution limitation in [#110](https://github.com/abanach-com/rust-discord-bot/issues/110).

### Send direct message

[`send_direct_message`](lua/actions.md#send-direct-message) asks Rust to deliver a direct message. Delivery can still fail because of the recipient's Discord settings.

### Delete message

[`delete_message`](lua/actions.md#delete-message) requires the documented channel/message identifiers and permission checks.

### Timeout member

[`timeout_member`](lua/actions.md#timeout-member), like other moderation actions, is subject to actor, application and hierarchy rules.

### Kick member

See [`kick_member`](lua/actions.md#kick-member) for fields and authorization.

### Ban member

See [`ban_member`](lua/actions.md#ban-member) for fields, deletion-window bounds and authorization.

### Add or remove role

See [`add_role` and `remove_role`](lua/actions.md) for role identifiers and hierarchy/managed-role restrictions.

### Purge

See [`purge`](lua/actions.md#purge) for count, age and permission constraints.

### Music

[`music`](lua/actions.md#music) uses a string `operation`. It depends on the host voice/media configuration and context.

### Audit

[`audit`](lua/actions.md#audit) accepts an event name and JSON-compatible map. Keep data small and omit secrets or unnecessary private content; the action validator does not impose a separate data size/depth bound.

### Moderation case

[`moderation_case`](lua/moderation-cases.md) uses a nested `operation` table and enforces case visibility, moderator and action-specific rules. It has no reserved module ID.

### Restricted host services

| Action | Required `manifest.id` | Additional context |
| --- | --- | --- |
| `progression` | `progression` | Guild and actor; staff adjustment also requires interactive authorization |
| `scheduler` | `scheduler` | Interactive command, guild and actor |
| `ticket` | `tickets` | Interactive command, guild, channel and actor |

These service actions use nested `operation` tables. Rust renders their results after Lua returns; a callback cannot synchronously query a service result. The [service guide](lua/services.md) explains these boundaries. [#129](https://github.com/abanach-com/rust-discord-bot/issues/129) documents invalid progression operations and module identities in two shipped modules.

## Built-in helper API

`rustybot` and `zuckerbot` reference the same table:

| Entry | Meaning |
| --- | --- |
| `api_version` | Integer `1` |
| `escape_mentions(text)` | Inserts U+200B after each `@` |
| `truncate(text, maximum)` | First N Unicode scalar values; no added ellipsis |
| `unix_time()` | Host wall-clock Unix time in seconds |
| `t(key, argument2, argument3)` | Translation lookup, locale selection and interpolation |
| `translate(...)` | Alias of the table's `t` function |

The host also exposes global `t`; there is no global `translate`. See [helpers and translations](lua/helpers-i18n.md) for complete argument rules, locale fallback and catalogue loading. Catalogues load once per process; command descriptions are localized, while command names are not automatically localized by this snapshot.

## Sandbox

The host removes `collectgarbage`, `coroutine`, `debug`, `dofile`, `io`, `load`, `loadfile`, `loadstring`, `os`, `package`, `print` and `require`, and limits VM resources. Default budgets are 8 MiB and 500,000 instructions with a hook every 1,000 instructions. These are not an elapsed-time deadline. See [security and errors](lua/security-errors.md) for exact environment settings and failure behavior.

There is no generic filesystem, HTTP, SQL, key-value storage, cross-module call, timer callback or persistent global state. Buttons, modals and autocomplete do not have a complete Lua callback/response path in this snapshot; an existing Rust helper or registration flag does not supply one. See [current support](lua/support-matrix.md#components-modals-and-autocomplete).

## Module development checklist

1. Read the [quickstart](lua/quickstart.md) and choose a unique module ID and command names.
2. Return the module wrapper, export callbacks and declare the commands/events they handle.
3. Validate package defaults and candidate guild configuration against the manifest schema.
4. Route actual nested options, retain IDs as strings and return dense action arrays.
5. Check the action/service permissions and context requirements before choosing an output.
6. Run the relevant [syntax, engine and integration checks](lua/testing.md), recording which layers actually ran.
7. Consult [known snapshot defects](lua/support-matrix.md#known-snapshot-defects-affecting-module-authors) before copying shipped patterns.

Primary implementation sources: [models](../src/lua/model.rs), [engine](../src/lua/engine.rs), [interaction/event handler](../src/discord/handler.rs) and [executor](../src/discord/executor.rs). The handbook describes their contract; it does not assert that the full Rust suite or live Discord integration passed during documentation preparation.
