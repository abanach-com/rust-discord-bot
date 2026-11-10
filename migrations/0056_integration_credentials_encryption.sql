-- Migration 0056: Application-layer credentials encryption at rest (Issue #349)
-- Integration credentials stored in PostgreSQL (telegram_bridges.secret_token,
-- integration_webhook_subscriptions.secret_token, integration_connections.access_token/refresh_token,
-- gitlab_subscriptions.secret_token, vk_subscriptions.secret_key) are encrypted at rest using
-- AES-256-GCM with envelope format 'enc:v1:<key_id>:<nonce_hex>:<ciphertext_hex>'
-- and strict AAD binding tenant isolation:
-- 'integration:v1:guild={guild_id}:entity={entity}:id={record_id}:field={field}'

COMMENT ON COLUMN telegram_bridges.secret_token IS 'AES-256-GCM encrypted envelope (enc:v1:<key_id>:<nonce_hex>:<ciphertext_hex>) with AAD guild/record binding';
COMMENT ON COLUMN integration_webhook_subscriptions.secret_token IS 'AES-256-GCM encrypted envelope (enc:v1:<key_id>:<nonce_hex>:<ciphertext_hex>) with AAD guild/record binding';
COMMENT ON COLUMN integration_connections.access_token IS 'AES-256-GCM encrypted envelope (enc:v1:<key_id>:<nonce_hex>:<ciphertext_hex>) with AAD guild/record binding';
COMMENT ON COLUMN integration_connections.refresh_token IS 'AES-256-GCM encrypted envelope (enc:v1:<key_id>:<nonce_hex>:<ciphertext_hex>) with AAD guild/record binding';
COMMENT ON COLUMN gitlab_subscriptions.secret_token IS 'AES-256-GCM encrypted envelope (enc:v1:<key_id>:<nonce_hex>:<ciphertext_hex>) with AAD guild/record binding';
COMMENT ON COLUMN vk_subscriptions.secret_key IS 'AES-256-GCM encrypted envelope (enc:v1:<key_id>:<nonce_hex>:<ciphertext_hex>) with AAD guild/record binding';
