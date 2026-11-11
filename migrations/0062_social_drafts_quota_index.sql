-- Composite index for fast social drafts guild quota queries (Issue #391)
CREATE INDEX IF NOT EXISTS idx_social_drafts_guild_status
    ON social_publication_drafts (guild_id, status);
