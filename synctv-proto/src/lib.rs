//! SyncTV Protocol Definitions
//!
//! This crate contains all protobuf definitions and generated code for SyncTV's
//! external APIs.

// Client API
pub mod client {
    #[allow(clippy::all)]
    #[allow(warnings)]
    include!("synctv.client.rs");
}

// Admin API
pub mod admin {
    #[allow(clippy::all)]
    #[allow(warnings)]
    include!("synctv.admin.rs");
}

// Providers
pub mod providers {
    pub mod bilibili {
        #[allow(clippy::all)]
        #[allow(warnings)]
        include!("providers/synctv.provider.bilibili.rs");
    }

    pub mod alist {
        #[allow(clippy::all)]
        #[allow(warnings)]
        include!("providers/synctv.provider.alist.rs");
    }

    pub mod emby {
        #[allow(clippy::all)]
        #[allow(warnings)]
        include!("providers/synctv.provider.emby.rs");
    }
}
