use std::sync::Arc;
use synctv_core::service::auth::{JwtService, JwtValidator};
use tonic::{metadata::MetadataMap, Request, Status};
use tracing::warn;
use std::fmt::Debug;

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
    jwt_validator: Arc<JwtValidator>,
}

impl AuthInterceptor {
    pub fn new(jwt_service: JwtService) -> Self {
        Self {
            jwt_validator: Arc::new(JwtValidator::new(Arc::new(jwt_service))),
        }
    }

    /// Inject UserContext - validates JWT and extracts user_id
    /// Used for UserService and AdminService
    pub fn inject_user<T>(&self, mut request: Request<T>) -> Result<Request<T>, Status> {
        // Use unified validator for gRPC validation
        let claims = self
            .jwt_validator
            .validate_grpc_as_status(request.metadata())?;

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
        // Use unified validator for gRPC validation
        let claims = self
            .jwt_validator
            .validate_grpc_as_status(request.metadata())?;

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

/// Logging interceptor for gRPC requests
///
/// Logs incoming requests with method name, timing, and status.
#[derive(Clone)]
pub struct LoggingInterceptor;

impl LoggingInterceptor {
    pub fn new() -> Self {
        Self
    }

    /// Log request with method name and timing
    pub fn log<T>(&self, method: &'static str, request: Request<T>) -> Request<T> {
        let metadata = request.metadata();
        let user_agent = metadata
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown");

        tracing::debug!(
            method = method,
            user_agent = user_agent,
            "Incoming gRPC request"
        );

        request
    }
}

impl Default for LoggingInterceptor {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for LoggingInterceptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoggingInterceptor").finish()
    }
}

/// Request validation interceptor
///
/// Validates common request constraints like size limits.
#[derive(Clone)]
pub struct ValidationInterceptor {
    max_request_size_mb: usize,
}

impl ValidationInterceptor {
    pub fn new(max_request_size_mb: usize) -> Self {
        Self {
            max_request_size_mb,
        }
    }

    /// Validate request size
    pub fn validate<T>(&self, method: &'static str, request: &Request<T>) -> Result<(), Status> {
        // Get content-length if available
        if let Some(content_length) = request.metadata().get("content-length") {
            let length_str = content_length
                .to_str()
                .map_err(|_| Status::invalid_argument("Invalid content-length header"))?;

            if let Ok(size_bytes) = length_str.parse::<usize>() {
                let max_bytes = self.max_request_size_mb * 1024 * 1024;
                if size_bytes > max_bytes {
                    warn!(
                        method = method,
                        size_bytes = size_bytes,
                        max_bytes = max_bytes,
                        "Request too large"
                    );
                    return Err(Status::resource_exhausted(format!(
                        "Request too large: {} bytes (max {} MB)",
                        size_bytes, self.max_request_size_mb
                    )));
                }
            }
        }

        Ok(())
    }
}

impl Debug for ValidationInterceptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidationInterceptor")
            .field("max_request_size_mb", &self.max_request_size_mb)
            .finish()
    }
}

/// Timeout/deadline enforcement interceptor
///
/// Ensures requests have appropriate timeout deadlines.
#[derive(Clone)]
pub struct TimeoutInterceptor {
    default_timeout_secs: u64,
}

impl TimeoutInterceptor {
    pub fn new(default_timeout_secs: u64) -> Self {
        Self {
            default_timeout_secs,
        }
    }

    /// Ensure request has a deadline
    pub fn enforce_timeout<T>(&self, _request: &mut Request<T>) {
        // Note: tonic deadlines should be set by the client using gRPC timeout headers
        // This interceptor is a placeholder for future timeout enforcement
        // For now, we rely on the client to set appropriate timeouts
    }
}

impl Debug for TimeoutInterceptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimeoutInterceptor")
            .field("default_timeout_secs", &self.default_timeout_secs)
            .finish()
    }
}
