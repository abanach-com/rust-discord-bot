-- Keyset pagination index for scheduled events (Issue #393)
CREATE INDEX IF NOT EXISTS idx_scheduled_events_page
    ON scheduled_events (guild_id, start_time ASC, id ASC);
