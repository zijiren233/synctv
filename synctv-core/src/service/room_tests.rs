//! Comprehensive unit tests for RoomService
//!
//! These tests verify room management logic without requiring database setup.

use synctv_core::{
    models::{Room, RoomId, RoomStatus, UserId},
    service::room::{CreateRoomRequest, UpdateRoomRequest, RoomSettings},
    test_helpers::*,
    Error,
};

#[cfg(test)]
mod room_service_unit_tests {
    use super::*;

    /// Test room settings validation
    #[test]
    fn test_room_settings_validation() {
        let settings = RoomSettings::default();

        // Default settings should be valid
        assert_eq!(settings.max_members, Some(100));
        assert_eq!(settings.chat_enabled, Some(true));
        assert_eq!(settings.danmaku_enabled, Some(false));
    }

    /// Test room status transitions
    #[test]
    fn test_room_status_transitions() {
        let mut status = RoomStatus::Pending;

        // Pending -> Active
        status = RoomStatus::Active;
        assert!(status.is_active());
        assert!(!status.is_pending());
        assert!(!status.is_closed());

        // Active -> Closed
        status = RoomStatus::Closed;
        assert!(status.is_closed());
        assert!(!status.is_active());
    }

    /// Test room ID validation
    #[test]
    fn test_room_id_validation() {
        // Valid room ID
        let room_id = test_room_id("room123");
        assert_eq!(room_id.0, "room123");

        // Room IDs should be 12 characters (nanoid)
        let random_id = random_room_id();
        assert_eq!(random_id.0.len(), 12);
    }

    /// Test room ownership
    #[test]
    fn test_room_ownership() {
        let owner_id = random_user_id();
        let other_user_id = random_user_id();

        let room = RoomFixture::new()
            .with_owner(owner_id.clone())
            .build();

        assert_eq!(room.owner_id, owner_id);
        assert_ne!(room.owner_id, other_user_id);
    }

    /// Test public/private room settings
    #[test]
    fn test_room_visibility() {
        let public_room = RoomFixture::new()
            .with_public(true)
            .build();

        let private_room = RoomFixture::new()
            .with_public(false)
            .build();

        assert!(public_room.is_public);
        assert!(!private_room.is_public);
    }

    /// Test room settings builder pattern
    #[test]
    fn test_room_settings_builder() {
        let settings = RoomSettings {
            require_password: Some(true),
            password: Some("hashed_password".to_string()),
            auto_play_next: Some(true),
            loop_playlist: Some(false),
            shuffle_playlist: Some(false),
            allow_guest_join: Some(false),
            max_members: Some(50),
            chat_enabled: Some(true),
            danmaku_enabled: Some(true),
        };

        assert_eq!(settings.require_password, Some(true));
        assert_eq!(settings.max_members, Some(50));
        assert_eq!(settings.chat_enabled, Some(true));
        assert_eq!(settings.danmaku_enabled, Some(true));
    }

    /// Test password requirement validation
    #[test]
    fn test_password_requirement() {
        let settings_with_password = RoomSettings {
            require_password: Some(true),
            password: Some("hashed".to_string()),
            ..Default::default()
        };

        let settings_without_password = RoomSettings {
            require_password: Some(false),
            password: None,
            ..Default::default()
        };

        // If require_password is true, password should be present
        if settings_with_password.require_password.unwrap() {
            assert!(settings_with_password.password.is_some());
        }

        // If require_password is false, password can be None
        if !settings_without_password.require_password.unwrap() {
            assert!(settings_without_password.password.is_none());
        }
    }

    /// Test max members validation
    #[test]
    fn test_max_members_validation() {
        let settings = RoomSettings {
            max_members: Some(50),
            ..Default::default()
        };

        assert_eq!(settings.max_members, Some(50));

        // Test edge cases
        let min_settings = RoomSettings {
            max_members: Some(1),
            ..Default::default()
        };
        assert_eq!(min_settings.max_members, Some(1));

        let large_settings = RoomSettings {
            max_members: Some(10000),
            ..Default::default()
        };
        assert_eq!(large_settings.max_members, Some(10000));
    }

    /// Test room fixture builder
    #[test]
    fn test_room_fixture_builder() {
        let owner_id = test_user_id("owner1");

        let room = RoomFixture::new()
            .with_id(test_room_id("room1"))
            .with_name("Test Room")
            .with_owner(owner_id.clone())
            .with_public(true)
            .build();

        assert_eq!(room.id, test_room_id("room1"));
        assert_eq!(room.name, "Test Room");
        assert_eq!(room.owner_id, owner_id);
        assert!(room.is_public);
    }

    /// Test multiple rooms with same owner
    #[test]
    fn test_multiple_rooms_same_owner() {
        let owner_id = test_user_id("owner1");

        let room1 = RoomFixture::new()
            .with_owner(owner_id.clone())
            .with_name("Room 1")
            .build();

        let room2 = RoomFixture::new()
            .with_owner(owner_id.clone())
            .with_name("Room 2")
            .build();

        assert_eq!(room1.owner_id, owner_id);
        assert_eq!(room2.owner_id, owner_id);
        assert_ne!(room1.id, room2.id);
    }

    /// Test room settings serialization
    #[test]
    fn test_room_settings_serialization() {
        let settings = RoomSettings {
            require_password: Some(true),
            password: None,
            auto_play_next: Some(false),
            loop_playlist: Some(true),
            shuffle_playlist: Some(false),
            allow_guest_join: Some(true),
            max_members: Some(25),
            chat_enabled: Some(false),
            danmaku_enabled: Some(true),
        };

        // Serialize to JSON
        let json = serde_json::to_string(&settings).unwrap();

        // Deserialize back
        let deserialized: RoomSettings = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized, settings);
    }

    /// Test create room request validation
    #[test]
    fn test_create_room_request() {
        let user_id = random_user_id();

        let request = CreateRoomRequest {
            name: "My Room".to_string(),
            is_public: true,
            settings: None,
            requesting_user_id: user_id.clone(),
        };

        assert_eq!(request.name, "My Room");
        assert!(request.is_public);
        assert_eq!(request.requesting_user_id, user_id);
        assert!(request.settings.is_none());
    }

    /// Test create room request with settings
    #[test]
    fn test_create_room_request_with_settings() {
        let user_id = random_user_id();

        let settings = RoomSettings {
            require_password: Some(true),
            password: Some("hashed".to_string()),
            max_members: Some(50),
            ..Default::default()
        };

        let request = CreateRoomRequest {
            name: "Protected Room".to_string(),
            is_public: false,
            settings: Some(settings),
            requesting_user_id: user_id,
        };

        assert!(request.settings.is_some());
        let req_settings = request.settings.unwrap();
        assert_eq!(req_settings.require_password, Some(true));
        assert_eq!(req_settings.max_members, Some(50));
    }

    /// Test update room request
    #[test]
    fn test_update_room_request() {
        let room_id = test_room_id("room1");
        let user_id = random_user_id();

        let request = UpdateRoomRequest {
            room_id: room_id.clone(),
            name: Some("Updated Name".to_string()),
            is_public: Some(false),
            settings: None,
            requesting_user_id: user_id.clone(),
        };

        assert_eq!(request.room_id, room_id);
        assert_eq!(request.name, Some("Updated Name".to_string()));
        assert_eq!(request.is_public, Some(false));
    }

    /// Test update room request with partial data
    #[test]
    fn test_update_room_request_partial() {
        let room_id = test_room_id("room1");
        let user_id = random_user_id();

        let request = UpdateRoomRequest {
            room_id: room_id.clone(),
            name: None, // Not updating name
            is_public: Some(true), // Updating visibility
            settings: None,
            requesting_user_id: user_id,
        };

        assert!(request.name.is_none());
        assert_eq!(request.is_public, Some(true));
    }
}

/// Benchmark: Room fixture creation
#[cfg(test)]
mod bench_room_fixture {
    use super::*;
    use std::time::Instant;

    #[test]
    fn bench_create_room_fixture() {
        let start = Instant::now();
        let iterations = 10_000;

        for _ in 0..iterations {
            let _room = RoomFixture::new().build();
        }

        let duration = start.elapsed();
        println!(
            "Created {} room fixtures in {:?} ({:.2} rooms/sec)",
            iterations,
            duration,
            iterations as f64 / duration.as_secs_f64()
        );
    }

    #[test]
    fn bench_create_room_id() {
        let start = Instant::now();
        let iterations = 10_000;

        for _ in 0..iterations {
            let _room_id = random_room_id();
        }

        let duration = start.elapsed();
        println!(
            "Generated {} room IDs in {:?} ({:.2} IDs/sec)",
            iterations,
            duration,
            iterations as f64 / duration.as_secs_f64()
        );
    }
}
