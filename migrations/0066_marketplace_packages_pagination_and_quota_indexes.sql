-- Keyset pagination index and author quota index for marketplace packages (Issue #404)
CREATE INDEX IF NOT EXISTS idx_plugin_marketplace_packages_cursor
    ON plugin_marketplace_packages (created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_plugin_marketplace_packages_author
    ON plugin_marketplace_packages (author);
