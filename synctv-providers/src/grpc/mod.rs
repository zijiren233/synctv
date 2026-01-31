//! gRPC Provider Services
//!
//! This module contains gRPC server implementations for all providers.
//! The proto-generated code is included from the build directory.

// Include generated protobuf code
pub mod alist {
    tonic::include_proto!("api.alist");
}

pub mod bilibili {
    tonic::include_proto!("api.bilibili");
}

pub mod emby {
    tonic::include_proto!("api.emby");
}

// Server implementations
pub mod alist_server;
pub mod bilibili_server;
pub mod emby_server;
