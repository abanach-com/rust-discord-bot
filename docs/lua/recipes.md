# 14. Recipes

[Handbook index](README.md) · Previous: [Test and troubleshoot](testing.md) · Next: [Support and limitations](support-matrix.md)

Start with a complete package from the [example collection](../../examples/lua/README.md).
Each complete source returns a module table with `manifest` and its exported callbacks. The inline
Lua blocks below are **callback fragments**, not standalone module files; use [Quickstart](quickstart.md) for the wrapper and installer.

| Complete example | What it demonstrates |
| --- | --- |
| [hello/module.lua](../../examples/lua/hello/module.lua) | Optional string/user options, string IDs and a private reply |
| [nested-router/module.lua](../../examples/lua/nested-router/module.lua) | Subcommands and a subcommand group using the actual nested option shape |
| [event-audit/module.lua](../../examples/lua/event-audit/module.lua) | Declared events, an opt-in configuration flag and small audit records |

## 1. Reply with bounded, nonempty text

Use this fragment inside `on_command` for a command with a declared string option named `text`.
It returns an action array and chooses a fallback before applying the output bound:

```lua
-- Callback fragment: inside on_command(command, ctx).
local text = (ctx.options or {}).text
if type(text) ~= "string" or not text:match("%S") then
    text = "No text supplied."
end
return {{
    type = "reply", ephemeral = true,
    content = rustybot.truncate(rustybot.escape_mentions(text), 1800),
}}
```

`truncate` counts Unicode scalar values; `#text` counts bytes. Escaping mentions does not escape
Markdown. See [helpers](helpers-i18n.md) and [reply delivery](actions.md#reply).
For an external effect, use a neutral acknowledgement or let the host service report its result:
[the first reply precedes other actions](security-errors.md#execution-order-and-partial-effects).

## 2. Route nested options and preserve explicit false

The complete router receives `/lua-route echo` at `ctx.options.echo`; its `inspect user` branch is
`ctx.options.inspect.user`. There is no `ctx.options.subcommand` discriminator.
To try the fragment below, extend that module's `echo.options` with an optional Boolean option
named `private` (`type = "boolean"`), after the required `text` option. Update the declaration as well as the callback.

```lua
-- Callback fragment: the echo branch of the matching on_command handler.
local echo = (ctx.options or {}).echo
if not echo then return {} end
local private = echo.private
if private == nil then private = true end
return {{
    type = "reply", ephemeral = private,
    content = rustybot.truncate(rustybot.escape_mentions(echo.text), 1800),
}}
```

Checking `nil` separately preserves an explicitly selected `false`. Apply the same rule to Boolean
configuration defaults. Keep user IDs from nested options as strings, without `tonumber`.
See [nested contexts](contexts-events.md#nested-options-are-the-routing-contract) and [manifest options](manifests.md).
A response already deferred privately stays private; follow-up visibility has its own rules in
[defer and privacy](security-errors.md#defer-privacy-and-the-known-ack-race).

## 3. Audit event metadata without copying message contents

The complete event example defaults to `record_events = false`. This fragment belongs in its
`on_event` callback, with `message_create` declared in `manifest.events`:

```lua
-- Callback fragment: inside on_event(event, ctx).
if event ~= "message_create" or (ctx.config or {}).record_events ~= true then
    return {}
end
local data = ctx.data or {}
return {{
    type = "audit", event = "recipe_message_seen",
    data = { message_id = data.message_id, channel_id = ctx.channel_id },
}}
```

Enable the flag through the module configuration when auditing is wanted. Keep records small and
exclude message bodies, tokens and whole-context dumps. Other events can lack an actor or channel;
use their actual [event contracts](contexts-events.md#event-reference). An event `reply` is a channel
message, so `ephemeral = true` cannot make it private. See [audit actions](actions.md#audit).

## 4. Request a warning case and let Rust report the outcome

A custom module may use `moderation_case`; this service has no reserved module ID. Declare a guild-only
command with required `member` (`user`) and `reason` (`string`, length 1–1000) options.
`default_member_permissions = "8192"` selects Manage Messages for command availability; Rust still
checks moderator authority and, for an interactive warning, target hierarchy. See [case permissions](moderation-cases.md#access-and-side-effects).

```lua
-- Callback fragment: inside on_command for that declared warning command.
local options = ctx.options or {}
if type(options.member) ~= "string" or type(options.reason) ~= "string"
    or not options.reason:match("%S") then
    return {{ type = "reply", content = "Supply a member and a reason.", ephemeral = true }}
end
return {{
    type = "moderation_case",
    operation = {
        type = "create", subject_user_id = options.member,
        kind = "warning", reason = options.reason, severity = "low",
        points = 1, visible_to_subject = true,
    },
}}
```

This requests a visible warning record, not a ban or timeout. It sends no advance success claim;
Rust provides the initial acknowledgement and the service result or error. Lua cannot capture the
new case number within that callback. Full fields and visibility rules are in [moderation cases](moderation-cases.md).

## 5. Extend the existing trusted service module

Choose the existing module before copying an operation fragment:

| Goal | Required `manifest.id` | Contract |
| --- | --- | --- |
| Profile, daily reward, transfer or staff adjustment | `progression` | [Progression](progression.md) |
| Reminder or recurring delivery | `scheduler` | [Scheduler](scheduler.md); supply all required policy limits |
| Ticket opening or management | `tickets` | [Tickets](tickets.md); opening requires its nested `policy` |

Extend the reviewed module and its declarations. A second module with the same ID collides at load
time; a different ID cannot call that service merely by returning the action. No manifest capability
field changes this rule. See [service access restrictions](services.md#service-access-restrictions).
For scheduler recurrence, write `["repeat"] = "1d"` in the operation table: `repeat` is a Lua keyword.
Use the documented progression operation `transfer`, not `transfer_coins` or another invented alias.

## 6. Use supported output and verify the complete package

There is no implemented Lua embed action or supported embed field on `reply`. Use bounded text via
[reply or send_message](actions.md), or extend the host model, validation and executor before exposing
an additional output type. Buttons, modals and autocomplete likewise require the host paths described
in the [support matrix](support-matrix.md); adding an arbitrary Lua callback does not implement them.

Before installation, ensure the complete file returns `{ manifest = manifest, on_command = on_command }`
or the corresponding `on_event` export, its configuration matches its schema, and command names/IDs are unique.
Use [the testing chapter](testing.md) for syntax, engine and integration checks; Lua-only execution cannot prove Discord delivery or persistence.
