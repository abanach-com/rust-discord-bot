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

#[tokio::test]
async fn test_lua_engine_eviction_preserves_active_strong_refs_and_max_per_guild() {
    let scripts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts");
    let engine = LuaEngine::load(scripts, limits())
        .expect("scripts load")
        .with_concurrency_limits(4, 1, 1000); // max_per_guild = 1

    // 1. Acquire reference to target guild semaphore
    let target_guild = "target_guild_123";
    let old_sem = engine.get_guild_semaphore(target_guild);

    // 2. Populate 501 filler guilds to trigger eviction threshold (> 500)
    for i in 0..501 {
        let _ = engine.get_guild_semaphore(&format!("filler_guild_{i}"));
    }

    // 3. Request target guild semaphore again
    let new_sem = engine.get_guild_semaphore(target_guild);

    // Because old_sem was held with strong_count > 1, eviction must NOT discard it
    assert!(
        std::sync::Arc::ptr_eq(&old_sem, &new_sem),
        "Eviction must not create duplicate semaphore when strong references exist"
    );

    // 4. Concurrency limit of max_per_guild = 1 must strictly hold
    let permit1 = old_sem.try_acquire_owned();
    assert!(permit1.is_ok(), "First permit acquired");

    let permit2 = new_sem.try_acquire_owned();
    assert!(
        permit2.is_err(),
        "Second permit must be denied under max_per_guild=1"
    );
}

#[test]
fn test_lua_command_limit_accounts_for_host_commands_issue_250() {
    let dir_95 = TempDir::new().expect("tempdir");
    let mut commands_lua_95 = String::new();
    for i in 0..95 {
        commands_lua_95.push_str(&format!(
            r#"{{ name = "cmd_{i:02}", description = "Command number {i}" }},"#
        ));
    }
    std::fs::write(
        dir_95.path().join("module_95.lua"),
        format!(
            r#"
            return {{
                manifest = {{
                    id = "module_95",
                    name = "Module 95",
                    version = "1.0.0",
                    description = "95 commands module.",
                    category = "test",
                    commands = {{ {commands_lua_95} }}
                }},
                on_command = function() return {{}} end
            }}
            "#
        ),
    )
    .expect("write 95 module");

    let engine_95 = LuaEngine::load(dir_95.path(), limits());
    assert!(
        engine_95.is_ok(),
        "95 Lua commands must be accepted since 95 + 5 host = 100 max"
    );

    // 96 commands must fail startup (96 + 5 = 101 > 100)
    let dir_96 = TempDir::new().expect("tempdir");
    let mut commands_lua_96 = String::new();
    for i in 0..96 {
        commands_lua_96.push_str(&format!(
            r#"{{ name = "cmd_{i:02}", description = "Command number {i}" }},"#
        ));
    }
    std::fs::write(
        dir_96.path().join("module_96.lua"),
        format!(
            r#"
            return {{
                manifest = {{
                    id = "module_96",
                    name = "Module 96",
                    version = "1.0.0",
                    description = "96 commands module.",
                    category = "test",
                    commands = {{ {commands_lua_96} }}
                }},
                on_command = function() return {{}} end
            }}
            "#
        ),
    )
    .expect("write 96 module");

    let err_96 = match LuaEngine::load(dir_96.path(), limits()) {
        Ok(_) => panic!("96 Lua commands must fail"),
        Err(err) => err,
    };
    let err_msg = err_96.to_string();
    assert!(
        err_msg.contains("exceeding limit of 95"),
        "error message must describe exceeding limit of 95: got {err_msg}"
    );
}

#[tokio::test]
async fn test_action_limit_25_accepted_and_26_rejected_issue_335() {
    let directory = TempDir::new().expect("tempdir");

    std::fs::write(
        directory.path().join("action_limits.lua"),
        r#"
        return {
            manifest = {
                id = "action_limits",
                name = "Action Limits",
                version = "1.0.0",
                description = "Tests 25 vs 26 actions.",
                category = "test",
                commands = {
                    { name = "exact_25", description = "Returns 25 actions." },
                    { name = "excess_26", description = "Returns 26 actions." }
                }
            },
            on_command = function(cmd)
                if cmd == "exact_25" then
                    local actions = {}
                    for i = 1, 25 do
                        actions[i] = { type = "reply", content = "reply " .. i, ephemeral = true }
                    end
                    return actions
                elseif cmd == "excess_26" then
                    local actions = {}
                    for i = 1, 26 do
                        actions[i] = { type = "reply", content = "reply " .. i, ephemeral = true }
                    end
                    return actions
                end
                return {}
            end
        }
        "#,
    )
    .expect("write test script");

    let engine = LuaEngine::load(directory.path(), limits()).expect("load engine");

    // 25 actions must succeed
    let res_25 = engine
        .execute_command("action_limits", "exact_25", context())
        .await;
    let actions = res_25.expect("25 actions must be accepted");
    assert_eq!(actions.len(), 25);
    for (idx, action) in actions.iter().enumerate() {
        assert!(
            matches!(action, LuaAction::Reply { content, ephemeral: true } if content == &format!("reply {}", idx + 1))
        );
    }

    // 26 actions must fail before execution with InvalidModule
    let res_26 = engine
        .execute_command("action_limits", "excess_26", context())
        .await;
    assert!(res_26.is_err());
    let err = res_26.expect_err("26 actions must fail");
    assert!(
        matches!(err, LuaEngineError::InvalidModule { ref message, .. } if message.contains("returned 26 actions; the limit is 25")),
        "expected InvalidModule with 26 actions limit error, got: {err}"
    );
}

#[tokio::test]
async fn test_action_limit_fast_preflight_for_huge_arrays_issue_335() {
    let directory = TempDir::new().expect("tempdir");

    std::fs::write(
        directory.path().join("huge_arrays.lua"),
        r#"
        return {
            manifest = {
                id = "huge_arrays",
                name = "Huge Arrays",
                version = "1.0.0",
                description = "Tests fast preflight rejection for large returned arrays.",
                category = "test",
                commands = {
                    { name = "array_1000", description = "Returns 1000 actions." },
                    { name = "array_20000", description = "Returns 20000 actions." }
                }
            },
            on_command = function(cmd)
                local shared = { type = "reply", content = "ok", ephemeral = true }
                local result = {}
                local count = (cmd == "array_1000") and 1000 or 20000
                for i = 1, count do
                    result[i] = shared
                end
                return result
            end
        }
        "#,
    )
    .expect("write test script");

    // Give sufficient Lua memory for the Lua VM itself to create the table
    let mut custom_limits = limits();
    custom_limits.memory_bytes = 16 * 1024 * 1024;
    custom_limits.instruction_limit = 1_000_000;

    let engine = LuaEngine::load(directory.path(), custom_limits).expect("load engine");

    let res_1000 = engine
        .execute_command("huge_arrays", "array_1000", context())
        .await;
    assert!(res_1000.is_err());
    let err_1000 = res_1000.expect_err("1000 actions must fail");
    assert!(
        matches!(err_1000, LuaEngineError::InvalidModule { ref message, .. } if message.contains("returned 1000 actions; the limit is 25")),
        "got: {err_1000}"
    );

    let res_20000 = engine
        .execute_command("huge_arrays", "array_20000", context())
        .await;
    assert!(res_20000.is_err());
    let err_20000 = res_20000.expect_err("20000 actions must fail");
    assert!(
        matches!(err_20000, LuaEngineError::InvalidModule { ref message, .. } if message.contains("returned 20000 actions; the limit is 25")),
        "got: {err_20000}"
    );
}

#[tokio::test]
async fn test_action_decoder_rejects_sparse_arrays_and_extreme_indices_issue_335() {
    let directory = TempDir::new().expect("tempdir");

    std::fs::write(
        directory.path().join("sparse_modules.lua"),
        r#"
        return {
            manifest = {
                id = "sparse_modules",
                name = "Sparse Modules",
                version = "1.0.0",
                description = "Tests rejection of sparse tables and out-of-bounds indices.",
                category = "test",
                commands = {
                    { name = "extreme_index", description = "Table with [1000000] index." },
                    { name = "gap_in_sequence", description = "Table with gap at index 2." },
                    { name = "negative_index", description = "Table with negative index." },
                    { name = "map_not_array", description = "Table with string keys." }
                }
            },
            on_command = function(cmd)
                if cmd == "extreme_index" then
                    local t = {}
                    t[1000000] = { type = "reply", content = "hi", ephemeral = true }
                    return t
                elseif cmd == "gap_in_sequence" then
                    local t = {}
                    t[1] = { type = "reply", content = "1", ephemeral = true }
                    t[3] = { type = "reply", content = "3", ephemeral = true }
                    return t
                elseif cmd == "negative_index" then
                    local t = {}
                    t[-1] = { type = "reply", content = "neg", ephemeral = true }
                    return t
                elseif cmd == "map_not_array" then
                    return { foo = "bar" }
                end
                return {}
            end
        }
        "#,
    )
    .expect("write test script");

    let engine = LuaEngine::load(directory.path(), limits()).expect("load engine");

    // Extreme index
    let err_extreme = engine
        .execute_command("sparse_modules", "extreme_index", context())
        .await
        .expect_err("extreme index must fail");
    assert!(
        matches!(err_extreme, LuaEngineError::InvalidModule { ref message, .. } if message.contains("out-of-bounds index 1000000")),
        "expected out-of-bounds index error, got: {err_extreme}"
    );

    // Gap in sequence
    let err_gap = engine
        .execute_command("sparse_modules", "gap_in_sequence", context())
        .await
        .expect_err("gap in sequence must fail");
    assert!(
        matches!(err_gap, LuaEngineError::InvalidModule { ref message, .. } if message.contains("sparse array")),
        "expected sparse array error, got: {err_gap}"
    );

    // Negative index
    let err_neg = engine
        .execute_command("sparse_modules", "negative_index", context())
        .await
        .expect_err("negative index must fail");
    assert!(
        matches!(err_neg, LuaEngineError::InvalidModule { ref message, .. } if message.contains("non-positive index -1")),
        "expected non-positive index error, got: {err_neg}"
    );

    // Map instead of array
    let err_map = engine
        .execute_command("sparse_modules", "map_not_array", context())
        .await
        .expect_err("map table must fail");
    assert!(
        matches!(err_map, LuaEngineError::InvalidModule { ref message, .. } if message.contains("non-integer keys")),
        "expected non-integer keys error, got: {err_map}"
    );
}

#[tokio::test]
async fn test_action_decoder_fails_early_on_invalid_element_issue_335() {
    let directory = TempDir::new().expect("tempdir");

    std::fs::write(
        directory.path().join("invalid_element.lua"),
        r#"
        return {
            manifest = {
                id = "invalid_element",
                name = "Invalid Element",
                version = "1.0.0",
                description = "Tests early failure on bad element types or shapes.",
                category = "test",
                commands = {
                    { name = "unknown_type", description = "Unknown action type." },
                    { name = "bad_audit_payload", description = "Audit action exceeding payload." },
                    { name = "non_table_return", description = "Returns integer instead of array." }
                }
            },
            on_command = function(cmd)
                if cmd == "unknown_type" then
                    return {
                        { type = "reply", content = "valid first", ephemeral = true },
                        { type = "unknown_action_type", foo = "bar" },
                        { type = "reply", content = "never reached", ephemeral = true }
                    }
                elseif cmd == "bad_audit_payload" then
                    local big_data = {}
                    for i = 1, 100 do
                        big_data["key_" .. i] = "val"
                    end
                    return {
                        { type = "audit", event = "excessive_audit", data = big_data }
                    }
                elseif cmd == "non_table_return" then
                    return 12345
                end
                return {}
            end
        }
        "#,
    )
    .expect("write test script");

    let engine = LuaEngine::load(directory.path(), limits()).expect("load engine");

    // Unknown action type fails during bounded element deserialization with Runtime error
    let err_unknown = engine
        .execute_command("invalid_element", "unknown_type", context())
        .await
        .expect_err("unknown action type must fail");
    assert!(
        matches!(err_unknown, LuaEngineError::Runtime { .. }),
        "expected Runtime error on unknown action variant, got: {err_unknown}"
    );

    // Bad audit payload (> 64 keys) fails during immediate element validation with InvalidModule
    let err_audit = engine
        .execute_command("invalid_element", "bad_audit_payload", context())
        .await
        .expect_err("excessive audit keys must fail");
    assert!(
        matches!(err_audit, LuaEngineError::InvalidModule { ref message, .. } if message.contains("audit data cannot contain more than 64 entries")),
        "expected InvalidModule on audit data limit, got: {err_audit}"
    );

    // Non-table return fails with Runtime error (canonical mlua deserialization error)
    let err_non_table = engine
        .execute_command("invalid_element", "non_table_return", context())
        .await
        .expect_err("non-table return must fail");
    assert!(
        matches!(err_non_table, LuaEngineError::Runtime { .. }),
        "expected Runtime error on non-table return, got: {err_non_table}"
    );
}
