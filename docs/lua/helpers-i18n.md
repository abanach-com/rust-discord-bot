# 11. Helpers and translations

[Handbook index](README.md) · Previous: [Moderation cases](moderation-cases.md) · Next: [Security and errors](security-errors.md)

The complete helper registration is in [engine.rs](../../src/lua/engine.rs). Translation lookup, locale normalization and catalogue loading are in [translations/mod.rs](../../src/translations/mod.rs); slash-command description localization is in [commands.rs](../../src/discord/commands.rs).

## Available names

`rustybot` and the compatibility name `zuckerbot` refer to the same helper table in each fresh VM. The table contains exactly these public entries:

| Entry | Arguments | Result and behavior |
| --- | --- | --- |
| `api_version` | Not a function | Integer `1`; version of this helper contract |
| `escape_mentions` | One string | Replaces every `@` with `@` followed by U+200B, a zero-width separator |
| `truncate` | String, nonnegative integer maximum | First N Rust Unicode scalar values of the input |
| `unix_time` | None | Current host wall-clock time in whole seconds since the Unix epoch |
| `t` | Key string, optional second argument, optional third argument | Translation lookup with locale selection and placeholder substitution |
| `translate` | Same as `t` | Alias of the table's `t` function |

The host also installs the global function `t`. It does **not** install a global `translate` function. Use `rustybot.translate(...)` or `zuckerbot.translate(...)` for that alias.

```lua
local api = rustybot -- zuckerbot is also available for older modules.
assert(api.api_version == 1)
local message = api.t("system.command_executed", "pl")
local same_message = t("system.command_executed", "pl")
local also_the_same = api.translate("system.command_executed", "pl")
```

These helpers do not provide a Discord client, logging handle, generic storage, timer registration, HTTP client or permission query. Lua receives data and returns [actions](actions.md); the host executes those actions afterward.

## Escaping and truncation

```lua
local name = rustybot.escape_mentions(ctx.user_name or "member")
local content = rustybot.truncate("Hello, " .. name .. "!", 1900)
return {{ type = "reply", content = content, ephemeral = true }}
```

`escape_mentions` changes visible text semantics around `@`; it is not a Markdown, HTML or URL sanitizer. It does not remove other formatting characters. Ordinary message actions also apply the host's Discord allowed-mentions policy. An apparent `<@id>` string in a response is not a guarantee of a notification.

`truncate("abc", 2)` returns `"ab"`; a limit of zero returns an empty string. An empty result is not valid ordinary message content, so choose a nonempty fallback where a zero-length input or configured limit is possible. The helper does not append an ellipsis. Its maximum is a Rust `usize`; use a nonnegative integer instead of a negative value or fraction.

The unit is a Unicode scalar value, not a byte or a displayed grapheme cluster. A combining sequence or a multi-code-point emoji can be split. Lua `#text` counts bytes and should not be used as a replacement for the host's character limit. Escape and assemble the final message before truncating it, so inserted characters and suffixes are included in the bound.

## Time

```lua
local requested_at = rustybot.unix_time()
return {{
    type = "audit", event = "example_requested",
    data = { requested_at = requested_at, user_id = ctx.user_id },
}}
```

`unix_time()` reads the host's current system clock. It is not monotonic, a simulation clock or persistent per-module state. Calling it does not schedule a future callback. Use the [scheduler](scheduler.md) for supported durable jobs, subject to its reserved module identity and command-context requirements.

For deterministic tests, the test harness must supply a controlled clock at the host/helper boundary. API 1 has no Lua function for setting the system clock or replacing the production host's clock.

## Translation call forms

The key is a required string. Each of arguments 2 and 3 may independently be:

| Value | Interpretation |
| --- | --- |
| String | Sets the locale |
| Table with string `locale` | Sets the locale, then reads the other string-keyed entries as interpolation arguments |
| Other table | Reads string-keyed entries as interpolation arguments |
| `nil` or another value | No additional locale or interpolation data |

Processing order is argument 2, then argument 3. The later locale wins, and later interpolation entries overwrite earlier entries with the same key. The reserved table key `locale` is excluded from interpolation. Strings, integers, numbers and booleans become text; nested tables and other unsupported values become empty strings. Do not pass complex objects as template values.

```lua
-- No implicit per-invocation locale: without an argument, lookup starts in English.
local english = rustybot.t("system.command_executed")

-- An explicit locale can be the second or third argument.
local polish = rustybot.t("system.command_executed", "pl")
local german = rustybot.t("system.command_executed", nil, "de")

-- A command context has locale, but there is no ambient ctx inside the helper.
local localized = rustybot.t("system.command_executed", ctx)

-- Prefer a small argument map and the locale string for interpolation.
local limited = rustybot.t("system.rate_limited", {
    command = "ping", seconds = 5,
}, ctx.locale)
```

For `system.rate_limited`, the template contains `{command}` and `{seconds}`. Replacement is literal text substitution, not `string.format`, ICU MessageFormat, plural selection or a Lua expression evaluator. A missing placeholder stays in the returned text. An unknown translation key is returned unchanged after fallback lookup fails.

Passing `ctx` as the third argument also imports its other string-keyed fields. For example, `user_name` from `ctx` can overwrite a deliberately escaped `user_name` supplied in argument 2. Passing only `ctx.locale` avoids that collision:

```lua
local args = { user_name = rustybot.escape_mentions(ctx.user_name or "member") }
local locale = ctx.locale or "en"
-- Example application key: add it to the catalogues before relying on the result.
local greeting = rustybot.t("examples.greeting", args, locale)
```

Translations do not automatically escape interpolated names or untrusted content. Apply the appropriate text treatment before interpolation and validate the final message length. Event contexts have no top-level `locale` field; choose an explicit locale from documented configuration or a fixed default for an event response.

## Catalogue format and fallback

The embedded base catalogues are [English](../../translations/en.json), [Polish](../../translations/pl.json) and [German](../../translations/de.json). Keys traverse nested JSON objects using dots; a dotted property name at the root is not equivalent to nested objects.

For a new example key, merge this structure into each applicable catalogue:

```json
{
  "examples": {
    "greeting": "Hello, {user_name}!"
  }
}
```

Do not replace a complete catalogue file with this tiny fragment. Runtime overrides replace the entire stored catalogue for a language; they are not deep merges with the embedded language.

Lookup first normalizes the requested locale, then tries that language, then English if the requested language is not English. If neither contains a string at the key, it returns the key itself. Normalization trims whitespace and lowercases the name; prefixes `pl`, `de` and `en` map to those base languages. Other names are reduced to the segment before `-`, so `fr-CA` becomes `fr` for lookup. Numeric values or JSON objects at a key are not translation strings.

## Loading changes

The translation store is a `LazyLock` initialized once when first accessed:

1. Load the three embedded defaults.
2. Read `.json` overrides in `./translations` relative to the working directory.
3. If the runtime environment contains `CARGO_MANIFEST_DIR`, read its `translations` directory afterward.

Later files for the same language replace the earlier catalogue. The filename stem is lowercased. Unreadable or invalid JSON files are skipped by the current loader. There is no filesystem watcher or Lua translation-reload helper in this implementation; restart the process to load edited catalogues reliably. Changing a file while the process is running does not mutate the already initialized store.

This behavior differs from a promise of automatic live translation updates. Include a missing-key check and JSON syntax validation in your authoring workflow rather than assuming startup always reports an invalid override.

## Slash-command description localization

Descriptions are prepared during Rust command registration. The builder can translate a description containing `.` as an English lookup key. Additional language descriptions are collected using:

- `commands.<command>.description`, with a fallback search under `modules.<module>.commands.<command>.description`.
- `commands.<command>.options.<option>.description`, with a similar module fallback.

The module fallback runs only when the `commands.*` lookup finds no non-English localizations at all. It does not fill missing languages when at least one `commands.*` localization exists. For example, a Polish entry under `commands.*` prevents a German entry stored only under `modules.*` from being discovered by that fallback.

The implementation adds **description** localizations. It does not call the builder's name-localization API, and the manifest has no general name-localizations map. Do not promise localized command names merely because translated descriptions exist. Nested options are looked up by top-level command name and the current option name; the helper does not include a full subcommand-group path.

Use supported Discord locale names when supplying registration localizations. Restart and register commands in the intended development/global scope after changing registration descriptions. Runtime reply translation and Discord's command registration are separate operations.

## Author checklist

- Use a real helper name and remember that `api_version` is a value.
- Pass the desired locale explicitly; event callbacks do not supply `ctx.locale`.
- Keep interpolation values scalar and escape user-provided text deliberately.
- Keep the complete locale catalogue when applying runtime overrides.
- Verify missing keys, fallback behavior and final message lengths.
- Use [testing guidance](testing.md) to distinguish helper stubs from execution of the actual Rust host.
