use chrono::{Duration, Utc};
use jsonwebtoken::{
    decode, encode, Algorithm, DecodingKey, EncodingKey, Header, TokenData, Validation,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{models::UserId, Error, Result};

/// JWT token type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    Access,  // 1 hour
    Refresh, // 30 days
}

/// JWT claims structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// User ID
    pub sub: String,
    /// User permissions (64-bit bitmask)
    pub permissions: i64,
    /// Token type (access or refresh)
    pub typ: String,
    /// Issued at (Unix timestamp)
    pub iat: i64,
    /// Expiration time (Unix timestamp)
    pub exp: i64,
}

impl Claims {
    pub fn user_id(&self) -> UserId {
        UserId::from_string(self.sub.clone())
    }

    pub fn is_access_token(&self) -> bool {
        self.typ == "access"
    }

    pub fn is_refresh_token(&self) -> bool {
        self.typ == "refresh"
    }
}

/// JWT service for signing and verifying tokens
#[derive(Clone)]
pub struct JwtService {
    encoding_key: Arc<EncodingKey>,
    decoding_key: Arc<DecodingKey>,
    algorithm: Algorithm,
}

impl JwtService {
    /// Create a new JWT service with RS256 keys
    ///
    /// # Arguments
    /// * `private_key_pem` - RSA private key in PEM format
    /// * `public_key_pem` - RSA public key in PEM format
    pub fn new(private_key_pem: &[u8], public_key_pem: &[u8]) -> Result<Self> {
        let encoding_key = EncodingKey::from_rsa_pem(private_key_pem)
            .map_err(|e| Error::Internal(format!("Failed to load private key: {}", e)))?;

        let decoding_key = DecodingKey::from_rsa_pem(public_key_pem)
            .map_err(|e| Error::Internal(format!("Failed to load public key: {}", e)))?;

        Ok(Self {
            encoding_key: Arc::new(encoding_key),
            decoding_key: Arc::new(decoding_key),
            algorithm: Algorithm::RS256,
        })
    }

    /// Generate RSA key pair (for development/testing)
    ///
    /// In production, keys should be generated externally and loaded from secure storage.
    #[cfg(test)]
    pub fn generate_keys() -> (Vec<u8>, Vec<u8>) {
        use rsa::{pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding}, RsaPrivateKey};
        use rsa::pkcs8::DecodePrivateKey;

        let mut rng = rand::thread_rng();
        let bits = 2048;
        let private_key = RsaPrivateKey::new(&mut rng, bits).expect("Failed to generate key");
        let public_key = private_key.to_public_key();

        let private_pem = private_key
            .to_pkcs8_pem(LineEnding::LF)
            .expect("Failed to encode private key")
            .as_bytes()
            .to_vec();

        let public_pem = public_key
            .to_public_key_pem(LineEnding::LF)
            .expect("Failed to encode public key")
            .as_bytes()
            .to_vec();

        (private_pem, public_pem)
    }

    /// Sign a token
    ///
    /// # Arguments
    /// * `user_id` - User ID
    /// * `permissions` - User permissions (64-bit bitmask)
    /// * `token_type` - Access or refresh token
    pub fn sign_token(
        &self,
        user_id: &UserId,
        permissions: i64,
        token_type: TokenType,
    ) -> Result<String> {
        let now = Utc::now();
        let duration = match token_type {
            TokenType::Access => Duration::hours(1),
            TokenType::Refresh => Duration::days(30),
        };

        let claims = Claims {
            sub: user_id.as_str().to_string(),
            permissions,
            typ: match token_type {
                TokenType::Access => "access".to_string(),
                TokenType::Refresh => "refresh".to_string(),
            },
            iat: now.timestamp(),
            exp: (now + duration).timestamp(),
        };

        let header = Header::new(self.algorithm);
        encode(&header, &claims, &self.encoding_key)
            .map_err(|e| Error::Internal(format!("Failed to sign token: {}", e)))
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
                _ => Error::Authentication(format!("Token verification failed: {}", e)),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_jwt_service() -> JwtService {
        let (private_pem, public_pem) = JwtService::generate_keys();
        JwtService::new(&private_pem, &public_pem).unwrap()
    }

    #[test]
    fn test_sign_and_verify_access_token() {
        let jwt = create_jwt_service();
        let user_id = UserId::new();
        let permissions = 12345;

        let token = jwt.sign_token(&user_id, permissions, TokenType::Access).unwrap();
        let claims = jwt.verify_access_token(&token).unwrap();

        assert_eq!(claims.sub, user_id.as_str());
        assert_eq!(claims.permissions, permissions);
        assert!(claims.is_access_token());
    }

    #[test]
    fn test_sign_and_verify_refresh_token() {
        let jwt = create_jwt_service();
        let user_id = UserId::new();
        let permissions = 67890;

        let token = jwt.sign_token(&user_id, permissions, TokenType::Refresh).unwrap();
        let claims = jwt.verify_refresh_token(&token).unwrap();

        assert_eq!(claims.sub, user_id.as_str());
        assert_eq!(claims.permissions, permissions);
        assert!(claims.is_refresh_token());
    }

    #[test]
    fn test_verify_wrong_token_type() {
        let jwt = create_jwt_service();
        let user_id = UserId::new();

        let access_token = jwt.sign_token(&user_id, 0, TokenType::Access).unwrap();
        let result = jwt.verify_refresh_token(&access_token);
        assert!(result.is_err());

        let refresh_token = jwt.sign_token(&user_id, 0, TokenType::Refresh).unwrap();
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

        let token = jwt.sign_token(&user_id, 0, TokenType::Access).unwrap();
        let mut parts: Vec<&str> = token.split('.').collect();
        parts[1] = "tampered_payload";
        let tampered_token = parts.join(".");

        let result = jwt.verify_token(&tampered_token);
        assert!(result.is_err());
    }
}
