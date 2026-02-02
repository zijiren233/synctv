use sqlx::{PgPool, postgres::PgRow, Row};

use crate::{
    models::{
        RoomMember, RoomMemberWithUser, RoomId, UserId, RoomRole, MemberStatus,
        RoomSettings,
    },
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

    /// Add user to room with role
    pub async fn add(&self, member: &RoomMember) -> Result<RoomMember> {
        let row = sqlx::query(
            "INSERT INTO room_members (
                room_id, user_id, role, status,
                added_permissions, removed_permissions,
                joined_at, version
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (room_id, user_id) DO UPDATE
             SET
                role = EXCLUDED.role,
                status = EXCLUDED.status,
                added_permissions = EXCLUDED.added_permissions,
                removed_permissions = EXCLUDED.removed_permissions,
                left_at = NULL,
                joined_at = EXCLUDED.joined_at,
                version = room_members.version + 1
             RETURNING
                room_id, user_id, role, status,
                added_permissions, removed_permissions,
                joined_at, left_at, version,
                banned_at, banned_by, banned_reason"
        )
        .bind(member.room_id.as_str())
        .bind(member.user_id.as_str())
        .bind(member.role.to_string())
        .bind(member.status.to_string())
        .bind(member.added_permissions)
        .bind(member.removed_permissions)
        .bind(member.joined_at)
        .bind(member.version)
        .fetch_one(&self.pool)
        .await?;

        self.row_to_member(row)
    }

    /// Remove user from room (soft delete - set left_at)
    pub async fn remove(&self, room_id: &RoomId, user_id: &UserId) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE room_members
             SET left_at = $3, version = version + 1
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
            "SELECT
                room_id, user_id, role, status,
                added_permissions, removed_permissions,
                joined_at, left_at, version,
                banned_at, banned_by, banned_reason
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

    /// Get member by ID (including banned/inactive)
    pub async fn get_any(&self, room_id: &RoomId, user_id: &UserId) -> Result<Option<RoomMember>> {
        let row = sqlx::query(
            "SELECT
                room_id, user_id, role, status,
                added_permissions, removed_permissions,
                joined_at, left_at, version,
                banned_at, banned_by, banned_reason
             FROM room_members
             WHERE room_id = $1 AND user_id = $2"
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
            "SELECT
                rm.room_id, rm.user_id, rm.role, rm.status,
                rm.added_permissions, rm.removed_permissions,
                rm.joined_at, rm.banned_at, rm.banned_reason,
                u.username
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

    /// List all active members in a room with online status
    pub async fn list_by_room_with_online(
        &self,
        room_id: &RoomId,
        online_user_ids: &[UserId],
    ) -> Result<Vec<RoomMemberWithUser>> {
        let rows = sqlx::query(
            "SELECT
                rm.room_id, rm.user_id, rm.role, rm.status,
                rm.added_permissions, rm.removed_permissions,
                rm.joined_at, rm.banned_at, rm.banned_reason,
                u.username
             FROM room_members rm
             JOIN users u ON rm.user_id = u.id
             WHERE rm.room_id = $1 AND rm.left_at IS NULL
             ORDER BY rm.joined_at ASC"
        )
        .bind(room_id.as_str())
        .fetch_all(&self.pool)
        .await?;

        let online_set: std::collections::HashSet<_> =
            online_user_ids.iter().map(|id| id.as_str()).collect();

        rows.into_iter()
            .map(|row| {
                let mut member = self.row_to_member_with_user(row)?;
                member.is_online = online_set.contains(member.user_id.as_str());
                Ok(member)
            })
            .collect()
    }

    /// Update member role with optimistic locking
    pub async fn update_role(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        role: RoomRole,
        current_version: i64,
    ) -> Result<RoomMember> {
        let row = sqlx::query(
            "UPDATE room_members
             SET
                role = $3,
                version = version + 1
             WHERE room_id = $1 AND user_id = $2 AND version = $4
             RETURNING
                room_id, user_id, role, status,
                added_permissions, removed_permissions,
                joined_at, left_at, version,
                banned_at, banned_by, banned_reason"
        )
        .bind(room_id.as_str())
        .bind(user_id.as_str())
        .bind(role.to_string())
        .bind(current_version)
        .fetch_one(&self.pool)
        .await?;

        self.row_to_member(row)
    }

    /// Update member status with optimistic locking
    pub async fn update_status(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        status: MemberStatus,
        current_version: i64,
    ) -> Result<RoomMember> {
        let row = sqlx::query(
            "UPDATE room_members
             SET
                status = $3,
                version = version + 1
             WHERE room_id = $1 AND user_id = $2 AND version = $4
             RETURNING
                room_id, user_id, role, status,
                added_permissions, removed_permissions,
                joined_at, left_at, version,
                banned_at, banned_by, banned_reason"
        )
        .bind(room_id.as_str())
        .bind(user_id.as_str())
        .bind(status.to_string())
        .bind(current_version)
        .fetch_one(&self.pool)
        .await?;

        self.row_to_member(row)
    }

    /// Update member Allow/Deny permissions with optimistic locking
    pub async fn update_permissions(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        added_permissions: Option<i64>,
        removed_permissions: Option<i64>,
        current_version: i64,
    ) -> Result<RoomMember> {
        let row = sqlx::query(
            "UPDATE room_members
             SET
                added_permissions = $3,
                removed_permissions = $4,
                version = version + 1
             WHERE room_id = $1 AND user_id = $2 AND version = $5
             RETURNING
                room_id, user_id, role, status,
                added_permissions, removed_permissions,
                joined_at, left_at, version,
                banned_at, banned_by, banned_reason"
        )
        .bind(room_id.as_str())
        .bind(user_id.as_str())
        .bind(added_permissions)
        .bind(removed_permissions)
        .bind(current_version)
        .fetch_one(&self.pool)
        .await?;

        self.row_to_member(row)
    }

    /// Reset member permissions to role default (clear added/removed)
    pub async fn reset_permissions(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        current_version: i64,
    ) -> Result<RoomMember> {
        let row = sqlx::query(
            "UPDATE room_members
             SET
                added_permissions = NULL,
                removed_permissions = NULL,
                version = version + 1
             WHERE room_id = $1 AND user_id = $2 AND version = $3
             RETURNING
                room_id, user_id, role, status,
                added_permissions, removed_permissions,
                joined_at, left_at, version,
                banned_at, banned_by, banned_reason"
        )
        .bind(room_id.as_str())
        .bind(user_id.as_str())
        .bind(current_version)
        .fetch_one(&self.pool)
        .await?;

        self.row_to_member(row)
    }

    /// Ban member from room
    pub async fn ban_member(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        banned_by: &UserId,
        reason: Option<String>,
    ) -> Result<RoomMember> {
        let row = sqlx::query(
            "UPDATE room_members
             SET
                status = 'banned',
                banned_at = $3,
                banned_by = $4,
                banned_reason = $5,
                version = version + 1
             WHERE room_id = $1 AND user_id = $2 AND left_at IS NULL
             RETURNING
                room_id, user_id, role, status,
                added_permissions, removed_permissions,
                joined_at, left_at, version,
                banned_at, banned_by, banned_reason"
        )
        .bind(room_id.as_str())
        .bind(user_id.as_str())
        .bind(chrono::Utc::now())
        .bind(banned_by.as_str())
        .bind(reason)
        .fetch_one(&self.pool)
        .await?;

        self.row_to_member(row)
    }

    /// Unban member from room
    pub async fn unban_member(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
    ) -> Result<RoomMember> {
        let row = sqlx::query(
            "UPDATE room_members
             SET
                status = 'active',
                banned_at = NULL,
                banned_by = NULL,
                banned_reason = NULL,
                version = version + 1
             WHERE room_id = $1 AND user_id = $2 AND left_at IS NULL
             RETURNING
                room_id, user_id, role, status,
                added_permissions, removed_permissions,
                joined_at, left_at, version,
                banned_at, banned_by, banned_reason"
        )
        .bind(room_id.as_str())
        .bind(user_id.as_str())
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
    /// Returns (room, role, status, member_count) tuples
    pub async fn list_by_user_with_details(
        &self,
        user_id: &UserId,
        page: i64,
        page_size: i64,
    ) -> Result<(Vec<(crate::models::Room, RoomRole, MemberStatus, i32)>, i64)> {
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

        // Get rooms with user role and member count in single query
        let rows = sqlx::query(
            r#"
            SELECT
                r.id, r.name, r.created_by, r.status, r.settings,
                r.created_at, r.updated_at, r.deleted_at,
                rm.role as user_role,
                rm.status as user_status,
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

        let results: Result<Vec<(crate::models::Room, RoomRole, MemberStatus, i32)>> = rows
            .into_iter()
            .map(|row| {
                let settings_json: serde_json::Value = row.try_get("settings")?;
                let status_str: String = row.try_get("status")?;
                let status = match status_str.as_str() {
                    "active" => crate::models::RoomStatus::Active,
                    "pending" => crate::models::RoomStatus::Pending,
                    "banned" => crate::models::RoomStatus::Banned,
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

                let role_str: String = row.try_get("user_role")?;
                let role = match role_str.as_str() {
                    "creator" => RoomRole::Creator,
                    "admin" => RoomRole::Admin,
                    "member" => RoomRole::Member,
                    "guest" => RoomRole::Guest,
                    _ => RoomRole::Member,
                };

                let member_status_str: String = row.try_get("user_status")?;
                let member_status = match member_status_str.as_str() {
                    "active" => MemberStatus::Active,
                    "pending" => MemberStatus::Pending,
                    "banned" => MemberStatus::Banned,
                    _ => MemberStatus::Active,
                };

                let member_count: i32 = row.try_get("member_count")?;

                Ok((room, role, member_status, member_count))
            })
            .collect();

        Ok((results?, count))
    }

    /// List all members including inactive (left) (admin view)
    pub async fn list_by_room_all(&self, room_id: &RoomId) -> Result<Vec<RoomMemberWithUser>> {
        let rows = sqlx::query(
            "SELECT
                rm.room_id, rm.user_id, rm.role, rm.status,
                rm.added_permissions, rm.removed_permissions,
                rm.joined_at, rm.banned_at, rm.banned_reason,
                u.username,
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
            .map(|row| {
                let is_active: bool = row.try_get("is_active")?;
                let mut member = self.row_to_member_with_user(row)?;
                member.is_online = is_active;
                Ok(member)
            })
            .collect()
    }

    /// Convert database row to RoomMember
    fn row_to_member(&self, row: PgRow) -> Result<RoomMember> {
        let role_str: String = row.try_get("role")?;
        let role = match role_str.as_str() {
            "creator" => RoomRole::Creator,
            "admin" => RoomRole::Admin,
            "member" => RoomRole::Member,
            "guest" => RoomRole::Guest,
            _ => return Err(Error::InvalidInput(format!("Unknown role: {}", role_str))),
        };

        let status_str: String = row.try_get("status")?;
        let status = match status_str.as_str() {
            "active" => MemberStatus::Active,
            "pending" => MemberStatus::Pending,
            "banned" => MemberStatus::Banned,
            _ => return Err(Error::InvalidInput(format!("Unknown status: {}", status_str))),
        };

        let banned_by: Option<String> = row.try_get("banned_by")?;

        Ok(RoomMember {
            room_id: RoomId::from_string(row.try_get("room_id")?),
            user_id: UserId::from_string(row.try_get("user_id")?),
            role,
            status,
            added_permissions: row.try_get("added_permissions")?,
            removed_permissions: row.try_get("removed_permissions")?,
            joined_at: row.try_get("joined_at")?,
            left_at: row.try_get("left_at")?,
            version: row.try_get("version")?,
            banned_at: row.try_get("banned_at")?,
            banned_by: banned_by.map(UserId::from_string),
            banned_reason: row.try_get("banned_reason")?,
        })
    }

    /// Convert database row to RoomMemberWithUser
    fn row_to_member_with_user(&self, row: PgRow) -> Result<RoomMemberWithUser> {
        let role_str: String = row.try_get("role")?;
        let role = match role_str.as_str() {
            "creator" => RoomRole::Creator,
            "admin" => RoomRole::Admin,
            "member" => RoomRole::Member,
            "guest" => RoomRole::Guest,
            _ => return Err(Error::InvalidInput(format!("Unknown role: {}", role_str))),
        };

        let status_str: String = row.try_get("status")?;
        let status = match status_str.as_str() {
            "active" => MemberStatus::Active,
            "pending" => MemberStatus::Pending,
            "banned" => MemberStatus::Banned,
            _ => return Err(Error::InvalidInput(format!("Unknown status: {}", status_str))),
        };

        Ok(RoomMemberWithUser {
            room_id: RoomId::from_string(row.try_get("room_id")?),
            user_id: UserId::from_string(row.try_get("user_id")?),
            username: row.try_get("username")?,
            role,
            status,
            added_permissions: row.try_get("added_permissions")?,
            removed_permissions: row.try_get("removed_permissions")?,
            joined_at: row.try_get("joined_at")?,
            is_online: false, // Will be populated by connection tracking
            banned_at: row.try_get("banned_at")?,
            banned_reason: row.try_get("banned_reason")?,
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
