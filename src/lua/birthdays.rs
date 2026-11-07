use serde::{Deserialize, Serialize};

use crate::storage::{validate_birthday_date, validate_snowflake};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum BirthdayOperation {
    Set {
        day: u8,
        month: u8,
    },
    Check {
        user_id: Option<String>,
    },
    Upcoming {
        #[serde(default = "default_upcoming_limit")]
        limit: usize,
    },
    Delete,
}

fn default_upcoming_limit() -> usize {
    10
}

impl BirthdayOperation {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Set { day, month } => {
                validate_birthday_date(*month as i16, *day as i16).map_err(|e| e.to_string())?;
                Ok(())
            }
            Self::Check { user_id } => {
                if let Some(uid) = user_id {
                    validate_snowflake("user_id", uid).map_err(|e| e.to_string())?;
                }
                Ok(())
            }
            Self::Upcoming { limit } => {
                if *limit == 0 || *limit > 50 {
                    return Err("limit must be between 1 and 50".to_owned());
                }
                Ok(())
            }
            Self::Delete => Ok(()),
        }
    }
}
