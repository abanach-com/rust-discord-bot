use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum StarboardOperation {
    Configure {
        channel_id: String,
        #[serde(default = "default_threshold")]
        threshold: u32,
    },
}

fn default_threshold() -> u32 {
    3
}

impl StarboardOperation {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Configure {
                channel_id,
                threshold,
            } => {
                validate_snowflake(channel_id)?;
                if !(1..=25).contains(threshold) {
                    return Err(format!("threshold {threshold} must be between 1 and 25"));
                }
                Ok(())
            }
        }
    }
}

fn validate_snowflake(value: &str) -> Result<(), String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| format!("{value:?} is not a Discord snowflake"))?;
    if parsed == 0 {
        return Err("snowflake cannot be 0".to_owned());
    }
    Ok(())
}
