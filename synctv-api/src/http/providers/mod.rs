//! Provider HTTP Routes
//!
//! Provider-specific HTTP endpoints for parse, browse, proxy, etc.
//!
//! Each provider module exports a `{name}_routes()` function that returns
//! an Axum Router with all the provider's HTTP endpoints.

use axum::Router;
use super::AppState;

pub mod alist;
pub mod bilibili;
pub mod emby;

/// Register all provider HTTP routes
///
/// This function is called during HTTP server initialization to register
/// all provider-specific routes.
///
/// # Returns
/// Router with all provider routes nested under `/api/providers/{name}`
pub fn register_all_routes() -> Router<AppState> {
    Router::new()
        .nest("/bilibili", bilibili::bilibili_routes())
        .nest("/alist", alist::alist_routes())
        .nest("/emby", emby::emby_routes())
}
