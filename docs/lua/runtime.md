# 1. Runtime and lifecycle

[Handbook index](README.md) · Next: [Quickstart](quickstart.md)

## Responsibility boundary

A Lua package returns a module table containing a `manifest` table and its callback functions. The manifest describes slash commands, event subscriptions and a configuration schema. A callback receives a plain data table and returns a list of action requests. Rust validates the complete list, checks the execution context, calls Discord or a host service, and records audit information. Lua has no Discord client, database connection, HTTP client or filesystem handle.

The implementation is in [engine.rs](../../src/lua/engine.rs), [model.rs](../../src/lua/model.rs), [handler.rs](../../src/discord/handler.rs) and [executor.rs](../../src/discord/executor.rs). Keep these four stages separate when diagnosing a feature: manifest acceptance, Discord command registration, callback dispatch, and action execution.

## Package discovery

`SCRIPTS_DIR` selects the script directory. When that directory contains `install.xml`, the installer is the authoritative list of selected packages. Disabled entries are excluded from loading. For each enabled entry the loader tries these source locations, in order:

1. `<folder>/<entry>` from the entry's declared path.
2. `<folder>/module.lua`.
3. `<folder>/<id>.lua`.
4. `<SCRIPTS_DIR>/<id>.lua`.

The configuration filename is the installer entry's `config` value, with `config.json` as the fallback. If `install.xml` does not exist, the legacy loader recursively discovers `.lua` files within `SCRIPTS_DIR`. To maintain strict directory containment and prevent recursion loops, symlinks are strictly forbidden during legacy discovery, maximum traversal depth is capped at 16, maximum discovered scripts at 512, and cyclic or escaping paths are rejected at load time. This fallback can discover both a package and a historical standalone copy, producing duplicate module or command names. Use an installer for a predictable deployment.

The installer ID and returned manifest ID are strictly verified for equality (`installer_entry.id == manifest.id`). Any mismatch or duplicate `<module id>` in `install.xml` immediately rejects loading without altering the active registry. All package paths (`folder`, `entry`, `config`) must be relative without parent directory traversal (`..`), and are confined within the module package directory; symlinks or traversal components that escape the package root are rejected at load time. The manifest ID becomes the runtime identity used for configuration, command ownership and restricted host services. Installer `author` and `folder` fill absent manifest metadata; other installer metadata is not a substitute for the Lua manifest.

## Load-time work

Loading a package evaluates its source in a sandbox, extracts `module.manifest` from the returned table, validates it and compiles `config_schema`. Duplicate module IDs, duplicate slash command names, invalid manifests, an empty installed module set, and a command total above 100 reject the candidate module set. A source whose first byte is `0x1b` is rejected as bytecode.

Evaluation must return the module table, normally as the last statement. The engine reads `module.manifest` and later `module.on_command` or `module.on_event`; a bare manifest or a global callback alone does not satisfy this contract. Defining a callback does not execute it at load time. The loader does not prove that every declared command has a useful implementation, or that every subscribed event has a corresponding host dispatcher.

```lua
local manifest = {
    id = "sample", name = "Sample", version = "1.0.0",
    description = "A small example", category = "examples",
    commands = {
        { name = "sample", description = "Run the sample" }
    }
}

local function on_command(command, ctx)
    if command ~= "sample" then return {} end
    return {{ type = "reply", content = "Loaded", ephemeral = true }}
end

return { manifest = manifest, on_command = on_command }
```

## A fresh VM for every invocation

Each command or event callback runs in a newly created Lua VM inside Rust's blocking execution pool. Rust registers the sandbox helpers, evaluates the module source again, finds the callback and supplies its context. The instruction and memory budgets apply to this work, including top-level code.

Consequently, local variables, global variables, caches and tables from an earlier invocation do not survive. For example, a callback exported from a module with this counter returns `1` on every invocation:

```lua
local count = 0
local function on_command(command, ctx)
    count = count + 1
    return {{ type = "reply", content = tostring(count) }}
end
-- Export on_command in the returned module table, alongside its manifest.
```

Top-level code should construct definitions, constants and the manifest. Do not attempt startup jobs, long-running loops, background coroutines, `require`, or per-guild caches there. There is no `on_load`, `on_ready`, `on_shutdown`, timer callback or continuous Lua process. Persistent business state belongs to the [host services](services.md).

## Configuration and enablement

There are three distinct decisions:

| Layer | Effect |
| --- | --- |
| Installer `enabled=false` | Package is not part of the loaded module set at all |
| Manifest `default_enabled` | Default runtime enablement when a guild has no saved setting |
| Saved guild module setting | Overrides runtime enablement and configuration for that guild |

A saved guild configuration **replaces the whole default configuration value**; the engine does not deep-merge missing fields from `config.json`. Command and event callbacks receive the effective configuration as `ctx.config`. With no guild-specific setting, the package default applies; DM command handling uses the available non-guild default path.

The loader compiles the schema but does not validate the shipped default JSON against it. Missing, unreadable or malformed configuration falls back to `{}`. Dashboard configuration writes call the schema validator. Thus a package can load successfully with defaults that a subsequent dashboard save would reject. Validate your example defaults explicitly and write defensive fallback logic where reasonable. See [manifest configuration](manifests.md#configuration-schema) and [current limitations](support-matrix.md).

## Callback result and execution phases

Both callbacks return one dense, one-based array of action tables. `{}` is a valid empty result. Always wrap an action in an array, for example `return {{ type = "reply", content = "Hello" }}`. A single keyed action table is not the action-list contract; do not rely on it being rejected or executed. Unsupported action types and incompatible field types in the returned sequence fail conversion.

Rust deserializes and validates the entire result before executing actions. The maximum is 25 actions. A validation error anywhere prevents the entire batch from starting. This preflight is not a transaction over the later Discord and database calls.

For commands, the executor selects the first `reply` action and sends it before all non-reply actions, even if it appeared later in the returned array. With no `reply`, Rust sends a default acknowledgement. Remaining actions preserve their order, skipping the reply already sent. Additional replies become follow-up messages. An execution failure stops the remaining command actions, and already completed effects remain.

For events, a `reply` is an ordinary message to the event channel. A missing channel makes that request fail. Event execution logs a failed action and continues with later actions. There is no interaction visibility setting for an event reply. See [actions](actions.md) for deferred replies, chunking and permission rules.

## Reloading

The engine has a reload operation that constructs a new candidate set and swaps it in only on success. This is an internal capability, not a documented Lua function. No Lua action or public HTTP reload endpoint is provided by this contract. Restarting the bot is the reliable operator workflow after editing source or installation files. Command registration must also run against the intended Discord application and scope.

## JSON and Lua data rules

JSON objects become keyed tables; arrays become one-based sequences. JSON null becomes Lua `nil`, so a nullable object field can be absent. Booleans stay booleans. Integer values that fit the host conversion become Lua integers; other numeric values can be floating point. IDs remain strings throughout these interfaces, including IDs nested in event payloads and command options.

Do not use `tonumber` to compare or rewrite Discord snowflakes. Copy the string from the context or configuration into an action. Lua's length operator counts string bytes, while most Rust text limits count Unicode scalar values. Use `rustybot.truncate` for a character-based limit, and leave headroom for text added afterward.
