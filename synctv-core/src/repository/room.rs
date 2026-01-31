use sqlx::{PgPool, postgres::PgRow, Row};
use serde_json::Value as JsonValue;

use crate::{
    models::{Room, RoomId, RoomStatus, UserId, RoomListQuery},
    Result,
};

/// Room repository for database operations
#[derive(Clone)]
pub struct RoomRepository {
    pool: PgPool,
}

impl RoomRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a new room
    pub async fn create(&self, room: &Room) -> Result<Room> {
        let settings_json = serde_json::to_value(&room.settings)?;

        let row = sqlx::query(
            "INSERT INTO rooms (id, name, created_by, status, settings, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             RETURNING id, name, created_by, status, settings, created_at, updated_at, deleted_at"
        )
        .bind(room.id.as_str())
        .bind(&room.name)
        .bind(room.created_by.as_str())
        .bind(self.status_to_str(&room.status))
        .bind(&settings_json)
        .bind(room.created_at)
        .bind(room.updated_at)
        .fetch_one(&self.pool)
        .await?;

        self.row_to_room(row)
    }

    /// Get room by ID
    pub async fn get_by_id(&self, room_id: &RoomId) -> Result<Option<Room>> {
        let row = sqlx::query(
            "SELECT id, name, created_by, status, settings, created_at, updated_at, deleted_at
             FROM rooms
             WHERE id = $1 AND deleted_at IS NULL"
        )
        .bind(room_id.as_str())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_room(row)?)),
            None => Ok(None),
        }
    }

    /// Update room
    pub async fn update(&self, room: &Room) -> Result<Room> {
        let settings_json = serde_json::to_value(&room.settings)?;

        let row = sqlx::query(
            "UPDATE rooms
             SET name = $2, status = $3, settings = $4, updated_at = $5
             WHERE id = $1 AND deleted_at IS NULL
             RETURNING id, name, created_by, status, settings, created_at, updated_at, deleted_at"
        )
        .bind(room.id.as_str())
        .bind(&room.name)
        .bind(self.status_to_str(&room.status))
        .bind(&settings_json)
        .bind(chrono::Utc::now())
        .fetch_one(&self.pool)
        .await?;

        self.row_to_room(row)
    }

    /// Soft delete room
    pub async fn delete(&self, room_id: &RoomId) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE rooms
             SET deleted_at = $2, updated_at = $2
             WHERE id = $1 AND deleted_at IS NULL"
        )
        .bind(room_id.as_str())
        .bind(chrono::Utc::now())
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// List rooms with pagination and filters
    pub async fn list(&self, query: &RoomListQuery) -> Result<(Vec<Room>, i64)> {
        let offset = (query.page - 1) * query.page_size;

        // Build filter conditions
        let mut conditions = vec!["deleted_at IS NULL"];
        let mut params: Vec<String> = vec![];

        if let Some(status) = &query.status {
            conditions.push("status = $");
            params.push(self.status_to_str(status).to_string());
        }

        if let Some(search) = &query.search {
            conditions.push("name ILIKE $");
            params.push(format!("%{}%", search));
        }

        let where_clause = conditions.join(" AND ");

        // Get total count
        let count_query = format!("SELECT COUNT(*) as count FROM rooms WHERE {}", where_clause);
        let count: i64 = sqlx::query_scalar(&count_query)
            .fetch_one(&self.pool)
            .await?;

        // Get rooms
        let list_query = format!(
            "SELECT id, name, created_by, status, settings, created_at, updated_at, deleted_at
             FROM rooms
             WHERE {}
             ORDER BY created_at DESC
             LIMIT $1 OFFSET $2",
            where_clause
        );

        let rows = sqlx::query(&list_query)
            .bind(query.page_size)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;

        let rooms: Result<Vec<Room>> = rows.into_iter().map(|row| self.row_to_room(row)).collect();

        Ok((rooms?, count))
    }

    /// Check if room exists and is active
    pub async fn exists(&self, room_id: &RoomId) -> Result<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) as count
             FROM rooms
             WHERE id = $1 AND deleted_at IS NULL AND status = 'active'"
        )
        .bind(room_id.as_str())
        .fetch_one(&self.pool)
        .await?;

        Ok(count > 0)
    }

    /// Get room member count
    pub async fn get_member_count(&self, room_id: &RoomId) -> Result<i32> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) as count
             FROM room_members
             WHERE room_id = $1 AND left_at IS NULL"
        )
        .bind(room_id.as_str())
        .fetch_one(&self.pool)
        .await?;

        Ok(count as i32)
    }

    /// Convert database row to Room model
    fn row_to_room(&self, row: PgRow) -> Result<Room> {
        let settings_json: JsonValue = row.try_get("settings")?;
        let status_str: String = row.try_get("status")?;

        Ok(Room {
            id: RoomId::from_string(row.try_get("id")?),
            name: row.try_get("name")?,
            created_by: UserId::from_string(row.try_get("created_by")?),
            status: self.str_to_status(&status_str),
            settings: settings_json,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            deleted_at: row.try_get("deleted_at")?,
        })
    }

    fn status_to_str(&self, status: &RoomStatus) -> &'static str {
        match status {
            RoomStatus::Active => "active",
            RoomStatus::Closed => "closed",
        }
    }

    fn str_to_status(&self, s: &str) -> RoomStatus {
        match s {
            "active" => RoomStatus::Active,
            "closed" => RoomStatus::Closed,
            _ => RoomStatus::Active,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Requires database"]
    async fn test_create_room() {
        // Integration test placeholder
    }
}
