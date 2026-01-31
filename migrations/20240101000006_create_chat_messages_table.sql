-- Create chat_messages table
CREATE TABLE IF NOT EXISTS chat_messages (
    id CHAR(12) PRIMARY KEY,
    room_id CHAR(12) NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    user_id CHAR(12) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    content TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    deleted_at TIMESTAMPTZ NULL
);

-- Create indexes
CREATE INDEX idx_chat_messages_room_id ON chat_messages(room_id, created_at DESC)
    WHERE deleted_at IS NULL;
CREATE INDEX idx_chat_messages_user_id ON chat_messages(user_id);
CREATE INDEX idx_chat_messages_created_at ON chat_messages(created_at);
CREATE INDEX idx_chat_messages_deleted_at ON chat_messages(deleted_at)
    WHERE deleted_at IS NOT NULL;

-- Comments
COMMENT ON TABLE chat_messages IS 'Persistent chat messages (last 500 per room kept)';
COMMENT ON COLUMN chat_messages.id IS '12-character nanoid';
COMMENT ON COLUMN chat_messages.content IS 'Message content (HTML sanitized)';
