use sqlx::{PgPool, postgres::PgRow, Row};

use crate::{
    models::{Room, RoomId, RoomStatus, UserId, RoomListQuery},
    Error, Result,
};

/// Room repository for database operations
#[derive(Clone)]
pub struct RoomRepository {
    pool: PgPool,
}

impl RoomRepository {
    #[must_use] 
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a new room
    pub async fn create(&self, room: &Room) -> Result<Room> {
        let row = sqlx::query(
            "INSERT INTO rooms (id, name, description, created_by, status, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             RETURNING id, name, description, created_by, status, created_at, updated_at, deleted_at"
        )
        .bind(room.id.as_str())
        .bind(&room.name)
        .bind(&room.description)
        .bind(room.created_by.as_str())
        .bind(self.status_to_str(&room.status))
        .bind(room.created_at)
        .bind(room.updated_at)
        .fetch_one(&self.pool)
        .await?;

        self.row_to_room(row)
    }

    /// Get room by ID
    pub async fn get_by_id(&self, room_id: &RoomId) -> Result<Option<Room>> {
        let row = sqlx::query(
            "SELECT id, name, description, created_by, status, created_at, updated_at, deleted_at
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
        let row = sqlx::query(
            "UPDATE rooms
             SET name = $2, description = $3, status = $4, updated_at = $5
             WHERE id = $1 AND deleted_at IS NULL
             RETURNING id, name, description, created_by, status, created_at, updated_at, deleted_at"
        )
        .bind(room.id.as_str())
        .bind(&room.name)
        .bind(&room.description)
        .bind(self.status_to_str(&room.status))
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

        // Build WHERE conditions
        let mut where_conditions = vec!["r.deleted_at IS NULL"];

        let status_filter = match &query.status {
            Some(RoomStatus::Pending) => "r.status = 'pending'",
            Some(RoomStatus::Active) => "r.status = 'active'",
            Some(RoomStatus::Banned) => "r.status = 'banned'",
            None => "",
        };
        if !status_filter.is_empty() {
            where_conditions.push(status_filter);
        }

        let has_search = query.search.is_some();
        if has_search {
            where_conditions.push("(r.name ILIKE $3 OR r.description ILIKE $3)");
        }

        let where_clause = where_conditions.join(" AND ");

        // Get total count
        let count_query = format!("SELECT COUNT(*) as count FROM rooms r WHERE {where_clause}");

        let count: i64 = if let Some(ref search) = query.search {
            let search_pattern = format!("%{search}%");
            sqlx::query_scalar(&count_query)
                .bind(&search_pattern)
                .fetch_one(&self.pool)
                .await?
        } else {
            sqlx::query_scalar(&count_query)
                .fetch_one(&self.pool)
                .await?
        };

        // Get rooms
        let list_query = format!(
            "SELECT r.id, r.name, r.description, r.created_by, r.status, r.created_at, r.updated_at, r.deleted_at
             FROM rooms r
             WHERE {where_clause}
             ORDER BY r.created_at DESC
             LIMIT $1 OFFSET $2"
        );

        let rows = if let Some(ref search) = query.search {
            let search_pattern = format!("%{search}%");
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

        let rooms: Result<Vec<Room>> = rows.into_iter().map(|row| self.row_to_room(row)).collect();

        Ok((rooms?, count))
    }

    /// List rooms with member count (optimized with JOIN)
    pub async fn list_with_count(&self, query: &RoomListQuery) -> Result<(Vec<crate::models::RoomWithCount>, i64)> {
        let offset = (query.page - 1) * query.page_size;

        // Build WHERE conditions
        let mut where_conditions = vec!["r.deleted_at IS NULL"];

        // Dynamic query building for status filter
        let status_filter = match &query.status {
            Some(RoomStatus::Pending) => "r.status = 'pending'",
            Some(RoomStatus::Active) => "r.status = 'active'",
            Some(RoomStatus::Banned) => "r.status = 'banned'",
            None => "",
        };
        if !status_filter.is_empty() {
            where_conditions.push(status_filter);
        }

        // Search filter
        let has_search = query.search.is_some();
        if has_search {
            where_conditions.push("(r.name ILIKE $3 OR r.description ILIKE $3)");
        }

        let where_clause = where_conditions.join(" AND ");

        // Get total count
        let count_query = format!(
            "SELECT COUNT(DISTINCT r.id) FROM rooms r WHERE {where_clause}"
        );

        let count: i64 = if let Some(ref search) = query.search {
            let search_pattern = format!("%{search}%");
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
            r"
            SELECT
                r.id, r.name, r.description, r.created_by, r.status,
                r.created_at, r.updated_at, r.deleted_at,
                COALESCE(COUNT(rm.user_id) FILTER (WHERE rm.left_at IS NULL), 0)::int as member_count
            FROM rooms r
            LEFT JOIN room_members rm ON r.id = rm.room_id
            WHERE {where_clause}
            GROUP BY r.id, r.name, r.description, r.created_by, r.status, r.created_at, r.updated_at, r.deleted_at
            ORDER BY r.created_at DESC
            LIMIT $1 OFFSET $2
            "
        );

        let rows = if let Some(ref search) = query.search {
            let search_pattern = format!("%{search}%");
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

        // Get total count
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) as count
             FROM rooms
             WHERE created_by = $1 AND deleted_at IS NULL"
        )
        .bind(creator_id.as_str())
        .fetch_one(&self.pool)
        .await?;

        // Get rooms
        let rows = sqlx::query(
            "SELECT id, name, description, created_by, status, created_at, updated_at, deleted_at
             FROM rooms
             WHERE created_by = $1 AND deleted_at IS NULL
             ORDER BY created_at DESC
             LIMIT $2 OFFSET $3"
        )
        .bind(creator_id.as_str())
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

        // Get total count
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) as count
             FROM rooms
             WHERE created_by = $1 AND deleted_at IS NULL"
        )
        .bind(creator_id.as_str())
        .fetch_one(&self.pool)
        .await?;

        // Get rooms with member count using LEFT JOIN
        let rows = sqlx::query(
            r"
            SELECT
                r.id, r.name, r.description, r.created_by, r.status,
                r.created_at, r.updated_at, r.deleted_at,
                COALESCE(COUNT(rm.user_id) FILTER (WHERE rm.left_at IS NULL), 0)::int as member_count
            FROM rooms r
            LEFT JOIN room_members rm ON r.id = rm.room_id
            WHERE r.created_by = $1 AND r.deleted_at IS NULL
            GROUP BY r.id, r.name, r.description, r.created_by, r.status, r.created_at, r.updated_at, r.deleted_at
            ORDER BY r.created_at DESC
            LIMIT $2 OFFSET $3
            "
        )
        .bind(creator_id.as_str())
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

    /// Convert database row to Room model
    fn row_to_room(&self, row: PgRow) -> Result<Room> {
        let status_str: String = row.try_get("status")?;

        Ok(Room {
            id: RoomId::from_string(row.try_get("id")?),
            name: row.try_get("name")?,
            description: row.try_get("description")?,
            created_by: UserId::from_string(row.try_get("created_by")?),
            status: self.str_to_status(&status_str),
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            deleted_at: row.try_get("deleted_at")?,
        })
    }

    /// Update room status
    pub async fn update_status(&self, room_id: &RoomId, status: RoomStatus) -> Result<Room> {
        let status_str = self.status_to_str(&status);

        let row = sqlx::query(
            r"
            UPDATE rooms
            SET status = $1, updated_at = CURRENT_TIMESTAMP
            WHERE id = $2 AND deleted_at IS NULL
            RETURNING id, name, description, created_by, status, created_at, updated_at, deleted_at
            ",
        )
        .bind(status_str)
        .bind(room_id.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(Error::Database)?;

        self.row_to_room(row)
    }

    /// Update room description
    pub async fn update_description(&self, room_id: &RoomId, description: &str) -> Result<Room> {
        let row = sqlx::query(
            r"
            UPDATE rooms
            SET description = $1, updated_at = CURRENT_TIMESTAMP
            WHERE id = $2 AND deleted_at IS NULL
            RETURNING id, name, description, created_by, status, created_at, updated_at, deleted_at
            ",
        )
        .bind(description)
        .bind(room_id.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(Error::Database)?;

        self.row_to_room(row)
    }

    const fn status_to_str(&self, status: &RoomStatus) -> &'static str {
        match status {
            RoomStatus::Pending => "pending",
            RoomStatus::Active => "active",
            RoomStatus::Banned => "banned",
        }
    }

    fn str_to_status(&self, s: &str) -> RoomStatus {
        match s {
            "pending" => RoomStatus::Pending,
            "active" => RoomStatus::Active,
            "banned" => RoomStatus::Banned,
            _ => RoomStatus::Active,
        }
    }
}

#[cfg(test)]
mod tests {

    #[tokio::test]
    #[ignore = "Requires database"]
    async fn test_create_room() {
        // Integration test placeholder
    }
}
