//! Provider gRPC Service Extensions
//!
//! Registry-based architecture for complete decoupling:
//! - Each provider module registers its own service builder
//! - No central code needs to know about specific provider types
//! - Adding new providers requires zero changes to common code

use crate::http::AppState;
use tonic::transport::server::Router;
use std::sync::{Arc, OnceLock};
use parking_lot::RwLock;

/// Type for service builder functions (takes AppState and Router, returns Router)
type ServiceBuilder = Box<dyn Fn(Arc<AppState>, Router) -> Router + Send + Sync>;

/// Global registry of gRPC service builders
static SERVICE_REGISTRY: OnceLock<RwLock<Vec<ServiceBuilder>>> = OnceLock::new();

/// Get or initialize the service registry
fn get_registry() -> &'static RwLock<Vec<ServiceBuilder>> {
    SERVICE_REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
}

/// Register a service builder for a provider
///
/// Each provider module calls this function to register its gRPC services.
/// The builder takes AppState and Router, returns a Router with services added.
///
/// # Example
/// ```rust,ignore
/// // In bilibili.rs module initialization
/// pub fn init() {
///     register_service_builder(|app_state, router| {
///         let service = BilibiliService::new(app_state);
///         router.add_service(BilibiliServer::new(service))
///     });
/// }
/// ```
pub fn register_service_builder<F>(builder: F)
where
    F: Fn(Arc<AppState>, Router) -> Router + Send + Sync + 'static,
{
    get_registry().write().push(Box::new(builder));
}

/// Build the complete gRPC services by calling all registered builders
///
/// No knowledge of specific provider types needed here!
pub fn build_provider_services(app_state: Arc<AppState>, mut router: Router) -> Router {
    let registry = get_registry().read();
    for builder in registry.iter() {
        router = builder(app_state.clone(), router);
    }
    router
}

// Provider-specific implementations
// Each module will self-register when loaded
pub mod bilibili;
pub mod alist;
pub mod emby;
