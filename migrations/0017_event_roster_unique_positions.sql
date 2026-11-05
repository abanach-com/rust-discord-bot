-- Migration 0017: Repair historical duplicate positions in event_rosters and enforce unique (event_id, position) (#155)

-- 1. Deterministically renumber positions per event_id based on (position, joined_at, user_id)
WITH renumbered AS (
    SELECT
        event_id,
        user_id,
        ROW_NUMBER() OVER (
            PARTITION BY event_id
            ORDER BY position ASC, joined_at ASC, user_id ASC
        )::INT AS new_pos
    FROM event_rosters
)
UPDATE event_rosters er
SET position = r.new_pos
FROM renumbered r
WHERE er.event_id = r.event_id
  AND er.user_id = r.user_id
  AND er.position <> r.new_pos;

-- 2. Add unique index on (event_id, position)
CREATE UNIQUE INDEX IF NOT EXISTS uq_event_rosters_event_position
ON event_rosters (event_id, position);
