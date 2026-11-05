# 12. Security and errors

[Handbook index](README.md) · Previous: [Helpers and translations](helpers-i18n.md) · Next: [Test and troubleshoot](testing.md)

## Scope and trust

Lua computes action requests from data. Rust owns Discord clients, storage, network access,
validation and execution. Installing a module is an operator trust decision: ordinary core actions
are available to installed code, while some services require an exact runtime module ID.
The sandbox does not make arbitrary third-party source safe to install without review.

There is no manifest capability allowlist that grants HTTP, SQL, filesystem or process access.
Keep credentials out of module configuration and output. A configuration value passed to Lua is
readable by that module; the sandbox does not make such application data secret from its author.
See [the architecture](../ARCHITECTURE.md) and [host services](services.md).

## Sandbox globals and helpers

[sandboxed_lua](../../src/lua/engine.rs#L656-L775) creates a fresh VM with `StdLib::ALL_SAFE`,
sets its memory limit, and explicitly replaces **all twelve** of these globals with `nil`:

```text
collectgarbage  coroutine  debug  dofile  io  load
loadfile       os         package  pcall  require  xpcall
```

Strings, numbers, tables and ordinary computation remain available. Module loading, dynamic chunk
loading, shell commands and direct file access are unavailable through those removed globals.
The host exposes no arbitrary HTTP client, database client, Discord client object or generic
`storage.get`/`storage.set` interface. Persistence is requested through typed host actions.

The registered helper table contains `api_version`, `escape_mentions`, `truncate`, `unix_time`,
`t` and `translate`. `rustybot` and `zuckerbot` name that table; global `t` aliases its translator.
These helpers compute values, including the current time; they do not execute database or Discord actions.
Their signatures are documented in [helpers and translations](helpers-i18n.md).

`pcall` and `xpcall` cannot catch an instruction-budget error because those functions are absent.
Check optional inputs with ordinary conditions and return a useful reply for expected user mistakes.
Unexpected exceptions propagate to Rust. Source beginning with byte `0x1b` is rejected as bytecode
at [load time and invocation](../../src/lua/engine.rs#L510-L516); distribute textual Lua source.

## Resource budgets

These are host environment settings, not fields that a Lua manifest can override:

| Setting | Default | Configuration validation |
| --- | --- | --- |
| `LUA_MEMORY_LIMIT_BYTES` | 8,388,608 bytes (8 MiB) | `usize`, at least 262,144 bytes |
| `LUA_INSTRUCTION_LIMIT` | 500,000 | Positive `i64` |
| `LUA_HOOK_GRANULARITY` | 1,000 | Positive `u32`; instructions between hook checks |
| `LUA_MAX_PARALLEL` | 32 | Positive `usize`; global pool concurrency limit |
| `LUA_MAX_PER_GUILD` | 8 | Positive `usize`, `<= LUA_MAX_PARALLEL`; per-guild concurrency limit |
| `LUA_COMMAND_TIMEOUT_MS` | 3,000 ms | Positive `u64`; command admission timeout before returning busy |
| Returned action count | 25 | Fixed engine maximum, after deserialization |

Defaults and accepted configuration are in [config.rs](../../src/config.rs);
[state construction](../../src/state.rs) passes them into the engine.
Admission control limits both global and per-guild concurrent VM executions before task scheduling,
preventing cross-guild starvation and unbounded worker thread pool saturation.
The global instruction hook subtracts the configured granularity on each check and raises
`Lua instruction limit exceeded` when the budget is exhausted. Enforcement happens at hook intervals,
not at an exact wall-clock deadline. Configuration does not require granularity to be below the budget.

Module evaluation and callback execution share one invocation's VM budget. Load-time evaluation has
its own VM. A large top-level initializer consumes resources before the callback starts.
The memory limit covers Lua VM allocations, not total Rust process memory or subsequent HTTP requests.
Invocations use Rust's blocking pool, not a separate sandbox process; no Lua wall-clock timeout is set
in [execute_command / execute_event](../../src/lua/engine.rs#L222-L292).

## Failure stages

Distinguish these stages when reading logs. Successful completion of one stage does not establish
that a later stage succeeded:

| Stage | Examples of failure | What has happened to the returned actions? |
| --- | --- | --- |
| Discovery and module loading | Invalid XML, unreadable source, bytecode marker, Lua syntax error, duplicate IDs, invalid manifest or schema | No callback action list exists |
| Dispatch and callback lookup | Unknown module/command/event, disabled module, missing callback | No list has been executed |
| Lua execution | Indexing `nil`, failed helper conversion, memory or instruction budget | Callback fails before action execution |
| Serde conversion | Wrong result shape, missing required field, unknown action/operation tag, wrong field type | The whole returned list is rejected |
| Action validation | More than 25 actions, invalid ID, text or numeric bound violation | No returned action starts, even if earlier entries were valid |
| Host execution | Permission, hierarchy, service policy, rate limit, Discord or database error | Earlier successful effects may already exist |

[LuaEngineError](../../src/lua/engine.rs#L65-L100) carries script/module context.
Callback and Serde errors are wrapped as runtime errors; explicit action validation errors use
`InvalidModule`. [execute_handler](../../src/lua/engine.rs#L579-L653) validates the entire list.
An interaction ACK can already have been sent by the separate defer path when Lua or validation fails.

Use the field names and enum tags in [the action model](../../src/lua/model.rs#L223-L450).
Always return a dense action array rather than one bare action object; do not rely on a malformed
table producing an error, and test the actual Rust decoding. Returning 26 otherwise valid actions
is an engine count failure. These are different from a valid
`progression` action rejected later because its module ID is not `progression`.

## Execution order and partial effects

The [command executor](../../src/discord/executor.rs#L151-L495) selects the first `reply` and sends it
before all non-reply actions, even if that reply was last in the array. Without a reply it sends a
private default acknowledgement. It then processes the remaining actions in order; extra replies
become follow-ups. A success-looking initial reply is therefore not proof that later effects succeeded.

An execution failure stops the remaining command actions. Earlier Discord changes or storage writes
are not rolled back as a batch. The [event executor](../../src/discord/executor.rs#L507-L835) logs
individual failures and continues, with special suppression of certain success-audit events after
failure. A service may protect its own database transaction; there is no transaction spanning an
arbitrary Lua action list, Discord and PostgreSQL, and no general exactly-once retry guarantee.

## Defer, privacy and the known ACK race

[handle_command](../../src/discord/handler.rs#L989-L1147) starts a timer of 1,500 milliseconds.
When its path sends a defer, it uses `ephemeral(true)`. Once that ACK has completed, the executor
edits the original response. Setting `ephemeral = false` in a later reply does not turn that original
private response public. Additional chunks and replies use their own requested follow-up visibility;
set `ephemeral = true` explicitly for confidential reply content. Service status/error follow-ups
from the command action loop are private.

**Known limitation, re-audit of `10a2ba1780a2becd6d7ba2bfc5c4f7a2fecc9882`:**
[#121](https://github.com/abanach-com/rust-discord-bot/issues/121) remains relevant while defer HTTP
is in flight. `deferred` becomes true only after that HTTP call completes; the main path can still
observe false and send a second initial response. Unconditional task cancellation and the two flags
can then leave an acknowledged interaction without a final result. Do not treat defer as a proven
single-response protocol in this snapshot. This is a host defect; catching errors in Lua cannot fix it.

## Actor, application and event permissions

[Command ActionOrigin](../../src/discord/executor.rs#L339-L373) carries the member's interaction
permissions, application permissions and roles, with `enforce_actor_permissions = true`.
For the core privileged actions, [required_permission and execution checks](../../src/discord/executor.rs#L885-L963)
check actor and application bits; member and role actions also run their relevant hierarchy checks.
`default_member_permissions` is used for command registration, not a generic host authorization check
for every returned action. Service-specific rules are additional gates.

[Event ActionOrigin](../../src/discord/executor.rs#L507-L541) has empty actor permissions,
`app_permissions = Permissions::all()` and `enforce_actor_permissions = false`.
This bypasses the common interactive permission/rate-limit gate, not Discord's real bot permissions.
It also does not grant a service's moderator predicate: moderation-case mutations still require it.
An event can lack an actor or channel; operations requiring those values reject the missing context.

**Known limitation:** [#110](https://github.com/abanach-com/rust-discord-bot/issues/110) remains relevant
in this snapshot. The [target-channel resolver](../../src/discord/executor.rs#L1891-L1997) applies target
role/member overwrites to source-channel effective permissions instead of rebuilding guild-role
permissions. A source-only allow can therefore survive in an inheriting target channel.
Keep target selection constrained; this documentation does not claim complete destination authorization.

## Restricted service identities

| Action | Required runtime module ID | Additional boundary |
| --- | --- | --- |
| `progression` | `progression` | Guild and actor; `adjust` also needs an interactive command and Manage Guild/administrator |
| `scheduler` | `scheduler` | Interactive command, guild and actor; operation-specific ownership/staff checks |
| `ticket` | `tickets` | Interactive command, guild, channel and actor; ticket policy and access checks |
| `moderation_case` | No reserved ID | Guild and actor; moderator/action-specific permissions and subject visibility rules |

These checks are in [progression](../../src/discord/progression.rs#L44-L72),
[scheduler](../../src/discord/scheduler.rs#L87-L145), [tickets](../../src/discord/tickets.rs#L58-L84)
and [moderation cases](../../src/discord/moderation_cases.rs#L47-L84). The identity is `manifest.id`;
changing a command name or returning an action field does not supply it. Reserved IDs belong to
reviewed service modules. See [host services](services.md) for operation-specific contracts.

## Safe output and configuration

Core reply, channel-message and direct-message builders disable automatic mentions with empty
`allowed_mentions`. Some host service messages deliberately permit one user, such as the scheduler's
optional creator mention or ticket notifications. An event reply is a channel message, not a private
interaction response; its `ephemeral` field cannot provide privacy. See [action delivery](actions.md).

`escape_mentions` inserts a zero-width space after each `@`; it does not escape Markdown or HTML.
`truncate` counts Unicode scalar values. Message validation allows 1–2,000 scalar values; IDs must
remain nonzero decimal strings. Audit `data` is a JSON-compatible map, with no separate size/depth cap
in `LuaAction::validate`; keep it small and exclude secrets, tokens and unnecessary private content.

The engine compiles `config_schema`, but package defaults are not validated against it at load time.
[Dashboard configuration writes](../../src/web/routes.rs#L340-L360) separately enforce schema validation
and a 64 KiB serialized size limit. Saved guild configuration replaces the defaults as a whole object.
Validate example defaults and guard optional values; see [runtime configuration](runtime.md#configuration-and-enablement).
