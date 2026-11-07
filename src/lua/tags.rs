use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum TagOperation {
    Get { name: String },
    Create { name: String, content: String },
    Delete { name: String },
}

impl TagOperation {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Get { name } | Self::Delete { name } => validate_tag_name(name),
            Self::Create { name, content } => {
                validate_tag_name(name)?;
                let trimmed = content.trim();
                let len = trimmed.chars().count();
                if len == 0 || len > 1500 {
                    return Err("tag content must contain 1 to 1500 characters".to_owned());
                }
                Ok(())
            }
        }
    }
}

pub fn normalize_tag_name(name: &str) -> String {
    name.trim().to_lowercase()
}

pub fn validate_tag_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    let len = trimmed.chars().count();
    if len == 0 || len > 64 {
        return Err("tag name must contain 1 to 64 characters".to_owned());
    }
    if !trimmed
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(
            "tag name may only contain alphanumeric characters, hyphens, underscores, or dots"
                .to_owned(),
        );
    }
    Ok(())
}
