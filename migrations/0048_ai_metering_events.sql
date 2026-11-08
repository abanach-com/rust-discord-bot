-- Migration 0048: Add AI metering events table for idempotent usage tracking (#311)

CREATE TABLE IF NOT EXISTS ai_metering_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    guild_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    request_id TEXT NOT NULL,
    tokens BIGINT NOT NULL,
    usd DOUBLE PRECISION NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_ai_metering_events_guild_provider_request UNIQUE (guild_id, provider, request_id)
);

CREATE INDEX IF NOT EXISTS idx_ai_metering_events_guild
    ON ai_metering_events (guild_id);
