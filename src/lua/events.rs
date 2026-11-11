use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::storage::validate_snowflake;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum EventVoiceQueueOperation {
    RequestReady {
        event_id: Uuid,
        user_id: String,
        #[serde(default)]
        consent_auto_move: bool,
    },
}

impl EventVoiceQueueOperation {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::RequestReady { user_id, .. } => {
                validate_snowflake("user_id", user_id).map_err(|e| e.to_string())?;
                Ok(())
            }
        }
    }
}
