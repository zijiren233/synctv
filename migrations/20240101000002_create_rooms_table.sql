-- Create rooms table
CREATE TABLE IF NOT EXISTS rooms (
    id CHAR(12) PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    created_by CHAR(12) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status VARCHAR(20) NOT NULL DEFAULT 'active',
    settings JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    deleted_at TIMESTAMPTZ NULL
);

-- Create indexes
CREATE INDEX idx_rooms_created_by ON rooms(created_by);
CREATE INDEX idx_rooms_status ON rooms(status) WHERE deleted_at IS NULL;
CREATE INDEX idx_rooms_created_at ON rooms(created_at);
CREATE INDEX idx_rooms_deleted_at ON rooms(deleted_at) WHERE deleted_at IS NOT NULL;
CREATE INDEX idx_rooms_name ON rooms USING gin(to_tsvector('english', name));

-- Performance optimization indexes
CREATE INDEX idx_rooms_status_created_at ON rooms(status, created_at DESC) WHERE deleted_at IS NULL;
CREATE INDEX idx_rooms_creator_status ON rooms(created_by, status, created_at DESC) WHERE deleted_at IS NULL;
CREATE INDEX idx_rooms_name_lower ON rooms(LOWER(name)) WHERE deleted_at IS NULL;

-- Create updated_at trigger
CREATE TRIGGER update_rooms_updated_at BEFORE UPDATE ON rooms
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Add check constraint for status
ALTER TABLE rooms ADD CONSTRAINT rooms_status_check
    CHECK (status IN ('active', 'closed'));

-- Comments
COMMENT ON TABLE rooms IS 'Video watching rooms';
COMMENT ON COLUMN rooms.id IS '12-character nanoid';
COMMENT ON COLUMN rooms.settings IS 'Room settings (JSON): password, auto_play, max_members, etc.';
