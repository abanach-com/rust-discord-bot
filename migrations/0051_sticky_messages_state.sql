-- Migration 0051: Extend sticky_messages with delivery state and generation tracking (#316)

ALTER TABLE sticky_messages
    ADD COLUMN IF NOT EXISTS last_posted_message_id TEXT,
    ADD COLUMN IF NOT EXISTS last_bumped_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS generation BIGINT NOT NULL DEFAULT 0;
