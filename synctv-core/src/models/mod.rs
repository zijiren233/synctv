pub mod chat;
pub mod id;
pub mod media;
pub mod notification;
pub mod oauth2_client;
pub mod permission;
pub mod playback;
pub mod playlist;
pub mod provider_instance;
pub mod room;
pub mod room_member;
pub mod room_settings;
pub mod settings;
pub mod user;

pub use chat::{
    ChatHistoryQuery, ChatMessage, DanmakuMessage, DanmakuPosition, SendChatRequest,
    SendDanmakuRequest,
};
pub use id::{generate_id, MediaId, PlaylistId, RoomId, UserId};
pub use media::{Media, MediaMetadata, ProviderType};
pub use notification::{
    CreateNotificationRequest, MarkAllAsReadRequest, MarkAsReadRequest, Notification,
    NotificationListQuery, NotificationType,
};
pub use oauth2_client::{
    OAuth2AuthUrlResponse, OAuth2CallbackRequest, OAuth2CallbackResponse, UserOAuthProviderMapping,
    OAuth2Provider, OAuth2UserInfo,
};
pub use permission::{PermissionBits, Role as RoomRole};
pub use playback::{
    ChangeSpeedRequest, PlaybackControlRequest, RoomPlaybackState, SeekRequest, SwitchMediaRequest,
};
pub use playlist::{Playlist, PlaylistWithCount, CreatePlaylistRequest, UpdatePlaylistRequest};
pub use provider_instance::{ProviderCredential, ProviderInstance, UserProviderCredential};
pub use room::{
    CreateRoomRequest, Room, RoomListQuery, RoomStatus, RoomWithCount,
    UpdateRoomRequest, PlayMode, AutoPlaySettings,
};
pub use room_settings::RoomSettings;
pub use room_member::{RoomMember, RoomMemberWithUser, MemberStatus};
pub use settings::{
    default_email_settings, default_oauth_settings, default_server_settings, get_default_settings,
    SettingsGroup, SettingsError,
};
pub use user::{CreateUserRequest, SignupMethod, UpdateUserRequest, User, UserListQuery};
