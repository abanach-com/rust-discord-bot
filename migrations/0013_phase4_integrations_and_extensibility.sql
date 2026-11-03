-- Migration 0013: Phase 4 Integrations, Ecosystem & Extensibility (#92-#107)

CREATE TABLE IF NOT EXISTS integration_connections (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    provider_account_id TEXT,
    access_token TEXT,
    refresh_token TEXT,
    token_expires_at TIMESTAMPTZ,
    scopes JSONB NOT NULL DEFAULT '[]'::jsonb,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_integration_connections_guild_provider ON integration_connections(guild_id, provider, provider_account_id);

CREATE TABLE IF NOT EXISTS integration_webhook_subscriptions (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    external_id TEXT NOT NULL,
    secret_token TEXT,
    target_channel_id TEXT NOT NULL,
    config JSONB NOT NULL DEFAULT '{}'::jsonb,
    active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_integration_webhook_sub UNIQUE (guild_id, provider, external_id)
);

CREATE TABLE IF NOT EXISTS integration_event_dedupes (
    provider TEXT NOT NULL,
    event_id TEXT NOT NULL,
    guild_id TEXT,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (provider, event_id)
);

CREATE TABLE IF NOT EXISTS integration_delivery_logs (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    event_type TEXT NOT NULL,
    status TEXT NOT NULL,
    error_message TEXT,
    payload_summary JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_integration_delivery_logs_guild ON integration_delivery_logs(guild_id, provider, created_at DESC);

-- Agent 04: Unified Identity (#93)
CREATE TABLE IF NOT EXISTS linked_accounts (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    discord_user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    provider_subject TEXT NOT NULL,
    attributes JSONB NOT NULL DEFAULT '{}'::jsonb,
    verified_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    synced_role_id TEXT,
    CONSTRAINT uq_linked_accounts_subject UNIQUE (guild_id, provider, provider_subject),
    CONSTRAINT uq_linked_accounts_user UNIQUE (guild_id, discord_user_id, provider)
);
CREATE INDEX IF NOT EXISTS idx_linked_accounts_lookup ON linked_accounts(guild_id, provider, discord_user_id);

-- Agent 05 & Agent 15: Steam & Gaming Hub LFG (#94, #104)
CREATE TABLE IF NOT EXISTS steam_lfg_posts (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    author_id TEXT NOT NULL,
    game_id TEXT NOT NULL,
    game_name TEXT NOT NULL,
    description TEXT NOT NULL,
    max_players INT NOT NULL,
    current_players JSONB NOT NULL DEFAULT '[]'::jsonb,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_steam_lfg_posts_guild ON steam_lfg_posts(guild_id, game_id, expires_at);

-- Agent 06: GitHub DevOps (#95)
CREATE TABLE IF NOT EXISTS github_devops_mappings (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    repo_full_name TEXT NOT NULL,
    thread_channel_id TEXT NOT NULL,
    issue_or_pr_number INT NOT NULL,
    discord_thread_id TEXT NOT NULL,
    last_status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_github_devops_mapping UNIQUE (guild_id, repo_full_name, issue_or_pr_number)
);

-- Agent 07: Twitch Extended (#96)
CREATE TABLE IF NOT EXISTS twitch_stream_subscriptions (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    streamer_login TEXT NOT NULL,
    streamer_id TEXT NOT NULL,
    discord_channel_id TEXT NOT NULL,
    live_role_id TEXT,
    last_message_id TEXT,
    is_live BOOLEAN NOT NULL DEFAULT false,
    last_title TEXT,
    last_game TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_twitch_stream_sub UNIQUE (guild_id, streamer_login)
);

-- Agent 08: Kick Integration (#97)
CREATE TABLE IF NOT EXISTS kick_stream_subscriptions (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    slug TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    discord_channel_id TEXT NOT NULL,
    live_role_id TEXT,
    last_message_id TEXT,
    is_live BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_kick_stream_sub UNIQUE (guild_id, slug)
);

-- Agent 09: YouTube Extended (#98)
CREATE TABLE IF NOT EXISTS youtube_channel_subscriptions (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    channel_title TEXT NOT NULL,
    discord_channel_id TEXT NOT NULL,
    last_video_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_youtube_channel_sub UNIQUE (guild_id, channel_id)
);

-- Agent 10: Telegram Bridge (#99)
CREATE TABLE IF NOT EXISTS telegram_bridges (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    telegram_chat_id TEXT NOT NULL,
    discord_channel_id TEXT NOT NULL,
    secret_token TEXT NOT NULL,
    paused BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_telegram_bridge UNIQUE (guild_id, discord_channel_id)
);

CREATE TABLE IF NOT EXISTS telegram_message_mappings (
    id UUID PRIMARY KEY,
    bridge_id UUID NOT NULL REFERENCES telegram_bridges(id) ON DELETE CASCADE,
    telegram_message_id TEXT NOT NULL,
    discord_message_id TEXT NOT NULL,
    direction TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_telegram_msg_map ON telegram_message_mappings(bridge_id, telegram_message_id, discord_message_id);

-- Agent 11: Workflow Builder (#100)
CREATE TABLE IF NOT EXISTS workflow_definitions (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    name TEXT NOT NULL,
    dag_nodes JSONB NOT NULL DEFAULT '[]'::jsonb,
    dag_edges JSONB NOT NULL DEFAULT '[]'::jsonb,
    is_active BOOLEAN NOT NULL DEFAULT false,
    version INT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS workflow_run_executions (
    id UUID PRIMARY KEY,
    workflow_id UUID NOT NULL REFERENCES workflow_definitions(id) ON DELETE CASCADE,
    guild_id TEXT NOT NULL,
    trigger_event TEXT NOT NULL,
    status TEXT NOT NULL,
    logs JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Agent 12: GitLab (#101)
CREATE TABLE IF NOT EXISTS gitlab_subscriptions (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    host TEXT NOT NULL,
    project_id TEXT NOT NULL,
    project_path TEXT NOT NULL,
    secret_token TEXT NOT NULL,
    discord_channel_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_gitlab_sub UNIQUE (guild_id, host, project_id)
);

-- Agent 13: VK Integration (#102)
CREATE TABLE IF NOT EXISTS vk_subscriptions (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    group_id TEXT NOT NULL,
    confirmation_code TEXT NOT NULL,
    secret_key TEXT NOT NULL,
    discord_channel_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_vk_sub UNIQUE (guild_id, group_id)
);

-- Agent 14: Reddit (#103)
CREATE TABLE IF NOT EXISTS reddit_subscriptions (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    subreddit TEXT NOT NULL,
    discord_channel_id TEXT NOT NULL,
    filter_flair TEXT,
    min_score INT NOT NULL DEFAULT 0,
    last_seen_fullname TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_reddit_sub UNIQUE (guild_id, subreddit)
);

-- Agent 15: Gaming Hub Game Statuses (#104)
CREATE TABLE IF NOT EXISTS gaming_hub_game_statuses (
    id UUID PRIMARY KEY,
    game_id TEXT NOT NULL,
    region TEXT NOT NULL,
    server_name TEXT NOT NULL,
    status TEXT NOT NULL,
    player_count INT NOT NULL DEFAULT 0,
    last_checked_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_gaming_hub_game_lookup ON gaming_hub_game_statuses(game_id, region);

-- Agent 16: Social Publisher (#105)
CREATE TABLE IF NOT EXISTS social_publication_drafts (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    author_id TEXT NOT NULL,
    content TEXT NOT NULL,
    media_urls JSONB NOT NULL DEFAULT '[]'::jsonb,
    targets JSONB NOT NULL DEFAULT '[]'::jsonb,
    scheduled_at TIMESTAMPTZ,
    approved_by TEXT,
    status TEXT NOT NULL DEFAULT 'draft',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Agent 17 & 18: AI + MCP (#106)
CREATE TABLE IF NOT EXISTS ai_guild_budgets (
    guild_id TEXT PRIMARY KEY,
    monthly_token_limit BIGINT NOT NULL DEFAULT 1000000,
    used_tokens BIGINT NOT NULL DEFAULT 0,
    monthly_usd_limit DOUBLE PRECISION NOT NULL DEFAULT 10.0,
    used_usd DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    is_halted BOOLEAN NOT NULL DEFAULT false,
    reset_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS ai_knowledge_documents (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    channel_acl JSONB NOT NULL DEFAULT '[]'::jsonb,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    source_type TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS mcp_tool_authorizations (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    allowed_roles JSONB NOT NULL DEFAULT '[]'::jsonb,
    requires_approval BOOLEAN NOT NULL DEFAULT true,
    enabled BOOLEAN NOT NULL DEFAULT true,
    CONSTRAINT uq_mcp_tool_auth UNIQUE (guild_id, tool_name)
);

-- Agent 19: Plugin Marketplace (#107)
CREATE TABLE IF NOT EXISTS plugin_marketplace_packages (
    id UUID PRIMARY KEY,
    plugin_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    author TEXT NOT NULL,
    sha256_hash TEXT NOT NULL,
    manifest JSONB NOT NULL,
    is_verified BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS plugin_guild_installations (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    plugin_id TEXT NOT NULL REFERENCES plugin_marketplace_packages(plugin_id) ON DELETE CASCADE,
    version TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    config JSONB NOT NULL DEFAULT '{}'::jsonb,
    installed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_plugin_guild_inst UNIQUE (guild_id, plugin_id)
);
