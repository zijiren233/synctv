-- Create chat_messages table
CREATE TABLE IF NOT EXISTS chat_messages (
    id CHAR(12) PRIMARY KEY,
    room_id CHAR(12) NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    user_id CHAR(12) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    content TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Create indexes
CREATE INDEX idx_chat_messages_user_id ON chat_messages(user_id);
CREATE INDEX idx_chat_messages_created_at ON chat_messages(created_at);

-- Performance optimization: covering index for chat pagination (includes user_id for JOIN avoidance)
CREATE INDEX idx_chat_messages_room_pagination ON chat_messages(room_id, created_at DESC, user_id);

-- Comments
COMMENT ON TABLE chat_messages IS 'Persistent chat messages (retention configurable per room)';
COMMENT ON COLUMN chat_messages.id IS '12-character nanoid';
COMMENT ON COLUMN chat_messages.content IS 'Message content (HTML sanitized)';
