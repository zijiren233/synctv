//! Provider API Implementations
//!
//! Unified implementation for all provider API operations.
//! Used by both HTTP and gRPC handlers.

pub mod alist;
pub mod bilibili;
pub mod emby;

pub use alist::AlistApiImpl;
pub use bilibili::BilibiliApiImpl;
pub use emby::EmbyApiImpl;
