//! Cluster gRPC communication

pub mod server;

// Include generated protobuf code
pub mod synctv {
    pub mod cluster {
        include!("proto/synctv.cluster.rs");
    }
}

pub use server::ClusterServer;
pub use synctv::cluster::cluster_service_server::ClusterServiceServer;
