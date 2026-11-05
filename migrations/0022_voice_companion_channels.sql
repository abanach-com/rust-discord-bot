-- Migration 0022: Voice Companion Channels
CREATE TABLE IF NOT EXISTS companion_channel_mappings (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    voice_channel_id TEXT NOT NULL,
    text_channel_id TEXT NOT NULL,
    allow_send_messages BOOLEAN NOT NULL DEFAULT true,
    allow_read_history BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT uq_companion_mapping UNIQUE (guild_id, voice_channel_id)
);

CREATE INDEX IF NOT EXISTS idx_companion_mappings_guild ON companion_channel_mappings (guild_id);

CREATE TABLE IF NOT EXISTS companion_channel_leases (
    guild_id TEXT NOT NULL,
    voice_channel_id TEXT NOT NULL,
    text_channel_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    granted_at TIMESTAMPTZ NOT NULL,
    generation UUID NOT NULL,
    PRIMARY KEY (guild_id, text_channel_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_companion_leases_guild ON companion_channel_leases (guild_id);
