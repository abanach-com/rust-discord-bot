# 5. Action reference

[Handbook index](README.md) · Previous: [Contexts](contexts-events.md) · Next: [State and services](services.md)

Sources: [action models](../../src/lua/model.rs), [validation](../../src/lua/engine.rs), [executor](../../src/discord/executor.rs), [music](../../src/discord/music.rs). Specialized operation models are linked from their chapters.

## Return format and shared validation

```lua
return {
    { type = "reply", content = "Request received.", ephemeral = true },
    { type = "audit", event = "example_requested", data = { command = command } }
}
```

Return a dense one-based array with at most **25** actions. `{}` is valid. Every action requires a recognized `type`. All actions are converted and validated before execution begins; one invalid action rejects the whole list. Do not depend on unknown extra fields having an effect.

A snowflake field is a decimal string that parses as a positive `u64`. Ordinary message content must contain **1–2,000 characters**. Optional reasons on basic moderation actions are at most **512 characters**; specialized services have different reason rules. Numeric fields must fit their Rust integer type as well as the documented semantic range.

## All top-level action types

| Type | Required fields beyond `type` | Optional fields and defaults | Reference |
| --- | --- | --- | --- |
| `reply` | `content` | `ephemeral=false` | [Reply](#reply) |
| `send_message` | `content` | `channel_id` defaults to current channel | [Channel messages](#send-message) |
| `send_direct_message` | `user_id`, `content` | None | [Direct messages](#send-direct-message) |
| `delete_message` | `channel_id`, `message_id` | None | [Delete](#delete-message) |
| `timeout_member` | `user_id`, `seconds` | `reason` absent | [Timeout](#timeout-member) |
| `kick_member` | `user_id` | `reason` absent | [Kick](#kick-member) |
| `ban_member` | `user_id` | `delete_message_days=0`, `reason` absent | [Ban](#ban-member) |
| `add_role` | `user_id`, `role_id` | `reason` absent | [Roles](#add-or-remove-role) |
| `remove_role` | `user_id`, `role_id` | `reason` absent | [Roles](#add-or-remove-role) |
| `purge` | `amount` | `channel_id` defaults to current channel | [Purge](#purge) |
| `music` | `operation` string | `query` conditionally required | [Music](#music) |
| `progression` | `operation` table | Fields depend on operation | [Progression](progression.md) |
| `scheduler` | `operation` table | Fields depend on operation | [Scheduler](scheduler.md) |
| `ticket` | `operation` table | Fields depend on operation | [Tickets](tickets.md) |
| `moderation_case` | `operation` table | Fields depend on operation | [Moderation cases](moderation-cases.md) |
| `audit` | `event` | `data={}` object | [Audit](#audit) |

This is the complete currently modeled action set. There is no action for embeds, buttons, select menus, modals, autocomplete results, arbitrary HTTP/SQL, generic key-value storage or invoking another Lua module.

## Reply

```lua
{ type = "reply", content = "Only you can see this.", ephemeral = true }
```

The first reply in a command's action list is sent before any other action. Additional replies become follow-ups. If no reply exists, Rust supplies its normal acknowledgement. Therefore a response such as “Member banned” in the first reply can appear before the ban is attempted; prefer “Processing the request” when subsequent work can fail, or rely on the service-generated result.

The handler defers a command after approximately 1,500 ms, with an **ephemeral** defer. An initial reply sent after that defer inherits the already chosen visibility; setting `ephemeral=false` cannot turn that deferred original response public. An immediate first reply uses its declared visibility. Avoid promising unconditional public initial replies for slow callbacks.

On an event, `reply` sends an ordinary message in the event channel. `ephemeral` has no visibility effect, and a missing event channel makes the action invalid at execution time.

## Send message

```lua
{ type = "send_message", content = "Posted in the invoking channel." }
{ type = "send_message", channel_id = "100000000000000002", content = "Explicit destination." }
```

Omitting `channel_id` uses the execution channel. Member join/leave events have no such channel. An explicit destination must pass the host's channel-scope checks; a guild action cannot redirect itself into a different guild or a category channel. Discord still requires the bot's actual sending permissions.

Ordinary outbound text disables uncontrolled mentions. An apparent `<@id>` string is not a promise to ping a user. Text is transported in chunks of approximately **1,950 UTF-8 bytes**, even though action validation uses the 2,000-character limit. A valid Unicode-heavy response can therefore produce multiple messages/chunks.

## Send direct message

```lua
{ type = "send_direct_message", user_id = "100000000000000005", content = "Your requested information." }
```

The target is a user snowflake string. Delivery can fail because Discord disallows DMs, blocks the bot or cannot reach the recipient. The action result is not returned to Lua. In a command, such a failure stops remaining actions; do not place essential unrelated work after an optional DM unless that ordering is intended.

## Delete message

```lua
{ type = "delete_message", channel_id = ctx.channel_id, message_id = ctx.data.message_id }
```

Both IDs are required. Interactive execution requires Manage Messages from actor and app, and target-channel validation. Event automation uses the event permission model described below. A deleted/nonexistent message or a Discord permission error can still fail execution.

## Timeout member

```lua
{
    type = "timeout_member", user_id = "100000000000000005",
    seconds = 600, reason = "Repeated disruption"
}
```

`seconds` is a required unsigned 64-bit integer from **1 through 2,419,200** (28 days). The optional reason has a 512-character bound. A guild is required; interactive actor and app need Moderate Members. Member/role hierarchy checks apply, including protected high-ranking targets.

This action directly requests a timeout. To associate a timeout with a durable case, use the case service and its additional policy; it is not automatically equivalent to creating a case record.

## Kick member

```lua
{ type = "kick_member", user_id = "100000000000000005", reason = "Repeated disruption" }
```

Requires a guild, actor/app Kick Members in interactive execution and applicable member hierarchy checks. Optional reason: at most 512 characters.

## Ban member

```lua
{
    type = "ban_member", user_id = "100000000000000005",
    delete_message_days = 1, reason = "Spam"
}
```

`delete_message_days` is an unsigned 8-bit integer, default **0**, allowed **0–7**. The host converts it for Discord's message-deletion window. Requires a guild, actor/app Ban Members in interactive execution and hierarchy checks where a target member is available. Optional reason: at most 512 characters. A basic `unban_member` action is not exposed; the moderation-case service has an `unban` case kind.

## Add or remove role

```lua
{ type = "add_role", user_id = "100000000000000005", role_id = "100000000000000004" }
{ type = "remove_role", user_id = "100000000000000005", role_id = "100000000000000004", reason = "Membership ended" }
```

The shape is identical for both types. Requires a guild, actor/app Manage Roles for interactive execution, and appropriate target-member and target-role checks. A configured role ID does not bypass hierarchy, managed-role restrictions or guild isolation. The optional reason is at most 512 characters.

## Purge

```lua
{ type = "purge", amount = 20 }
```

`amount` is a required unsigned 8-bit integer from **1 through 100**. An optional `channel_id` overrides the current channel. Interactive actor/app require Manage Messages, and the channel must pass scope checks. The number is the requested fetch/deletion amount; actual deletions depend on available messages and Discord's bulk-delete rules. Do not claim an exact deleted count in Lua before Rust executes the request.

## Music

Music uses a string operation rather than the nested table used by the other services:

```lua
{ type = "music", operation = "play", query = "a song title" }
{ type = "music", operation = "volume", query = "75" }
{ type = "music", operation = "queue" }
```

| Operation | `query` | Behavior |
| --- | --- | --- |
| `play` | Required nonblank search text or permitted HTTPS URL | Connects to the invoking user's voice channel when needed, enqueues one source |
| `pause` | Unused | Pauses the queue |
| `resume` | Unused | Resumes playback |
| `skip` | Unused | Skips the current item |
| `stop` | Unused | Stops playback and clears the queue |
| `leave` | Unused | Disconnects; disconnected state returns a status message |
| `queue` | Unused | Returns a track-count status; it is not a Lua-readable queue list |
| `volume` | Required numeric string | Current-track volume, **0–150** percent; `"75"` becomes a factor of `0.75` |
| `loop` | Unused | Toggles infinite looping for the current track; no active track is an error |

The queue limit is **100 tracks**. Except for queue inspection, controlling a connected player requires the user to be in the same voice channel as the bot. A new play request requires the user's cached voice channel. Playback depends on the Rust voice manager and the configured media environment.

Source validation rejects control characters and leading command-option syntax; URL input is constrained to the host's permitted HTTPS sources. Plain search text and URLs follow different source paths. Lua cannot change the allowlist or execute a media process itself. Read [security](security-errors.md) and [music implementation](../../src/discord/music.rs) before broadening accepted input in a deployment.

A `default_volume` property in a Lua configuration is not an argument to this action. The action's `query` contract is percentage text. Do not assume every shipped module configuration matches its declared schema; see [#128](https://github.com/abanach-com/rust-discord-bot/issues/128).

## Specialized service actions

```lua
{ type = "progression", operation = { type = "balance" } }
{ type = "scheduler", operation = { type = "list", scope = "mine", limit = 10 } }
{ type = "ticket", operation = { type = "info" } }
{ type = "moderation_case", operation = { type = "view", case_number = 1 } }
```

These are valid shapes, but their availability depends on module identity, actor, guild and interaction context. In particular, only module `progression` can call progression, only `scheduler` can call scheduler and only `tickets` can call ticket operations. Scheduler and ticket operations are command-only. The service chapters describe every nested field; do not copy the snippets into an arbitrary module and expect execution to succeed.

Service-generated status text is sent by Rust as an interaction follow-up where applicable. Lua cannot capture a created job ID, case details, balance or ticket object to compute another action in the same callback.

## Audit

```lua
{
    type = "audit", event = "example_requested",
    data = { command = command, selected_user_id = (ctx.options or {}).person }
}
```

`event` must be nonblank after trimming and at most **64 bytes**. `data` is a JSON object, default `{}`. Use short event names, small serializable values and only data that is necessary for diagnostics. Infrastructure secrets should never have been supplied to Lua in the first place.

An explicit audit action is additional application-level metadata; privileged execution also has its own host audit behavior. Avoid recording a success event before the operation succeeds. Event execution has special handling for some `*_sent` records after a preceding action failure; do not treat that convention as a general transaction or rollback mechanism.

## Permissions and execution failures

For command-driven basic moderation, Rust rechecks the actor's and app's permission bitfields. Administrator is treated as satisfying the permission requirement. Required bits are Manage Messages for delete/purge, Moderate Members for timeout, Kick Members for kick, Ban Members for ban, and Manage Roles for role changes. Role hierarchy and destination guild checks are separate requirements.

The generic event execution context disables the human actor permission check for installed automation and supplies a permissive app bitfield to that internal check. Discord's real bot permissions and relevant hierarchy checks still constrain actual operations. This is why installing a Lua source package is an operator trust decision. Specialized services can impose stricter independent actor or interaction requirements.

Interactive rate-limit categories include ticket creation, scheduler creation, music play and destructive administrative actions. These are host categories, not a configurable Lua rate-limit API.

For commands, an execution error is reported and later actions stop; earlier effects remain. For events, errors are logged/audited and later actions can continue. No action list is a database-and-Discord transaction. Review [errors](security-errors.md#failure-stages) before designing multi-step workflows.
