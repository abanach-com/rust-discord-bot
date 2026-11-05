-- Migration 0020: Trim and sanitize store item role IDs to prevent whitespace discrepancies
UPDATE store_items
SET role_id = BTRIM(role_id)
WHERE role_id IS NOT NULL AND role_id <> BTRIM(role_id);
