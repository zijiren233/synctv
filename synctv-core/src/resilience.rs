//! Production-grade resilience patterns for external services
//!
//! This module provides timeout, retry configuration, and circuit breaker patterns
//! for production-ready resilience.

pub mod timeout {
    //! Timeout configuration for external service calls

    use std::time::Duration;

    /// Default timeout for database operations
    pub const DB_QUERY_TIMEOUT: Duration = Duration::from_secs(30);

    /// Default timeout for Redis operations
    pub const REDIS_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

    /// Default timeout for external HTTP requests
    pub const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

    /// Default timeout for gRPC calls
    pub const GRPC_CALL_TIMEOUT: Duration = Duration::from_secs(30);

    /// Timeout configuration
    #[derive(Debug, Clone, Copy)]
    pub struct TimeoutConfig {
        pub db_query: Duration,
        pub redis: Duration,
        pub http: Duration,
        pub grpc: Duration,
    }

    impl Default for TimeoutConfig {
        fn default() -> Self {
            Self {
                db_query: DB_QUERY_TIMEOUT,
                redis: REDIS_OPERATION_TIMEOUT,
                http: HTTP_REQUEST_TIMEOUT,
                grpc: GRPC_CALL_TIMEOUT,
            }
        }
    }

    impl TimeoutConfig {
        /// Create custom timeout config
        #[must_use] 
        pub fn new() -> Self {
            Self::default()
        }

        /// Set database query timeout
        #[must_use] 
        pub const fn with_db_query_timeout(mut self, timeout: Duration) -> Self {
            self.db_query = timeout;
            self
        }

        /// Set Redis timeout
        #[must_use] 
        pub const fn with_redis_timeout(mut self, timeout: Duration) -> Self {
            self.redis = timeout;
            self
        }

        /// Set HTTP request timeout
        #[must_use] 
        pub const fn with_http_timeout(mut self, timeout: Duration) -> Self {
            self.http = timeout;
            self
        }

        /// Set gRPC timeout
        #[must_use] 
        pub const fn with_grpc_timeout(mut self, timeout: Duration) -> Self {
            self.grpc = timeout;
            self
        }
    }
}

pub mod retry {
    //! Retry configuration for transient failures

    use std::time::Duration;

    /// Retry configuration
    #[derive(Debug, Clone, Copy)]
    pub struct RetryConfig {
        pub max_attempts: u32,
        pub base_delay_ms: u64,
        pub max_delay_ms: u64,
    }

    impl Default for RetryConfig {
        fn default() -> Self {
            Self {
                max_attempts: 3,
                base_delay_ms: 100,
                max_delay_ms: 5000,
            }
        }
    }

    impl RetryConfig {
        /// Create custom retry config
        #[must_use] 
        pub fn new() -> Self {
            Self::default()
        }

        /// Set max retry attempts
        #[must_use] 
        pub const fn with_max_attempts(mut self, attempts: u32) -> Self {
            self.max_attempts = attempts;
            self
        }

        /// Set base delay between retries
        #[must_use] 
        pub const fn with_base_delay(mut self, delay: Duration) -> Self {
            self.base_delay_ms = delay.as_millis() as u64;
            self
        }

        /// Set max delay between retries
        #[must_use] 
        pub const fn with_max_delay(mut self, delay: Duration) -> Self {
            self.max_delay_ms = delay.as_millis() as u64;
            self
        }
    }

    /// Check if an error should be retried
    ///
    /// Checks the error for known transient I/O error kinds, then falls back to
    /// string matching for errors that don't expose `std::io::Error` directly.
    pub fn should_retry_error(err: &(dyn std::error::Error + 'static), attempt: u32, max_attempts: u32) -> bool {
        if attempt >= max_attempts {
            return false;
        }

        // Check top-level error for std::io::Error with transient kinds
        if let Some(io_err) = err.downcast_ref::<std::io::Error>() {
            return is_transient_io_error(io_err);
        }

        // Fallback: check the display message for transient indicators.
        // This covers wrapped error types (e.g. hyper, tonic, anyhow) that
        // include the underlying I/O error message in their Display output.
        let err_msg = err.to_string().to_lowercase();
        err_msg.contains("timed out")
            || err_msg.contains("timeout")
            || err_msg.contains("connection reset")
            || err_msg.contains("connection refused")
            || err_msg.contains("connection aborted")
            || err_msg.contains("broken pipe")
    }

    /// Check if an I/O error is transient and worth retrying
    fn is_transient_io_error(err: &std::io::Error) -> bool {
        matches!(
            err.kind(),
            std::io::ErrorKind::TimedOut
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionRefused
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::UnexpectedEof
        )
    }

    /// Calculate delay before next retry using exponential backoff
    #[must_use] 
    pub fn calculate_retry_delay(attempt: u32, base_delay_ms: u64, max_delay_ms: u64) -> Duration {
        let delay_ms = (base_delay_ms * 2_u64.pow(attempt.saturating_sub(1))).min(max_delay_ms);
        Duration::from_millis(delay_ms)
    }
}

pub mod circuit_breaker {
    //! Circuit breaker pattern for external services

    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// Circuit breaker state
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum CircuitState {
        Closed,
        Open,
        HalfOpen,
    }

    /// Circuit breaker configuration
    #[derive(Debug, Clone)]
    pub struct CircuitBreakerConfig {
        pub failure_threshold: u32,
        pub success_threshold: u32,
        pub timeout: Duration,
    }

    impl Default for CircuitBreakerConfig {
        fn default() -> Self {
            Self {
                failure_threshold: 5,
                success_threshold: 2,
                timeout: Duration::from_mins(1),
            }
        }
    }

    /// Simple circuit breaker
    ///
    /// Uses `parking_lot::Mutex` instead of `std::sync::Mutex` to avoid
    /// blocking the async runtime (`parking_lot` never yields to the OS scheduler
    /// for short critical sections like this).
    #[derive(Debug, Clone)]
    pub struct CircuitBreaker {
        config: CircuitBreakerConfig,
        state: Arc<parking_lot::Mutex<CircuitBreakerState>>,
    }

    #[derive(Debug)]
    struct CircuitBreakerState {
        state: CircuitState,
        failures: u32,
        successes: u32,
        last_failure_time: Option<Instant>,
        open_since: Option<Instant>,
    }

    impl CircuitBreaker {
        /// Create new circuit breaker
        #[must_use] 
        pub fn new(config: CircuitBreakerConfig) -> Self {
            Self {
                config,
                state: Arc::new(parking_lot::Mutex::new(CircuitBreakerState {
                    state: CircuitState::Closed,
                    failures: 0,
                    successes: 0,
                    last_failure_time: None,
                    open_since: None,
                })),
            }
        }

        /// Check if request is allowed
        #[must_use] 
        pub fn allow_request(&self) -> bool {
            let mut state = self.state.lock();

            match state.state {
                CircuitState::Closed => true,
                CircuitState::Open => {
                    // Check if we should transition to HalfOpen
                    if let Some(open_since) = state.open_since {
                        if open_since.elapsed() >= self.config.timeout {
                            state.state = CircuitState::HalfOpen;
                            state.successes = 0;
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                }
                CircuitState::HalfOpen => true,
            }
        }

        /// Record successful request
        pub fn record_success(&self) {
            let mut state = self.state.lock();

            match state.state {
                CircuitState::HalfOpen => {
                    state.successes += 1;
                    if state.successes >= self.config.success_threshold {
                        state.state = CircuitState::Closed;
                        state.failures = 0;
                    }
                }
                CircuitState::Closed => {
                    state.failures = 0;
                }
                CircuitState::Open => {
                    // Should not happen
                }
            }
        }

        /// Record failed request
        pub fn record_failure(&self) {
            let mut state = self.state.lock();

            match state.state {
                CircuitState::Closed | CircuitState::HalfOpen => {
                    state.failures += 1;
                    state.last_failure_time = Some(Instant::now());

                    if state.failures >= self.config.failure_threshold {
                        state.state = CircuitState::Open;
                        state.open_since = Some(Instant::now());
                    }
                }
                CircuitState::Open => {
                    // Already open, just update failure time
                    state.last_failure_time = Some(Instant::now());
                }
            }
        }

        /// Get current state
        #[must_use] 
        pub fn state(&self) -> CircuitState {
            let state = self.state.lock();
            state.state
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use timeout::TimeoutConfig;
    use retry::RetryConfig;
    use circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitState};

    #[test]
    fn test_timeout_config() {
        let config = TimeoutConfig::new()
            .with_db_query_timeout(Duration::from_secs(60));

        assert_eq!(config.db_query.as_secs(), 60);
    }

    #[test]
    fn test_retry_config() {
        let config = RetryConfig::new()
            .with_max_attempts(5);

        assert_eq!(config.max_attempts, 5);
    }

    #[test]
    fn test_circuit_breaker() {
        let config = CircuitBreakerConfig::default();
        let cb = CircuitBreaker::new(config);

        // Initially closed
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.allow_request());

        // Record failures
        for _ in 0..5 {
            cb.record_failure();
        }

        // Should be open now
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(!cb.allow_request());
    }

    #[test]
    fn test_retry_delay_calculation() {
        // Exponential backoff (1-based: attempt 0 and 1 both give base delay)
        assert_eq!(
            retry::calculate_retry_delay(0, 100, 5000).as_millis(),
            100
        );
        assert_eq!(
            retry::calculate_retry_delay(1, 100, 5000).as_millis(),
            100
        );
        assert_eq!(
            retry::calculate_retry_delay(2, 100, 5000).as_millis(),
            200
        );
        assert_eq!(
            retry::calculate_retry_delay(3, 100, 5000).as_millis(),
            400
        );
        // Should cap at max_delay
        assert_eq!(
            retry::calculate_retry_delay(10, 100, 5000).as_millis(),
            5000
        );
    }
}
