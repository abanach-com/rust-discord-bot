# 6. State and host services

[Handbook index](README.md) · Previous: [Actions](actions.md) · Next: [Progression](progression.md)

## The persistence boundary

Every invocation starts a fresh Lua VM. A table, counter, cache, or global assigned during one callback disappears when that invocation ends. Lua cannot read files, open PostgreSQL, perform HTTP requests, or keep a coroutine running. There is no `bot`, `storage`, `db`, `state.get`, or `state.set` API.

Rust owns the [storage layer](../../src/storage) and the service implementations under [src/discord](../../src/discord). With `DATABASE_URL`, the bot uses its PostgreSQL storage implementation and migrations. Without it, the development fallback stores data in memory; a process restart loses that state. Treat that fallback as a development mode when testing durable features.

| Information | Where it lives | What Lua receives or requests |
| --- | --- | --- |
| Module source and manifest | Files selected by installer; loaded source snapshot | Source is evaluated again for each invocation |
| Package default configuration | Package `config.json` | Effective `ctx.config` |
| Guild module configuration and enablement | Host storage | Effective `ctx.config`; no general configuration mutation action |
| XP, coins, reputation and daily streaks | Progression service/storage | Progression action requests; Rust renders results |
| Scheduled deliveries | Scheduler service/storage | Scheduler action requests; worker runs independently of Lua |
| Tickets, participants and transcripts | Ticket service/storage | Ticket action requests; Rust manages channels and records |
| Moderation cases, notes and evidence | Case service/storage | Case action requests; Rust applies visibility and staff rules |
| Audit records | Host audit infrastructure/storage | `audit` action with JSON-object data; keep payloads deliberately small |

Configuration is an input to behavior, not a writable per-member database. A callback cannot return a new `ctx.config` to persist it. Modifying the input table changes only that invocation's copy.

## Service access restrictions

A service action has an outer action `type` and a nested `operation.type`:

```lua
{ type = "progression", operation = { type = "balance" } }
```

Acceptance by the Lua deserializer does not grant access to the service. The executors enforce these identities and contexts:

| Outer action | Required runtime module ID | Context requirement | Further checks |
| --- | --- | --- | --- |
| `progression` | Exactly `progression` | Guild and actor | `adjust` additionally needs an interactive command and `MANAGE_GUILD`/administrator |
| `scheduler` | Exactly `scheduler` | Interactive command, guild and actor | Staff permission for recurring jobs, explicit destination and cross-owner administration |
| `ticket` | Exactly `tickets` | Interactive command, guild, channel and actor | Staff/creator/participant rules and policy checks per operation |
| `moderation_case` | No reserved module ID | Guild and actor | Moderator permission, subject visibility, and action-specific permissions |
| `audit` | No reserved module ID | Available callback context | Event name and data shape validation |

The identity used here is `manifest.id`, not the package folder, XML display name, command name, or an action field. No manifest capability list overrides these restrictions. A custom module called `my_economy` cannot return `progression` successfully. Giving untrusted source a reserved ID would give that source the corresponding trust; the operator must control installed code and prevent duplicate IDs.

Service examples in the following chapters are **operation fragments for an authorized existing module**. They are not standalone packages for arbitrary IDs. The complete [handbook examples](../../examples/lua/README.md) use unrestricted core actions and do not claim reserved service access.

## Data flows in one direction

Callbacks compute an action list synchronously. Rust executes it after Lua returns. A profile, balance, list, ticket info, or case view operation therefore cannot supply data back to that Lua callback:

```lua
-- This is an action request to include in the returned list.
local request = { type = "progression", operation = { type = "profile" } }
return { request }
```

There is no synchronous `rustybot.get_profile()` equivalent. Rust builds the response text or other service result. For interactive commands, a service's textual result is sent as a follow-up after the initial reply/acknowledgement. An event does not have an interaction response; returned service status text is not a Lua value or an automatic event-channel reply.

If a feature needs a query result to make a later Lua decision, the current API must be extended in Rust. Do not simulate that query using globals or infer a balance from a previously displayed response.

## Consistency and failures

The engine validates every action before execution, but the list is not one transaction across Discord and PostgreSQL. A successful role change followed by a failed case write, for example, can leave the role change in place. A command stops executing its remaining actions after a failure; an event logs an action failure and continues. Service-specific transactions protect their own database operations where implemented, not an arbitrary multi-action script.

Prefer a single service operation for a business operation that the service already owns. For example, `transfer` uses the service's balance rules; trying to represent it as two separate `adjust` requests would have different authorization and failure behavior.

## Further reading

The field contracts are in [progression](progression.md), [scheduler](scheduler.md), [tickets](tickets.md), and [moderation cases](moderation-cases.md). Executor checks are in [progression.rs](../../src/discord/progression.rs), [scheduler.rs](../../src/discord/scheduler.rs), [tickets.rs](../../src/discord/tickets.rs), and [moderation_cases.rs](../../src/discord/moderation_cases.rs).
