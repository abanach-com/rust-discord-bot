# 2. Build and install a module

[Handbook index](README.md) · Previous: [Runtime](runtime.md) · Next: [Manifests](manifests.md)

The complete, runnable packages are in [examples/lua](../../examples/lua/README.md). This chapter builds the `hello` package. Its behavior is deliberately small: a slash command accepts optional text and an optional user, then returns an ephemeral response.

## Files to create

Create a package directory containing `module.lua` and `config.json`, and select it from `install.xml` in the script root. The example script root contains:

- [install.xml](../../examples/lua/install.xml)
- [hello/module.lua](../../examples/lua/hello/module.lua)
- [hello/config.json](../../examples/lua/hello/config.json)

The production installer is [scripts/install.xml](../../scripts/install.xml). The examples do not change that file.

## Complete source

```lua
local manifest = {
    id = "lua_hello",
    name = "Lua Hello",
    version = "1.0.0",
    description = "A complete first Lua module",
    category = "examples",
    default_enabled = true,
    commands = {
        {
            name = "lua-hello",
            description = "Try the Lua example",
            integration_types = { "guild" },
            contexts = { "guild" },
            options = {
                {
                    type = "string", name = "text",
                    description = "Greeting text", required = false,
                    min_length = 1, max_length = 100
                },
                {
                    type = "user", name = "person",
                    description = "Optional person to identify", required = false
                }
            }
        }
    },
    config_schema = {
        type = "object",
        additionalProperties = false,
        properties = {
            greeting = { type = "string", minLength = 1, maxLength = 80 }
        },
        required = { "greeting" }
    }
}

local function on_command(command, ctx)
    if command ~= "lua-hello" then return {} end
    local options = ctx.options or {}
    local config = ctx.config or {}
    local text = options.text or config.greeting or "Hello from Lua"
    local name = ctx.user_name or "friend"
    local content = rustybot.escape_mentions(text)
        .. ", " .. rustybot.escape_mentions(name) .. "!"
    if options.person then
        content = content .. " Selected user ID: " .. options.person
    end
    return {{ type = "reply", content = content, ephemeral = true }}
end

return { manifest = manifest, on_command = on_command }
```

This is the same API pattern as the linked package; the checked-in package is the executable source of truth for the example. `rustybot.escape_mentions` protects the rendered text from mention-like syntax. Rust also disables uncontrolled Discord mentions on ordinary message actions.

## Default JSON

```json
{
  "greeting": "Hello from Lua"
}
```

`config_schema` is JSON Schema expressed as a Lua table. Its property names use JSON Schema casing (`minLength`, `additionalProperties`); command options use their own snake-case API (`min_length`). The schema requires `greeting`, allows one through 80 characters and rejects unknown keys. The supplied JSON satisfies those rules.

The loader does not validate default JSON for you. A dashboard's saved guild configuration replaces defaults; it must contain required fields. Do not rely on a schema's `default` annotation to insert missing values.

## Installer

A minimal installer selecting only this package is:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<installer version="1.0.0">
  <bot name="Lua handbook example" />
  <modules>
    <module id="lua_hello" enabled="true">
      <name>Lua Hello</name>
      <version>1.0.0</version>
      <folder>hello</folder>
      <entry>module.lua</entry>
      <config>config.json</config>
    </module>
  </modules>
</installer>
```

Use the [checked-in example installer](../../examples/lua/install.xml) to load all three documentation packages. Keep the installer ID identical to the manifest ID. An installer selects source; it does not grant Discord permissions or replace the manifest returned from the source.

## Run in a development guild

Use the bot's normal `.env` setup described in the [root README](../../README.md), including the Discord token, and set `DISCORD_DEV_GUILD_ID` to your test guild. Then run from the repository root:

```bash
SCRIPTS_DIR=examples/lua cargo run --release
```

This uses the alternative example installer. Command registration can replace the registered application command set in the selected scope, so point it at the intended development application/guild. Global registration has different propagation and context behavior from guild-scoped registration.

Run `/lua-hello`, then `/lua-hello text:Welcome`, then provide the optional `person`. The selected person arrives as a decimal string ID, not as an object with a username. A private reply is expected. Check startup logs if the command does not appear, and the invocation logs if Discord reports an execution error.

## Add behavior incrementally

For regular options, read `ctx.options.<name>`. For subcommands, read the nested selected branch, for example `ctx.options.echo.text`. Use [nested-router](../../examples/lua/nested-router/module.lua) as the complete routing example. There is no `ctx.options.subcommand` discriminator.

For events, declare an event name and implement `on_event(event, ctx)`. The [event-audit](../../examples/lua/event-audit/module.lua) example subscribes to three real events and records limited metadata only when its configuration enables the behavior. It needs no message channel to write an `audit` action.

To save balances, cases, tickets or jobs, read the relevant host-service chapter before adding an action. Some service actions have a reserved module ID requirement; a new arbitrary module cannot call every operation merely by returning its shape.

## What this exercise verifies

Reading or running a Lua fixture can verify manifest construction, callback branches and action-table contents. It does not verify Rust deserialization, schema compilation, permission enforcement, gateway intents, Discord hierarchy, database persistence or live message delivery. Use the [testing chapter](testing.md) for those layers and record exactly which checks ran.
