-- Migration 0026: Moderator Duty & On-Call System (#168)

CREATE TABLE IF NOT EXISTS moderator_duty_settings (
    guild_id TEXT PRIMARY KEY,
    staff_role_id TEXT,
    on_call_role_id TEXT,
    max_override_duration_mins INT NOT NULL DEFAULT 1440,
    fallback_channel_id TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS moderator_duty_profiles (
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    timezone TEXT NOT NULL DEFAULT 'UTC',
    recurring_schedule JSONB NOT NULL DEFAULT '{}'::jsonb,
    manual_override TEXT CHECK (manual_override IN ('force_on', 'force_off') OR manual_override IS NULL),
    override_expires_at TIMESTAMPTZ,
    version BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (guild_id, user_id)
);

CREATE TABLE IF NOT EXISTS moderator_duty_state (
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    expected_on_call BOOLEAN NOT NULL DEFAULT FALSE,
    state_generation BIGINT NOT NULL DEFAULT 0,
    last_reconciled_at TIMESTAMPTZ,
    last_error TEXT,
    PRIMARY KEY (guild_id, user_id)
);

CREATE INDEX IF NOT EXISTS moderator_duty_profiles_guild_idx
    ON moderator_duty_profiles (guild_id);

CREATE INDEX IF NOT EXISTS moderator_duty_state_guild_expected_idx
    ON moderator_duty_state (guild_id, expected_on_call);
