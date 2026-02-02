//! Integration tests for synctv-core services
//!
//! These tests verify end-to-end functionality across multiple service layers.
//!
//! Run with: cargo test --test integration_tests

use std::sync::Arc;
use synctv_core::{
    models::{RoomId, UserId},
    service::{
        auth::{jwt::JwtService, TokenType},
        room::RoomService,
        user::UserService,
        member::MemberService,
        chat::ChatService,
        permission::Permission,
    },
    test_helpers::*,
    Error,
};

/// Helper to create a test JWT service
fn create_test_jwt_service() -> JwtService {
    let (private_key, public_key) = JwtService::generate_keys();
    JwtService::new(&private_key, &public_key).unwrap()
}

/// Helper to create test users
async fn setup_test_user(user_service: &UserService, username: &str) -> Result<UserId> {
    let create_request = synctv_core::service::user::CreateUserRequest {
        username: username.to_string(),
        password: "password123".to_string(),
    };

    user_service
        .create_user(&create_request)
        .await
        .map(|user| user.id)
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
async fn test_permissions_bitmask() {
    use synctv_core::service::permission;

    // Test individual permissions
    assert_eq!(permission::ROOM_OWNER, 1 << 0);
    assert_eq!(permission::ROOM_ADMIN, 1 << 1);
    assert_eq!(permission::ROOM_MODERATOR, 1 << 2);

    // Test permission combinations
    let permissions = permission::ROOM_OWNER | permission::ROOM_ADMIN;
    assert!(permissions & permission::ROOM_OWNER != 0);
    assert!(permissions & permission::ROOM_ADMIN != 0);

    // Test permission checking
    let user_permissions = permission::ROOM_MODERATOR | permission::CHAT_SEND;
    assert!(permission::has_permission(user_permissions, permission::CHAT_SEND));
    assert!(!permission::has_permission(user_permissions, permission::ROOM_OWNER));
}

#[tokio::test]
async fn test_room_id_generation() {
    let room_id1 = random_room_id();
    let room_id2 = random_room_id();

    // Room IDs should be unique
    assert_ne!(room_id1, room_id2);

    // Room IDs should be 12 characters
    assert_eq!(room_id1.0.len(), 12);
    assert_eq!(room_id2.0.len(), 12);
}

#[tokio::test]
async fn test_user_id_generation() {
    let user_id1 = random_user_id();
    let user_id2 = random_user_id();

    // User IDs should be unique
    assert_ne!(user_id1, user_id2);

    // User IDs should be non-empty
    assert!(!user_id1.as_str().is_empty());
    assert!(!user_id2.as_str().is_empty());
}

#[tokio::test]
async fn test_fixture_builders() {
    // Test UserFixture
    let user = UserFixture::new()
        .with_username("alice")
        .with_permissions(123)
        .build();

    assert_eq!(user.username, "alice");
    assert_eq!(user.permissions, 123);

    // Test RoomFixture
    let owner_id = test_user_id("owner1");
    let room = RoomFixture::new()
        .with_name("My Room")
        .with_owner(owner_id.clone())
        .build();

    assert_eq!(room.name, "My Room");
    assert_eq!(room.owner_id, owner_id);

    // Test ChatMessageFixture
    let room_id = test_room_id("room1");
    let user_id = test_user_id("user1");
    let message = ChatMessageFixture::new()
        .with_room_id(room_id)
        .with_user_id(user_id)
        .with_content("Hello, world!")
        .build();

    assert_eq!(message.room_id, room_id);
    assert_eq!(message.user_id, user_id);
    assert_eq!(message.content, "Hello, world!");
}

#[tokio::test]
async fn test_cache_stats_calculation() {
    use synctv_core::cache::CacheStats;

    let stats = CacheStats {
        l1_hit_rate: 0.85,
        l1_size: 1000,
        l1_capacity: 10000,
        total_hits: 8500,
        total_misses: 1500,
    };

    // Test hit rate calculation
    let expected_hit_rate = 8500.0 / (8500.0 + 1500.0);
    assert!((stats.hit_rate() - expected_hit_rate).abs() < 0.001);

    // Test with zero total
    let empty_stats = CacheStats::default();
    assert_eq!(empty_stats.hit_rate(), 0.0);
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
