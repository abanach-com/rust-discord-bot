-- 0010_phase1_governance_and_defense.sql
-- 1. Appeals table for Issue #74 (Moderation appeals)
CREATE TABLE IF NOT EXISTS moderation_appeals (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    case_number BIGINT NOT NULL,
    user_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    reason TEXT NOT NULL,
    evidence_url TEXT,
    staff_notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS moderation_appeals_guild_status_idx
    ON moderation_appeals (guild_id, status);
CREATE INDEX IF NOT EXISTS moderation_appeals_guild_user_idx
    ON moderation_appeals (guild_id, user_id);

-- 2. Module drafts table for Issue #73 (Draft vs Published config)
CREATE TABLE IF NOT EXISTS module_draft_configs (
    guild_id TEXT NOT NULL,
    module_id TEXT NOT NULL,
    config_json JSONB NOT NULL,
    updated_by TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (guild_id, module_id)
);

-- 3. Timed roles grants table for Issue #76 (Expiring roles)
CREATE TABLE IF NOT EXISTS timed_role_grants (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    role_id TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS timed_role_grants_expires_idx
    ON timed_role_grants (expires_at);
CREATE INDEX IF NOT EXISTS timed_role_grants_user_idx
    ON timed_role_grants (guild_id, user_id);

-- 4. Persistent user roles table for Issue #76 (Role re-grant upon rejoin)
CREATE TABLE IF NOT EXISTS persistent_user_roles (
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    role_ids JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (guild_id, user_id)
);
