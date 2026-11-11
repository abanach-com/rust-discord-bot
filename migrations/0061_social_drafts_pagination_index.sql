-- Keyset pagination index for social publication drafts queue (Issue #390)
CREATE INDEX IF NOT EXISTS idx_social_drafts_page
    ON social_publication_drafts (guild_id, status, created_at DESC, id DESC);
