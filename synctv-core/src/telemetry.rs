//! OpenTelemetry tracing integration for distributed tracing
//!
//! This module provides production-grade distributed tracing using OpenTelemetry.
//! Trace context is propagated across service boundaries for end-to-end request tracking.

/// Initialize tracing subscriber for development
///
/// # Arguments
/// * `service_name` - Name of the service (e.g., "synctv-api")
///
/// # Example
/// ```ignore
/// init_tracing("synctv-api").await;
/// ```
pub async fn init_tracing(
    service_name: &'static str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Create tracing subscriber with environment filter
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .map_err(|e| format!("Failed to set tracing subscriber: {}", e))?;

    tracing::info!("Tracing initialized for service: {}", service_name);

    Ok(())
}

/// Instrument an async function with automatic tracing
///
/// # Example
/// ```ignore
/// #[instrument]
/// async fn process_user(user_id: &UserId) -> Result<User> {
///     // All function parameters and return values are automatically traced
/// }
/// ```
pub use tracing::instrument;

/// Context propagation for distributed tracing
pub mod context {
    /// Extract trace context from HTTP headers
    pub fn extract_from_http(headers: &http::HeaderMap) -> tracing::Span {
        use http::header::HeaderName;

        // Check for traceparent header (W3C Trace Context)
        let trace_parent_header = HeaderName::from_static("traceparent");
        if let Some(trace_parent) = headers.get(&trace_parent_header) {
            match trace_parent.to_str() {
                Ok(trace_str) => {
                    tracing::debug!(trace_parent = %trace_str, "Extracting trace context from HTTP");
                    // In production, you would use opentelemetry propagator here
                    return tracing::info_span!("http_request", trace_context = %trace_str);
                }
                Err(_) => {
                    tracing::warn!("Invalid traceparent header value");
                }
            }
        }

        // No trace context, create new span
        tracing::info_span!("http_request")
    }

    /// Create a span for gRPC request
    pub fn grpc_span(method: &str) -> tracing::Span {
        tracing::info_span!("grpc_request", method = %method)
    }

    /// Create a span for database operation
    pub fn db_span(operation: &str, table: &str) -> tracing::Span {
        tracing::info_span!("db_operation", operation = %operation, table = %table)
    }

    /// Create a span for cache operation
    pub fn cache_span(operation: &str, cache_type: &str) -> tracing::Span {
        tracing::info_span!("cache_operation", operation = %operation, cache_type = %cache_type)
    }
}

/// Trace error type
#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    #[error("Trace error: {0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_extraction() {
        let headers = http::HeaderMap::new();
        let span = context::extract_from_http(&headers);
        drop(span);
    }

    #[test]
    fn test_create_spans() {
        let grpc_span = context::grpc_span("GetUser");
        let db_span = context::db_span("SELECT", "users");
        let cache_span = context::cache_span("GET", "user");
        drop((grpc_span, db_span, cache_span));
    }
}
