-- Create audit_logs table (partitioned by month)
CREATE TABLE IF NOT EXISTS audit_logs (
    id BIGSERIAL,
    event_type VARCHAR(50) NOT NULL,
    user_id CHAR(12) NULL REFERENCES users(id) ON DELETE SET NULL,
    room_id CHAR(12) NULL REFERENCES rooms(id) ON DELETE SET NULL,
    ip_address INET NULL,
    user_agent TEXT NULL,
    details JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);

-- Create initial partitions (current month + next 3 months)
CREATE TABLE audit_logs_2024_01 PARTITION OF audit_logs
    FOR VALUES FROM ('2024-01-01') TO ('2024-02-01');
CREATE TABLE audit_logs_2024_02 PARTITION OF audit_logs
    FOR VALUES FROM ('2024-02-01') TO ('2024-03-01');
CREATE TABLE audit_logs_2024_03 PARTITION OF audit_logs
    FOR VALUES FROM ('2024-03-01') TO ('2024-04-01');
CREATE TABLE audit_logs_2024_04 PARTITION OF audit_logs
    FOR VALUES FROM ('2024-04-01') TO ('2024-05-01');

-- Create indexes on each partition
CREATE INDEX idx_audit_logs_2024_01_event_type ON audit_logs_2024_01(event_type);
CREATE INDEX idx_audit_logs_2024_01_user_id ON audit_logs_2024_01(user_id);
CREATE INDEX idx_audit_logs_2024_01_room_id ON audit_logs_2024_01(room_id);
CREATE INDEX idx_audit_logs_2024_01_created_at ON audit_logs_2024_01(created_at);

CREATE INDEX idx_audit_logs_2024_02_event_type ON audit_logs_2024_02(event_type);
CREATE INDEX idx_audit_logs_2024_02_user_id ON audit_logs_2024_02(user_id);
CREATE INDEX idx_audit_logs_2024_02_room_id ON audit_logs_2024_02(room_id);
CREATE INDEX idx_audit_logs_2024_02_created_at ON audit_logs_2024_02(created_at);

CREATE INDEX idx_audit_logs_2024_03_event_type ON audit_logs_2024_03(event_type);
CREATE INDEX idx_audit_logs_2024_03_user_id ON audit_logs_2024_03(user_id);
CREATE INDEX idx_audit_logs_2024_03_room_id ON audit_logs_2024_03(room_id);
CREATE INDEX idx_audit_logs_2024_03_created_at ON audit_logs_2024_03(created_at);

CREATE INDEX idx_audit_logs_2024_04_event_type ON audit_logs_2024_04(event_type);
CREATE INDEX idx_audit_logs_2024_04_user_id ON audit_logs_2024_04(user_id);
CREATE INDEX idx_audit_logs_2024_04_room_id ON audit_logs_2024_04(room_id);
CREATE INDEX idx_audit_logs_2024_04_created_at ON audit_logs_2024_04(created_at);

-- Comments
COMMENT ON TABLE audit_logs IS 'Security and operational audit log (partitioned by month)';
COMMENT ON COLUMN audit_logs.event_type IS 'Event type: login, logout, create_room, etc.';
COMMENT ON COLUMN audit_logs.details IS 'Event-specific details (JSON)';

-- Note: Add new partitions monthly via cron or application code
-- Example: CREATE TABLE audit_logs_2024_05 PARTITION OF audit_logs
--            FOR VALUES FROM ('2024-05-01') TO ('2024-06-01');
