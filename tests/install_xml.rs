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
