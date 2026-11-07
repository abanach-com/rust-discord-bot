-- Migration 0041: Add economy cooldowns and idempotency tables (#278)

CREATE TABLE IF NOT EXISTS economy_cooldowns (
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    action_type TEXT NOT NULL,
    last_action_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (guild_id, user_id, action_type)
);

CREATE TABLE IF NOT EXISTS economy_idempotency (
    guild_id TEXT NOT NULL,
    interaction_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    action_type TEXT NOT NULL,
    outcome_json TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (guild_id, interaction_id)
);

CREATE INDEX IF NOT EXISTS idx_economy_idempotency_created
    ON economy_idempotency (created_at);
