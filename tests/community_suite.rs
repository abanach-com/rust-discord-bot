use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use rusty_discord_bot::lua::{
    LuaAction, LuaEngine, LuaEventContext, LuaExecutionContext, LuaLimits, MusicOperation,
    ProgressionOperation,
};
use serde_json::json;

fn limits() -> LuaLimits {
    LuaLimits {
        memory_bytes: 4 * 1024 * 1024,
        instruction_limit: 200_000,
        hook_granularity: 100,
    }
}

fn engine() -> LuaEngine {
    let scripts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts");
    LuaEngine::load(scripts, limits()).expect("the complete bundled module suite should load")
}

fn context(options: serde_json::Value, config: serde_json::Value) -> LuaExecutionContext {
    LuaExecutionContext {
        guild_id: Some("100000000000000001".to_owned()),
        channel_id: "100000000000000002".to_owned(),
        user_id: "100000000000000003".to_owned(),
        user_name: "Tester".to_owned(),
        member_roles: Vec::new(),
        member_permissions: "0".to_owned(),
        locale: "pl".to_owned(),
        options,
        config,
    }
}

#[tokio::test]
async fn bundled_suite_registers_expected_modules_and_commands() {
    let engine = engine();
    let manifests = engine.manifests().await;
    let commands = engine.command_definitions().await;

    assert!(manifests.len() >= 13, "expected the expanded module suite");
    assert!(
        commands.len() >= 43,
        "expected the expanded command catalog"
    );

    for def in &commands {
        assert!(
            def.validate_options_order().is_ok(),
            "bundled command /{} has invalid option ordering: {:?}",
            def.name,
            def.validate_options_order().err()
        );
    }

    for (command, module_id) in [
        ("calculate", "utility"),
        ("coinflip", "games"),
        ("suggest", "community"),
        ("announce", "staff_tools"),
        ("rules", "server_info"),
    ] {
        assert_eq!(
            engine.module_for_command(command).await.as_deref(),
            Some(module_id),
            "command /{command} should be owned by {module_id}"
        );
    }
}

#[tokio::test]
async fn calculator_executes_inside_the_lua_sandbox() {
    let actions = engine()
        .execute_command(
            "utility",
            "calculate",
            context(
                json!({ "a": 6, "operation": "multiply", "b": 7 }),
                json!({}),
            ),
        )
        .await
        .expect("calculator command should execute");

    assert!(matches!(
        actions.first(),
        Some(LuaAction::Reply {
            content,
            ephemeral: false,
        }) if content.contains("42")
    ));
}

#[tokio::test]
async fn report_routes_only_to_the_configured_staff_channel() {
    let actions = engine()
        .execute_command(
            "community",
            "report",
            context(
                json!({
                    "user": "100000000000000004",
                    "reason": "Testowe zgłoszenie naruszenia regulaminu",
                    "evidence": "100000000000000005"
                }),
                json!({ "report_channel_id": "100000000000000006" }),
            ),
        )
        .await
        .expect("report command should execute");

    assert_eq!(actions.len(), 1);
    assert!(matches!(
        &actions[0],
        LuaAction::PublishSubmission {
            channel_id,
            content,
            success_reply,
            audit_event,
            ..
        } if channel_id == "100000000000000006"
            && success_reply.contains("Report has been safely delivered")
            && audit_event == "user_report_created"
            && content.contains("Private Report")
    ));
}

#[tokio::test]
async fn staff_tools_reject_users_without_permissions() {
    let actions = engine()
        .execute_command(
            "staff_tools",
            "announce",
            context(
                json!({
                    "channel": "100000000000000006",
                    "title": "Test",
                    "message": "Treść"
                }),
                json!({}),
            ),
        )
        .await
        .expect("permission denial should be a normal Lua response");

    assert_eq!(actions.len(), 1);
    assert!(matches!(
        actions.first(),
        Some(LuaAction::Reply {
            content,
            ephemeral: true,
        }) if content.contains("uprawnień") || content.contains("permission")
    ));
}

#[tokio::test]
async fn automod_detects_case_insensitive_blocked_phrases() {
    let event = LuaEventContext {
        name: "message_create".to_owned(),
        guild_id: Some("100000000000000001".to_owned()),
        channel_id: Some("100000000000000002".to_owned()),
        actor_id: Some("100000000000000003".to_owned()),
        data: json!({
            "message_id": "100000000000000004",
            "content": "To zawiera ZABLOKOWANA FRAZA w środku",
            "mentions": [],
            "attachments": []
        }),
        config: json!({
            "blocked_words": ["zablokowana fraza"],
            "delete_messages": true,
            "send_notice": false
        }),
    };

    let actions = engine()
        .execute_event("automod", "message_create", event)
        .await
        .expect("automod event should execute");

    assert!(matches!(
        actions.first(),
        Some(LuaAction::DeleteMessage { .. })
    ));
    assert!(matches!(actions.last(), Some(LuaAction::Audit { .. })));
}

#[tokio::test]
async fn join_guard_flags_a_fresh_discord_account() {
    const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_millis() as u64;
    let user_id = ((now_ms - DISCORD_EPOCH_MS) << 22).to_string();

    let event = LuaEventContext {
        name: "guild_member_add".to_owned(),
        guild_id: Some("100000000000000001".to_owned()),
        channel_id: None,
        actor_id: Some(user_id.clone()),
        data: json!({
            "user": {
                "id": user_id,
                "name": "FreshAccount",
                "global_name": "Fresh Account",
                "bot": false
            },
            "roles": [],
            "joined_at": null
        }),
        config: json!({
            "alert_channel_id": "100000000000000006",
            "minimum_account_age_days": 7
        }),
    };

    let actions = engine()
        .execute_event("join_guard", "guild_member_add", event)
        .await
        .expect("join guard event should execute");

    assert!(matches!(
        actions.first(),
        Some(LuaAction::SendMessage {
            channel_id: Some(channel_id),
            content: _,
        }) if channel_id == "100000000000000006"
    ));
    assert!(matches!(actions.last(), Some(LuaAction::Audit { .. })));
}

#[tokio::test]
async fn automod_intercepts_message_update_with_discord_invite() {
    let event = LuaEventContext {
        name: "message_update".to_owned(),
        guild_id: Some("100000000000000001".to_owned()),
        channel_id: Some("100000000000000002".to_owned()),
        actor_id: Some("100000000000000003".to_owned()),
        data: json!({
            "message_id": "100000000000000004",
            "content": "Hej, wpadajcie do nas: dsc.gg/zucker-community",
            "mentions": [],
            "attachments": [],
            "is_edit": true
        }),
        config: json!({
            "block_invites": true,
            "delete_messages": true,
            "send_notice": true,
            "alert_channel_id": "100000000000000006"
        }),
    };

    let actions = engine()
        .execute_event("automod", "message_update", event)
        .await
        .expect("automod message_update event should execute");

    assert!(matches!(
        actions.first(),
        Some(LuaAction::DeleteMessage { .. })
    ));
    assert!(actions.iter().any(|action| matches!(
        action,
        LuaAction::SendMessage { content, .. } if content.contains("edycji wiadomości") || content.contains("message edit")
    )));
    assert!(matches!(
        actions.last(),
        Some(LuaAction::Audit { event, .. }) if event == "automod_edit_rule_match"
    ));
}

#[tokio::test]
async fn automod_detects_naked_domains_and_userinfo_urls() {
    let engine = engine();

    // 1. Naked domain
    let event1 = LuaEventContext {
        name: "message_create".to_owned(),
        guild_id: Some("100000000000000001".to_owned()),
        channel_id: Some("100000000000000002".to_owned()),
        actor_id: Some("100000000000000003".to_owned()),
        data: json!({
            "message_id": "100000000000000010",
            "content": "Check out grabify.link for free nitro!",
            "attachments": []
        }),
        config: json!({
            "block_suspicious_links": true,
            "delete_messages": true
        }),
    };
    let actions1 = engine
        .execute_event("automod", "message_create", event1)
        .await
        .expect("automod event should execute");
    assert!(
        matches!(actions1.first(), Some(LuaAction::DeleteMessage { .. })),
        "expected delete message for naked suspicious domain"
    );

    // 2. URL with userinfo: https://user:pass@grabify.link/path
    let event2 = LuaEventContext {
        name: "message_create".to_owned(),
        guild_id: Some("100000000000000001".to_owned()),
        channel_id: Some("100000000000000002".to_owned()),
        actor_id: Some("100000000000000003".to_owned()),
        data: json!({
            "message_id": "100000000000000011",
            "content": "Click https://admin:secret@grabify.link/track here",
            "attachments": []
        }),
        config: json!({
            "block_suspicious_links": true,
            "delete_messages": true
        }),
    };
    let actions2 = engine
        .execute_event("automod", "message_create", event2)
        .await
        .expect("automod event should execute");
    assert!(
        matches!(actions2.first(), Some(LuaAction::DeleteMessage { .. })),
        "expected delete message for url with userinfo"
    );

    // 3. Naked domain with path: bit.ly/offer
    let event3 = LuaEventContext {
        name: "message_create".to_owned(),
        guild_id: Some("100000000000000001".to_owned()),
        channel_id: Some("100000000000000002".to_owned()),
        actor_id: Some("100000000000000003".to_owned()),
        data: json!({
            "message_id": "100000000000000012",
            "content": "Check this out bit.ly/offer right now",
            "attachments": []
        }),
        config: json!({
            "block_suspicious_links": true,
            "delete_messages": true
        }),
    };
    let actions3 = engine
        .execute_event("automod", "message_create", event3)
        .await
        .expect("automod event should execute");
    assert!(
        matches!(actions3.first(), Some(LuaAction::DeleteMessage { .. })),
        "expected delete message for naked domain with path bit.ly/offer"
    );

    // 4. Userinfo trick: https://trusted.example@evil.example/path
    // Whitelist includes trusted.example, blacklist includes evil.example.
    // The actual destination is evil.example, so it MUST be blocked!
    let event4 = LuaEventContext {
        name: "message_create".to_owned(),
        guild_id: Some("100000000000000001".to_owned()),
        channel_id: Some("100000000000000002".to_owned()),
        actor_id: Some("100000000000000003".to_owned()),
        data: json!({
            "message_id": "100000000000000013",
            "content": "Visit https://trusted.example@evil.example/path for account verification",
            "attachments": []
        }),
        config: json!({
            "allowed_domains": ["trusted.example"],
            "blocked_domains": ["evil.example"],
            "delete_messages": true
        }),
    };
    let actions4 = engine
        .execute_event("automod", "message_create", event4)
        .await
        .expect("automod event should execute");
    assert!(
        matches!(actions4.first(), Some(LuaAction::DeleteMessage { .. })),
        "expected delete message when evil destination is disguised with trusted userinfo"
    );

    // 5. Inverted userinfo trick: https://evil.example@trusted.example/path
    // Whitelist includes trusted.example, blacklist includes evil.example.
    // The actual destination is trusted.example (allowed), userinfo is evil.example.
    // This MUST be allowed (no DeleteMessage action).
    let event5 = LuaEventContext {
        name: "message_create".to_owned(),
        guild_id: Some("100000000000000001".to_owned()),
        channel_id: Some("100000000000000002".to_owned()),
        actor_id: Some("100000000000000003".to_owned()),
        data: json!({
            "message_id": "100000000000000014",
            "content": "Safe link https://evil.example@trusted.example/path to docs",
            "attachments": []
        }),
        config: json!({
            "allowed_domains": ["trusted.example"],
            "blocked_domains": ["evil.example"],
            "delete_messages": true
        }),
    };
    let actions5 = engine
        .execute_event("automod", "message_create", event5)
        .await
        .expect("automod event should execute");
    assert!(
        actions5.is_empty(),
        "expected no actions (allowed) when destination is trusted.example despite userinfo containing evil.example"
    );

    // 6. Whitelist exemption for naked domain with path: bit.ly/offer
    let event6 = LuaEventContext {
        name: "message_create".to_owned(),
        guild_id: Some("100000000000000001".to_owned()),
        channel_id: Some("100000000000000002".to_owned()),
        actor_id: Some("100000000000000003".to_owned()),
        data: json!({
            "message_id": "100000000000000015",
            "content": "Official shortlink bit.ly/offer for server members",
            "attachments": []
        }),
        config: json!({
            "block_suspicious_links": true,
            "allowed_domains": ["bit.ly"],
            "delete_messages": true
        }),
    };
    let actions6 = engine
        .execute_event("automod", "message_create", event6)
        .await
        .expect("automod event should execute");
    assert!(
        actions6.is_empty(),
        "expected naked domain to be allowed when present in allowed_domains"
    );
}

#[tokio::test]
async fn join_guard_detects_raid_burst_and_triggers_lockdown() {
    let event = LuaEventContext {
        name: "guild_member_add".to_owned(),
        guild_id: Some("100000000000000001".to_owned()),
        channel_id: None,
        actor_id: Some("100000000000000003".to_owned()),
        data: json!({
            "user": {
                "id": "100000000000000003",
                "name": "RaidBot1",
                "global_name": "Raid Bot 1",
                "bot": false,
                "has_avatar": false
            },
            "roles": [],
            "joined_at": null,
            "recent_joins_count": 8,
            "recent_joins_window_seconds": 60
        }),
        config: json!({
            "alert_channel_id": "100000000000000006",
            "raid_burst_threshold": 5,
            "action_on_raid": "kick"
        }),
    };

    let actions = engine()
        .execute_event("join_guard", "guild_member_add", event)
        .await
        .expect("join guard event should execute");

    assert!(actions.iter().any(|action| matches!(
        action,
        LuaAction::KickMember { reason, .. } if reason.as_deref().is_some_and(|r| r.contains("Lockdown"))
    )));
    assert!(actions.iter().any(|action| matches!(
        action,
        LuaAction::SendMessage { content, .. } if content.contains("ALARM OCHRONY PRZED RAJDEM") || content.contains("RAID PROTECTION ALERT")
    )));
}

#[tokio::test]
async fn music_command_supports_volume_and_loop() {
    let engine = engine();

    let volume_actions = engine
        .execute_command(
            "music",
            "music",
            context(json!({ "action": "volume", "query": "80" }), json!({})),
        )
        .await
        .expect("music volume command should execute");

    assert!(matches!(
        volume_actions.get(1),
        Some(LuaAction::Music { operation, query: Some(q) })
            if matches!(operation, MusicOperation::Volume) && q == "80"
    ));

    let loop_actions = engine
        .execute_command(
            "music",
            "music",
            context(json!({ "action": "loop" }), json!({})),
        )
        .await
        .expect("music loop command should execute");

    assert!(matches!(
        loop_actions.get(1),
        Some(LuaAction::Music { operation, query: None })
            if matches!(operation, MusicOperation::Loop)
    ));

    let now_playing_actions = engine
        .execute_command(
            "music",
            "music",
            context(json!({ "action": "now_playing" }), json!({})),
        )
        .await
        .expect("music now_playing command should execute");

    assert!(matches!(
        now_playing_actions.get(1),
        Some(LuaAction::Music { operation, query: None })
            if matches!(operation, MusicOperation::NowPlaying)
    ));

    let leave_actions = engine
        .execute_command(
            "music",
            "music",
            context(json!({ "action": "leave" }), json!({})),
        )
        .await
        .expect("music leave command should execute");

    assert!(matches!(
        leave_actions.get(1),
        Some(LuaAction::Music { operation, query: None })
            if matches!(operation, MusicOperation::Leave)
    ));
}

#[tokio::test]
async fn music_catalog_choices_match_rust_music_operations_issue_362() {
    let engine = engine();
    let commands = engine.command_definitions().await;
    let music_cmd = commands
        .iter()
        .find(|c| c.name == "music")
        .expect("music command should be registered");

    let action_opt = music_cmd
        .options
        .iter()
        .find(|opt| opt.name == "action")
        .expect("action option must exist on /music");

    let choices = &action_opt.choices;

    let expected_operations = [
        ("play", MusicOperation::Play),
        ("pause", MusicOperation::Pause),
        ("resume", MusicOperation::Resume),
        ("skip", MusicOperation::Skip),
        ("stop", MusicOperation::Stop),
        ("leave", MusicOperation::Leave),
        ("volume", MusicOperation::Volume),
        ("loop", MusicOperation::Loop),
        ("now_playing", MusicOperation::NowPlaying),
        ("queue", MusicOperation::Queue),
    ];

    assert_eq!(
        choices.len(),
        expected_operations.len(),
        "all expected music operations must have a corresponding choice"
    );

    for (value_str, expected_op) in expected_operations {
        let choice = choices
            .iter()
            .find(|c| c.value == value_str)
            .unwrap_or_else(|| panic!("choice with value '{value_str}' should be defined"));
        assert!(!choice.name.is_empty());

        let json_value = format!("\"{value_str}\"");
        let deserialized: MusicOperation = serde_json::from_str(&json_value)
            .unwrap_or_else(|_| panic!("serde should deserialize '{value_str}' to MusicOperation"));
        assert_eq!(deserialized, expected_op);
    }
}

#[tokio::test]
async fn progression_event_awards_xp_and_passes_role_rewards() {
    let event = LuaEventContext {
        name: "message_create".to_owned(),
        guild_id: Some("100000000000000001".to_owned()),
        channel_id: Some("100000000000000002".to_owned()),
        actor_id: Some("100000000000000003".to_owned()),
        data: json!({
            "message_id": "100000000000000004",
            "content": "Witajcie serdecznie na naszym serwerze!",
        }),
        config: json!({
            "xp_min": 20,
            "xp_max": 20,
            "xp_cooldown_seconds": 45,
            "announce_level_up": true,
            "role_rewards": [
                { "level": 5, "role_id": "100000000000000005" },
                { "level": 10, "role_id": "100000000000000010" }
            ]
        }),
    };

    let actions = engine()
        .execute_event("progression", "message_create", event)
        .await
        .expect("progression message_create event should execute");

    assert_eq!(actions.len(), 1);
    match actions.first().unwrap() {
        LuaAction::Progression { operation } => match operation {
            ProgressionOperation::AwardMessageXp {
                amount,
                cooldown_seconds,
                announce_level_up,
                role_rewards,
                ..
            } => {
                assert_eq!(*amount, 20);
                assert_eq!(*cooldown_seconds, 45);
                assert!(*announce_level_up);
                assert_eq!(role_rewards.len(), 2);
                assert_eq!(role_rewards[0].level, 5);
                assert_eq!(role_rewards[0].role_id, "100000000000000005");
                assert_eq!(role_rewards[1].level, 10);
                assert_eq!(role_rewards[1].role_id, "100000000000000010");
            }
            other => panic!("expected AwardMessageXp operation, got {other:?}"),
        },
        other => panic!("expected Progression action, got {other:?}"),
    }
}

#[tokio::test]
async fn welcome_module_greets_new_member_assigns_auto_roles_and_sends_dm() {
    let event = LuaEventContext {
        name: "guild_member_add".to_owned(),
        guild_id: Some("100000000000000001".to_owned()),
        channel_id: None,
        actor_id: Some("100000000000000003".to_owned()),
        data: json!({
            "user": {
                "id": "100000000000000003",
                "name": "tester",
                "global_name": "Tester",
                "bot": false
            },
            "guild_name": "Rust Polska",
            "member_count": 250,
            "roles": [],
            "joined_at": null
        }),
        config: json!({
            "channel_id": "100000000000000002",
            "message": "Witaj {user} na serwerze {server}! Jesteś naszym {member_count}. członkiem.",
            "dm_welcome": true,
            "dm_message": "Cześć {username}! Witamy na serwerze {server}.",
            "auto_roles": ["100000000000000007"]
        }),
    };

    let actions = engine()
        .execute_event("welcome", "guild_member_add", event)
        .await
        .expect("welcome event should execute");

    assert_eq!(actions.len(), 4);

    assert!(matches!(
        &actions[0],
        LuaAction::SendMessage { channel_id: Some(cid), content }
            if cid == "100000000000000002" && content.contains("Witaj <@100000000000000003> na serwerze Rust Polska! Jesteś naszym 250. członkiem.")
    ));

    assert!(matches!(
        &actions[1],
        LuaAction::Audit { event, .. } if event == "welcome_sent"
    ));

    assert!(matches!(
        &actions[2],
        LuaAction::SendDirectMessage { user_id, content }
            if user_id == "100000000000000003" && content.contains("Cześć Tester! Witamy na serwerze Rust Polska.")
    ));

    assert!(matches!(
        &actions[3],
        LuaAction::AddRole { user_id, role_id, .. }
            if user_id == "100000000000000003" && role_id == "100000000000000007"
    ));
}

#[tokio::test]
async fn welcome_module_sends_goodbye_message_on_member_remove() {
    let event = LuaEventContext {
        name: "guild_member_remove".to_owned(),
        guild_id: Some("100000000000000001".to_owned()),
        channel_id: None,
        actor_id: Some("100000000000000003".to_owned()),
        data: json!({
            "user": {
                "id": "100000000000000003",
                "name": "tester",
                "global_name": "Tester",
                "bot": false
            },
            "guild_name": "Rust Polska",
            "member_count": 249,
            "roles": []
        }),
        config: json!({
            "send_goodbye": true,
            "goodbye_channel_id": "100000000000000009",
            "goodbye_message": "Żegnaj {username} z serwera {server}! Zostało nas {member_count}."
        }),
    };

    let actions = engine()
        .execute_event("welcome", "guild_member_remove", event)
        .await
        .expect("welcome goodbye event should execute");

    assert_eq!(actions.len(), 2);

    assert!(matches!(
        &actions[0],
        LuaAction::SendMessage { channel_id: Some(cid), content }
            if cid == "100000000000000009" && content == "Żegnaj Tester z serwera Rust Polska! Zostało nas 249."
    ));

    assert!(matches!(
        &actions[1],
        LuaAction::Audit { event, .. } if event == "goodbye_sent"
    ));
}
