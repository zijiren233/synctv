-- Create users table
CREATE TABLE IF NOT EXISTS users (
    id CHAR(12) PRIMARY KEY,
    username VARCHAR(50) UNIQUE NOT NULL,
    email VARCHAR(255) UNIQUE,  -- NULL allowed (e.g., OAuth2 users without email)
    password_hash VARCHAR(255) NOT NULL,
    signup_method VARCHAR(20),  -- NULL for legacy users, 'email' or 'oauth2' for new users
    role VARCHAR(20) NOT NULL DEFAULT 'user',
    status VARCHAR(20) NOT NULL DEFAULT 'active',
    email_verified BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    deleted_at TIMESTAMPTZ NULL,

    -- Ensure email is not empty or whitespace-only
    CONSTRAINT users_email_not_empty CHECK (email IS NULL OR length(trim(email)) > 0),
    -- Signup method constraint (NULL allowed for legacy users)
    CONSTRAINT users_signup_method_check CHECK (signup_method IS NULL OR signup_method IN ('email', 'oauth2')),
    -- Role constraint
    CONSTRAINT users_role_check CHECK (role IN ('root', 'admin', 'user')),
    -- Status constraint
    CONSTRAINT users_status_check CHECK (status IN ('active', 'pending', 'banned'))
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
CREATE INDEX idx_users_role ON users(role) WHERE deleted_at IS NULL;
CREATE INDEX idx_users_status ON users(status) WHERE deleted_at IS NULL;
CREATE INDEX idx_users_signup_method ON users(signup_method) WHERE deleted_at IS NULL;

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
COMMENT ON TABLE users IS 'User accounts with soft delete support (RBAC: role-based access control)';
COMMENT ON COLUMN users.id IS '12-character nanoid';
COMMENT ON COLUMN users.username IS 'Unique username (NEVER reusable, even after deletion)';
COMMENT ON COLUMN users.email IS 'User email (NULL allowed for OAuth2 users, non-empty values are unique and never reusable)';
COMMENT ON COLUMN users.signup_method IS 'Method used to register: email or oauth2';
COMMENT ON COLUMN users.role IS 'User RBAC role: root, admin, or user (global access level)';
COMMENT ON COLUMN users.status IS 'User account status: active, pending (email verification), or banned';
COMMENT ON COLUMN users.email_verified IS 'Whether the user email has been verified';
COMMENT ON COLUMN users.deleted_at IS 'Soft delete timestamp (NULL = active user)';
COMMENT ON CONSTRAINT users_email_not_empty ON users IS 'Ensures email is either NULL or a non-empty string';
