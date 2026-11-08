use serde::{Deserialize, Serialize};

use super::engine::LuaEngine;
use crate::storage::Storage;

/// Represents a routing decision for a Lua module handling a Discord event.
/// Contains strictly diagnostic metadata without exposing raw module configurations,
/// secrets, member details, or event payloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LuaEventRouteDecision {
    pub module_id: String,
    pub subscribed: bool,
    pub configured_enabled: bool,
    pub config_source: String,
    pub decision_reason: String,
}

/// Execution plan for a single Lua module on a given event.
/// Used both by the diagnostic endpoint and the live `run_event` dispatcher
/// to ensure strict behavioral parity.
#[derive(Debug, Clone)]
pub struct PlannedModuleExecution {
    pub module_id: String,
    pub decision: LuaEventRouteDecision,
    pub config: Option<serde_json::Value>,
}

/// Validates that an event name conforms to expected identifier rules.
/// Must be non-empty, at most 64 characters, and contain only lowercase ASCII
/// alphanumeric characters or underscores.
pub fn validate_event_name(event: &str) -> Result<(), &'static str> {
    if event.is_empty() {
        return Err("Event name cannot be empty");
    }
    if event.len() > 64 {
        return Err("Event name cannot exceed 64 characters");
    }
    if !event
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(
            "Event name must contain only lowercase ASCII alphanumeric characters or underscores",
        );
    }
    Ok(())
}

/// Computes the event routing plan for all modules registered for `event_name`.
///
/// If `guild_id` is provided, guild-specific overrides are evaluated from `storage`.
/// If no override exists, the module manifest's `default_enabled` is used.
/// In case of storage errors, the decision is fail-closed (`configured_enabled = false`,
/// `decision_reason = "storage_error"`).
pub async fn plan_event_routing(
    scripts: &LuaEngine,
    storage: &Storage,
    guild_id: Option<&str>,
    event_name: &str,
) -> Vec<PlannedModuleExecution> {
    let module_ids = scripts.modules_for_event(event_name).await;
    let mut plans = Vec::with_capacity(module_ids.len());

    for module_id in module_ids {
        let manifest = scripts.manifest(&module_id).await;

        let Some(manifest) = manifest else {
            plans.push(PlannedModuleExecution {
                module_id: module_id.clone(),
                decision: LuaEventRouteDecision {
                    module_id,
                    subscribed: true,
                    configured_enabled: false,
                    config_source: "manifest_default".to_string(),
                    decision_reason: "missing_manifest".to_string(),
                },
                config: None,
            });
            continue;
        };

        if let Some(gid) = guild_id {
            match storage.get_module_settings(gid, &module_id).await {
                Ok(Some(settings)) => {
                    let enabled = settings.enabled;
                    plans.push(PlannedModuleExecution {
                        module_id: module_id.clone(),
                        decision: LuaEventRouteDecision {
                            module_id,
                            subscribed: true,
                            configured_enabled: enabled,
                            config_source: "guild_override".to_string(),
                            decision_reason: if enabled {
                                "enabled".to_string()
                            } else {
                                "guild_disabled".to_string()
                            },
                        },
                        config: if enabled { Some(settings.config) } else { None },
                    });
                }
                Ok(None) => {
                    let enabled = manifest.default_enabled;
                    let config = if enabled {
                        Some(scripts.default_config(&module_id).await)
                    } else {
                        None
                    };
                    plans.push(PlannedModuleExecution {
                        module_id: module_id.clone(),
                        decision: LuaEventRouteDecision {
                            module_id,
                            subscribed: true,
                            configured_enabled: enabled,
                            config_source: "manifest_default".to_string(),
                            decision_reason: if enabled {
                                "enabled".to_string()
                            } else {
                                "default_disabled".to_string()
                            },
                        },
                        config,
                    });
                }
                Err(error) => {
                    tracing::error!(
                        module_id = %module_id,
                        event_name = %event_name,
                        guild_id = %gid,
                        ?error,
                        "cannot load module settings for event routing plan"
                    );
                    plans.push(PlannedModuleExecution {
                        module_id: module_id.clone(),
                        decision: LuaEventRouteDecision {
                            module_id,
                            subscribed: true,
                            configured_enabled: false,
                            config_source: "guild_override".to_string(),
                            decision_reason: "storage_error".to_string(),
                        },
                        config: None,
                    });
                }
            }
        } else {
            let enabled = manifest.default_enabled;
            let config = if enabled {
                Some(scripts.default_config(&module_id).await)
            } else {
                None
            };
            plans.push(PlannedModuleExecution {
                module_id: module_id.clone(),
                decision: LuaEventRouteDecision {
                    module_id,
                    subscribed: true,
                    configured_enabled: enabled,
                    config_source: "manifest_default".to_string(),
                    decision_reason: if enabled {
                        "enabled".to_string()
                    } else {
                        "default_disabled".to_string()
                    },
                },
                config,
            });
        }
    }

    plans
}
