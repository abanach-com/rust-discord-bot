-- Migration 0044: Reaction Role Grants Ledger (Issue #286)

CREATE TABLE IF NOT EXISTS reaction_role_grants (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    role_id TEXT NOT NULL,
    binding_id UUID NOT NULL REFERENCES reaction_role_bindings(id) ON DELETE CASCADE,
    granted_by_bot BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_reaction_role_grant UNIQUE (guild_id, user_id, binding_id)
);

CREATE INDEX IF NOT EXISTS idx_reaction_role_grants_user_role
    ON reaction_role_grants (guild_id, user_id, role_id);

CREATE INDEX IF NOT EXISTS idx_reaction_role_grants_binding
    ON reaction_role_grants (binding_id);
