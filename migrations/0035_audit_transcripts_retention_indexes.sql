-- Audit events and ticket transcripts retention and pagination indexes (#249, #251)
CREATE INDEX IF NOT EXISTS idx_audit_events_created_at ON audit_events(created_at);
CREATE INDEX IF NOT EXISTS idx_audit_events_guild_created_id ON audit_events(guild_id, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_ticket_transcripts_created_at ON ticket_transcripts(created_at);
