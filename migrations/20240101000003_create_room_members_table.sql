-- Create room_members table
CREATE TABLE IF NOT EXISTS room_members (
    room_id CHAR(12) NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    user_id CHAR(12) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    permissions BIGINT NOT NULL DEFAULT 0,
    joined_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    left_at TIMESTAMPTZ NULL,
    PRIMARY KEY (room_id, user_id)
);

-- Create indexes
CREATE INDEX idx_room_members_user_id ON room_members(user_id);
CREATE INDEX idx_room_members_joined_at ON room_members(joined_at);
CREATE INDEX idx_room_members_active ON room_members(room_id, user_id)
    WHERE left_at IS NULL;

-- Performance optimization indexes (covering indexes to avoid table lookups)
CREATE INDEX idx_room_members_user_active ON room_members(user_id, room_id, permissions, joined_at DESC)
    WHERE left_at IS NULL;
CREATE INDEX idx_room_members_room_count ON room_members(room_id)
    WHERE left_at IS NULL;

-- Comments
COMMENT ON TABLE room_members IS 'Room membership and permissions';
COMMENT ON COLUMN room_members.permissions IS '64-bit permission bitmask for this room';
COMMENT ON COLUMN room_members.left_at IS 'NULL if currently in room, timestamp if left';
