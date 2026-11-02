-- Phase 3: Games, RP & Knowledge (Issues #89-#91)

CREATE TABLE IF NOT EXISTS game_account_links (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    platform TEXT NOT NULL,
    platform_account_id TEXT NOT NULL,
    verification_code TEXT,
    verified BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_game_link UNIQUE (guild_id, user_id, platform)
);

CREATE INDEX IF NOT EXISTS idx_game_links_guild_platform
ON game_account_links (guild_id, platform, verified);

CREATE TABLE IF NOT EXISTS counting_games (
    guild_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    current_count BIGINT NOT NULL DEFAULT 0,
    last_user_id TEXT,
    highest_count BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (guild_id, channel_id)
);

CREATE TABLE IF NOT EXISTS card_collections (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    card_id TEXT NOT NULL,
    card_name TEXT NOT NULL,
    rarity TEXT NOT NULL DEFAULT 'common',
    acquired_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_card_collections_guild_user
ON card_collections (guild_id, user_id);

CREATE TABLE IF NOT EXISTS card_trades (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    initiator_user_id TEXT NOT NULL,
    initiator_card_id UUID NOT NULL,
    target_user_id TEXT NOT NULL,
    target_card_id UUID NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_card_trades_guild_status
ON card_trades (guild_id, status);

CREATE TABLE IF NOT EXISTS tupper_profiles (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    avatar_url TEXT,
    prefix_or_brackets TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_tupper_user_prefix UNIQUE (guild_id, user_id, prefix_or_brackets)
);

CREATE INDEX IF NOT EXISTS idx_tupper_profiles_guild_user
ON tupper_profiles (guild_id, user_id);

CREATE TABLE IF NOT EXISTS faq_entries (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    question TEXT NOT NULL,
    answer TEXT NOT NULL,
    aliases_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    category TEXT NOT NULL DEFAULT 'general',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_faq_entries_guild_category
ON faq_entries (guild_id, category);
