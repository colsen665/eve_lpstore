use thiserror::Error;

#[derive(Debug, Error)]
pub enum EsiError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("ESI error (status {status}): {message}")]
    Esi { status: u16, message: String },

    #[error("Rate limit exceeded: {remain} errors remain, reset in {reset}s")]
    RateLimit { remain: u32, reset: u32 },

    #[error("Deserialization error: {0}")]
    Deserialize(#[from] serde_json::Error),

    /// Retries of a 420 or a 5xx ran out. `status` is the last one, which says whether ESI was
    /// failing or limiting.
    #[error("Max retries exceeded (last status {status})")]
    Throttle { status: u16 },

    #[error("Invalid URL: {0}")]
    Url(#[from] url::ParseError),

    #[error("Invalid header value: {0}")]
    InvalidHeader(#[from] reqwest::header::InvalidHeaderValue),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Shared(SharedError),
}

/// A cloneable summary of an error, used when one request's result is shared between callers.
///
/// Variants that callers match on keep their fields. Other errors keep their message.
#[derive(Debug, Clone, Error)]
pub enum SharedError {
    #[error("ESI error (status {status}): {message}")]
    Esi { status: u16, message: String },

    #[error("Rate limit exceeded: {remain} errors remain, reset in {reset}s")]
    RateLimit { remain: u32, reset: u32 },

    #[error("Max retries exceeded (last status {status})")]
    Throttle { status: u16 },

    #[error("{0}")]
    Other(String),
}

impl From<&EsiError> for SharedError {
    fn from(error: &EsiError) -> Self {
        match error {
            EsiError::Esi { status, message } => Self::Esi {
                status: *status,
                message: message.clone(),
            },
            EsiError::RateLimit { remain, reset } => Self::RateLimit {
                remain: *remain,
                reset: *reset,
            },
            EsiError::Throttle { status } => Self::Throttle { status: *status },
            other => Self::Other(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_keeps_rate_limit_fields() {
        let error = EsiError::RateLimit { remain: 0, reset: 42 };
        match SharedError::from(&error) {
            SharedError::RateLimit { remain, reset } => assert_eq!((remain, reset), (0, 42)),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn running_out_of_retries_names_the_last_status() {
        let error = EsiError::Throttle { status: 503 };
        assert_eq!(error.to_string(), "Max retries exceeded (last status 503)");
        assert_eq!(
            SharedError::from(&error).to_string(),
            "Max retries exceeded (last status 503)"
        );
    }

    #[test]
    fn shared_keeps_esi_status() {
        let error = EsiError::Esi {
            status: 404,
            message: "missing".to_string(),
        };
        match SharedError::from(&error) {
            SharedError::Esi { status, message } => {
                assert_eq!(status, 404);
                assert_eq!(message, "missing");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }
}
