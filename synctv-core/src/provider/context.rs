// Provider Context
//
// Contains all information needed for provider execution

/// Provider execution context
///
/// Provides access to database, Redis, user information, and other resources
/// needed by providers to generate playback information.
#[derive(Debug, Clone)]
pub struct ProviderContext<'a> {
    /// User ID requesting playback (optional)
    pub user_id: Option<&'a str>,

    /// Room ID (optional)
    pub room_id: Option<&'a str>,

    /// Base URL for generating proxy URLs
    pub base_url: Option<&'a str>,

    /// Cache key prefix (e.g., "synctv")
    pub key_prefix: &'a str,
    // TODO: Add database and Redis pools when implementing
    // pub db: &'a PgPool,
    // pub redis: &'a Pool,
}

impl<'a> ProviderContext<'a> {
    /// Create new context with defaults
    pub fn new(key_prefix: &'a str) -> Self {
        Self {
            user_id: None,
            room_id: None,
            base_url: None,
            key_prefix,
        }
    }

    /// Set user ID
    pub fn with_user_id(mut self, user_id: &'a str) -> Self {
        self.user_id = Some(user_id);
        self
    }

    /// Set room ID
    pub fn with_room_id(mut self, room_id: &'a str) -> Self {
        self.room_id = Some(room_id);
        self
    }

    /// Set base URL
    pub fn with_base_url(mut self, base_url: &'a str) -> Self {
        self.base_url = Some(base_url);
        self
    }
}
