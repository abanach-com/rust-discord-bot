# 10. Moderation cases

[Handbook index](README.md) · Previous: [Tickets](tickets.md) · Next: [Helpers and translations](helpers-i18n.md)

## Access and side effects

Use `{ type = "moderation_case", operation = { type = "...", ... } }`. This action has **no reserved module-ID requirement**, but it needs a guild and actor and the executor enforces permissions and record visibility. It is not a general database interface.

The executor considers an actor a moderator if they have any of: administrator, `MANAGE_GUILD`, `MODERATE_MEMBERS`, `BAN_MEMBERS`, `KICK_MEMBERS`, or `MANAGE_MESSAGES`. Actual sanctions additionally require the corresponding sanction permission and Discord hierarchy checks. Creating a warning can therefore be authorized differently from creating a ban.

The host stores cases, notes, evidence and version history and renders responses. Lua receives no case number back within the same invocation. A callback cannot create a case and use a synchronously returned ID in a second action.

## Operation reference

Case numbers are positive signed 64-bit integers scoped to the current guild. They are not Discord snowflake strings. User IDs remain strings. Text limits are measured on trimmed text.

| `operation.type` | Required fields | Optional fields and defaults | Bounds / behavior |
| --- | --- | --- | --- |
| `create` | `subject_user_id`, `reason` | `kind = "warning"`, `severity = "medium"`, `points = 0`, `visible_to_subject = true`, `duration_seconds`, `delete_days` | Reason 1–2000 characters; details below |
| `view` | `case_number` | None | Moderator or subject of a visible case |
| `list` | None | `subject_user_id`, `limit = 10`, `visible_to_subject_only = false` | Limit 1–25; nonmoderator restricted to their own visible cases |
| `add_note` | `case_number`, `body` | `visible_to_subject = false` | Body 1–2000 characters; moderator only |
| `add_evidence` | `case_number`, `label`, `value` | None | Label 1–200; value 1–4096 characters; moderator only |
| `void` | `case_number`, `reason` | `expected_version = 0` | Reason 1–2000; moderator only |
| `edit` | `case_number` | `expected_version = 0`, `reason`, `severity`, `points`, `visible_to_subject`, `duration_seconds` | Optional updates obey create-field bounds; moderator only |
| `restore` | `case_number` | `expected_version = 0` | Restores a voided record; moderator only |
| `stats` | None | `moderator_user_id` | Moderator only; optional user filter |

### Create and edit fields

| Field | Accepted values |
| --- | --- |
| `kind` | `warning`, `staff_note`, `timeout`, `kick`, `ban`, `unban`, `softban`, `automod`, `other` |
| `severity` | `info`, `low`, `medium`, `high`, `critical` |
| `points` | Integer 0–10,000 |
| `duration_seconds` | Optional unsigned integer 1–315,619,200 (3653 days) at model validation; actual timeout execution is additionally bounded by Discord's 28-day limit |
| `delete_days` | Optional unsigned integer 0–7; used by ban/softban |

Omitting an edit field preserves its value. This operation does not expose an explicit null-clearing form for `duration_seconds`. Do not assume an empty edit performs a useful mutation; send only intended changes.

### Case creation and sanctions

A `create` operation for `timeout`, `kick`, `ban`, `unban`, or `softban` can perform Discord moderation as part of the workflow. The service checks the actor's action-specific permissions; interactive creation also checks member hierarchy except for unban. Ban uses zero days of message deletion when `delete_days` is absent; softban defaults to one day and performs a ban followed by unban.

`kind = "timeout"` with no duration can create a case without applying a timed timeout. To request a timeout, supply `duration_seconds` explicitly and keep it within the actual Discord limit. Case validation's larger duration bound must not be mistaken for Discord timeout support.

`warning`, `staff_note`, `automod`, and `other` record their corresponding case kinds; naming a case `automod` is not a separate automatic scanning callback.

## Examples

This fragment records a visible warning from an authorized command:

```lua
return {
    {
        type = "moderation_case",
        operation = {
            type = "create",
            subject_user_id = "100000000000000004",
            kind = "warning",
            reason = "Repeated unsolicited promotional messages after a reminder",
            severity = "low",
            points = 1,
            visible_to_subject = true,
        },
    },
}
```

A private staff note on an existing case:

```lua
return {
    {
        type = "moderation_case",
        operation = {
            type = "add_note",
            case_number = 12,
            body = "Second moderator reviewed the original messages.",
            visible_to_subject = false,
        },
    },
}
```

These are action-list fragments, not complete module files. A full file must return a module table with `manifest` and its exported callback; see [quickstart](quickstart.md).

## Visibility and evidence

A nonmoderator can view their own case only when it is visible to the subject. The host filters notes to visible notes for that viewer; evidence is exposed to moderators. Nonmoderator listing forces the actor's own user ID and visible cases even when the caller asks for another subject or a broader filter. Never recreate these checks solely in Lua or add private evidence to a separate public `reply` action.

Evidence is stored text with a label. It does not cause the host to fetch a URL, preserve external media, or verify a claim. Use a Discord message URL as evidence only when the moderators who need it can access the original message.

## Version guards and reversals

`void`, `edit`, and `restore` support optimistic concurrency through `expected_version`. A positive value requests a mutation against that version. When the value is zero or negative, the executor obtains the current version. The default therefore does not let a Lua author detect that a human's older view has become stale; pass an observed positive version when a workflow supplies one.

Voiding, editing or restoring a case changes the case record. It does **not** automatically reverse a previously applied timeout, kick, ban or other Discord effect. An unban is a separate moderation operation. Database transactions do not make Discord and database writes one atomic operation, and a sequence of Lua actions has no global rollback.

## Sources

See [operation types and validation](../../src/lua/moderation_cases.rs), [Discord executor and visibility](../../src/discord/moderation_cases.rs), [case storage](../../src/moderation_cases), and the [moderation case operator guide](../MODERATION_CASES.md).
