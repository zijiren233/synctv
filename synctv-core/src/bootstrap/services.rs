//! Service initialization and dependency injection

use std::sync::Arc;

use sqlx::PgPool;
use tracing::{error, info, warn};

use crate::{
    cache::UsernameCache,
    repository::{UserOAuthProviderRepository, ProviderInstanceRepository, UserProviderCredentialRepository, SettingsRepository},
    service::{
        ContentFilter, JwtService, OAuth2Service, ProviderInstanceManager, RateLimitConfig,
        RateLimiter, TokenBlacklistService, UserService, RoomService, ProvidersManager,
        SettingsService, SettingsRegistry,
    },
    Config,
};

/// Container for all initialized services
#[derive(Clone)]
pub struct Services {
    /// User authentication and management service
    pub user_service: Arc<UserService>,
    /// Room management service
    pub room_service: Arc<RoomService>,
    /// JWT token service
    pub jwt_service: JwtService,
    /// Token blacklist (uses Redis)
    pub token_blacklist: TokenBlacklistService,
    /// Rate limiter (uses Redis)
    pub rate_limiter: RateLimiter,
    /// Rate limit configuration
    pub rate_limit_config: RateLimitConfig,
    /// Content filter for chat and danmaku
    pub content_filter: ContentFilter,
    /// Provider instance manager
    pub provider_instance_manager: Arc<ProviderInstanceManager>,
    /// Provider instances repository
    pub provider_instance_repo: Arc<ProviderInstanceRepository>,
    /// User provider credential repository
    pub user_provider_credential_repo: Arc<UserProviderCredentialRepository>,
    /// Providers manager
    pub providers_manager: Arc<ProvidersManager>,
    /// OAuth2 service (optional, requires configuration)
    pub oauth2_service: Option<Arc<OAuth2Service>>,
    /// Settings service
    pub settings_service: Arc<SettingsService>,
    /// Settings registry with type-safe setting variables
    pub settings_registry: Arc<SettingsRegistry>,
}

/// Initialize all core services
pub async fn init_services(
    pool: PgPool,
    config: &Config,
) -> Result<Services, anyhow::Error> {
    info!("Initializing services...");

    // Initialize JWT service
    info!("Loading JWT keys...");
    let jwt_service = load_jwt_service(config)?;
    info!("JWT service initialized");

    // Initialize token blacklist and rate limiter (both use Redis)
    let redis_url = if !config.redis.url.is_empty() {
        Some(config.redis.url.clone())
    } else {
        None
    };

    // Initialize token blacklist service
    let token_blacklist = TokenBlacklistService::new(redis_url.clone())?;
    if token_blacklist.is_enabled() {
        info!("Token blacklist service initialized with Redis");
    } else {
        info!("Token blacklist service disabled (Redis not configured)");
    }

    // Initialize username cache
    let username_cache = UsernameCache::new(
        redis_url.clone(),
        format!("{}username:", config.redis.key_prefix),
        1000, // Cache up to 1000 usernames in memory
        3600, // Cache for 1 hour in Redis
    )?;
    info!("Username cache initialized");

    // Initialize UserService
    let user_service = UserService::new(pool.clone(), jwt_service.clone(), token_blacklist.clone(), username_cache);
    info!("UserService initialized");

    // Initialize RoomService
    let room_service = RoomService::new(pool.clone(), user_service.clone());
    info!("RoomService initialized");

    // Initialize ProviderInstanceRepository
    let provider_instance_repo = Arc::new(ProviderInstanceRepository::new(pool.clone()));
    info!("ProviderInstanceRepository initialized");

    // Initialize UserProviderCredentialRepository
    let user_provider_credential_repo = Arc::new(UserProviderCredentialRepository::new(pool.clone()));
    info!("UserProviderCredentialRepository initialized");

    // Initialize rate limiter
    let rate_limiter = RateLimiter::new(redis_url.clone(), config.redis.key_prefix.clone())?;
    let rate_limit_config = RateLimitConfig::default();
    info!(
        "Rate limiter initialized (chat: {}/s, danmaku: {}/s)",
        rate_limit_config.chat_per_second, rate_limit_config.danmaku_per_second
    );

    // Initialize content filter
    let content_filter = ContentFilter::new();
    info!(
        "Content filter initialized (max chat: {} chars, max danmaku: {} chars)",
        content_filter.max_chat_length, content_filter.max_danmaku_length
    );

    // Initialize ProviderInstanceManager
    info!("Initializing ProviderInstanceManager...");
    let provider_instance_manager = Arc::new(ProviderInstanceManager::new(provider_instance_repo.clone()));

    // Load all enabled provider instances from database
    if let Err(e) = provider_instance_manager.init().await {
        tracing::error!("Failed to initialize ProviderInstanceManager: {}", e);
        tracing::error!("Continuing without remote provider instances");
    } else {
        info!("ProviderInstanceManager initialized successfully");
    }

    // Initialize ProvidersManager
    info!("Initializing ProvidersManager...");
    let providers_manager = Arc::new(ProvidersManager::new(
        provider_instance_manager.clone(),
    ));
    info!("ProvidersManager initialized");

    // Initialize OAuth2 service (optional - requires OAuth2_* env vars)
    let oauth2_service = init_oauth2_service(pool.clone(), config).await?;
    if oauth2_service.is_some() {
        info!("OAuth2 service initialized");
    } else {
        info!("OAuth2 service not configured (set SYNCTV__OAUTH2__ENCRYPTION_KEY)");
    }

    // Initialize Settings service
    info!("Initializing Settings service...");
    let settings_repo = SettingsRepository::new(pool.clone());
    let settings_service = SettingsService::new(settings_repo);
    settings_service.initialize().await?;
    info!("Settings service initialized with {} groups", {
        let groups = settings_service.get_all().await;
        if groups.is_ok() {
            groups.unwrap().len()
        } else {
            0
        }
    });

    // Wrap settings_service in Arc before creating registry
    let settings_service = Arc::new(settings_service);

    // Initialize Settings registry
    info!("Initializing Settings registry...");
    let settings_registry = SettingsRegistry::new(settings_service.clone());
    settings_registry.init().await?;
    info!("Settings registry initialized");

    Ok(Services {
        user_service: Arc::new(user_service),
        room_service: Arc::new(room_service),
        jwt_service,
        token_blacklist,
        rate_limiter,
        rate_limit_config,
        content_filter,
        provider_instance_manager,
        provider_instance_repo,
        user_provider_credential_repo,
        providers_manager,
        oauth2_service,
        settings_service,
        settings_registry: Arc::new(settings_registry),
    })
}

/// Initialize OAuth2 service with modular provider system
///
/// Uses factory pattern to create providers from configuration.
/// OAuth2 configuration is part of the main config file.
async fn init_oauth2_service(
    pool: PgPool,
    config: &Config,
) -> Result<Option<Arc<OAuth2Service>>, anyhow::Error> {
    // 0. Initialize provider registry (register all factory functions)
    crate::oauth2::providers::init_providers();
    info!("OAuth2 provider registry initialized");

    // 1. Get OAuth2 provider configurations from main config
    let providers_value = &config.oauth2.providers;

    // Extract provider instance names from the YAML mapping
    let provider_instances = if let Some(mapping) = providers_value.as_mapping() {
        mapping.keys()
            .filter_map(|k| k.as_str().map(|s| s.to_string()))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    if provider_instances.is_empty() {
        info!("No OAuth2 providers configured");
        return Ok(None);
    }

    // 2. Create OAuth2 provider repository and service
    let oauth2_repo = UserOAuthProviderRepository::new(pool.clone());
    let oauth2_service = OAuth2Service::new(oauth2_repo);
    let oauth2_service = Arc::new(oauth2_service);

    // 3. Initialize each provider instance using factory pattern
    for instance_name in provider_instances {
        // Get the full config for this instance
        let full_config = providers_value.get(&instance_name)
            .ok_or_else(|| anyhow::anyhow!("Provider instance {} not found in config", instance_name))?;

        // Get provider type from config (check for explicit "type" field)
        let provider_type = if let Some(map) = full_config.as_mapping() {
            map.get(&serde_yaml::Value::String("type".to_string()))
                .and_then(|v| v.as_str())
                .unwrap_or(&instance_name)
                .to_string()
        } else {
            instance_name.clone()
        };

        // Create a mutable config for adding redirect_url
        let mut full_config = full_config.clone();

        // Add redirect_url to config (merge it in)
        let redirect_url = format!("http://{}/api/oauth2/{}/callback", config.server.host, instance_name);
        if let Some(mapping) = full_config.as_mapping_mut() {
            mapping.insert(
                serde_yaml::Value::String("redirect_url".to_string()),
                serde_yaml::Value::String(redirect_url.clone())
            );
        }

        // Use factory to create provider with full config
        match crate::oauth2::create_provider(&provider_type, &full_config).await {
            Ok(provider) => {
                let provider_enum = match provider_type.as_str() {
                    "github" => crate::models::oauth2_client::OAuth2Provider::GitHub,
                    "google" => crate::models::oauth2_client::OAuth2Provider::Google,
                    "logto" => crate::models::oauth2_client::OAuth2Provider::Logto,
                    "oidc" => crate::models::oauth2_client::OAuth2Provider::Oidc,
                    _ => continue,
                };

                // Store provider for later use
                oauth2_service.register_provider(instance_name.clone(), provider_enum, provider).await;
                info!("Registered OAuth2 provider: {} (type: {})", instance_name, provider_type);
            }
            Err(e) => {
                warn!("Failed to create OAuth2 provider {}: {}", instance_name, e);
            }
        }
    }

    Ok(Some(oauth2_service))
}


/// Load JWT service from key files or generate keys for development
fn load_jwt_service(config: &Config) -> Result<JwtService, anyhow::Error> {
    // Try to load keys from files
    let private_key = std::fs::read(&config.jwt.private_key_path);
    let public_key = std::fs::read(&config.jwt.public_key_path);

    match (private_key, public_key) {
        (Ok(priv_key), Ok(pub_key)) => {
            info!("Loaded JWT keys from files");
            JwtService::new(&priv_key, &pub_key)
                .map_err(|e| anyhow::anyhow!("Failed to initialize JWT service: {}", e))
        }
        _ => {
            // In development, generate temporary keys
            error!("JWT key files not found. Generating temporary keys for development.");
            error!("WARNING: These keys will not persist across restarts!");
            error!("For production, generate keys with: openssl genrsa -out jwt_private.pem 2048");
            error!("                                  openssl rsa -in jwt_private.pem -pubout -out jwt_public.pem");

            // For now, return error - keys must be provided
            Err(anyhow::anyhow!(
                "JWT keys not found at {} and {}. Please generate keys with:\n  openssl genrsa -out {} 2048\n  openssl rsa -in {} -pubout -out {}",
                config.jwt.private_key_path,
                config.jwt.public_key_path,
                config.jwt.private_key_path,
                config.jwt.private_key_path,
                config.jwt.public_key_path
            ))
        }
    }
}
