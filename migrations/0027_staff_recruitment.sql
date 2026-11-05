-- Migration 0027: Staff Recruitment Pipeline
-- Feature #166: Tracks, Applications, Reviews, and Role Fulfillment Outbox

CREATE TABLE IF NOT EXISTS recruitment_tracks (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    slug TEXT NOT NULL,
    name TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    target_role_id TEXT,
    config JSONB NOT NULL DEFAULT '{}'::jsonb,
    version BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (guild_id, slug)
);

CREATE INDEX IF NOT EXISTS idx_recruitment_tracks_guild
    ON recruitment_tracks (guild_id);

CREATE TABLE IF NOT EXISTS recruitment_applications (
    id UUID PRIMARY KEY,
    guild_id TEXT NOT NULL,
    track_id UUID NOT NULL REFERENCES recruitment_tracks(id) ON DELETE CASCADE,
    applicant_user_id TEXT NOT NULL,
    track_version BIGINT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('draft', 'submitted', 'screening', 'interview', 'accepted', 'rejected', 'withdrawn', 'expired')),
    submitted_answers JSONB NOT NULL DEFAULT '[]'::jsonb,
    current_stage TEXT NOT NULL DEFAULT 'submitted',
    version BIGINT NOT NULL DEFAULT 1,
    submitted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    decided_at TIMESTAMPTZ,
    decision_reason TEXT,
    decided_by TEXT
);

CREATE INDEX IF NOT EXISTS idx_recruitment_applications_queue
    ON recruitment_applications (guild_id, track_id, status, submitted_at);

CREATE INDEX IF NOT EXISTS idx_recruitment_applications_applicant
    ON recruitment_applications (guild_id, applicant_user_id);

CREATE TABLE IF NOT EXISTS recruitment_reviews (
    id UUID PRIMARY KEY,
    application_id UUID NOT NULL REFERENCES recruitment_applications(id) ON DELETE CASCADE,
    guild_id TEXT NOT NULL,
    reviewer_user_id TEXT NOT NULL,
    recommendation TEXT NOT NULL CHECK (recommendation IN ('approve', 'reject', 'needs_info', 'abstain')),
    note TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (application_id, reviewer_user_id)
);

CREATE INDEX IF NOT EXISTS idx_recruitment_reviews_app
    ON recruitment_reviews (application_id);

CREATE TABLE IF NOT EXISTS recruitment_role_fulfillment (
    application_id UUID PRIMARY KEY REFERENCES recruitment_applications(id) ON DELETE CASCADE,
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    role_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'completed', 'failed')),
    attempts INT NOT NULL DEFAULT 0,
    last_error TEXT,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_recruitment_fulfillment_pending
    ON recruitment_role_fulfillment (status, next_attempt_at);
