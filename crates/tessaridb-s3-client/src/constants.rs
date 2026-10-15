//! The numbers the S3 protocol fixes, each with its unit and the reason it is what it is.

/// The shortest presigned URL lifetime, in seconds: `X-Amz-Expires` must be at least 1.
pub(crate) const PRESIGN_MIN_SECS: u64 = 1;

/// The longest presigned URL lifetime, in seconds — seven days, because a SigV4 signing key is valid for at most seven.
pub(crate) const PRESIGN_MAX_SECS: u64 = 604_800;

/// The shortest bucket name, in characters.
pub(crate) const BUCKET_NAME_MIN_CHARS: usize = 3;

/// The longest bucket name, in characters.
pub(crate) const BUCKET_NAME_MAX_CHARS: usize = 63;

/// The longest object key, in bytes of UTF-8.
pub(crate) const OBJECT_KEY_MAX_BYTES: usize = 1024;

/// The most keys one listing page can hold — the S3 maximum for `max-keys`.
pub(crate) const LIST_MAX_KEYS: u16 = 1000;

/// The largest XML document the client reads, in bytes: a full listing page of 1 000 keys of 1 024 bytes, each
/// percent-encoded up to three times its length, is about 4 MiB, so 8 MiB leaves room and still bounds memory.
pub(crate) const XML_BODY_MAX_BYTES: usize = 8 * 1024 * 1024;

/// How deeply nested a response document may be; S3 responses are four levels deep at most.
pub(crate) const XML_MAX_DEPTH: usize = 8;

/// How long to wait for a TCP + TLS connection, in seconds.
pub(crate) const CONNECT_TIMEOUT_SECS: u64 = 10;

/// How long a transfer may go without receiving anything, in seconds — the bound on a body of any size.
pub(crate) const READ_TIMEOUT_SECS: u64 = 60;

/// The largest object one `PUT` may carry, in bytes (5 GiB); larger ones go in parts.
pub(crate) const SINGLE_PUT_MAX_BYTES: u64 = 5 * 1024 * 1024 * 1024;

/// How much of a file is read or written at a time, in bytes.
pub(crate) const FILE_CHUNK_BYTES: usize = 1024 * 1024;

/// How long one request may take end to end, in seconds, for calls with a small body.
pub(crate) const REQUEST_TIMEOUT_SECS: u64 = 60;

/// How many times a call is attempted, the first included — the AWS SDKs' standard retry mode.
pub(crate) const RETRY_MAX_ATTEMPTS: u32 = 3;

/// The first backoff ceiling, in milliseconds; each retry doubles it.
pub(crate) const RETRY_BASE_DELAY_MS: u64 = 200;

/// The largest backoff ceiling, in milliseconds.
pub(crate) const RETRY_MAX_DELAY_MS: u64 = 5_000;

/// How long a call may spend across all its attempts before no further one starts, in seconds.
pub(crate) const RETRY_MAX_ELAPSED_SECS: u64 = 120;
