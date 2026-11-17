-- When 0067 is pending, the application creates this before applying 0067 in the
-- same transaction. Otherwise this creates the table for operators recovering
-- rows from a backup. No historical release or active installation is inferred.
CREATE TABLE IF NOT EXISTS plugin_guild_installation_quarantine (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    plugin_id TEXT NOT NULL,
    version TEXT NOT NULL,
    enabled BOOLEAN NOT NULL,
    config JSONB NOT NULL,
    installed_at TIMESTAMPTZ NOT NULL,
    UNIQUE (guild_id, plugin_id)
);
