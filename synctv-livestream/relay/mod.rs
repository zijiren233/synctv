// Stream relay module for multi-replica coordination
pub mod registry;
pub mod registry_trait;
pub mod publisher_manager;
#[cfg(test)]
pub mod mock_registry;

pub use registry::{StreamRegistry, PublisherInfo, HEARTBEAT_INTERVAL_SECS, PUBLISHER_TTL_SECS};
pub use registry_trait::StreamRegistryTrait;

#[cfg(test)]
pub use mock_registry::MockStreamRegistry;

pub use publisher_manager::PublisherManager;
