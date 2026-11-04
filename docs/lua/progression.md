# 7. Progression

[Handbook index](README.md) · Previous: [State and services](services.md) · Next: [Scheduler](scheduler.md)

**Access:** the runtime module ID must be exactly `progression`, and execution must have a guild and actor. These operation fragments are for that trusted module. The `economy` or `achievements` IDs do not acquire access by returning a well-formed progression action. The implementation is [src/lua/progression.rs](../../src/lua/progression.rs) and [src/discord/progression.rs](../../src/discord/progression.rs).

## Envelope and result

```lua
return {
    { type = "progression", operation = { type = "balance" } }
}
```

The outer action always uses `type = "progression"`. The inner `operation.type` selects one of the eight operations below. Results are rendered by Rust; Lua cannot capture a profile, leaderboard or transaction result in a variable.

Discord user and role IDs are strings containing nonzero `u64` values. Never convert them to floating point. Bounds below are enforced before execution unless described as service behavior.

## `award_message_xp`

| Field | Type | Required/default | Constraint |
| --- | --- | --- | --- |
| `amount` | Unsigned 32-bit integer | Required | 1–1,000 |
| `cooldown_seconds` | Unsigned 32-bit integer | Required | 1–86,400 |
| `announce_level_up` | Boolean | `false` | Request a level-up announcement |
| `level_up_message` | String or absent | Absent | If supplied, 1–500 characters |
| `role_rewards` | Array | `{}` | At most 50 entries |
| `role_rewards[].level` | Signed 32-bit integer | Required | 1–1,000 |
| `role_rewards[].role_id` | Snowflake string | Required | Valid nonzero ID; Discord can reject the role grant for permissions or hierarchy |

Use from the trusted progression module's message-event handler. The service records the message activity even when the XP cooldown prevents an award; message count and earned XP are different metrics. The service evaluates level changes and role rewards. This path calls Discord directly for a newly earned role and logs failures; it does not run the additional role suitability checks used by the basic `add_role` executor. Discord can reject a grant because of the bot's permissions or hierarchy.

The level-up template supports `{user}`, `{level}` and `{xp}`. These placeholders belong to this service template, independently of the general translation helper.

```lua
-- Fragment inside the existing progression module's on_event callback.
if event == "message_create" then
    return {{
        type = "progression",
        operation = {
            type = "award_message_xp",
            amount = 20,
            cooldown_seconds = 60,
            announce_level_up = true,
            level_up_message = "{user} reached level {level} ({xp} XP).",
            role_rewards = {}
        }
    }}
end
return {}
```

## `profile` and `balance`

Each accepts optional `user_id`. Omit it to target the actor; provide a string ID to request another user's profile or balance in the current guild. Neither operation takes `recipient_user_id`, `member_id`, or an arbitrary guild ID.

```lua
{ type = "progression", operation = { type = "profile", user_id = ctx.user_id } }
```

A profile contains the host's current progression information; the action does not return a Lua table. The persistent member model includes XP, level, coins, reputation, message count, daily streak, last XP/daily timestamps and creation/update timestamps, scoped to guild and user.

## `leaderboard`

| Field | Required/default | Values |
| --- | --- | --- |
| `metric` | Required | `xp`, `coins`, `reputation`, `messages` |
| `limit` | Unsigned 8-bit integer, default `10` | 1–25 |

```lua
{ type = "progression", operation = { type = "leaderboard", metric = "xp", limit = 10 } }
```

The query is scoped to the current guild. It cannot fetch a cross-guild leaderboard or return rows to Lua for custom sorting.

## `daily`

| Field | Type | Required/default | Constraint |
| --- | --- | --- | --- |
| `base_reward` | Unsigned 32-bit integer | Required | 1–1,000,000 |
| `streak_bonus` | Unsigned 32-bit integer | `0` | At most 100,000 |
| `max_streak_bonus` | Unsigned 16-bit integer | `30` | At most 365 |

```lua
{ type = "progression", operation = {
    type = "daily", base_reward = 100, streak_bonus = 10, max_streak_bonus = 30
} }
```

The host enforces daily eligibility and updates the balance and streak. A rejected early claim cannot be bypassed with another Lua VM. The configured reward values are trusted policy from the installed progression module; do not expose unrestricted user-supplied reward values as slash options.

## `transfer`

| Field | Required/default | Constraint |
| --- | --- | --- |
| `user_id` | Required snowflake string | Recipient in the current guild |
| `amount` | Required unsigned 64-bit integer | 1–1,000,000,000,000 |

The actor is the sender. The service rejects self-transfer and insufficient funds and performs its balance changes through its storage operation. Lua does not supply a sender ID.

```lua
-- Correct nested option shape for /pay transfer person:... amount:...
local options = (ctx.options or {}).transfer
if options then
    return {{ type = "progression", operation = {
        type = "transfer", user_id = options.person, amount = options.amount
    } }}
end
```

This fragment still requires `manifest.id = "progression"` and a matching command declaration in that module. It does not authorize an independent package.

## `give_reputation`

| Field | Type | Required/default | Constraint |
| --- | --- | --- | --- |
| `user_id` | Snowflake string | Required | Recipient |
| `amount` | Unsigned 16-bit integer | `1` | 1–100 |
| `cooldown_seconds` | Unsigned 32-bit integer | `86400` | 60–604,800 |

The cooldown belongs to the giver within the guild, across recipients. Changing the recipient does not reset that cooldown. The service rejects self-reputation. Keep award size and cooldown in trusted module policy.

## `adjust`

| Field | Type | Required/default | Constraint |
| --- | --- | --- | --- |
| `user_id` | Snowflake string | Required | Target member |
| `xp_delta` | Signed 64-bit integer | `0` | Absolute value at most 1,000,000,000 |
| `coins_delta` | Signed 64-bit integer | `0` | Same bound |
| `reputation_delta` | Signed 64-bit integer | `0` | Same bound |
| `reason` | String or absent | Absent | At most 512 characters |

At least one delta must be nonzero. This operation requires an interactive command and `MANAGE_GUILD` or administrator permission. An event callback cannot use `adjust` to manufacture privileged staff authority. The host recomputes relevant progression state and records its service result.

```lua
{ type = "progression", operation = {
    type = "adjust", user_id = "100000000000000001",
    coins_delta = 25, reason = "Staff event reward"
} }
```

## Levels and compatibility

The storage progression model uses cumulative threshold `50 * level * (level + 1)`: level 1 requires 100 total XP, level 2 requires 300, and level 10 requires 5,500. Consult the [storage implementation](../../src/storage) when changing a progression policy; Lua does not define a replacement level formula through this action.

`view_progress`, `transfer_coins`, and `adjust_member_progress` are **not aliases**. The current shipped `economy` and `achievements` code contains incompatible requests and is also outside the allowed service ID. See [issue #129](https://github.com/abanach-com/rust-discord-bot/issues/129). A syntax-valid Lua file or successfully loaded manifest does not establish that a returned operation is supported.
