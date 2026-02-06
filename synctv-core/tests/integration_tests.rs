//! Integration tests for synctv-core services
//!
//! These tests verify end-to-end functionality across multiple service layers.
//!
//! Run with: cargo test --test integration_tests

use synctv_core::{
    models::{UserId, UserRole},
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
        .sign_token(&user_id, UserRole::User, TokenType::Access)
        .unwrap();

    // Verify token
    let claims = jwt_service.verify_access_token(&access_token).unwrap();
    assert_eq!(claims.sub, user_id.as_str());
    assert!(claims.is_access_token());

    // Generate refresh token
    let refresh_token = jwt_service
        .sign_token(&user_id, UserRole::User, TokenType::Refresh)
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
        .sign_token(&user_id, UserRole::User, TokenType::Access)
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
        .sign_token(&user_id, UserRole::User, TokenType::Access)
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

// ==================== Publish Key Service Tests ====================

#[tokio::test]
async fn test_publish_key_generation_and_validation() {
    use synctv_core::models::{MediaId, RoomId};
    use synctv_core::service::PublishKeyService;

    let jwt_service = create_test_jwt_service();
    let publish_key_service = PublishKeyService::with_default_ttl(jwt_service);

    let room_id = RoomId::new();
    let media_id = MediaId::new();
    let user_id = UserId::new();

    // Generate publish key
    let key = publish_key_service
        .generate_publish_key(room_id.clone(), media_id.clone(), user_id.clone())
        .await
        .expect("Failed to generate publish key");

    assert_eq!(key.room_id, room_id.as_str());
    assert_eq!(key.media_id, media_id.as_str());
    assert_eq!(key.user_id, user_id.as_str());
    assert!(key.expires_at > 0);
    assert!(!key.token.is_empty());

    // Validate the key
    let claims = publish_key_service
        .validate_publish_key(&key.token)
        .await
        .expect("Failed to validate publish key");

    assert_eq!(claims.room_id, room_id.as_str());
    assert_eq!(claims.media_id, media_id.as_str());
    assert_eq!(claims.user_id, user_id.as_str());
    assert!(claims.perm_start_live);
}

#[tokio::test]
async fn test_publish_key_room_media_verification() {
    use synctv_core::models::{MediaId, RoomId};
    use synctv_core::service::PublishKeyService;

    let jwt_service = create_test_jwt_service();
    let publish_key_service = PublishKeyService::with_default_ttl(jwt_service);

    let room_id = RoomId::new();
    let media_id = MediaId::new();
    let user_id = UserId::new();

    let key = publish_key_service
        .generate_publish_key(room_id.clone(), media_id.clone(), user_id.clone())
        .await
        .unwrap();

    // Verify for correct room/media
    let result = publish_key_service
        .verify_publish_key_for_stream(&key.token, &room_id, &media_id)
        .await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().as_str(), user_id.as_str());

    // Verify for wrong room
    let wrong_room = RoomId::new();
    let result = publish_key_service
        .verify_publish_key_for_stream(&key.token, &wrong_room, &media_id)
        .await;
    assert!(result.is_err());

    // Verify for wrong media
    let wrong_media = MediaId::new();
    let result = publish_key_service
        .verify_publish_key_for_stream(&key.token, &room_id, &wrong_media)
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_publish_key_invalid_token() {
    use synctv_core::service::PublishKeyService;

    let jwt_service = create_test_jwt_service();
    let publish_key_service = PublishKeyService::with_default_ttl(jwt_service);

    let result = publish_key_service
        .validate_publish_key("invalid.token.here")
        .await;
    assert!(result.is_err());
}

// ==================== Permission Bitmask Tests ====================

#[test]
fn test_permission_bits_operations() {
    use synctv_core::models::PermissionBits;

    // Test individual bit flags
    let mut perms = PermissionBits(0);
    assert!(!perms.has(PermissionBits::SEND_CHAT));

    perms.grant(PermissionBits::SEND_CHAT);
    assert!(perms.has(PermissionBits::SEND_CHAT));
    assert!(!perms.has(PermissionBits::ADD_MEDIA));

    perms.grant(PermissionBits::ADD_MEDIA);
    assert!(perms.has(PermissionBits::SEND_CHAT));
    assert!(perms.has(PermissionBits::ADD_MEDIA));

    perms.revoke(PermissionBits::SEND_CHAT);
    assert!(!perms.has(PermissionBits::SEND_CHAT));
    assert!(perms.has(PermissionBits::ADD_MEDIA));
}

#[test]
fn test_permission_default_roles() {
    use synctv_core::models::PermissionBits;

    // Default member should have SEND_CHAT
    let member = PermissionBits(PermissionBits::DEFAULT_MEMBER);
    assert!(member.has(PermissionBits::SEND_CHAT));
    assert!(member.has(PermissionBits::ADD_MEDIA));
    assert!(!member.has(PermissionBits::MANAGE_ADMIN));

    // Default admin should have member permissions plus admin permissions
    let admin = PermissionBits(PermissionBits::DEFAULT_ADMIN);
    assert!(admin.has(PermissionBits::SEND_CHAT));
    assert!(admin.has(PermissionBits::KICK_MEMBER));
    assert!(admin.has(PermissionBits::SET_ROOM_SETTINGS));

    // Default guest should have minimal permissions
    let guest = PermissionBits(PermissionBits::DEFAULT_GUEST);
    assert!(guest.has(PermissionBits::VIEW_PLAYLIST));
    assert!(!guest.has(PermissionBits::SEND_CHAT));

    // NONE should have no permissions
    let none = PermissionBits(PermissionBits::NONE);
    assert!(!none.has(PermissionBits::SEND_CHAT));
    assert!(!none.has(PermissionBits::VIEW_PLAYLIST));

    // ALL should have all permissions
    let all = PermissionBits(PermissionBits::ALL);
    assert!(all.has(PermissionBits::SEND_CHAT));
    assert!(all.has(PermissionBits::MANAGE_ADMIN));
    assert!(all.has(PermissionBits::DELETE_ROOM));
}

// ==================== Playlist Model Tests ====================

#[test]
fn test_playlist_model() {
    use synctv_core::models::{Playlist, PlaylistId, RoomId};

    // Test root playlist (no parent, empty name)
    let root = Playlist {
        id: PlaylistId::new(),
        room_id: RoomId::new(),
        creator_id: UserId::new(),
        name: String::new(),
        parent_id: None,
        position: 0,
        source_provider: None,
        source_config: None,
        provider_instance_name: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    assert!(root.is_root());
    assert!(!root.is_dynamic());
    assert!(root.is_static());

    // Test static folder (has parent, named)
    let folder = Playlist {
        id: PlaylistId::new(),
        room_id: RoomId::new(),
        creator_id: UserId::new(),
        name: "My Folder".to_string(),
        parent_id: Some(root.id.clone()),
        position: 0,
        source_provider: None,
        source_config: None,
        provider_instance_name: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    assert!(!folder.is_root());
    assert!(!folder.is_dynamic());
    assert!(folder.is_static());

    // Test dynamic folder (has source_provider)
    let dynamic = Playlist {
        id: PlaylistId::new(),
        room_id: RoomId::new(),
        creator_id: UserId::new(),
        name: "Alist Folder".to_string(),
        parent_id: Some(root.id.clone()),
        position: 1,
        source_provider: Some("alist".to_string()),
        source_config: Some(serde_json::json!({"url": "http://example.com"})),
        provider_instance_name: Some("my_alist".to_string()),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    assert!(!dynamic.is_root());
    assert!(dynamic.is_dynamic());
    assert!(!dynamic.is_static());
}

// ==================== ID Model Tests ====================

#[test]
fn test_id_generation_uniqueness() {
    use synctv_core::models::{MediaId, PlaylistId, RoomId};

    // IDs should be unique
    let id1 = RoomId::new();
    let id2 = RoomId::new();
    assert_ne!(id1.as_str(), id2.as_str());

    let mid1 = MediaId::new();
    let mid2 = MediaId::new();
    assert_ne!(mid1.as_str(), mid2.as_str());

    let pid1 = PlaylistId::new();
    let pid2 = PlaylistId::new();
    assert_ne!(pid1.as_str(), pid2.as_str());

    // from_string should preserve the value
    let room_id = RoomId::from_string("test_room_123".to_string());
    assert_eq!(room_id.as_str(), "test_room_123");
}

// ==================== JWT Token Version Tests ====================

#[tokio::test]
async fn test_jwt_token_role_validation() {
    let jwt_service = create_test_jwt_service();
    let user_id = UserId::new();

    // Generate token with User role
    let user_token = jwt_service
        .sign_token(&user_id, UserRole::User, TokenType::Access)
        .unwrap();

    let claims = jwt_service.verify_access_token(&user_token).unwrap();
    assert_eq!(claims.sub, user_id.as_str());
    assert_eq!(claims.role, "user");

    // Generate token with Admin role
    let admin_token = jwt_service
        .sign_token(&user_id, UserRole::Admin, TokenType::Access)
        .unwrap();

    let claims = jwt_service.verify_access_token(&admin_token).unwrap();
    assert_eq!(claims.role, "admin");

    // Generate token with Root role
    let root_token = jwt_service
        .sign_token(&user_id, UserRole::Root, TokenType::Access)
        .unwrap();

    let claims = jwt_service.verify_access_token(&root_token).unwrap();
    assert_eq!(claims.role, "root");
}

#[tokio::test]
async fn test_jwt_access_and_refresh_tokens_different() {
    let jwt_service = create_test_jwt_service();
    let user_id = UserId::new();

    let access_token = jwt_service
        .sign_token(&user_id, UserRole::User, TokenType::Access)
        .unwrap();
    let refresh_token = jwt_service
        .sign_token(&user_id, UserRole::User, TokenType::Refresh)
        .unwrap();

    // Tokens should be different
    assert_ne!(access_token, refresh_token);

    // Access token should not validate as refresh
    let result = jwt_service.verify_refresh_token(&access_token);
    assert!(result.is_err());

    // Refresh token should not validate as access
    let result = jwt_service.verify_access_token(&refresh_token);
    assert!(result.is_err());
}

// ==================== Error Type Tests ====================

#[test]
fn test_error_types() {
    use synctv_core::Error;

    let errors = vec![
        Error::Authentication("auth failed".to_string()),
        Error::Authorization("not authorized".to_string()),
        Error::NotFound("resource missing".to_string()),
        Error::InvalidInput("bad input".to_string()),
        Error::PermissionDenied("access denied".to_string()),
        Error::Internal("internal error".to_string()),
    ];

    for error in &errors {
        let msg = format!("{}", error);
        assert!(!msg.is_empty());
    }

    // Verify error variants match
    assert!(matches!(errors[0], Error::Authentication(_)));
    assert!(matches!(errors[1], Error::Authorization(_)));
    assert!(matches!(errors[2], Error::NotFound(_)));
    assert!(matches!(errors[3], Error::InvalidInput(_)));
    assert!(matches!(errors[4], Error::PermissionDenied(_)));
    assert!(matches!(errors[5], Error::Internal(_)));
}

// Database-dependent tests are marked #[ignore] - run with:
// cargo test --test integration_tests -- --ignored
#[tokio::test]
#[ignore = "Requires database connection"]
async fn test_create_room_and_join() {
    // Placeholder: requires full service initialization with database
}

#[tokio::test]
#[ignore = "Requires database connection"]
async fn test_playlist_operations() {
    // Placeholder: requires PlaylistService with database
}

#[tokio::test]
#[ignore = "Requires database connection"]
async fn test_permission_checks() {
    // Placeholder: requires PermissionService with database
}

#[tokio::test]
#[ignore = "Requires database connection"]
async fn test_playback_sync() {
    // Placeholder: requires PlaybackService with database
}
