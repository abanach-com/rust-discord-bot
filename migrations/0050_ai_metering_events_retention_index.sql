-- Migration 0050: Add index on (created_at, id) for ai_metering_events retention pruning (#314)

CREATE INDEX IF NOT EXISTS idx_ai_metering_events_created_at_id
    ON ai_metering_events (created_at, id);
