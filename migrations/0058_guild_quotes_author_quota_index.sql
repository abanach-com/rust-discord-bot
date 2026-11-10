-- Migration 0058: Index for Guild Quotes Quota Checks (Issue #385)
CREATE INDEX IF NOT EXISTS idx_guild_quotes_author_quota ON guild_quotes (guild_id, added_by);
