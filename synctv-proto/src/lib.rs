//! SyncTV Protocol Definitions
//!
//! This crate contains all protobuf definitions and generated code for SyncTV's
//! external APIs.

// Client API
pub mod client {
    #[allow(clippy::all)]
    #[allow(warnings)]
    include!("synctv_client.rs");
}

// Admin API
pub mod admin {
    #[allow(clippy::all)]
    #[allow(warnings)]
    include!("synctv_admin.rs");
}

// Providers
pub mod providers {
    pub mod bilibili {
        #[allow(clippy::all)]
        #[allow(warnings)]
        include!("providers/synctv_provider_bilibili.rs");
    }

    pub mod alist {
        #[allow(clippy::all)]
        #[allow(warnings)]
        include!("providers/synctv_provider_alist.rs");
    }

    pub mod emby {
        #[allow(clippy::all)]
        #[allow(warnings)]
        include!("providers/synctv_provider_emby.rs");
    }
}
