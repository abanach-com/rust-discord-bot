ALTER TABLE moderator_duty_settings
    ADD COLUMN role_authorized_by_user_id TEXT,
    ADD COLUMN version BIGINT NOT NULL DEFAULT 1;
