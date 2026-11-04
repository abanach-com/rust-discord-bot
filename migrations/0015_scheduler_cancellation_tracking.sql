-- Migration 0015: Scheduler cancellation tracking and graceful in-flight cancellation
ALTER TABLE scheduled_jobs
    ADD COLUMN IF NOT EXISTS cancellation_requested_at TIMESTAMPTZ NULL;
