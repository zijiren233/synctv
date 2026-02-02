//! Provider gRPC Services
//!
//! Provider-specific gRPC services for parse, browse, etc.
//!
//! Each provider module exports a `{name}_service()` function that returns
/// a tonic service wrapper.

use crate::http::AppState;
use std::sync::Arc;
use tonic::transport::server::Router;

pub mod bilibili;
pub mod alist;
pub mod emby;

/// Register all provider gRPC services
///
/// This function is called during gRPC server initialization to register
/// all provider-specific services.
///
/// # Arguments
/// - `app_state`: Application state for HTTP routes
/// - `router`: Base Tonic router to add services to
///
/// # Returns
/// Router with all provider services added
pub fn register_all_services(app_state: Arc<AppState>, router: Router) -> Router {
    let router = bilibili::register_service(app_state.clone(), router);
    let router = alist::register_service(app_state.clone(), router);
    let router = emby::register_service(app_state, router);
    router
}
