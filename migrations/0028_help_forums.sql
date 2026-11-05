-- Migration 0028: Help Forum Lifecycle
-- Feature #164: Public help sessions, solved state, and inactivity auto-archive

CREATE TABLE IF NOT EXISTS help_forum_configs (
    guild_id TEXT NOT NULL,
    forum_channel_id TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    idle_close_minutes INT NOT NULL DEFAULT 1440,
    owner_can_close BOOLEAN NOT NULL DEFAULT TRUE,
    helper_role_ids JSONB NOT NULL DEFAULT '[]'::jsonb,
    lock_on_close BOOLEAN NOT NULL DEFAULT TRUE,
    archive_on_close BOOLEAN NOT NULL DEFAULT TRUE,
    require_question_tag BOOLEAN NOT NULL DEFAULT FALSE,
    welcome_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (guild_id, forum_channel_id)
);

CREATE INDEX IF NOT EXISTS idx_help_forum_configs_guild
    ON help_forum_configs (guild_id);

CREATE TABLE IF NOT EXISTS help_forum_sessions (
    guild_id TEXT NOT NULL,
    forum_channel_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    owner_user_id TEXT NOT NULL,
    status TEXT NOT NULL
        CHECK (status IN ('open', 'solved', 'inactive', 'deleted', 'owner_left')),
    opened_at TIMESTAMPTZ NOT NULL,
    last_activity_at TIMESTAMPTZ NOT NULL,
    first_helper_response_at TIMESTAMPTZ,
    closed_at TIMESTAMPTZ,
    closing_reason TEXT,
    generation BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (guild_id, thread_id)
);

CREATE INDEX IF NOT EXISTS help_forum_open_deadline_idx
    ON help_forum_sessions (guild_id, status, last_activity_at)
    WHERE status = 'open';

CREATE INDEX IF NOT EXISTS idx_help_forum_sessions_forum
    ON help_forum_sessions (guild_id, forum_channel_id);
