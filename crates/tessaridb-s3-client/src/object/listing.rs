//! What a listing asks for and what it answers: one page of objects and the token for the next, and what `HEAD`
//! reports.

use chrono::{DateTime, Utc};

use super::ObjectKey;

/// One object's metadata, from a listing or from `HEAD`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectInfo {
    /// The object's key.
    pub key: String,
    /// The size in bytes.
    pub size: u64,
    /// The entity tag with its quotes, e.g. `"9b2cf535f27731c974343645a3985328"`.
    pub etag: String,
    /// When the object was last written, when the server says.
    pub last_modified: Option<DateTime<Utc>>,
}

/// One page of a `ListObjectsV2` listing, in the UTF-8 byte order of the keys.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListPage {
    /// The objects on this page.
    pub objects: Vec<ObjectInfo>,
    /// The prefixes the delimiter rolled keys up into, each once.
    pub common_prefixes: Vec<String>,
    /// The token that resumes after this page; `None` on the last page.
    pub next_continuation_token: Option<String>,
}

/// What to list: everything under `prefix`, rolled up at `delimiter`, at most `max_keys` per page.
#[derive(Debug, Clone, Default)]
pub struct ListObjects {
    /// Only keys starting with this.
    pub prefix: Option<String>,
    /// Keys containing this after the prefix roll up into one common prefix.
    pub delimiter: Option<String>,
    /// The page size, 1 to 1 000; the server's default (1 000) when `None`.
    pub max_keys: Option<u16>,
    /// The token a previous page returned.
    pub continuation_token: Option<String>,
    /// Start after this key (the first page only).
    pub start_after: Option<ObjectKey>,
}
