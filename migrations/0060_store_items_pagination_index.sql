-- Migration 0060: Index for Store Items Keyset Pagination (Issue #389)

CREATE INDEX IF NOT EXISTS idx_store_items_page_guild
    ON store_items (guild_id, created_at DESC, id DESC);
