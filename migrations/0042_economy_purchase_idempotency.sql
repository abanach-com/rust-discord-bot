-- 0042_economy_purchase_idempotency.sql
-- Persistent request idempotency tracking for /economy buy store purchases

CREATE TABLE IF NOT EXISTS economy_purchase_requests (
    guild_id TEXT NOT NULL,
    interaction_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    item_id UUID NOT NULL,
    purchase_id UUID NOT NULL,
    balance_after BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (guild_id, interaction_id)
);

CREATE INDEX IF NOT EXISTS idx_economy_purchase_requests_created_at 
    ON economy_purchase_requests(created_at);
