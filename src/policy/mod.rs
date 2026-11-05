use std::sync::Arc;

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::storage::validate_snowflake;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicySubject {
    User { user_id: String },
    Role { role_id: String },
    Channel { channel_id: String },
    VoiceChannel { channel_id: String },
    Guild,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyResource {
    Command {
        module_id: String,
        command_name: String,
    },
    Module {
        module_id: String,
    },
    Action {
        action_key: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyEffect {
    Allow,
    Deny,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct CommandPolicyRule {
    pub id: Uuid,
    pub guild_id: String,
    pub subject_kind: String,
    pub subject_id: Option<String>,
    pub resource_kind: String,
    pub module_id: Option<String>,
    pub command_name: Option<String>,
    pub action_key: Option<String>,
    pub effect: String,
    pub priority: i32,
    pub enabled: bool,
    pub version: i64,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub fn validate_policy_rule_fields(rule: &CommandPolicyRule) -> anyhow::Result<()> {
    validate_snowflake("guild_id", &rule.guild_id)?;
    let subject_kind = rule.subject_kind.to_lowercase();
    if !["user", "role", "channel", "voice_channel", "guild"].contains(&subject_kind.as_str()) {
        anyhow::bail!("Invalid subject_kind: {}", rule.subject_kind);
    }
    if subject_kind == "guild" && rule.subject_id.is_some() {
        anyhow::bail!("Guild subject must not specify subject_id");
    }
    if subject_kind != "guild" && rule.subject_id.is_none() {
        anyhow::bail!("{subject_kind} subject requires subject_id");
    }
    if let Some(ref sid) = rule.subject_id {
        validate_snowflake("subject_id", sid)?;
    }

    let resource_kind = rule.resource_kind.to_lowercase();
    if !["command", "module", "action"].contains(&resource_kind.as_str()) {
        anyhow::bail!("Invalid resource_kind: {}", rule.resource_kind);
    }
    if resource_kind == "command" && (rule.module_id.is_none() || rule.command_name.is_none()) {
        anyhow::bail!("Command resource requires both module_id and command_name");
    }
    if resource_kind == "module" && rule.module_id.is_none() {
        anyhow::bail!("Module resource requires module_id");
    }
    if resource_kind == "action" && rule.action_key.is_none() {
        anyhow::bail!("Action resource requires action_key");
    }

    let effect = rule.effect.to_lowercase();
    if effect != "allow" && effect != "deny" {
        anyhow::bail!("Effect must be 'allow' or 'deny', got '{}'", rule.effect);
    }
    Ok(())
}

impl CommandPolicyRule {
    pub fn new_user_rule(
        guild_id: String,
        user_id: String,
        resource: PolicyResource,
        effect: PolicyEffect,
        priority: i32,
        created_by: String,
    ) -> anyhow::Result<Self> {
        validate_snowflake("guild_id", &guild_id)?;
        validate_snowflake("user_id", &user_id)?;
        Self::build_rule(
            guild_id,
            "user",
            Some(user_id),
            resource,
            effect,
            priority,
            created_by,
        )
    }

    pub fn new_role_rule(
        guild_id: String,
        role_id: String,
        resource: PolicyResource,
        effect: PolicyEffect,
        priority: i32,
        created_by: String,
    ) -> anyhow::Result<Self> {
        validate_snowflake("guild_id", &guild_id)?;
        validate_snowflake("role_id", &role_id)?;
        Self::build_rule(
            guild_id,
            "role",
            Some(role_id),
            resource,
            effect,
            priority,
            created_by,
        )
    }

    pub fn new_channel_rule(
        guild_id: String,
        channel_id: String,
        resource: PolicyResource,
        effect: PolicyEffect,
        priority: i32,
        created_by: String,
    ) -> anyhow::Result<Self> {
        validate_snowflake("guild_id", &guild_id)?;
        validate_snowflake("channel_id", &channel_id)?;
        Self::build_rule(
            guild_id,
            "channel",
            Some(channel_id),
            resource,
            effect,
            priority,
            created_by,
        )
    }

    pub fn new_guild_rule(
        guild_id: String,
        resource: PolicyResource,
        effect: PolicyEffect,
        priority: i32,
        created_by: String,
    ) -> anyhow::Result<Self> {
        validate_snowflake("guild_id", &guild_id)?;
        Self::build_rule(
            guild_id, "guild", None, resource, effect, priority, created_by,
        )
    }

    fn build_rule(
        guild_id: String,
        subject_kind: &str,
        subject_id: Option<String>,
        resource: PolicyResource,
        effect: PolicyEffect,
        priority: i32,
        created_by: String,
    ) -> anyhow::Result<Self> {
        let (resource_kind, module_id, command_name, action_key) = match resource {
            PolicyResource::Command {
                module_id,
                command_name,
            } => ("command", Some(module_id), Some(command_name), None),
            PolicyResource::Module { module_id } => ("module", Some(module_id), None, None),
            PolicyResource::Action { action_key } => ("action", None, None, Some(action_key)),
        };

        let now = Utc::now();
        Ok(Self {
            id: Uuid::new_v4(),
            guild_id,
            subject_kind: subject_kind.to_string(),
            subject_id,
            resource_kind: resource_kind.to_string(),
            module_id,
            command_name,
            action_key,
            effect: match effect {
                PolicyEffect::Allow => "allow".to_string(),
                PolicyEffect::Deny => "deny".to_string(),
            },
            priority,
            enabled: true,
            version: 1,
            created_by,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn effect_enum(&self) -> PolicyEffect {
        if self.effect.eq_ignore_ascii_case("deny") {
            PolicyEffect::Deny
        } else {
            PolicyEffect::Allow
        }
    }

    pub fn matches_resource(&self, target: &PolicyResource) -> bool {
        match target {
            PolicyResource::Command {
                module_id,
                command_name,
            } => match self.resource_kind.as_str() {
                "command" => {
                    self.module_id.as_deref() == Some(module_id)
                        && self.command_name.as_deref() == Some(command_name)
                }
                "module" => self.module_id.as_deref() == Some(module_id),
                _ => false,
            },
            PolicyResource::Module { module_id } => match self.resource_kind.as_str() {
                "module" => self.module_id.as_deref() == Some(module_id),
                _ => false,
            },
            PolicyResource::Action { action_key } => match self.resource_kind.as_str() {
                "action" => self.action_key.as_deref() == Some(action_key),
                _ => false,
            },
        }
    }

    /// Evaluates if subject matches and returns (tier, position_weight).
    /// Tier ranking:
    /// 5 = Explicit User
    /// 4 = Direct Invocation Channel
    /// 3 = Inherited Thread Parent Channel
    /// 2 = Voice Context Channel
    /// 1 = Role Match (position_weight = role position in hierarchy)
    /// 0 = Guild Default
    pub fn match_subject_tier(&self, ctx: &PolicyContext) -> Option<(u8, i64)> {
        match self.subject_kind.as_str() {
            "user" => {
                if self.subject_id.as_deref() == Some(&ctx.user_id) {
                    Some((5, 0))
                } else {
                    None
                }
            }
            "channel" => {
                if self.subject_id.as_deref() == Some(&ctx.channel_id) {
                    Some((4, 0))
                } else if let Some(ref parent) = ctx.thread_parent_id {
                    if self.subject_id.as_deref() == Some(parent) {
                        Some((3, 0))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            "voice_channel" => {
                if let Some(ref voice) = ctx.voice_channel_id {
                    if self.subject_id.as_deref() == Some(voice) {
                        Some((2, 0))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            "role" => {
                if let Some(ref role_id) = self.subject_id {
                    ctx.roles
                        .iter()
                        .find(|(r, _)| r == role_id)
                        .map(|(_, pos)| (1, *pos))
                } else {
                    None
                }
            }
            "guild" => Some((0, 0)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PolicyContext {
    pub guild_id: String,
    pub user_id: String,
    pub roles: Vec<(String, i64)>, // (role_id, position)
    pub channel_id: String,
    pub thread_parent_id: Option<String>,
    pub voice_channel_id: Option<String>,
    pub is_guild_owner: bool,
    pub is_administrator: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolicyEvaluationStep {
    pub step: String,
    pub description: String,
    pub matched_rule_id: Option<Uuid>,
    pub effect: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolicyEvaluationOutcome {
    pub allowed: bool,
    pub matched_rule_id: Option<Uuid>,
    pub reason: String,
    pub trace: Vec<PolicyEvaluationStep>,
}

#[derive(Clone)]
pub struct CommandPolicyEngine {
    cache: Arc<DashMap<String, (i64, Vec<CommandPolicyRule>)>>,
}

impl Default for CommandPolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandPolicyEngine {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(DashMap::new()),
        }
    }

    pub fn invalidate(&self, guild_id: &str) {
        self.cache.remove(guild_id);
    }

    pub fn set_cached_rules(&self, guild_id: &str, revision: i64, rules: Vec<CommandPolicyRule>) {
        self.cache.insert(guild_id.to_string(), (revision, rules));
    }

    pub fn get_cached_rules(&self, guild_id: &str) -> Option<(i64, Vec<CommandPolicyRule>)> {
        self.cache.get(guild_id).map(|r| r.value().clone())
    }

    /// Evaluates permission to execute a resource in context.
    /// Implements deterministic precedence:
    /// 1. System Safety Floor (Lockout protection for policy recovery commands by guild owner/admin)
    /// 2. Specificity tiers: User (5) > Direct Channel (4) > Thread Parent (3) > Voice Context (2) > Highest Matching Role (1) > Guild Default (0)
    /// 3. Within same tier: higher priority wins. If same priority: Command > Action > Module.
    pub fn evaluate(
        &self,
        ctx: &PolicyContext,
        resource: &PolicyResource,
        rules: &[CommandPolicyRule],
        module_enabled: bool,
    ) -> PolicyEvaluationOutcome {
        let mut trace = Vec::new();

        // 1. Lockout Protection: guild owner or administrator can always run policy recovery
        if is_policy_recovery_command(resource) && (ctx.is_guild_owner || ctx.is_administrator) {
            trace.push(PolicyEvaluationStep {
                step: "system_safety_floor".to_string(),
                description: "Guild owner/administrator lockout protection granted access to policy recovery tool".to_string(),
                matched_rule_id: None,
                effect: Some("allow".to_string()),
            });
            return PolicyEvaluationOutcome {
                allowed: true,
                matched_rule_id: None,
                reason: "Lockout protection: guild owner/administrator access granted for policy recovery"
                    .to_string(),
                trace,
            };
        }

        // 2. Module Disabled Floor: a disabled module blocks commands regardless of lower-tier allows
        if !module_enabled {
            trace.push(PolicyEvaluationStep {
                step: "module_state".to_string(),
                description: "Module is disabled for this guild".to_string(),
                matched_rule_id: None,
                effect: Some("deny".to_string()),
            });
            return PolicyEvaluationOutcome {
                allowed: false,
                matched_rule_id: None,
                reason: "This module is disabled for the server".to_string(),
                trace,
            };
        }

        // 3. Filter enabled matching rules
        struct Candidate<'a> {
            rule: &'a CommandPolicyRule,
            tier: u8,
            role_position: i64,
            priority: i32,
            resource_specificity: u8, // 3 = Command, 2 = Action, 1 = Module
        }

        let mut candidates = Vec::new();

        for rule in rules {
            if !rule.enabled || rule.guild_id != ctx.guild_id {
                continue;
            }

            if !rule.matches_resource(resource) {
                continue;
            }

            if let Some((tier, role_pos)) = rule.match_subject_tier(ctx) {
                let r_spec = match rule.resource_kind.as_str() {
                    "command" => 3,
                    "action" => 2,
                    _ => 1,
                };
                candidates.push(Candidate {
                    rule,
                    tier,
                    role_position: role_pos,
                    priority: rule.priority,
                    resource_specificity: r_spec,
                });
            }
        }

        if candidates.is_empty() {
            trace.push(PolicyEvaluationStep {
                step: "default".to_string(),
                description: "No specific policy rules matched; defaulting to allow".to_string(),
                matched_rule_id: None,
                effect: Some("allow".to_string()),
            });
            return PolicyEvaluationOutcome {
                allowed: true,
                matched_rule_id: None,
                reason: "Allowed by default (no conflicting policy rules)".to_string(),
                trace,
            };
        }

        // Sort candidates deterministically:
        // 1. Tier descending (User > Direct Channel > Thread Parent > Voice Context > Role > Guild)
        // 2. Role position descending (if tier == Role)
        // 3. Priority descending
        // 4. Resource specificity descending (Command > Action > Module)
        candidates.sort_by(|a, b| {
            b.tier
                .cmp(&a.tier)
                .then_with(|| b.role_position.cmp(&a.role_position))
                .then_with(|| b.priority.cmp(&a.priority))
                .then_with(|| b.resource_specificity.cmp(&a.resource_specificity))
        });

        let winning = &candidates[0];
        let allowed = winning.rule.effect_enum() == PolicyEffect::Allow;

        for (idx, c) in candidates.iter().enumerate() {
            let tier_name = match c.tier {
                5 => "explicit_user",
                4 => "invocation_channel",
                3 => "thread_parent_channel",
                2 => "voice_channel",
                1 => "role",
                _ => "guild_default",
            };
            trace.push(PolicyEvaluationStep {
                step: format!("candidate_{idx}_{tier_name}"),
                description: format!(
                    "Rule {} (tier={}, priority={}, resource={}) -> {}",
                    c.rule.id, c.tier, c.priority, c.rule.resource_kind, c.rule.effect
                ),
                matched_rule_id: Some(c.rule.id),
                effect: Some(c.rule.effect.clone()),
            });
        }

        PolicyEvaluationOutcome {
            allowed,
            matched_rule_id: Some(winning.rule.id),
            reason: if allowed {
                format!(
                    "Allowed by rule {} ({}, priority {})",
                    winning.rule.id, winning.rule.subject_kind, winning.priority
                )
            } else {
                format!(
                    "Denied by policy rule {} ({}, priority {})",
                    winning.rule.id, winning.rule.subject_kind, winning.priority
                )
            },
            trace,
        }
    }
}

fn is_policy_recovery_command(resource: &PolicyResource) -> bool {
    match resource {
        PolicyResource::Command {
            module_id,
            command_name,
        } => {
            module_id == "policy"
                && (command_name == "reset"
                    || command_name == "explain"
                    || command_name == "canrun")
        }
        PolicyResource::Action { action_key } => {
            action_key == "policy:reset" || action_key == "policy:explain"
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_deny_beats_role_allow() {
        let engine = CommandPolicyEngine::new();
        let guild_id = "100000000000000001";
        let user_id = "200000000000000001";
        let role_vip = "300000000000000001";
        let channel_id = "400000000000000001";

        let resource = PolicyResource::Command {
            module_id: "music".to_string(),
            command_name: "play".to_string(),
        };

        let user_deny = CommandPolicyRule::new_user_rule(
            guild_id.to_string(),
            user_id.to_string(),
            resource.clone(),
            PolicyEffect::Deny,
            0,
            "admin".to_string(),
        )
        .unwrap();

        let role_allow = CommandPolicyRule::new_role_rule(
            guild_id.to_string(),
            role_vip.to_string(),
            resource.clone(),
            PolicyEffect::Allow,
            10, // Higher priority role allow
            "admin".to_string(),
        )
        .unwrap();

        let ctx = PolicyContext {
            guild_id: guild_id.to_string(),
            user_id: user_id.to_string(),
            roles: vec![(role_vip.to_string(), 100)],
            channel_id: channel_id.to_string(),
            thread_parent_id: None,
            voice_channel_id: None,
            is_guild_owner: false,
            is_administrator: false,
        };

        let outcome = engine.evaluate(&ctx, &resource, &[role_allow, user_deny.clone()], true);
        assert!(
            !outcome.allowed,
            "User deny must beat role allow despite lower priority"
        );
        assert_eq!(outcome.matched_rule_id, Some(user_deny.id));
    }

    #[test]
    fn test_thread_channel_inheritance_and_override() {
        let engine = CommandPolicyEngine::new();
        let guild_id = "100000000000000001";
        let user_id = "200000000000000001";
        let parent_channel = "400000000000000001";
        let thread_channel = "400000000000000002";

        let resource = PolicyResource::Command {
            module_id: "fun".to_string(),
            command_name: "meme".to_string(),
        };

        // 1. Parent channel deny rule
        let parent_deny = CommandPolicyRule::new_channel_rule(
            guild_id.to_string(),
            parent_channel.to_string(),
            resource.clone(),
            PolicyEffect::Deny,
            0,
            "admin".to_string(),
        )
        .unwrap();

        let ctx_in_thread = PolicyContext {
            guild_id: guild_id.to_string(),
            user_id: user_id.to_string(),
            roles: vec![],
            channel_id: thread_channel.to_string(),
            thread_parent_id: Some(parent_channel.to_string()),
            voice_channel_id: None,
            is_guild_owner: false,
            is_administrator: false,
        };

        // Without direct thread rule: inherits parent deny
        let outcome1 = engine.evaluate(
            &ctx_in_thread,
            &resource,
            std::slice::from_ref(&parent_deny),
            true,
        );
        assert!(
            !outcome1.allowed,
            "Thread should inherit parent channel deny"
        );

        // 2. Add explicit thread allow rule
        let thread_allow = CommandPolicyRule::new_channel_rule(
            guild_id.to_string(),
            thread_channel.to_string(),
            resource.clone(),
            PolicyEffect::Allow,
            0,
            "admin".to_string(),
        )
        .unwrap();

        // Direct thread rule wins over parent channel
        let outcome2 = engine.evaluate(
            &ctx_in_thread,
            &resource,
            &[parent_deny, thread_allow.clone()],
            true,
        );
        assert!(
            outcome2.allowed,
            "Direct thread rule must override parent channel rule"
        );
        assert_eq!(outcome2.matched_rule_id, Some(thread_allow.id));
    }

    #[test]
    fn test_guild_owner_lockout_protection() {
        let engine = CommandPolicyEngine::new();
        let guild_id = "100000000000000001";
        let owner_id = "200000000000000001";

        let recovery_resource = PolicyResource::Command {
            module_id: "policy".to_string(),
            command_name: "reset".to_string(),
        };

        // Hostile rule trying to deny policy reset to everyone
        let deny_all = CommandPolicyRule::new_guild_rule(
            guild_id.to_string(),
            recovery_resource.clone(),
            PolicyEffect::Deny,
            100,
            "attacker".to_string(),
        )
        .unwrap();

        let owner_ctx = PolicyContext {
            guild_id: guild_id.to_string(),
            user_id: owner_id.to_string(),
            roles: vec![],
            channel_id: "400000000000000001".to_string(),
            thread_parent_id: None,
            voice_channel_id: None,
            is_guild_owner: true,
            is_administrator: true,
        };

        let outcome = engine.evaluate(&owner_ctx, &recovery_resource, &[deny_all], true);
        assert!(
            outcome.allowed,
            "Guild owner must never be locked out of policy recovery"
        );
    }

    #[test]
    fn test_disabled_module_beats_command_allow() {
        let engine = CommandPolicyEngine::new();
        let guild_id = "100000000000000001";
        let user_id = "200000000000000001";

        let resource = PolicyResource::Command {
            module_id: "tickets".to_string(),
            command_name: "open".to_string(),
        };

        let user_allow = CommandPolicyRule::new_user_rule(
            guild_id.to_string(),
            user_id.to_string(),
            resource.clone(),
            PolicyEffect::Allow,
            100,
            "admin".to_string(),
        )
        .unwrap();

        let ctx = PolicyContext {
            guild_id: guild_id.to_string(),
            user_id: user_id.to_string(),
            roles: vec![],
            channel_id: "400000000000000001".to_string(),
            thread_parent_id: None,
            voice_channel_id: None,
            is_guild_owner: false,
            is_administrator: false,
        };

        // Module is disabled at guild level
        let outcome = engine.evaluate(&ctx, &resource, &[user_allow], false);
        assert!(
            !outcome.allowed,
            "Disabled module must beat command allow rule"
        );
        assert!(outcome.reason.contains("disabled"));
    }

    #[test]
    fn test_higher_role_precedence() {
        let engine = CommandPolicyEngine::new();
        let guild_id = "100000000000000001";
        let user_id = "200000000000000001";
        let role_junior = "300000000000000001";
        let role_senior = "300000000000000002";

        let resource = PolicyResource::Command {
            module_id: "moderation".to_string(),
            command_name: "mute".to_string(),
        };

        // Junior role (pos 10) allows mute
        let junior_allow = CommandPolicyRule::new_role_rule(
            guild_id.to_string(),
            role_junior.to_string(),
            resource.clone(),
            PolicyEffect::Allow,
            100, // even with higher rule priority
            "admin".to_string(),
        )
        .unwrap();

        // Senior role (pos 50) denies mute
        let senior_deny = CommandPolicyRule::new_role_rule(
            guild_id.to_string(),
            role_senior.to_string(),
            resource.clone(),
            PolicyEffect::Deny,
            0,
            "admin".to_string(),
        )
        .unwrap();

        let ctx = PolicyContext {
            guild_id: guild_id.to_string(),
            user_id: user_id.to_string(),
            roles: vec![(role_junior.to_string(), 10), (role_senior.to_string(), 50)],
            channel_id: "400000000000000001".to_string(),
            thread_parent_id: None,
            voice_channel_id: None,
            is_guild_owner: false,
            is_administrator: false,
        };

        let outcome = engine.evaluate(&ctx, &resource, &[junior_allow, senior_deny.clone()], true);
        assert!(
            !outcome.allowed,
            "Higher role position must win over lower role position"
        );
        assert_eq!(outcome.matched_rule_id, Some(senior_deny.id));
    }

    #[test]
    fn test_owner_can_deny_self_normal_commands() {
        let engine = CommandPolicyEngine::new();
        let guild_id = "100000000000000001";
        let owner_id = "200000000000000001";

        let resource = PolicyResource::Command {
            module_id: "moderation".to_string(),
            command_name: "ban".to_string(),
        };

        let user_deny = CommandPolicyRule::new_user_rule(
            guild_id.to_string(),
            owner_id.to_string(),
            resource.clone(),
            PolicyEffect::Deny,
            0,
            "admin".to_string(),
        )
        .unwrap();

        let owner_ctx = PolicyContext {
            guild_id: guild_id.to_string(),
            user_id: owner_id.to_string(),
            roles: vec![],
            channel_id: "400000000000000001".to_string(),
            thread_parent_id: None,
            voice_channel_id: None,
            is_guild_owner: true,
            is_administrator: true,
        };

        let outcome = engine.evaluate(
            &owner_ctx,
            &resource,
            std::slice::from_ref(&user_deny),
            true,
        );
        assert!(
            !outcome.allowed,
            "Owner can voluntarily deny themselves normal/destructive commands"
        );
        assert_eq!(outcome.matched_rule_id, Some(user_deny.id));
    }

    #[test]
    fn test_guild_default_acts_only_as_fallback() {
        let engine = CommandPolicyEngine::new();
        let guild_id = "100000000000000001";
        let user_id = "200000000000000001";
        let role_id = "300000000000000001";

        let resource = PolicyResource::Command {
            module_id: "utility".to_string(),
            command_name: "ping".to_string(),
        };

        let guild_allow = CommandPolicyRule::new_guild_rule(
            guild_id.to_string(),
            resource.clone(),
            PolicyEffect::Allow,
            100, // High priority guild rule
            "admin".to_string(),
        )
        .unwrap();

        let role_deny = CommandPolicyRule::new_role_rule(
            guild_id.to_string(),
            role_id.to_string(),
            resource.clone(),
            PolicyEffect::Deny,
            0, // Low priority role rule
            "admin".to_string(),
        )
        .unwrap();

        let ctx = PolicyContext {
            guild_id: guild_id.to_string(),
            user_id: user_id.to_string(),
            roles: vec![(role_id.to_string(), 1)],
            channel_id: "400000000000000001".to_string(),
            thread_parent_id: None,
            voice_channel_id: None,
            is_guild_owner: false,
            is_administrator: false,
        };

        let outcome = engine.evaluate(&ctx, &resource, &[guild_allow, role_deny.clone()], true);
        assert!(
            !outcome.allowed,
            "Role tier must beat guild tier regardless of priority"
        );
        assert_eq!(outcome.matched_rule_id, Some(role_deny.id));
    }
}
