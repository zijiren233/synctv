-- Migration: Provider Instances
-- Purpose: Store gRPC provider instance configurations for cross-region deployment

CREATE TABLE provider_instances (
    -- Primary Key
    name VARCHAR(64) PRIMARY KEY,

    -- Basic Information
    endpoint VARCHAR(512) NOT NULL,
    comment TEXT,

    -- gRPC Configuration
    jwt_secret VARCHAR(256),        -- JWT secret (encrypted storage)
    custom_ca TEXT,                 -- Custom CA certificate (encrypted storage)
    timeout VARCHAR(32) NOT NULL DEFAULT '10s',
    tls BOOLEAN NOT NULL DEFAULT false,
    insecure_tls BOOLEAN NOT NULL DEFAULT false,  -- Skip TLS verification (unsafe, dev/test only)

    -- Provider Support (which providers can use this instance)
    providers TEXT[] NOT NULL DEFAULT '{}',  -- e.g., {'bilibili', 'alist', 'emby'}

    -- Status
    enabled BOOLEAN NOT NULL DEFAULT true,

    -- Audit
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Constraints
    CONSTRAINT valid_name CHECK (length(trim(name)) > 0 AND length(name) <= 64),
    CONSTRAINT valid_endpoint CHECK (length(trim(endpoint)) > 0)
);

-- Indexes
CREATE INDEX idx_provider_instances_enabled ON provider_instances(enabled);
CREATE INDEX idx_provider_instances_providers ON provider_instances USING gin(providers);
CREATE INDEX idx_provider_instances_endpoint ON provider_instances(endpoint);

-- Updated At Trigger
CREATE TRIGGER update_provider_instances_updated_at
    BEFORE UPDATE ON provider_instances
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- Comments
COMMENT ON TABLE provider_instances IS 'gRPC provider instance configurations for cross-region deployment';
COMMENT ON COLUMN provider_instances.name IS 'Instance name (unique identifier)';
COMMENT ON COLUMN provider_instances.endpoint IS 'gRPC service endpoint (e.g., grpc://beijing.example.com:50051)';
COMMENT ON COLUMN provider_instances.jwt_secret IS 'JWT secret for authentication (encrypted storage)';
COMMENT ON COLUMN provider_instances.custom_ca IS 'Custom CA certificate (encrypted storage)';
COMMENT ON COLUMN provider_instances.timeout IS 'Request timeout (e.g., 10s, 30s)';
COMMENT ON COLUMN provider_instances.tls IS 'Enable TLS';
COMMENT ON COLUMN provider_instances.insecure_tls IS 'Skip TLS certificate verification (unsafe, dev/test only)';
COMMENT ON COLUMN provider_instances.providers IS 'Supported provider types (array), e.g., {bilibili, alist, emby}';
COMMENT ON CONSTRAINT valid_name ON provider_instances IS 'Name must not be empty or whitespace';
COMMENT ON CONSTRAINT valid_endpoint ON provider_instances IS 'Endpoint must not be empty or whitespace';
