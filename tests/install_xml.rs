use rusty_discord_bot::lua::{LuaEngine, LuaLimits};
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
