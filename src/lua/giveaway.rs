use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum GiveawayOperation {
    Start {
        prize: String,
        #[serde(default = "default_giveaway_duration")]
        duration_minutes: i64,
        #[serde(default = "default_giveaway_winners")]
        winners: i32,
        #[serde(default)]
        channel_id: Option<String>,
    },
    End {
        giveaway_id: String,
    },
    Reroll {
        giveaway_id: String,
    },
}

impl GiveawayOperation {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Start {
                prize,
                duration_minutes,
                winners,
                channel_id,
            } => {
                let prize_len = prize.trim().chars().count();
                if prize_len == 0 || prize_len > 256 {
                    return Err("giveaway prize must contain 1 to 256 characters".to_owned());
                }
                if *duration_minutes < 1 || *duration_minutes > 10080 {
                    return Err(
                        "giveaway duration_minutes must be between 1 and 10080 (7 days)".to_owned(),
                    );
                }
                if *winners < 1 || *winners > 20 {
                    return Err("giveaway winners must be between 1 and 20".to_owned());
                }
                if let Some(channel_id) = channel_id {
                    validate_snowflake(channel_id)?;
                }
                Ok(())
            }
            Self::End { giveaway_id } | Self::Reroll { giveaway_id } => {
                validate_giveaway_id(giveaway_id)
            }
        }
    }
}

fn validate_giveaway_id(value: &str) -> Result<(), String> {
    Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| format!("{value:?} is not a valid UUID for giveaway_id"))
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

fn default_giveaway_duration() -> i64 {
    60
}

fn default_giveaway_winners() -> i32 {
    1
}
