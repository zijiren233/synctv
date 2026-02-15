//! Cluster node discovery and health monitoring

pub mod node_registry;
pub mod health_monitor;
pub mod load_balancer;
pub mod k8s_dns;

pub use node_registry::{HeartbeatResult, NodeInfo, NodeRegistry};
pub use health_monitor::{HealthMonitor, NodeHealth};
pub use load_balancer::{LoadBalancer, LoadBalancingStrategy};
pub use k8s_dns::K8sDnsDiscovery;
