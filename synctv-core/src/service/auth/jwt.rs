use chrono::{Duration, Utc};
use jsonwebtoken::{
    decode, encode, Algorithm, DecodingKey, EncodingKey, Header, TokenData, Validation,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::str::FromStr;

use crate::{models::{UserId, RoomId}, models::UserRole, Error, Result};

/// JWT token type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    Access,  // 1 hour
    Refresh, // 30 days
    Guest,   // 4 hours (for guest sessions)
}

/// JWT claims structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// User ID
    pub sub: String,
    /// User RBAC role (root, admin, user)
    pub role: String,
    /// Token type (access or refresh)
    pub typ: String,
    /// Issued at (Unix timestamp)
    pub iat: i64,
    /// Expiration time (Unix timestamp)
    pub exp: i64,
}

impl Claims {
    #[must_use]
    pub fn user_id(&self) -> UserId {
        UserId::from_string(self.sub.clone())
    }

    /// Parse role from string
    pub fn role(&self) -> Result<UserRole> {
        UserRole::from_str(&self.role)
            .map_err(|_| Error::Internal(format!("Invalid role in token: {}", self.role)))
    }

    #[must_use]
    pub fn is_access_token(&self) -> bool {
        self.typ == "access"
    }

    #[must_use]
    pub fn is_refresh_token(&self) -> bool {
        self.typ == "refresh"
    }

    #[must_use]
    pub fn is_guest_token(&self) -> bool {
        self.typ == "guest"
    }
}

/// Guest token claims structure (stateless guest authentication)
///
/// Guest tokens contain the room ID and a random session ID instead of a user ID.
/// Format: `guest:{room_id}:{session_id}`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuestClaims {
    /// Guest subject (format: "guest:{room_id}:{session_id}")
    pub sub: String,
    /// Room ID
    pub room_id: String,
    /// Random session ID for this guest
    pub session_id: String,
    /// Token type (always "guest")
    pub typ: String,
    /// Issued at (Unix timestamp)
    pub iat: i64,
    /// Expiration time (Unix timestamp)
    pub exp: i64,
}

impl GuestClaims {
    /// Parse room ID from claims
    #[must_use]
    pub fn room_id(&self) -> RoomId {
        RoomId::from_string(self.room_id.clone())
    }

    /// Get session ID
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Check if this is a guest token
    #[must_use]
    pub fn is_guest(&self) -> bool {
        self.sub.starts_with("guest:")
    }
}

/// JWT service for signing and verifying tokens
#[derive(Clone)]
pub struct JwtService {
    encoding_key: Arc<EncodingKey>,
    decoding_key: Arc<DecodingKey>,
    algorithm: Algorithm,
}

impl std::fmt::Debug for JwtService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwtService")
            .field("algorithm", &self.algorithm)
            .finish()
    }
}

impl JwtService {
    /// Create a new JWT service with HS256 secret
    ///
    /// # Arguments
    /// * `secret` - Secret string for HMAC signing
    pub fn new(secret: &str) -> Result<Self> {
        if secret.is_empty() {
            return Err(Error::Internal("JWT secret cannot be empty".to_string()));
        }

        let encoding_key = EncodingKey::from_secret(secret.as_bytes());
        let decoding_key = DecodingKey::from_secret(secret.as_bytes());

        Ok(Self {
            encoding_key: Arc::new(encoding_key),
            decoding_key: Arc::new(decoding_key),
            algorithm: Algorithm::HS256,
        })
    }

    /// Sign a token
    ///
    /// # Arguments
    /// * `user_id` - User ID
    /// * `role` - User RBAC role (root, admin, user)
    /// * `token_type` - Access, refresh, or guest token
    pub fn sign_token(
        &self,
        user_id: &UserId,
        role: UserRole,
        token_type: TokenType,
    ) -> Result<String> {
        let now = Utc::now();
        let duration = match token_type {
            TokenType::Access => Duration::hours(1),
            TokenType::Refresh => Duration::days(30),
            TokenType::Guest => Duration::hours(4),
        };

        let claims = Claims {
            sub: user_id.as_str().to_string(),
            role: role.as_str().to_string(),
            typ: match token_type {
                TokenType::Access => "access".to_string(),
                TokenType::Refresh => "refresh".to_string(),
                TokenType::Guest => "guest".to_string(),
            },
            iat: now.timestamp(),
            exp: (now + duration).timestamp(),
        };

        let header = Header::new(self.algorithm);
        encode(&header, &claims, &self.encoding_key)
            .map_err(|e| Error::Internal(format!("Failed to sign token: {e}")))
    }

    /// Verify a token and extract claims
    ///
    /// # Arguments
    /// * `token` - JWT token string
    pub fn verify_token(&self, token: &str) -> Result<Claims> {
        let mut validation = Validation::new(self.algorithm);
        validation.validate_exp = true;
        validation.validate_nbf = false;
        validation.leeway = 60; // 60 seconds leeway for clock skew

        let token_data: TokenData<Claims> = decode(token, &self.decoding_key, &validation)
            .map_err(|e| match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => {
                    Error::Authentication("Token expired".to_string())
                }
                jsonwebtoken::errors::ErrorKind::InvalidToken => {
                    Error::Authentication("Invalid token".to_string())
                }
                jsonwebtoken::errors::ErrorKind::InvalidSignature => {
                    Error::Authentication("Invalid token signature".to_string())
                }
                _ => Error::Authentication(format!("Token verification failed: {e}")),
            })?;

        Ok(token_data.claims)
    }

    /// Verify an access token (convenience method)
    pub fn verify_access_token(&self, token: &str) -> Result<Claims> {
        let claims = self.verify_token(token)?;
        if !claims.is_access_token() {
            return Err(Error::Authentication("Not an access token".to_string()));
        }
        Ok(claims)
    }

    /// Verify a refresh token (convenience method)
    pub fn verify_refresh_token(&self, token: &str) -> Result<Claims> {
        let claims = self.verify_token(token)?;
        if !claims.is_refresh_token() {
            return Err(Error::Authentication("Not a refresh token".to_string()));
        }
        Ok(claims)
    }

    /// Sign a guest token for stateless guest authentication
    ///
    /// Guest tokens do NOT store user information in the database.
    /// Instead, they contain the room ID and a random session ID.
    ///
    /// # Arguments
    /// * `room_id` - Room ID the guest is joining
    ///
    /// # Returns
    /// * Guest JWT token string
    pub fn sign_guest_token(&self, room_id: &RoomId) -> Result<String> {
        let now = Utc::now();
        let duration = Duration::hours(4); // Guest tokens expire after 4 hours
        let session_id = nanoid::nanoid!(16); // Generate random session ID

        let guest_claims = GuestClaims {
            sub: format!("guest:{}:{}", room_id.as_str(), session_id),
            room_id: room_id.as_str().to_string(),
            session_id,
            typ: "guest".to_string(),
            iat: now.timestamp(),
            exp: (now + duration).timestamp(),
        };

        let header = Header::new(self.algorithm);
        encode(&header, &guest_claims, &self.encoding_key)
            .map_err(|e| Error::Internal(format!("Failed to sign guest token: {e}")))
    }

    /// Verify a guest token and extract guest claims
    ///
    /// # Arguments
    /// * `token` - Guest JWT token string
    ///
    /// # Returns
    /// * Guest claims with room ID and session ID
    pub fn verify_guest_token(&self, token: &str) -> Result<GuestClaims> {
        let mut validation = Validation::new(self.algorithm);
        validation.validate_exp = true;
        validation.validate_nbf = false;
        validation.leeway = 60; // 60 seconds leeway for clock skew

        let token_data: TokenData<GuestClaims> = decode(token, &self.decoding_key, &validation)
            .map_err(|e| match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => {
                    Error::Authentication("Guest token expired".to_string())
                }
                jsonwebtoken::errors::ErrorKind::InvalidToken => {
                    Error::Authentication("Invalid guest token".to_string())
                }
                jsonwebtoken::errors::ErrorKind::InvalidSignature => {
                    Error::Authentication("Invalid guest token signature".to_string())
                }
                _ => Error::Authentication(format!("Guest token verification failed: {e}")),
            })?;

        let claims = token_data.claims;

        // Verify it's actually a guest token
        if !claims.is_guest() {
            return Err(Error::Authentication("Not a guest token".to_string()));
        }

        Ok(claims)
    }

    /// Check if a token string is a guest token (by attempting to parse it)
    ///
    /// # Arguments
    /// * `token` - JWT token string
    ///
    /// # Returns
    /// * `true` if token is a valid guest token, `false` otherwise
    pub fn is_guest_token(&self, token: &str) -> bool {
        self.verify_guest_token(token).is_ok()
    }

    /// Sign a custom JSON value as JWT
    ///
    /// This allows signing arbitrary claims (not just the standard Claims struct).
    /// Useful for RTMP publish keys and other custom tokens.
    ///
    /// # Arguments
    /// * `claims` - JSON value containing the claims
    ///
    /// # Returns
    /// Signed JWT token string
    pub async fn sign_custom(&self, claims: &serde_json::Value) -> Result<String> {
        let now = Utc::now();

        // Add standard JWT claims if not present
        let mut claims_with_standard = claims.clone();
        if let Some(obj) = claims_with_standard.as_object_mut() {
            obj.entry("iat".to_string())
                .or_insert_with(|| serde_json::Value::Number(now.timestamp().into()));

            if !obj.contains_key("exp") {
                obj.entry("exp".to_string())
                    .or_insert_with(|| serde_json::Value::Number((now.timestamp() + 86400).into())); // Default 24h
            }
        }

        let header = Header::new(self.algorithm);
        encode(&header, &claims_with_standard, &self.encoding_key)
            .map_err(|e| Error::Internal(format!("Failed to sign custom token: {e}")))
    }

    /// Verify a custom JWT token
    ///
    /// This allows verifying tokens with arbitrary claims.
    ///
    /// # Arguments
    /// * `token` - JWT token string
    ///
    /// # Returns
    /// JSON value containing the claims
    pub async fn verify_custom(&self, token: &str) -> Result<serde_json::Value> {
        let mut validation = Validation::new(self.algorithm);
        validation.validate_exp = true;
        validation.validate_nbf = false;
        validation.leeway = 60; // 60 seconds leeway for clock skew

        let token_data = decode(token, &self.decoding_key, &validation)
            .map_err(|e| match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => {
                    Error::Authentication("Token expired".to_string())
                }
                jsonwebtoken::errors::ErrorKind::InvalidToken => {
                    Error::Authentication("Invalid token".to_string())
                }
                jsonwebtoken::errors::ErrorKind::InvalidSignature => {
                    Error::Authentication("Invalid token signature".to_string())
                }
                _ => Error::Authentication(format!("Token verification failed: {e}")),
            })?;

        Ok(token_data.claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_jwt_service() -> JwtService {
        JwtService::new("test-secret-key-for-jwt").unwrap()
    }

    #[test]
    fn test_sign_and_verify_access_token() {
        let jwt = create_jwt_service();
        let user_id = UserId::new();
        let role = UserRole::Admin;

        let token = jwt.sign_token(&user_id, role, TokenType::Access).unwrap();
        let claims = jwt.verify_access_token(&token).unwrap();

        assert_eq!(claims.sub, user_id.as_str());
        assert_eq!(claims.role().unwrap(), UserRole::Admin);
        assert!(claims.is_access_token());
    }

    #[test]
    fn test_sign_and_verify_refresh_token() {
        let jwt = create_jwt_service();
        let user_id = UserId::new();
        let role = UserRole::User;

        let token = jwt.sign_token(&user_id, role, TokenType::Refresh).unwrap();
        let claims = jwt.verify_refresh_token(&token).unwrap();

        assert_eq!(claims.sub, user_id.as_str());
        assert_eq!(claims.role().unwrap(), UserRole::User);
        assert!(claims.is_refresh_token());
    }

    #[test]
    fn test_verify_wrong_token_type() {
        let jwt = create_jwt_service();
        let user_id = UserId::new();

        let access_token = jwt.sign_token(&user_id, UserRole::User, TokenType::Access).unwrap();
        let result = jwt.verify_refresh_token(&access_token);
        assert!(result.is_err());

        let refresh_token = jwt.sign_token(&user_id, UserRole::User, TokenType::Refresh).unwrap();
        let result = jwt.verify_access_token(&refresh_token);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_token() {
        let jwt = create_jwt_service();
        let result = jwt.verify_token("invalid.token.here");
        assert!(result.is_err());
    }

    #[test]
    fn test_tampered_token() {
        let jwt = create_jwt_service();
        let user_id = UserId::new();

        let token = jwt.sign_token(&user_id, UserRole::User, TokenType::Access).unwrap();
        let mut parts: Vec<&str> = token.split('.').collect();
        parts[1] = "tampered_payload";
        let tampered_token = parts.join(".");

        let result = jwt.verify_token(&tampered_token);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_secret() {
        let result = JwtService::new("");
        assert!(result.is_err());
    }

    #[test]
    fn test_sign_and_verify_guest_token() {
        let jwt = create_jwt_service();
        let room_id = RoomId::new();

        let token = jwt.sign_guest_token(&room_id).unwrap();
        let claims = jwt.verify_guest_token(&token).unwrap();

        assert_eq!(claims.room_id(), room_id);
        assert!(claims.is_guest());
        assert_eq!(claims.typ, "guest");
        assert!(!claims.session_id().is_empty());
        assert!(claims.sub.starts_with("guest:"));
    }

    #[test]
    fn test_guest_token_contains_session_id() {
        let jwt = create_jwt_service();
        let room_id = RoomId::new();

        let token1 = jwt.sign_guest_token(&room_id).unwrap();
        let token2 = jwt.sign_guest_token(&room_id).unwrap();

        let claims1 = jwt.verify_guest_token(&token1).unwrap();
        let claims2 = jwt.verify_guest_token(&token2).unwrap();

        // Each guest token should have a unique session ID
        assert_ne!(claims1.session_id(), claims2.session_id());
    }

    #[test]
    fn test_is_guest_token() {
        let jwt = create_jwt_service();
        let room_id = RoomId::new();

        let guest_token = jwt.sign_guest_token(&room_id).unwrap();
        assert!(jwt.is_guest_token(&guest_token));

        let user_id = UserId::new();
        let access_token = jwt.sign_token(&user_id, UserRole::User, TokenType::Access).unwrap();
        assert!(!jwt.is_guest_token(&access_token));
    }

    #[test]
    fn test_verify_regular_token_as_guest_fails() {
        let jwt = create_jwt_service();
        let user_id = UserId::new();

        let access_token = jwt.sign_token(&user_id, UserRole::User, TokenType::Access).unwrap();
        let result = jwt.verify_guest_token(&access_token);
        assert!(result.is_err());
    }
}
