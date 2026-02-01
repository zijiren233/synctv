use sqlx::{PgPool, postgres::PgRow, Row};

use crate::{
    models::{RoomMember, RoomMemberWithUser, RoomId, UserId, PermissionBits},
    Error, Result,
};

/// Room member repository for database operations
#[derive(Clone)]
pub struct RoomMemberRepository {
    pool: PgPool,
}

impl RoomMemberRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Add user to room
    pub async fn add(&self, member: &RoomMember) -> Result<RoomMember> {
        let row = sqlx::query(
            "INSERT INTO room_members (room_id, user_id, permissions, joined_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (room_id, user_id) DO UPDATE
             SET left_at = NULL, joined_at = $4
             RETURNING room_id, user_id, permissions, joined_at, left_at"
        )
        .bind(member.room_id.as_str())
        .bind(member.user_id.as_str())
        .bind(member.permissions.0)
        .bind(member.joined_at)
        .fetch_one(&self.pool)
        .await?;

        self.row_to_member(row)
    }

    /// Remove user from room (soft delete - set left_at)
    pub async fn remove(&self, room_id: &RoomId, user_id: &UserId) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE room_members
             SET left_at = $3
             WHERE room_id = $1 AND user_id = $2 AND left_at IS NULL"
        )
        .bind(room_id.as_str())
        .bind(user_id.as_str())
        .bind(chrono::Utc::now())
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Get member by room and user
    pub async fn get(&self, room_id: &RoomId, user_id: &UserId) -> Result<Option<RoomMember>> {
        let row = sqlx::query(
            "SELECT room_id, user_id, permissions, joined_at, left_at
             FROM room_members
             WHERE room_id = $1 AND user_id = $2 AND left_at IS NULL"
        )
        .bind(room_id.as_str())
        .bind(user_id.as_str())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_member(row)?)),
            None => Ok(None),
        }
    }

    /// List all active members in a room
    pub async fn list_by_room(&self, room_id: &RoomId) -> Result<Vec<RoomMemberWithUser>> {
        let rows = sqlx::query(
            "SELECT rm.room_id, rm.user_id, rm.permissions, rm.joined_at, u.username
             FROM room_members rm
             JOIN users u ON rm.user_id = u.id
             WHERE rm.room_id = $1 AND rm.left_at IS NULL
             ORDER BY rm.joined_at ASC"
        )
        .bind(room_id.as_str())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| self.row_to_member_with_user(row))
            .collect()
    }

    /// Update member permissions
    pub async fn update_permissions(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        permissions: PermissionBits,
    ) -> Result<RoomMember> {
        let row = sqlx::query(
            "UPDATE room_members
             SET permissions = $3
             WHERE room_id = $1 AND user_id = $2 AND left_at IS NULL
             RETURNING room_id, user_id, permissions, joined_at, left_at"
        )
        .bind(room_id.as_str())
        .bind(user_id.as_str())
        .bind(permissions.0)
        .fetch_one(&self.pool)
        .await?;

        self.row_to_member(row)
    }

    /// Check if user is member of room
    pub async fn is_member(&self, room_id: &RoomId, user_id: &UserId) -> Result<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) as count
             FROM room_members
             WHERE room_id = $1 AND user_id = $2 AND left_at IS NULL"
        )
        .bind(room_id.as_str())
        .bind(user_id.as_str())
        .fetch_one(&self.pool)
        .await?;

        Ok(count > 0)
    }

    /// Get member count for room
    pub async fn count_by_room(&self, room_id: &RoomId) -> Result<i32> {
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

    /// Get rooms where a user is a member
    pub async fn list_by_user(&self, user_id: &UserId, page: i64, page_size: i64) -> Result<(Vec<RoomId>, i64)> {
        let offset = (page - 1) * page_size;

        // Get total count
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) as count
             FROM room_members rm
             JOIN rooms r ON rm.room_id = r.id
             WHERE rm.user_id = $1 AND rm.left_at IS NULL AND r.deleted_at IS NULL"
        )
        .bind(user_id.as_str())
        .fetch_one(&self.pool)
        .await?;

        // Get room IDs
        let rows = sqlx::query_scalar::<_, String>(
            "SELECT rm.room_id
             FROM room_members rm
             JOIN rooms r ON rm.room_id = r.id
             WHERE rm.user_id = $1 AND rm.left_at IS NULL AND r.deleted_at IS NULL
             ORDER BY rm.joined_at DESC
             LIMIT $2 OFFSET $3"
        )
        .bind(user_id.as_str())
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let room_ids = rows.into_iter().map(RoomId::from_string).collect();

        Ok((room_ids, count))
    }

    /// Get rooms where a user is a member with full room details and member count (optimized)
    /// Returns (room, user_permissions, member_count) tuples
    pub async fn list_by_user_with_details(
        &self,
        user_id: &UserId,
        page: i64,
        page_size: i64,
    ) -> Result<(Vec<(crate::models::Room, PermissionBits, i32)>, i64)> {
        let offset = (page - 1) * page_size;

        // Get total count
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) as count
             FROM room_members rm
             JOIN rooms r ON rm.room_id = r.id
             WHERE rm.user_id = $1 AND rm.left_at IS NULL AND r.deleted_at IS NULL"
        )
        .bind(user_id.as_str())
        .fetch_one(&self.pool)
        .await?;

        // Get rooms with user permissions and member count in single query
        let rows = sqlx::query(
            r#"
            SELECT
                r.id, r.name, r.created_by, r.status, r.settings,
                r.created_at, r.updated_at, r.deleted_at,
                rm.permissions as user_permissions,
                (
                    SELECT COUNT(*)::int
                    FROM room_members rm2
                    WHERE rm2.room_id = r.id AND rm2.left_at IS NULL
                ) as member_count
            FROM room_members rm
            JOIN rooms r ON rm.room_id = r.id
            WHERE rm.user_id = $1 AND rm.left_at IS NULL AND r.deleted_at IS NULL
            ORDER BY rm.joined_at DESC
            LIMIT $2 OFFSET $3
            "#
        )
        .bind(user_id.as_str())
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let results: Result<Vec<(crate::models::Room, PermissionBits, i32)>> = rows
            .into_iter()
            .map(|row| {
                let settings_json: serde_json::Value = row.try_get("settings")?;
                let status_str: String = row.try_get("status")?;
                let status = match status_str.as_str() {
                    "active" => crate::models::RoomStatus::Active,
                    "closed" => crate::models::RoomStatus::Closed,
                    _ => crate::models::RoomStatus::Active,
                };

                let room = crate::models::Room {
                    id: RoomId::from_string(row.try_get("id")?),
                    name: row.try_get("name")?,
                    created_by: UserId::from_string(row.try_get("created_by")?),
                    status,
                    settings: settings_json,
                    created_at: row.try_get("created_at")?,
                    updated_at: row.try_get("updated_at")?,
                    deleted_at: row.try_get("deleted_at")?,
                };

                let permissions = PermissionBits::new(row.try_get("user_permissions")?);
                let member_count: i32 = row.try_get("member_count")?;

                Ok((room, permissions, member_count))
            })
            .collect();

        Ok((results?, count))
    }

    /// List all members including inactive (left) (admin view)
    pub async fn list_by_room_all(&self, room_id: &RoomId) -> Result<Vec<RoomMemberWithUser>> {
        let rows = sqlx::query(
            "SELECT rm.room_id, rm.user_id, rm.permissions, rm.joined_at, u.username,
                   CASE WHEN rm.left_at IS NULL THEN true ELSE false END as is_active
             FROM room_members rm
             JOIN users u ON rm.user_id = u.id
             WHERE rm.room_id = $1
             ORDER BY rm.joined_at ASC"
        )
        .bind(room_id.as_str())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| self.row_to_member_with_user_all(row))
            .collect()
    }

    /// Update member role
    pub async fn update_role(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        role: crate::models::Role,
    ) -> Result<RoomMember> {
        // Get current member to preserve joined_at
        let member = self.get(room_id, user_id).await?
            .ok_or_else(|| Error::NotFound("Member not found".to_string()))?;

        // Update with new role permissions
        let updated_member = RoomMember {
            room_id: room_id.clone(),
            user_id: user_id.clone(),
            permissions: role.permissions(),
            joined_at: member.joined_at,
            left_at: member.left_at,
        };

        // Delete and re-add member (simple way to update all fields)
        sqlx::query(
            "DELETE FROM room_members WHERE room_id = $1 AND user_id = $2"
        )
        .bind(room_id.as_str())
        .bind(user_id.as_str())
        .execute(&self.pool)
        .await?;

        self.add(&updated_member).await
    }

    /// Convert database row to RoomMember
    fn row_to_member(&self, row: PgRow) -> Result<RoomMember> {
        Ok(RoomMember {
            room_id: RoomId::from_string(row.try_get("room_id")?),
            user_id: UserId::from_string(row.try_get("user_id")?),
            permissions: PermissionBits::new(row.try_get("permissions")?),
            joined_at: row.try_get("joined_at")?,
            left_at: row.try_get("left_at")?,
        })
    }

    /// Convert database row to RoomMemberWithUser
    fn row_to_member_with_user(&self, row: PgRow) -> Result<RoomMemberWithUser> {
        Ok(RoomMemberWithUser {
            room_id: RoomId::from_string(row.try_get("room_id")?),
            user_id: UserId::from_string(row.try_get("user_id")?),
            username: row.try_get("username")?,
            permissions: PermissionBits::new(row.try_get("permissions")?),
            joined_at: row.try_get("joined_at")?,
            is_online: false, // Will be populated by connection tracking
        })
    }

    /// Convert database row to RoomMemberWithUser (including inactive)
    fn row_to_member_with_user_all(&self, row: PgRow) -> Result<RoomMemberWithUser> {
        let is_active: bool = row.try_get("is_active")?;
        Ok(RoomMemberWithUser {
            room_id: RoomId::from_string(row.try_get("room_id")?),
            user_id: UserId::from_string(row.try_get("user_id")?),
            username: row.try_get("username")?,
            permissions: PermissionBits::new(row.try_get("permissions")?),
            joined_at: row.try_get("joined_at")?,
            is_online: is_active, // Use is_active flag from query
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Requires database"]
    async fn test_add_member() {
        // Integration test placeholder
    }
}
