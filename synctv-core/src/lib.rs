pub mod models;
pub mod repository;
pub mod service;
pub mod cache;
pub mod provider;
pub mod config;
pub mod oauth2;
pub mod error;
pub mod logging;
pub mod bootstrap;
pub mod transaction;

pub use config::Config;
pub use error::{Error, Result};
pub use transaction::{UnitOfWork, with_transaction};

// Global server start time for uptime calculation
use once_cell::sync::Lazy;
use std::time::Instant;

pub static SERVER_START_TIME: Lazy<Instant> = Lazy::new(Instant::now);
