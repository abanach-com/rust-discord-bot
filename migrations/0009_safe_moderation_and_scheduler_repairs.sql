-- 0009_safe_moderation_and_scheduler_repairs.sql
-- 1. Scheduler task repair: jobs with completed_at must be 'completed', exhausted jobs 'failed' (#61)
UPDATE scheduled_jobs
SET status = 'completed'
WHERE completed_at IS NOT NULL AND status <> 'completed';

UPDATE scheduled_jobs
SET status = 'failed', last_error = COALESCE(last_error, 'worker lease expired after reaching maximum attempts')
WHERE status = 'active' AND attempts >= max_attempts;

-- 2. Granted role rewards table for idempotent progression rewards without re-granting revoked roles (#43)
CREATE TABLE IF NOT EXISTS member_role_rewards (
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    role_id TEXT NOT NULL,
    level BIGINT NOT NULL,
    granted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (guild_id, user_id, role_id)
);
CREATE INDEX IF NOT EXISTS member_role_rewards_guild_user_idx
    ON member_role_rewards (guild_id, user_id);

-- 3. Moderation intent ledger for resilient 2-phase operations (#50)
CREATE TABLE IF NOT EXISTS moderation_execution_intents (
    id UUID PRIMARY KEY,
    interaction_id TEXT,
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    step TEXT NOT NULL,
    reason TEXT,
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS moderation_execution_intents_lookup_idx
    ON moderation_execution_intents (guild_id, user_id, kind, step);
