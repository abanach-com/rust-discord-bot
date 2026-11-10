-- Migration 0057: Index for AI knowledge documents guild lookups and quota enforcement (Issue #377)
CREATE INDEX IF NOT EXISTS idx_ai_knowledge_documents_guild_id ON ai_knowledge_documents (guild_id);
