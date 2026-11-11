use anyhow::Result;
use serenity::all::{
    Channel, ChannelId, ChannelType, Context, EditMember, GuildId, Permissions, UserId,
};

use crate::state::AppState;
use crate::storage::{
    EVENT_VOICE_JOB_KIND_MOVE_VOICE, EVENT_VOICE_JOB_KIND_NOTIFY_READY, EVENT_VOICE_MODE_AUTO_MOVE,
    EVENT_VOICE_READY_STATUS_READY, EventVoiceOutboxJob,
};

#[derive(Debug, Clone)]
pub struct ProcessEventVoiceJobOutcome {
    pub success: bool,
    pub canceled: bool,
    pub reason: String,
    pub retry_delay: Option<std::time::Duration>,
}

pub async fn process_claimed_event_voice_job(
    ctx: Option<&Context>,
    state: &AppState,
    worker_id: &str,
    job: &EventVoiceOutboxJob,
) -> Result<ProcessEventVoiceJobOutcome> {
    // 1. Verify event still exists and guild matches
    let _event = match state.storage.get_scheduled_event(job.event_id).await? {
        Some(ev) if ev.guild_id == job.guild_id => ev,
        _ => {
            state
                .storage
                .cancel_event_voice_outbox_job(job.id, worker_id, "Event canceled or deleted")
                .await?;
            return Ok(ProcessEventVoiceJobOutcome {
                success: false,
                canceled: true,
                reason: "Event canceled or deleted".to_string(),
                retry_delay: None,
            });
        }
    };

    // 2. Verify voice queue config is still enabled
    let config = match state
        .storage
        .get_event_voice_queue_config(job.event_id)
        .await?
    {
        Some(cfg) if cfg.enabled => cfg,
        _ => {
            state
                .storage
                .cancel_event_voice_outbox_job(job.id, worker_id, "Voice queue disabled")
                .await?;
            return Ok(ProcessEventVoiceJobOutcome {
                success: false,
                canceled: true,
                reason: "Voice queue disabled".to_string(),
                retry_delay: None,
            });
        }
    };

    // 3. Verify user is in roster and has role_type == "main"
    let rosters = state.storage.get_event_rosters(job.event_id).await?;
    let in_main = rosters
        .iter()
        .any(|r| r.user_id == job.user_id && r.role_type == "main");
    if !in_main {
        state
            .storage
            .cancel_event_voice_outbox_job(job.id, worker_id, "User not in main roster")
            .await?;
        return Ok(ProcessEventVoiceJobOutcome {
            success: false,
            canceled: true,
            reason: "User not in main roster".to_string(),
            retry_delay: None,
        });
    }

    // 4. Verify member state and generation
    let member_state = match state
        .storage
        .get_event_voice_member_state(job.event_id, &job.user_id)
        .await?
    {
        Some(s) if s.generation == job.generation => s,
        _ => {
            state
                .storage
                .cancel_event_voice_outbox_job(
                    job.id,
                    worker_id,
                    "Generation mismatch or missing state",
                )
                .await?;
            return Ok(ProcessEventVoiceJobOutcome {
                success: false,
                canceled: true,
                reason: "Generation mismatch or missing state".to_string(),
                retry_delay: None,
            });
        }
    };

    // 5. Execute action based on job kind
    match job.kind.as_str() {
        EVENT_VOICE_JOB_KIND_MOVE_VOICE => {
            if config.mode != EVENT_VOICE_MODE_AUTO_MOVE {
                state
                    .storage
                    .cancel_event_voice_outbox_job(job.id, worker_id, "Mode is not auto_move")
                    .await?;
                return Ok(ProcessEventVoiceJobOutcome {
                    success: false,
                    canceled: true,
                    reason: "Mode is not auto_move".to_string(),
                    retry_delay: None,
                });
            }
            if !member_state.consent_auto_move {
                state
                    .storage
                    .cancel_event_voice_outbox_job(
                        job.id,
                        worker_id,
                        "User did not consent to auto_move",
                    )
                    .await?;
                return Ok(ProcessEventVoiceJobOutcome {
                    success: false,
                    canceled: true,
                    reason: "User did not consent to auto_move".to_string(),
                    retry_delay: None,
                });
            }
            if member_state.ready_status != EVENT_VOICE_READY_STATUS_READY {
                state
                    .storage
                    .cancel_event_voice_outbox_job(job.id, worker_id, "User is not in ready status")
                    .await?;
                return Ok(ProcessEventVoiceJobOutcome {
                    success: false,
                    canceled: true,
                    reason: "User is not in ready status".to_string(),
                    retry_delay: None,
                });
            }

            let lobby_str = config.lobby_channel_id.as_deref().unwrap_or("");
            let target_str = config.target_channel_id.as_deref().unwrap_or("");
            if lobby_str.is_empty() || target_str.is_empty() {
                state
                    .storage
                    .cancel_event_voice_outbox_job(
                        job.id,
                        worker_id,
                        "Lobby or target channel not configured",
                    )
                    .await?;
                return Ok(ProcessEventVoiceJobOutcome {
                    success: false,
                    canceled: true,
                    reason: "Lobby or target channel not configured".to_string(),
                    retry_delay: None,
                });
            }

            if let Some(ctx) = ctx {
                let guild_num = match job.guild_id.parse::<u64>() {
                    Ok(n) => n,
                    Err(_) => {
                        state
                            .storage
                            .cancel_event_voice_outbox_job(
                                job.id,
                                worker_id,
                                "Invalid guild snowflake",
                            )
                            .await?;
                        return Ok(ProcessEventVoiceJobOutcome {
                            success: false,
                            canceled: true,
                            reason: "Invalid guild snowflake".to_string(),
                            retry_delay: None,
                        });
                    }
                };
                let guild_id = GuildId::new(guild_num);
                let user_num = match job.user_id.parse::<u64>() {
                    Ok(n) => n,
                    Err(_) => {
                        state
                            .storage
                            .cancel_event_voice_outbox_job(
                                job.id,
                                worker_id,
                                "Invalid user snowflake",
                            )
                            .await?;
                        return Ok(ProcessEventVoiceJobOutcome {
                            success: false,
                            canceled: true,
                            reason: "Invalid user snowflake".to_string(),
                            retry_delay: None,
                        });
                    }
                };
                let user_id = UserId::new(user_num);

                let lobby_num = match lobby_str.parse::<u64>() {
                    Ok(n) => n,
                    Err(_) => {
                        state
                            .storage
                            .cancel_event_voice_outbox_job(
                                job.id,
                                worker_id,
                                "Invalid lobby snowflake",
                            )
                            .await?;
                        return Ok(ProcessEventVoiceJobOutcome {
                            success: false,
                            canceled: true,
                            reason: "Invalid lobby snowflake".to_string(),
                            retry_delay: None,
                        });
                    }
                };
                let lobby_channel_id = ChannelId::new(lobby_num);

                let target_num = match target_str.parse::<u64>() {
                    Ok(n) => n,
                    Err(_) => {
                        state
                            .storage
                            .cancel_event_voice_outbox_job(
                                job.id,
                                worker_id,
                                "Invalid target snowflake",
                            )
                            .await?;
                        return Ok(ProcessEventVoiceJobOutcome {
                            success: false,
                            canceled: true,
                            reason: "Invalid target snowflake".to_string(),
                            retry_delay: None,
                        });
                    }
                };
                let target_channel_id = ChannelId::new(target_num);

                // Fetch current voice state of user
                let current_channel = ctx
                    .cache
                    .guild(guild_id)
                    .and_then(|g| g.voice_states.get(&user_id).and_then(|vs| vs.channel_id));

                if current_channel != Some(lobby_channel_id) {
                    let reason =
                        "User is not currently in the configured lobby channel".to_string();
                    state
                        .storage
                        .cancel_event_voice_outbox_job(job.id, worker_id, &reason)
                        .await?;
                    return Ok(ProcessEventVoiceJobOutcome {
                        success: false,
                        canceled: true,
                        reason,
                        retry_delay: None,
                    });
                }

                // Verify target channel is on same guild
                let target_channel = if let Some(chan) = ctx
                    .cache
                    .guild(guild_id)
                    .and_then(|g| g.channels.get(&target_channel_id).cloned())
                {
                    Some(chan)
                } else if let Ok(Channel::Guild(chan)) =
                    ctx.http.get_channel(target_channel_id).await
                {
                    Some(chan)
                } else {
                    None
                };

                let target_channel = match target_channel {
                    Some(c) => c,
                    None => {
                        let reason = format!("Target channel {} not found", target_channel_id);
                        state
                            .storage
                            .fail_event_voice_outbox_job(job.id, worker_id, &reason, None)
                            .await?;
                        return Ok(ProcessEventVoiceJobOutcome {
                            success: false,
                            canceled: false,
                            reason,
                            retry_delay: None,
                        });
                    }
                };

                if target_channel.guild_id != guild_id {
                    let reason = format!(
                        "Target channel {} belongs to different guild {}",
                        target_channel_id, target_channel.guild_id
                    );
                    state
                        .storage
                        .cancel_event_voice_outbox_job(job.id, worker_id, &reason)
                        .await?;
                    return Ok(ProcessEventVoiceJobOutcome {
                        success: false,
                        canceled: true,
                        reason,
                        retry_delay: None,
                    });
                }

                if !matches!(target_channel.kind, ChannelType::Voice | ChannelType::Stage) {
                    let reason = format!(
                        "Target channel {} is not a voice/stage channel",
                        target_channel_id
                    );
                    state
                        .storage
                        .cancel_event_voice_outbox_job(job.id, worker_id, &reason)
                        .await?;
                    return Ok(ProcessEventVoiceJobOutcome {
                        success: false,
                        canceled: true,
                        reason,
                        retry_delay: None,
                    });
                }

                // Check bot permissions: MOVE_MEMBERS on guild, CONNECT on target channel
                let bot_user = ctx.cache.current_user().clone();
                let bot_perms = if let Ok(guild) = guild_id.to_partial_guild(&ctx.http).await {
                    if let Ok(bot_member) = guild_id.member(&ctx.http, bot_user.id).await {
                        Some(
                            crate::discord::scheduler::calculate_effective_channel_permissions(
                                &guild,
                                &bot_member.roles,
                                bot_user.id,
                                &target_channel,
                            ),
                        )
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some(perms) = bot_perms {
                    if !perms.contains(Permissions::MOVE_MEMBERS) {
                        let reason = "Bot lacks MOVE_MEMBERS permission to move voice participants"
                            .to_string();
                        state
                            .storage
                            .fail_event_voice_outbox_job(job.id, worker_id, &reason, None)
                            .await?;
                        return Ok(ProcessEventVoiceJobOutcome {
                            success: false,
                            canceled: false,
                            reason,
                            retry_delay: None,
                        });
                    }
                    if !perms.contains(Permissions::CONNECT) {
                        let reason =
                            "Bot lacks CONNECT permission in target voice channel".to_string();
                        state
                            .storage
                            .fail_event_voice_outbox_job(job.id, worker_id, &reason, None)
                            .await?;
                        return Ok(ProcessEventVoiceJobOutcome {
                            success: false,
                            canceled: false,
                            reason,
                            retry_delay: None,
                        });
                    }
                }

                // Execute move via edit_member
                let edit_builder = EditMember::new().voice_channel(target_channel_id);
                match guild_id.edit_member(&ctx.http, user_id, edit_builder).await {
                    Ok(_) => {
                        state
                            .storage
                            .complete_event_voice_outbox_job(job.id, worker_id)
                            .await?;
                        let _ = state
                            .storage
                            .record_audit(
                                Some(&job.guild_id),
                                Some(&job.user_id),
                                "event_voice_queue",
                                "user_moved_from_lobby",
                                serde_json::json!({
                                    "event_id": job.event_id,
                                    "lobby_channel_id": lobby_str,
                                    "target_channel_id": target_str,
                                    "generation": job.generation,
                                }),
                            )
                            .await;
                        Ok(ProcessEventVoiceJobOutcome {
                            success: true,
                            canceled: false,
                            reason: "Successfully moved member to target voice channel".to_string(),
                            retry_delay: None,
                        })
                    }
                    Err(err) => {
                        let err_str = err.to_string();
                        let is_rate_limit = err_str.contains("429")
                            || err_str.to_lowercase().contains("rate limit");
                        if is_rate_limit {
                            let delay = std::time::Duration::from_secs(5);
                            state
                                .storage
                                .fail_event_voice_outbox_job(
                                    job.id,
                                    worker_id,
                                    &err_str,
                                    Some(chrono::Duration::seconds(5)),
                                )
                                .await?;
                            Ok(ProcessEventVoiceJobOutcome {
                                success: false,
                                canceled: false,
                                reason: err_str,
                                retry_delay: Some(delay),
                            })
                        } else {
                            state
                                .storage
                                .fail_event_voice_outbox_job(job.id, worker_id, &err_str, None)
                                .await?;
                            Ok(ProcessEventVoiceJobOutcome {
                                success: false,
                                canceled: false,
                                reason: err_str,
                                retry_delay: None,
                            })
                        }
                    }
                }
            } else {
                // In test or simulated mode without live Discord Context: complete job
                state
                    .storage
                    .complete_event_voice_outbox_job(job.id, worker_id)
                    .await?;
                let _ = state
                    .storage
                    .record_audit(
                        Some(&job.guild_id),
                        Some(&job.user_id),
                        "event_voice_queue",
                        "user_moved_from_lobby_simulated",
                        serde_json::json!({
                            "event_id": job.event_id,
                            "lobby_channel_id": lobby_str,
                            "target_channel_id": target_str,
                            "generation": job.generation,
                        }),
                    )
                    .await;
                Ok(ProcessEventVoiceJobOutcome {
                    success: true,
                    canceled: false,
                    reason: "Simulated move completed".to_string(),
                    retry_delay: None,
                })
            }
        }
        EVENT_VOICE_JOB_KIND_NOTIFY_READY => {
            state
                .storage
                .complete_event_voice_outbox_job(job.id, worker_id)
                .await?;
            let _ = state
                .storage
                .record_audit(
                    Some(&job.guild_id),
                    Some(&job.user_id),
                    "event_voice_queue",
                    "ready_notification_dispatched",
                    serde_json::json!({
                        "event_id": job.event_id,
                        "generation": job.generation,
                        "kind": job.kind,
                    }),
                )
                .await;
            Ok(ProcessEventVoiceJobOutcome {
                success: true,
                canceled: false,
                reason: "Notification dispatched".to_string(),
                retry_delay: None,
            })
        }
        other => {
            let reason = format!("Unknown job kind: {}", other);
            state
                .storage
                .cancel_event_voice_outbox_job(job.id, worker_id, &reason)
                .await?;
            Ok(ProcessEventVoiceJobOutcome {
                success: false,
                canceled: true,
                reason,
                retry_delay: None,
            })
        }
    }
}
