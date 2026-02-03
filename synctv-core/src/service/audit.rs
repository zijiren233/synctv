//! Audit logging service
//!
//! Tracks admin actions and permission changes for compliance and debugging.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::Result;

/// Audit log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLog {
    pub id: String,
    pub actor_id: String,
    pub actor_username: String,
    pub action: AuditAction,
    pub target_type: AuditTargetType,
    pub target_id: Option<String>,
    pub details: serde_json::Value,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Parameters for logging an audit event
#[derive(Debug, Clone)]
pub struct AuditEventParams {
    pub actor_id: String,
    pub actor_username: String,
    pub action: AuditAction,
    pub target_type: AuditTargetType,
    pub target_id: Option<String>,
    pub details: serde_json::Value,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

/// Audit actions
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    UserCreated,
    UserDeleted,
    UserBanned,
    UserUnbanned,
    UserPasswordUpdated,
    UserUsernameUpdated,
    UserRoleUpdated,
    RoomCreated,
    RoomDeleted,
    RoomBanned,
    RoomUnbanned,
    RoomPasswordUpdated,
    PermissionGranted,
    PermissionRevoked,
    ProviderInstanceCreated,
    ProviderInstanceUpdated,
    ProviderInstanceDeleted,
    SettingsUpdated,
}

/// Target types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditTargetType {
    User,
    Room,
    ProviderInstance,
    Settings,
    System,
}

/// Audit logging service
///
/// Records audit logs for security-relevant actions.
pub struct AuditService {
    pool: PgPool,
}

impl AuditService {
    /// Create a new audit service
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Log an audit event
    #[allow(clippy::too_many_arguments)]
    pub async fn log(
        &self,
        actor_id: String,
        actor_username: String,
        action: AuditAction,
        target_type: AuditTargetType,
        target_id: Option<String>,
        details: serde_json::Value,
        ip_address: Option<String>,
        user_agent: Option<String>,
    ) -> Result<()> {
        let audit_log = AuditLog {
            id: nanoid::nanoid!(12),
            actor_id,
            actor_username,
            action,
            target_type,
            target_id,
            details,
            ip_address,
            user_agent,
            created_at: Utc::now(),
        };

        // Insert into database
        // Note: This assumes an audit_logs table exists
        // In production, you would create the table with migrations
        let query = r#"
            INSERT INTO audit_logs (
                id, actor_id, actor_username, action, target_type, target_id,
                details, ip_address, user_agent, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#;

        let action_str = serde_json::to_string(&audit_log.action)?;
        let target_str = serde_json::to_string(&audit_log.target_type)?;
        let details_str = serde_json::to_string(&audit_log.details)?;

        sqlx::query(query)
            .bind(&audit_log.id)
            .bind(&audit_log.actor_id)
            .bind(&audit_log.actor_username)
            .bind(&action_str)
            .bind(&target_str)
            .bind(&audit_log.target_id)
            .bind(&details_str)
            .bind(&audit_log.ip_address)
            .bind(&audit_log.user_agent)
            .bind(audit_log.created_at)
            .execute(&self.pool)
            .await?;

        tracing::debug!(
            actor_id = %audit_log.actor_id,
            action = %action_str,
            target_type = %target_str,
            "Audit log recorded"
        );

        Ok(())
    }

    /// Log an audit event with parameters struct
    pub async fn log_with_params(&self, params: AuditEventParams) -> Result<()> {
        self.log(
            params.actor_id,
            params.actor_username,
            params.action,
            params.target_type,
            params.target_id,
            params.details,
            params.ip_address,
            params.user_agent,
        )
        .await
    }

    /// Log user creation
    pub async fn log_user_created(
        &self,
        actor_id: String,
        actor_username: String,
        target_user_id: String,
    ) -> Result<()> {
        self.log(
            actor_id,
            actor_username,
            AuditAction::UserCreated,
            AuditTargetType::User,
            Some(target_user_id),
            serde_json::json!({"reason": "User created via admin panel"}),
            None,
            None,
        )
        .await
    }

    /// Log user ban
    pub async fn log_user_banned(
        &self,
        actor_id: String,
        actor_username: String,
        target_user_id: String,
    ) -> Result<()> {
        self.log(
            actor_id,
            actor_username,
            AuditAction::UserBanned,
            AuditTargetType::User,
            Some(target_user_id),
            serde_json::json!({"reason": "User banned by admin"}),
            None,
            None,
        )
        .await
    }

    /// Log permission change
    pub async fn log_permission_changed(
        &self,
        actor_id: String,
        actor_username: String,
        target_type: AuditTargetType,
        target_id: String,
        old_permissions: u64,
        new_permissions: u64,
    ) -> Result<()> {
        self.log(
            actor_id,
            actor_username,
            AuditAction::PermissionGranted,
            target_type,
            Some(target_id),
            serde_json::json!({
                "old_permissions": old_permissions,
                "new_permissions": new_permissions
            }),
            None,
            None,
        )
        .await
    }

    /// Log room deletion
    pub async fn log_room_deleted(
        &self,
        actor_id: String,
        actor_username: String,
        room_id: String,
    ) -> Result<()> {
        self.log(
            actor_id,
            actor_username,
            AuditAction::RoomDeleted,
            AuditTargetType::Room,
            Some(room_id),
            serde_json::json!({"reason": "Room deleted by admin"}),
            None,
            None,
        )
        .await
    }
}

impl std::fmt::Debug for AuditService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditService").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_action_serialization() {
        let action = AuditAction::UserCreated;
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("user_created"));
    }
}
