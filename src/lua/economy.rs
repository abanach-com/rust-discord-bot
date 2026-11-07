use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EconomyOperation {
    Work {
        #[serde(default)]
        roll: Option<f64>,
    },
    Crime {
        #[serde(default)]
        roll: Option<f64>,
    },
    Shop,
    Buy {
        item: String,
    },
    Inventory,
}

impl EconomyOperation {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Work { roll } | Self::Crime { roll } => {
                if let Some(r) = roll
                    && (!r.is_finite() || !(0.0..=1.0).contains(r))
                {
                    return Err("roll must be a finite number between 0.0 and 1.0".to_string());
                }
                Ok(())
            }
            Self::Shop | Self::Inventory => Ok(()),
            Self::Buy { item } => {
                if item.trim().is_empty() {
                    return Err("item reference cannot be empty".to_string());
                }
                if item.chars().count() > 100 {
                    return Err("item reference cannot exceed 100 characters".to_string());
                }
                Ok(())
            }
        }
    }
}
