-- Migration 0021: Cleanup Plans for preview/confirm advanced moderation
CREATE TABLE IF NOT EXISTS cleanup_plans (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    actor_user_id TEXT NOT NULL,
    message_ids JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_cleanup_plans_guild_expiry ON cleanup_plans (guild_id, expires_at);
