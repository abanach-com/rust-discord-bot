# Complete Lua handbook examples

These examples use the source contract documented in the [Lua handbook](../../docs/lua/README.md). They are not included in the production `scripts/install.xml`.

| Package | Callback | Test input | Expected request |
| --- | --- | --- | --- |
| [hello](hello/module.lua) | `on_command("lua-hello", ctx)` | `options = { text = "Welcome", person = "100000000000000004" }` | One ephemeral reply showing the greeting and string ID |
| [nested-router](nested-router/module.lua) | `on_command("lua-route", ctx)` | `options = { echo = { text = "Hello" } }` | One ephemeral reply `Hello` |
| [nested-router](nested-router/module.lua) | `on_command("lua-route", ctx)` | `options = { inspect = { user = { person = "100000000000000004" } } }` | One ephemeral reply showing the ID |
| [event-audit](event-audit/module.lua) | `on_event("message_create", ctx)` | `config = { record_events = false }` | Empty action list |
| [event-audit](event-audit/module.lua) | `on_event("message_create", ctx)` | `config = { record_events = true }`, `data.message_id` and `ctx.channel_id` set | One audit action with metadata |
| [event-audit](event-audit/module.lua) | `on_event("guild_member_add", ctx)` | `config = { record_events = true }`, `ctx.actor_id` set | One member-join audit action |

## Load all examples in a development bot

Use the normal project environment configuration and a development Discord application. From the repository root:

```sh
SCRIPTS_DIR=examples/lua cargo run --release
```

Set `DISCORD_DEV_GUILD_ID` in the environment to register in a test guild. Registration uses the selected module set and can replace that application's existing registered commands for the target scope. This installer selects the example commands rather than the normal production modules.

The event example defaults to `record_events: false`. Enable it through the module configuration if you want to test event auditing. Existing stored guild configuration replaces the package default configuration; editing the JSON file alone may therefore have no effect on a guild with saved settings.

## Add one example to an existing scripts directory

Copy its folder beneath your configured `SCRIPTS_DIR`, add a matching entry to that directory's installer, and restart the development bot. The Lua `manifest.id` and installer `id` should match. Keep command names unique. The complete installer in this directory is a selection example, not an append-only patch for the production installer.

## Offline fixture contract

A minimal command fixture is:

```lua
local ctx = {
    guild_id = "100000000000000001",
    channel_id = "100000000000000002",
    user_id = "100000000000000003",
    user_name = "Tester",
    member_roles = {},
    member_permissions = "0",
    locale = "en",
    options = {},
    config = { greeting = "Hello from Lua" },
}
```

The example `hello` module requires the host `rustybot.escape_mentions`. The router uses the same helper. The event example uses no host function. A native Lua test harness must provide this helper or use the actual `LuaEngine`. Every invocation must evaluate the module again to reflect the production fresh-VM lifecycle. Unknown command/event names return `{}`; production dispatch also checks declarations before invoking callbacks.

Lua-only execution checks syntax and returned values. Rust engine tests additionally check Serde decoding, action validation, module selection and instruction/memory limits. Live Discord tests are needed for actual command registration and message delivery. See [testing](../../docs/lua/testing.md).
