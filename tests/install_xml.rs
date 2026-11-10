use rusty_discord_bot::lua::{LuaEngine, LuaEngineError, LuaLimits};
use tempfile::TempDir;

fn limits() -> LuaLimits {
    LuaLimits {
        memory_bytes: 2 * 1024 * 1024,
        instruction_limit: 100_000,
        hook_granularity: 100,
    }
}

#[tokio::test]
async fn install_manifest_disables_modules_and_loads_config() {
    let temp = TempDir::new().expect("temp dir");
    let base = temp.path();

    // Write install.xml
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<installer version="1.0.0">
    <bot name="rusty-discord-bot" min_engine_version="0.2.0" />
    <modules>
        <module id="mod_active" enabled="true" version="1.0.0">
            <name>Active Module</name>
            <description>This module is enabled</description>
            <folder>active</folder>
            <entry>module.lua</entry>
            <config>config.json</config>
        </module>
        <module id="mod_inactive" enabled="false" version="1.0.0">
            <name>Inactive Module</name>
            <description>This module is disabled</description>
            <folder>inactive</folder>
            <entry>module.lua</entry>
            <config>config.json</config>
        </module>
    </modules>
</installer>"#;
    std::fs::write(base.join("install.xml"), xml).expect("write install.xml");

    // Create folders
    let active_dir = base.join("active");
    let inactive_dir = base.join("inactive");
    std::fs::create_dir_all(&active_dir).expect("create active dir");
    std::fs::create_dir_all(&inactive_dir).expect("create inactive dir");

    // Write module.lua for active
    std::fs::write(
        active_dir.join("module.lua"),
        r#"
        return {
            manifest = {
                id = "mod_active",
                name = "Active Module",
                version = "1.0.0",
                description = "This module is enabled",
                category = "test",
                commands = {
                    { name = "active_cmd", description = "Test active cmd" }
                }
            },
            on_command = function()
                return { { type = "reply", content = "Active works!" } }
            end
        }
        "#,
    )
    .expect("write active module.lua");

    // Write config.json for active
    std::fs::write(
        active_dir.join("config.json"),
        r#"{"custom_threshold": 42, "welcome_note": "hello world"}"#,
    )
    .expect("write active config.json");

    // Write module.lua for inactive
    std::fs::write(
        inactive_dir.join("module.lua"),
        r#"
        return {
            manifest = {
                id = "mod_inactive",
                name = "Inactive Module",
                version = "1.0.0",
                description = "This module is disabled",
                category = "test",
                commands = {
                    { name = "inactive_cmd", description = "Test inactive cmd" }
                }
            },
            on_command = function()
                return {}
            end
        }
        "#,
    )
    .expect("write inactive module.lua");

    // Load LuaEngine
    let engine = LuaEngine::load(base, limits()).expect("engine should load with install.xml");

    // Check manifest
    let installer = engine
        .installer_manifest()
        .await
        .expect("installer manifest should be populated");
    assert_eq!(installer.version, "1.0.0");
    assert_eq!(installer.bot_name.as_deref(), Some("rusty-discord-bot"));

    // Verify active module is loaded
    assert!(engine.manifest("mod_active").await.is_some());
    assert_eq!(
        engine.module_for_command("active_cmd").await.as_deref(),
        Some("mod_active")
    );

    // Verify inactive module was SKIPPED per install.xml
    assert!(engine.manifest("mod_inactive").await.is_none());
    assert!(engine.module_for_command("inactive_cmd").await.is_none());
    assert!(!engine.is_module_enabled_in_installer("mod_inactive").await);

    // Verify default_config was loaded from config.json
    let cfg = engine.default_config("mod_active").await;
    assert_eq!(cfg["custom_threshold"], 42);
    assert_eq!(cfg["welcome_note"], "hello world");
}

#[tokio::test]
async fn test_install_xml_rejects_module_id_spoofing() {
    let temp = TempDir::new().expect("temp dir");
    let base = temp.path();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<installer version="1.0.0">
    <modules>
        <module id="community_helper" enabled="true">
            <folder>helper</folder>
            <entry>module.lua</entry>
        </module>
    </modules>
</installer>"#;
    std::fs::write(base.join("install.xml"), xml).expect("write install.xml");

    let helper_dir = base.join("helper");
    std::fs::create_dir_all(&helper_dir).expect("create helper dir");

    // Malicious module claims to be "tickets" inside Lua manifest, while install.xml calls it "community_helper"
    std::fs::write(
        helper_dir.join("module.lua"),
        r#"
        return {
            manifest = {
                id = "tickets",
                name = "Spoofed Tickets",
                version = "1.0.0",
                description = "Attempting to spoof reserved tickets module",
                category = "moderation",
                commands = {
                    { name = "ticket", description = "Spoofed ticket command" }
                }
            },
            on_command = function()
                return {}
            end
        }
        "#,
    )
    .expect("write spoofed module.lua");

    let res = LuaEngine::load(base, limits());
    assert!(res.is_err(), "engine must reject mismatched module id");
    match res.err().unwrap() {
        LuaEngineError::InvalidModule { message, .. } => {
            assert!(
                message.contains("differs from Lua manifest id"),
                "expected error message about id mismatch, got: {message}"
            );
        }
        other => panic!("expected InvalidModule error, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_install_xml_rejects_path_traversal_and_symlink_escape() {
    let temp = TempDir::new().expect("temp dir");
    let base = temp.path();

    // 1. Direct path traversal in install.xml config
    let traversal_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<installer version="1.0.0">
    <modules>
        <module id="demo" enabled="true">
            <folder>demo</folder>
            <entry>module.lua</entry>
            <config>../../private/secret.json</config>
        </module>
    </modules>
</installer>"#;
    std::fs::write(base.join("install.xml"), traversal_xml).expect("write install.xml");

    let res = LuaEngine::load(base, limits());
    assert!(res.is_err(), "must reject traversal in config");
    match res.err().unwrap() {
        LuaEngineError::InvalidInstallXml { message, .. } => {
            assert!(
                message.contains("parent traversal"),
                "expected traversal error message, got: {message}"
            );
        }
        other => panic!("expected InvalidInstallXml error, got: {other:?}"),
    }

    // 2. Symlink escape detection
    let demo_dir = base.join("demo");
    std::fs::create_dir_all(&demo_dir).expect("create demo dir");
    std::fs::write(
        demo_dir.join("module.lua"),
        r#"
        return {
            manifest = {
                id = "demo",
                name = "Demo",
                version = "1.0.0",
                description = "Demo",
                category = "test",
                commands = {}
            }
        }
        "#,
    )
    .expect("write module.lua");

    // Create private dir outside scripts
    let private_dir = temp
        .path()
        .parent()
        .unwrap()
        .join(format!("private_{}", uuid::Uuid::new_v4().simple()));
    let _ = std::fs::create_dir_all(&private_dir);
    let secret_file = private_dir.join("secret.json");
    std::fs::write(&secret_file, r#"{"leak": true}"#).expect("write secret");

    // Write valid install.xml with config name "escaped.json"
    let symlink_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<installer version="1.0.0">
    <modules>
        <module id="demo" enabled="true">
            <folder>demo</folder>
            <entry>module.lua</entry>
            <config>escaped.json</config>
        </module>
    </modules>
</installer>"#;
    std::fs::write(base.join("install.xml"), symlink_xml).expect("write install.xml");

    // Attempt to create symlink if OS permits (on Windows symlinks may require developer mode or elevation)
    #[cfg(windows)]
    let symlink_created =
        std::os::windows::fs::symlink_file(&secret_file, demo_dir.join("escaped.json")).is_ok();
    #[cfg(unix)]
    let symlink_created =
        std::os::unix::fs::symlink(&secret_file, demo_dir.join("escaped.json")).is_ok();

    if symlink_created {
        let sym_res = LuaEngine::load(base, limits());
        assert!(sym_res.is_err(), "must reject symlink escaping module dir");
        match sym_res.err().unwrap() {
            LuaEngineError::InvalidModule { message, .. } => {
                assert!(
                    message.contains("escapes"),
                    "expected escape message, got: {message}"
                );
            }
            other => panic!("expected InvalidModule error, got: {other:?}"),
        }
    }

    let _ = std::fs::remove_dir_all(&private_dir);
}

#[tokio::test]
async fn test_install_xml_rejects_multiple_roots_and_unclosed_trailing_fragments() {
    let temp = TempDir::new().expect("temp dir");
    let base = temp.path();

    // 1. Multiple roots
    let bad_xml = r#"<installer version="1.0.0"/><other/>"#;
    std::fs::write(base.join("install.xml"), bad_xml).expect("write install.xml");

    let res = LuaEngine::load(base, limits());
    assert!(
        res.is_err(),
        "engine must reject multiple root elements in install.xml"
    );
    match res.err().unwrap() {
        LuaEngineError::InvalidInstallXml { message, .. } => {
            assert!(
                message.contains("multiple root elements"),
                "expected multiple root elements error, got: {message}"
            );
        }
        other => panic!("expected InvalidInstallXml error, got: {other:?}"),
    }

    // 2. Trailing unclosed fragment
    let unclosed_xml = r#"<installer/><broken>"#;
    std::fs::write(base.join("install.xml"), unclosed_xml).expect("write install.xml");

    let res2 = LuaEngine::load(base, limits());
    assert!(
        res2.is_err(),
        "engine must reject trailing unclosed fragment in install.xml"
    );
    match res2.err().unwrap() {
        LuaEngineError::InvalidInstallXml { message, .. } => {
            assert!(
                message.contains("multiple root elements") || message.contains("unclosed"),
                "expected parse error, got: {message}"
            );
        }
        other => panic!("expected InvalidInstallXml error, got: {other:?}"),
    }

    // 3. Trailing text after root
    let text_xml = r#"<installer/>unexpected-text"#;
    std::fs::write(base.join("install.xml"), text_xml).expect("write install.xml");

    let res3 = LuaEngine::load(base, limits());
    assert!(
        res3.is_err(),
        "engine must reject unexpected trailing text in install.xml"
    );
    match res3.err().unwrap() {
        LuaEngineError::InvalidInstallXml { message, .. } => {
            assert!(
                message.contains("text content outside root element"),
                "expected trailing text error, got: {message}"
            );
        }
        other => panic!("expected InvalidInstallXml error, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_legacy_loader_rejects_symlinks_and_loops_without_install_xml() {
    let temp = TempDir::new().expect("temp dir");
    let base = temp.path();

    // 1. Normal nested script works in legacy mode (no install.xml)
    let nested_dir = base.join("nested");
    std::fs::create_dir_all(&nested_dir).expect("create nested dir");
    std::fs::write(
        nested_dir.join("sample.lua"),
        r#"
        return {
            manifest = {
                id = "sample",
                name = "Sample",
                version = "1.0.0",
                description = "Sample",
                category = "test",
                commands = {}
            }
        }
        "#,
    )
    .expect("write sample.lua");

    let engine =
        LuaEngine::load(base, limits()).expect("legacy loader should load regular scripts");
    assert!(
        engine.manifest("sample").await.is_some(),
        "sample module should be loaded"
    );

    // 2. Symlinks outside SCRIPTS_DIR are rejected
    let private_dir = temp
        .path()
        .parent()
        .unwrap()
        .join(format!("legacy_outside_{}", uuid::Uuid::new_v4().simple()));
    let _ = std::fs::create_dir_all(&private_dir);
    let outside_script = private_dir.join("outside.lua");
    std::fs::write(
        &outside_script,
        r#"
        return {
            manifest = {
                id = "outside",
                name = "Outside",
                version = "1.0.0",
                description = "Outside",
                category = "test",
                commands = {}
            }
        }
        "#,
    )
    .expect("write outside.lua");

    #[cfg(windows)]
    let sym_file_ok =
        std::os::windows::fs::symlink_file(&outside_script, base.join("sym_file.lua")).is_ok();
    #[cfg(unix)]
    let sym_file_ok =
        std::os::unix::fs::symlink(&outside_script, base.join("sym_file.lua")).is_ok();

    if sym_file_ok {
        let res = engine.reload().await;
        assert!(res.is_err(), "legacy loader must reject symlinked file");
        // Active registry must be preserved
        assert!(
            engine.manifest("sample").await.is_some(),
            "active registry must be preserved"
        );
        let _ = std::fs::remove_file(base.join("sym_file.lua"));
    }

    // 3. Symlink directory loop: loop -> .
    #[cfg(windows)]
    let sym_dir_ok = std::os::windows::fs::symlink_dir(base, base.join("loop")).is_ok();
    #[cfg(unix)]
    let sym_dir_ok = std::os::unix::fs::symlink(base, base.join("loop")).is_ok();

    if sym_dir_ok {
        let res = engine.reload().await;
        assert!(
            res.is_err(),
            "legacy loader must reject symlink directory loop"
        );
        // Active registry must still be preserved
        assert!(
            engine.manifest("sample").await.is_some(),
            "active registry must be preserved"
        );
        let _ = std::fs::remove_dir(base.join("loop"));
    }

    let _ = std::fs::remove_dir_all(&private_dir);
}

#[tokio::test]
async fn test_install_xml_oversized_file_rejected_issue_336() {
    let temp = TempDir::new().expect("temp dir");
    let base = temp.path();

    // 1. Valid initial install.xml
    let valid_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<installer version="1.0.0">
    <modules>
        <module id="mod_base" enabled="true">
            <folder>base</folder>
            <entry>module.lua</entry>
        </module>
    </modules>
</installer>"#;
    std::fs::write(base.join("install.xml"), valid_xml).expect("write valid install.xml");

    let base_mod_dir = base.join("base");
    std::fs::create_dir_all(&base_mod_dir).expect("create base mod dir");
    std::fs::write(
        base_mod_dir.join("module.lua"),
        r#"return { manifest = { id = "mod_base", name = "Base", version = "1.0.0", description = "Base", category = "test", commands = {} } }"#,
    )
    .expect("write base module.lua");

    let engine = LuaEngine::load(base, limits()).expect("load engine");
    assert!(engine.manifest("mod_base").await.is_some());

    // 2. Overwrite install.xml with file exceeding 512 KiB
    let mut oversized = String::from("<installer>");
    oversized.push_str(&" ".repeat(512 * 1024 + 10));
    oversized.push_str("</installer>");
    std::fs::write(base.join("install.xml"), oversized).expect("write oversized install.xml");

    let reload_res = engine.reload().await;
    assert!(
        reload_res.is_err(),
        "reload must fail on oversized install.xml"
    );
    // Verify previous registry is safely preserved
    assert!(
        engine.manifest("mod_base").await.is_some(),
        "active registry must be preserved after reload failure"
    );
}

#[tokio::test]
async fn test_lua_module_source_oversized_rejected_issue_343() {
    let temp = TempDir::new().expect("temp dir");
    let base = temp.path();

    let valid_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<installer version="1.0.0">
    <modules>
        <module id="mod_big" enabled="true">
            <folder>big</folder>
            <entry>module.lua</entry>
        </module>
    </modules>
</installer>"#;
    std::fs::write(base.join("install.xml"), valid_xml).expect("write valid install.xml");

    let big_mod_dir = base.join("big");
    std::fs::create_dir_all(&big_mod_dir).expect("create big mod dir");

    // Write module.lua exceeding 1 MiB (e.g. 1 MiB + 10 bytes of comments)
    let mut oversized_lua = String::from(
        "return { manifest = { id = \"mod_big\", name = \"Big\", version = \"1.0.0\", description = \"Big\", category = \"test\", commands = {} } } --",
    );
    oversized_lua.push_str(&"-".repeat(1024 * 1024 + 10));
    std::fs::write(big_mod_dir.join("module.lua"), oversized_lua)
        .expect("write oversized module.lua");

    let load_res = LuaEngine::load(base, limits());
    assert!(
        load_res.is_err(),
        "engine load must fail when module source exceeds MAX_MODULE_SOURCE_BYTES"
    );
    match load_res.err().unwrap() {
        LuaEngineError::InvalidModule { message, .. } => {
            assert!(
                message.contains("exceeds 1048576 bytes limit"),
                "unexpected message: {message}"
            );
        }
        other => panic!("expected InvalidModule error, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_lua_config_json_oversized_rejected_issue_343() {
    let temp = TempDir::new().expect("temp dir");
    let base = temp.path();

    let valid_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<installer version="1.0.0">
    <modules>
        <module id="mod_cfg" enabled="true">
            <folder>cfg</folder>
            <entry>module.lua</entry>
            <config>config.json</config>
        </module>
    </modules>
</installer>"#;
    std::fs::write(base.join("install.xml"), valid_xml).expect("write valid install.xml");

    let cfg_mod_dir = base.join("cfg");
    std::fs::create_dir_all(&cfg_mod_dir).expect("create cfg mod dir");
    std::fs::write(
        cfg_mod_dir.join("module.lua"),
        r#"return { manifest = { id = "mod_cfg", name = "Cfg", version = "1.0.0", description = "Cfg", category = "test", commands = {} } }"#,
    )
    .expect("write module.lua");

    // Write config.json exceeding 256 KiB
    let mut oversized_json = String::from("{\"key\":\"");
    oversized_json.push_str(&"x".repeat(256 * 1024 + 20));
    oversized_json.push_str("\"}");
    std::fs::write(cfg_mod_dir.join("config.json"), oversized_json)
        .expect("write oversized config.json");

    let load_res = LuaEngine::load(base, limits());
    assert!(
        load_res.is_err(),
        "engine load must fail when config.json exceeds MAX_DEFAULT_CONFIG_BYTES"
    );
    match load_res.err().unwrap() {
        LuaEngineError::InvalidConfigSchema { message, .. } => {
            assert!(
                message.contains("exceeds 262144 bytes limit"),
                "unexpected message: {message}"
            );
        }
        other => panic!("expected InvalidConfigSchema error, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_install_xml_duplicate_attributes_rejected_issue_381() {
    let temp = TempDir::new().expect("temp dir");
    let base = temp.path();

    // 1. Initial valid setup
    let valid_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<installer version="1.0.0">
    <modules>
        <module id="valid_mod" enabled="true">
            <folder>mod</folder>
            <entry>module.lua</entry>
        </module>
    </modules>
</installer>"#;
    std::fs::write(base.join("install.xml"), valid_xml).expect("write valid install.xml");

    let mod_dir = base.join("mod");
    std::fs::create_dir_all(&mod_dir).expect("create mod dir");
    std::fs::write(
        mod_dir.join("module.lua"),
        r#"return { manifest = { id = "valid_mod", name = "Valid", version = "1.0.0", description = "Valid", category = "test", commands = {} } }"#,
    )
    .expect("write module.lua");

    let engine = LuaEngine::load(base, limits()).expect("initial engine load should succeed");
    assert!(engine.manifest("valid_mod").await.is_some());

    // 2. Overwrite install.xml with duplicate enabled attribute
    let malformed_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<installer version="1.0.0">
    <modules>
        <module id="valid_mod" enabled="false" enabled="true">
            <folder>mod</folder>
            <entry>module.lua</entry>
        </module>
    </modules>
</installer>"#;
    std::fs::write(base.join("install.xml"), malformed_xml).expect("write malformed install.xml");

    let reload_res = engine.reload().await;
    assert!(
        reload_res.is_err(),
        "reload must fail when install.xml contains duplicate attributes"
    );
    match reload_res.err().unwrap() {
        LuaEngineError::InvalidInstallXml { message, .. } => {
            assert!(
                message.contains("duplicate XML attribute 'enabled'"),
                "unexpected message: {message}"
            );
        }
        other => panic!("expected InvalidInstallXml error, got: {other:?}"),
    }

    // Previous registry must remain intact after failed reload
    assert!(
        engine.manifest("valid_mod").await.is_some(),
        "existing registry must be preserved after failed reload"
    );

    // 3. Cold start with duplicate attribute must fail fast
    let cold_load_res = LuaEngine::load(base, limits());
    assert!(
        cold_load_res.is_err(),
        "cold engine load must fail when install.xml contains duplicate attributes"
    );
}
