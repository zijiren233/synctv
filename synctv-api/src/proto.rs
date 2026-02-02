// Re-export proto types from synctv-proto
pub use synctv_proto::{client, admin, providers};

// Re-export server traits for gRPC service implementations
pub use client::{
    auth_service_server,
    email_service_server,
    media_service_server,
    public_service_server,
    room_service_server,
    user_service_server,
};
pub use admin::admin_service_server;

// Re-export provider server traits
pub use providers::{
    bilibili::bilibili_provider_service_server,
    alist::alist_provider_service_server,
    emby::emby_provider_service_server,
};
