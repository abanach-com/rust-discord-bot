-- Migration 0025: Unified Command Policy Engine (#171)

CREATE TABLE IF NOT EXISTS command_policy_rules (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    subject_kind TEXT NOT NULL CHECK (subject_kind IN ('user', 'role', 'channel', 'voice_channel', 'guild')),
    subject_id TEXT,
    resource_kind TEXT NOT NULL CHECK (resource_kind IN ('command', 'module', 'action')),
    module_id TEXT,
    command_name TEXT,
    action_key TEXT,
    effect TEXT NOT NULL CHECK (effect IN ('allow', 'deny')),
    priority INT NOT NULL DEFAULT 0,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    version BIGINT NOT NULL DEFAULT 1,
    created_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS command_policy_rules_lookup_idx
    ON command_policy_rules (guild_id, resource_kind, module_id, command_name, enabled);

CREATE INDEX IF NOT EXISTS command_policy_rules_subject_idx
    ON command_policy_rules (guild_id, subject_kind, subject_id);

CREATE TABLE IF NOT EXISTS command_policy_revisions (
    guild_id TEXT PRIMARY KEY,
    revision BIGINT NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
