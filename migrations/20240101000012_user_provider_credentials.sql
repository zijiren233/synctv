-- Migration: User Provider Credentials
-- Purpose: Store user credentials for media providers (Bilibili, Alist, Emby)

CREATE TABLE user_provider_credentials (
    -- Primary Key
    id CHAR(12) PRIMARY KEY,  -- nanoid(12)

    -- User and Provider
    user_id CHAR(12) NOT NULL,  -- nanoid(12)
    provider VARCHAR(32) NOT NULL,  -- bilibili, alist, emby

    -- Server Identifier (required, distinguishes different servers/accounts)
    server_id VARCHAR(64) NOT NULL,  -- Alist/Emby: MD5(host), Bilibili: "bilibili" or account id

    -- Associated Provider Instance (optional)
    provider_instance_name VARCHAR(64),

    -- Credential Data (JSONB, plaintext storage per design doc)
    credential_data JSONB NOT NULL,

    -- Expiration Time (optional, for tokens/cookies with TTL)
    expires_at TIMESTAMPTZ,

    -- Audit
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Constraints
    CONSTRAINT fk_user FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    CONSTRAINT fk_provider_instance FOREIGN KEY (provider_instance_name) REFERENCES provider_instances(name) ON DELETE SET NULL,
    CONSTRAINT unique_user_provider_server UNIQUE(user_id, provider, server_id),
    CONSTRAINT valid_server_id CHECK (length(trim(server_id)) > 0 AND length(server_id) <= 64)
);

-- Indexes
CREATE INDEX idx_user_credentials_user ON user_provider_credentials(user_id);
CREATE INDEX idx_user_credentials_provider ON user_provider_credentials(provider);
CREATE INDEX idx_user_credentials_instance ON user_provider_credentials(provider_instance_name);
CREATE INDEX idx_user_credentials_expires ON user_provider_credentials(expires_at) WHERE expires_at IS NOT NULL;

-- Updated At Trigger
CREATE TRIGGER update_user_credentials_updated_at
    BEFORE UPDATE ON user_provider_credentials
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- Comments
COMMENT ON TABLE user_provider_credentials IS 'User credentials for media providers';
COMMENT ON COLUMN user_provider_credentials.provider IS 'Provider type (bilibili, alist, emby)';
COMMENT ON COLUMN user_provider_credentials.server_id IS 'Server identifier (required): Bilibili uses "bilibili" (one per user), Alist/Emby use MD5(host)';
COMMENT ON COLUMN user_provider_credentials.provider_instance_name IS 'Associated provider instance name (optional, for specifying parsing instance)';
COMMENT ON COLUMN user_provider_credentials.credential_data IS 'Credential data (JSONB, plaintext storage)';
COMMENT ON COLUMN user_provider_credentials.expires_at IS 'Credential expiration time (optional, for tokens/cookies with TTL)';
COMMENT ON CONSTRAINT valid_server_id ON user_provider_credentials IS 'server_id must not be empty or whitespace';
COMMENT ON CONSTRAINT unique_user_provider_server ON user_provider_credentials IS 'User can only have one credential per provider per server (Bilibili: one per user, Alist/Emby: multiple allowed)';
