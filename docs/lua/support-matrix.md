# 15. Support and current limitations

[Handbook index](README.md) · Previous: [Recipes](recipes.md)

This matrix describes the implementation at snapshot [`10a2ba1`](https://github.com/abanach-com/rust-discord-bot/tree/10a2ba1780a2becd6d7ba2bfc5c4f7a2fecc9882). “Supported” means the source contains a complete Lua-to-host path for the stated context; it does not assert successful delivery in every deployment. Discord permissions, configuration, storage and external media dependencies can still reject requests.

## How to assess a feature

A usable feature needs all applicable stages: a manifest field accepted by the loader, a Discord registration or event path, a callback invoked by the handler, an action decoded by the engine, and an executor that accepts the module and context. A helper type or a manifest flag implements only its own stage.

| Feature | Present implementation | Lua author guidance |
| --- | --- | --- |
| Slash commands | Manifest, command builder, command interaction dispatch, `on_command` | Supported; use nested options for subcommands |
| Command defaults and option choices | Manifest validation and Discord registration | Supported subject to validation and Discord acceptance |
| Installation and interaction contexts | Manifest and global command registration | Supported; development-guild registration omits these fields |
| Command description localization | Host catalogue applied by command builder | Supported for available translations |
| Command name localization | No implemented registration mapping | Do not declare localized names as a working feature |
| Guild message creation | Handler dispatches `message_create` | Supported for eligible nonbot guild messages |
| Guild message edits | Handler dispatches `message_update` with usable content | Supported with partial author data and other documented omissions |
| Guild message deletion | Handler dispatches `message_delete` | Supported metadata-only event |
| Member joins and leaves | Handler dispatches `guild_member_add` / `guild_member_remove` | Supported; no event channel |
| Reaction, voice or presence callbacks | No corresponding Lua dispatch | Unsupported even if a manifest lists a syntactically valid event name |
| Startup, shutdown or timer callbacks | No Lua callback path | Unsupported; host scheduler delivers messages independently |
| Slash command replies | `reply` through interaction executor | Supported, with deferred visibility and reply-order rules |
| Plain channel messages and DMs | Core action variants and executors | Supported with channel scope and Discord delivery constraints |
| Basic moderation and role changes | Core variants, validation and executor checks | Supported subject to permission and hierarchy checks |
| Music | String operation plus host voice/media execution | Supported with deployment requirements and voice context |
| Progression | Nested operation model and service executor | Supported only for module `progression` |
| Scheduler | Nested operation model, storage and delivery worker | Supported only for module `scheduler`, command context |
| Tickets | Nested operation model, storage and channel executor | Supported only for module `tickets`, command context |
| Moderation cases | Nested operation model, storage and executor | Supported for custom modules subject to actor/visibility rules |
| Audit | Core action and host audit path | Supported; choose small object payloads |
| Generic key-value/database access | No Lua host function or action | Unsupported |
| Runtime configuration mutation from Lua | No persistence action | Unsupported; `ctx.config` is input |
| HTTP, filesystem and process access from Lua | No permitted API | Unsupported by the sandbox |
| Cross-module calls | No dispatch action or host function | Unsupported |

Sources: [engine](../../src/lua/engine.rs), [models](../../src/lua/model.rs), [command registration](../../src/discord/commands.rs), [handler](../../src/discord/handler.rs), [executor](../../src/discord/executor.rs), and [host service references](services.md).

## Components, modals and autocomplete

| Item | Code that exists | Missing part for Lua use | Status |
| --- | --- | --- | --- |
| Signed component IDs | Rust payload, encoding, signature and validation helpers in [components.rs](../../src/discord/components.rs) | Lua component-producing action and interaction callback routing | Host helper only |
| Buttons and select menus | Component-related Rust helpers | Supported action fields/builders plus component interaction dispatch | Unavailable from current Lua API |
| Modals | No complete Lua response/submit path | Modal-open action and submit callback with context | Unavailable |
| Autocomplete option flag | `autocomplete` in manifest/model and slash option registration | Autocomplete interaction dispatch and choices response action | Registration only |
| `on_component` | No callback lookup from the handler | Dispatcher and context/action contract | Unsupported callback |
| `on_modal` / `on_modal_submit` | No callback lookup from the handler | Dispatcher and context/action contract | Unsupported callback |
| `on_autocomplete` | No callback lookup from the handler | Dispatcher and response contract | Unsupported callback |

The [interaction handler](../../src/discord/handler.rs) currently accepts command interactions for Lua execution. Adding a function with one of the names above does not cause it to run. A module may pass manifest validation with `autocomplete = true`, but that alone does not provide suggestions to users.

Likewise, a `components`, `embeds` or `modal` property on a `reply` action is not documented support. The [action model](../../src/lua/model.rs) defines the accepted behavior, and the executor must actually construct the corresponding Discord response. Unknown properties should never be used as an extension mechanism.

The signed-component helper is useful groundwork for a future host feature. It should not be presented as an exported Lua API or a working ticket-button flow.

The concrete extension proposals are tracked in [#10](https://github.com/abanach-com/rust-discord-bot/issues/10#issuecomment-6623776606) for a complete component/modal interaction path and [#4](https://github.com/abanach-com/rust-discord-bot/issues/4#issuecomment-6623776059) for executable documentation, engine fixtures and generated editor annotations. Their proposed APIs are not part of API 1.

## Exact callback surface

The module table exports only the callbacks currently requested by the host:

```lua
return {
    manifest = manifest,
    on_command = on_command,
    on_event = on_event,
}
```

Declare and export only the callbacks the package needs. `on_command(command, ctx)` handles the command name and its nested options. `on_event(event, ctx)` handles exactly the event string dispatched by Rust. Event names in a manifest are subscriptions; the loader's syntactic validation does not promise that Rust emits every possible name.

The actual event set is `message_create`, `message_update`, `message_delete`, `guild_member_add`, and `guild_member_remove`. Refer to [callbacks and contexts](contexts-events.md) for eligibility filters and complete payload tables.

## State and service restrictions

Every callback starts with a fresh VM and evaluates the source again. File-level locals, globals, counters and closures are not durable state. There is no `bot.*` object, `rustybot.storage`, SQL connection, generic key-value store, or synchronous query interface.

The service action is a request executed after the callback returns. A balance/profile/list result is rendered by Rust; it does not become a Lua return value that the same callback can inspect. A service response cannot feed a later action without a host design that explicitly supports that flow.

Reserved service access is based on `manifest.id`, not folder name, command name or installer display metadata. Only `progression`, `scheduler` and `tickets` can access their corresponding services. Scheduler and tickets additionally require an interactive command. A well-typed operation returned by another module is still denied. See [services](services.md).

## Known snapshot defects affecting module authors

These are specific implementation findings, not additional supported API variants.

| Issue | Observed incompatibility | Authoring consequence |
| --- | --- | --- |
| [#126: nested subcommands](https://github.com/abanach-com/rust-discord-bot/issues/126) | The handler nests subcommand/group options, while several shipped modules inspect a flat `options.subcommand` and flat arguments | Use `options.<subcommand>.<option>` or `options.<group>.<subcommand>.<option>`; use the handbook router example |
| [#128: package defaults](https://github.com/abanach-com/rust-discord-bot/issues/128) | Some shipped `config.json` values violate their manifest schemas; loader compiles schemas but does not validate package defaults | Validate defaults independently; do not copy shipped defaults without checking their schema |
| [#129: progression requests](https://github.com/abanach-com/rust-discord-bot/issues/129) | Economy/achievement scripts use unsupported operation names/fields and are also outside the reserved progression module identity | Use the documented Rust operation contract inside the permitted module; correcting a name alone does not fix the identity gate |
| [#110: target-channel permissions](https://github.com/abanach-com/rust-discord-bot/issues/110#issuecomment-6623746190) | Destination checks apply target overwrites to source-channel effective permissions instead of rebuilding the guild-role base | Keep destination choices constrained; do not treat the current resolver as complete authorization |
| [#121: interaction acknowledgement](https://github.com/abanach-com/rust-discord-bot/issues/121#issuecomment-6623747972) | In-flight defer and initial-response paths can race | An acknowledgement does not prove a final result; the host response state machine needs a fix |
| [#127: CI formatting](https://github.com/abanach-com/rust-discord-bot/issues/127) | Main CI job fails formatting; downstream checks in that run are skipped | Do not infer that Rust tests or Clippy passed from unrelated successful jobs |

The flat-option routing issue affects the snapshot's `economy`, `giveaway`, `afk`, `birthdays`, `achievements`, `logging`, `quotes`, `reaction_roles`, `starboard`, and `tags` scripts. Those scripts can still be useful as descriptions of intended user features, but their dispatcher pattern is not the implemented host contract.

In progression requests, the real operation names include `profile`, `balance`, `transfer` and `adjust`. A transfer's recipient is `user_id`. The spellings `view_progress`, `transfer_coins` with `recipient_user_id`, and `adjust_member_progress` are not aliases recognized by the current Rust enum.

Configuration has two additional pitfalls: valid scalar or array JSON can be loaded because the host stores a general JSON value, and unreadable/malformed package JSON falls back to `{}`. A module that expects `ctx.config.greeting` should declare an object schema, provide valid defaults and test the effective saved guild configuration.

## Distinguish intended features from working contracts

The repository's [feature matrix](../FEATURE_MATRIX.md) and shipped module names describe a broad product surface. For module implementation, verify the actual callback and executor path before claiming end-to-end support. A Lua file that parses, a manifest that registers, and a source module that appears in a catalogue each prove less than a successful invocation of its intended operation.

Similarly, a test suite can pass only the paths it exercises. Native Lua fixtures can confirm the nested dispatcher and returned data but cannot prove Rust decoding or Discord permissions. The [testing guide](testing.md) describes the required distinctions.

## Extending the host contract

A new feature that is absent from this matrix needs coordinated Rust changes rather than an undocumented Lua table field. Define its input and output contract, add validation, route the relevant interaction/event, enforce actor/guild/module boundaries, implement execution, and cover the boundary cases with meaningful tests. Update the handbook and complete examples alongside that implementation.

When extending a callback, document whether fields are optional, whether IDs are strings, whether a result is synchronous or an action request, and what survives a fresh VM. When extending a service, define its trust and permission model explicitly. These decisions determine whether a script can actually use the feature.

## Bundled module inventory

The production [installer](../../scripts/install.xml) selects **42 modules** declaring **86 top-level slash commands** at the audited snapshot. Subcommands are not counted separately. The table comes from their returned manifests; it records declarations, not verified end-to-end behavior. Some packages provide an intended feature or an acknowledgement without a complete working operation, as the defects above illustrate.

| Module | Top-level command declarations | Event subscriptions |
| --- | --- | --- |
| [`core`](../../scripts/core/module.lua) | `/ping`, `/about` | None |
| [`moderation`](../../scripts/moderation/module.lua) | `/moderate`, `/warn`, `/case` | None |
| [`automod`](../../scripts/automod/module.lua) | None | `message_create`, `message_update` |
| [`tickets`](../../scripts/tickets/module.lua) | `/ticket` | None |
| [`scheduler`](../../scripts/scheduler/module.lua) | `/remind`, `/reminders`, `/remindcontrol`, `/schedulemessage`, `/schedules`, `/schedulecontrol` | None |
| [`progression`](../../scripts/progression/module.lua) | `/rank`, `/balance`, `/leaderboard`, `/daily`, `/pay`, `/rep`, `/progressadmin` | `message_create` |
| [`music`](../../scripts/music/module.lua) | `/music` | None |
| [`community`](../../scripts/community/module.lua) | `/suggest`, `/report`, `/feedback`, `/apply`, `/bugreport` | None |
| [`welcome`](../../scripts/welcome/module.lua) | None | `guild_member_add`, `guild_member_remove` |
| [`join_guard`](../../scripts/join_guard/module.lua) | None | `guild_member_add` |
| [`roles`](../../scripts/roles/module.lua) | `/role` | None |
| [`fun`](../../scripts/fun/module.lua) | `/meme`, `/roll`, `/choose`, `/eightball` | None |
| [`games`](../../scripts/games/module.lua) | `/coinflip`, `/dice`, `/rps`, `/rate`, `/ship`, `/teams`, `/duel`, `/lootdrop` | None |
| [`utility`](../../scripts/utility/module.lua) | `/calculate`, `/convert`, `/snowflake`, `/timestamp`, `/randomnumber`, `/texttool`, `/ids`, `/color` | None |
| [`staff_tools`](../../scripts/staff_tools/module.lua) | `/announce`, `/broadcast`, `/staffalert`, `/maintenance`, `/rulespost`, `/auditnote` | None |
| [`server_info`](../../scripts/server_info/module.lua) | `/rules`, `/faq`, `/links`, `/supportinfo`, `/staff`, `/schedule`, `/aboutserver` | None |
| [`economy`](../../scripts/economy/module.lua) | `/economy` | None |
| [`giveaway`](../../scripts/giveaway/module.lua) | `/giveaway` | None |
| [`starboard`](../../scripts/starboard/module.lua) | `/starboard` | None |
| [`afk`](../../scripts/afk/module.lua) | `/afk` | `message_create` |
| [`tags`](../../scripts/tags/module.lua) | `/tag` | None |
| [`birthdays`](../../scripts/birthdays/module.lua) | `/birthday` | None |
| [`logging`](../../scripts/logging/module.lua) | `/logging` | None |
| [`reaction_roles`](../../scripts/reaction_roles/module.lua) | `/reactionrole` | None |
| [`quotes`](../../scripts/quotes/module.lua) | `/quote` | None |
| [`achievements`](../../scripts/achievements/module.lua) | `/achievements` | None |
| [`integrations_core`](../../scripts/integrations_core/module.lua) | `/integrations` | None |
| [`unified_identity`](../../scripts/unified_identity/module.lua) | `/identity` | None |
| [`steam_extended`](../../scripts/steam_extended/module.lua) | `/steam` | None |
| [`github_devops`](../../scripts/github_devops/module.lua) | `/github` | None |
| [`twitch_extended`](../../scripts/twitch_extended/module.lua) | `/twitch` | None |
| [`kick_live`](../../scripts/kick_live/module.lua) | `/kick` | None |
| [`youtube_extended`](../../scripts/youtube_extended/module.lua) | `/youtube` | None |
| [`telegram_bridge`](../../scripts/telegram_bridge/module.lua) | `/telegram` | None |
| [`workflow_builder`](../../scripts/workflow_builder/module.lua) | `/workflow` | None |
| [`gitlab_devops`](../../scripts/gitlab_devops/module.lua) | `/gitlab` | None |
| [`vk_community`](../../scripts/vk_community/module.lua) | `/vk` | None |
| [`reddit_feed`](../../scripts/reddit_feed/module.lua) | `/reddit` | None |
| [`gaming_hub`](../../scripts/gaming_hub/module.lua) | `/games` | None |
| [`social_publisher`](../../scripts/social_publisher/module.lua) | `/publish` | None |
| [`ai_mcp`](../../scripts/ai_mcp/module.lua) | `/ai`, `/mcp` | None |
| [`plugin_marketplace`](../../scripts/plugin_marketplace/module.lua) | `/plugin` | None |
