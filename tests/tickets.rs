use std::path::PathBuf;

use chrono::Utc;
use rusty_discord_bot::{
    AppState,
    config::Config,
    lua::{LuaAction, LuaEngine, LuaExecutionContext, LuaLimits, TicketOperation},
    tickets::{
        NewTicketReservation, NewTicketTranscript, ParticipantMutationOutcome,
        ReserveTicketOutcome, STATUS_CLAIMED, STATUS_CLOSED, STATUS_OPEN,
        TRANSCRIPT_CRYPTO_VERSION, TicketMutationOutcome, TicketStore, TranscriptCrypto,
    },
};
use serde_json::json;

fn limits() -> LuaLimits {
    LuaLimits {
        memory_bytes: 4 * 1024 * 1024,
        instruction_limit: 300_000,
        hook_granularity: 100,
    }
}

fn engine() -> LuaEngine {
    let scripts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts");
    LuaEngine::load(scripts, limits()).expect("bundled Lua modules should load")
}

fn lua_context(options: serde_json::Value) -> LuaExecutionContext {
    LuaExecutionContext {
        guild_id: Some("100000000000000001".to_owned()),
        channel_id: "100000000000000002".to_owned(),
        user_id: "100000000000000003".to_owned(),
        user_name: "Tester".to_owned(),
        member_roles: Vec::new(),
        member_permissions: "0".to_owned(),
        locale: "pl".to_owned(),
        options,
        config: json!({
            "open_category_id": "100000000000000010",
            "archive_category_id": "100000000000000011",
            "log_channel_id": "100000000000000012",
            "support_role_ids": ["100000000000000013"],
            "allowed_queues": ["support", "billing"],
            "default_queue": "support",
            "max_open_per_user": 3,
            "channel_name_prefix": "ticket",
            "creator_can_close": true,
            "generate_transcript_on_close": true,
            "transcript_max_messages": 500
        }),
    }
}

fn reservation(creator_user_id: &str) -> NewTicketReservation {
    NewTicketReservation {
        guild_id: "100000000000000001".to_owned(),
        creator_user_id: creator_user_id.to_owned(),
        subject: "Problem z kontem".to_owned(),
        description: "Nie mogę otworzyć panelu ustawień.".to_owned(),
        queue: "support".to_owned(),
    }
}

async fn reserved_ticket(
    store: &TicketStore,
    creator_user_id: &str,
    limit: u8,
) -> rusty_discord_bot::tickets::TicketRecord {
    match store
        .reserve_ticket(reservation(creator_user_id), limit)
        .await
        .expect("memory ticket reservation should succeed")
    {
        ReserveTicketOutcome::Reserved(ticket) => *ticket,
        ReserveTicketOutcome::LimitReached { limit, .. } => {
            panic!("unexpected ticket quota {limit}")
        }
    }
}

async fn active_ticket(store: &TicketStore) -> rusty_discord_bot::tickets::TicketRecord {
    let ticket = reserved_ticket(store, "100000000000000003", 3).await;
    match store
        .activate_ticket(
            &ticket.id,
            "100000000000000020",
            "ticket-000001-problem-z-kontem",
        )
        .await
        .expect("memory ticket activation should succeed")
    {
        TicketMutationOutcome::Updated(ticket) => *ticket,
        outcome => panic!("unexpected activation outcome: {outcome:?}"),
    }
}

#[tokio::test]
async fn lua_open_subcommand_emits_a_valid_ticket_action() {
    let engine = engine();
    assert_eq!(
        engine.module_for_command("ticket").await.as_deref(),
        Some("tickets")
    );

    let actions = engine
        .execute_command(
            "tickets",
            "ticket",
            lua_context(json!({
                "open": {
                    "subject": "Płatność",
                    "description": "Płatność została pobrana dwa razy.",
                    "queue": "billing"
                }
            })),
        )
        .await
        .expect("ticket open should execute in the Lua sandbox");

    assert!(matches!(
        actions.first(),
        Some(LuaAction::Ticket {
            operation: TicketOperation::Open {
                queue,
                policy,
                ..
            }
        }) if queue == "billing"
            && policy.open_category_id == "100000000000000010"
            && policy.support_role_ids == ["100000000000000013"]
    ));
}

#[tokio::test]
async fn lua_rejects_a_queue_outside_server_configuration() {
    let engine = engine();
    let actions = engine
        .execute_command(
            "tickets",
            "ticket",
            lua_context(json!({
                "open": {
                    "subject": "Inne",
                    "description": "Test niedozwolonej kolejki.",
                    "queue": "secret"
                }
            })),
        )
        .await
        .expect("Lua validation response should execute");

    assert!(matches!(
        actions.first(),
        Some(LuaAction::Reply {
            ephemeral: true,
            content,
        }) if content.contains("nie jest dozwolona") || content.contains("not allowed")
    ));
}

#[tokio::test]
async fn per_user_open_ticket_quota_is_enforced_atomically_by_the_store_contract() {
    let store = TicketStore::memory();
    reserved_ticket(&store, "100000000000000003", 1).await;

    let outcome = store
        .reserve_ticket(reservation("100000000000000003"), 1)
        .await
        .expect("second reservation should return a quota outcome");
    assert!(matches!(
        outcome,
        ReserveTicketOutcome::LimitReached {
            limit: 1,
            active_count: 1,
        }
    ));
}

#[tokio::test]
async fn ticket_claim_close_and_reopen_preserve_the_lifecycle_contract() {
    let store = TicketStore::memory();
    let ticket = active_ticket(&store).await;
    assert_eq!(ticket.status, STATUS_OPEN);

    let claimed = store
        .claim_ticket(&ticket.id, "100000000000000030")
        .await
        .expect("claim should succeed");
    let TicketMutationOutcome::Updated(claimed) = claimed else {
        panic!("expected updated claim outcome")
    };
    assert_eq!(claimed.status, STATUS_CLAIMED);
    assert_eq!(
        claimed.claimed_by_user_id.as_deref(),
        Some("100000000000000030")
    );

    let closed = store
        .close_ticket(&ticket.id, "100000000000000030", Some("Rozwiązano"))
        .await
        .expect("close should succeed");
    let TicketMutationOutcome::Updated(closed) = closed else {
        panic!("expected updated close outcome")
    };
    assert_eq!(closed.status, STATUS_CLOSED);
    assert_eq!(closed.close_reason.as_deref(), Some("Rozwiązano"));

    let reopened = store
        .reopen_ticket(&ticket.id, "100000000000000031")
        .await
        .expect("reopen should succeed");
    let TicketMutationOutcome::Updated(reopened) = reopened else {
        panic!("expected updated reopen outcome")
    };
    assert_eq!(reopened.status, STATUS_OPEN);
    assert!(reopened.closed_at.is_none());
}

#[tokio::test]
async fn creator_cannot_be_added_or_removed_as_a_regular_participant() {
    let store = TicketStore::memory();
    let ticket = active_ticket(&store).await;

    let add = store
        .add_participant(&ticket.id, &ticket.creator_user_id, "100000000000000030")
        .await
        .expect("participant mutation should return a domain outcome");
    assert!(matches!(add, ParticipantMutationOutcome::CreatorProtected));

    let remove = store
        .remove_participant(&ticket.id, &ticket.creator_user_id, "100000000000000030")
        .await
        .expect("participant mutation should return a domain outcome");
    assert!(matches!(
        remove,
        ParticipantMutationOutcome::CreatorProtected
    ));
}

#[tokio::test]
async fn participant_and_transcript_changes_are_persisted_with_ticket_events() {
    let store = TicketStore::memory();
    let ticket = active_ticket(&store).await;

    let participant = store
        .add_participant(&ticket.id, "100000000000000040", "100000000000000030")
        .await
        .expect("participant add should succeed");
    assert!(matches!(participant, ParticipantMutationOutcome::Added(_)));
    assert_eq!(
        store
            .list_participants(&ticket.id)
            .await
            .expect("participant list should succeed")
            .len(),
        1
    );

    let transcript = store
        .store_transcript(NewTicketTranscript {
            ticket_id: ticket.id.clone(),
            guild_id: ticket.guild_id.clone(),
            channel_id: ticket
                .channel_id
                .clone()
                .expect("active ticket should have a channel"),
            generated_by_user_id: "100000000000000030".to_owned(),
            message_count: 2,
            content: "[message one]\n[message two]\n".to_owned(),
        })
        .await
        .expect("transcript storage should succeed");
    assert_eq!(transcript.message_count, 2);
    assert_eq!(transcript.ticket_id, ticket.id);

    let events = store
        .list_events(&ticket.id, 20)
        .await
        .expect("event history should succeed");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "ticket_participant_added")
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "ticket_transcript_created")
    );
    assert!(events.iter().any(|event| event.created_at <= Utc::now()));
}

#[test]
fn test_render_html_transcript_sanitization_and_csp() {
    use rusty_discord_bot::discord::tickets::render_html_transcript;

    let ticket_id = "ticket_123<script>";
    let guild_id = "guild_456&co";
    let content = "Hello <script>alert('pwned')</script> & goodbye \"world\"!";

    let html = render_html_transcript(ticket_id, guild_id, content);

    assert!(html.contains("Content-Security-Policy"));
    assert!(!html.contains("<script>alert"));
    assert!(html.contains("&lt;script&gt;alert(&#39;pwned&#39;)&lt;/script&gt;"));
    assert!(html.contains("&amp; goodbye &quot;world&quot;!"));
}

#[tokio::test]
async fn ticket_close_reopen_compensating_contract_issue_253() {
    let store = TicketStore::memory();
    let ticket = active_ticket(&store).await;
    assert_eq!(ticket.status, STATUS_OPEN);

    // 1. Reopening an already open ticket returns InvalidState (must trigger compensating rollback)
    let invalid_reopen = store
        .reopen_ticket(&ticket.id, "100000000000000030")
        .await
        .expect("reopen call should succeed");
    assert!(
        matches!(
            invalid_reopen,
            TicketMutationOutcome::InvalidState {
                ref current_status,
            } if current_status == STATUS_OPEN
        ),
        "Reopening open ticket must yield InvalidState"
    );

    // 2. Close ticket transitions to closed
    let closed = store
        .close_ticket(&ticket.id, "100000000000000030", Some("Closed"))
        .await
        .expect("close should succeed");
    let TicketMutationOutcome::Updated(closed_ticket) = closed else {
        panic!("expected updated closed ticket");
    };
    assert_eq!(closed_ticket.status, STATUS_CLOSED);

    // 3. Closing an already closed ticket returns InvalidState (must trigger compensating rollback)
    let invalid_close = store
        .close_ticket(&ticket.id, "100000000000000030", Some("Again"))
        .await
        .expect("second close call should succeed");
    assert!(
        matches!(
            invalid_close,
            TicketMutationOutcome::InvalidState {
                ref current_status,
            } if current_status == STATUS_CLOSED
        ),
        "Closing closed ticket must yield InvalidState"
    );

    // 4. Reopening closed ticket succeeds
    let reopened = store
        .reopen_ticket(&ticket.id, "100000000000000030")
        .await
        .expect("reopen call should succeed");
    let TicketMutationOutcome::Updated(reopened_ticket) = reopened else {
        panic!("expected updated reopened ticket");
    };
    assert_eq!(reopened_ticket.status, STATUS_OPEN);
    assert!(reopened_ticket.version > closed_ticket.version);
}

#[tokio::test]
async fn test_ticket_transcript_lifecycle_and_crypto() {
    let store = TicketStore::memory();
    let guild_id = "100000000000000001";
    let ticket_id = "f47ac10b-58cc-4372-a567-0e02b2c3d479";
    let channel_id = "100000000000000002";
    let actor_id = "100000000000000003";

    // 1. Store transcript in memory store
    let content = "User: Hello!\nSupport: How can I help you today?";
    let stored = store
        .store_transcript(NewTicketTranscript {
            ticket_id: ticket_id.to_string(),
            guild_id: guild_id.to_string(),
            channel_id: channel_id.to_string(),
            generated_by_user_id: actor_id.to_string(),
            message_count: 2,
            content: content.to_string(),
        })
        .await
        .expect("storing transcript should succeed");

    assert_eq!(stored.ticket_id, ticket_id);
    assert_eq!(stored.guild_id, guild_id);
    assert_eq!(stored.content, content);
    assert_eq!(stored.message_count, 2);

    // 2. Fetch transcript by ID
    let fetched = store
        .get_transcript(guild_id, &stored.id)
        .await
        .expect("fetching transcript should succeed")
        .expect("transcript should exist");
    assert_eq!(fetched.id, stored.id);
    assert_eq!(fetched.content, content);

    // Tenant isolation: fetching with different guild_id returns None
    let other_guild_fetch = store
        .get_transcript("999999999999999999", &stored.id)
        .await
        .expect("fetch call should succeed");
    assert!(other_guild_fetch.is_none());

    // 3. Fetch latest transcript by ticket ID
    let latest = store
        .get_latest_transcript(guild_id, ticket_id)
        .await
        .expect("fetching latest transcript should succeed")
        .expect("latest transcript should exist");
    assert_eq!(latest.id, stored.id);

    // 4. Test TranscriptCrypto primitives
    let key1_hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let key2_hex = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

    // Invalid hex lengths
    assert!(TranscriptCrypto::parse_hex_key("too_short").is_err());
    assert!(TranscriptCrypto::parse_hex_key(&format!("{key1_hex}extra")).is_err());

    // Keyring parsing (comma format)
    let keyring_spec = format!("v1:{key1_hex},v2:{key2_hex}");
    let crypto = TranscriptCrypto::from_keyring_spec("v2", &keyring_spec)
        .expect("parse keyring spec should succeed");
    assert_eq!(crypto.active_key_id(), "v2");

    // Keyring parsing (JSON format)
    let json_keyring = format!(r#"{{"v1": "{key1_hex}", "v2": "{key2_hex}"}}"#);
    let crypto_json = TranscriptCrypto::from_keyring_spec("v2", &json_keyring)
        .expect("parse JSON keyring should succeed");
    assert_eq!(crypto_json.active_key_id(), "v2");

    // Redaction in Debug
    let debug_output = format!("{crypto:?}");
    assert!(!debug_output.contains(key1_hex));
    assert!(!debug_output.contains(key2_hex));
    assert!(debug_output.contains("TranscriptCrypto"));

    // Encryption roundtrip
    let payload = crypto
        .encrypt(guild_id, ticket_id, content)
        .expect("encryption should succeed");
    assert_eq!(payload.key_id, "v2");
    assert_eq!(payload.crypto_version, TRANSCRIPT_CRYPTO_VERSION);
    assert_eq!(payload.nonce.len(), 12);

    let decrypted = crypto
        .decrypt(
            &payload.key_id,
            guild_id,
            ticket_id,
            &payload.nonce,
            &payload.ciphertext,
        )
        .expect("decryption should succeed");
    assert_eq!(decrypted, content);

    // Nonce uniqueness across 10 repeated encryptions
    let mut nonces = std::collections::HashSet::new();
    for _ in 0..10 {
        let p = crypto.encrypt(guild_id, ticket_id, content).unwrap();
        assert!(nonces.insert(p.nonce), "Nonces must be unique per write");
    }

    // AAD mismatch: wrong guild
    let wrong_guild_res = crypto.decrypt(
        &payload.key_id,
        "other_guild",
        ticket_id,
        &payload.nonce,
        &payload.ciphertext,
    );
    assert!(
        wrong_guild_res.is_err(),
        "Decryption must fail on guild AAD mismatch"
    );

    // AAD mismatch: wrong ticket
    let wrong_ticket_res = crypto.decrypt(
        &payload.key_id,
        guild_id,
        "other_ticket",
        &payload.nonce,
        &payload.ciphertext,
    );
    assert!(
        wrong_ticket_res.is_err(),
        "Decryption must fail on ticket AAD mismatch"
    );

    // Tampering with ciphertext
    let mut tampered_ciphertext = payload.ciphertext.clone();
    tampered_ciphertext[0] ^= 0x01;
    let tamper_res = crypto.decrypt(
        &payload.key_id,
        guild_id,
        ticket_id,
        &payload.nonce,
        &tampered_ciphertext,
    );
    assert!(
        tamper_res.is_err(),
        "Decryption must fail on tampered ciphertext"
    );

    // Historical key decryption (rotation support)
    let crypto_v1_only = TranscriptCrypto::from_hex_key("v1", key1_hex).unwrap();
    let payload_v1 = crypto_v1_only
        .encrypt(guild_id, ticket_id, "Legacy v1 content")
        .unwrap();

    // Multikey keyring (active = v2) can still decrypt v1
    let decrypted_v1 = crypto
        .decrypt(
            &payload_v1.key_id,
            guild_id,
            ticket_id,
            &payload_v1.nonce,
            &payload_v1.ciphertext,
        )
        .expect("should decrypt legacy v1 record with versioned keyring");
    assert_eq!(decrypted_v1, "Legacy v1 content");

    // Unknown key id fails closed
    let unknown_key_res = crypto.decrypt(
        "v999",
        guild_id,
        ticket_id,
        &payload_v1.nonce,
        &payload_v1.ciphertext,
    );
    assert!(unknown_key_res.is_err(), "Unknown key ID must fail closed");
}

fn base_test_config() -> Config {
    Config {
        discord_token: "dummy_token".to_string(),
        discord_application_id: 123456789,
        discord_client_id: "123456789".to_string(),
        discord_client_secret: "secret".to_string(),
        discord_oauth_redirect_url: "http://127.0.0.1:8080/auth/discord/callback".to_string(),
        discord_dev_guild_id: None,
        dashboard_bind: "127.0.0.1:0".parse().unwrap(),
        dashboard_public_url: "http://127.0.0.1:8080".to_string(),
        database_url: None,
        scripts_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts"),
        session_cookie_secure: false,
        session_ttl_seconds: 86400,
        lua_memory_limit_bytes: 4 * 1024 * 1024,
        lua_instruction_limit: 200_000,
        lua_hook_granularity: 100,
        lua_max_parallel: 32,
        lua_max_per_guild: 8,
        lua_command_timeout_ms: 3000,
        music_allowed_hosts: std::collections::HashSet::new(),
        trusted_proxies: ["127.0.0.1".parse().unwrap(), "::1".parse().unwrap()]
            .into_iter()
            .collect(),
        offline_mode: false,
        discord_api_base_url: "https://discord.com/api/v10".to_string(),
        transcript_encryption_key: None,
        transcript_active_key_id: "v1".to_string(),
        transcript_keyring: None,
        ticket_transcripts_disabled: false,
        integration_credential_key: None,
        integration_credential_active_key_id: "v1".to_string(),
        integration_credential_keyring: None,
        integration_credentials_disabled: true,
        metrics_token: None,
        metrics_allowed_ips: std::collections::HashSet::new(),
    }
}

#[tokio::test]
async fn test_postgres_startup_preflight_fails_fast_without_transcript_encryption_key() {
    // 1. PostgreSQL enabled, no encryption keys, transcripts NOT explicitly disabled:
    // Must fail fast during startup preflight before opening connections.
    let mut config = base_test_config();
    config.database_url = Some("postgres://dummy:dummy@127.0.0.1:5432/dummy".to_string());
    config.transcript_encryption_key = None;
    config.transcript_keyring = None;
    config.ticket_transcripts_disabled = false;

    let res = AppState::bootstrap(config).await;
    let err_msg = match res {
        Err(e) => e.to_string(),
        Ok(_) => panic!("Must fail fast when PostgreSQL has no encryption key"),
    };
    assert!(
        err_msg.contains(
            "PostgreSQL ticket storage requires TRANSCRIPT_KEYRING or TRANSCRIPT_ENCRYPTION_KEY"
        ),
        "Error message should explain requirement: {err_msg}"
    );

    // 2. Malformed keyring: must fail fast with parsing error
    let mut config_malformed = base_test_config();
    config_malformed.database_url = Some("postgres://dummy:dummy@127.0.0.1:5432/dummy".to_string());
    config_malformed.transcript_encryption_key = Some("not_a_valid_64_hex_key".to_string());
    let res_malformed = AppState::bootstrap(config_malformed).await;
    assert!(res_malformed.is_err());

    // 3. PostgreSQL enabled, no keys, BUT ticket_transcripts_disabled is explicitly true:
    // Startup preflight does NOT fail on missing encryption key.
    let mut config_disabled = base_test_config();
    config_disabled.database_url = Some("postgres://dummy:dummy@127.0.0.1:5432/dummy".to_string());
    config_disabled.ticket_transcripts_disabled = true;
    let res_disabled = AppState::bootstrap(config_disabled).await;
    let err_msg_disabled = match res_disabled {
        Err(e) => e.to_string(),
        Ok(_) => panic!("Expected connection failure to dummy PostgreSQL URL"),
    };
    assert!(
        !err_msg_disabled.contains("PostgreSQL ticket storage requires TRANSCRIPT_KEYRING"),
        "Preflight should be skipped when transcripts are explicitly disabled: {err_msg_disabled}"
    );

    // 4. MemoryStore without keys: succeeds without preflight error
    let config_mem = base_test_config();
    assert!(config_mem.database_url.is_none());
    let res_mem = AppState::bootstrap(config_mem).await;
    assert!(res_mem.is_ok(), "MemoryStore must bootstrap without keys");
}

#[test]
fn test_ticket_transcript_format_does_not_persist_expiring_cdn_signatures_issue_371() {
    use rusty_discord_bot::discord::tickets::format_transcript_message;
    use serenity::model::channel::Message;

    let message_json = serde_json::json!({
        "id": "100000000000000001",
        "channel_id": "200000000000000002",
        "author": {
            "id": "300000000000000003",
            "username": "alice",
            "discriminator": "0001",
            "avatar": null
        },
        "content": "Please check this proof screenshot.",
        "timestamp": "2026-11-10T12:00:00Z",
        "tts": false,
        "mention_everyone": false,
        "mentions": [],
        "mention_roles": [],
        "attachments": [
            {
                "id": "400000000000000004",
                "filename": "proof.png",
                "size": 2048,
                "url": "https://cdn.discordapp.com/attachments/200000000000000002/400000000000000004/proof.png?ex=67310000&is=672feb00&hm=deadbeef1234567890abcdef",
                "proxy_url": "https://media.discordapp.net/attachments/200000000000000002/400000000000000004/proof.png",
                "content_type": "image/png"
            }
        ],
        "embeds": [],
        "pinned": false,
        "type": 0
    });

    let message: Message =
        serde_json::from_value(message_json).expect("deserialize serenity message");
    let formatted = format_transcript_message(&message);

    // 1. Expiring signature parameters MUST NOT be persisted in transcript
    assert!(
        !formatted.contains("hm="),
        "must not persist expiring signature hash"
    );
    assert!(
        !formatted.contains("ex="),
        "must not persist expiring expiration timestamp"
    );
    assert!(!formatted.contains("deadbeef1234567890abcdef"));

    // 2. Durable identifiers and metadata MUST be preserved
    assert!(formatted.contains("proof.png"));
    assert!(formatted.contains("channel_id=200000000000000002"));
    assert!(formatted.contains("message_id=100000000000000001"));
    assert!(formatted.contains("attachment_id=400000000000000004"));
    assert!(formatted.contains("size=2048"));
    assert!(formatted.contains("type=image/png"));
    assert!(formatted.contains("archived=false"));
}

#[tokio::test]
async fn test_ticket_activation_error_handling_and_recovery_contract_issue_384() {
    let store = TicketStore::memory();
    let creator_user_id = "100000000000000099";
    let reserved = reserved_ticket(&store, creator_user_id, 1).await;
    assert_eq!(
        reserved.status,
        rusty_discord_bot::tickets::STATUS_PROVISIONING
    );

    // 1. Injected activation failure (simulating DB Err)
    store.set_activation_failure_injected(true);
    let channel_id = "100000000000000088";
    let channel_name = "ticket-000001-problem";
    let activation_res = store
        .activate_ticket(&reserved.id, channel_id, channel_name)
        .await;
    assert!(
        activation_res.is_err(),
        "activation must return Err when injected"
    );

    // 2. Recovery check via get_by_channel shows channel is not linked yet
    let recovery_check = store
        .get_by_channel(&reserved.guild_id, channel_id)
        .await
        .expect("get_by_channel query succeeds");
    assert!(
        recovery_check.is_none(),
        "uncommitted ticket must not be linked to the channel"
    );

    // 3. Compensation: failing provisioning marks ticket as failed and releases user quota
    let fail_res = store
        .fail_provisioning(
            &reserved.id,
            "ticket activation failed: Simulated database activation error",
        )
        .await
        .expect("fail_provisioning succeeds");
    let TicketMutationOutcome::Updated(failed) = fail_res else {
        panic!("expected updated fail outcome");
    };
    assert_eq!(failed.status, rusty_discord_bot::tickets::STATUS_FAILED);
    assert!(
        failed
            .provisioning_error
            .as_deref()
            .unwrap_or("")
            .contains("Simulated database activation error")
    );
    assert!(
        !failed.is_active(),
        "failed ticket must not count as active"
    );

    // 4. User can now reserve a new ticket within their quota (limit 1)
    store.set_activation_failure_injected(false);
    let new_reserved = store
        .reserve_ticket(reservation(creator_user_id), 1)
        .await
        .expect("reservation should succeed after previous provisioning failure");
    assert!(matches!(new_reserved, ReserveTicketOutcome::Reserved(_)));
}
