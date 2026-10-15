//! Buckets: a name checked against the S3 naming rules once, when it is made, and what a bucket listing reports.

use chrono::{DateTime, Utc};

use crate::constants::{BUCKET_NAME_MAX_CHARS, BUCKET_NAME_MIN_CHARS};
use crate::{Error, Result};

/// A bucket name that follows the S3 naming rules for general-purpose buckets.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BucketName(String);

impl BucketName {
    /// `name` as a bucket name.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] naming the rule `name` breaks.
    pub fn new(name: &str) -> Result<Self> {
        bucket_rule(name).map_or_else(
            || Ok(Self(name.to_owned())),
            |rule| Err(invalid("bucket name", rule)),
        )
    }

    /// The name as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BucketName {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A bucket as `ListBuckets` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketInfo {
    /// The bucket's name, as the server reported it.
    pub name: String,
    /// When the bucket was created, when the server says.
    pub created: Option<DateTime<Utc>>,
}

pub(crate) const fn invalid(what: &'static str, rule: &'static str) -> Error {
    Error::InvalidArgument { what, rule }
}

const RESERVED_PREFIXES: [&str; 3] = ["xn--", "sthree-", "amzn-s3-demo-"];
const RESERVED_SUFFIXES: [&str; 5] = ["-s3alias", "--ol-s3", ".mrap", "--x-s3", "--table-s3"];

/// The first naming rule `name` breaks, if any.
fn bucket_rule(name: &str) -> Option<&'static str> {
    let bytes = name.as_bytes();
    if !(BUCKET_NAME_MIN_CHARS..=BUCKET_NAME_MAX_CHARS).contains(&bytes.len()) {
        return Some("3 to 63 characters");
    }
    if !bytes.iter().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
    }) {
        return Some("only lowercase letters, digits, `.` and `-`");
    }
    let alphanumeric =
        |byte: Option<&u8>| byte.is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    if !alphanumeric(bytes.first()) || !alphanumeric(bytes.last()) {
        return Some("begins and ends with a letter or a digit");
    }
    if name.contains("..") {
        return Some("no two adjacent `.`");
    }
    if name.parse::<std::net::Ipv4Addr>().is_ok() {
        return Some("not an IP address");
    }
    if RESERVED_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
        || RESERVED_SUFFIXES
            .iter()
            .any(|suffix| name.ends_with(suffix))
    {
        return Some("no reserved prefix or suffix");
    }
    None
}

#[cfg(test)]
#[path = "bucket_tests.rs"]
mod tests;
