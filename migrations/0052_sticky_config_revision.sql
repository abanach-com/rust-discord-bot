-- Migration 0052: Add config_revision UUID to sticky_messages for optimistic concurrency and ABA prevention (#319)

ALTER TABLE sticky_messages
    ADD COLUMN IF NOT EXISTS config_revision UUID NOT NULL DEFAULT gen_random_uuid();
