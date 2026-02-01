//! Provider HTTP Route Extensions
//!
//! Registry-based architecture for complete decoupling:
//! - Each provider module registers its own route builder
//! - No central code needs to know about specific provider types
//! - Adding new providers requires zero changes to common code

use crate::http::AppState;
use axum::Router;
use parking_lot::RwLock;
use std::sync::OnceLock;

/// Type for route builder functions
type RouteBuilder = Box<dyn Fn() -> (String, Router<AppState>) + Send + Sync>;

/// Global registry of HTTP route builders
static ROUTE_REGISTRY: OnceLock<RwLock<Vec<RouteBuilder>>> = OnceLock::new();

/// Get or initialize the route registry
fn get_registry() -> &'static RwLock<Vec<RouteBuilder>> {
    ROUTE_REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
}

/// Register a route builder for a provider
///
/// Each provider module calls this function to register its routes.
/// The builder returns (prefix, router) where prefix is the URL path prefix.
///
/// # Example
/// ```rust,ignore
/// // In bilibili.rs module initialization
/// pub fn init() {
///     register_route_builder(|| {
///         ("bilibili".to_string(), bilibili_routes())
///     });
/// }
/// ```
pub fn register_route_builder<F>(builder: F)
where
    F: Fn() -> (String, Router<AppState>) + Send + Sync + 'static,
{
    get_registry().write().push(Box::new(builder));
}

/// Build the complete provider routes by calling all registered builders
///
/// No knowledge of specific provider types needed here!
pub fn build_provider_routes() -> Router<AppState> {
    let mut router = Router::new();

    let registry = get_registry().read();
    for builder in registry.iter() {
        let (prefix, sub_router) = builder();
        router = router.nest(&format!("/{}", prefix), sub_router);
        tracing::info!("Registered HTTP routes for provider: {}", prefix);
    }

    router
}

// Provider-specific implementations
// Each module will self-register when loaded
pub mod alist;
pub mod bilibili;
pub mod emby;
