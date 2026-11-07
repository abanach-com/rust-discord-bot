use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::storage::validate_snowflake;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum QuoteOperation {
    Add {
        author_id: String,
        text: String,
    },
    Random,
    List {
        #[serde(default = "default_list_limit")]
        limit: usize,
    },
    Delete {
        quote_id: Uuid,
    },
}

fn default_list_limit() -> usize {
    10
}

impl QuoteOperation {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Add { author_id, text } => {
                validate_snowflake("author_id", author_id).map_err(|e| e.to_string())?;
                let trimmed = text.trim();
                let len = trimmed.chars().count();
                if len == 0 || len > 500 {
                    return Err("quote text must contain 1 to 500 characters".to_owned());
                }
                Ok(())
            }
            Self::Random => Ok(()),
            Self::List { limit } => {
                if *limit == 0 || *limit > 100 {
                    return Err("limit must be between 1 and 100".to_owned());
                }
                Ok(())
            }
            Self::Delete { .. } => Ok(()),
        }
    }
}
