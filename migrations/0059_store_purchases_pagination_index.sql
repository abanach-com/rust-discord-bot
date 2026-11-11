-- Migration 0059: Indexes for Store Purchases Keyset Pagination (Issue #387)

CREATE INDEX IF NOT EXISTS idx_store_purchases_page_guild_user
    ON store_purchases (guild_id, user_id, created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_store_purchases_page_guild
    ON store_purchases (guild_id, created_at DESC, id DESC);
