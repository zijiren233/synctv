//! SyncTV Protocol Definitions
//!
//! This crate contains all protobuf definitions and generated code for SyncTV's
//! external APIs.

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
        include!("providers/synctv_provider_bilibili.rs");
    }

    pub mod alist {
        include!("providers/synctv_provider_alist.rs");
    }

    pub mod emby {
        include!("providers/synctv_provider_emby.rs");
    }
}
