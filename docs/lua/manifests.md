# 3. Manifest and slash-command reference

[Handbook index](README.md) · Previous: [Quickstart](quickstart.md) · Next: [Contexts](contexts-events.md)

Sources: [model.rs](../../src/lua/model.rs), [engine.rs](../../src/lua/engine.rs), [commands.rs](../../src/discord/commands.rs), [module installer](../../src/lua).

## Manifest fields

The source file must evaluate to a **module table** containing a `manifest` field and any exported callbacks. The fields below belong to `module.manifest`; returning the manifest by itself is not sufficient.

| Field | Type | Required/default | Meaning and validation |
| --- | --- | --- | --- |
| `id` | string | Required | Runtime identity; 1–64 bytes, lowercase ASCII letters, digits, `_`, `-` |
| `name` | string | Required | Nonblank display name, at most 64 characters |
| `version` | string | Required | Version metadata; no semantic-version validation in this contract |
| `description` | string | Required | Nonblank module description, at most 200 characters |
| `category` | string | Required | Nonblank grouping label, at most 32 characters |
| `default_enabled` | boolean | `true` | Default when a guild has no saved enablement override |
| `commands` | array of commands | `{}` | Slash commands owned by this module |
| `events` | array of strings | `{}` | Event subscriptions; each identifier follows the ID character rule and 1–64 byte bound |
| `config_schema` | JSON-compatible table | `{}` | Compiled JSON Schema for guild configuration |
| `author` | string or absent | Absent | Optional metadata, may be supplied by installer |
| `folder` | string or absent | Absent | Optional package metadata, may be supplied by installer |

IDs and command names must be unique in the loaded module set. The total across all loaded modules is at most 95 commands (Discord enforces a limit of 100 CHAT_INPUT commands per scope, with 5 commands reserved by host features: `policy`, `duty`, `help`, `emoji`, `sticker`). Event-name syntax validation is not a supported-event allowlist: an unknown but syntactically valid name can load and never be dispatched. Use the five names in [events](contexts-events.md#event-reference).

There is no manifest `permissions`, `capabilities`, `dependencies`, `imports`, host HTTP grant or generic storage grant. Adding such fields does not enable a capability. Module ID restrictions on host services are described in [state and services](services.md).

## Command fields

| Field | Type | Required/default | Behavior |
| --- | --- | --- | --- |
| `name` | string | Required | 1–32 bytes; lowercase ASCII letters, digits, `_`, `-` |
| `description` | string | Required | 1–100 characters; can also be a translation key under the host's translation convention |
| `options` | array of options | `{}` | Up to 25 per level |
| `default_member_permissions` | string or absent | Absent | Decimal Discord permission bitfield supplied to registration |
| `integration_types` | array or absent | Guild installation by default | Values `"guild"`, `"user"` |
| `contexts` | array or absent | Guild context by default | Values `"guild"`, `"bot_dm"`, `"private_channel"` |
| `nsfw` | boolean | `false` | Discord command registration flag |
| `dm_permission` | boolean or absent | Absent | Legacy shorthand; cannot coexist with `contexts` |

`default_member_permissions` is a decimal string, for example `"8192"` for a command requiring Manage Messages by default. It affects Discord registration/discoverability; privileged actions still undergo host checks against the invoking actor and bot. Do not parse `ctx.member_permissions` as a floating-point number to implement a replacement permission system.

The host validates nonempty, duplicate-free `integration_types` and `contexts` arrays. `private_channel` requires `user` installation. `dm_permission=true` maps to `guild` plus `bot_dm`; `false` maps to `guild` only. Prefer explicit current fields.

Global registration sends installation and interaction contexts to Discord. Development guild command registration omits those global-only settings. A command that can register globally in a DM context may still return an action requiring a guild; Rust then rejects that action. Design the command behavior for every declared context.

## Option fields

| Field | Type | Required/default | Notes |
| --- | --- | --- | --- |
| `type` | string | Required | One of the types below |
| `name` | string | Required | Same 1–32 byte character rule as command names |
| `description` | string | Required | 1–100 characters |
| `required` | boolean | `false` | Required scalar options must precede optional ones at the same level |
| `autocomplete` | boolean | `false` | Registration flag exists; **no autocomplete interaction dispatcher** |
| `choices` | array | `{}` | At most 25; allowed for string, integer and number options |
| `options` | array | `{}` | Nested options, allowed only on subcommands and subcommand groups |
| `min_value`, `max_value` | number or absent | Absent | Numeric bounds, with integer-builder caveats below |
| `min_length`, `max_length` | unsigned 16-bit integer or absent | Absent | String length constraints passed to registration |

Supported type names:

| Type | Value received by command callback |
| --- | --- |
| `string` | String |
| `integer` | Integer number |
| `number` | Number |
| `boolean` | Boolean |
| `user` | User snowflake string |
| `channel` | Channel snowflake string |
| `role` | Role snowflake string |
| `mentionable` | Selected user or role snowflake string; no resolved-kind field |
| `attachment` | Attachment snowflake string; no resolved attachment object |
| `subcommand` | Nested table keyed by selected subcommand name |
| `subcommand_group` | Nested table keyed by group, then selected subcommand |

The registration builder implements these option types. The callback receives only the value conversion described above, so user objects, attachment URLs and resolved channel metadata are not automatically available from an option.

A choice has exactly the useful authoring shape `{ name = "Display label", value = ... }`. `name` is a string; `value` must match the option kind. The integer choice path reads a signed 64-bit integer and then converts it to **32-bit signed** for the Discord builder. Stay within that representable range. Integer `min_value`/`max_value` use a nonnegative integral conversion to `u64`; negative integer bounds are rejected by this implementation. Number-option bounds use floating point.

The host validates its own constraints; Discord additionally validates command structure and limits. Do not assume that an arbitrary combination of choices, bounds, required flags, nested groups and autocomplete will be accepted merely because its Lua table can deserialize.

## Correct subcommand declaration and routing

```lua
commands = {
    {
        name = "example", description = "An example command",
        options = {
            {
                type = "subcommand", name = "echo", description = "Echo text",
                options = {
                    { type = "string", name = "text",
                      description = "Text", required = true }
                }
            }
        }
    }
}
```

`/example echo text:hello` produces `ctx.options.echo.text == "hello"`. Route by branch presence:

```lua
local selected = (ctx.options or {}).echo
if selected then
    return {{ type = "reply", content = selected.text, ephemeral = true }}
end
return {}
```

Groups add another level. See the complete [nested-router package](../../examples/lua/nested-router/module.lua). The known shipped-module defect [#126](https://github.com/abanach-com/rust-discord-bot/issues/126) is why existing flat dispatchers should not be copied as a template.

## Configuration schema

`config_schema` is converted from Lua into JSON and compiled by the host's JSON Schema validator. Use JSON Schema vocabulary, not command-option vocabulary:

```lua
config_schema = {
    type = "object",
    additionalProperties = false,
    properties = {
        enabled = { type = "boolean" },
        destination_channel_id = {
            type = "string", pattern = "^[0-9]+$"
        },
        prefix = { type = "string", minLength = 1, maxLength = 30 }
    },
    required = { "enabled", "prefix" }
}
```

This schema checks configuration shape. It does not prove a channel belongs to the guild, a role is below the bot, or a caller may send a message. Runtime execution performs relevant checks. Conversely, the loader does not validate `config.json` against this schema. Dashboard writes do, and schema `default` annotations do not merge or populate data for callbacks.

Avoid using empty Lua tables where distinguishing an empty JSON object from an empty JSON array matters to a schema: use complete nonempty schema structures and test their serialized form in the Rust engine. The shipped examples use ordinary objects and nonempty `required` arrays where relevant.

## Installer fields

The XML root must be `<installer>` or `<install>`. A `<modules>` child contains package entries; without it, entries are read directly under the root. Use the complete format shown in [quickstart](quickstart.md#installer). A `<modules>` root alone is rejected. The parser is [install_xml.rs](../../src/lua/install_xml.rs).

| Installer field | Location and default |
| --- | --- |
| `version` | Root attribute, then child element; default `1.0.0` |
| `bot_name` | `<bot name="..." />`, or text inside `<bot>`; optional metadata |
| `min_engine_version` | Attribute on `<bot>`, or root child element; optional metadata, not an enforced compatibility gate |
| Module entries | `<module>`; parser also accepts `<plugin>` and `<feature>` |

| Package field | Default and behavior |
| --- | --- |
| `id` | Required, nonempty; attribute or child. Use the Lua manifest ID even though equality is not enforced |
| `enabled` | `true`; attribute or child; determines whether the package is loaded |
| `version` | `1.0.0`; attribute or child; metadata |
| `name` | ID; child or attribute; metadata |
| `description` | Empty string; child or attribute; metadata |
| `folder` | ID; child or attribute; package directory relative to the script root |
| `entry` | `module.lua`; child or attribute; source entry filename |
| `config` | `config.json`; child or attribute; empty value becomes absent and the loader uses its fallback |
| `author` | Absent; child or attribute; fallback for a Lua manifest without author |
| `category` | Absent; child or attribute; installer metadata |

Prefer child elements for package metadata and file paths, matching the production installer. For `id`, `enabled`, and `version`, an attribute takes precedence over a child. For the remaining package fields, a child takes precedence over an attribute. `min_engine_version` belongs to installer metadata, not to a package entry.

The loader's fallback search is documented in [runtime](runtime.md#package-discovery). Use one package directory, one returned module table and explicit installation; avoid relying on fallback names for a deployment-critical package.

## Permission design

Declare an appropriate default command permission for staff commands and return the smallest action set needed. The command declaration does not grant authority. Rust checks action permissions, guild/channel scope and relevant role hierarchy. Event automation has a different actor-permission model, so an event subscriber must only be installed from trusted code. Specialized service operations also enforce their own ownership or staff policy.

Do not treat a UI visibility setting or a Lua-side conditional as the final authorization boundary. See [security](security-errors.md) and the per-operation tables in the service chapters.
