use std::time::Duration;

/// Calculate the delay for a given retry attempt using exponential backoff with jitter.
pub fn backoff_delay(attempt: u32) -> Duration {
    let base = Duration::from_millis(500);
    let exp = base.saturating_mul(1 << attempt.min(6)); // cap at 64x
    let jitter = Duration::from_millis(rand::random::<u64>() % 200);
    exp + jitter
}

/// Maximum number of retry attempts for transient failures.
pub const MAX_RETRIES: u32 = 5;

/// Maximum number of times one request is sent again after a 429. Each waits the Retry-After ESI
/// gave, which is when enough budget is back, so running out of these means something other than
/// a spent budget is refusing the request.
pub const MAX_RATE_LIMIT_RETRIES: u32 = 10;
