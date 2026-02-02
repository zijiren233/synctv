//! Playlist management service
//!
//! Design reference: /Volumes/workspace/rust/design/04-数据库设计.md §2.4.1
//!
//! Manages playlist/folder operations including:
//! - Creating folders (static and dynamic)
//! - Tree structure navigation
//! - Position management

use crate::{
    models::{Playlist, PlaylistId, RoomId, UserId, PermissionBits},
    repository::PlaylistRepository,
    service::permission::PermissionService,
    Error, Result,
};
use serde_json::Value as JsonValue;

/// Request to create a playlist/folder
#[derive(Debug, Clone)]
pub struct CreatePlaylistRequest {
    pub room_id: RoomId,
    pub name: String,
    pub parent_id: Option<PlaylistId>,
    pub position: Option<i32>,

    // Dynamic folder fields
    pub source_provider: Option<String>,
    pub source_config: Option<JsonValue>,
    pub provider_instance_name: Option<String>,
}

/// Request to update a playlist/folder
#[derive(Debug, Clone)]
pub struct UpdatePlaylistRequest {
    pub playlist_id: PlaylistId,
    pub name: Option<String>,
    pub position: Option<i32>,
}

/// Playlist management service
///
/// Responsible for playlist/folder operations:
/// - Create static folders (manually added media)
/// - Create dynamic folders (Alist/Emby directories)
/// - Tree structure navigation
#[derive(Clone)]
pub struct PlaylistService {
    playlist_repo: PlaylistRepository,
    permission_service: PermissionService,
}

impl std::fmt::Debug for PlaylistService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaylistService").finish()
    }
}

impl PlaylistService {
    /// Create a new playlist service
    pub fn new(
        playlist_repo: PlaylistRepository,
        permission_service: PermissionService,
    ) -> Self {
        Self {
            playlist_repo,
            permission_service,
        }
    }

    /// Create a new playlist/folder
    pub async fn create_playlist(
        &self,
        room_id: RoomId,
        user_id: UserId,
        request: CreatePlaylistRequest,
    ) -> Result<Playlist> {
        // Check permission
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::ADD_MEDIA)
            .await?;

        // Verify parent exists and belongs to room
        if let Some(ref parent_id) = request.parent_id {
            let parent = self
                .playlist_repo
                .get_by_id(parent_id)
                .await?
                .ok_or_else(|| Error::NotFound("Parent playlist not found".to_string()))?;

            if parent.room_id != room_id {
                return Err(Error::Authorization("Parent playlist does not belong to this room".to_string()));
            }

            // Check if name is unique in parent
            // Note: Database has UNIQUE constraint, so this will fail anyway
        }

        // Calculate position if not provided
        let position = if let Some(pos) = request.position {
            pos
        } else {
            self.playlist_repo
                .get_next_position(&room_id.clone(), request.parent_id.as_ref())
                .await?
        };

        // Validate dynamic folder requirements
        if request.source_provider.is_some() && request.source_config.is_none() {
            return Err(Error::InvalidInput(
                "source_config is required for dynamic folders".to_string()
            ));
        }

        // Create playlist
        let playlist = Playlist {
            id: crate::models::PlaylistId::new(),
            room_id: room_id.clone(),
            creator_id: user_id,
            name: request.name,
            parent_id: request.parent_id,
            position,
            source_provider: request.source_provider,
            source_config: request.source_config,
            provider_instance_name: request.provider_instance_name,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let created_playlist = self.playlist_repo.create(&playlist).await?;

        tracing::info!(
            room_id = %room_id.as_str(),
            playlist_id = %created_playlist.id.as_str(),
            name = %created_playlist.name,
            is_dynamic = created_playlist.is_dynamic(),
            "Playlist created"
        );

        Ok(created_playlist)
    }

    /// Get playlist by ID
    pub async fn get_playlist(&self, playlist_id: &PlaylistId) -> Result<Option<Playlist>> {
        self.playlist_repo.get_by_id(playlist_id).await
    }

    /// Get root playlist for a room
    pub async fn get_root_playlist(&self, room_id: &RoomId) -> Result<Playlist> {
        self.playlist_repo.get_root_playlist(room_id).await
    }

    /// Get children playlists
    pub async fn get_children(&self, parent_id: &PlaylistId) -> Result<Vec<Playlist>> {
        self.playlist_repo.get_children(parent_id).await
    }

    /// Get all playlists in a room (tree structure)
    pub async fn get_room_playlists(&self, room_id: &RoomId) -> Result<Vec<Playlist>> {
        self.playlist_repo.get_by_room(room_id).await
    }

    /// Update playlist
    pub async fn update_playlist(
        &self,
        room_id: RoomId,
        user_id: UserId,
        request: UpdatePlaylistRequest,
    ) -> Result<Playlist> {
        // Check permission
        self.permission_service
            .check_permission(&room_id, &user_id, PermissionBits::ADD_MEDIA)
            .await?;

        // Get existing playlist
        let mut playlist = self
            .playlist_repo
            .get_by_id(&request.playlist_id)
            .await?
            .ok_or_else(|| Error::NotFound("Playlist not found".to_string()))?;

        // Verify playlist belongs to room
        if playlist.room_id != room_id {
            return Err(Error::Authorization("Playlist does not belong to this room".to_string()));
        }

        // Update fields
        if let Some(name) = request.name {
            playlist.name = name;
        }
        if let Some(position) = request.position {
            playlist.position = position;
        }

        let updated_playlist = self.playlist_repo.update(&playlist).await?;

        tracing::info!(
            room_id = %room_id.as_str(),
            playlist_id = %request.playlist_id.as_str(),
            "Playlist updated"
        );

        Ok(updated_playlist)
    }

    /// Delete playlist
    pub async fn delete_playlist(
        &self,
        room_id: RoomId,
        user_id: UserId,
        playlist_id: PlaylistId,
    ) -> Result<()> {
        // Check permission (admin or creator)
        // TODO: Verify user is creator or admin

        // Get playlist to verify ownership
        let playlist = self
            .playlist_repo
            .get_by_id(&playlist_id)
            .await?
            .ok_or_else(|| Error::NotFound("Playlist not found".to_string()))?;

        if playlist.room_id != room_id {
            return Err(Error::Authorization("Playlist does not belong to this room".to_string()));
        }

        // Cannot delete root playlist
        if playlist.is_root() {
            return Err(Error::InvalidInput("Cannot delete root playlist".to_string()));
        }

        // Delete (will cascade to children and media)
        self.playlist_repo.delete(&playlist_id).await?;

        tracing::info!(
            room_id = %room_id.as_str(),
            playlist_id = %playlist_id.as_str(),
            "Playlist deleted"
        );

        Ok(())
    }

    /// Get playlist path (breadcrumbs)
    pub async fn get_playlist_path(&self, playlist_id: &PlaylistId) -> Result<Vec<Playlist>> {
        let mut path = Vec::new();
        let mut current = self
            .playlist_repo
            .get_by_id(playlist_id)
            .await?
            .ok_or_else(|| Error::NotFound("Playlist not found".to_string()))?;

        path.push(current.clone());

        // Walk up the tree
        while let Some(parent_id) = current.parent_id.clone() {
            current = self
                .playlist_repo
                .get_by_id(&parent_id)
                .await?
                .ok_or_else(|| Error::NotFound("Parent playlist not found".to_string()))?;

            path.push(current.clone());
        }

        // Reverse to get root → leaf order
        path.reverse();
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Requires database"]
    async fn test_create_playlist() {
        // Integration test placeholder
    }
}
