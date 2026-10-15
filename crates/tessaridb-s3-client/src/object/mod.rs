//! Objects: a key checked once, when it is made, so nothing invalid is ever sent; and what listings and `HEAD`
//! report about objects.

mod listing;

pub use listing::{ListObjects, ListPage, ObjectInfo};

use crate::Result;
use crate::bucket::invalid;
use crate::constants::OBJECT_KEY_MAX_BYTES;

/// An object key: 1 to 1 024 bytes of UTF-8, compared byte by byte as S3 orders keys.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectKey(String);

impl ObjectKey {
    /// `key` as an object key.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] when `key` is empty, longer than 1 024 bytes, or has a `.` or `..` path segment —
    /// S3 accepts those, but an HTTP URL collapses them, so the key that arrived would not be the key that was signed.
    pub fn new(key: &str) -> Result<Self> {
        if key.is_empty() {
            return Err(invalid("object key", "at least one byte"));
        }
        if key.len() > OBJECT_KEY_MAX_BYTES {
            return Err(invalid("object key", "at most 1024 bytes of UTF-8"));
        }
        if key
            .split('/')
            .any(|segment| segment == "." || segment == "..")
        {
            return Err(invalid(
                "object key",
                "no `.` or `..` path segment (a URL would collapse it)",
            ));
        }
        Ok(Self(key.to_owned()))
    }

    /// The key as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ObjectKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests;
