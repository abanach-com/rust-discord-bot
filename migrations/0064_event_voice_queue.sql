-- Migration 0064: Event voice queue configuration, member readiness and durable outbox
CREATE TABLE IF NOT EXISTS event_voice_queue_configs (
    event_id UUID PRIMARY KEY REFERENCES scheduled_events(id) ON DELETE CASCADE,
    guild_id TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT FALSE,
    lobby_channel_id TEXT,
    target_channel_id TEXT,
    mode TEXT NOT NULL DEFAULT 'notify_only',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_event_voice_queue_configs_guild
ON event_voice_queue_configs (guild_id);

CREATE TABLE IF NOT EXISTS event_voice_member_states (
    event_id UUID NOT NULL REFERENCES scheduled_events(id) ON DELETE CASCADE,
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    ready_status TEXT NOT NULL DEFAULT 'waiting',
    consent_auto_move BOOLEAN NOT NULL DEFAULT FALSE,
    generation BIGINT NOT NULL DEFAULT 1,
    ready_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (event_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_event_voice_member_states_guild_event
ON event_voice_member_states (guild_id, event_id);

CREATE TABLE IF NOT EXISTS event_voice_outbox (
    id UUID PRIMARY KEY,
    event_id UUID NOT NULL REFERENCES scheduled_events(id) ON DELETE CASCADE,
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending',
    attempts INT NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    lease_owner TEXT,
    lease_until TIMESTAMPTZ,
    generation BIGINT NOT NULL DEFAULT 1,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_event_voice_outbox_unique_job
ON event_voice_outbox (event_id, user_id, generation, kind);

CREATE INDEX IF NOT EXISTS idx_event_voice_outbox_claim
ON event_voice_outbox (state, next_attempt_at)
WHERE state IN ('pending', 'leased');
