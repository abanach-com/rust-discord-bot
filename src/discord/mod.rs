pub mod afk;
pub mod birthdays;
mod commands;
pub mod companion;
pub mod components;
pub mod delete_queue;
pub use delete_queue::{DeleteCoordinator, DeleteOutcome, DeleteRequest, run_delete_queue_janitor};
pub mod economy;
pub mod event_voice;
pub mod events;
pub mod executor;
pub mod giveaway;
pub mod handler;
pub use handler::Handler;
pub mod moderation_cases;
pub mod music;
pub mod progression;
pub mod quotes;
pub mod reaction_roles;
pub mod scheduler;
pub mod starboard;
pub mod sticky;
pub use sticky::{StickyCoordinator, StickyRefresh};
pub mod tags;
pub mod tickets;

/// Validates whether a Discord channel type can accept direct text message creation.
/// Rejects categories, forum parents, media channels, and directories.
pub fn ensure_send_message_channel_type(kind: serenity::all::ChannelType) -> Result<()> {
    match kind {
        serenity::all::ChannelType::Category => {
            anyhow::bail!("channel categories cannot be the target of messages");
        }
        serenity::all::ChannelType::Forum => {
            anyhow::bail!(
                "forum channels do not accept direct messages; create a thread or post instead"
            );
        }
        serenity::all::ChannelType::Directory | serenity::all::ChannelType::Unknown(16) => {
            anyhow::bail!("media and directory channels cannot be the target of messages");
        }
        _ => Ok(()),
    }
}

use std::sync::Arc;

use anyhow::Result;
use serenity::{
    Client,
    all::{ApplicationId, GatewayIntents},
};
use songbird::SerenityInit;

use crate::AppState;

pub async fn run(state: AppState) -> Result<()> {
    let (_tx, rx) = tokio::sync::broadcast::channel(1);
    run_with_shutdown(state, rx).await
}

pub async fn run_with_shutdown(
    state: AppState,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) -> Result<()> {
    if state.config.is_offline_mode() {
        tracing::warn!("offline/mock test mode active: skipping live Discord Gateway connection");
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(86400 * 365)) => {}
            _ = shutdown_rx.recv() => {
                tracing::info!("offline discord worker received shutdown signal");
            }
        }
        return Ok(());
    }

    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::GUILD_MESSAGE_REACTIONS
        | GatewayIntents::GUILD_MEMBERS
        | GatewayIntents::GUILD_MODERATION
        | GatewayIntents::GUILD_VOICE_STATES
        | GatewayIntents::MESSAGE_CONTENT;

    let handler = handler::Handler::new(state.clone());
    let mut client = Client::builder(&state.config.discord_token, intents)
        .application_id(ApplicationId::new(state.config.discord_application_id))
        .event_handler(handler)
        .register_songbird()
        .await?;

    let scheduler_worker = tokio::spawn(crate::scheduler::run_worker(
        Arc::clone(&client.http),
        state.clone(),
    ));

    let giveaway_worker = tokio::spawn(giveaway::run_giveaway_worker(
        Arc::clone(&client.http),
        state.clone(),
    ));

    let shard_manager = Arc::clone(&client.shard_manager);
    tokio::spawn(async move {
        let _ = shutdown_rx.recv().await;
        tracing::info!("discord client received shutdown signal, shutting down shards");
        shard_manager.shutdown_all().await;
    });

    let client_result = client.start_autosharded().await;
    scheduler_worker.abort();
    giveaway_worker.abort();
    client_result?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serenity::all::ChannelType;

    #[test]
    fn test_ensure_send_message_channel_type_issue_388() {
        // Allowed channel types for direct message creation
        assert!(ensure_send_message_channel_type(ChannelType::Text).is_ok());
        assert!(ensure_send_message_channel_type(ChannelType::Voice).is_ok());
        assert!(ensure_send_message_channel_type(ChannelType::News).is_ok());
        assert!(ensure_send_message_channel_type(ChannelType::Stage).is_ok());
        assert!(ensure_send_message_channel_type(ChannelType::PublicThread).is_ok());
        assert!(ensure_send_message_channel_type(ChannelType::PrivateThread).is_ok());
        assert!(ensure_send_message_channel_type(ChannelType::NewsThread).is_ok());

        // Disallowed channel types for direct message creation
        let err_cat = ensure_send_message_channel_type(ChannelType::Category).unwrap_err();
        assert!(
            err_cat
                .to_string()
                .contains("channel categories cannot be the target of messages")
        );

        let err_forum = ensure_send_message_channel_type(ChannelType::Forum).unwrap_err();
        assert!(
            err_forum
                .to_string()
                .contains("forum channels do not accept direct messages")
        );

        let err_dir = ensure_send_message_channel_type(ChannelType::Directory).unwrap_err();
        assert!(
            err_dir
                .to_string()
                .contains("media and directory channels cannot be the target of messages")
        );

        let err_media = ensure_send_message_channel_type(ChannelType::Unknown(16)).unwrap_err();
        assert!(
            err_media
                .to_string()
                .contains("media and directory channels cannot be the target of messages")
        );
    }
}
