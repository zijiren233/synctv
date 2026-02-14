//! Service initialization and dependency injection

use std::sync::Arc;

use sqlx::PgPool;
use tracing::{debug, error, info, warn};

use crate::{
    cache::UsernameCache,
    repository::{UserOAuthProviderRepository, ProviderInstanceRepository, UserProviderCredentialRepository, SettingsRepository, NotificationRepository},
    service::{
        ContentFilter, JwtService, OAuth2Service, RemoteProviderManager, RateLimitConfig,
        RateLimiter, TokenBlacklistService, UserService, RoomService, ProvidersManager,
        SettingsService, SettingsRegistry, EmailService, EmailTokenService, EmailConfig, PublishKeyService, UserNotificationService,
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
    pub provider_instance_manager: Arc<RemoteProviderManager>,
    /// Provider instances repository
    pub provider_instance_repo: Arc<ProviderInstanceRepository>,
    /// User provider credential repository
    pub user_provider_credential_repo: Arc<UserProviderCredentialRepository>,
    /// Providers manager
    pub providers_manager: Arc<ProvidersManager>,
    /// `OAuth2` service (optional, requires configuration)
    pub oauth2_service: Option<Arc<OAuth2Service>>,
    /// Settings service
    pub settings_service: Arc<SettingsService>,
    /// Settings registry with type-safe setting variables
    pub settings_registry: Arc<SettingsRegistry>,
    /// Email service (optional, requires SMTP configuration)
    pub email_service: Option<Arc<EmailService>>,
    /// Email token service for verification codes (optional, requires SMTP configuration)
    pub email_token_service: Option<Arc<EmailTokenService>>,
    /// Publish key service for RTMP streaming
    pub publish_key_service: Arc<PublishKeyService>,
    /// User notification service
    pub notification_service: Arc<UserNotificationService>,
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

    // Initialize shared Redis connection (used by token blacklist, rate limiter, and username cache)
    let redis_conn = if !config.redis.url.is_empty() {
        let client = redis::Client::open(config.redis.url.clone())?;
        Some(redis::aio::ConnectionManager::new(client).await?)
    } else {
        None
    };

    // Initialize token blacklist service
    let token_blacklist = TokenBlacklistService::new(redis_conn.clone());
    if token_blacklist.is_enabled() {
        info!("Token blacklist service initialized with Redis");
    } else {
        warn!("⚠ Token blacklist DISABLED (no Redis) — revoked tokens will remain valid until expiry");
    }

    // Initialize username cache
    let username_cache = UsernameCache::new(
        redis_conn.clone(),
        format!("{}username:", config.redis.key_prefix),
        1000, // Cache up to 1000 usernames in memory
        3600, // Cache for 1 hour in Redis
    );
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
    let rate_limiter = RateLimiter::new(redis_conn.clone(), config.redis.key_prefix.clone());
    let rate_limit_config = RateLimitConfig::default();
    if redis_conn.is_some() {
        info!(
            "Rate limiter initialized (chat: {}/s, danmaku: {}/s)",
            rate_limit_config.chat_per_second, rate_limit_config.danmaku_per_second
        );
    } else {
        warn!(
            "⚠ Rate limiting DISABLED (no Redis) — all rate limits are unenforced"
        );
    }

    // Initialize content filter
    let content_filter = ContentFilter::new();
    info!(
        "Content filter initialized (max chat: {} chars, max danmaku: {} chars)",
        content_filter.max_chat_length, content_filter.max_danmaku_length
    );

    // Initialize RemoteProviderManager
    info!("Initializing RemoteProviderManager...");
    let provider_instance_manager = Arc::new(RemoteProviderManager::new(provider_instance_repo.clone()));

    // Load all enabled provider instances from database
    if let Err(e) = provider_instance_manager.init().await {
        tracing::error!("Failed to initialize RemoteProviderManager: {}", e);
        tracing::error!("Continuing without remote provider instances");
    } else {
        info!("RemoteProviderManager initialized successfully");
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
    let settings_service = SettingsService::new(settings_repo, pool.clone());
    settings_service.initialize().await?;
    info!("Settings service initialized with {} groups", {
        settings_service.get_all().await.map_or(0, |g| g.len())
    });

    // Start PostgreSQL LISTEN for hot reload
    let _settings_listen_task = settings_service.start_listen_task();
    info!("Settings hot reload (PostgreSQL LISTEN) started");

    // Wrap settings_service in Arc before creating registry
    let settings_service = Arc::new(settings_service);

    // Initialize Settings registry
    info!("Initializing Settings registry...");
    let settings_registry = SettingsRegistry::new(settings_service.clone());
    settings_registry.init().await?;
    info!("Settings registry initialized");

    // Initialize Email service (optional - requires SMTP configuration)
    let email_service = init_email_service(config);
    if email_service.is_some() {
        info!("Email service initialized");
    } else {
        info!("Email service not configured (set SYNCTV_EMAIL_SMTP_HOST)");
    }

    // Initialize Email Token service (optional - requires email service)
    let email_token_service = if email_service.is_some() {
        Some(Arc::new(EmailTokenService::new(pool.clone())))
    } else {
        None
    };
    if email_token_service.is_some() {
        info!("Email token service initialized");
    } else {
        info!("Email token service not configured (requires email service)");
    }

    // Initialize Publish Key service (for RTMP streaming)
    let publish_key_service = PublishKeyService::with_default_ttl(jwt_service.clone());
    info!("Publish key service initialized");

    // Initialize User Notification service
    let notification_repo = NotificationRepository::new(pool.clone());
    let notification_service = UserNotificationService::new(notification_repo);
    info!("User notification service initialized");

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
        email_service,
        email_token_service,
        publish_key_service: Arc::new(publish_key_service),
        notification_service: Arc::new(notification_service),
    })
}

/// Initialize `OAuth2` service with modular provider system
///
/// Uses factory pattern to create providers from configuration.
/// `OAuth2` configuration is part of the main config file.
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
            .filter_map(|k| k.as_str().map(std::string::ToString::to_string))
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
            .ok_or_else(|| anyhow::anyhow!("Provider instance {instance_name} not found in config"))?;

        // Get provider type from config (check for explicit "type" field)
        let provider_type = if let Some(map) = full_config.as_mapping() {
            map.get(serde_yaml::Value::String("type".to_string()))
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

    // Spawn background task to clean up expired OAuth2 states
    // This prevents memory leaks from expired authorization flows
    let oauth2_service_clone = oauth2_service.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_hours(1)); // Run every hour
        loop {
            interval.tick().await;
            match oauth2_service_clone.cleanup_expired_states(7200).await {
                Ok(()) => {
                    debug!("OAuth2 state cleanup completed successfully");
                }
                Err(e) => {
                    error!("Failed to cleanup expired OAuth2 states: {}", e);
                }
            }
        }
    });

    Ok(Some(oauth2_service))
}


/// Load JWT service from secret in configuration
fn load_jwt_service(config: &Config) -> Result<JwtService, anyhow::Error> {
    if config.jwt.secret.is_empty() {
        return Err(anyhow::anyhow!(
            "JWT secret is empty. Please set SYNCTV__JWT__SECRET environment variable or configure jwt.secret in config file"
        ));
    }

    if config.jwt.secret == "change-me-in-production" {
        warn!("Using default JWT secret! This is insecure for production use.");
        warn!("Please set SYNCTV__JWT__SECRET to a strong random value.");
    }

    JwtService::with_durations(
        &config.jwt.secret,
        config.jwt.access_token_duration_hours,
        config.jwt.refresh_token_duration_days,
    )
    .map_err(|e| anyhow::anyhow!("Failed to initialize JWT service: {e}"))
}

/// Initialize Email service (optional - requires SMTP configuration)
fn init_email_service(config: &Config) -> Option<Arc<EmailService>> {
    // Check if SMTP host is configured
    if config.email.smtp_host.is_empty() {
        return None;
    }

    let email_config = EmailConfig {
        smtp_host: config.email.smtp_host.clone(),
        smtp_port: config.email.smtp_port,
        smtp_username: config.email.smtp_username.clone(),
        smtp_password: config.email.smtp_password.clone(),
        from_email: config.email.from_email.clone(),
        from_name: config.email.from_name.clone(),
        use_tls: config.email.use_tls,
    };

    match EmailService::new(Some(email_config)) {
        Ok(service) => Some(Arc::new(service)),
        Err(e) => {
            error!("Failed to initialize email service: {}", e);
            None
        }
    }
}
