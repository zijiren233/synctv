// Stream relay module for multi-replica coordination
pub mod registry;
pub mod registry_trait;
pub mod publisher_manager;

pub use registry::{StreamRegistry, PublisherInfo};
pub use registry_trait::StreamRegistryTrait;

#[cfg(test)]
pub use registry_trait::MockStreamRegistry;

pub use publisher_manager::PublisherManager;
