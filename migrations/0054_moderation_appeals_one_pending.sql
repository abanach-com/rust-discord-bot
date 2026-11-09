-- Migration 0054: Limit active moderation appeals to one pending appeal per (guild_id, case_number, user_id)
-- If there are existing duplicate pending appeals, keep the earliest one and mark newer ones superseded
WITH ranked_dupes AS (
    SELECT id,
           ROW_NUMBER() OVER (
               PARTITION BY guild_id, case_number, user_id
               ORDER BY created_at ASC, id ASC
           ) as rn
    FROM moderation_appeals
    WHERE status = 'pending'
)
UPDATE moderation_appeals
SET status = 'superseded', updated_at = NOW()
WHERE id IN (
    SELECT id FROM ranked_dupes WHERE rn > 1
);

CREATE UNIQUE INDEX IF NOT EXISTS moderation_appeals_one_pending_per_case
ON moderation_appeals (guild_id, case_number, user_id)
WHERE status = 'pending';
