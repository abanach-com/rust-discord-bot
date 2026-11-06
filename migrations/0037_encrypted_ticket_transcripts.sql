-- Migration 0037: Add AEAD encryption fields for ticket transcripts at rest (#254)

-- Add columns for ciphertext, nonce, key_id and crypto_version
ALTER TABLE ticket_transcripts
    ADD COLUMN IF NOT EXISTS ciphertext BYTEA,
    ADD COLUMN IF NOT EXISTS nonce BYTEA,
    ADD COLUMN IF NOT EXISTS key_id TEXT,
    ADD COLUMN IF NOT EXISTS crypto_version SMALLINT DEFAULT 1;

-- Allow content to be NULL for encrypted records
ALTER TABLE ticket_transcripts
    ALTER COLUMN content DROP NOT NULL;

-- Update content check constraint so NULL content is permitted
ALTER TABLE ticket_transcripts
    DROP CONSTRAINT IF EXISTS ticket_transcripts_content_check;

ALTER TABLE ticket_transcripts
    ADD CONSTRAINT ticket_transcripts_content_check
    CHECK (content IS NULL OR char_length(content) > 0);

-- Ensure each transcript record contains either plaintext content (during migration window)
-- or complete AEAD crypto fields (ciphertext, nonce, key_id)
ALTER TABLE ticket_transcripts
    DROP CONSTRAINT IF EXISTS ticket_transcripts_payload_check;

ALTER TABLE ticket_transcripts
    ADD CONSTRAINT ticket_transcripts_payload_check
    CHECK (content IS NOT NULL OR (ciphertext IS NOT NULL AND nonce IS NOT NULL AND key_id IS NOT NULL));
