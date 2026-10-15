//! Object operations with bodies held in memory. Files are streamed by the transfer layer, not here.

use bytes::Bytes;
use chrono::{DateTime, Utc};
use reqwest::Method;
use reqwest::header::HeaderMap;

use super::Client;
use crate::constants::LIST_MAX_KEYS;
use crate::sign::uri_encode;
use crate::transport::Request;
use crate::{BucketName, ListObjects, ListPage, ObjectInfo, ObjectKey};
use crate::{Error, Result, xml};

impl Client {
    /// Stores `body` as `bucket/key`, replacing any object there, and returns its ETag (quoted). The body is signed:
    /// the server stores it only if its SHA-256 matches.
    ///
    /// # Errors
    /// The server's refusal, or a transport failure after retries.
    pub async fn put_object(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        body: impl Into<Bytes>,
        content_type: Option<&str>,
    ) -> Result<String> {
        let mut request = Request::new(Method::PUT, Some(bucket), Some(key));
        request.body = body.into();
        if let Some(content_type) = content_type {
            request
                .headers
                .push(("content-type".to_owned(), content_type.to_owned()));
        }
        let response = self.call(&request).await?;
        etag(&response.headers)
    }

    /// The whole of `bucket/key` in memory.
    ///
    /// # Errors
    /// `NotFound` (`NoSuchKey`, `NoSuchBucket`), or another refusal.
    pub async fn get_object(&self, bucket: &BucketName, key: &ObjectKey) -> Result<Bytes> {
        let response = self
            .call(&Request::new(Method::GET, Some(bucket), Some(key)))
            .await?;
        Ok(response.body)
    }

    /// The metadata of `bucket/key` without its body.
    ///
    /// # Errors
    /// `NotFound` (an answer to `HEAD` carries no code), or another refusal.
    pub async fn head_object(&self, bucket: &BucketName, key: &ObjectKey) -> Result<ObjectInfo> {
        let response = self
            .call(&Request::new(Method::HEAD, Some(bucket), Some(key)))
            .await?;
        let size = header(&response.headers, "content-length")
            .and_then(|length| length.parse().ok())
            .ok_or(Error::InvalidResponse {
                what: "a HEAD answer without a size",
            })?;
        let last_modified = header(&response.headers, "last-modified")
            .map(|text| {
                DateTime::parse_from_rfc2822(text)
                    .map(|time| time.with_timezone(&Utc))
                    .map_err(|_| Error::InvalidResponse {
                        what: "a Last-Modified that is not an HTTP date",
                    })
            })
            .transpose()?;
        Ok(ObjectInfo {
            key: key.to_string(),
            size,
            etag: etag(&response.headers)?,
            last_modified,
        })
    }

    /// Deletes `bucket/key`; deleting a key that does not exist succeeds, as S3 defines it.
    ///
    /// # Errors
    /// The server's refusal, or a transport failure after retries.
    pub async fn delete_object(&self, bucket: &BucketName, key: &ObjectKey) -> Result<()> {
        self.call(&Request::new(Method::DELETE, Some(bucket), Some(key)))
            .await
            .map(drop)
    }

    /// Copies `source_bucket/source_key` to `bucket/key` on the server, metadata included, and returns the copy's
    /// ETag.
    ///
    /// # Errors
    /// `NotFound` for a missing source, or another refusal — including one the server reports inside a 200.
    pub async fn copy_object(
        &self,
        source_bucket: &BucketName,
        source_key: &ObjectKey,
        bucket: &BucketName,
        key: &ObjectKey,
    ) -> Result<String> {
        let mut request = Request::new(Method::PUT, Some(bucket), Some(key));
        request.headers.push((
            "x-amz-copy-source".to_owned(),
            format!(
                "/{source_bucket}/{}",
                uri_encode(source_key.as_str(), false)
            ),
        ));
        let response = self.call(&request).await?;
        xml::copy_result(&response.body, response.status)
    }

    /// One page of the objects in `bucket`, keys in UTF-8 byte order; pass the page's `next_continuation_token` back
    /// in `listing.continuation_token` for the next one.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] for `max_keys` outside 1 to 1 000; the server's refusal otherwise.
    pub async fn list_objects(
        &self,
        bucket: &BucketName,
        listing: &ListObjects,
    ) -> Result<ListPage> {
        let mut request = Request::new(Method::GET, Some(bucket), None);
        request.query.push(("list-type".to_owned(), "2".to_owned()));
        request
            .query
            .push(("encoding-type".to_owned(), "url".to_owned()));
        if let Some(max_keys) = listing.max_keys {
            if !(1..=LIST_MAX_KEYS).contains(&max_keys) {
                return Err(Error::InvalidArgument {
                    what: "max keys",
                    rule: "1 to 1000",
                });
            }
            request
                .query
                .push(("max-keys".to_owned(), max_keys.to_string()));
        }
        let optional = [
            ("prefix", listing.prefix.as_deref()),
            ("delimiter", listing.delimiter.as_deref()),
            ("continuation-token", listing.continuation_token.as_deref()),
            (
                "start-after",
                listing.start_after.as_ref().map(ObjectKey::as_str),
            ),
        ];
        for (name, value) in optional {
            if let Some(value) = value {
                request.query.push((name.to_owned(), value.to_owned()));
            }
        }
        let response = self.call(&request).await?;
        xml::list_objects_v2(&response.body)
    }
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn etag(headers: &HeaderMap) -> Result<String> {
    header(headers, "etag")
        .map(str::to_owned)
        .ok_or(Error::InvalidResponse {
            what: "an answer without an ETag",
        })
}
