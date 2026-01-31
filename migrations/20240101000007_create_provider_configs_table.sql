-- Create provider_configs table
CREATE TABLE IF NOT EXISTS provider_configs (
    id CHAR(12) PRIMARY KEY,
    user_id CHAR(12) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider VARCHAR(20) NOT NULL,
    name VARCHAR(100) NOT NULL,
    credentials_encrypted BYTEA NOT NULL,
    encryption_iv BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(user_id, provider, name)
);

-- Create indexes
CREATE INDEX idx_provider_configs_user_id ON provider_configs(user_id);
CREATE INDEX idx_provider_configs_provider ON provider_configs(provider);

-- Create updated_at trigger
CREATE TRIGGER update_provider_configs_updated_at
    BEFORE UPDATE ON provider_configs
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Add check constraint for provider
ALTER TABLE provider_configs ADD CONSTRAINT provider_configs_provider_check
    CHECK (provider IN ('bilibili', 'alist', 'emby'));

-- Comments
COMMENT ON TABLE provider_configs IS 'Encrypted credentials for media providers';
COMMENT ON COLUMN provider_configs.id IS '12-character nanoid';
COMMENT ON COLUMN provider_configs.credentials_encrypted IS 'AES-256-GCM encrypted credentials';
COMMENT ON COLUMN provider_configs.encryption_iv IS 'Initialization vector for encryption';
