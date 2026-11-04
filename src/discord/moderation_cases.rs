use anyhow::{Result, anyhow};
use chrono::{Duration, Utc};
use serenity::{
    all::{ChannelId, GuildId, Permissions, UserId},
    builder::EditMember,
    client::Context,
};

use crate::{
    AppState,
    lua::ModerationCaseOperation,
    moderation_cases::{CaseFilter, CaseKind, CaseSeverity, CaseStatus, CreateCase, UpdateCase},
    storage::ModerationExecutionIntent,
};

#[derive(Clone, Copy)]
pub struct ModerationExecutionContext {
    pub guild_id: Option<GuildId>,
    pub channel_id: Option<ChannelId>,
    pub actor_id: Option<UserId>,
    pub actor_permissions: Permissions,
    pub app_permissions: Permissions,
    pub command_context: bool,
}

impl ModerationExecutionContext {
    #[must_use]
    pub const fn new(
        guild_id: Option<GuildId>,
        channel_id: Option<ChannelId>,
        actor_id: Option<UserId>,
        actor_permissions: Permissions,
        app_permissions: Permissions,
        command_context: bool,
    ) -> Self {
        Self {
            guild_id,
            channel_id,
            actor_id,
            actor_permissions,
            app_permissions,
            command_context,
        }
    }
}

pub fn is_moderator(permissions: Permissions) -> bool {
    permissions.contains(Permissions::ADMINISTRATOR)
        || permissions.contains(Permissions::MANAGE_GUILD)
        || permissions.contains(Permissions::MODERATE_MEMBERS)
        || permissions.contains(Permissions::BAN_MEMBERS)
        || permissions.contains(Permissions::KICK_MEMBERS)
        || permissions.contains(Permissions::MANAGE_MESSAGES)
}

pub async fn execute(
    ctx: &Context,
    state: &AppState,
    module_id: &str,
    execution: ModerationExecutionContext,
    operation: &ModerationCaseOperation,
) -> Result<Option<String>> {
    let guild_id = execution
        .guild_id
        .ok_or_else(|| anyhow!("moderation cases system operates exclusively within a server"))?;
    let actor_id = execution
        .actor_id
        .ok_or_else(|| anyhow!("this operation requires user context"))?;
    let now = Utc::now();
    let is_mod = is_moderator(execution.actor_permissions);

    match operation {
        ModerationCaseOperation::Create {
            subject_user_id,
            kind,
            reason,
            severity,
            points,
            visible_to_subject,
            duration_seconds,
            delete_days,
        } => {
            if !is_mod {
                return Err(anyhow!("missing moderator permissions to create cases"));
            }

            let subject_u64 = subject_user_id
                .parse::<u64>()
                .map_err(|_| anyhow!("invalid user ID"))?;
            let target_user_id = UserId::new(subject_u64);

            let case_kind = match kind.as_str() {
                "warning" => CaseKind::Warning,
                "staff_note" => CaseKind::StaffNote,
                "timeout" => CaseKind::Timeout,
                "kick" => CaseKind::Kick,
                "ban" => CaseKind::Ban,
                "unban" => CaseKind::Unban,
                "softban" => CaseKind::Softban,
                "automod" => CaseKind::Automod,
                _ => CaseKind::Other,
            };

            if matches!(
                case_kind,
                CaseKind::Ban | CaseKind::Unban | CaseKind::Softban
            ) && !execution
                .actor_permissions
                .contains(Permissions::BAN_MEMBERS)
                && !execution
                    .actor_permissions
                    .contains(Permissions::ADMINISTRATOR)
            {
                return Err(anyhow!(
                    "brak uprawnienia BAN_MEMBERS do wykonania tej akcji"
                ));
            }
            if case_kind == CaseKind::Kick
                && !execution
                    .actor_permissions
                    .contains(Permissions::KICK_MEMBERS)
                && !execution
                    .actor_permissions
                    .contains(Permissions::ADMINISTRATOR)
            {
                return Err(anyhow!(
                    "brak uprawnienia KICK_MEMBERS do wykonania tej akcji"
                ));
            }
            if case_kind == CaseKind::Timeout
                && !execution
                    .actor_permissions
                    .contains(Permissions::MODERATE_MEMBERS)
                && !execution
                    .actor_permissions
                    .contains(Permissions::ADMINISTRATOR)
            {
                return Err(anyhow!(
                    "brak uprawnienia MODERATE_MEMBERS do wykonania tej akcji"
                ));
            }

            if execution.command_context && case_kind != CaseKind::Unban {
                crate::discord::executor::ensure_member_hierarchy(
                    ctx,
                    guild_id,
                    execution.actor_id,
                    target_user_id,
                )
                .await?;
            }

            let case_severity = match severity.as_str() {
                "info" => CaseSeverity::Info,
                "low" => CaseSeverity::Low,
                "medium" => CaseSeverity::Medium,
                "high" => CaseSeverity::High,
                "critical" => CaseSeverity::Critical,
                _ => CaseSeverity::Medium,
            };

            let expires_at = duration_seconds.map(|secs| now + Duration::seconds(secs as i64));

            // Execute underlying Discord action if applicable with proper error propagation
            if case_kind == CaseKind::Timeout {
                if let Some(secs) = duration_seconds {
                    if *secs == 0 || *secs > 2_419_200 {
                        return Err(anyhow!(
                            "timeout duration must be between 1 and 2419200 seconds (28 days)"
                        ));
                    }
                    let until = (now + Duration::seconds(*secs as i64)).to_rfc3339();
                    guild_id
                        .edit_member(
                            &ctx.http,
                            target_user_id,
                            EditMember::new().disable_communication_until(until),
                        )
                        .await
                        .map_err(|e| anyhow!("Discord API error while timing out member: {e}"))?;
                }
            } else if case_kind == CaseKind::Kick {
                guild_id
                    .kick_with_reason(&ctx.http, target_user_id, reason)
                    .await
                    .map_err(|e| anyhow!("Discord API error while kicking member: {e}"))?;
            } else if case_kind == CaseKind::Ban {
                let ban_delete_days = delete_days.unwrap_or(0);
                guild_id
                    .ban_with_reason(&ctx.http, target_user_id, ban_delete_days, reason)
                    .await
                    .map_err(|e| anyhow!("Discord API error while banning member: {e}"))?;
            } else if case_kind == CaseKind::Softban {
                let softban_delete_days = delete_days.unwrap_or(1);
                let intent_id = uuid::Uuid::new_v4();
                let intent = ModerationExecutionIntent {
                    id: intent_id,
                    interaction_id: None,
                    guild_id: guild_id.get().to_string(),
                    user_id: subject_user_id.clone(),
                    kind: "softban".to_string(),
                    step: "pending".to_string(),
                    reason: Some(reason.clone()),
                    error: None,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                let _ = state.storage.record_moderation_intent(&intent).await;

                if let Err(ban_err) = guild_id
                    .ban_with_reason(&ctx.http, target_user_id, softban_delete_days, reason)
                    .await
                {
                    let _ = state
                        .storage
                        .update_moderation_intent_step(
                            intent_id,
                            "ban_failed",
                            Some(&ban_err.to_string()),
                        )
                        .await;
                    return Err(anyhow!(
                        "Discord API error while banning (softban): {ban_err}"
                    ));
                }

                let _ = state
                    .storage
                    .update_moderation_intent_step(intent_id, "banned", None)
                    .await;

                if let Err(unban_err) = guild_id.unban(&ctx.http, target_user_id).await {
                    let _ = state
                        .storage
                        .update_moderation_intent_step(
                            intent_id,
                            "unban_failed",
                            Some(&unban_err.to_string()),
                        )
                        .await;

                    let _ = state
                        .storage
                        .record_audit(
                            Some(&guild_id.get().to_string()),
                            Some(&actor_id.get().to_string()),
                            module_id,
                            "softban_unban_failed",
                            serde_json::json!({
                                "subject_user_id": subject_user_id,
                                "delete_days": softban_delete_days,
                                "error": unban_err.to_string(),
                                "alert": "User was banned, but unbanning failed; manual team intervention required.",
                            }),
                        )
                        .await;

                    let _ = state
                        .moderation_cases
                        .create_case(
                            CreateCase {
                                guild_id: guild_id.get(),
                                subject_user_id: subject_u64,
                                actor_user_id: actor_id.get(),
                                kind: CaseKind::Softban,
                                severity: CaseSeverity::High,
                                points: *points,
                                reason: format!(
                                    "{reason} [UWAGA: odbanowanie nie powiodło się: {unban_err}]"
                                ),
                                source_module: module_id.to_owned(),
                                visible_to_subject: *visible_to_subject,
                                expires_at,
                            },
                            now,
                        )
                        .await;

                    return Err(anyhow!(
                        "Discord API error while unbanning (softban): {unban_err}"
                    ));
                }

                let _ = state
                    .storage
                    .update_moderation_intent_step(intent_id, "completed", None)
                    .await;
            } else if case_kind == CaseKind::Unban {
                guild_id
                    .unban(&ctx.http, target_user_id)
                    .await
                    .map_err(|e| anyhow!("Discord API error while unbanning: {e}"))?;
            }

            let created = state
                .moderation_cases
                .create_case(
                    CreateCase {
                        guild_id: guild_id.get(),
                        subject_user_id: subject_u64,
                        actor_user_id: actor_id.get(),
                        kind: case_kind,
                        severity: case_severity,
                        points: *points,
                        reason: reason.clone(),
                        source_module: module_id.to_owned(),
                        visible_to_subject: *visible_to_subject,
                        expires_at,
                    },
                    now,
                )
                .await
                .map_err(|err| anyhow!("case save error: {err}"))?;

            let _ = state
                .storage
                .record_audit(
                    Some(&guild_id.get().to_string()),
                    Some(&actor_id.get().to_string()),
                    module_id,
                    "case_created",
                    serde_json::json!({
                        "case_number": created.case_number,
                        "subject_user_id": subject_user_id,
                        "kind": kind,
                        "severity": severity,
                        "points": points,
                        "reason": reason,
                        "delete_days": delete_days,
                    }),
                )
                .await;

            let _ = state.guild_events.send(crate::state::GuildRealtimeEvent {
                guild_id: guild_id.get().to_string(),
                event_type: "case_created".to_string(),
                payload: serde_json::json!({
                    "case_number": created.case_number,
                    "subject_user_id": subject_user_id,
                    "kind": kind,
                }),
            });

            Ok(Some(format!(
                "⚖️ Registered moderation case **#{case_number}** for <@{subject_user_id}> [{kind} | {severity} | points: {points}]: {reason}",
                case_number = created.case_number,
            )))
        }

        ModerationCaseOperation::View { case_number } => {
            let details = state
                .moderation_cases
                .get_case(guild_id.get(), *case_number, now)
                .await
                .map_err(|err| anyhow!("case #{case_number} was not found: {err}"))?;

            if !is_mod
                && (details.case.subject_user_id != actor_id.get()
                    || !details.case.visible_to_subject)
            {
                return Err(anyhow!(
                    "insufficient permissions to view case #{case_number}"
                ));
            }

            let c = &details.case;
            let status_label = match c.status {
                CaseStatus::Active => "Aktywna",
                CaseStatus::Expired => "Expired",
                CaseStatus::Voided => "Voided",
            };

            let expiry_info = match c.expires_at {
                Some(exp) => format!("Wygasa: {}", exp.to_rfc3339()),
                None => "Wygasa: Nigdy".to_owned(),
            };

            let mut message = format!(
                "📋 **Case #{case_number}** [{kind:?} | {severity:?}] — Status: **{status}**\n• Target: <@{subject}> | Moderator: <@{actor}>\n• Points: {points} | {expiry_info}\n• Reason: {reason}\n• Created: {created}",
                case_number = c.case_number,
                kind = c.kind,
                severity = c.severity,
                status = status_label,
                subject = c.subject_user_id,
                actor = c.actor_user_id,
                points = c.points,
                reason = c.reason,
                created = c.created_at.to_rfc3339(),
            );

            if let Some(void_reason) = &c.void_reason {
                message.push_str(&format!(
                    "\n• ⚠️ Voided by <@{}>: {}",
                    c.voided_by_user_id.unwrap_or(0),
                    void_reason
                ));
            }

            let visible_notes: Vec<_> = details
                .notes
                .iter()
                .filter(|n| is_mod || n.visible_to_subject)
                .collect();
            if !visible_notes.is_empty() {
                message.push_str("\n\n**Notatki:**");
                for note in visible_notes {
                    message.push_str(&format!(
                        "\n- <@{}> ({}): {}",
                        note.author_user_id,
                        note.created_at.to_rfc3339(),
                        note.body
                    ));
                }
            }

            if !details.evidence.is_empty() && is_mod {
                message.push_str("\n\n**Dowody:**");
                for ev in &details.evidence {
                    message.push_str(&format!("\n- [{}] {}", ev.label, ev.value));
                }
            }

            Ok(Some(message))
        }

        ModerationCaseOperation::List {
            subject_user_id,
            limit,
            visible_to_subject_only,
        } => {
            let filter_subject = if !is_mod {
                Some(actor_id.get())
            } else {
                subject_user_id
                    .as_deref()
                    .and_then(|id| id.parse::<u64>().ok())
            };

            let visible_only = if !is_mod {
                true
            } else {
                *visible_to_subject_only
            };

            let cases = state
                .moderation_cases
                .list_cases(
                    guild_id.get(),
                    CaseFilter {
                        subject_user_id: filter_subject,
                        actor_user_id: None,
                        kind: None,
                        status: None,
                        visible_to_subject_only: visible_only,
                        limit: *limit as u16,
                    },
                    now,
                )
                .await
                .map_err(|err| anyhow!("error fetching cases list: {err}"))?;

            if cases.is_empty() {
                return Ok(Some(
                    "Brak zarejestrowanych spraw moderacyjnych.".to_owned(),
                ));
            }

            let mut message = format!("📋 **Sprawy moderacyjne ({})**:\n", cases.len());
            for c in cases {
                message.push_str(&format!(
                    "• **#{}** [{:?}] dla <@{}>: {} (Status: {:?})\n",
                    c.case_number, c.kind, c.subject_user_id, c.reason, c.status
                ));
            }

            Ok(Some(message))
        }

        ModerationCaseOperation::AddNote {
            case_number,
            body,
            visible_to_subject,
        } => {
            if !is_mod {
                return Err(anyhow!("missing moderator permissions to add notes"));
            }

            state
                .moderation_cases
                .add_note(
                    guild_id.get(),
                    *case_number,
                    actor_id.get(),
                    body.clone(),
                    *visible_to_subject,
                    now,
                )
                .await
                .map_err(|err| anyhow!("error adding note: {err}"))?;

            let _ = state
                .storage
                .record_audit(
                    Some(&guild_id.get().to_string()),
                    Some(&actor_id.get().to_string()),
                    module_id,
                    "case_note_added",
                    serde_json::json!({
                        "case_number": case_number,
                        "body": body,
                        "visible_to_subject": visible_to_subject,
                    }),
                )
                .await;

            Ok(Some(format!("Added note to case #{case_number}.")))
        }

        ModerationCaseOperation::AddEvidence {
            case_number,
            label,
            value,
        } => {
            if !is_mod {
                return Err(anyhow!("missing moderator permissions to add evidence"));
            }

            state
                .moderation_cases
                .add_evidence(
                    guild_id.get(),
                    *case_number,
                    actor_id.get(),
                    label.clone(),
                    value.clone(),
                    now,
                )
                .await
                .map_err(|err| anyhow!("error adding evidence: {err}"))?;

            let _ = state
                .storage
                .record_audit(
                    Some(&guild_id.get().to_string()),
                    Some(&actor_id.get().to_string()),
                    module_id,
                    "case_evidence_added",
                    serde_json::json!({
                        "case_number": case_number,
                        "label": label,
                        "value": value,
                    }),
                )
                .await;

            Ok(Some(format!(
                "Added evidence [{label}] to case #{case_number}."
            )))
        }

        ModerationCaseOperation::Void {
            case_number,
            expected_version,
            reason,
        } => {
            if !is_mod {
                return Err(anyhow!("missing moderator permissions to void cases"));
            }

            let version = if *expected_version <= 0 {
                let current = state
                    .moderation_cases
                    .get_case(guild_id.get(), *case_number, now)
                    .await
                    .map_err(|err| anyhow!("nie znaleziono sprawy #{case_number}: {err}"))?;
                current.case.version
            } else {
                *expected_version
            };

            state
                .moderation_cases
                .void_case(
                    guild_id.get(),
                    *case_number,
                    actor_id.get(),
                    version,
                    reason.clone(),
                    now,
                )
                .await
                .map_err(|err| anyhow!("error voiding case: {err}"))?;

            let _ = state
                .storage
                .record_audit(
                    Some(&guild_id.get().to_string()),
                    Some(&actor_id.get().to_string()),
                    module_id,
                    "case_voided",
                    serde_json::json!({
                        "case_number": case_number,
                        "reason": reason,
                    }),
                )
                .await;

            Ok(Some(format!(
                "Case #{case_number} has been voided. Reason: {reason}"
            )))
        }

        ModerationCaseOperation::Edit {
            case_number,
            expected_version,
            reason,
            severity,
            points,
            visible_to_subject,
            duration_seconds,
        } => {
            if !is_mod {
                return Err(anyhow!("missing moderator permissions to edit cases"));
            }

            let version = if *expected_version <= 0 {
                let current = state
                    .moderation_cases
                    .get_case(guild_id.get(), *case_number, now)
                    .await
                    .map_err(|err| anyhow!("nie znaleziono sprawy #{case_number}: {err}"))?;
                current.case.version
            } else {
                *expected_version
            };

            let sev = severity.as_deref().map(|s| match s {
                "info" => CaseSeverity::Info,
                "low" => CaseSeverity::Low,
                "medium" => CaseSeverity::Medium,
                "high" => CaseSeverity::High,
                "critical" => CaseSeverity::Critical,
                _ => CaseSeverity::Medium,
            });

            let expires_at = duration_seconds.map(|secs| now + Duration::seconds(secs as i64));

            let update = UpdateCase {
                reason: reason.clone(),
                severity: sev,
                points: *points,
                visible_to_subject: *visible_to_subject,
                expires_at,
                clear_expiry: false,
            };

            let updated = state
                .moderation_cases
                .update_case(
                    guild_id.get(),
                    *case_number,
                    actor_id.get(),
                    version,
                    update,
                    now,
                )
                .await
                .map_err(|err| anyhow!("error updating case: {err}"))?;

            let _ = state
                .storage
                .record_audit(
                    Some(&guild_id.get().to_string()),
                    Some(&actor_id.get().to_string()),
                    module_id,
                    "case_edited",
                    serde_json::json!({
                        "case_number": case_number,
                        "version": updated.version,
                        "reason": reason,
                        "severity": severity,
                        "points": points,
                    }),
                )
                .await;

            Ok(Some(format!(
                "✏️ Updated moderation case **#{case_number}** (version {version}).",
                version = updated.version
            )))
        }

        ModerationCaseOperation::Restore {
            case_number,
            expected_version,
        } => {
            if !is_mod {
                return Err(anyhow!("missing moderator permissions to restore cases"));
            }

            let version = if *expected_version <= 0 {
                let current = state
                    .moderation_cases
                    .get_case(guild_id.get(), *case_number, now)
                    .await
                    .map_err(|err| anyhow!("nie znaleziono sprawy #{case_number}: {err}"))?;
                current.case.version
            } else {
                *expected_version
            };

            let restored = state
                .moderation_cases
                .restore_case(guild_id.get(), *case_number, actor_id.get(), version, now)
                .await
                .map_err(|err| anyhow!("error restoring case: {err}"))?;

            let _ = state
                .storage
                .record_audit(
                    Some(&guild_id.get().to_string()),
                    Some(&actor_id.get().to_string()),
                    module_id,
                    "case_restored",
                    serde_json::json!({
                        "case_number": case_number,
                    }),
                )
                .await;

            Ok(Some(format!(
                "Case #{case_number} has been restored (Status: {:?}).",
                restored.status
            )))
        }

        ModerationCaseOperation::Stats { moderator_user_id } => {
            if !is_mod {
                return Err(anyhow!("missing moderator permissions to view statistics"));
            }

            let mod_id = moderator_user_id
                .as_deref()
                .and_then(|id| id.parse::<u64>().ok());

            let stats = state
                .moderation_cases
                .stats(guild_id.get(), mod_id, now)
                .await
                .map_err(|err| anyhow!("error fetching statistics: {err}"))?;

            Ok(Some(format!(
                "📊 **Moderation Statistics**:\n• Total cases: {}\n• Active: {}\n• Expired: {}\n• Voided: {}",
                stats.total_cases, stats.active_cases, stats.expired_cases, stats.voided_cases
            )))
        }
    }
}
