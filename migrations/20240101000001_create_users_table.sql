-- Create users table
CREATE TABLE IF NOT EXISTS users (
    id CHAR(12) PRIMARY KEY,
    username VARCHAR(50) UNIQUE NOT NULL,
    email VARCHAR(255) UNIQUE,  -- NULL allowed (e.g., OAuth2 users without email)
    password_hash VARCHAR(255) NOT NULL,
    permissions BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    deleted_at TIMESTAMPTZ NULL,

    -- Ensure email is not empty or whitespace-only
    CONSTRAINT users_email_not_empty CHECK (email IS NULL OR length(trim(email)) > 0)
);

-- Create indexes
-- Note: UNIQUE constraints on username/email are global (including soft-deleted records)
-- This ensures username/email can NEVER be reused, even after account deletion
-- For email, NULL values don't count as duplicates (multiple users can have NULL email)
CREATE INDEX idx_users_username ON users(username) WHERE deleted_at IS NULL;
CREATE INDEX idx_users_email ON users(email) WHERE deleted_at IS NULL;
CREATE INDEX idx_users_created_at ON users(created_at);
CREATE INDEX idx_users_deleted_at ON users(deleted_at) WHERE deleted_at IS NOT NULL;

-- Performance optimization indexes
CREATE INDEX idx_users_username_lower ON users(LOWER(username)) WHERE deleted_at IS NULL;
CREATE INDEX idx_users_email_lower ON users(LOWER(email)) WHERE deleted_at IS NULL AND email IS NOT NULL;
CREATE INDEX idx_users_permissions ON users(permissions, created_at DESC) WHERE deleted_at IS NULL;

-- Create updated_at trigger
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = CURRENT_TIMESTAMP;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER update_users_updated_at BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Comments
COMMENT ON TABLE users IS 'User accounts with soft delete support';
COMMENT ON COLUMN users.id IS '12-character nanoid';
COMMENT ON COLUMN users.username IS 'Unique username (NEVER reusable, even after deletion)';
COMMENT ON COLUMN users.email IS 'User email (NULL allowed for OAuth2 users, non-empty values are unique and never reusable)';
COMMENT ON COLUMN users.permissions IS '64-bit permission bitmask';
COMMENT ON COLUMN users.deleted_at IS 'Soft delete timestamp (NULL = active user)';
COMMENT ON CONSTRAINT users_email_not_empty ON users IS 'Ensures email is either NULL or a non-empty string';
