//! Room settings repository
//!
//! Manages loading and saving room settings from/to the room_settings table.
//!
//! # Architecture
//!
//! This repository uses a **key-value based storage** approach:
//! - Each room setting is stored as a separate row (room_id, key, value)
//! - Settings are loaded and merged with defaults
//! - Uses serde for automatic serialization/deserialization

use std::sync::Arc;
use std::collections::HashMap;
use sqlx::{PgPool, postgres::PgRow, Row};

use crate::{
    models::{RoomId, RoomSettings},
    Error, Result,
};

/// Room settings repository
#[derive(Clone)]
pub struct RoomSettingsRepository {
    pool: PgPool,
}

impl RoomSettingsRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get all settings for a room as RoomSettings struct
    ///
    /// This uses **automatic serde deserialization** - no manual mapping needed!
    /// The entire settings struct is stored as a single JSON value under key "_settings".
    pub async fn get(&self, room_id: &RoomId) -> Result<RoomSettings> {
        let room_id_str = room_id.as_str();

        // Try to load the complete settings JSON first
        let row = sqlx::query(
            r#"
            SELECT value
            FROM room_settings
            WHERE room_id = $1 AND key = '_settings'
        "#
        )
        .bind(room_id_str)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            let value: String = row.try_get("value")?;
            // Deserialize directly using serde - no manual mapping!
            let settings: RoomSettings = serde_json::from_str(&value)
                .map_err(|e| Error::Internal(format!("Failed to deserialize room settings: {}", e)))?;
            Ok(settings)
        } else {
            // No settings stored, return defaults
            Ok(RoomSettings::default())
        }
    }

    /// Set a specific setting for a room
    pub async fn set(&self, room_id: &RoomId, key: &str, value: &str) -> Result<()> {
        let room_id_str = room_id.as_str();

        sqlx::query(
            r#"
            INSERT INTO room_settings (room_id, key, value)
            VALUES ($1, $2, $3)
            ON CONFLICT (room_id, key)
            DO UPDATE SET value = $3, updated_at = NOW()
        "#
        )
        .bind(room_id_str)
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get a specific setting value for a room
    pub async fn get_value(&self, room_id: &RoomId, key: &str) -> Result<Option<String>> {
        let room_id_str = room_id.as_str();

        let result = sqlx::query(
            r#"
            SELECT value
            FROM room_settings
            WHERE room_id = $1 AND key = $2
        "#
        )
        .bind(room_id_str)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.map(|row| row.try_get("value")).transpose()?)
    }

    /// Get password hash for a room
    pub async fn get_password_hash(&self, room_id: &RoomId) -> Result<Option<String>> {
        self.get_value(room_id, "password").await
    }

    /// Set multiple settings at once
    ///
    /// This uses **automatic serde serialization** - the entire struct becomes JSON!
    /// All settings are stored as a single JSON value under key "_settings".
    pub async fn set_settings(&self, room_id: &RoomId, settings: &RoomSettings) -> Result<()> {
        let room_id_str = room_id.as_str();

        // Serialize entire settings struct to JSON - one line!
        let json_value = serde_json::to_string(settings)
            .map_err(|e| Error::Internal(format!("Failed to serialize room settings: {}", e)))?;

        // Delete old settings
        sqlx::query("DELETE FROM room_settings WHERE room_id = $1 AND key = '_settings'")
            .bind(room_id_str)
            .execute(&self.pool)
            .await?;

        // Insert new settings as single JSON value
        sqlx::query(
            r#"
            INSERT INTO room_settings (room_id, key, value)
            VALUES ($1, '_settings', $2)
            ON CONFLICT (room_id, key)
            DO UPDATE SET value = $2, updated_at = NOW()
        "#
        )
        .bind(room_id_str)
        .bind(&json_value)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Delete a specific setting for a room (revert to default)
    pub async fn delete(&self, room_id: &RoomId, key: &str) -> Result<()> {
        let room_id_str = room_id.as_str();

        sqlx::query("DELETE FROM room_settings WHERE room_id = $1 AND key = $2")
            .bind(room_id_str)
            .bind(key)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Delete all settings for a room
    pub async fn delete_all(&self, room_id: &RoomId) -> Result<()> {
        let room_id_str = room_id.as_str();

        sqlx::query("DELETE FROM room_settings WHERE room_id = $1")
            .bind(room_id_str)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Get all settings for a room as raw HashMap (for type-safe settings system)
    pub async fn get_all_raw(&self, room_id: &RoomId) -> Result<std::collections::HashMap<String, String>> {
        let room_id_str = room_id.as_str();

        let rows = sqlx::query(
            r#"
            SELECT key, value
            FROM room_settings
            WHERE room_id = $1
        "#
        )
        .bind(room_id_str)
        .fetch_all(&self.pool)
        .await?;

        let mut settings = std::collections::HashMap::new();
        for row in rows {
            let key: String = row.try_get("key")?;
            let value: String = row.try_get("value")?;
            settings.insert(key, value);
        }

        Ok(settings)
    }
}
