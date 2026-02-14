//! Shared utilities for the livestream crate.

/// Exponential backoff with jitter.
///
/// Delays for `initial_ms * 2^(attempt-1)` capped at `max_ms`, with +/- 25% jitter
/// to prevent thundering herd on retry storms.
pub async fn backoff(attempt: u32, initial_ms: u64, max_ms: u64) {
    let base = initial_ms.saturating_mul(1u64 << attempt.min(16).saturating_sub(1));
    let capped = base.min(max_ms);
    // Add jitter: +/- 25%
    let jitter = capped / 4;
    let random_offset = u64::from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos(),
    ) % (jitter * 2 + 1);
    let delay = (capped.saturating_sub(jitter) + random_offset).min(max_ms);
    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
}
