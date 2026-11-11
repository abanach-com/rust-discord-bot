-- Migration 0067: Marketplace Immutable Releases and Guild Installation Pinning (Issue #407)

CREATE TABLE IF NOT EXISTS plugin_marketplace_releases (
    plugin_id TEXT NOT NULL REFERENCES plugin_marketplace_packages(plugin_id) ON DELETE CASCADE,
    version TEXT NOT NULL,
    sha256_hash TEXT NOT NULL,
    manifest JSONB NOT NULL,
    is_verified BOOLEAN NOT NULL DEFAULT false,
    published_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (plugin_id, version)
);

-- Backfill existing current releases from plugin_marketplace_packages
INSERT INTO plugin_marketplace_releases
    (plugin_id, version, sha256_hash, manifest, is_verified, published_at)
SELECT plugin_id, version, sha256_hash, manifest, is_verified, created_at
FROM plugin_marketplace_packages
ON CONFLICT (plugin_id, version) DO NOTHING;

-- Clean up any orphaned installations referencing nonexistent releases
DELETE FROM plugin_guild_installations pgi
WHERE NOT EXISTS (
    SELECT 1 FROM plugin_marketplace_releases pmr
    WHERE pmr.plugin_id = pgi.plugin_id AND pmr.version = pgi.version
);

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'fk_plugin_guild_installations_release'
    ) THEN
        ALTER TABLE plugin_guild_installations
        ADD CONSTRAINT fk_plugin_guild_installations_release
        FOREIGN KEY (plugin_id, version)
        REFERENCES plugin_marketplace_releases(plugin_id, version)
        ON DELETE RESTRICT;
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS idx_plugin_marketplace_releases_pkg
    ON plugin_marketplace_releases (plugin_id, published_at DESC);
