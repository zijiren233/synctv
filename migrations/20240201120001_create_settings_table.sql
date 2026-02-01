-- Runtime system settings (key-value storage)
-- Settings are grouped by category (e.g., 'server', 'email', 'oauth')
-- Each group stores JSON settings that can be updated at runtime without restart

CREATE TABLE IF NOT EXISTS settings (
    id BIGSERIAL PRIMARY KEY,
    group_name VARCHAR(100) NOT NULL UNIQUE,
    settings_json JSONB NOT NULL DEFAULT '{}',
    description TEXT,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- Index for faster group lookups
CREATE INDEX IF NOT EXISTS idx_settings_group_name ON settings(group_name);

-- Trigger to update updated_at timestamp
CREATE OR REPLACE FUNCTION update_settings_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trigger_update_settings_updated_at
BEFORE UPDATE ON settings
FOR EACH ROW
EXECUTE FUNCTION update_settings_updated_at();

-- Insert default settings groups
INSERT INTO settings (group_name, settings_json, description) VALUES
(
    'server',
    '{
        "allow_registration": true,
        "allow_room_creation": true,
        "max_rooms_per_user": 10,
        "max_members_per_room": 100,
        "default_room_settings": {
            "require_password": false,
            "allow_guest": true
        }
    }'::jsonb,
    'Server-wide settings for user and room management'
),
(
    'email',
    '{
        "enabled": false,
        "smtp_host": "",
        "smtp_port": 587,
        "smtp_username": "",
        "use_tls": true,
        "from_address": "noreply@synctv.example.com",
        "from_name": "SyncTV"
    }'::jsonb,
    'Email configuration for notifications and password reset'
),
(
    'oauth',
    '{
        "github_enabled": false,
        "google_enabled": false,
        "microsoft_enabled": false,
        "discord_enabled": false
    }'::jsonb,
    'OAuth2/OIDC provider settings'
),
(
    'rate_limit',
    '{
        "enabled": true,
        "api_rate_limit": 100,
        "api_rate_window": 60,
        "ws_rate_limit": 50,
        "ws_rate_window": 60
    }'::jsonb,
    'Rate limiting configuration'
),
(
    'content_moderation',
    '{
        "enabled": false,
        "filter_profanity": false,
        "max_message_length": 1000,
        "link_filter_enabled": false
    }'::jsonb,
    'Content moderation and filtering settings'
)
ON CONFLICT (group_name) DO NOTHING;

-- Add comment
COMMENT ON TABLE settings IS 'Runtime system settings organized by groups with JSON values';
COMMENT ON COLUMN settings.group_name IS 'Unique group identifier (e.g., server, email, oauth)';
COMMENT ON COLUMN settings.settings_json IS 'JSON settings for this group';
COMMENT ON COLUMN settings.description IS 'Human-readable description of this settings group';
