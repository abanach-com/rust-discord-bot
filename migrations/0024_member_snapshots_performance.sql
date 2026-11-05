-- Migration 0024: Indexes for Member Intelligence performance and keyset pagination (#169)

CREATE INDEX IF NOT EXISTS member_snapshots_keyset_idx
    ON member_snapshots (guild_id, joined_at DESC NULLS LAST, user_id ASC);

CREATE INDEX IF NOT EXISTS member_snapshots_roles_gin_idx
    ON member_snapshots USING gin (role_ids);
