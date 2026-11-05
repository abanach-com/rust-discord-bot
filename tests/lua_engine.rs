use std::path::PathBuf;

use rusty_discord_bot::lua::{
    LuaAction, LuaEngine, LuaEngineError, LuaEventContext, LuaExecutionContext, LuaLimits,
};
use serde_json::json;
use tempfile::TempDir;

fn limits() -> LuaLimits {
    LuaLimits {
        memory_bytes: 2 * 1024 * 1024,
        instruction_limit: 100_000,
        hook_granularity: 100,
    }
}

fn context() -> LuaExecutionContext {
    LuaExecutionContext {
        guild_id: Some("100000000000000001".to_owned()),
        channel_id: "100000000000000002".to_owned(),
        user_id: "100000000000000003".to_owned(),
        user_name: "Tester".to_owned(),
        member_roles: Vec::new(),
        member_permissions: "0".to_owned(),
        locale: "pl".to_owned(),
        options: json!({}),
        config: json!({}),
    }
}

#[tokio::test]
async fn bundled_modules_load_and_ping_executes() {
    let scripts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts");
    let engine = LuaEngine::load(scripts, limits()).expect("bundled scripts should load");

    let manifests = engine.manifests().await;
    assert!(manifests.len() >= 7);
    assert_eq!(
        engine.module_for_command("ping").await.as_deref(),
        Some("core")
    );

    let actions = engine
        .execute_command("core", "ping", context())
        .await
        .expect("ping should execute");

    assert!(matches!(
        actions.first(),
        Some(LuaAction::Reply {
            content: _,
            ephemeral: true
        })
    ));
}

#[tokio::test]
async fn infinite_loop_hits_instruction_limit() {
    let directory = TempDir::new().expect("temporary directory");
    std::fs::write(
        directory.path().join("loop.lua"),
        r#"
        return {
            manifest = {
                id = "loop",
                name = "Loop",
                version = "1.0.0",
                description = "Instruction limit test module.",
                category = "test",
                commands = {
                    { name = "loop", description = "Runs an endless loop." }
                }
            },
            on_command = function()
                while true do end
            end
        }
        "#,
    )
    .expect("write test module");

    let engine = LuaEngine::load(directory.path(), limits()).expect("module should load");
    let result = engine.execute_command("loop", "loop", context()).await;

    assert!(result.is_err(), "endless scripts must be interrupted");
    assert!(
        result
            .expect_err("expected instruction-limit failure")
            .to_string()
            .contains("instruction limit")
    );
}

#[tokio::test]
async fn dangerous_lua_globals_are_unavailable() {
    let directory = TempDir::new().expect("temporary directory");
    std::fs::write(
        directory.path().join("sandbox.lua"),
        r#"
        return {
            manifest = {
                id = "sandbox",
                name = "Sandbox",
                version = "1.0.0",
                description = "Sandbox global-denylist test module.",
                category = "test",
                commands = {
                    { name = "sandbox", description = "Checks denied Lua globals." }
                }
            },
            on_command = function()
                assert(collectgarbage == nil)
                assert(coroutine == nil)
                assert(debug == nil)
                assert(dofile == nil)
                assert(io == nil)
                assert(load == nil)
                assert(loadfile == nil)
                assert(os == nil)
                assert(package == nil)
                assert(pcall == nil)
                assert(require == nil)
                assert(xpcall == nil)
                return {}
            end
        }
        "#,
    )
    .expect("write sandbox test module");

    let engine = LuaEngine::load(directory.path(), limits()).expect("module should load");
    let actions = engine
        .execute_command("sandbox", "sandbox", context())
        .await
        .expect("sandbox globals should remain unavailable");

    assert!(actions.is_empty());
}

#[test]
fn duplicate_commands_are_rejected() {
    let directory = TempDir::new().expect("temporary directory");
    for (file, module_id) in [("one.lua", "one"), ("two.lua", "two")] {
        std::fs::write(
            directory.path().join(file),
            format!(
                r#"
                return {{
                    manifest = {{
                        id = "{module_id}",
                        name = "{module_id}",
                        version = "1.0.0",
                        description = "Duplicate command test module.",
                        category = "test",
                        commands = {{
                            {{ name = "same", description = "Duplicate command." }}
                        }}
                    }},
                    on_command = function() return {{}} end
                }}
                "#
            ),
        )
        .expect("write test module");
    }

    let result = LuaEngine::load(directory.path(), limits());
    assert!(
        result.is_err(),
        "duplicate slash commands must fail startup"
    );
}

fn event_context(guild_id: Option<&str>) -> LuaEventContext {
    LuaEventContext {
        name: "message_create".to_owned(),
        guild_id: guild_id.map(|s| s.to_owned()),
        channel_id: Some("100000000000000002".to_owned()),
        actor_id: Some("100000000000000003".to_owned()),
        data: json!({
            "message_id": "100000000000000004",
            "content": "hello world",
            "author": {
                "id": "100000000000000003",
                "name": "Tester",
                "global_name": null,
            },
            "mentions": [],
            "mention_roles": [],
            "mention_everyone": false,
            "attachments": [],
        }),
        config: json!({}),
    }
}

#[tokio::test]
async fn test_lua_engine_global_admission_control_and_metrics() {
    let scripts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts");
    let engine = LuaEngine::load(scripts, limits())
        .expect("scripts load")
        .with_concurrency_limits(2, 2, 50);

    assert_eq!(engine.max_parallel(), 2);
    assert_eq!(engine.max_per_guild(), 2);
    assert_eq!(engine.active_executions(), 0);

    // Acquire all 2 global permits manually to simulate saturation
    let p1 = engine
        .execution_slots()
        .try_acquire_owned()
        .expect("acquire slot 1");
    let p2 = engine
        .execution_slots()
        .try_acquire_owned()
        .expect("acquire slot 2");

    assert_eq!(engine.active_executions(), 2);

    // Event execution must be rejected immediately with Busy (fail-closed load shedding)
    let event_err = engine
        .execute_event(
            "automod",
            "message_create",
            event_context(Some("100000000000000001")),
        )
        .await;
    assert!(
        matches!(event_err, Err(LuaEngineError::Busy)),
        "Saturated global capacity must immediately reject event with Busy"
    );

    // Slash command execution must wait up to timeout and then return Timeout/Busy
    let cmd_err = engine.execute_command("core", "ping", context()).await;
    assert!(
        matches!(cmd_err, Err(LuaEngineError::Timeout)),
        "Saturated global capacity must timeout slash command acquisition"
    );

    // Release permits
    drop(p1);
    drop(p2);
    assert_eq!(engine.active_executions(), 0);

    // Now executions must succeed again
    let res = engine.execute_command("core", "ping", context()).await;
    assert!(
        res.is_ok(),
        "Command should succeed once capacity is restored"
    );
}

#[tokio::test]
async fn test_lua_engine_per_guild_isolation_prevents_starvation() {
    let scripts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts");
    let engine = LuaEngine::load(scripts, limits())
        .expect("scripts load")
        .with_concurrency_limits(4, 2, 50); // 4 global slots, but only 2 per guild

    let guild_g = "guild_g";
    let guild_h = "guild_h";

    // Guild G exhausts its per-guild quota (2 permits)
    let g_sem = engine.get_guild_semaphore(guild_g);
    let p1 = g_sem.clone().try_acquire_owned().expect("acquire g slot 1");
    let p2 = g_sem.try_acquire_owned().expect("acquire g slot 2");

    // Guild G event must be rejected because its guild quota is exhausted
    let res_g = engine
        .execute_event("automod", "message_create", event_context(Some(guild_g)))
        .await;
    assert!(
        matches!(res_g, Err(LuaEngineError::Busy)),
        "Guild G must be rejected when its per-guild quota is exhausted"
    );

    // But Guild H still has available quota and global capacity is not exhausted (2 of 4 slots free)
    let res_h = engine
        .execute_event("automod", "message_create", event_context(Some(guild_h)))
        .await;
    assert!(
        res_h.is_ok(),
        "Guild H must NOT be starved when Guild G exhausts its quota"
    );

    // Release Guild G permits
    drop(p1);
    drop(p2);

    let res_g_after = engine
        .execute_event("automod", "message_create", event_context(Some(guild_g)))
        .await;
    assert!(
        res_g_after.is_ok(),
        "Guild G must succeed once its quota frees up"
    );
}

#[tokio::test]
async fn test_lua_engine_cancellation_and_error_does_not_leak_permits() {
    let scripts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts");
    let engine = LuaEngine::load(scripts, limits())
        .expect("scripts load")
        .with_concurrency_limits(2, 2, 50);

    assert_eq!(engine.active_executions(), 0);

    // Call unknown command (error before spawn)
    let _ = engine
        .execute_command("core", "non_existent_command", context())
        .await;
    assert_eq!(engine.active_executions(), 0);

    // Cancelled future while waiting
    let p1 = engine.execution_slots().try_acquire_owned().unwrap();
    let p2 = engine.execution_slots().try_acquire_owned().unwrap();

    let engine_clone = engine.clone();
    let handle = tokio::spawn(async move {
        engine_clone
            .execute_command("core", "ping", context())
            .await
    });

    // Abort task while waiting
    handle.abort();
    let _ = handle.await;

    drop(p1);
    drop(p2);

    assert_eq!(engine.active_executions(), 0);
    assert_eq!(engine.execution_slots().available_permits(), 2);
}
