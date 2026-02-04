use config::{Config as ConfigBuilder, ConfigError, Environment, File};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Application configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub redis: RedisConfig,
    pub jwt: JwtConfig,
    pub logging: LoggingConfig,
    pub streaming: StreamingConfig,
    pub oauth2: OAuth2Config,
    pub email: EmailConfig,
    pub media_providers: MediaProvidersConfig,
    pub webrtc: WebRTCConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
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

/// `OAuth2` configuration
///
/// Stores `OAuth2` provider configurations in the main config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Config {
    /// Provider configurations (e.g., github, google, logto1, logto2)
    #[serde(default)]
    pub providers: serde_yaml::Value,
}

impl Default for OAuth2Config {
    fn default() -> Self {
        Self {
            providers: serde_yaml::Value::Mapping(serde_yaml::mapping::Mapping::new()),
        }
    }
}

/// Media providers configuration
///
/// Stores media provider configurations (Alist, Emby, Bilibili, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaProvidersConfig {
    /// Provider configurations (e.g., alist, emby, jellyfin, bilibili)
    #[serde(default)]
    pub providers: serde_json::Value,
}

impl Default for MediaProvidersConfig {
    fn default() -> Self {
        Self {
            providers: serde_json::json!({}),
        }
    }
}

/// WebRTC configuration for audio/video calls
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebRTCConfig {
    /// WebRTC operation mode
    pub mode: WebRTCMode,

    // STUN Configuration
    /// Enable built-in STUN server
    pub enable_builtin_stun: bool,
    /// Built-in STUN server port
    pub builtin_stun_port: u16,
    /// Built-in STUN server host
    pub builtin_stun_host: String,
    /// External STUN server URLs (fallback/backup)
    pub external_stun_servers: Vec<String>,

    // TURN Configuration (optional, for NAT traversal)
    /// TURN mode: "builtin", "external", or "disabled"
    pub turn_mode: TurnMode,

    // Built-in TURN server configuration
    /// Enable built-in TURN server
    pub enable_builtin_turn: bool,
    /// Built-in TURN server port (same as STUN by default)
    pub builtin_turn_port: u16,
    /// Built-in TURN relay port range (min)
    pub builtin_turn_min_port: u16,
    /// Built-in TURN relay port range (max)
    pub builtin_turn_max_port: u16,
    /// Maximum concurrent TURN allocations (limit resource usage)
    pub builtin_turn_max_allocations: usize,

    // External TURN server configuration
    /// External TURN server URL (e.g., "turn:turn.example.com:3478")
    pub external_turn_server_url: Option<String>,
    /// External TURN static secret for generating temporary credentials
    /// Must match coturn's `static-auth-secret` configuration
    pub external_turn_static_secret: Option<String>,
    /// TURN credential TTL in seconds (default 24 hours)
    pub turn_credential_ttl: u64,

    // SFU Configuration (for large rooms)
    /// Room size threshold to switch to SFU mode (only for Hybrid mode)
    pub sfu_threshold: usize,
    /// Enable Simulcast (multiple quality layers)
    pub enable_simulcast: bool,
    /// Maximum concurrent SFU rooms (0 = unlimited)
    pub max_sfu_rooms: usize,
    /// Maximum peers per SFU room
    pub max_peers_per_sfu_room: usize,
}

/// TURN server mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnMode {
    /// Use built-in TURN server (simple deployment, limited scale)
    Builtin,
    /// Use external TURN server (production, high scale)
    External,
    /// Disable TURN (P2P + STUN only, ~85-90% success rate)
    Disabled,
}

impl Default for TurnMode {
    fn default() -> Self {
        Self::Builtin // Default to built-in for ease of use
    }
}

/// WebRTC operation mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebRTCMode {
    /// Pure P2P mode (zero server cost)
    /// - Signaling only, no STUN/TURN/SFU
    /// - Best for: personal deployments
    /// - Connection success rate: ~70-75%
    SignalingOnly,

    /// P2P with STUN/TURN support (recommended for most deployments)
    /// - P2P connections with NAT traversal
    /// - STUN for reflexive candidates
    /// - TURN fallback for difficult NAT scenarios
    /// - Best for: small to medium deployments
    /// - Connection success rate: ~99%
    PeerToPeer,

    /// Hybrid mode (P2P for small rooms, SFU for large rooms)
    /// - Automatically switches based on room size
    /// - P2P for rooms < threshold
    /// - SFU for rooms >= threshold
    /// - Best for: flexible deployments with mixed room sizes
    /// - Optimal balance of cost and performance
    Hybrid,

    /// Pure SFU mode (enterprise grade)
    /// - All rooms use SFU regardless of size
    /// - Server receives and forwards all media streams
    /// - Best for: large scale deployments, recording, monitoring
    /// - Highest server cost, best quality and reliability
    #[serde(rename = "sfu")]
    SFU,
}

impl Default for WebRTCConfig {
    fn default() -> Self {
        Self {
            // Default to Hybrid mode (balanced)
            mode: WebRTCMode::Hybrid,

            // STUN enabled by default
            enable_builtin_stun: true,
            builtin_stun_port: 3478,
            builtin_stun_host: "0.0.0.0".to_string(),
            external_stun_servers: vec![
                "stun:stun.l.google.com:19302".to_string(),
                "stun:stun1.l.google.com:19302".to_string(),
            ],

            // TURN mode (default to built-in for ease of use)
            turn_mode: TurnMode::Builtin,

            // Built-in TURN configuration
            enable_builtin_turn: false, // Disabled by default (higher resource usage)
            builtin_turn_port: 3478, // Same as STUN by default
            builtin_turn_min_port: 49152,
            builtin_turn_max_port: 65535,
            builtin_turn_max_allocations: 100,

            // External TURN configuration
            external_turn_server_url: None,
            external_turn_static_secret: None,
            turn_credential_ttl: 86400, // 24 hours

            // SFU configuration
            sfu_threshold: 5, // Switch to SFU for 5+ participants
            enable_simulcast: true,
            max_sfu_rooms: 0, // No limit by default
            max_peers_per_sfu_room: 50,
        }
    }
}

/// Email configuration for SMTP
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailConfig {
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_username: String,
    pub smtp_password: String,
    pub from_email: String,
    pub from_name: String,
    pub use_tls: bool,
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            smtp_host: String::new(),
            smtp_port: 587,
            smtp_username: String::new(),
            smtp_password: String::new(),
            from_email: String::new(),
            from_name: "SyncTV".to_string(),
            use_tls: true,
        }
    }
}

impl Config {
    /// Load configuration from multiple sources with priority:
    /// 1. Environment variables (highest priority)
    /// 2. Config file (if provided)
    /// 3. Defaults (lowest priority)
    pub fn load(config_file: Option<&str>) -> Result<Self, ConfigError> {
        let mut builder = ConfigBuilder::builder();

        // Load config file if provided
        if let Some(path) = config_file {
            if Path::new(path).exists() {
                builder = builder.add_source(File::with_name(path));
            }
        }

        // Override with environment variables (SYNCTV_SERVER_HOST, etc.)
        builder = builder.add_source(
            Environment::with_prefix("SYNCTV")
                .separator("_")
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
    #[must_use] 
    pub fn database_url(&self) -> &str {
        &self.database.url
    }

    /// Get Redis URL
    #[must_use] 
    pub fn redis_url(&self) -> &str {
        &self.redis.url
    }

    /// Get gRPC address
    #[must_use] 
    pub fn grpc_address(&self) -> String {
        format!("{}:{}", self.server.host, self.server.grpc_port)
    }

    /// Get HTTP address
    #[must_use] 
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
            oauth2: OAuth2Config::default(),
            email: EmailConfig::default(),
            media_providers: MediaProvidersConfig::default(),
            webrtc: WebRTCConfig::default(),
        });

        assert!(!config.database_url().is_empty());
        assert!(!config.redis_url().is_empty());
        assert!(config.server.grpc_port > 0);
        assert!(config.server.http_port > 0);
        assert!(config.webrtc.enable_builtin_stun);
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
            oauth2: OAuth2Config::default(),
            email: EmailConfig::default(),
            media_providers: MediaProvidersConfig::default(),
            webrtc: WebRTCConfig::default(),
        };

        assert_eq!(config.grpc_address(), "127.0.0.1:50051");
        assert_eq!(config.http_address(), "127.0.0.1:8080");
    }
}
