# 9. Tickets

[Handbook index](README.md) · Previous: [Scheduler](scheduler.md) · Next: [Moderation cases](moderation-cases.md)

## Access and request shape

The outer action is `ticket` and its `operation.type` selects the operation. The executor accepts it only from runtime module ID **`tickets`**, in an interactive command with a guild, channel, and actor. The fragments below belong inside that trusted module's returned action list. A separate custom module cannot call the ticket service by declaring a similarly named command.

Rust creates channels, controls permission overwrites, maintains ticket records, stores transcripts, and formats responses. A Lua callback cannot synchronously read a ticket record or capture a newly created channel ID. Most operations address the ticket registered to **the current channel**; there is no general `ticket_id` field in these operations.

## Operations

All IDs are nonzero decimal strings. Role arrays default to `{}`; they permit at most 20 unique IDs. Text limits below apply after trimming.

| `operation.type` | Required fields | Optional fields and defaults | Validation / target |
| --- | --- | --- | --- |
| `open` | `subject`, `description`, `policy` | `queue = "support"` | Subject 1–100 characters; description 1–1800; queue 1–32 ASCII lowercase letters, digits, `_`, `-` |
| `list` | None | `scope = "mine"`, `limit = 10`, `support_role_ids = {}` | Scope `mine` or `all`; limit 1–25 |
| `info` | None | None | Current ticket channel; includes participants and recent events |
| `claim` | None | `support_role_ids = {}` | Current ticket; staff only |
| `unclaim` | None | `support_role_ids = {}` | Current ticket; current assignee or staff |
| `close` | `policy` | `reason` | Current ticket; reason, if supplied, 1–512 characters |
| `reopen` | `open_category_id` | `support_role_ids = {}` | Current closed ticket; staff only |
| `add_member` | `user_id` | `support_role_ids = {}`, `creator_can_manage_participants = false` | Current ticket; staff or permitted creator |
| `remove_member` | `user_id` | Same as `add_member` | Current ticket; staff or permitted creator |
| `rename` | `name` | `support_role_ids = {}`, `creator_can_rename = false` | Name 2–100 characters before channel-name sanitization |
| `set_priority` | `priority` | `support_role_ids = {}` | Priority `low`, `normal`, `high`, or `urgent`; staff only |
| `transcript` | None | `support_role_ids = {}`, `log_channel_id`, `max_messages = 500` | Message limit 1–1000; creator, staff, or participant |

Ticket staff means an actor with administrator, `MANAGE_CHANNELS`, **or `MANAGE_GUILD`**, or membership in one of the supplied support roles. When opening a ticket, the service validates configured support roles against the guild and rejects the everyone role. Other operations validate the supplied ID syntax and uniqueness, then use role membership in their staff checks; they do not all repeat the opening-time guild-role validation. Supplying support roles is a policy decision by trusted module code, not a grant that an end user should freely choose.

`scope = "all"` requires staff. `info` relies on the caller's access to the current Discord channel; its operation has no support-role policy snapshot. Other operations apply their own creator, participant, assignee and staff checks. The bot also needs the relevant Discord channel permissions, including channel management for lifecycle changes and history access for transcripts.

## Open policy

`open.policy` is required, even though most individual policy fields have defaults.

| Field | Type | Default / requirement |
| --- | --- | --- |
| `open_category_id` | String ID | Required; must be a category in the current guild |
| `archive_category_id` | String ID | Omitted; if supplied, must be a category in the guild |
| `support_role_ids` | Array of string IDs | Empty; at most 20 unique roles |
| `log_channel_id` | String ID | Omitted; destination checked against the current guild |
| `max_open_per_user` | Integer (`u8`) | `3`; valid 1–25 |
| `channel_name_prefix` | String | `"ticket"`; 1–24 ASCII lowercase letters, digits, `_`, `-` |
| `welcome_message` | String | Omitted; if supplied, 1–1500 trimmed characters |

Rust reserves a ticket before channel creation, checks the per-user open limit, creates the channel with overwrites, and activates the record. The welcome post contains the creator, ticket number, queue, subject and description, followed by `welcome_message` or the host default. The custom welcome field is appended text; it has no Lua template-variable expansion.

```lua
-- Fragment for manifest.id == "tickets", inside on_command.
return {
    {
        type = "ticket",
        operation = {
            type = "open",
            subject = "Account access",
            description = "I need help accessing the community portal.",
            queue = "support",
            policy = {
                open_category_id = "100000000000000010",
                support_role_ids = { "100000000000000011" },
                max_open_per_user = 3,
                channel_name_prefix = "ticket",
                welcome_message = "Please describe the problem without posting credentials.",
            },
        },
    },
}
```

Replace example IDs with existing guild resources. The operation cannot create its own category or support role.

## Close policy and reopening

| `close.policy` field | Type | Default / bounds |
| --- | --- | --- |
| `support_role_ids` | Array of IDs | Empty; at most 20 unique roles |
| `archive_category_id` | String ID | Omitted |
| `log_channel_id` | String ID | Omitted |
| `creator_can_close` | Boolean | `true` |
| `generate_transcript` | Boolean | `true` |
| `transcript_max_messages` | Integer (`u16`) | `500`; valid 1–1000 |

```lua
-- Fragment for the trusted tickets module in the current ticket channel.
return {
    {
        type = "ticket",
        operation = {
            type = "close",
            reason = "Resolved with the requester",
            policy = {
                support_role_ids = { "100000000000000011" },
                creator_can_close = true,
                generate_transcript = true,
                transcript_max_messages = 500,
            },
        },
    },
}
```

Closing changes ticket state and channel permissions; an archive category can relocate the channel. Reopening requires a closed record and an explicit open category. The service maintains states such as provisioning, open, claimed, closed and failed; it reports rejected transitions instead of treating every operation as unconditional success. Do not model these transitions with a Lua global.

## Participants and transcripts

Participant operations update both the service's participant list and Discord channel access. They have checks beyond a syntactically valid user ID, including restrictions on the creator and bot. Renaming sanitizes the requested string into a Discord channel name; the final name may differ from the supplied text.

A transcript reads a bounded amount of channel history and stores a plaintext transcript. The renderer caps its content at **7 MiB** and can truncate before the requested message count. The service posts a text attachment in the ticket channel and can send a copy to a configured log channel. A transcript is a point-in-time capture, not a live message archive. HTML rendering exists in the host code but is not a separate Lua action.

There are no button or modal ticket callbacks in the Lua dispatch path. Use slash commands for the implemented ticket flow; see the [interaction support matrix](support-matrix.md#components-modals-and-autocomplete).

## Sources

The complete field contract and validation live in [src/lua/tickets.rs](../../src/lua/tickets.rs). Identity checks, permissions, lifecycle changes, transcript limits and rendering live in [src/discord/tickets.rs](../../src/discord/tickets.rs). The [ticket storage model](../../src/tickets) and [operator guide](../TICKETS.md) cover persistence and deployment.
