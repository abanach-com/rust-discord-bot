-- Migration 0038: Index for targeted per-member companion channel lease reconciliation (#261)
CREATE INDEX IF NOT EXISTS idx_companion_leases_guild_user_status
ON companion_channel_leases (guild_id, user_id, status);
