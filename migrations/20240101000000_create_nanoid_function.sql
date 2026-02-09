-- Enable pgcrypto extension for random generation
CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- Create nanoid function for PostgreSQL
-- This generates URL-safe random strings similar to the Rust nanoid library
CREATE OR REPLACE FUNCTION nanoid(size INT DEFAULT 21)
RETURNS TEXT AS $$
DECLARE
    -- URL-safe alphabet (same as nanoid default)
    alphabet TEXT := '_-0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ';
    id TEXT := '';
    i INT := 0;
    pos INT;
BEGIN
    -- Generate random characters
    WHILE i < size LOOP
        -- Get random byte and convert to alphabet index
        pos := (get_byte(gen_random_bytes(1), 0) % 64) + 1;
        id := id || substr(alphabet, pos, 1);
        i := i + 1;
    END LOOP;

    RETURN id;
END;
$$ LANGUAGE plpgsql VOLATILE;

COMMENT ON FUNCTION nanoid(INT) IS 'Generate URL-safe random ID (nanoid compatible)';
