//! Configuration loading

use anyhow::Result;
use tracing::info;

use crate::Config;

/// Load configuration from environment variables
///
/// For now, this just calls `Config::from_env()`. In the future,
/// it could also load from config files.
pub fn load_config() -> Result<Config> {
    // Try to load from environment, with fallback to defaults
    let config = Config::from_env().unwrap_or_else(|e| {
        eprintln!("Failed to load config: {e}");
        eprintln!("Using default configuration");
        Config {
            server: crate::config::ServerConfig::default(),
            database: crate::config::DatabaseConfig::default(),
            redis: crate::config::RedisConfig::default(),
            jwt: crate::config::JwtConfig::default(),
            logging: crate::config::LoggingConfig::default(),
            streaming: crate::config::StreamingConfig::default(),
            oauth2: crate::config::OAuth2Config::default(),
            email: crate::config::EmailConfig::default(),
            media_providers: crate::config::MediaProvidersConfig::default(),
            webrtc: crate::config::WebRTCConfig::default(),
        }
    });

    // Validate configuration (fail fast on misconfigurations)
    if let Err(errors) = config.validate() {
        for error in &errors {
            tracing::error!("Config validation error: {}", error);
        }
        return Err(anyhow::anyhow!(
            "Configuration validation failed with {} error(s): {}",
            errors.len(),
            errors.join("; ")
        ));
    }

    info!("Configuration loaded and validated successfully");
    info!("gRPC address: {}", config.grpc_address());
    info!("HTTP address: {}", config.http_address());

    Ok(config)
}
