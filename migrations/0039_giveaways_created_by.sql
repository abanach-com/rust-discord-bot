ALTER TABLE giveaways
    ADD COLUMN IF NOT EXISTS created_by_user_id TEXT NULL;

CREATE INDEX IF NOT EXISTS idx_giveaways_guild_creator
    ON giveaways (guild_id, created_by_user_id);
