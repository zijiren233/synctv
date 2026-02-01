//! Media and playlist management service
//!
//! Handles media operations in playlists including adding, removing,
//! reordering, and metadata extraction.

use crate::{
    models::{Media, MediaId, RoomId, UserId, ProviderType, PermissionBits},
    repository::MediaRepository,
    service::permission::PermissionService,
    Error, Result,
};

/// Media management service
///
/// Responsible for playlist operations and media management.
#[derive(Clone)]
pub struct MediaService {
    media_repo: MediaRepository,
    permission_service: PermissionService,
}

impl std::fmt::Debug for MediaService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaService").finish()
    }
}

impl MediaService {
    /// Create a new media service
    pub fn new(media_repo: MediaRepository, permission_service: PermissionService) -> Self {
        Self {
            media_repo,
            permission_service,
        }
    }

    /// Add media to a room's playlist
    pub async fn add_media(
        &self,
        room_id: RoomId,
        user_id: UserId,
        url: String,
        provider: ProviderType,
        title: String,
    ) -> Result<Media> {
        // Check permission
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::ADD_MEDIA)
            .await?;

        // Get next position
        let position = self.media_repo.get_next_position(&room_id).await?;

        // Create media with empty metadata (metadata can be fetched asynchronously)
        let media = Media::new(
            room_id,
            url,
            provider,
            title,
            serde_json::json!({}),
            position,
            user_id,
        );

        let created_media = self.media_repo.create(&media).await?;

        Ok(created_media)
    }

    /// Remove media from a room's playlist
    pub async fn remove_media(
        &self,
        room_id: RoomId,
        user_id: UserId,
        media_id: MediaId,
    ) -> Result<()> {
        // Check permission
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::REMOVE_MEDIA)
            .await?;

        // Verify media belongs to the room
        let media = self
            .media_repo
            .get_by_id(&media_id)
            .await?
            .ok_or_else(|| Error::NotFound("Media not found".to_string()))?;

        if media.room_id != room_id {
            return Err(Error::Authorization("Media does not belong to this room".to_string()));
        }

        // Delete media
        self.media_repo.delete(&media_id).await?;

        Ok(())
    }

    /// Swap positions of two media items in playlist
    pub async fn swap_media(
        &self,
        room_id: RoomId,
        user_id: UserId,
        media_id1: MediaId,
        media_id2: MediaId,
    ) -> Result<()> {
        // Check permission
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::REORDER_PLAYLIST)
            .await?;

        // Verify both media belong to the room
        let media1 = self
            .media_repo
            .get_by_id(&media_id1)
            .await?
            .ok_or_else(|| Error::NotFound("Media not found".to_string()))?;

        let media2 = self
            .media_repo
            .get_by_id(&media_id2)
            .await?
            .ok_or_else(|| Error::NotFound("Media not found".to_string()))?;

        if media1.room_id != room_id || media2.room_id != room_id {
            return Err(Error::Authorization("Media does not belong to this room".to_string()));
        }

        // Swap positions
        self.media_repo.swap_positions(&media_id1, &media_id2).await?;

        Ok(())
    }

    /// Get playlist for a room
    pub async fn get_playlist(&self, room_id: &RoomId) -> Result<Vec<Media>> {
        self.media_repo.get_playlist(room_id).await
    }

    /// Get a specific media item
    pub async fn get_media(&self, media_id: &MediaId) -> Result<Option<Media>> {
        self.media_repo.get_by_id(media_id).await
    }

    /// Get current media in playlist (the one being played)
    pub async fn get_current_media(&self, room_id: &RoomId) -> Result<Option<Media>> {
        let playlist = self.get_playlist(room_id).await?;
        // Return first media item as current (this could be enhanced with playback state)
        Ok(playlist.first().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Requires database"]
    async fn test_add_media() {
        // Integration test placeholder
    }

    #[tokio::test]
    #[ignore = "Requires database"]
    async fn test_swap_media() {
        // Integration test placeholder
    }
}
