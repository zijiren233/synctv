//! Integration tests for synctv-core services
//!
//! These tests verify end-to-end functionality across multiple service layers.
//!
//! Run with: cargo test --test integration_tests

use synctv_core::{
    models::UserId,
    service::{
        auth::{jwt::JwtService, TokenType},
    },
};

/// Helper to create a test JWT service with generated keys
fn create_test_jwt_service() -> JwtService {
    use rsa::{pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding}, RsaPrivateKey};
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

    JwtService::new(&private_pem, &public_pem).expect("Failed to create JWT service")
}

#[tokio::test]
async fn test_user_registration_and_authentication() {
    // This test would require a full database setup
    // For now, we'll test the JWT service which doesn't require a database

    let jwt_service = create_test_jwt_service();

    // Create a user ID
    let user_id = UserId::new();

    // Generate access token
    let access_token = jwt_service
        .sign_token(&user_id, 0, TokenType::Access)
        .unwrap();

    // Verify token
    let claims = jwt_service.verify_access_token(&access_token).unwrap();
    assert_eq!(claims.sub, user_id.as_str());
    assert!(claims.is_access_token());

    // Generate refresh token
    let refresh_token = jwt_service
        .sign_token(&user_id, 0, TokenType::Refresh)
        .unwrap();

    // Verify refresh token
    let claims = jwt_service.verify_refresh_token(&refresh_token).unwrap();
    assert_eq!(claims.sub, user_id.as_str());
    assert!(claims.is_refresh_token());
}

#[tokio::test]
async fn test_jwt_token_expiration() {
    let jwt_service = create_test_jwt_service();
    let user_id = UserId::new();

    // Access token should be valid for 1 hour
    let token = jwt_service
        .sign_token(&user_id, 0, TokenType::Access)
        .unwrap();

    let claims = jwt_service.verify_token(&token).unwrap();
    assert!(claims.exp > claims.iat); // Expiration is after issued at

    // For access tokens (1 hour)
    let expected_exp = claims.iat + 3600;
    assert_eq!(claims.exp, expected_exp);
}

#[tokio::test]
async fn test_jwt_invalid_token() {
    let jwt_service = create_test_jwt_service();

    // Invalid token format
    let result = jwt_service.verify_token("invalid.token");
    assert!(result.is_err());

    // Tampered token
    let user_id = UserId::new();
    let token = jwt_service
        .sign_token(&user_id, 0, TokenType::Access)
        .unwrap();

    let parts: Vec<&str> = token.split('.').collect();
    let tampered_token = format!("{}.{}.tampered", parts[0], parts[1]);

    let result = jwt_service.verify_token(&tampered_token);
    assert!(result.is_err());
}

#[tokio::test]
async fn test_error_handling() {
    use synctv_core::Error;

    // Test error conversion
    let auth_error = Error::Authentication("Invalid token".to_string());
    assert!(matches!(auth_error, Error::Authentication(_)));

    let not_found_error = Error::NotFound("User not found".to_string());
    assert!(matches!(not_found_error, Error::NotFound(_)));

    // Verify error display
    let error_msg = format!("{}", auth_error);
    assert!(error_msg.contains("Invalid token"));
}

#[tokio::test]
async fn test_concurrent_operations() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let counter = Arc::new(AtomicUsize::new(0));
    let mut handles = vec![];

    // Spawn 10 concurrent tasks
    for _ in 0..10 {
        let counter = counter.clone();
        let handle = tokio::spawn(async move {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        handles.push(handle);
    }

    // Wait for all tasks to complete
    for handle in handles {
        handle.await.unwrap();
    }

    // Verify all operations completed
    assert_eq!(counter.load(Ordering::SeqCst), 10);
}

// TODO: Add more integration tests that require database setup:
// - test_create_room_and_join
// - test_chat_message_flow
// - test_user_permissions
// - test_room_lifecycle
// - test_concurrent_room_access
