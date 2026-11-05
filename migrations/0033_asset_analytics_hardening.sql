-- Migration 0033: Asset Analytics Hardening, Indexing, and Ownership Refinement
ALTER TABLE guild_asset_inventory
    ADD COLUMN IF NOT EXISTS is_active BOOLEAN NOT NULL DEFAULT TRUE;

ALTER TABLE guild_asset_inventory
    ALTER COLUMN is_guild_owned DROP NOT NULL,
    ALTER COLUMN is_guild_owned DROP DEFAULT;

CREATE INDEX IF NOT EXISTS guild_asset_usage_guild_kind_date_idx
    ON guild_asset_usage_daily (guild_id, asset_kind, usage_date);

CREATE INDEX IF NOT EXISTS guild_asset_inventory_active_idx
    ON guild_asset_inventory (guild_id, asset_kind, is_active);
