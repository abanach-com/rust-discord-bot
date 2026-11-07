use std::path::PathBuf;

use rusty_discord_bot::lua::{
    GiveawayOperation, LuaAction, LuaEngine, LuaExecutionContext, LuaLimits, ProgressionOperation,
    TagOperation,
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
    LuaEngine::load(scripts, limits())
        .expect("bundled Lua modules should load and schemas must be valid")
}

fn context(options: serde_json::Value) -> LuaExecutionContext {
    context_with_config(options, json!({}))
}

fn context_with_config(
    options: serde_json::Value,
    config: serde_json::Value,
) -> LuaExecutionContext {
    LuaExecutionContext {
        guild_id: Some("111111111111111111".to_owned()),
        channel_id: "222222222222222222".to_owned(),
        user_id: "333333333333333333".to_owned(),
        user_name: "TestUser".to_owned(),
        member_roles: Vec::new(),
        member_permissions: "0".to_owned(),
        locale: "en".to_owned(),
        options,
        config,
    }
}

#[tokio::test]
async fn test_progression_deserialization_contract() {
    // Issue #129: unsupported operation names must fail deserialization
    let unsupported_view = json!({
        "type": "progression",
        "operation": { "type": "view_progress", "user_id": "333333333333333333" }
    });
    assert!(serde_json::from_value::<LuaAction>(unsupported_view).is_err());

    let unsupported_transfer = json!({
        "type": "progression",
        "operation": { "type": "transfer_coins", "recipient_user_id": "444", "amount": 10 }
    });
    assert!(serde_json::from_value::<LuaAction>(unsupported_transfer).is_err());

    // Valid operations
    let valid_balance = json!({
        "type": "progression",
        "operation": { "type": "balance", "user_id": "333333333333333333" }
    });
    assert!(serde_json::from_value::<LuaAction>(valid_balance).is_ok());

    let valid_transfer = json!({
        "type": "progression",
        "operation": { "type": "transfer", "user_id": "444444444444444444", "amount": 25 }
    });
    assert!(serde_json::from_value::<LuaAction>(valid_transfer).is_ok());
}

#[tokio::test]
async fn test_economy_subcommands_and_progression_actions() {
    let eng = engine();

    // 1. /economy balance
    let actions = eng
        .execute_command("economy", "economy", context(json!({ "balance": {} })))
        .await
        .expect("economy balance should succeed");
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        LuaAction::Progression {
            operation: ProgressionOperation::Balance { user_id },
        } => {
            assert_eq!(user_id.as_deref(), Some("333333333333333333"));
        }
        other => panic!("expected ProgressionOperation::Balance, got {other:?}"),
    }

    // 2. /economy transfer
    let actions = eng
        .execute_command(
            "economy",
            "economy",
            context(json!({
                "transfer": {
                    "recipient": "444444444444444444",
                    "amount": 25
                }
            })),
        )
        .await
        .expect("economy transfer should succeed");
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        LuaAction::Progression {
            operation: ProgressionOperation::Transfer { user_id, amount },
        } => {
            assert_eq!(user_id, "444444444444444444");
            assert_eq!(*amount, 25);
        }
        other => panic!("expected ProgressionOperation::Transfer, got {other:?}"),
    }

    // 3. /economy buy
    let actions = eng
        .execute_command(
            "economy",
            "economy",
            context(json!({
                "buy": { "item": "vip_pass" }
            })),
        )
        .await
        .expect("economy buy should succeed");
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, LuaAction::Reply { content, .. } if content.contains("vip_pass")))
    );

    // 4. Missing or unsupported subcommand
    let actions = eng
        .execute_command("economy", "economy", context(json!({})))
        .await
        .expect("economy with missing subcommand should return safe reply");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { content, .. } if content.contains("Unsupported or missing subcommand."))));
}

#[tokio::test]
async fn test_giveaway_subcommands_no_accidental_start() {
    let eng = engine();

    // /giveaway end should emit GiveawayOperation::End, never start a new giveaway
    let actions = eng
        .execute_command(
            "giveaway",
            "giveaway",
            context(json!({
                "end": { "giveaway_id": "00000000-0000-0000-0000-000000000999" }
            })),
        )
        .await
        .expect("giveaway end should succeed");
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, LuaAction::Audit { event, .. } if event == "giveaway_start"))
    );
    assert!(actions.iter().any(|a| matches!(
        a,
        LuaAction::Giveaway {
            operation: GiveawayOperation::End { giveaway_id }
        } if giveaway_id == "00000000-0000-0000-0000-000000000999"
    )));

    // /giveaway reroll should emit GiveawayOperation::Reroll, never start a new giveaway
    let actions = eng
        .execute_command(
            "giveaway",
            "giveaway",
            context(json!({
                "reroll": { "giveaway_id": "00000000-0000-0000-0000-000000000888" }
            })),
        )
        .await
        .expect("giveaway reroll should succeed");
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, LuaAction::Audit { event, .. } if event == "giveaway_start"))
    );
    assert!(actions.iter().any(|a| matches!(
        a,
        LuaAction::Giveaway {
            operation: GiveawayOperation::Reroll { giveaway_id }
        } if giveaway_id == "00000000-0000-0000-0000-000000000888"
    )));

    // /giveaway start should emit GiveawayOperation::Start
    let actions = eng
        .execute_command(
            "giveaway",
            "giveaway",
            context(json!({
                "start": {
                    "prize": "Discord Nitro",
                    "duration_minutes": 120,
                    "winners": 2
                }
            })),
        )
        .await
        .expect("giveaway start should succeed");
    assert!(actions.iter().any(|a| matches!(
        a,
        LuaAction::Giveaway {
            operation: GiveawayOperation::Start {
                prize,
                duration_minutes: 120,
                winners: 2,
                ..
            }
        } if prize == "Discord Nitro"
    )));

    // Missing subcommand
    let actions = eng
        .execute_command("giveaway", "giveaway", context(json!({})))
        .await
        .expect("giveaway missing subcommand should return safe reply");
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, LuaAction::Audit { event, .. } if event == "giveaway_start"))
    );
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { content, .. } if content.contains("Unsupported or missing subcommand."))));
}

#[tokio::test]
async fn test_giveaway_module_config_enforcement() {
    let eng = engine();

    // 1. Configured max_winners = 2; attempting 5 winners should return ephemeral reply rejection
    let actions = eng
        .execute_command(
            "giveaway",
            "giveaway",
            context_with_config(
                json!({
                    "start": {
                        "prize": "Nitro",
                        "winners": 5
                    }
                }),
                json!({
                    "max_winners": 2
                }),
            ),
        )
        .await
        .expect("giveaway start execution");
    assert_eq!(actions.len(), 1);
    assert!(actions.iter().any(|a| matches!(
        a,
        LuaAction::Reply { content, ephemeral: true } if content.contains("Liczba zwycięzców nie może przekraczać limitu serwera (2)")
    )));

    // 2. Default duration and announcement channel configured
    let actions = eng
        .execute_command(
            "giveaway",
            "giveaway",
            context_with_config(
                json!({
                    "start": {
                        "prize": "Server Merch"
                    }
                }),
                json!({
                    "default_duration_minutes": 180,
                    "announcement_channel_id": "888888888888888888"
                }),
            ),
        )
        .await
        .expect("giveaway start execution");
    assert!(actions.iter().any(|a| matches!(
        a,
        LuaAction::Giveaway {
            operation: GiveawayOperation::Start {
                prize,
                duration_minutes: 180,
                winners: 1,
                channel_id: Some(ch)
            }
        } if prize == "Server Merch" && ch == "888888888888888888"
    )));
}

#[tokio::test]
async fn test_achievements_subcommands_and_safe_claim() {
    let eng = engine();

    // /achievements claim returns honest unavailable response and does not adjust progress
    let actions = eng
        .execute_command(
            "achievements",
            "achievements",
            context(json!({
                "claim": { "badge_id": "first_msg" }
            })),
        )
        .await
        .expect("achievements claim should succeed");
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        LuaAction::Reply { content, ephemeral } => {
            assert!(*ephemeral);
            assert!(content.contains("not yet available"));
        }
        other => panic!("expected safe reply, got {other:?}"),
    }

    // /achievements list
    let actions = eng
        .execute_command(
            "achievements",
            "achievements",
            context(json!({ "list": {} })),
        )
        .await
        .expect("achievements list should succeed");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { .. })));

    // /achievements view
    let actions = eng
        .execute_command(
            "achievements",
            "achievements",
            context(json!({ "view": { "user": "555" } })),
        )
        .await
        .expect("achievements view should succeed");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { .. })));

    // Missing subcommand
    let actions = eng
        .execute_command("achievements", "achievements", context(json!({})))
        .await
        .expect("achievements missing subcommand should return safe reply");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { content, .. } if content.contains("Unsupported or missing subcommand."))));
}

#[tokio::test]
async fn test_afk_subcommands() {
    let eng = engine();

    // /afk set
    let actions = eng
        .execute_command(
            "afk",
            "afk",
            context(json!({ "set": { "reason": "Working on Rust" } })),
        )
        .await
        .expect("afk set should succeed");
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, LuaAction::Audit { event, .. } if event == "afk_set"))
    );

    // /afk clear
    let actions = eng
        .execute_command("afk", "afk", context(json!({ "clear": {} })))
        .await
        .expect("afk clear should succeed");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { .. })));

    // /afk status
    let actions = eng
        .execute_command("afk", "afk", context(json!({ "status": {} })))
        .await
        .expect("afk status should succeed");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { .. })));

    // Missing subcommand
    let actions = eng
        .execute_command("afk", "afk", context(json!({})))
        .await
        .expect("afk missing subcommand should return safe reply");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { content, .. } if content.contains("Unsupported or missing subcommand."))));
}

#[tokio::test]
async fn test_birthdays_subcommands() {
    let eng = engine();

    // /birthday set
    let actions = eng
        .execute_command(
            "birthdays",
            "birthday",
            context(json!({ "set": { "day": 12, "month": 5 } })),
        )
        .await
        .expect("birthday set should succeed");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { .. })));

    // /birthday check
    let actions = eng
        .execute_command(
            "birthdays",
            "birthday",
            context(json!({ "check": { "user": "999" } })),
        )
        .await
        .expect("birthday check should succeed");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { .. })));

    // /birthday upcoming
    let actions = eng
        .execute_command("birthdays", "birthday", context(json!({ "upcoming": {} })))
        .await
        .expect("birthday upcoming should succeed");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { .. })));

    // Missing subcommand
    let actions = eng
        .execute_command("birthdays", "birthday", context(json!({})))
        .await
        .expect("birthday missing subcommand should return safe reply");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { content, .. } if content.contains("Unsupported or missing subcommand."))));
}

#[tokio::test]
async fn test_logging_subcommands() {
    let eng = engine();

    // /logging config
    let actions = eng
        .execute_command(
            "logging",
            "logging",
            context(json!({ "config": { "channel": "12345678" } })),
        )
        .await
        .expect("logging config should succeed");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { .. })));

    // /logging test
    let actions = eng
        .execute_command("logging", "logging", context(json!({ "test": {} })))
        .await
        .expect("logging test should succeed");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { .. })));

    // Missing subcommand
    let actions = eng
        .execute_command("logging", "logging", context(json!({})))
        .await
        .expect("logging missing subcommand should return safe reply");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { content, .. } if content.contains("Unsupported or missing subcommand."))));
}

#[tokio::test]
async fn test_quotes_subcommands() {
    let eng = engine();

    // /quote random
    let actions = eng
        .execute_command("quotes", "quote", context(json!({ "random": {} })))
        .await
        .expect("quotes random should succeed");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { .. })));

    // /quote add
    let actions = eng
        .execute_command("quotes", "quote", context(json!({ "add": {} })))
        .await
        .expect("quotes add should succeed");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { .. })));

    // /quote list
    let actions = eng
        .execute_command("quotes", "quote", context(json!({ "list": {} })))
        .await
        .expect("quotes list should succeed");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { .. })));

    // Missing subcommand
    let actions = eng
        .execute_command("quotes", "quote", context(json!({})))
        .await
        .expect("quotes missing subcommand should return safe reply");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { content, .. } if content.contains("Unsupported or missing subcommand."))));
}

#[tokio::test]
async fn test_reaction_roles_subcommands() {
    let eng = engine();

    // /reactionrole create
    let actions = eng
        .execute_command(
            "reaction_roles",
            "reactionrole",
            context(json!({ "create": { "emoji": "🎉", "role": "12345" } })),
        )
        .await
        .expect("reaction_roles create should succeed");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { .. })));

    // /reactionrole remove
    let actions = eng
        .execute_command(
            "reaction_roles",
            "reactionrole",
            context(json!({ "remove": { "emoji": "🎉" } })),
        )
        .await
        .expect("reaction_roles remove should succeed");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { .. })));

    // Missing subcommand
    let actions = eng
        .execute_command("reaction_roles", "reactionrole", context(json!({})))
        .await
        .expect("reaction_roles missing subcommand should return safe reply");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { content, .. } if content.contains("Unsupported or missing subcommand."))));
}

#[tokio::test]
async fn test_starboard_subcommands() {
    let eng = engine();

    // /starboard config
    let actions = eng
        .execute_command(
            "starboard",
            "starboard",
            context(json!({ "config": { "channel": "987654", "threshold": 5 } })),
        )
        .await
        .expect("starboard config should succeed");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { .. })));

    // /starboard stats
    let actions = eng
        .execute_command("starboard", "starboard", context(json!({ "stats": {} })))
        .await
        .expect("starboard stats should succeed");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { .. })));

    // Missing subcommand
    let actions = eng
        .execute_command("starboard", "starboard", context(json!({})))
        .await
        .expect("starboard missing subcommand should return safe reply");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { content, .. } if content.contains("Unsupported or missing subcommand."))));
}

#[tokio::test]
async fn test_tags_subcommands() {
    let eng = engine();

    // /tag get
    let actions = eng
        .execute_command("tags", "tag", context(json!({ "get": { "name": "info" } })))
        .await
        .expect("tag get should succeed");
    assert!(
        actions.iter().any(|a| matches!(
            a,
            LuaAction::Tag {
                operation: TagOperation::Get { name }
            } if name == "info"
        )),
        "expected LuaAction::Tag with Get operation"
    );

    // /tag create
    let actions = eng
        .execute_command(
            "tags",
            "tag",
            context(json!({ "create": { "name": "info", "content": "Witaj na serwerze!" } })),
        )
        .await
        .expect("tag create should succeed");
    assert!(
        actions.iter().any(|a| matches!(
            a,
            LuaAction::Tag {
                operation: TagOperation::Create { name, content }
            } if name == "info" && content == "Witaj na serwerze!"
        )),
        "expected LuaAction::Tag with Create operation"
    );

    // /tag delete
    let actions = eng
        .execute_command(
            "tags",
            "tag",
            context(json!({ "delete": { "name": "info" } })),
        )
        .await
        .expect("tag delete should succeed");
    assert!(
        actions.iter().any(|a| matches!(
            a,
            LuaAction::Tag {
                operation: TagOperation::Delete { name }
            } if name == "info"
        )),
        "expected LuaAction::Tag with Delete operation"
    );

    // Missing subcommand
    let actions = eng
        .execute_command("tags", "tag", context(json!({})))
        .await
        .expect("tag missing subcommand should return safe reply");
    assert!(actions.iter().any(|a| matches!(a, LuaAction::Reply { content, .. } if content.contains("Unsupported or missing subcommand."))));
}

#[tokio::test]
async fn test_tags_storage_lifecycle() {
    use chrono::Utc;
    use rusty_discord_bot::storage::{Storage, TagRecord};

    let storage = Storage::memory();
    let guild_id = "111111111111111111";

    // 1. Tag doesn't exist
    assert!(
        storage
            .get_tag(guild_id, "rules")
            .await
            .expect("query tag")
            .is_none()
    );

    // 2. Create tag
    let tag = TagRecord {
        name: "rules".to_string(),
        content: "1. Be polite\n2. No spam".to_string(),
        author_id: "222222222222222222".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    storage
        .set_tag(guild_id, tag.clone())
        .await
        .expect("set tag");

    // 3. Read tag (case-insensitive)
    let fetched = storage
        .get_tag(guild_id, "RULES")
        .await
        .expect("get tag")
        .expect("tag must exist");
    assert_eq!(fetched.name, "rules");
    assert_eq!(fetched.content, tag.content);
    assert_eq!(fetched.author_id, tag.author_id);

    // 4. Delete tag
    let deleted = storage
        .delete_tag(guild_id, "rules")
        .await
        .expect("delete tag");
    assert!(deleted);

    // 5. Subsequent get and delete
    assert!(
        storage
            .get_tag(guild_id, "rules")
            .await
            .expect("query after delete")
            .is_none()
    );
    let deleted_again = storage
        .delete_tag(guild_id, "rules")
        .await
        .expect("delete again");
    assert!(!deleted_again);
}
