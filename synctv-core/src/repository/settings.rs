//! Settings repository for database operations

use anyhow::Result;
use sqlx::{PgPool, Row};
use tracing::{debug, info};

use crate::models::settings::{get_default_settings, SettingsGroup};

/// Settings repository
#[derive(Clone)]
pub struct SettingsRepository {
    pool: PgPool,
}

impl SettingsRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get all settings groups
    pub async fn get_all(&self) -> Result<Vec<SettingsGroup>> {
        let rows = sqlx::query(
            r#"
            SELECT id, group_name, settings_json, description, created_at, updated_at
            FROM settings
            ORDER BY group_name
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut groups = Vec::new();
        for row in rows {
            groups.push(self.row_to_settings_group(row)?);
        }

        debug!("Retrieved {} settings groups", groups.len());
        Ok(groups)
    }

    /// Get settings group by name
    pub async fn get_by_name(&self, group_name: &str) -> Result<Option<SettingsGroup>> {
        let row = sqlx::query(
            r#"
            SELECT id, group_name, settings_json, description, created_at, updated_at
            FROM settings
            WHERE group_name = $1
            "#,
        )
        .bind(group_name)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            Ok(Some(self.row_to_settings_group(row)?))
        } else {
            Ok(None)
        }
    }

    /// Get or create settings group (with defaults if not exists)
    pub async fn get_or_create(&self, group_name: &str) -> Result<SettingsGroup> {
        // Try to get existing
        if let Some(group) = self.get_by_name(group_name).await? {
            return Ok(group);
        }

        // Create with defaults
        info!("Creating settings group '{}' with defaults", group_name);
        self.create_with_defaults(group_name).await
    }

    /// Update settings group
    pub async fn update(
        &self,
        group_name: &str,
        settings_json: &serde_json::Value,
    ) -> Result<SettingsGroup> {
        let row = sqlx::query(
            r#"
            UPDATE settings
            SET settings_json = $1, updated_at = NOW()
            WHERE group_name = $2
            RETURNING id, group_name, settings_json, description, created_at, updated_at
            "#,
        )
        .bind(settings_json)
        .bind(group_name)
        .fetch_one(&self.pool)
        .await?;

        info!("Updated settings group '{}'", group_name);
        Ok(self.row_to_settings_group(row)?)
    }

    /// Create settings group with default values
    async fn create_with_defaults(&self, group_name: &str) -> Result<SettingsGroup> {
        let default_json = get_default_settings(group_name)
            .unwrap_or_else(|| serde_json::json!({}));

        let row = sqlx::query(
            r#"
            INSERT INTO settings (group_name, settings_json, description)
            VALUES ($1, $2, $3)
            RETURNING id, group_name, settings_json, description, created_at, updated_at
            "#,
        )
        .bind(group_name)
        .bind(default_json)
        .bind(format!("Auto-generated settings for {}", group_name))
        .fetch_one(&self.pool)
        .await?;

        Ok(self.row_to_settings_group(row)?)
    }

    /// Delete settings group
    pub async fn delete(&self, group_name: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM settings WHERE group_name = $1")
            .bind(group_name)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Reset settings group to defaults
    pub async fn reset_to_defaults(&self, group_name: &str) -> Result<SettingsGroup> {
        let default_json = get_default_settings(group_name)
            .ok_or_else(|| anyhow::anyhow!("No defaults defined for group '{}'", group_name))?;

        let row = sqlx::query(
            r#"
            UPDATE settings
            SET settings_json = $1, updated_at = NOW()
            WHERE group_name = $2
            RETURNING id, group_name, settings_json, description, created_at, updated_at
            "#,
        )
        .bind(default_json)
        .bind(group_name)
        .fetch_one(&self.pool)
        .await?;

        info!("Reset settings group '{}' to defaults", group_name);
        Ok(self.row_to_settings_group(row)?)
    }

    fn row_to_settings_group(&self, row: sqlx::postgres::PgRow) -> Result<SettingsGroup> {
        Ok(SettingsGroup {
            id: row.try_get("id")?,
            group_name: row.try_get("group_name")?,
            settings_json: row.try_get("settings_json")?,
            description: row.try_get("description")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}
