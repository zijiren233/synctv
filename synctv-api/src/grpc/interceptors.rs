use tonic::{Request, Status, metadata::MetadataMap};
use synctv_core::service::auth::{JwtService, Claims};

/// Authenticated user context
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub user_id: String,
    pub permissions: i64,
}

impl From<Claims> for AuthContext {
    fn from(claims: Claims) -> Self {
        Self {
            user_id: claims.sub,
            permissions: claims.permissions,
        }
    }
}

/// Authentication interceptor
#[derive(Clone)]
pub struct AuthInterceptor {
    jwt_service: JwtService,
    /// Paths that don't require authentication
    skip_auth_paths: Vec<&'static str>,
}

impl AuthInterceptor {
    pub fn new(jwt_service: JwtService) -> Self {
        Self {
            jwt_service,
            skip_auth_paths: vec![
                "/bilibili.BilibiliService/Login",
                "/bilibili.BilibiliService/Register",
                "/bilibili.BilibiliService/NewQRCode",
                "/bilibili.BilibiliService/LoginWithQRCode",
                "/bilibili.BilibiliService/NewCaptcha",
                "/bilibili.BilibiliService/NewSMS",
                "/bilibili.BilibiliService/LoginWithSMS",
                "/bilibili.BilibiliService/Match",
                "/alist.AlistService/Login",
                "/emby.EmbyService/Login",
                "/grpc.health.v1.Health/Check",
            ],
        }
    }

    /// Check if the request path should skip authentication
    fn should_skip_auth(&self, uri: &str) -> bool {
        self.skip_auth_paths.iter().any(|path| uri.contains(path))
    }

    /// Extract Bearer token from Authorization header
    fn extract_token(&self, metadata: &MetadataMap) -> Result<String, Status> {
        let auth_header = metadata
            .get("authorization")
            .ok_or_else(|| Status::unauthenticated("Missing authorization header"))?
            .to_str()
            .map_err(|_| Status::unauthenticated("Invalid authorization header format"))?;

        // Check if it starts with "Bearer "
        if !auth_header.starts_with("Bearer ") && !auth_header.starts_with("bearer ") {
            return Err(Status::unauthenticated("Invalid authorization header format"));
        }

        // Extract token (skip "Bearer " prefix)
        Ok(auth_header[7..].to_string())
    }

    pub fn intercept<T>(&self, mut request: Request<T>, uri: &str) -> Result<Request<T>, Status> {
        // Skip authentication for certain endpoints
        if self.should_skip_auth(uri) {
            return Ok(request);
        }

        // Extract token from metadata
        let token = self.extract_token(request.metadata())?;

        // Verify JWT and extract claims
        let claims = self
            .jwt_service
            .verify_access_token(&token)
            .map_err(|e| Status::unauthenticated(format!("Token verification failed: {}", e)))?;

        // Create auth context and inject into request extensions
        let auth_context = AuthContext::from(claims);
        request.extensions_mut().insert(auth_context);

        Ok(request)
    }
}

impl std::fmt::Debug for AuthInterceptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthInterceptor")
            .field("skip_auth_paths", &self.skip_auth_paths)
            .finish()
    }
}


/// Logging interceptor for request/response logging
#[derive(Debug, Clone)]
pub struct LoggingInterceptor {}

impl LoggingInterceptor {
    pub fn new() -> Self {
        Self {}
    }

    pub fn intercept<T>(&self, request: Request<T>) -> Result<Request<T>, Status> {
        // Log incoming request
        tracing::debug!("gRPC request received");

        Ok(request)
    }
}

impl Default for LoggingInterceptor {
    fn default() -> Self {
        Self::new()
    }
}
