-- Runtime system settings (key-value storage)
-- Settings are grouped by category (e.g., 'server', 'email', 'oauth')
-- Each group stores JSON settings that can be updated at runtime without restart

CREATE TABLE IF NOT EXISTS settings (
    key VARCHAR(200) PRIMARY KEY,
    group VARCHAR(100) NOT NULL,
    value TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_settings_group ON settings(group);

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

-- Add comment
COMMENT ON TABLE settings IS 'Runtime system settings organized by groups with JSON values';
COMMENT ON COLUMN settings.key IS 'Unique settings key (e.g., server, email, oauth)';
COMMENT ON COLUMN settings.group IS 'Settings group name (e.g., server, email, oauth)';
