//! Optimized room repository with type-safe query building
//!
//! Uses BoolExpr and FilterBuilder for safe, composable SQL queries

use sqlx::{PgPool, postgres::PgRow, Row};
use serde_json::Value as JsonValue;

use crate::{
    models::{Room, RoomId, RoomStatus, UserId},
    repository::{BoolExpr, Column, FilterBuilder},
    Error, Result,
};

/// Query builder for room list operations
pub struct RoomListQuery {
    /// Page number (1-indexed)
    pub page: i64,
    /// Number of items per page
    pub page_size: i64,
    /// Filter by room status
    pub status: Option<RoomStatus>,
    /// Search in room name
    pub search: Option<String>,
    /// Additional boolean filter
    pub filter: Option<BoolExpr>,
}

impl Default for RoomListQuery {
    fn default() -> Self {
        Self {
            page: 1,
            page_size: 20,
            status: None,
            search: None,
            filter: None,
        }
    }
}

/// Room repository for database operations
#[derive(Clone)]
pub struct RoomRepository {
    pool: PgPool,
}

impl RoomRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get the database pool
    pub fn pool(&self) -> &PgPool {
        &self.pool
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
             SET deleted_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP
             WHERE id = $1 AND deleted_at IS NULL"
        )
        .bind(room_id.as_str())
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// List rooms with type-safe query builder
    pub async fn list(&self, query: &RoomListQuery) -> Result<(Vec<Room>, i64)> {
        let filter = self.build_filter(query);
        let offset = (query.page - 1) * query.page_size;

        // Get total count
        let count_query = format!(
            "SELECT COUNT(*) as count FROM rooms r WHERE {}",
            filter.to_sql()
        );
        let count: i64 = sqlx::query_scalar(&count_query)
            .fetch_one(&self.pool)
            .await?;

        // Get rooms
        let list_query = format!(
            "SELECT r.id, r.name, r.created_by, r.status, r.settings, r.created_at, r.updated_at, r.deleted_at
             FROM rooms r
             WHERE {}
             ORDER BY r.created_at DESC
             LIMIT $1 OFFSET $2",
            filter.to_sql()
        );

        let rows = sqlx::query(&list_query)
            .bind(query.page_size)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;

        let rooms: Result<Vec<Room>> = rows.into_iter().map(|row| self.row_to_room(row)).collect();

        Ok((rooms?, count))
    }

    /// List rooms with member count (optimized with JOIN and boolean filter)
    pub async fn list_with_count(&self, query: &RoomListQuery) -> Result<(Vec<crate::models::RoomWithCount>, i64)> {
        let filter = self.build_filter(query);
        let offset = (query.page - 1) * query.page_size;

        // Get total count
        let count_query = format!(
            "SELECT COUNT(DISTINCT r.id) FROM rooms r WHERE {}",
            filter.to_sql()
        );

        let count: i64 = if let Some(search) = &query.search {
            let search_pattern = format!("%{}%", search);
            sqlx::query_scalar(&count_query)
                .bind(&search_pattern)
                .fetch_one(&self.pool)
                .await?
        } else {
            sqlx::query_scalar(&count_query)
                .fetch_one(&self.pool)
                .await?
        };

        // Get rooms with member count using LEFT JOIN
        let list_query = format!(
            r#"
            SELECT
                r.id, r.name, r.created_by, r.status, r.settings,
                r.created_at, r.updated_at, r.deleted_at,
                COALESCE(COUNT(rm.user_id) FILTER (WHERE rm.left_at IS NULL), 0)::int as member_count
            FROM rooms r
            LEFT JOIN room_members rm ON r.id = rm.room_id
            WHERE {}
            GROUP BY r.id, r.name, r.created_by, r.status, r.settings, r.created_at, r.updated_at, r.deleted_at
            ORDER BY r.created_at DESC
            LIMIT $1 OFFSET $2
            "#,
            filter.to_sql()
        );

        let rows = if let Some(search) = &query.search {
            let search_pattern = format!("%{}%", search);
            sqlx::query(&list_query)
                .bind(query.page_size)
                .bind(offset)
                .bind(&search_pattern)
                .fetch_all(&self.pool)
                .await?
        } else {
            sqlx::query(&list_query)
                .bind(query.page_size)
                .bind(offset)
                .fetch_all(&self.pool)
                .await?
        };

        let rooms_with_count: Result<Vec<crate::models::RoomWithCount>> = rows
            .into_iter()
            .map(|row| {
                let member_count: i32 = row.try_get("member_count")?;
                let room = self.row_to_room(row)?;
                Ok(crate::models::RoomWithCount {
                    room,
                    member_count,
                })
            })
            .collect();

        Ok((rooms_with_count?, count))
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

    /// Get rooms created by a specific user
    pub async fn list_by_creator(&self, creator_id: &UserId, page: i64, page_size: i64) -> Result<(Vec<Room>, i64)> {
        let offset = (page - 1) * page_size;

        // Use boolean filter for type-safe query building
        let filter = FilterBuilder::new()
            .eq("r.created_by", creator_id.as_str())
            .is_null("r.deleted_at")
            .build();

        // Get total count
        let count_query = format!(
            "SELECT COUNT(*) as count FROM rooms r WHERE {}",
            filter.to_sql()
        );
        let count: i64 = sqlx::query_scalar(&count_query)
            .fetch_one(&self.pool)
            .await?;

        // Get rooms
        let list_query = format!(
            "SELECT r.id, r.name, r.created_by, r.status, r.settings, r.created_at, r.updated_at, r.deleted_at
             FROM rooms r
             WHERE {}
             ORDER BY r.created_at DESC
             LIMIT $1 OFFSET $2",
            filter.to_sql()
        );

        let rows = sqlx::query(&list_query)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;

        let rooms: Result<Vec<Room>> = rows.into_iter().map(|row| self.row_to_room(row)).collect();

        Ok((rooms?, count))
    }

    /// Get rooms created by a specific user with member count (optimized)
    pub async fn list_by_creator_with_count(
        &self,
        creator_id: &UserId,
        page: i64,
        page_size: i64,
    ) -> Result<(Vec<crate::models::RoomWithCount>, i64)> {
        let offset = (page - 1) * page_size;

        // Build filter with boolean expressions
        let filter = FilterBuilder::new()
            .eq("r.created_by", creator_id.as_str())
            .is_null("r.deleted_at")
            .build();

        // Get total count
        let count_query = format!(
            "SELECT COUNT(*) as count FROM rooms r WHERE {}",
            filter.to_sql()
        );
        let count: i64 = sqlx::query_scalar(&count_query)
            .fetch_one(&self.pool)
            .await?;

        // Get rooms with member count using LEFT JOIN
        let list_query = format!(
            r#"
            SELECT
                r.id, r.name, r.created_by, r.status, r.settings,
                r.created_at, r.updated_at, r.deleted_at,
                COALESCE(COUNT(rm.user_id) FILTER (WHERE rm.left_at IS NULL), 0)::int as member_count
            FROM rooms r
            LEFT JOIN room_members rm ON r.id = rm.room_id
            WHERE {}
            GROUP BY r.id, r.name, r.created_by, r.status, r.settings, r.created_at, r.updated_at, r.deleted_at
            ORDER BY r.created_at DESC
            LIMIT $1 OFFSET $2
            "#,
            filter.to_sql()
        );

        let rows = sqlx::query(&list_query)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;

        let rooms_with_count: Result<Vec<crate::models::RoomWithCount>> = rows
            .into_iter()
            .map(|row| {
                let member_count: i32 = row.try_get("member_count")?;
                let room = self.row_to_room(row)?;
                Ok(crate::models::RoomWithCount {
                    room,
                    member_count,
                })
            })
            .collect();

        Ok((rooms_with_count?, count))
    }

    /// Find rooms by complex criteria using boolean filter
    pub async fn find(&self, filter: &BoolExpr, page: i64, page_size: i64) -> Result<(Vec<Room>, i64)> {
        let offset = (page - 1) * page_size;

        // Always exclude deleted rooms
        let filter = BoolExpr::and(vec![
            filter.clone(),
            BoolExpr::is_null("r.deleted_at"),
        ]);

        // Get total count
        let count_query = format!(
            "SELECT COUNT(*) as count FROM rooms r WHERE {}",
            filter.to_sql()
        );
        let count: i64 = sqlx::query_scalar(&count_query)
            .fetch_one(&self.pool)
            .await?;

        // Get rooms
        let list_query = format!(
            "SELECT r.id, r.name, r.created_by, r.status, r.settings, r.created_at, r.updated_at, r.deleted_at
             FROM rooms r
             WHERE {}
             ORDER BY r.created_at DESC
             LIMIT $1 OFFSET $2",
            filter.to_sql()
        );

        let rows = sqlx::query(&list_query)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;

        let rooms: Result<Vec<Room>> = rows.into_iter().map(|row| self.row_to_room(row)).collect();

        Ok((rooms?, count))
    }

    /// Build filter from RoomListQuery
    fn build_filter(&self, query: &RoomListQuery) -> BoolExpr {
        let mut builder = FilterBuilder::new();

        // Always exclude deleted rooms
        builder = builder.is_null("deleted_at");

        // Add status filter if provided
        if let Some(status) = &query.status {
            builder = builder.eq("r.status", self.status_to_str(status));
        }

        // Add search filter if provided
        if let Some(search) = &query.search {
            builder = builder.ilike("r.name", format!("%{}%", search));
        }

        // Add custom filter if provided
        if let Some(custom_filter) = &query.filter {
            builder = builder.add(custom_filter.clone());
        }

        builder.build()
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

    /// Update room status
    pub async fn update_status(&self, room_id: &RoomId, status: RoomStatus) -> Result<Room> {
        let status_str = self.status_to_str(&status);

        let row = sqlx::query(
            r#"
            UPDATE rooms
            SET status = $1, updated_at = CURRENT_TIMESTAMP
            WHERE id = $2 AND deleted_at IS NULL
            RETURNING id, name, created_by, status, settings, created_at, updated_at, deleted_at
            "#,
        )
        .bind(status_str)
        .bind(room_id.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| Error::Database(e))?;

        self.row_to_room(row)
    }

    fn status_to_str(&self, status: &RoomStatus) -> &'static str {
        match status {
            RoomStatus::Pending => "pending",
            RoomStatus::Active => "active",
            RoomStatus::Closed => "closed",
            RoomStatus::Banned => "banned",
        }
    }

    fn str_to_status(&self, s: &str) -> RoomStatus {
        match s {
            "pending" => RoomStatus::Pending,
            "active" => RoomStatus::Active,
            "closed" => RoomStatus::Closed,
            "banned" => RoomStatus::Banned,
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

    #[tokio::test]
    fn test_filter_building() {
        // Test boolean filter building
        let filter1 = FilterBuilder::new()
            .eq("status", "active")
            .is_null("deleted_at")
            .build();

        assert!(filter1.to_sql().contains("status = 'active'"));
        assert!(filter1.to_sql().contains("deleted_at IS NULL"));

        // Test with search
        let filter2 = FilterBuilder::new()
            .ilike("name", "%test%")
            .eq("created_by", "user123")
            .build();

        assert!(filter2.to_sql().contains("name ILIKE '%test%'"));
        assert!(filter2.to_sql().contains("created_by = 'user123'"));

        // Test complex filter with OR
        let filter3 = BoolExpr::and(vec![
            BoolExpr::eq("status", "active"),
            BoolExpr::or(vec![
                BoolExpr::eq("visibility", "public"),
                BoolExpr::eq("visibility", "unlisted"),
            ]),
        ]);

        assert!(filter3.to_sql().contains("AND"));
        assert!(filter3.to_sql().contains("OR"));

        // Test qualified columns
        let filter4 = FilterBuilder::new()
            .eq(Column::qualified("rooms", "status"), "active")
            .build();

        assert!(filter4.to_sql().contains("rooms.status"));
    }

    #[tokio::test]
    fn test_filter_composition() {
        // Test filter reuse and composition
        let base_filter = FilterBuilder::new()
            .is_null("deleted_at")
            .build();

        let active_filter = BoolExpr::and(vec![
            base_filter,
            BoolExpr::eq("status", "active"),
        ]);

        assert!(active_filter.to_sql().contains("deleted_at IS NULL"));
        assert!(active_filter.to_sql().contains("status = 'active'"));
    }
}
