# 13. Test and troubleshoot

[Handbook index](README.md) · Previous: [Security and errors](security-errors.md) · Next: [Recipes](recipes.md)

Test the layer that implements the behavior. A successfully loaded manifest does not prove that an invoked branch returns a supported action or that its executor permits the operation.
Existing starting points are [tests/lua_engine.rs](../../tests/lua_engine.rs) and [tests/config_validation.rs](../../tests/config_validation.rs).

## Keep verification layers separate

| Check | What it establishes | What it does not establish |
| --- | --- | --- |
| Native Lua 5.4, with explicit helper stubs | Syntax, fixture branches and returned Lua values | Actual `mlua`/Serde decoding, Rust validation or Discord execution |
| Rust `LuaEngine` test | Real loading, sandbox helpers, typed action decoding and validation | Command registration, executor permissions, database writes or delivery |
| Service/HTTP/database integration test | The authorization and storage paths actually exercised | Acceptance by Discord or untested storage backends |
| Development Discord application and guild | Real registration, event inputs and delivery in that configuration | Correctness for every guild, role hierarchy or failure scenario |

## Existing repository commands

Run these from the repository root with the required toolchain and build dependencies. **These are instructions, not a record of commands executed or passing during documentation work.**

```sh
cargo test --all-features --test lua_engine
cargo test --all-features --test config_validation
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

The broader release checks are in [AGENTS.md](../../AGENTS.md) and the [version policy](../VERSION_POLICY.md). Report unavailable, skipped and failed checks separately; a green job that skipped tests is not a test result.

## A complete engine test for the documentation examples

The following proposed test can be saved as `tests/lua_documentation_examples.rs`. It uses the existing dependencies and checked-in [examples/lua](../../examples/lua/README.md), without temporary files or a Discord connection.

```rust
use std::path::PathBuf;
use rusty_discord_bot::lua::{
    LuaAction, LuaEngine, LuaEngineError, LuaExecutionContext, LuaLimits,
};
use serde_json::json;

#[tokio::test]
async fn handbook_examples_decode_actions_and_validate_configs() {
    const PERSON: &str = "900719925474099301";
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let engine = LuaEngine::load(
        root.join("examples/lua"),
        LuaLimits {
            memory_bytes: 4 * 1024 * 1024,
            instruction_limit: 200_000,
            hook_granularity: 100,
        },
    ).expect("example modules and schemas should load");
    for manifest in engine.manifests().await {
        let defaults = engine.default_config(&manifest.id).await;
        engine.validate_module_config(&manifest.id, &defaults)
            .await.expect("each example default must satisfy its schema");
    }
    assert!(engine.validate_module_config("lua_hello", &json!({"greeting": 7}))
        .await.is_err());
    let context = LuaExecutionContext {
        guild_id: Some("100000000000000001".to_owned()),
        channel_id: "100000000000000002".to_owned(),
        user_id: "100000000000000003".to_owned(),
        user_name: "Tester".to_owned(),
        member_roles: Vec::new(),
        member_permissions: "0".to_owned(),
        locale: "en".to_owned(),
        options: json!({"text": "Hello @everyone", "person": PERSON}),
        config: engine.default_config("lua_hello").await,
    };
    let actions = engine.execute_command("lua_hello", "lua-hello", context.clone())
        .await.expect("callback should decode and validate");
    match actions.as_slice() {
        [LuaAction::Reply { content, ephemeral }] => {
            assert!(*ephemeral);
            let expected = format!("Hello @\u{200b}everyone, Tester! Selected user ID: {PERSON}");
            assert_eq!(content, &expected);
        }
        other => panic!("unexpected typed actions: {other:?}"),
    }
    assert!(matches!(
        engine.execute_command("lua_hello", "unknown", context.clone()).await,
        Err(LuaEngineError::UnknownCommand { .. })
    ));
    assert!(matches!(
        engine.execute_command("missing", "lua-hello", context).await,
        Err(LuaEngineError::UnknownModule(_))
    ));
}
```

After adding that file, run `cargo test --all-features --test lua_documentation_examples`. The snippet is proposed code; it has not been compiled or executed as part of this guide. Matching `LuaAction::Reply` checks the real engine's typed result, not merely the shape of a Lua table.

Validate **both package defaults and complete candidate guild configurations**. Repeat the default-validation loop with a separate engine loading `scripts` when checking production packages; [#128](https://github.com/abanach-com/rust-discord-bot/issues/128) records invalid defaults at the audited snapshot. Schema compilation and a discarded result from validating `{}` do not establish that the actual default JSON is valid. Saved guild configuration replaces defaults rather than deep-merging them.

## Extend fixtures using actual context shapes

| Case | Input / assertion |
| --- | --- |
| Ordinary options | Omit `text` to test the configured greeting; omit `person` to test the optional branch |
| Direct subcommand | `lua_router` / `lua-route`, `options={"echo":{"text":"Hello"}}`; assert one typed reply containing `Hello` |
| Subcommand group | `options={"inspect":{"user":{"person":"900719925474099301"}}}`; assert the entire ID survives unchanged |
| Context types | Keep guild/user/channel/role IDs and permission bits as strings; test missing optional event IDs and `data.author={}` |
| Configuration | Valid replacement, wrong type, missing required property and unknown property; assert validation results instead of ignoring them |
| Event example | `record_events=false` produces no actions; `true` produces the expected typed audit action with bounded metadata |
| Host errors | Unknown module/command/event produces the appropriate `LuaEngineError`; an invalid action inside a list fails before execution |

The engine rejects an undeclared command before calling Lua. Calling an example's callback directly with an unknown name can instead return `{}`; those are different tests. Use dense one-based action arrays and test actual Rust decoding rather than assuming every malformed table must produce an error. Existing engine tests cover instruction exhaustion, denied globals and duplicate commands.

## Diagnose the failing stage

| Symptom | First check |
| --- | --- |
| Module cannot load | Installer root/paths, returned module table, manifest fields, duplicate identifiers and schema compilation |
| Subcommand takes the wrong branch | [#126](https://github.com/abanach-com/rust-discord-bot/issues/126): use the nested input from [contexts](contexts-events.md), not `options.subcommand` |
| Module loads but saving its defaults fails | [#128](https://github.com/abanach-com/rust-discord-bot/issues/128): validate the actual `config.json` against its manifest schema |
| Progression request fails decoding or execution | [#129](https://github.com/abanach-com/rust-discord-bot/issues/129): supported operation names/fields and the reserved `progression` module ID are separate requirements |
| Command acknowledges without its intended effect | Exported callback, selected branch and typed action list; an acknowledgement is not proof of a later action's success |
| Valid action is denied or delivery fails | Actor/app permissions, guild/channel scope, hierarchy, service identity and live API response; see [actions](actions.md) |

## Finish with the relevant integration checks

Use a development application and `DISCORD_DEV_GUILD_ID`; `SCRIPTS_DIR=examples/lua cargo run --release` selects the examples and can replace that application's commands in the chosen scope. Exercise both router paths, an optional input and the explicitly enabled audit event. This is a live smoke-test instruction, not an offline engine test.
For durable behavior, test the configured storage backend, migrations, restart and failure/retry path separately. A memory-store test cannot establish PostgreSQL correctness or persistence.
Record the source revision, actual command, fixture and outcome. State whether helper stubs, Rust, a database and Discord were used; keep credentials out of diagnostic contexts.
