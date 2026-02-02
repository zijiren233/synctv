//! Media and playlist management service
//!
//! Design reference: /Volumes/workspace/rust/design/08-视频内容管理.md
//!
//! Three-stage workflow:
//! 1. Parse - Parse user input to get options
//! 2. Add Media - Store source_config in database
//! 3. Generate Playback - Dynamically generate playback info when playing

use crate::{
    models::{Media, MediaId, PlaylistId, RoomId, UserId, PermissionBits},
    repository::{MediaRepository, PlaylistRepository},
    service::{permission::PermissionService, ProvidersManager},
    provider::{MediaProvider, ProviderContext},
    Error, Result,
};
use serde_json::Value as JsonValue;
use std::sync::Arc;

/// Request to add a media item
///
/// Design note: According to the three-stage workflow,
/// clients should call parse endpoint first, then construct source_config,
/// and finally call add_media with the validated source_config.
///
/// Uses provider registry pattern - provider_instance_name identifies which
/// provider instance to use (e.g., "bilibili_main", "alist_company").
#[derive(Debug, Clone)]
pub struct AddMediaRequest {
    pub playlist_id: PlaylistId,
    pub name: String,
    /// Provider instance name (e.g., "bilibili_main", "alist_company")
    /// The provider will be looked up from the provider registry
    pub provider_instance_name: String,
    pub source_config: JsonValue,
    pub metadata: Option<JsonValue>,
}

/// Request to edit a media item
#[derive(Debug, Clone)]
pub struct EditMediaRequest {
    pub media_id: MediaId,
    pub name: Option<String>,
    pub position: Option<i32>,
    pub metadata: Option<JsonValue>,
}

/// Media management service
///
/// Responsible for media operations based on the new architecture:
/// - Media belongs to a playlist (not directly to room)
/// - Media stores source_config (persistent configuration)
/// - Playback info is generated dynamically by providers
/// - Uses provider registry pattern to avoid enum switching
#[derive(Clone)]
pub struct MediaService {
    media_repo: MediaRepository,
    playlist_repo: PlaylistRepository,
    permission_service: PermissionService,
    providers_manager: Arc<ProvidersManager>,
}

impl std::fmt::Debug for MediaService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaService").finish()
    }
}

impl MediaService {
    /// Create a new media service
    pub fn new(
        media_repo: MediaRepository,
        playlist_repo: PlaylistRepository,
        permission_service: PermissionService,
        providers_manager: Arc<ProvidersManager>,
    ) -> Self {
        Self {
            media_repo,
            playlist_repo,
            permission_service,
            providers_manager,
        }
    }

    /// Add media to a playlist
    ///
    /// Three-stage workflow - Stage 2:
    /// 1. Client calls parse endpoint (Stage 1)
    /// 2. Client constructs source_config
    /// 3. Client calls add_media with source_config
    /// 4. Service validates using provider and stores in database
    pub async fn add_media(
        &self,
        room_id: RoomId,
        user_id: UserId,
        request: AddMediaRequest,
    ) -> Result<Media> {
        // Check permission
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::ADD_MEDIA)
            .await?;

        // Verify playlist belongs to room
        let playlist = self
            .playlist_repo
            .get_by_id(&request.playlist_id)
            .await?
            .ok_or_else(|| Error::NotFound("Playlist not found".to_string()))?;

        if playlist.room_id != room_id {
            return Err(Error::Authorization("Playlist does not belong to this room".to_string()));
        }

        // Get provider from registry by instance name
        // The registry stores actual Arc<dyn MediaProvider> instances
        let provider = self
            .providers_manager
            .get(&request.provider_instance_name)
            .await
            .ok_or_else(|| {
                Error::NotFound(format!(
                    "Provider instance not found: {}",
                    request.provider_instance_name
                ))
            })?;

        // Validate source_config using provider trait method
        let ctx = ProviderContext::new("synctv")
            .with_user_id(user_id.as_str())
            .with_room_id(room_id.as_str());

        provider
            .validate_source_config(&ctx, &request.source_config)
            .await
            .map_err(|e| Error::InvalidInput(format!("Invalid source_config: {}", e)))?;

        // Get next position in playlist
        let position = self.media_repo.get_next_position(&request.playlist_id).await?;

        // Create media with provider info (no enum conversion needed)
        // Business logic will use provider_instance_name to get provider from registry
        let media = Media::from_provider(
            request.playlist_id.clone(),
            room_id.clone(),
            user_id.clone(),
            request.name.clone(),
            request.source_config.clone(),
            provider.name(),  // Provider type name (e.g., "bilibili")
            request.provider_instance_name.clone(),  // Instance name (e.g., "bilibili_main")
            position,
        );

        let created_media = self.media_repo.create(&media).await?;

        tracing::info!(
            room_id = %room_id.as_str(),
            playlist_id = %request.playlist_id.as_str(),
            media_id = %created_media.id.as_str(),
            name = %created_media.name,
            provider = %request.provider_instance_name,
            "Media added to playlist"
        );

        Ok(created_media)
    }

    /// Add multiple media items to a playlist
    pub async fn add_media_batch(
        &self,
        room_id: RoomId,
        user_id: UserId,
        playlist_id: PlaylistId,
        items: Vec<AddMediaRequest>,
    ) -> Result<Vec<Media>> {
        // Check permission
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::ADD_MEDIA)
            .await?;

        // Verify playlist belongs to room
        let playlist = self
            .playlist_repo
            .get_by_id(&playlist_id)
            .await?
            .ok_or_else(|| Error::NotFound("Playlist not found".to_string()))?;

        if playlist.room_id != room_id {
            return Err(Error::Authorization("Playlist does not belong to this room".to_string()));
        }

        if items.is_empty() {
            return Ok(Vec::new());
        }

        // Get starting position
        let start_position = self.media_repo.get_next_position(&playlist_id).await?;

        // Create provider context for validation
        let ctx = ProviderContext::new("synctv")
            .with_user_id(user_id.as_str())
            .with_room_id(room_id.as_str());

        // Create media items with provider validation
        let mut media_items = Vec::with_capacity(items.len());
        for (index, item) in items.into_iter().enumerate() {
            // Get provider from registry by instance name
            let provider = self
                .providers_manager
                .get(&item.provider_instance_name)
                .await
                .ok_or_else(|| {
                    Error::NotFound(format!(
                        "Provider instance not found: {}",
                        item.provider_instance_name
                    ))
                })?;

            // Validate source_config using provider trait method
            provider
                .validate_source_config(&ctx, &item.source_config)
                .await
                .map_err(|e| Error::InvalidInput(format!("Invalid source_config for item '{}': {}", item.name, e)))?;

            let media = Media::from_provider(
                item.playlist_id,
                room_id.clone(),
                user_id.clone(),
                item.name,
                item.source_config,
                provider.name(),  // Provider type name
                item.provider_instance_name,  // Instance name
                start_position + index as i32,
            );
            media_items.push(media);
        }

        // Batch insert
        let created_items = self.media_repo.create_batch(&media_items).await?;

        tracing::info!(
            room_id = %room_id.as_str(),
            playlist_id = %playlist_id.as_str(),
            count = created_items.len(),
            "Batch added media to playlist"
        );

        Ok(created_items)
    }

    /// Edit media item
    pub async fn edit_media(
        &self,
        room_id: RoomId,
        user_id: UserId,
        request: EditMediaRequest,
    ) -> Result<Media> {
        // Check permission (use ADD_MEDIA for editing)
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::ADD_MEDIA)
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
        if let Some(name) = request.name {
            media.name = name;
        }
        if let Some(position) = request.position {
            media.position = position;
        }
        if let Some(metadata) = request.metadata {
            media.metadata = metadata;
        }

        let updated_media = self.media_repo.update(&media).await?;

        tracing::info!(
            room_id = %room_id.as_str(),
            media_id = %request.media_id.as_str(),
            "Media edited"
        );

        Ok(updated_media)
    }

    /// Remove media from playlist
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

        // Get existing media to verify ownership
        let media = self
            .media_repo
            .get_by_id(&media_id)
            .await?
            .ok_or_else(|| Error::NotFound("Media not found".to_string()))?;

        // Verify media belongs to the room
        if media.room_id != room_id {
            return Err(Error::Authorization("Media does not belong to this room".to_string()));
        }

        // Soft delete
        self.media_repo.delete(&media_id).await?;

        tracing::info!(
            room_id = %room_id.as_str(),
            media_id = %media_id.as_str(),
            "Media removed from playlist"
        );

        Ok(())
    }

    /// Get media by ID
    pub async fn get_media(&self, media_id: &MediaId) -> Result<Option<Media>> {
        self.media_repo.get_by_id(media_id).await
    }

    /// Get all media in a playlist
    pub async fn get_playlist_media(&self, playlist_id: &PlaylistId) -> Result<Vec<Media>> {
        self.media_repo.get_by_playlist(playlist_id).await
    }

    /// Get paginated media in a playlist
    pub async fn get_playlist_media_paginated(
        &self,
        playlist_id: &PlaylistId,
        page: i32,
        page_size: i32,
    ) -> Result<(Vec<Media>, i64)> {
        self.media_repo.get_playlist_paginated(playlist_id, page, page_size).await
    }

    /// Swap positions of two media items
    pub async fn swap_media_positions(
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

        // Verify both media exist and belong to the room
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

        tracing::info!(
            room_id = %room_id.as_str(),
            media_id1 = %media_id1.as_str(),
            media_id2 = %media_id2.as_str(),
            "Media positions swapped"
        );

        Ok(())
    }

    /// Count media items in a playlist
    pub async fn count_playlist_media(&self, playlist_id: &PlaylistId) -> Result<i64> {
        self.media_repo.count_by_playlist(playlist_id).await
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
}
