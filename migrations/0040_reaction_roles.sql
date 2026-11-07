-- Migration 0040: Reaction Role Bindings (Issue #277)

CREATE TABLE IF NOT EXISTS reaction_role_bindings (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    channel_id TEXT,
    message_id TEXT NOT NULL,
    emoji_key TEXT NOT NULL,
    role_id TEXT NOT NULL,
    created_by TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_reaction_role_binding UNIQUE (guild_id, message_id, emoji_key)
);

CREATE INDEX IF NOT EXISTS idx_reaction_role_bindings_lookup
    ON reaction_role_bindings (guild_id, message_id, emoji_key);

CREATE INDEX IF NOT EXISTS idx_reaction_role_bindings_guild
    ON reaction_role_bindings (guild_id);
