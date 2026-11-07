use serde::{Deserialize, Serialize};

use crate::storage::validate_snowflake;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum AfkOperation {
    Set {
        #[serde(default)]
        reason: Option<String>,
    },
    Clear,
    Status {
        #[serde(default)]
        user_id: Option<String>,
    },
    OnMessage {
        author_id: String,
        #[serde(default)]
        mentioned_user_ids: Vec<String>,
        #[serde(default = "default_true")]
        auto_clear: bool,
    },
}

fn default_true() -> bool {
    true
}

impl AfkOperation {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Set { reason } => {
                if let Some(r) = reason
                    && r.len() > 500
                {
                    return Err("reason length cannot exceed 500 characters".to_owned());
                }
                Ok(())
            }
            Self::Clear => Ok(()),
            Self::Status { user_id } => {
                if let Some(uid) = user_id {
                    validate_snowflake("user_id", uid).map_err(|e| e.to_string())?;
                }
                Ok(())
            }
            Self::OnMessage {
                author_id,
                mentioned_user_ids,
                ..
            } => {
                validate_snowflake("author_id", author_id).map_err(|e| e.to_string())?;
                for mid in mentioned_user_ids {
                    validate_snowflake("mentioned_user_id", mid).map_err(|e| e.to_string())?;
                }
                Ok(())
            }
        }
    }
}
