//! What can go wrong in this crate, as one error type the caller can match on.
//!
//! Every error belongs to one [`ErrorCategory`] — `validation`, `not_found`, `conflict`, `unauthorized`, `forbidden`,
//! `timeout`, `unavailable` or `internal` — and the category decides how a caller reacts, never the message text. An
//! answer from the server keeps the S3 error code, HTTP status and request ID it carried; its category comes from the
//! code when the code is one S3 documents, otherwise from the status. A request the server refuses as not
//! implemented is `validation`: asking again will not help, asking differently might.

use std::time::Duration;

/// An error from the client.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A value the caller passed does not satisfy the rule named in `rule`; nothing was sent.
    #[error("invalid {what}: {rule}")]
    InvalidArgument {
        /// Which argument, e.g. "bucket name".
        what: &'static str,
        /// The rule it breaks, e.g. "3 to 63 characters".
        rule: &'static str,
    },
    /// The server answered with an error.
    #[error("the server refused the request: HTTP {status}{}", code.as_deref().map(|code| format!(" {code}")).unwrap_or_default())]
    Service {
        /// The HTTP status.
        status: u16,
        /// The S3 error code, e.g. `NoSuchKey`; `None` when the answer carried none (a `HEAD`, or a proxy's page).
        code: Option<String>,
        /// The server's message, when it sent one.
        message: Option<String>,
        /// The request ID, for the server's logs.
        request_id: Option<String>,
    },
    /// The request did not complete within its time budget.
    #[error("the request did not complete within {0:?}")]
    Timeout(Duration),
    /// The server could not be reached, or the connection failed mid-request.
    #[error("the server could not be reached")]
    Transport(#[source] TransportFailure),
    /// The server's answer is not the shape S3 defines for it.
    #[error("the server's answer is not valid: {what}")]
    InvalidResponse {
        /// What was wrong with it.
        what: &'static str,
    },
    /// Every attempt the retry policy allowed failed with a transient error; `last` is the final one.
    #[error("gave up after {attempts} attempts")]
    RetriesExhausted {
        /// How many attempts were made.
        attempts: u32,
        /// The last attempt's error.
        #[source]
        last: Box<Error>,
    },
    /// The client could not be set up.
    #[error("the client cannot be set up: {what}")]
    Setup {
        /// What could not be set up.
        what: &'static str,
    },
}

/// The cause of a [`Error::Transport`], kept as a source without exposing the HTTP library's error type.
#[derive(Debug)]
pub struct TransportFailure(pub(crate) Box<dyn std::error::Error + Send + Sync>);

impl std::fmt::Display for TransportFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for TransportFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

/// What kind of failure an [`Error`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    /// The request was invalid; asking again unchanged fails again.
    Validation,
    /// The bucket, key or upload does not exist.
    NotFound,
    /// The resource's state forbids it now: it exists, is not empty, or changed since it was read.
    Conflict,
    /// The credentials were not accepted.
    Unauthorized,
    /// The credentials were accepted and do not allow this.
    Forbidden,
    /// The request ran out of time.
    Timeout,
    /// The server was unreachable, overloaded or failing; it may work later.
    Unavailable,
    /// The client or the server broke its own contract.
    Internal,
}

impl Error {
    /// This error's category.
    #[must_use]
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::InvalidArgument { .. } => ErrorCategory::Validation,
            Self::Service { status, code, .. } => code
                .as_deref()
                .and_then(code_category)
                .unwrap_or_else(|| status_category(*status)),
            Self::Timeout(_) => ErrorCategory::Timeout,
            Self::Transport(_) => ErrorCategory::Unavailable,
            Self::InvalidResponse { .. } | Self::Setup { .. } => ErrorCategory::Internal,
            Self::RetriesExhausted { last, .. } => last.category(),
        }
    }

    /// Whether the same request may succeed if sent again: a timeout, a lost connection, throttling, or a server
    /// failure (500, 502, 503, 504) — the set the AWS SDKs retry.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Timeout(_) | Self::Transport(_) => true,
            Self::Service { status, code, .. } => {
                matches!(status, 429 | 500 | 502 | 503 | 504)
                    || code
                        .as_deref()
                        .is_some_and(|code| RETRYABLE_CODES.contains(&code))
            }
            Self::InvalidArgument { .. }
            | Self::InvalidResponse { .. }
            | Self::RetriesExhausted { .. }
            | Self::Setup { .. } => false,
        }
    }

    /// The S3 error code the server sent, e.g. `NoSuchBucket`.
    #[must_use]
    pub fn s3_code(&self) -> Option<&str> {
        match self {
            Self::Service { code, .. } => code.as_deref(),
            Self::RetriesExhausted { last, .. } => last.s3_code(),
            _ => None,
        }
    }
}

/// Codes the AWS SDKs retry whatever their status.
const RETRYABLE_CODES: [&str; 6] = [
    "RequestTimeout",
    "RequestTimeoutException",
    "PriorRequestNotComplete",
    "SlowDown",
    "Throttling",
    "ThrottlingException",
];

fn code_category(code: &str) -> Option<ErrorCategory> {
    use ErrorCategory::{
        Conflict, Forbidden, NotFound, Timeout, Unauthorized, Unavailable, Validation,
    };
    Some(match code {
        "NoSuchBucket" | "NoSuchKey" | "NoSuchUpload" | "NoSuchVersion" => NotFound,
        "BucketAlreadyExists"
        | "BucketAlreadyOwnedByYou"
        | "BucketNotEmpty"
        | "OperationAborted"
        | "ConditionalRequestConflict"
        | "PreconditionFailed"
        | "InvalidBucketState" => Conflict,
        "InvalidAccessKeyId"
        | "SignatureDoesNotMatch"
        | "RequestTimeTooSkewed"
        | "ExpiredToken"
        | "MissingAuthenticationToken"
        | "InvalidToken" => Unauthorized,
        "AccessDenied" => Forbidden,
        "RequestTimeout" => Timeout,
        "SlowDown" | "ServiceUnavailable" | "InternalError" => Unavailable,
        "InvalidArgument"
        | "InvalidBucketName"
        | "InvalidRequest"
        | "MalformedXML"
        | "KeyTooLongError"
        | "EntityTooLarge"
        | "EntityTooSmall"
        | "InvalidDigest"
        | "BadDigest"
        | "InvalidPart"
        | "InvalidPartOrder"
        | "MetadataTooLarge"
        | "InvalidRange"
        | "NotImplemented"
        | "MethodNotAllowed"
        | "MissingContentLength"
        | "IncompleteBody" => Validation,
        _ => return None,
    })
}

fn status_category(status: u16) -> ErrorCategory {
    match status {
        401 => ErrorCategory::Unauthorized,
        403 => ErrorCategory::Forbidden,
        404 => ErrorCategory::NotFound,
        408 => ErrorCategory::Timeout,
        409 | 412 => ErrorCategory::Conflict,
        429 | 500..=599 => ErrorCategory::Unavailable,
        400..=499 => ErrorCategory::Validation,
        _ => ErrorCategory::Internal,
    }
}

/// The result of a client call.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
