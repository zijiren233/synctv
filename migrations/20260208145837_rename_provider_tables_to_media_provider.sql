-- Migration: Rename provider tables to media_provider for clarity
-- Purpose: Align table names with the media provider terminology used throughout the codebase

-- Rename provider_instances to media_provider_instances
ALTER TABLE provider_instances RENAME TO media_provider_instances;

-- Rename indexes for media_provider_instances
ALTER INDEX idx_provider_instances_enabled RENAME TO idx_media_provider_instances_enabled;
ALTER INDEX idx_provider_instances_providers RENAME TO idx_media_provider_instances_providers;
ALTER INDEX idx_provider_instances_endpoint RENAME TO idx_media_provider_instances_endpoint;

-- Rename trigger for media_provider_instances
DROP TRIGGER update_provider_instances_updated_at ON media_provider_instances;
CREATE TRIGGER update_media_provider_instances_updated_at
    BEFORE UPDATE ON media_provider_instances
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- Update table comment
COMMENT ON TABLE media_provider_instances IS 'Media provider gRPC instance configurations for cross-region deployment';

-- Rename user_provider_credentials to user_media_provider_credentials
ALTER TABLE user_provider_credentials RENAME TO user_media_provider_credentials;

-- Rename constraint
ALTER TABLE user_media_provider_credentials
    RENAME CONSTRAINT fk_provider_instance TO fk_media_provider_instance;

-- Rename indexes for user_media_provider_credentials
ALTER INDEX idx_user_credentials_user RENAME TO idx_user_media_provider_credentials_user;
ALTER INDEX idx_user_credentials_provider RENAME TO idx_user_media_provider_credentials_provider;
ALTER INDEX idx_user_credentials_instance RENAME TO idx_user_media_provider_credentials_instance;
ALTER INDEX idx_user_credentials_expires RENAME TO idx_user_media_provider_credentials_expires;

-- Rename constraint
ALTER TABLE user_media_provider_credentials
    RENAME CONSTRAINT unique_user_provider_server TO unique_user_media_provider_server;

-- Rename trigger for user_media_provider_credentials
DROP TRIGGER update_user_credentials_updated_at ON user_media_provider_credentials;
CREATE TRIGGER update_user_media_provider_credentials_updated_at
    BEFORE UPDATE ON user_media_provider_credentials
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- Update table comment
COMMENT ON TABLE user_media_provider_credentials IS 'User credentials for media providers';

-- Update foreign key constraint to reference the renamed table
ALTER TABLE user_media_provider_credentials
    DROP CONSTRAINT fk_media_provider_instance;

ALTER TABLE user_media_provider_credentials
    ADD CONSTRAINT fk_media_provider_instance
    FOREIGN KEY (provider_instance_name)
    REFERENCES media_provider_instances(name)
    ON DELETE SET NULL;
