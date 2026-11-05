use anyhow::{Context as _, Result, anyhow};
use serenity::{
    all::{
        Command, CommandOptionType, GuildId, InstallationContext, InteractionContext, Permissions,
    },
    builder::{CreateCommand, CreateCommandOption},
    client::Context,
};

use crate::{
    AppState,
    lua::{
        LuaCommandDefinition, LuaCommandOption, LuaInstallationContext, LuaInteractionContext,
        LuaOptionKind,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RegistrationScope {
    Global,
    Guild,
}

pub async fn register(ctx: &Context, state: &AppState) -> Result<usize> {
    let definitions = state.scripts.command_definitions().await;
    let scope = if state.config.discord_dev_guild_id.is_some() {
        RegistrationScope::Guild
    } else {
        RegistrationScope::Global
    };

    if definitions
        .iter()
        .any(|d| d.name == "policy" || d.name == "duty")
    {
        anyhow::bail!("Lua script cannot define reserved host command 'policy' or 'duty'");
    }

    let mut commands = definitions
        .iter()
        .map(|definition| build_command(definition, scope))
        .collect::<Result<Vec<_>>>()?;

    commands.push(build_policy_command(scope));
    commands.push(build_duty_command(scope));
    let count = commands.len();

    if let Some(guild_id) = state.config.discord_dev_guild_id {
        GuildId::new(guild_id)
            .set_commands(&ctx.http, commands)
            .await
            .context("failed to register development-guild commands")?;
        tracing::info!(guild_id, count, "registered development guild commands");
    } else {
        Command::set_global_commands(&ctx.http, commands)
            .await
            .context("failed to register global commands")?;
        tracing::info!(count, "registered global commands");
    }

    Ok(count)
}

pub fn build_policy_command(scope: RegistrationScope) -> CreateCommand {
    let mut command = CreateCommand::new("policy")
        .description("Inspect or reset command policy rules")
        .default_member_permissions(Permissions::ADMINISTRATOR)
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "reset",
                "Reset all rules to default",
            )
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::Boolean, "confirm", "Confirm reset")
                    .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "explain",
                "Explain policy evaluation for a command",
            )
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "command",
                    "Command name to inspect",
                )
                .required(true),
            )
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::User,
                    "user",
                    "Target user to evaluate policy for",
                )
                .required(false),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "canrun",
                "Check if a command can run",
            )
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "command",
                    "Command name to check",
                )
                .required(true),
            )
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::User,
                    "user",
                    "Target user to evaluate policy for",
                )
                .required(false),
            ),
        );

    if scope == RegistrationScope::Global {
        command = command
            .integration_types(vec![InstallationContext::Guild])
            .contexts(vec![InteractionContext::Guild]);
    }

    command
}

pub fn build_duty_command(scope: RegistrationScope) -> CreateCommand {
    let mut command = CreateCommand::new("duty")
        .description("Manage moderator on-call duty availability")
        .add_option(CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "on",
            "Go on duty",
        ))
        .add_option(
            CreateCommandOption::new(CommandOptionType::SubCommand, "off", "Go off duty")
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "duration",
                        "Duration (e.g. 2h, 30m)",
                    )
                    .required(false),
                ),
        )
        .add_option(CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "status",
            "View current duty status",
        ))
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommandGroup,
                "schedule",
                "Manage duty schedule",
            )
            .add_sub_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "clear",
                "Clear recurring schedule",
            )),
        );

    if scope == RegistrationScope::Global {
        command = command
            .integration_types(vec![InstallationContext::Guild])
            .contexts(vec![InteractionContext::Guild]);
    }

    command
}

fn build_command(
    definition: &LuaCommandDefinition,
    scope: RegistrationScope,
) -> Result<CreateCommand> {
    definition
        .validate_contexts()
        .map_err(|message| anyhow!("command /{}: {message}", definition.name))?;
    definition
        .validate_options_order()
        .map_err(|message| anyhow!("{message}"))?;

    let description = if definition.description.contains('.') {
        crate::translations::t("en", &definition.description)
    } else {
        definition.description.clone()
    };

    let mut command = CreateCommand::new(&definition.name)
        .description(&description)
        .nsfw(definition.nsfw);

    for (locale, desc) in crate::translations::get_command_localizations(&definition.name) {
        command = command.description_localized(locale, desc);
    }

    if scope == RegistrationScope::Global {
        command = command
            .integration_types(
                definition
                    .resolved_integration_types()
                    .into_iter()
                    .map(installation_context)
                    .collect(),
            )
            .contexts(
                definition
                    .resolved_contexts()
                    .into_iter()
                    .map(interaction_context)
                    .collect(),
            );
    }

    if let Some(bits) = &definition.default_member_permissions {
        let bits = bits.parse::<u64>().with_context(|| {
            format!(
                "command /{} has invalid default_member_permissions",
                definition.name
            )
        })?;
        command = command.default_member_permissions(Permissions::from_bits_truncate(bits));
    }

    for option in &definition.options {
        command = command.add_option(build_option(option, &definition.name)?);
    }

    Ok(command)
}

const fn installation_context(context: LuaInstallationContext) -> InstallationContext {
    match context {
        LuaInstallationContext::Guild => InstallationContext::Guild,
        LuaInstallationContext::User => InstallationContext::User,
    }
}

const fn interaction_context(context: LuaInteractionContext) -> InteractionContext {
    match context {
        LuaInteractionContext::Guild => InteractionContext::Guild,
        LuaInteractionContext::BotDm => InteractionContext::BotDm,
        LuaInteractionContext::PrivateChannel => InteractionContext::PrivateChannel,
    }
}

fn build_option(definition: &LuaCommandOption, cmd_name: &str) -> Result<CreateCommandOption> {
    let option_type = option_type(definition.kind);
    let description = if definition.description.contains('.') {
        crate::translations::t("en", &definition.description)
    } else {
        definition.description.clone()
    };
    let mut option = CreateCommandOption::new(option_type, &definition.name, &description)
        .required(definition.required)
        .set_autocomplete(definition.autocomplete);

    for (locale, desc) in
        crate::translations::get_command_option_localizations(cmd_name, &definition.name)
    {
        option = option.description_localized(locale, desc);
    }

    if let Some(min_length) = definition.min_length {
        option = option.min_length(min_length);
    }
    if let Some(max_length) = definition.max_length {
        option = option.max_length(max_length);
    }

    match definition.kind {
        LuaOptionKind::Integer => {
            if let Some(value) = definition.min_value {
                option = option.min_int_value(non_negative_integer(value, "min_value")?);
            }
            if let Some(value) = definition.max_value {
                option = option.max_int_value(non_negative_integer(value, "max_value")?);
            }
        }
        LuaOptionKind::Number => {
            if let Some(value) = definition.min_value {
                option = option.min_number_value(value);
            }
            if let Some(value) = definition.max_value {
                option = option.max_number_value(value);
            }
        }
        _ => {}
    }

    for choice in &definition.choices {
        option = match definition.kind {
            LuaOptionKind::String => option.add_string_choice(
                &choice.name,
                choice
                    .value
                    .as_str()
                    .ok_or_else(|| anyhow!("string choice {} must be a string", choice.name))?,
            ),
            LuaOptionKind::Integer => {
                let value = choice
                    .value
                    .as_i64()
                    .ok_or_else(|| anyhow!("integer choice {} must be an integer", choice.name))?;
                let value = i32::try_from(value).with_context(|| {
                    format!("integer choice {} is outside Discord's range", choice.name)
                })?;
                option.add_int_choice(&choice.name, value)
            }
            LuaOptionKind::Number => option.add_number_choice(
                &choice.name,
                choice
                    .value
                    .as_f64()
                    .ok_or_else(|| anyhow!("number choice {} must be numeric", choice.name))?,
            ),
            _ => {
                return Err(anyhow!(
                    "choices are only valid for string, integer and number options"
                ));
            }
        };
    }

    for child in &definition.options {
        option = option.add_sub_option(build_option(child, cmd_name)?);
    }

    Ok(option)
}

const fn option_type(kind: LuaOptionKind) -> CommandOptionType {
    match kind {
        LuaOptionKind::String => CommandOptionType::String,
        LuaOptionKind::Integer => CommandOptionType::Integer,
        LuaOptionKind::Number => CommandOptionType::Number,
        LuaOptionKind::Boolean => CommandOptionType::Boolean,
        LuaOptionKind::User => CommandOptionType::User,
        LuaOptionKind::Channel => CommandOptionType::Channel,
        LuaOptionKind::Role => CommandOptionType::Role,
        LuaOptionKind::Mentionable => CommandOptionType::Mentionable,
        LuaOptionKind::Attachment => CommandOptionType::Attachment,
        LuaOptionKind::Subcommand => CommandOptionType::SubCommand,
        LuaOptionKind::SubcommandGroup => CommandOptionType::SubCommandGroup,
    }
}

fn non_negative_integer(value: f64, name: &str) -> Result<u64> {
    if value < 0.0 || value.fract() != 0.0 {
        return Err(anyhow!("{name} must be a non-negative integer"));
    }
    if value > u64::MAX as f64 {
        return Err(anyhow!("{name} is too large"));
    }
    Ok(value as u64)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{RegistrationScope, build_command, build_duty_command, build_policy_command};
    use crate::lua::LuaCommandDefinition;

    fn definition(value: serde_json::Value) -> LuaCommandDefinition {
        serde_json::from_value(value).expect("command definition should deserialize")
    }

    #[test]
    fn global_command_uses_current_installation_and_interaction_contexts() {
        let definition = definition(json!({
            "name": "hello",
            "description": "Says hello.",
            "integration_types": ["guild", "user"],
            "contexts": ["guild", "bot_dm", "private_channel"],
            "nsfw": true
        }));

        let payload = serde_json::to_value(
            build_command(&definition, RegistrationScope::Global)
                .expect("global command should build"),
        )
        .expect("command should serialize");

        assert_eq!(
            payload["integration_types"]
                .as_array()
                .expect("integration_types array")
                .len(),
            2
        );
        assert_eq!(
            payload["contexts"]
                .as_array()
                .expect("contexts array")
                .len(),
            3
        );
        assert_eq!(payload["nsfw"], json!(true));
        assert!(payload.get("dm_permission").is_none());
    }

    #[test]
    fn guild_command_omits_global_only_context_fields() {
        let definition = definition(json!({
            "name": "hello",
            "description": "Says hello.",
            "integration_types": ["guild"],
            "contexts": ["guild"]
        }));

        let payload = serde_json::to_value(
            build_command(&definition, RegistrationScope::Guild)
                .expect("guild command should build"),
        )
        .expect("command should serialize");

        assert!(payload.get("integration_types").is_none());
        assert!(payload.get("contexts").is_none());
        assert!(payload.get("dm_permission").is_none());
    }

    #[test]
    fn legacy_dm_permission_is_migrated_without_using_the_deprecated_payload_field() {
        let definition = definition(json!({
            "name": "hello",
            "description": "Says hello.",
            "dm_permission": true
        }));

        let payload = serde_json::to_value(
            build_command(&definition, RegistrationScope::Global)
                .expect("legacy command should build"),
        )
        .expect("command should serialize");

        assert_eq!(
            payload["contexts"]
                .as_array()
                .expect("contexts array")
                .len(),
            2
        );
        assert!(payload.get("dm_permission").is_none());
    }

    #[test]
    fn command_with_invalid_option_ordering_is_rejected() {
        let definition = definition(json!({
            "name": "badcmd",
            "description": "Invalid option order.",
            "options": [
                { "type": "string", "name": "opt", "description": "optional", "required": false },
                { "type": "string", "name": "req", "description": "required", "required": true }
            ]
        }));

        let result = build_command(&definition, RegistrationScope::Global);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("required option 'req' cannot follow an optional option")
        );
    }

    #[test]
    fn policy_command_builds_with_subcommands_and_options() {
        for scope in [RegistrationScope::Global, RegistrationScope::Guild] {
            let cmd = build_policy_command(scope);
            let val = serde_json::to_value(&cmd).expect("serialize policy command");
            assert_eq!(val["name"], "policy");
            let options = val["options"].as_array().expect("options array");
            assert_eq!(options.len(), 3);

            let subnames: Vec<&str> = options.iter().filter_map(|o| o["name"].as_str()).collect();
            assert!(subnames.contains(&"reset"));
            assert!(subnames.contains(&"explain"));
            assert!(subnames.contains(&"canrun"));

            let reset_opt = options.iter().find(|o| o["name"] == "reset").unwrap();
            let reset_sub = reset_opt["options"].as_array().unwrap();
            assert_eq!(reset_sub[0]["name"], "confirm");
            assert_eq!(reset_sub[0]["required"], true);

            let explain_opt = options.iter().find(|o| o["name"] == "explain").unwrap();
            let explain_sub = explain_opt["options"].as_array().unwrap();
            assert_eq!(explain_sub[0]["name"], "command");
            assert_eq!(explain_sub[0]["required"], true);

            if scope == RegistrationScope::Global {
                assert!(val["contexts"].is_array());
                assert!(val["integration_types"].is_array());
            } else {
                assert!(val.get("contexts").is_none());
            }
        }
    }

    #[test]
    fn duty_command_builds_with_subcommands() {
        for scope in [RegistrationScope::Global, RegistrationScope::Guild] {
            let cmd = build_duty_command(scope);
            let val = serde_json::to_value(&cmd).expect("serialize duty command");
            assert_eq!(val["name"], "duty");
            let options = val["options"].as_array().expect("options array");
            assert_eq!(options.len(), 4);

            let subnames: Vec<&str> = options.iter().filter_map(|o| o["name"].as_str()).collect();
            assert!(subnames.contains(&"on"));
            assert!(subnames.contains(&"off"));
            assert!(subnames.contains(&"status"));
            assert!(subnames.contains(&"schedule"));
        }
    }
}
