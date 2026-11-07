-- 0043_store_purchases_delivery_lease.sql
-- Add lease tracking for concurrent store purchase role fulfillment (#293)

ALTER TABLE store_purchases
    ADD COLUMN IF NOT EXISTS lease_expires_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_store_purchases_lease
    ON store_purchases (fulfillment_status, lease_expires_at)
    WHERE fulfillment_status = 'processing';
