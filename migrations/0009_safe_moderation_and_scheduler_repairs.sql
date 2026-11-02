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

-- 4. Safe migration of legacy moderation cases from 0001 (#51)
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.tables 
        WHERE table_name = 'legacy_0001_moderation_cases'
    ) THEN
        INSERT INTO moderation_cases (
            id,
            case_number,
            guild_id,
            subject_user_id,
            actor_user_id,
            kind,
            status,
            severity,
            points,
            reason,
            source_module,
            visible_to_subject,
            expires_at,
            created_at,
            updated_at,
            version
        )
        SELECT
            gen_random_uuid(),
            row_number() OVER (PARTITION BY guild_id ORDER BY created_at ASC, id ASC),
            guild_id,
            target_user_id,
            moderator_user_id,
            CASE 
                WHEN action IN ('warning', 'staff_note', 'timeout', 'kick', 'ban', 'unban', 'automod') THEN action
                ELSE 'other'
            END,
            CASE 
                WHEN expires_at IS NOT NULL AND expires_at < NOW() THEN 'expired'
                ELSE 'active'
            END,
            'medium',
            0,
            COALESCE(NULLIF(reason, ''), 'Brak podanego powodu'),
            'legacy_migration',
            TRUE,
            expires_at,
            created_at,
            created_at,
            1
        FROM legacy_0001_moderation_cases;

        INSERT INTO moderation_case_counters (guild_id, next_case_number)
        SELECT guild_id, COALESCE(MAX(case_number), 0) + 1
        FROM moderation_cases
        GROUP BY guild_id
        ON CONFLICT (guild_id) DO UPDATE
        SET next_case_number = GREATEST(moderation_case_counters.next_case_number, EXCLUDED.next_case_number);

        DROP TABLE legacy_0001_moderation_cases;
    END IF;
END $$;

