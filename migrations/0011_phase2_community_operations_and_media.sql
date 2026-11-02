-- Phase 2: Community, Operations, Media & Platform (Issues #78-#88)

CREATE TABLE IF NOT EXISTS server_snapshots (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    name TEXT NOT NULL,
    snapshot_json JSONB NOT NULL,
    created_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_server_snapshots_guild_created
ON server_snapshots (guild_id, created_at DESC);

CREATE TABLE IF NOT EXISTS custom_automations (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    name TEXT NOT NULL,
    trigger_type TEXT NOT NULL,
    conditions_json JSONB NOT NULL,
    actions_json JSONB NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_custom_automations_guild
ON custom_automations (guild_id, enabled);

CREATE TABLE IF NOT EXISTS sticky_messages (
    guild_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    content TEXT NOT NULL,
    cooldown_seconds INT NOT NULL DEFAULT 5,
    last_updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (guild_id, channel_id)
);

CREATE TABLE IF NOT EXISTS community_polls (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    question TEXT NOT NULL,
    options_json JSONB NOT NULL,
    tallies_json JSONB NOT NULL,
    is_anonymous BOOLEAN NOT NULL DEFAULT FALSE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_community_polls_guild_expires
ON community_polls (guild_id, expires_at);

CREATE TABLE IF NOT EXISTS giveaways (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    prize TEXT NOT NULL,
    winners_count INT NOT NULL DEFAULT 1,
    participants_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    winners_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    ended BOOLEAN NOT NULL DEFAULT FALSE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_giveaways_guild_ended
ON giveaways (guild_id, ended, expires_at);

CREATE TABLE IF NOT EXISTS starboard_entries (
    guild_id TEXT NOT NULL,
    source_channel_id TEXT NOT NULL,
    source_message_id TEXT NOT NULL,
    starboard_message_id TEXT NOT NULL,
    reaction_count INT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (guild_id, source_message_id)
);

CREATE TABLE IF NOT EXISTS store_items (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    price BIGINT NOT NULL,
    role_id TEXT,
    stock INT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_store_items_guild
ON store_items (guild_id);

CREATE TABLE IF NOT EXISTS store_purchases (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    item_id UUID NOT NULL REFERENCES store_items(id) ON DELETE CASCADE,
    price_paid BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_store_purchases_guild_user
ON store_purchases (guild_id, user_id);

CREATE TABLE IF NOT EXISTS progression_seasons (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    season_number INT NOT NULL,
    name TEXT NOT NULL,
    standings_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    archived_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_progression_seasons_guild
ON progression_seasons (guild_id, season_number DESC);

CREATE TABLE IF NOT EXISTS scheduled_events (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    start_time TIMESTAMPTZ NOT NULL,
    end_time TIMESTAMPTZ,
    timezone TEXT NOT NULL DEFAULT 'UTC',
    max_participants INT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_scheduled_events_guild_start
ON scheduled_events (guild_id, start_time);

CREATE TABLE IF NOT EXISTS event_rosters (
    event_id UUID NOT NULL REFERENCES scheduled_events(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    role_type TEXT NOT NULL DEFAULT 'main',
    position INT NOT NULL DEFAULT 0,
    joined_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (event_id, user_id)
);

CREATE TABLE IF NOT EXISTS tournaments (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    name TEXT NOT NULL,
    bracket_json JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_tournaments_guild
ON tournaments (guild_id);

CREATE TABLE IF NOT EXISTS invites_tracking (
    guild_id TEXT NOT NULL,
    code TEXT NOT NULL,
    inviter_id TEXT NOT NULL,
    uses INT NOT NULL DEFAULT 0,
    campaign_tag TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (guild_id, code)
);

CREATE TABLE IF NOT EXISTS dynamic_voice_rooms (
    channel_id TEXT PRIMARY KEY,
    guild_id TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    empty_since TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_dynamic_voice_rooms_guild
ON dynamic_voice_rooms (guild_id);

CREATE TABLE IF NOT EXISTS feed_subscriptions (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    feed_type TEXT NOT NULL,
    url_or_target TEXT NOT NULL,
    watermark TEXT,
    last_polled_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_feed_subscriptions_guild_type
ON feed_subscriptions (guild_id, feed_type);
