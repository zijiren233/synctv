-- Create media table (playlist items)
CREATE TABLE IF NOT EXISTS media (
    id CHAR(12) PRIMARY KEY,
    room_id CHAR(12) NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    url TEXT NOT NULL,
    provider VARCHAR(20) NOT NULL,
    title VARCHAR(500) NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}',
    position INTEGER NOT NULL,
    added_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    added_by CHAR(12) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    deleted_at TIMESTAMPTZ NULL
);

-- Create indexes
CREATE INDEX idx_media_room_id ON media(room_id, position) WHERE deleted_at IS NULL;
CREATE INDEX idx_media_added_by ON media(added_by);
CREATE INDEX idx_media_added_at ON media(added_at);
CREATE INDEX idx_media_deleted_at ON media(deleted_at) WHERE deleted_at IS NOT NULL;

-- Add check constraint for provider
ALTER TABLE media ADD CONSTRAINT media_provider_check
    CHECK (provider IN ('bilibili', 'alist', 'emby', 'directurl'));

-- Comments
COMMENT ON TABLE media IS 'Media items (videos/audio) in room playlists';
COMMENT ON COLUMN media.id IS '12-character nanoid';
COMMENT ON COLUMN media.provider IS 'Media provider: bilibili, alist, emby, directurl';
COMMENT ON COLUMN media.metadata IS 'Provider-specific metadata (JSON)';
COMMENT ON COLUMN media.position IS 'Position in playlist (0-indexed)';
