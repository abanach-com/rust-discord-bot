-- Migration 0029: Recruitment fulfillment hardening and concurrency
ALTER TABLE recruitment_role_fulfillment
    DROP CONSTRAINT IF EXISTS recruitment_role_fulfillment_status_check;

ALTER TABLE recruitment_role_fulfillment
    ADD CONSTRAINT recruitment_role_fulfillment_status_check
    CHECK (status IN ('pending', 'processing', 'completed', 'failed', 'cancelled'));

CREATE INDEX IF NOT EXISTS idx_recruitment_fulfillment_guild_pending
    ON recruitment_role_fulfillment (guild_id, status, next_attempt_at);
