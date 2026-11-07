use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum ReactionRoleOperation {
    Create {
        message_id: String,
        emoji: String,
        role_id: String,
    },
    Remove {
        message_id: String,
        emoji: String,
    },
    List,
}

impl ReactionRoleOperation {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Create {
                message_id,
                emoji,
                role_id,
            } => {
                validate_snowflake(message_id)?;
                validate_snowflake(role_id)?;
                let trimmed = emoji.trim();
                if trimmed.is_empty() || trimmed.len() > 128 {
                    return Err("emoji must contain 1 to 128 characters".to_owned());
                }
                Ok(())
            }
            Self::Remove { message_id, emoji } => {
                validate_snowflake(message_id)?;
                let trimmed = emoji.trim();
                if trimmed.is_empty() || trimmed.len() > 128 {
                    return Err("emoji must contain 1 to 128 characters".to_owned());
                }
                Ok(())
            }
            Self::List => Ok(()),
        }
    }
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
