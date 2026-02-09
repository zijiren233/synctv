-- Drop the create_root_playlist trigger and nanoid function
-- Root playlist creation is now handled in Rust code (RoomService::create_room)
-- This aligns with the codebase pattern where all ID generation and business logic is in Rust

-- Drop the trigger that auto-creates root playlist
DROP TRIGGER IF EXISTS create_room_root_playlist ON rooms;

-- Drop the trigger function
DROP FUNCTION IF EXISTS create_root_playlist();

-- Drop the nanoid function (no longer used anywhere)
DROP FUNCTION IF EXISTS nanoid(INT);

-- Comment explaining the change
COMMENT ON TABLE playlists IS 'Playlists/folders table. Root playlist (empty name, NULL parent_id) is created in Rust when room is created.';
