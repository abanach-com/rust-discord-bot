-- Indexes for integration dedupe and delivery logs retention and pagination (Issue #346)
CREATE INDEX IF NOT EXISTS idx_event_dedupes_processed_at ON integration_event_dedupes (processed_at);
CREATE INDEX IF NOT EXISTS idx_delivery_logs_created_at ON integration_delivery_logs (created_at);
CREATE INDEX IF NOT EXISTS idx_delivery_logs_pagination ON integration_delivery_logs (guild_id, created_at DESC, id DESC);
