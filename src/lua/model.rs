use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    afk::AfkOperation, birthdays::BirthdayOperation, economy::EconomyOperation,
    giveaway::GiveawayOperation, moderation_cases::ModerationCaseOperation,
    progression::ProgressionOperation, quotes::QuoteOperation,
    reaction_roles::ReactionRoleOperation, scheduler::SchedulerOperation,
    starboard::StarboardOperation, tags::TagOperation, tickets::TicketOperation,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LuaModuleManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub category: String,
    #[serde(default = "default_true")]
    pub default_enabled: bool,
    #[serde(default)]
    pub commands: Vec<LuaCommandDefinition>,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default = "empty_object")]
    pub config_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LuaCommandDefinition {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub options: Vec<LuaCommandOption>,
    #[serde(default)]
    pub default_member_permissions: Option<String>,
    #[serde(default)]
    pub integration_types: Option<Vec<LuaInstallationContext>>,
    #[serde(default)]
    pub contexts: Option<Vec<LuaInteractionContext>>,
    #[serde(default)]
    pub nsfw: bool,
    /// Compatibility field for pre-context modules. New modules must use `contexts`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dm_permission: Option<bool>,
}

impl LuaCommandDefinition {
    pub fn resolved_integration_types(&self) -> Vec<LuaInstallationContext> {
        self.integration_types
            .clone()
            .unwrap_or_else(|| vec![LuaInstallationContext::Guild])
    }

    pub fn resolved_contexts(&self) -> Vec<LuaInteractionContext> {
        self.contexts.clone().unwrap_or_else(|| {
            if self.dm_permission == Some(true) {
                vec![LuaInteractionContext::Guild, LuaInteractionContext::BotDm]
            } else {
                vec![LuaInteractionContext::Guild]
            }
        })
    }

    pub fn validate_contexts(&self) -> Result<(), String> {
        if self.contexts.is_some() && self.dm_permission.is_some() {
            return Err(
                "contexts and the deprecated dm_permission field cannot be used together"
                    .to_owned(),
            );
        }

        let integration_types = self.resolved_integration_types();
        if integration_types.is_empty() {
            return Err("integration_types cannot be empty".to_owned());
        }
        if has_duplicates(&integration_types) {
            return Err("integration_types cannot contain duplicates".to_owned());
        }

        let contexts = self.resolved_contexts();
        if contexts.is_empty() {
            return Err("contexts cannot be empty".to_owned());
        }
        if has_duplicates(&contexts) {
            return Err("contexts cannot contain duplicates".to_owned());
        }
        if contexts.contains(&LuaInteractionContext::PrivateChannel)
            && !integration_types.contains(&LuaInstallationContext::User)
        {
            return Err("private_channel context requires the user installation type".to_owned());
        }

        Ok(())
    }

    pub fn validate_options_order(&self) -> Result<(), String> {
        Self::validate_option_slice(&self.options, &format!("/{}", self.name))
    }

    fn validate_option_slice(options: &[LuaCommandOption], path: &str) -> Result<(), String> {
        let mut saw_optional = false;
        for option in options {
            if matches!(
                option.kind,
                LuaOptionKind::Subcommand | LuaOptionKind::SubcommandGroup
            ) {
                Self::validate_option_slice(&option.options, &format!("{path} {}", option.name))?;
            } else {
                if option.required {
                    if saw_optional {
                        return Err(format!(
                            "command {path}: required option '{}' cannot follow an optional option",
                            option.name
                        ));
                    }
                } else {
                    saw_optional = true;
                }
                if !option.options.is_empty() {
                    Self::validate_option_slice(
                        &option.options,
                        &format!("{path} {}", option.name),
                    )?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LuaInstallationContext {
    Guild,
    User,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LuaInteractionContext {
    Guild,
    BotDm,
    PrivateChannel,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LuaCommandOption {
    #[serde(rename = "type")]
    pub kind: LuaOptionKind,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub autocomplete: bool,
    #[serde(default)]
    pub choices: Vec<LuaCommandChoice>,
    #[serde(default)]
    pub options: Vec<Self>,
    #[serde(default)]
    pub min_value: Option<f64>,
    #[serde(default)]
    pub max_value: Option<f64>,
    #[serde(default)]
    pub min_length: Option<u16>,
    #[serde(default)]
    pub max_length: Option<u16>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LuaCommandChoice {
    pub name: String,
    pub value: Value,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LuaOptionKind {
    String,
    Integer,
    Number,
    Boolean,
    User,
    Channel,
    Role,
    Mentionable,
    Attachment,
    Subcommand,
    SubcommandGroup,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LuaExecutionContext {
    pub guild_id: Option<String>,
    pub channel_id: String,
    pub user_id: String,
    pub user_name: String,
    pub member_roles: Vec<String>,
    pub member_permissions: String,
    pub locale: String,
    #[serde(default)]
    pub options: Value,
    #[serde(default = "empty_object")]
    pub config: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LuaEventContext {
    pub name: String,
    pub guild_id: Option<String>,
    pub channel_id: Option<String>,
    pub actor_id: Option<String>,
    #[serde(default = "empty_object")]
    pub data: Value,
    #[serde(default = "empty_object")]
    pub config: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LuaAction {
    Reply {
        content: String,
        #[serde(default)]
        ephemeral: bool,
    },
    SendMessage {
        #[serde(default)]
        channel_id: Option<String>,
        content: String,
    },
    SendDirectMessage {
        user_id: String,
        content: String,
    },
    DeleteMessage {
        channel_id: String,
        message_id: String,
    },
    TimeoutMember {
        user_id: String,
        seconds: u64,
        #[serde(default)]
        reason: Option<String>,
    },
    KickMember {
        user_id: String,
        #[serde(default)]
        reason: Option<String>,
    },
    BanMember {
        user_id: String,
        #[serde(default)]
        delete_message_days: u8,
        #[serde(default)]
        reason: Option<String>,
    },
    AddRole {
        user_id: String,
        role_id: String,
        #[serde(default)]
        reason: Option<String>,
    },
    RemoveRole {
        user_id: String,
        role_id: String,
        #[serde(default)]
        reason: Option<String>,
    },
    Purge {
        #[serde(default)]
        channel_id: Option<String>,
        amount: u8,
    },
    Cleanup {
        #[serde(default)]
        channel_id: Option<String>,
        #[serde(default = "default_cleanup_limit")]
        scan_limit: u16,
        #[serde(default = "default_cleanup_limit")]
        delete_limit: u16,
        #[serde(default)]
        filter: crate::storage::CleanupFilter,
        #[serde(default)]
        dry_run: bool,
    },
    ConfirmCleanup {
        plan_id: String,
    },
    Music {
        operation: MusicOperation,
        #[serde(default)]
        query: Option<String>,
    },
    Progression {
        operation: ProgressionOperation,
    },
    Scheduler {
        operation: SchedulerOperation,
    },
    Ticket {
        operation: TicketOperation,
    },
    ModerationCase {
        operation: ModerationCaseOperation,
    },
    Giveaway {
        operation: GiveawayOperation,
    },
    Tag {
        operation: TagOperation,
    },
    ReactionRole {
        operation: ReactionRoleOperation,
    },
    Starboard {
        operation: StarboardOperation,
    },
    Quote {
        operation: QuoteOperation,
    },
    Birthday {
        operation: BirthdayOperation,
    },
    Afk {
        operation: AfkOperation,
    },
    Economy {
        operation: EconomyOperation,
    },
    PublishSubmission {
        channel_id: String,
        content: String,
        success_reply: String,
        audit_event: String,
        #[serde(default)]
        audit_data: BTreeMap<String, Value>,
    },
    Audit {
        event: String,
        #[serde(default)]
        data: BTreeMap<String, Value>,
    },
}

impl LuaAction {
    pub fn policy_action_key(&self) -> &'static str {
        match self {
            Self::Reply { .. } => "reply",
            Self::SendMessage { .. } => "send_message",
            Self::SendDirectMessage { .. } => "send_direct_message",
            Self::DeleteMessage { .. } => "delete_message",
            Self::TimeoutMember { .. } => "timeout_member",
            Self::KickMember { .. } => "kick_member",
            Self::BanMember { .. } => "ban_member",
            Self::AddRole { .. } => "add_role",
            Self::RemoveRole { .. } => "remove_role",
            Self::Purge { .. } => "purge",
            Self::Cleanup { .. } => "cleanup",
            Self::ConfirmCleanup { .. } => "confirm_cleanup",
            Self::Music { .. } => "music",
            Self::Progression { .. } => "progression",
            Self::Scheduler { .. } => "scheduler",
            Self::Ticket { .. } => "ticket",
            Self::ModerationCase { .. } => "moderation_case",
            Self::Giveaway { .. } => "giveaway",
            Self::Tag { .. } => "tag",
            Self::ReactionRole { .. } => "reaction_role",
            Self::Starboard { .. } => "starboard",
            Self::Quote { .. } => "quote",
            Self::Birthday { .. } => "birthday",
            Self::Afk { .. } => "afk",
            Self::Economy { .. } => "economy",
            Self::PublishSubmission { .. } => "publish_submission",
            Self::Audit { .. } => "audit",
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Reply { content, .. } => validate_content(content),
            Self::SendMessage {
                channel_id,
                content,
            } => {
                if let Some(channel_id) = channel_id {
                    validate_snowflake(channel_id)?;
                }
                validate_content(content)
            }
            Self::SendDirectMessage { user_id, content } => {
                validate_snowflake(user_id)?;
                validate_content(content)
            }
            Self::DeleteMessage {
                channel_id,
                message_id,
            } => {
                validate_snowflake(channel_id)?;
                validate_snowflake(message_id)
            }
            Self::TimeoutMember {
                user_id,
                seconds,
                reason,
            } => {
                validate_snowflake(user_id)?;
                if *seconds == 0 || *seconds > 2_419_200 {
                    return Err(
                        "timeout seconds must be between 1 and 2419200 (28 days)".to_owned()
                    );
                }
                validate_reason(reason)
            }
            Self::KickMember { user_id, reason } => {
                validate_snowflake(user_id)?;
                validate_reason(reason)
            }
            Self::BanMember {
                user_id,
                delete_message_days,
                reason,
            } => {
                validate_snowflake(user_id)?;
                if *delete_message_days > 7 {
                    return Err("delete_message_days cannot exceed 7".to_owned());
                }
                validate_reason(reason)
            }
            Self::AddRole {
                user_id,
                role_id,
                reason,
            }
            | Self::RemoveRole {
                user_id,
                role_id,
                reason,
            } => {
                validate_snowflake(user_id)?;
                validate_snowflake(role_id)?;
                validate_reason(reason)
            }
            Self::Purge { channel_id, amount } => {
                if let Some(channel_id) = channel_id {
                    validate_snowflake(channel_id)?;
                }
                if !(1..=100).contains(amount) {
                    return Err("purge amount must be between 1 and 100".to_owned());
                }
                Ok(())
            }
            Self::Cleanup {
                channel_id,
                scan_limit,
                delete_limit,
                filter,
                ..
            } => {
                if let Some(channel_id) = channel_id {
                    validate_snowflake(channel_id)?;
                }
                if *scan_limit == 0 || *scan_limit > 1000 {
                    return Err("cleanup scan_limit must be between 1 and 1000".to_owned());
                }
                if *delete_limit == 0 || *delete_limit > 1000 {
                    return Err("cleanup delete_limit must be between 1 and 1000".to_owned());
                }
                if let Some(ref aid) = filter.author_id {
                    validate_snowflake(aid)?;
                }
                if let Some(ref bid) = filter.before_message_id {
                    validate_snowflake(bid)?;
                }
                if let Some(ref aid) = filter.after_message_id {
                    validate_snowflake(aid)?;
                }
                Ok(())
            }
            Self::ConfirmCleanup { plan_id } => {
                uuid::Uuid::parse_str(plan_id)
                    .map_err(|_| "plan_id must be a valid UUID".to_owned())?;
                Ok(())
            }
            Self::Music { query, operation } => {
                if matches!(operation, MusicOperation::Play)
                    && query.as_deref().is_none_or(str::is_empty)
                {
                    return Err("music play requires a query".to_owned());
                }
                if matches!(operation, MusicOperation::Volume)
                    && query.as_deref().is_none_or(str::is_empty)
                {
                    return Err("music volume requires a level value".to_owned());
                }
                Ok(())
            }
            Self::Progression { operation } => operation.validate(),
            Self::Scheduler { operation } => operation.validate(),
            Self::Ticket { operation } => operation.validate(),
            Self::ModerationCase { operation } => operation.validate(),
            Self::Giveaway { operation } => operation.validate(),
            Self::Tag { operation } => operation.validate(),
            Self::ReactionRole { operation } => operation.validate(),
            Self::Starboard { operation } => operation.validate(),
            Self::Quote { operation } => operation.validate(),
            Self::Birthday { operation } => operation.validate(),
            Self::Afk { operation } => operation.validate(),
            Self::Economy { operation } => operation.validate(),
            Self::PublishSubmission {
                channel_id,
                content,
                success_reply,
                audit_event,
                audit_data,
            } => {
                validate_snowflake(channel_id)?;
                validate_content(content)?;
                validate_content(success_reply)?;
                if audit_event.trim().is_empty() || audit_event.len() > 64 {
                    return Err("audit event must contain 1 to 64 bytes".to_owned());
                }
                validate_audit_data(audit_data)?;
                Ok(())
            }
            Self::Audit { event, data } => {
                if event.trim().is_empty() || event.len() > 64 {
                    return Err("audit event must contain 1 to 64 bytes".to_owned());
                }
                validate_audit_data(data)?;
                Ok(())
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MusicOperation {
    Play,
    Pause,
    Resume,
    Skip,
    Stop,
    Leave,
    Queue,
    Volume,
    Loop,
    NowPlaying,
}

fn has_duplicates<T: PartialEq>(values: &[T]) -> bool {
    values
        .iter()
        .enumerate()
        .any(|(index, value)| values[index + 1..].contains(value))
}

fn validate_content(content: &str) -> Result<(), String> {
    let length = content.chars().count();
    if length == 0 || length > 2_000 {
        return Err("message content must contain 1 to 2000 characters".to_owned());
    }
    Ok(())
}

fn validate_reason(reason: &Option<String>) -> Result<(), String> {
    if reason
        .as_ref()
        .is_some_and(|value| value.chars().count() > 512)
    {
        return Err("audit reason cannot exceed 512 characters".to_owned());
    }
    Ok(())
}

fn validate_snowflake(value: &str) -> Result<(), String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| format!("{value:?} is not a Discord snowflake"))?;
    if parsed == 0 {
        return Err("Discord snowflakes must be greater than zero".to_owned());
    }
    Ok(())
}

fn validate_audit_data(data: &BTreeMap<String, Value>) -> Result<(), String> {
    const MAX_AUDIT_KEYS: usize = 64;
    const MAX_KEY_LENGTH: usize = 64;
    const MAX_TOTAL_JSON_BYTES: usize = 16_384;

    if data.len() > MAX_AUDIT_KEYS {
        return Err(format!(
            "audit data cannot contain more than {MAX_AUDIT_KEYS} entries; got {}",
            data.len()
        ));
    }
    for key in data.keys() {
        if key.trim().is_empty() || key.len() > MAX_KEY_LENGTH {
            return Err(format!(
                "audit data key {key:?} must contain between 1 and {MAX_KEY_LENGTH} bytes"
            ));
        }
    }
    let serialized = serde_json::to_vec(data)
        .map_err(|e| format!("audit data failed to serialize to JSON: {e}"))?;
    if serialized.len() > MAX_TOTAL_JSON_BYTES {
        return Err(format!(
            "audit data JSON representation exceeds {MAX_TOTAL_JSON_BYTES} bytes (size: {} bytes)",
            serialized.len()
        ));
    }
    Ok(())
}

const fn default_true() -> bool {
    true
}

const fn default_cleanup_limit() -> u16 {
    100
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{LuaCommandDefinition, LuaInstallationContext, LuaInteractionContext};

    fn command(value: serde_json::Value) -> LuaCommandDefinition {
        serde_json::from_value(value).expect("command definition should deserialize")
    }

    #[test]
    fn command_contexts_default_to_guild_only() {
        let definition = command(json!({
            "name": "hello",
            "description": "Says hello."
        }));

        assert_eq!(
            definition.resolved_integration_types(),
            vec![LuaInstallationContext::Guild]
        );
        assert_eq!(
            definition.resolved_contexts(),
            vec![LuaInteractionContext::Guild]
        );
        assert!(definition.validate_contexts().is_ok());
    }

    #[test]
    fn legacy_dm_permission_maps_without_serializing_the_deprecated_field() {
        let definition = command(json!({
            "name": "hello",
            "description": "Says hello.",
            "dm_permission": true
        }));

        assert_eq!(
            definition.resolved_contexts(),
            vec![LuaInteractionContext::Guild, LuaInteractionContext::BotDm]
        );
        assert!(definition.validate_contexts().is_ok());
    }

    #[test]
    fn private_channel_requires_user_installation() {
        let definition = command(json!({
            "name": "hello",
            "description": "Says hello.",
            "integration_types": ["guild"],
            "contexts": ["private_channel"]
        }));

        assert_eq!(
            definition.validate_contexts(),
            Err("private_channel context requires the user installation type".to_owned())
        );
    }

    #[test]
    fn contexts_cannot_be_mixed_with_legacy_dm_permission() {
        let definition = command(json!({
            "name": "hello",
            "description": "Says hello.",
            "contexts": ["guild"],
            "dm_permission": false
        }));

        assert!(definition.validate_contexts().is_err());
    }

    #[test]
    fn required_option_after_optional_is_rejected() {
        let invalid = command(json!({
            "name": "test",
            "description": "Test command.",
            "options": [
                { "type": "string", "name": "opt", "description": "optional", "required": false },
                { "type": "string", "name": "req", "description": "required", "required": true }
            ]
        }));
        assert!(invalid.validate_options_order().is_err());

        let valid = command(json!({
            "name": "test",
            "description": "Test command.",
            "options": [
                { "type": "string", "name": "req", "description": "required", "required": true },
                { "type": "string", "name": "opt", "description": "optional", "required": false }
            ]
        }));
        assert!(valid.validate_options_order().is_ok());
    }

    #[test]
    fn test_audit_action_validation_and_payload_limits() {
        use super::LuaAction;
        use std::collections::BTreeMap;

        let mut data = BTreeMap::new();
        data.insert("user_id".to_owned(), json!("123456789012345678"));
        data.insert("reason".to_owned(), json!("spamming"));

        let valid_audit = LuaAction::Audit {
            event: "test_event".to_owned(),
            data: data.clone(),
        };
        assert!(valid_audit.validate().is_ok());

        // Reject empty or whitespace-only event
        let empty_event = LuaAction::Audit {
            event: "   ".to_owned(),
            data: data.clone(),
        };
        assert!(empty_event.validate().is_err());

        // Reject > 64 keys
        let mut excessive_keys = BTreeMap::new();
        for i in 0..65 {
            excessive_keys.insert(format!("key_{i}"), json!("val"));
        }
        let too_many_keys = LuaAction::Audit {
            event: "test_event".to_owned(),
            data: excessive_keys,
        };
        assert!(too_many_keys.validate().is_err());

        // Reject > 16 KiB serialized JSON
        let mut large_data = BTreeMap::new();
        large_data.insert("large".to_owned(), json!("a".repeat(20_000)));
        let too_large = LuaAction::Audit {
            event: "test_event".to_owned(),
            data: large_data,
        };
        assert!(too_large.validate().is_err());
    }
}
