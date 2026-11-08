-- Migration 0049: Enforce finite non-negative USD constraints on AI budgets and metering (#313)

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'chk_ai_metering_events_usd_finite') THEN
        ALTER TABLE ai_metering_events
            ADD CONSTRAINT chk_ai_metering_events_usd_finite
            CHECK (usd >= 0.0 AND usd < 'Infinity'::double precision);
    END IF;

    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'chk_ai_guild_budgets_used_usd_finite') THEN
        ALTER TABLE ai_guild_budgets
            ADD CONSTRAINT chk_ai_guild_budgets_used_usd_finite
            CHECK (used_usd >= 0.0 AND used_usd < 'Infinity'::double precision);
    END IF;
END $$;
