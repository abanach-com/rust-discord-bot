-- Migration 0045: Guild Quotes Storage (Issue #282)

CREATE TABLE IF NOT EXISTS guild_quotes (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    added_by TEXT NOT NULL,
    author_id TEXT NOT NULL,
    body TEXT NOT NULL CHECK (char_length(body) BETWEEN 1 AND 500),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_guild_quotes_history
    ON guild_quotes (guild_id, created_at DESC, id DESC);
