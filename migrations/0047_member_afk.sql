-- Migration 0047: Member AFK status storage (Issue #280)

CREATE TABLE IF NOT EXISTS member_afk (
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    reason TEXT NOT NULL,
    set_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (guild_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_member_afk_lookup
    ON member_afk (guild_id, user_id);
