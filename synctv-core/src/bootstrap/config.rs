//! Configuration loading

use anyhow::Result;
use tracing::info;

use crate::Config;

/// Load configuration from config file or environment variables
///
/// Tries to load from config.yaml first, then falls back to environment variables
pub fn load_config() -> Result<Config> {
    // Try to load from config file first
    let config = if std::path::Path::new("config.yaml").exists() {
        eprintln!("Loading config from config.yaml");
        match Config::from_file("config.yaml") {
            Ok(cfg) => {
                eprintln!("Successfully loaded config.yaml");
                eprintln!("JWT secret length: {}", cfg.jwt.secret.len());
                cfg
            }
            Err(e) => {
                eprintln!("Failed to load config.yaml: {e}");
                eprintln!("Falling back to environment variables");
                Config::from_env().unwrap_or_default()
            }
        }
    } else {
        eprintln!("config.yaml not found, using environment variables");
        // Fall back to environment variables
        Config::from_env().unwrap_or_else(|e| {
            eprintln!("Failed to load config: {e}");
            eprintln!("Using default configuration");
            Config::default()
        })
    };

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
