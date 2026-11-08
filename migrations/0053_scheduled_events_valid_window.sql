-- Migration 0053: Enforce valid chronological window for scheduled events (Issue #331)

-- Sanitize any legacy records with invalid timestamps before adding constraint
UPDATE scheduled_events
SET end_time = NULL
WHERE end_time IS NOT NULL AND end_time <= start_time;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'scheduled_events_valid_window'
    ) THEN
        ALTER TABLE scheduled_events
            ADD CONSTRAINT scheduled_events_valid_window
            CHECK (end_time IS NULL OR end_time > start_time);
    END IF;
END $$;
