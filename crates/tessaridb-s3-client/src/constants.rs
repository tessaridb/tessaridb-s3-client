//! The numbers the S3 protocol fixes, each with its unit and the reason it is what it is.

/// The shortest presigned URL lifetime, in seconds: `X-Amz-Expires` must be at least 1.
pub(crate) const PRESIGN_MIN_SECS: u64 = 1;

/// The longest presigned URL lifetime, in seconds — seven days, because a SigV4 signing key is valid for at most seven.
pub(crate) const PRESIGN_MAX_SECS: u64 = 604_800;
