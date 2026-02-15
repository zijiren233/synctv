//! User operations: get_profile, set_username, set_password

use synctv_core::models::UserId;

use super::ClientApiImpl;
use super::convert::user_to_proto;

impl ClientApiImpl {
    pub async fn get_profile(
        &self,
        user_id: &str,
    ) -> Result<crate::proto::client::GetProfileResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let user = self.user_service.get_user(&uid).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::GetProfileResponse {
            user: Some(user_to_proto(&user)),
        })
    }

    pub async fn set_username(
        &self,
        user_id: &str,
        req: crate::proto::client::SetUsernameRequest,
    ) -> Result<crate::proto::client::SetUsernameResponse, String> {
        let uid = UserId::from_string(user_id.to_string());
        let user = self.user_service.get_user(&uid).await
            .map_err(|e| e.to_string())?;

        let updated_user = synctv_core::models::User {
            username: req.new_username,
            ..user
        };

        let result_user = self.user_service.update_user(&updated_user).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::SetUsernameResponse {
            user: Some(user_to_proto(&result_user)),
        })
    }

    pub async fn set_password(
        &self,
        user_id: &str,
        req: crate::proto::client::SetPasswordRequest,
    ) -> Result<crate::proto::client::SetPasswordResponse, String> {
        let uid = UserId::from_string(user_id.to_string());

        // Verify old password before allowing change
        self.user_service.change_password(&uid, &req.old_password, &req.new_password).await
            .map_err(|e| e.to_string())?;

        Ok(crate::proto::client::SetPasswordResponse {
            success: true,
        })
    }
}
