//! Presigned URLs: a path-style URL whose query carries a SigV4 signature over the method, the path and the host,
//! valid for a bounded time. Whoever holds it can make that one request — nothing else — until it expires.

use super::{Transport, host_header, request_path};
use crate::sign::{canonical_uri, uri_encode};
use crate::{BucketName, ObjectKey, Result};

impl Transport {
    /// The URL that lets its holder send `method` to `bucket/key` for `expires_secs` seconds from now.
    ///
    /// # Errors
    /// [`crate::Error::InvalidArgument`] when `expires_secs` is not 1 to 604 800.
    pub(crate) fn presign(
        &self,
        method: &str,
        bucket: &BucketName,
        key: &ObjectKey,
        expires_secs: u64,
    ) -> Result<String> {
        let path = request_path(Some(bucket), Some(key));
        let mut url = self.base.clone();
        url.set_path(&canonical_uri(&path));
        let query = self
            .signer
            .presign(
                method,
                &path,
                &host_header(&url),
                expires_secs,
                chrono::Utc::now(),
            )?
            .iter()
            .map(|(name, value)| format!("{}={}", uri_encode(name, true), uri_encode(value, true)))
            .collect::<Vec<_>>()
            .join("&");
        url.set_query(Some(&query));
        Ok(url.to_string())
    }
}
