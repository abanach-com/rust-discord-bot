-- Migration 0019: Member Intelligence - Identity History and Member Snapshots (#159)

CREATE TABLE IF NOT EXISTS member_identity_history (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('username', 'global_name', 'nickname')),
    old_value TEXT,
    new_value TEXT,
    observed_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS member_identity_history_lookup_idx
    ON member_identity_history (guild_id, user_id, observed_at DESC);

CREATE INDEX IF NOT EXISTS member_identity_history_expiry_idx
    ON member_identity_history (expires_at);

CREATE TABLE IF NOT EXISTS member_snapshots (
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    username TEXT NOT NULL,
    global_name TEXT,
    nickname TEXT,
    role_ids JSONB NOT NULL DEFAULT '[]',
    joined_at TIMESTAMPTZ,
    account_created_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (guild_id, user_id)
);

CREATE INDEX IF NOT EXISTS member_snapshots_guild_joined_idx
    ON member_snapshots (guild_id, joined_at DESC);

CREATE INDEX IF NOT EXISTS member_snapshots_guild_created_idx
    ON member_snapshots (guild_id, account_created_at DESC);
