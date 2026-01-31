use config::{Config as ConfigBuilder, ConfigError, Environment, File};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub redis: RedisConfig,
    pub jwt: JwtConfig,
    pub logging: LoggingConfig,
    pub streaming: StreamingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub grpc_port: u16,
    pub http_port: u16,
    pub enable_reflection: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            grpc_port: 50051,
            http_port: 8080,
            enable_reflection: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub connect_timeout_seconds: u64,
    pub idle_timeout_seconds: u64,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: "postgresql://synctv:synctv@localhost:5432/synctv".to_string(),
            max_connections: 20,
            min_connections: 5,
            connect_timeout_seconds: 10,
            idle_timeout_seconds: 600,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisConfig {
    pub url: String,
    pub pool_size: u32,
    pub connect_timeout_seconds: u64,
    pub key_prefix: String,
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            url: "redis://localhost:6379".to_string(),
            pool_size: 10,
            connect_timeout_seconds: 5,
            key_prefix: "synctv:".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtConfig {
    pub private_key_path: String,
    pub public_key_path: String,
    pub access_token_duration_hours: u64,
    pub refresh_token_duration_days: u64,
}

impl Default for JwtConfig {
    fn default() -> Self {
        Self {
            private_key_path: "./keys/jwt_private.pem".to_string(),
            public_key_path: "./keys/jwt_public.pem".to_string(),
            access_token_duration_hours: 1,
            refresh_token_duration_days: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String, // "json" or "pretty"
    pub file_path: Option<String>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: "pretty".to_string(),
            file_path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingConfig {
    pub rtmp_port: u16,
    pub hls_port: u16,
    pub max_streams: u32,
    pub gop_cache_size: u32, // In number of GOPs
    pub stream_timeout_seconds: u64,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            rtmp_port: 1935,
            hls_port: 8081,
            max_streams: 50,
            gop_cache_size: 2,
            stream_timeout_seconds: 300,
        }
    }
}

impl Config {
    /// Load configuration from multiple sources with priority:
    /// 1. Environment variables (highest priority)
    /// 2. Config file (if provided)
    /// 3. Defaults (lowest priority)
    pub fn load(config_file: Option<&str>) -> Result<Self, ConfigError> {
        let mut builder = ConfigBuilder::builder()
            // Start with defaults
            .set_default("server.host", ServerConfig::default().host)?
            .set_default("server.grpc_port", ServerConfig::default().grpc_port as i64)?
            .set_default("server.http_port", ServerConfig::default().http_port as i64)?
            .set_default("server.enable_reflection", ServerConfig::default().enable_reflection)?
            .set_default("database.url", DatabaseConfig::default().url)?
            .set_default("database.max_connections", DatabaseConfig::default().max_connections as i64)?
            .set_default("database.min_connections", DatabaseConfig::default().min_connections as i64)?
            .set_default("database.connect_timeout_seconds", DatabaseConfig::default().connect_timeout_seconds as i64)?
            .set_default("database.idle_timeout_seconds", DatabaseConfig::default().idle_timeout_seconds as i64)?
            .set_default("redis.url", RedisConfig::default().url)?
            .set_default("redis.pool_size", RedisConfig::default().pool_size as i64)?
            .set_default("redis.connect_timeout_seconds", RedisConfig::default().connect_timeout_seconds as i64)?
            .set_default("redis.key_prefix", RedisConfig::default().key_prefix)?
            .set_default("jwt.private_key_path", JwtConfig::default().private_key_path)?
            .set_default("jwt.public_key_path", JwtConfig::default().public_key_path)?
            .set_default("jwt.access_token_duration_hours", JwtConfig::default().access_token_duration_hours as i64)?
            .set_default("jwt.refresh_token_duration_days", JwtConfig::default().refresh_token_duration_days as i64)?
            .set_default("logging.level", LoggingConfig::default().level)?
            .set_default("logging.format", LoggingConfig::default().format)?
            .set_default("streaming.rtmp_port", StreamingConfig::default().rtmp_port as i64)?
            .set_default("streaming.hls_port", StreamingConfig::default().hls_port as i64)?
            .set_default("streaming.max_streams", StreamingConfig::default().max_streams as i64)?
            .set_default("streaming.gop_cache_size", StreamingConfig::default().gop_cache_size as i64)?
            .set_default("streaming.stream_timeout_seconds", StreamingConfig::default().stream_timeout_seconds as i64)?;

        // Load config file if provided
        if let Some(path) = config_file {
            if Path::new(path).exists() {
                builder = builder.add_source(File::with_name(path));
            }
        }

        // Override with environment variables (SYNCTV__SERVER__HOST, etc.)
        builder = builder.add_source(
            Environment::with_prefix("SYNCTV")
                .separator("__")
                .try_parsing(true),
        );

        let config = builder.build()?;
        config.try_deserialize()
    }

    /// Load from environment variables only (for Docker/K8s)
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::load(None)
    }

    /// Load from file path
    pub fn from_file(path: &str) -> Result<Self, ConfigError> {
        Self::load(Some(path))
    }

    /// Get database URL
    pub fn database_url(&self) -> &str {
        &self.database.url
    }

    /// Get Redis URL
    pub fn redis_url(&self) -> &str {
        &self.redis.url
    }

    /// Get gRPC address
    pub fn grpc_address(&self) -> String {
        format!("{}:{}", self.server.host, self.server.grpc_port)
    }

    /// Get HTTP address
    pub fn http_address(&self) -> String {
        format!("{}:{}", self.server.host, self.server.http_port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::from_env().unwrap_or_else(|_| Config {
            server: ServerConfig::default(),
            database: DatabaseConfig::default(),
            redis: RedisConfig::default(),
            jwt: JwtConfig::default(),
            logging: LoggingConfig::default(),
            streaming: StreamingConfig::default(),
        });

        assert!(!config.database_url().is_empty());
        assert!(!config.redis_url().is_empty());
        assert!(config.server.grpc_port > 0);
        assert!(config.server.http_port > 0);
    }

    #[test]
    fn test_grpc_address() {
        let config = Config {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                grpc_port: 50051,
                http_port: 8080,
                enable_reflection: true,
            },
            database: DatabaseConfig::default(),
            redis: RedisConfig::default(),
            jwt: JwtConfig::default(),
            logging: LoggingConfig::default(),
            streaming: StreamingConfig::default(),
        };

        assert_eq!(config.grpc_address(), "127.0.0.1:50051");
        assert_eq!(config.http_address(), "127.0.0.1:8080");
    }
}
