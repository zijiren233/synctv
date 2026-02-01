//! SyncTV Protocol Definitions
//!
//! This crate contains all protobuf definitions and generated code for SyncTV's
//! external APIs, including:
//! - Client API (client.proto)
//! - Admin API (admin.proto)
//! - Provider protocols (providers/*.proto)
//!
//! Note: Internal cluster communication (cluster.proto) is generated in synctv-cluster crate
//!
//! Both the service layer and API layer use these types to ensure consistency.

// We include proto files in submodules to avoid type name conflicts
// between client.proto and admin.proto (they both define types like RoomMember, etc.)

pub mod client {
    #![allow(clippy::all)]
    #![allow(warnings)]
    include!("synctv.client.rs");
}

pub mod admin {
    #![allow(clippy::all)]
    #![allow(warnings)]
    include!("synctv.admin.rs");
}

pub mod providers {
    pub mod bilibili {
        #![allow(clippy::all)]
        #![allow(warnings)]
        include!("synctv.provider.bilibili.rs");
    }

    pub mod alist {
        #![allow(clippy::all)]
        #![allow(warnings)]
        include!("synctv.provider.alist.rs");
    }

    pub mod emby {
        #![allow(clippy::all)]
        #![allow(warnings)]
        include!("synctv.provider.emby.rs");
    }
}

// Re-export server traits for gRPC service implementations
pub use client::{
    auth_service_server,
    user_service_server,
    room_service_server,
    media_service_server,
    public_service_server,
};

pub use admin::admin_service_server;

// Re-export commonly used types from client API
pub use client::{
    // User operations
    RegisterRequest, RegisterResponse,
    LoginRequest, LoginResponse,
    RefreshTokenRequest, RefreshTokenResponse,
    GetCurrentUserRequest, GetCurrentUserResponse,
    LogoutRequest, LogoutResponse,
    UpdateUsernameRequest, UpdateUsernameResponse,
    UpdatePasswordRequest, UpdatePasswordResponse,

    // Room operations
    CreateRoomRequest, CreateRoomResponse,
    GetRoomRequest, GetRoomResponse,
    JoinRoomRequest, JoinRoomResponse,
    LeaveRoomRequest, LeaveRoomResponse,
    ListRoomsRequest, ListRoomsResponse,
    DeleteRoomRequest, DeleteRoomResponse,
    UpdateRoomSettingsRequest, UpdateRoomSettingsResponse,

    // Member operations
    UpdateMemberPermissionRequest, UpdateMemberPermissionResponse,
    KickMemberRequest, KickMemberResponse,

    // Media operations
    AddMediaRequest, AddMediaResponse,
    RemoveMediaRequest, RemoveMediaResponse,
    GetPlaylistRequest, GetPlaylistResponse,
    SwapMediaRequest, SwapMediaResponse,

    // Playback operations
    PlayRequest, PlayResponse,
    PauseRequest, PauseResponse,
    SeekRequest, SeekResponse,
    ChangeSpeedRequest, ChangeSpeedResponse,
    SwitchMediaRequest, SwitchMediaResponse,
    GetPlaybackStateRequest, GetPlaybackStateResponse,

    // Chat operations
    GetChatHistoryRequest, GetChatHistoryResponse,

    // Additional room operations
    GetMyRoomsRequest, GetMyRoomsResponse,
    GetJoinedRoomsRequest, GetJoinedRoomsResponse,
    CheckRoomRequest, CheckRoomResponse,
    GetHotRoomsRequest, GetHotRoomsResponse,

    // Provider operations
    NewPublishKeyRequest, NewPublishKeyResponse,

    // Messaging
    ClientMessage, ServerMessage,
};

// Admin API types
pub use admin::{
    GetRoomRequest as AdminGetRoomRequest,
    GetRoomResponse as AdminGetRoomResponse,
    ListRoomsRequest as AdminListRoomsRequest,
    ListRoomsResponse as AdminListRoomsResponse,
    DeleteRoomRequest as AdminDeleteRoomRequest,
    DeleteRoomResponse as AdminDeleteRoomResponse,
    UpdateRoomPasswordRequest, UpdateRoomPasswordResponse,
    GetRoomMembersRequest, GetRoomMembersResponse,
    RoomMember,
    AdminUser, AdminRoom,
    ProviderInstance,
};

// Provider protocols
// Note: Provider service modules are exported but types may vary by provider
pub use providers::{
    bilibili::bilibili_provider_service_server,
    alist::alist_provider_service_server,
    emby::emby_provider_service_server,
};
