-- Steam LFG retention janitor performance index (#238)
CREATE INDEX IF NOT EXISTS idx_steam_lfg_posts_expires_at ON steam_lfg_posts(expires_at);
