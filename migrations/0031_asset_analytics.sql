-- Migration 0031: Emoji & Sticker Asset Usage Analytics and Inventory
CREATE TABLE IF NOT EXISTS guild_asset_usage_daily (
    guild_id TEXT NOT NULL,
    usage_date DATE NOT NULL,
    asset_kind TEXT NOT NULL CHECK (asset_kind IN ('emoji', 'sticker')),
    asset_id TEXT NOT NULL,
    usage_count BIGINT NOT NULL CHECK (usage_count >= 0),
    last_used_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (guild_id, usage_date, asset_kind, asset_id)
);

CREATE INDEX IF NOT EXISTS guild_asset_usage_recent_idx
    ON guild_asset_usage_daily (guild_id, asset_kind, last_used_at DESC);

CREATE INDEX IF NOT EXISTS guild_asset_usage_date_idx
    ON guild_asset_usage_daily (guild_id, usage_date);

CREATE TABLE IF NOT EXISTS guild_asset_inventory (
    guild_id TEXT NOT NULL,
    asset_kind TEXT NOT NULL CHECK (asset_kind IN ('emoji', 'sticker')),
    asset_id TEXT NOT NULL,
    name TEXT,
    animated BOOLEAN NOT NULL DEFAULT FALSE,
    is_guild_owned BOOLEAN NOT NULL DEFAULT TRUE,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (guild_id, asset_kind, asset_id)
);

CREATE INDEX IF NOT EXISTS guild_asset_inventory_kind_idx
    ON guild_asset_inventory (guild_id, asset_kind);
