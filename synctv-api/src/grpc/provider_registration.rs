/// Provider Service Registration
///
/// This module handles dynamic registration of provider-specific gRPC services.
/// Each provider (Bilibili, Alist, Emby) can expose client-facing APIs like:
/// - Parse: Convert user input (URL, request) to source_config options
/// - Browse: List files/folders in provider's storage
/// - Search: Search for media content
///
/// Architecture:
/// - synctv-core: MediaProvider trait with needs_service_registration() flag
/// - synctv-providers: gRPC service implementations (BilibiliServer, AlistServer, etc.)
/// - synctv-api: This module registers services from providers into the main gRPC server

use std::sync::Arc;
use tonic::transport::server::Router;
use synctv_core::provider::MediaProvider;

/// Register all provider services that need client-facing APIs
///
/// # Parameters
/// - `router`: The main gRPC router
/// - `providers`: List of providers that need service registration
///
/// # Returns
/// Modified router with all provider services registered
///
/// # Example
/// ```rust,ignore
/// let providers = provider_registry.get_providers_needing_registration();
/// let router = register_provider_services(router, providers).await?;
/// ```
pub async fn register_provider_services(
    mut router: Router,
    providers: Vec<Arc<dyn MediaProvider>>,
) -> anyhow::Result<Router> {
    for provider in providers {
        router = match provider.name() {
            "bilibili" => register_bilibili_service(router, provider).await?,
            "alist" => register_alist_service(router, provider).await?,
            "emby" => register_emby_service(router, provider).await?,
            _ => {
                tracing::warn!(
                    provider = provider.name(),
                    "Provider needs service registration but no handler found"
                );
                router
            }
        };
    }
    Ok(router)
}

/// Register Bilibili gRPC service
async fn register_bilibili_service(
    router: Router,
    _provider: Arc<dyn MediaProvider>,
) -> anyhow::Result<Router> {
    use synctv_providers::grpc::bilibili::bilibili_server::BilibiliServer;
    use synctv_providers::grpc::BilibiliService;

    tracing::info!("Registering Bilibili gRPC service");

    let service = BilibiliService::new();
    Ok(router.add_service(BilibiliServer::new(service)))
}

/// Register Alist gRPC service
async fn register_alist_service(
    router: Router,
    _provider: Arc<dyn MediaProvider>,
) -> anyhow::Result<Router> {
    use synctv_providers::grpc::alist::alist_server::AlistServer;
    use synctv_providers::grpc::AlistService;

    tracing::info!("Registering Alist gRPC service");

    let service = AlistService::new();
    Ok(router.add_service(AlistServer::new(service)))
}

/// Register Emby gRPC service
async fn register_emby_service(
    router: Router,
    _provider: Arc<dyn MediaProvider>,
) -> anyhow::Result<Router> {
    use synctv_providers::grpc::emby::emby_server::EmbyServer;
    use synctv_providers::grpc::EmbyService;

    tracing::info!("Registering Emby gRPC service");

    let service = EmbyService::new();
    Ok(router.add_service(EmbyServer::new(service)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_empty_providers() {
        let router = tonic::transport::Server::builder();
        let result = register_provider_services(router, vec![]).await;
        assert!(result.is_ok());
    }
}
