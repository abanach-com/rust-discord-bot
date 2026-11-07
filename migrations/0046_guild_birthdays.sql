-- Migration 0046: Guild Member Birthdays Storage (Issue #281)

CREATE TABLE IF NOT EXISTS guild_birthdays (
    guild_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    month SMALLINT NOT NULL CHECK (month BETWEEN 1 AND 12),
    day SMALLINT NOT NULL CHECK (day BETWEEN 1 AND 31),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (guild_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_guild_birthdays_date
    ON guild_birthdays (guild_id, month, day);
