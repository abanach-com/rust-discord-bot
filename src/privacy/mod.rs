use serde::Serialize;

use crate::storage::RetainedRecordNotice;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyAction {
    Delete,
    Pseudonymize,
    Retain { days: u32, reason: &'static str },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrivacyDataset {
    pub key: &'static str,
    pub display_name: &'static str,
    pub owner_module: &'static str,
    pub storage_table: &'static str,
    pub subject_fields: &'static [&'static str],
    pub action: PrivacyAction,
    pub is_exportable: bool,
    pub export_key: Option<&'static str>,
    pub description: &'static str,
}

pub const PRIVACY_DATASETS: &[PrivacyDataset] = &[
    PrivacyDataset {
        key: "member_progress",
        display_name: "Level progression",
        owner_module: "levels",
        storage_table: "member_progress",
        subject_fields: &["user_id"],
        action: PrivacyAction::Delete,
        is_exportable: true,
        export_key: Some("progression"),
        description: "User experience points, level progression and message counters",
    },
    PrivacyDataset {
        key: "guild_quotes",
        display_name: "Quotes",
        owner_module: "quotes",
        storage_table: "guild_quotes",
        subject_fields: &["added_by", "author_id"],
        action: PrivacyAction::Delete,
        is_exportable: true,
        export_key: Some("quotes"),
        description: "User submitted or authored quotes within the guild",
    },
    PrivacyDataset {
        key: "guild_birthdays",
        display_name: "Birthdays",
        owner_module: "birthdays",
        storage_table: "guild_birthdays",
        subject_fields: &["user_id"],
        action: PrivacyAction::Delete,
        is_exportable: true,
        export_key: Some("birthday"),
        description: "User registered birthday (day, month, optional year)",
    },
    PrivacyDataset {
        key: "member_afk",
        display_name: "AFK status",
        owner_module: "afk",
        storage_table: "member_afk",
        subject_fields: &["user_id"],
        action: PrivacyAction::Delete,
        is_exportable: true,
        export_key: Some("afk"),
        description: "User AFK status, reason and timestamp",
    },
    PrivacyDataset {
        key: "reaction_role_grants",
        display_name: "Reaction role grants",
        owner_module: "reaction_roles",
        storage_table: "reaction_role_grants",
        subject_fields: &["user_id"],
        action: PrivacyAction::Delete,
        is_exportable: true,
        export_key: Some("reaction_roles"),
        description: "Ledger of active roles granted to user via reactions",
    },
    PrivacyDataset {
        key: "moderation_cases",
        display_name: "Moderation cases",
        owner_module: "moderation",
        storage_table: "moderation_cases",
        subject_fields: &["subject_user_id"],
        action: PrivacyAction::Retain {
            days: 365,
            reason: "Server security, sanctions audit and anti-abuse accountability",
        },
        is_exportable: true,
        export_key: Some("moderation_cases"),
        description: "Historical sanctions, warnings, mutes, kicks and bans",
    },
    PrivacyDataset {
        key: "audit_logs",
        display_name: "Audit log entries",
        owner_module: "audit",
        storage_table: "audit_logs",
        subject_fields: &["actor_id"],
        action: PrivacyAction::Retain {
            days: 45,
            reason: "Administrative action audit trail and server governance",
        },
        is_exportable: false,
        export_key: None,
        description: "Administrative actions performed by users and moderators",
    },
    PrivacyDataset {
        key: "economy_cooldowns",
        display_name: "Economy cooldowns",
        owner_module: "economy",
        storage_table: "economy_cooldowns",
        subject_fields: &["user_id"],
        action: PrivacyAction::Delete,
        is_exportable: false,
        export_key: None,
        description: "Work and crime rate limit cooldown tracking",
    },
    PrivacyDataset {
        key: "economy_idempotency",
        display_name: "Economy idempotency",
        owner_module: "economy",
        storage_table: "economy_idempotency",
        subject_fields: &["user_id"],
        action: PrivacyAction::Delete,
        is_exportable: false,
        export_key: None,
        description: "Deduplication keys for recent economy claims and transactions",
    },
];

pub fn get_dataset_by_key(key: &str) -> Option<&'static PrivacyDataset> {
    PRIVACY_DATASETS.iter().find(|ds| ds.key == key)
}

pub fn list_exportable_datasets() -> impl Iterator<Item = &'static PrivacyDataset> {
    PRIVACY_DATASETS.iter().filter(|ds| ds.is_exportable)
}

pub fn list_deletable_datasets() -> impl Iterator<Item = &'static PrivacyDataset> {
    PRIVACY_DATASETS
        .iter()
        .filter(|ds| matches!(ds.action, PrivacyAction::Delete))
}

pub fn list_retained_datasets() -> impl Iterator<Item = &'static PrivacyDataset> {
    PRIVACY_DATASETS
        .iter()
        .filter(|ds| matches!(ds.action, PrivacyAction::Retain { .. }))
}

pub fn build_default_retained_notices() -> Vec<RetainedRecordNotice> {
    PRIVACY_DATASETS
        .iter()
        .filter_map(|ds| match ds.action {
            PrivacyAction::Retain { days, reason } => Some(RetainedRecordNotice {
                entity: ds.display_name.to_string(),
                reason: reason.to_string(),
                retention_policy: format!("{days} days"),
            }),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_privacy_datasets_have_unique_keys() {
        let mut keys = std::collections::HashSet::new();
        for ds in PRIVACY_DATASETS {
            assert!(
                keys.insert(ds.key),
                "Duplicate privacy dataset key: {}",
                ds.key
            );
        }
    }

    #[test]
    fn test_exportable_datasets_have_export_keys() {
        for ds in list_exportable_datasets() {
            assert!(
                ds.export_key.is_some(),
                "Exportable dataset {} must specify export_key",
                ds.key
            );
        }
    }

    #[test]
    fn test_quotes_covers_both_added_by_and_author_id() {
        let quotes = get_dataset_by_key("guild_quotes").expect("guild_quotes dataset");
        assert!(quotes.subject_fields.contains(&"added_by"));
        assert!(quotes.subject_fields.contains(&"author_id"));
    }

    #[test]
    fn test_retained_notices_generation() {
        let notices = build_default_retained_notices();
        assert_eq!(notices.len(), 2);
        assert!(notices.iter().any(|n| n.entity == "Moderation cases"));
        assert!(notices.iter().any(|n| n.entity == "Audit log entries"));
    }
}
