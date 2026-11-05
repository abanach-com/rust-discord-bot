-- Migration 0018: Add fulfillment tracking and role snapshot to store_purchases (#156)

ALTER TABLE store_purchases
    ADD COLUMN IF NOT EXISTS role_id TEXT,
    ADD COLUMN IF NOT EXISTS fulfillment_status TEXT NOT NULL DEFAULT 'legacy_unverified',
    ADD COLUMN IF NOT EXISTS failure_reason TEXT,
    ADD COLUMN IF NOT EXISTS fulfilled_at TIMESTAMPTZ;

-- Backfill role_id from store_items for existing records where available
UPDATE store_purchases sp
SET role_id = si.role_id
FROM store_items si
WHERE sp.item_id = si.id AND sp.role_id IS NULL;

-- Mark legacy purchases without a role as completed, and legacy purchases with a role as legacy_unverified
UPDATE store_purchases
SET fulfillment_status = 'completed'
WHERE role_id IS NULL AND fulfillment_status = 'legacy_unverified';

CREATE INDEX IF NOT EXISTS idx_store_purchases_fulfillment
ON store_purchases (guild_id, fulfillment_status);
