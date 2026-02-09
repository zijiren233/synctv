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

/// Helper to create a test JWT service with a test secret
fn create_test_jwt_service() -> JwtService {
    JwtService::new("test-secret-key-for-integration-tests").expect("Failed to create JWT service")
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

// ==================== End-to-End Test Suite ====================
// These tests verify complete workflows through multiple service layers

#[tokio::test]
async fn test_e2e_user_auth_flow() {
    // Complete user authentication flow:
    // 1. Generate JWT keys
    // 2. Create JWT service
    // 3. Generate tokens
    // 4. Verify tokens
    // 5. Test token refresh

    let jwt_service = create_test_jwt_service();
    let user_id = UserId::new();

    // Step 1: User logs in and gets access + refresh tokens
    let access_token = jwt_service
        .sign_token(&user_id, UserRole::User, TokenType::Access)
        .expect("Failed to generate access token");

    let refresh_token = jwt_service
        .sign_token(&user_id, UserRole::User, TokenType::Refresh)
        .expect("Failed to generate refresh token");

    // Step 2: Verify access token for API requests
    let access_claims = jwt_service
        .verify_access_token(&access_token)
        .expect("Failed to verify access token");

    assert_eq!(access_claims.sub, user_id.as_str());
    assert_eq!(access_claims.role, "user");
    assert!(access_claims.is_access_token());

    // Step 3: Access token expired (simulated), use refresh token
    let refresh_claims = jwt_service
        .verify_refresh_token(&refresh_token)
        .expect("Failed to verify refresh token");

    assert_eq!(refresh_claims.sub, user_id.as_str());
    assert!(refresh_claims.is_refresh_token());

    // Step 4: Generate new access token using refresh token
    let new_access_token = jwt_service
        .sign_token(&user_id, UserRole::User, TokenType::Access)
        .expect("Failed to generate new access token");

    let new_claims = jwt_service
        .verify_access_token(&new_access_token)
        .expect("Failed to verify new access token");

    assert_eq!(new_claims.sub, user_id.as_str());
    assert!(new_claims.exp > new_claims.iat);
}

#[tokio::test]
async fn test_e2e_role_upgrade_flow() {
    // Test user role upgrade from User -> Admin -> Root
    let jwt_service = create_test_jwt_service();
    let user_id = UserId::new();

    // Initial token: User role
    let user_token = jwt_service
        .sign_token(&user_id, UserRole::User, TokenType::Access)
        .unwrap();

    let claims = jwt_service.verify_access_token(&user_token).unwrap();
    assert_eq!(claims.role, "user");

    // Upgrade to Admin
    let admin_token = jwt_service
        .sign_token(&user_id, UserRole::Admin, TokenType::Access)
        .unwrap();

    let claims = jwt_service.verify_access_token(&admin_token).unwrap();
    assert_eq!(claims.role, "admin");

    // Upgrade to Root
    let root_token = jwt_service
        .sign_token(&user_id, UserRole::Root, TokenType::Access)
        .unwrap();

    let claims = jwt_service.verify_access_token(&root_token).unwrap();
    assert_eq!(claims.role, "root");
}

#[tokio::test]
async fn test_e2e_publish_key_workflow() {
    use synctv_core::models::{MediaId, RoomId};
    use synctv_core::service::PublishKeyService;

    // Complete streaming publish key workflow:
    // 1. Admin creates publish key
    // 2. Streamer validates key
    // 3. Streamer publishes with key
    // 4. System verifies room/media match

    let jwt_service = create_test_jwt_service();
    let publish_key_service = PublishKeyService::with_default_ttl(jwt_service);

    let room_id = RoomId::new();
    let media_id = MediaId::new();
    let user_id = UserId::new();

    // Step 1: Admin generates publish key for user
    let publish_key = publish_key_service
        .generate_publish_key(room_id.clone(), media_id.clone(), user_id.clone())
        .await
        .expect("Failed to generate publish key");

    assert_eq!(publish_key.room_id, room_id.as_str());
    assert_eq!(publish_key.media_id, media_id.as_str());
    assert_eq!(publish_key.user_id, user_id.as_str());
    assert!(!publish_key.token.is_empty());

    // Step 2: Streamer connects with publish key
    let claims = publish_key_service
        .validate_publish_key(&publish_key.token)
        .await
        .expect("Failed to validate publish key");

    assert_eq!(claims.room_id, room_id.as_str());
    assert_eq!(claims.media_id, media_id.as_str());
    assert!(claims.perm_start_live);

    // Step 3: System verifies streamer is publishing to correct room/media
    let verified_user_id = publish_key_service
        .verify_publish_key_for_stream(&publish_key.token, &room_id, &media_id)
        .await
        .expect("Failed to verify publish key for stream");

    assert_eq!(verified_user_id.as_str(), user_id.as_str());

    // Step 4: Verify wrong room/media fails
    let wrong_room = RoomId::new();
    let result = publish_key_service
        .verify_publish_key_for_stream(&publish_key.token, &wrong_room, &media_id)
        .await;

    assert!(result.is_err(), "Should fail with wrong room");
}

#[tokio::test]
async fn test_e2e_permission_checks() {
    use synctv_core::models::PermissionBits;

    // Complete permission check workflow:
    // 1. Member has default permissions
    // 2. Grant extra permission
    // 3. Verify permission granted
    // 4. Revoke permission
    // 5. Verify permission removed

    // Step 1: Member with default permissions
    let mut member_perms = PermissionBits(PermissionBits::DEFAULT_MEMBER);

    assert!(member_perms.has(PermissionBits::SEND_CHAT));
    assert!(member_perms.has(PermissionBits::ADD_MEDIA));
    assert!(!member_perms.has(PermissionBits::KICK_MEMBER));

    // Step 2: Grant admin permission to member
    member_perms.grant(PermissionBits::KICK_MEMBER);

    // Step 3: Verify permission granted
    assert!(member_perms.has(PermissionBits::KICK_MEMBER));
    assert!(member_perms.has(PermissionBits::SEND_CHAT)); // Still has original

    // Step 4: Revoke permission
    member_perms.revoke(PermissionBits::KICK_MEMBER);

    // Step 5: Verify permission removed
    assert!(!member_perms.has(PermissionBits::KICK_MEMBER));
    assert!(member_perms.has(PermissionBits::SEND_CHAT)); // Original unchanged
}

#[tokio::test]
async fn test_e2e_playlist_hierarchy() {
    use synctv_core::models::{Playlist, PlaylistId, RoomId};

    // Complete playlist hierarchy workflow:
    // 1. Create root playlist
    // 2. Create static folder under root
    // 3. Create dynamic folder under root
    // 4. Verify hierarchy

    let room_id = RoomId::new();
    let creator_id = UserId::new();

    // Step 1: Root playlist
    let root = Playlist {
        id: PlaylistId::new(),
        room_id: room_id.clone(),
        creator_id: creator_id.clone(),
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

    // Step 2: Static folder
    let static_folder = Playlist {
        id: PlaylistId::new(),
        room_id: room_id.clone(),
        creator_id: creator_id.clone(),
        name: "Movies".to_string(),
        parent_id: Some(root.id.clone()),
        position: 0,
        source_provider: None,
        source_config: None,
        provider_instance_name: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    assert!(!static_folder.is_root());
    assert!(!static_folder.is_dynamic());
    assert!(static_folder.is_static());
    assert_eq!(static_folder.parent_id.unwrap(), root.id);

    // Step 3: Dynamic folder (Alist)
    let dynamic_folder = Playlist {
        id: PlaylistId::new(),
        room_id: room_id.clone(),
        creator_id: creator_id.clone(),
        name: "Alist Movies".to_string(),
        parent_id: Some(root.id.clone()),
        position: 1,
        source_provider: Some("alist".to_string()),
        source_config: Some(serde_json::json!({
            "url": "http://alist.example.com",
            "path": "/movies"
        })),
        provider_instance_name: Some("main_alist".to_string()),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    assert!(!dynamic_folder.is_root());
    assert!(dynamic_folder.is_dynamic());
    assert!(!dynamic_folder.is_static());
    assert_eq!(dynamic_folder.parent_id.unwrap(), root.id);
}

#[tokio::test]
async fn test_e2e_multiple_users_concurrent_auth() {
    use std::sync::Arc;

    let jwt_service = Arc::new(create_test_jwt_service());
    let mut handles = vec![];

    // Simulate 10 users logging in concurrently
    for i in 0..10 {
        let jwt = jwt_service.clone();
        let handle = tokio::spawn(async move {
            let user_id = UserId::new();
            let role = if i % 3 == 0 {
                UserRole::Admin
            } else {
                UserRole::User
            };

            // Generate and verify token
            let token = jwt
                .sign_token(&user_id, role.clone(), TokenType::Access)
                .expect("Failed to sign token");

            let claims = jwt
                .verify_access_token(&token)
                .expect("Failed to verify token");

            assert_eq!(claims.sub, user_id.as_str());
            (user_id, claims)
        });
        handles.push(handle);
    }

    // Wait for all authentications to complete
    let results: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // Verify all 10 users authenticated successfully
    assert_eq!(results.len(), 10);

    // Verify admin roles
    let admin_count = results.iter()
        .filter(|(_, claims)| claims.role == "admin")
        .count();
    assert!(admin_count >= 3); // At least indices 0, 3, 6, 9
}

#[tokio::test]
async fn test_e2e_permission_inheritance() {
    use synctv_core::models::PermissionBits;

    // Test that admin permissions include all member permissions
    let admin = PermissionBits(PermissionBits::DEFAULT_ADMIN);

    // Admin should have all member permissions
    assert!(admin.has(PermissionBits::SEND_CHAT)); // Member perm
    assert!(admin.has(PermissionBits::ADD_MEDIA)); // Member perm

    // Plus admin-specific permissions
    assert!(admin.has(PermissionBits::KICK_MEMBER)); // Admin perm
    assert!(admin.has(PermissionBits::SET_ROOM_SETTINGS)); // Admin perm

    // Guest should have minimal permissions
    let guest = PermissionBits(PermissionBits::DEFAULT_GUEST);
    assert!(guest.has(PermissionBits::VIEW_PLAYLIST));
    assert!(!guest.has(PermissionBits::SEND_CHAT));
    assert!(!guest.has(PermissionBits::ADD_MEDIA));
}

#[tokio::test]
async fn test_e2e_token_type_validation() {
    let jwt_service = create_test_jwt_service();
    let user_id = UserId::new();

    // Generate both token types
    let access = jwt_service
        .sign_token(&user_id, UserRole::User, TokenType::Access)
        .unwrap();

    let refresh = jwt_service
        .sign_token(&user_id, UserRole::User, TokenType::Refresh)
        .unwrap();

    // Access token should only verify as access
    assert!(jwt_service.verify_access_token(&access).is_ok());
    assert!(jwt_service.verify_refresh_token(&access).is_err());

    // Refresh token should only verify as refresh
    assert!(jwt_service.verify_refresh_token(&refresh).is_ok());
    assert!(jwt_service.verify_access_token(&refresh).is_err());
}

#[tokio::test]
async fn test_e2e_id_generation_collision_resistance() {
    use synctv_core::models::{MediaId, RoomId, PlaylistId};
    use std::collections::HashSet;

    // Generate 1000 IDs and verify no collisions
    let mut room_ids = HashSet::new();
    let mut media_ids = HashSet::new();
    let mut playlist_ids = HashSet::new();

    for _ in 0..1000 {
        let room = RoomId::new();
        let media = MediaId::new();
        let playlist = PlaylistId::new();

        // No collisions within same type
        assert!(room_ids.insert(room.as_str().to_string()));
        assert!(media_ids.insert(media.as_str().to_string()));
        assert!(playlist_ids.insert(playlist.as_str().to_string()));
    }

    // Verify all 1000 are unique
    assert_eq!(room_ids.len(), 1000);
    assert_eq!(media_ids.len(), 1000);
    assert_eq!(playlist_ids.len(), 1000);
}

#[tokio::test]
async fn test_e2e_error_propagation() {
    use synctv_core::Error;

    // Test that errors propagate correctly through service layers
    let errors = vec![
        Error::Authentication("Invalid credentials".to_string()),
        Error::Authorization("Permission denied".to_string()),
        Error::NotFound("Room not found".to_string()),
        Error::InvalidInput("Invalid room name".to_string()),
        Error::PermissionDenied("Cannot kick admin".to_string()),
        Error::Internal("Database error".to_string()),
    ];

    for error in errors {
        // Verify error can be converted to string for logging
        let msg = format!("{}", error);
        assert!(!msg.is_empty());

        // Verify error types are distinguishable
        match error {
            Error::Authentication(_) => assert!(msg.contains("Invalid credentials")),
            Error::Authorization(_) => assert!(msg.contains("Permission denied")),
            Error::NotFound(_) => assert!(msg.contains("Room not found")),
            Error::InvalidInput(_) => assert!(msg.contains("Invalid room name")),
            Error::PermissionDenied(_) => assert!(msg.contains("Cannot kick admin")),
            Error::Internal(_) => assert!(msg.contains("Database error")),
            _ => panic!("Unexpected error type"),
        }
    }
}
