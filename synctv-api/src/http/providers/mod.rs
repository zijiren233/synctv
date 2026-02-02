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
