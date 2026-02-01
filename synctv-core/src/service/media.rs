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

/// Request to add a media item
#[derive(Debug, Clone)]
pub struct AddMediaRequest {
    pub url: String,
    pub provider: ProviderType,
    pub title: String,
    pub metadata: Option<serde_json::Value>,
}

/// Request to edit a media item
#[derive(Debug, Clone)]
pub struct EditMediaRequest {
    pub media_id: MediaId,
    pub url: Option<String>,
    pub title: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

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

    /// Add multiple media items to a room's playlist
    pub async fn add_media_batch(
        &self,
        room_id: RoomId,
        user_id: UserId,
        items: Vec<AddMediaRequest>,
    ) -> Result<Vec<Media>> {
        // Check permission
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::ADD_MEDIA)
            .await?;

        if items.is_empty() {
            return Ok(Vec::new());
        }

        // Get starting position
        let start_position = self.media_repo.get_next_position(&room_id).await?;

        // Create media items
        let mut media_items = Vec::with_capacity(items.len());
        for (index, item) in items.into_iter().enumerate() {
            let media = Media::new(
                room_id.clone(),
                item.url,
                item.provider,
                item.title,
                item.metadata.unwrap_or_default(),
                start_position + index as i32,
                user_id.clone(),
            );
            media_items.push(media);
        }

        // Batch insert
        let created_items = self.media_repo.create_batch(&media_items).await?;

        Ok(created_items)
    }

    /// Edit media item
    pub async fn edit_media(
        &self,
        room_id: RoomId,
        user_id: UserId,
        request: EditMediaRequest,
    ) -> Result<Media> {
        // Check permission
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::REMOVE_MEDIA)
            .await?;

        // Get existing media
        let mut media = self
            .media_repo
            .get_by_id(&request.media_id)
            .await?
            .ok_or_else(|| Error::NotFound("Media not found".to_string()))?;

        // Verify media belongs to the room
        if media.room_id != room_id {
            return Err(Error::Authorization("Media does not belong to this room".to_string()));
        }

        // Update fields
        if let Some(url) = request.url {
            media.url = url;
        }
        if let Some(title) = request.title {
            media.title = title;
        }
        if let Some(metadata) = request.metadata {
            media.metadata = metadata;
        }

        // Save changes
        let updated_media = self.media_repo.update(&media).await?;

        Ok(updated_media)
    }

    /// Clear all media from a room's playlist
    pub async fn clear_playlist(
        &self,
        room_id: RoomId,
        user_id: UserId,
    ) -> Result<usize> {
        // Check permission
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::REMOVE_MEDIA)
            .await?;

        // Delete all media in the room
        let count = self.media_repo.delete_by_room(&room_id).await?;

        Ok(count)
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

    /// Set current playing media
    pub async fn set_current_media(
        &self,
        room_id: RoomId,
        user_id: UserId,
        media_id: MediaId,
    ) -> Result<Media> {
        // Check permission
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::SWITCH_MEDIA)
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

        // Move media to first position (or use a separate current_media tracking mechanism)
        // For now, we'll reorder the playlist
        self.media_repo.move_to_first(&media_id).await?;

        Ok(media)
    }

    /// Get playlist for a room
    pub async fn get_playlist(&self, room_id: &RoomId) -> Result<Vec<Media>> {
        self.media_repo.get_playlist(room_id).await
    }

    /// Get paginated playlist
    pub async fn get_playlist_paginated(
        &self,
        room_id: &RoomId,
        page: i32,
        page_size: i32,
    ) -> Result<(Vec<Media>, i64)> {
        self.media_repo.get_playlist_paginated(room_id, page, page_size).await
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

    /// Get media count for a room
    pub async fn get_media_count(&self, room_id: &RoomId) -> Result<i64> {
        self.media_repo.count_by_room(room_id).await
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
