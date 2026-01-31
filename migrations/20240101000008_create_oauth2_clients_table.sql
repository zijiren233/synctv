-- Create oauth2_clients table
CREATE TABLE IF NOT EXISTS oauth2_clients (
    id CHAR(12) PRIMARY KEY,
    provider VARCHAR(50) NOT NULL,
    provider_user_id VARCHAR(255) NOT NULL,
    user_id CHAR(12) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    access_token_encrypted BYTEA NOT NULL,
    refresh_token_encrypted BYTEA NULL,
    encryption_iv BYTEA NOT NULL,
    expires_at TIMESTAMPTZ NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(provider, provider_user_id)
);

-- Create indexes
CREATE INDEX idx_oauth2_clients_user_id ON oauth2_clients(user_id);
CREATE INDEX idx_oauth2_clients_provider ON oauth2_clients(provider);
CREATE INDEX idx_oauth2_clients_expires_at ON oauth2_clients(expires_at);

-- Create updated_at trigger
CREATE TRIGGER update_oauth2_clients_updated_at
    BEFORE UPDATE ON oauth2_clients
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Comments
COMMENT ON TABLE oauth2_clients IS 'OAuth2/OIDC authentication tokens';
COMMENT ON COLUMN oauth2_clients.id IS '12-character nanoid';
COMMENT ON COLUMN oauth2_clients.provider IS 'OAuth2 provider (github, google, etc.)';
COMMENT ON COLUMN oauth2_clients.provider_user_id IS 'User ID from OAuth2 provider';
