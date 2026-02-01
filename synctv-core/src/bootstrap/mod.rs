//! Bootstrap module for initializing the SyncTV server
//!
//! This module handles:
//! - Database initialization
//! - Configuration loading
//! - Service initialization and dependency injection

pub mod database;
pub mod config;
pub mod services;

pub use database::init_database;
pub use config::load_config;
pub use services::init_services;
