-- Keyset pagination index for workflow definitions (Issue #395)
CREATE INDEX IF NOT EXISTS idx_workflow_definitions_page
    ON workflow_definitions (guild_id, created_at DESC, id DESC);
