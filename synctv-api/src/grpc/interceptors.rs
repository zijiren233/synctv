use synctv_core::service::auth::JwtService;
use tonic::{metadata::MetadataMap, Request, Status};

/// User context - contains user_id extracted from JWT
/// Used by UserService and AdminService methods
#[derive(Debug, Clone)]
pub struct UserContext {
    pub user_id: String,
}

/// Room context - contains UserContext and room_id
/// Used by RoomService and MediaService methods
#[derive(Debug, Clone)]
pub struct RoomContext {
    #[allow(dead_code)] // Nested for future use when both user and room info needed
    pub user_ctx: UserContext,
    pub room_id: String,
}

/// Simple JWT auth interceptor (synchronous, compatible with tonic::service::Interceptor)
/// Only validates JWT and extracts user_id into AuthContext
/// Service methods should call helper functions to load entities from database
#[derive(Clone)]
pub struct AuthInterceptor {
    jwt_service: JwtService,
}

impl AuthInterceptor {
    pub fn new(jwt_service: JwtService) -> Self {
        Self { jwt_service }
    }

    /// Extract Bearer token from Authorization header
    fn extract_token(&self, metadata: &MetadataMap) -> Result<String, Status> {
        let auth_header = metadata
            .get("authorization")
            .ok_or_else(|| Status::unauthenticated("Missing authorization header"))?
            .to_str()
            .map_err(|_| Status::unauthenticated("Invalid authorization header format"))?;

        if !auth_header.starts_with("Bearer ") && !auth_header.starts_with("bearer ") {
            return Err(Status::unauthenticated(
                "Invalid authorization header format",
            ));
        }

        Ok(auth_header[7..].to_string())
    }

    /// Inject UserContext - validates JWT and extracts user_id
    /// Used for UserService and AdminService
    pub fn inject_user<T>(&self, mut request: Request<T>) -> Result<Request<T>, Status> {
        // Extract and verify JWT (synchronous)
        let token = self.extract_token(request.metadata())?;

        let claims = self
            .jwt_service
            .verify_access_token(&token)
            .map_err(|e| Status::unauthenticated(format!("Token verification failed: {}", e)))?;

        // Inject UserContext with user_id
        let user_context = UserContext {
            user_id: claims.sub,
        };
        request.extensions_mut().insert(user_context);

        Ok(request)
    }

    /// Inject RoomContext - validates JWT, extracts user_id and room_id from x-room-id header
    /// Used for RoomService and MediaService
    pub fn inject_room<T>(&self, mut request: Request<T>) -> Result<Request<T>, Status> {
        // Extract and verify JWT (synchronous)
        let token = self.extract_token(request.metadata())?;

        let claims = self
            .jwt_service
            .verify_access_token(&token)
            .map_err(|e| Status::unauthenticated(format!("Token verification failed: {}", e)))?;

        // Extract room_id from x-room-id header
        let room_id = request
            .metadata()
            .get("x-room-id")
            .ok_or_else(|| Status::invalid_argument("Missing x-room-id header"))?
            .to_str()
            .map_err(|_| Status::invalid_argument("Invalid x-room-id header"))?
            .to_string();

        // Inject UserContext (for nested structure)
        let user_context = UserContext {
            user_id: claims.sub.clone(),
        };
        request.extensions_mut().insert(user_context);

        // Inject RoomContext
        let room_context = RoomContext {
            user_ctx: UserContext {
                user_id: claims.sub,
            },
            room_id,
        };
        request.extensions_mut().insert(room_context);

        Ok(request)
    }
}

impl std::fmt::Debug for AuthInterceptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthInterceptor").finish()
    }
}
