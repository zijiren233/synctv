//! `SyncTV` Protocol Definitions
//!
//! This crate contains all protobuf definitions and generated code for `SyncTV`'s
//! external APIs.

// Common shared types (enums, RoomMember)
pub mod common {
    include!("synctv.common.rs");
}

// Client API
pub mod client {
    include!("synctv.client.rs");
}

// Admin API
pub mod admin {
    include!("synctv.admin.rs");
}

// Providers
pub mod providers {
    pub mod bilibili {
        include!("providers/synctv.provider.bilibili.rs");
    }

    pub mod alist {
        include!("providers/synctv.provider.alist.rs");
    }

    pub mod emby {
        include!("providers/synctv.provider.emby.rs");
    }
}
