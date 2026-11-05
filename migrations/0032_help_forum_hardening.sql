-- Migration 0032: Help Forum Hardening
-- Allow 'needs_tag' status in help_forum_sessions and optimize enabled guild queries

ALTER TABLE help_forum_sessions
    DROP CONSTRAINT IF EXISTS help_forum_sessions_status_check;

ALTER TABLE help_forum_sessions
    ADD CONSTRAINT help_forum_sessions_status_check
    CHECK (status IN ('open', 'solved', 'inactive', 'deleted', 'owner_left', 'needs_tag'));

CREATE INDEX IF NOT EXISTS idx_help_forum_configs_enabled
    ON help_forum_configs (enabled, guild_id);
