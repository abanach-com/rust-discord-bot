-- Migration 0030: Moderator Duty bot-owned role grants and revoke lifecycle
CREATE TABLE IF NOT EXISTS moderator_duty_role_grants (
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    role_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'revoke_pending', 'revoked', 'failed')),
    attempts INT NOT NULL DEFAULT 0,
    last_error TEXT,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    generation BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (guild_id, user_id, role_id)
);

CREATE INDEX IF NOT EXISTS idx_duty_role_grants_revoke_pending
    ON moderator_duty_role_grants (guild_id, status, next_attempt_at);
