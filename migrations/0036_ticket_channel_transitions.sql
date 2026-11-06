-- Per-ticket channel state transitions tracking table (#253)
CREATE TABLE IF NOT EXISTS ticket_channel_transitions (
    ticket_id UUID PRIMARY KEY REFERENCES tickets(id) ON DELETE CASCADE,
    guild_id TEXT NOT NULL,
    expected_version BIGINT NOT NULL CHECK (expected_version > 0),
    target_status TEXT NOT NULL CHECK (target_status IN ('open', 'closed')),
    phase TEXT NOT NULL DEFAULT 'pending'
        CHECK (phase IN ('pending', 'applying')),
    attempts INT NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_ticket_channel_transitions_next ON ticket_channel_transitions(next_attempt_at) WHERE phase = 'pending';
