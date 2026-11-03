-- Migration 0014: Phase 5 Audit Fixes & Multi-Tenant Enforcement

-- Poll votes ledger for one-vote-per-user tracking & fraud prevention (#122)
CREATE TABLE IF NOT EXISTS community_poll_votes (
    poll_id UUID NOT NULL,
    user_id TEXT NOT NULL,
    option_idx INT NOT NULL,
    voted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (poll_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_poll_votes_poll_id ON community_poll_votes (poll_id);
