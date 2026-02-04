-- Create room_members table with Allow/Deny permission pattern
CREATE TABLE IF NOT EXISTS room_members (
    room_id CHAR(12) NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    user_id CHAR(12) NOT NULL REFERENCES users(id) ON DELETE CASCADE,

    -- Role and Status (separated as per design)
    role SMALLINT NOT NULL DEFAULT 3,  -- 1=creator, 2=admin, 3=member, 4=guest
    status SMALLINT NOT NULL DEFAULT 1,  -- 1=active, 2=pending, 3=banned

    -- Allow/Deny permission pattern
    -- effective_permissions = ((global_default | room_added | admin_added | member_added) & ~(room_removed | admin_removed | member_removed))
    -- For regular members: uses member_added/member_removed
    -- For admins: uses admin_added/admin_removed (overrides member-level)
    added_permissions BIGINT UNSIGNED DEFAULT 0,      -- For member role: extra permissions
    removed_permissions BIGINT UNSIGNED DEFAULT 0,    -- For member role: removed permissions
    admin_added_permissions BIGINT UNSIGNED DEFAULT 0,     -- For admin role: extra permissions (on top of admin default)
    admin_removed_permissions BIGINT UNSIGNED DEFAULT 0,   -- For admin role: removed permissions (overrides admin default)

    -- Timestamps
    joined_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    left_at TIMESTAMPTZ NULL,

    -- Optimistic locking for permission updates
    version BIGINT NOT NULL DEFAULT 0,

    -- Banned info
    banned_at TIMESTAMPTZ,
    banned_by CHAR(12) REFERENCES users(id),
    banned_reason TEXT,

    PRIMARY KEY (room_id, user_id)
);

-- Create indexes
CREATE INDEX idx_room_members_user_id ON room_members(user_id);
CREATE INDEX idx_room_members_joined_at ON room_members(joined_at);
CREATE INDEX idx_room_members_active ON room_members(room_id, user_id)
    WHERE left_at IS NULL;

-- Performance optimization indexes (covering indexes to avoid table lookups)
CREATE INDEX idx_room_members_user_active ON room_members(user_id, room_id, role, joined_at DESC)
    WHERE left_at IS NULL;
CREATE INDEX idx_room_members_room_count ON room_members(room_id)
    WHERE left_at IS NULL;

-- Permission-related indexes
CREATE INDEX idx_room_members_role ON room_members(room_id, role)
    WHERE left_at IS NULL;
CREATE INDEX idx_room_members_status ON room_members(room_id, status)
    WHERE left_at IS NULL;
CREATE INDEX idx_room_members_banned ON room_members(banned_at)
    WHERE banned_at IS NOT NULL;
CREATE INDEX idx_room_members_version ON room_members(room_id, user_id, version)
    WHERE left_at IS NULL;

-- Constraints: 1=creator, 2=admin, 3=member, 4=guest
ALTER TABLE room_members
    ADD CONSTRAINT check_room_members_role
    CHECK (role BETWEEN 1 AND 4);

-- Status constraint: 1=active, 2=pending, 3=banned
ALTER TABLE room_members
    ADD CONSTRAINT check_room_members_status
    CHECK (status BETWEEN 1 AND 3);

-- Comments
COMMENT ON TABLE room_members IS 'Room membership with Allow/Deny permission pattern';
COMMENT ON COLUMN room_members.role IS 'Room role: 1=creator, 2=admin, 3=member, 4=guest';
COMMENT ON COLUMN room_members.status IS 'Member status: 1=active, 2=pending, 3=banned';
COMMENT ON COLUMN room_members.added_permissions IS 'Extra permissions added to role default (Allow pattern)';
COMMENT ON COLUMN room_members.removed_permissions IS 'Permissions removed from role default (Deny pattern)';
COMMENT ON COLUMN room_members.version IS 'Optimistic lock version for permission updates';
COMMENT ON COLUMN room_members.banned_at IS 'Timestamp when member was banned';
COMMENT ON COLUMN room_members.banned_by IS 'User ID who banned this member';
COMMENT ON COLUMN room_members.banned_reason IS 'Reason for banning';
COMMENT ON COLUMN room_members.left_at IS 'NULL if currently in room, timestamp if left';
