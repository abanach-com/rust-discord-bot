# 4. Callbacks, contexts and events

[Handbook index](README.md) · Previous: [Manifests](manifests.md) · Next: [Actions](actions.md)

Sources: [model.rs](../../src/lua/model.rs), [handler.rs](../../src/discord/handler.rs), [engine.rs](../../src/lua/engine.rs).

## The two callbacks

```lua
local function on_command(command, ctx)
    -- command is the top-level registered slash-command name.
    return {}
end

local function on_event(event, ctx)
    -- event is the dispatched event-name string.
    return {}
end
```

These are the only Lua callback entry points used by the current handler. Each returns an array of actions. A module need only implement the callbacks it actually uses; a missing callback when invoked produces an execution error. A module does not receive commands it has not declared or events it has not subscribed to.

There are no dispatched `on_button`, `on_component`, `on_modal`, `on_autocomplete`, `on_timer`, `on_reaction`, `on_ready` or voice-state callbacks. The existence of component helper types elsewhere in the code does not add a callback. See [support matrix](support-matrix.md).

Export these functions as `on_command` and `on_event` fields of the returned module table, alongside `manifest`. A global function alone is not dispatched. See the complete [quickstart](quickstart.md#complete-source).

## Command context

| Field | Lua type | Meaning |
| --- | --- | --- |
| `guild_id` | string or `nil` | Origin guild, absent for a non-guild interaction |
| `channel_id` | string | Origin channel snowflake |
| `user_id` | string | Invoking user snowflake |
| `user_name` | string | Global display name when present, otherwise username |
| `member_roles` | array of strings | Guild member role IDs; no role objects |
| `member_permissions` | string | Decimal permission bitfield; `"0"` when not supplied |
| `locale` | string | Discord interaction locale |
| `options` | table in real command dispatch | Converted option values, with nested subcommands; model default is null when omitted in a manually constructed context |
| `config` | JSON-derived value | Effective module configuration; normally a table when the schema declares an object; empty object by default |

A representative plain-options fixture is:

```lua
local ctx = {
    guild_id = "100000000000000001",
    channel_id = "100000000000000002",
    user_id = "100000000000000003",
    user_name = "Example User",
    member_roles = { "100000000000000004" },
    member_permissions = "8192",
    locale = "en-US",
    options = { text = "Hello", person = "100000000000000005" },
    config = { greeting = "Hello from Lua" }
}
```

IDs are decimal strings even when they contain only digits. The role array is one-based. Options identify users, roles, channels, mentionables and attachments by ID only. There is no resolved-object bag, member nickname, user creation timestamp or arbitrary Discord cache query in this context.

## Nested options are the routing contract

For a normal option, use `ctx.options.amount`. A selected subcommand adds its name as an object key:

```lua
-- /wallet transfer recipient:@example amount:10
ctx.options = {
    transfer = {
        recipient = "100000000000000005",
        amount = 10
    }
}
```

A subcommand group adds another object level:

```lua
-- /wallet admin adjust user:@example amount:10
ctx.options = {
    admin = {
        adjust = {
            user = "100000000000000005",
            amount = 10
        }
    }
}
```

There is **no `options.subcommand` property** and child values are not flattened. Even a subcommand with no scalar options is represented by a selected branch table. Preserve booleans by testing `value ~= nil` when `false` is meaningful; `value or default` replaces `false` with the fallback.

```lua
local options = ctx.options or {}
if options.transfer then
    local args = options.transfer
    -- args.recipient and args.amount belong to this branch.
elseif options.admin and options.admin.adjust then
    local args = options.admin.adjust
    -- Handle the grouped command.
end
```

The complete [nested-router example](../../examples/lua/nested-router/module.lua) demonstrates this pattern. A number of shipped modules currently use a flat dispatcher; that defect is tracked in [#126](https://github.com/abanach-com/rust-discord-bot/issues/126). Their dispatcher code is not a template for new modules.

## Event envelope

| Field | Lua type | Meaning |
| --- | --- | --- |
| `name` | string | Event name; same value as the first callback argument |
| `guild_id` | string or `nil` | Origin guild |
| `channel_id` | string or `nil` | Event channel when available |
| `actor_id` | string or `nil` | User associated with the event when available; not necessarily a moderator |
| `data` | table | Event-specific payload |
| `config` | JSON-derived value | Effective module configuration for this guild; the schema determines the intended type |

Event payloads reflect gateway data and cache availability. Optional data is absent or `nil`; guard it before indexing. `actor_id` identifies the event's user when known; it is not evidence that this user authorized every action requested by an installed automation module.

## Event reference

### `message_create`

The handler dispatches this event for non-bot messages in guilds. Bot-authored messages and DMs do not reach Lua subscribers through this path.

Envelope: `guild_id`, `channel_id` and `actor_id` are present. `actor_id` is the author ID.

| `data` field | Type | Notes |
| --- | --- | --- |
| `message_id` | string | Message snowflake |
| `content` | string | Content supplied by Discord; availability depends on the application's gateway/intents configuration |
| `author.id` | string | Author snowflake |
| `author.name` | string | Username |
| `author.global_name` | string or `nil` | Global display name |
| `mentions` | array of strings | Mentioned user IDs |
| `mention_roles` | array of strings | Mentioned role IDs |
| `mention_everyone` | boolean | Gateway mention-everyone flag |
| `attachments` | array of tables | Attachment metadata described below |

Each attachment contains `id` (string), `filename` (string), `size` (number), `content_type` (string or `nil`) and `url` (string). Lua can inspect the URL as data but cannot fetch its contents through a host HTTP helper.

### `message_update`

This path dispatches guild message edits with usable content. Known bot authors, guildless updates and updates without content are skipped. The handler combines gateway update data and the available updated message; partial events can still lack author information.

The payload has the message fields above and `is_edit=true`. The handler always constructs an `author` object, but its fields can be absent: without author information Lua receives `author = {}`. In this event, `author.id`, `author.name`, `author.global_name` and `actor_id` are optional. Testing only `if author` does not establish that `author.id` exists. Collections may be empty where a partial update did not supply them. There is no guaranteed old content, before/after diff or original creation-time snapshot. Do not interpret an absent field as a confirmed deletion of that field in Discord.

```lua
local author = ctx.data.author
local author_id = author and author.id or ctx.actor_id
if not author_id then return {} end
```

### `guild_member_add`

Envelope: guild and actor IDs are present, channel ID is absent. `actor_id` is the arriving member.

| `data` field | Type | Notes |
| --- | --- | --- |
| `user.id`, `user.name` | string | Snowflake and username |
| `user.global_name` | string or `nil` | Global display name |
| `user.bot` | boolean | Whether the account is a bot |
| `user.avatar` | string or `nil` | Avatar hash, not a complete image URL |
| `user.has_avatar` | boolean | Whether a custom avatar is present |
| `roles` | array of strings | Current member role IDs |
| `joined_at` | string or `nil` | Serialized join timestamp |
| `recent_joins_count` | number | Count within the host's recent-join window, including this join |
| `recent_joins_window_seconds` | number | Currently `60` |
| `guild_name` | string or `nil` | Available from cache |
| `member_count` | number or `nil` | Available from cache |

Use an explicit configured channel for a welcome message. `reply` and `send_message` without `channel_id` cannot obtain a channel from this event envelope.

### `guild_member_remove`

Envelope: guild and actor IDs are present, channel ID is absent. The user object contains `id`, `name`, optional `global_name`, and `bot`. `roles` and `joined_at` are present only when the old member is available from cache. `guild_name` and `member_count` also depend on cache data.

This event does not distinguish voluntary leave, kick or ban, and supplies no reliable moderator/reason. Do not label a removal as a sanction without another verified source.

### `message_delete`

Envelope: guild and channel IDs are present, actor ID is absent. `data` contains only:

```lua
{
    message_id = "100000000000000006",
    channel_id = "100000000000000002"
}
```

The deletion payload does not identify the deleter, author, message content or reason. It supports logging a deletion identifier, not reconstructing the deleted message or assigning blame.

## Example event handler

```lua
local function on_event(event, ctx)
    if event ~= "guild_member_add" then return {} end
    local config = ctx.config or {}
    if not config.welcome_channel_id then return {} end
    local user = ctx.data.user or {}
    return {
        {
            type = "send_message",
            channel_id = config.welcome_channel_id,
            content = "Welcome, " .. rustybot.escape_mentions(user.name or "member") .. "!"
        }
    }
end
```

Declare the subscription and corresponding configuration schema in the manifest. This fragment is callback logic; the complete, schema-aligned event package is [event-audit](../../examples/lua/event-audit/module.lua).

## Delivery and failure behavior

A subscribed module must be enabled for the guild. An error in one event module is logged and does not prevent dispatch to other eligible modules. Event action execution is not a slash-command interaction: there are no ephemeral event messages, service text responses are not passed back to Lua, and some command-only host services reject event use.

Gateway events can be partial and application availability can interrupt delivery. This API supplies no exactly-once event-processing or generic Lua deduplication store. Use the host service designed for the persistent operation and verify its idempotency contract when needed.
