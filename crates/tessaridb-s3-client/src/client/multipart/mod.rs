//! Large files in parts. One read of the file plans every part — its range and its SHA-256, MD5 and CRC64NVME — and
//! the whole file's CRC64NVME; up to four parts are then sent at a time, each signed with its own hash and declaring
//! its own checksum, and the upload is completed with the whole-file CRC64NVME, which the server checks against the
//! parts it combined. An upload whose parts keep failing transiently is left open and reported with its ID, so
//! `resume_upload` can finish it later, sending only the parts the server does not already hold intact.

use std::fmt::Write as _;
use std::path::Path;

use futures_util::{StreamExt as _, TryStreamExt as _, stream};
use reqwest::Method;

use super::Client;
use super::objects::upload_checksum;
use crate::checksum::UPLOAD_ALGORITHM;
use crate::constants::PART_CONCURRENCY;
use crate::transport::{Payload, Request};
use crate::{BucketName, Error, ObjectKey, Result, xml};

mod plan;

use plan::{FilePlan, PartPlan, missing, plan};

impl Client {
    /// Opens a multipart upload of `bucket/key` declaring a whole-object CRC64NVME, and returns its upload ID.
    ///
    /// # Errors
    /// The server's refusal, or a transport failure after retries.
    pub async fn begin_upload(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        content_type: Option<&str>,
    ) -> Result<String> {
        let mut request = Request::new(Method::POST, Some(bucket), Some(key));
        request.query.push(("uploads".to_owned(), String::new()));
        request.headers.push((
            "x-amz-checksum-algorithm".to_owned(),
            UPLOAD_ALGORITHM.to_owned(),
        ));
        request
            .headers
            .push(("x-amz-checksum-type".to_owned(), "FULL_OBJECT".to_owned()));
        if let Some(content_type) = content_type {
            request
                .headers
                .push(("content-type".to_owned(), content_type.to_owned()));
        }
        xml::upload_id(&self.call(&request).await?.body)
    }

    /// Finishes the multipart upload `upload_id` of the file at `path` as `bucket/key`: sends every part the server
    /// does not already hold intact, completes the upload and returns the object's ETag.
    ///
    /// # Errors
    /// `NotFound` (`NoSuchUpload`) for an upload that was completed or aborted; [`Error::Io`] when the file cannot be
    /// read; the server's refusal or a transport failure after retries otherwise. The upload stays open on failure.
    pub async fn resume_upload(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        path: impl AsRef<Path>,
        upload_id: &str,
    ) -> Result<String> {
        let path = path.as_ref().to_path_buf();
        let length = tokio::fs::metadata(&path)
            .await
            .map_err(|source| Error::Io {
                what: "the file to upload",
                source,
            })?
            .len();
        let plan = plan(path.clone(), length).await?;
        let listed = self.list_all_parts(bucket, key, upload_id).await?;
        let sends = missing(&plan, &listed)
            .into_iter()
            .map(|part| self.upload_part(bucket, key, upload_id, &path, plan.length, part));
        stream::iter(sends)
            .buffer_unordered(PART_CONCURRENCY)
            .try_collect::<Vec<()>>()
            .await?;
        self.complete(bucket, key, upload_id, &plan).await
    }

    /// Uploads the file at `path` in parts: a transient failure leaves the upload open and reports its ID, a
    /// permanent one aborts it.
    pub(super) async fn upload_in_parts(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        path: &Path,
        content_type: Option<&str>,
    ) -> Result<String> {
        let upload_id = self.begin_upload(bucket, key, content_type).await?;
        match self.resume_upload(bucket, key, path, &upload_id).await {
            Ok(etag) => Ok(etag),
            Err(error)
                if error.is_retryable() || matches!(error, Error::RetriesExhausted { .. }) =>
            {
                Err(Error::UploadInterrupted {
                    upload_id,
                    last: Box::new(error),
                })
            }
            Err(error) => {
                if let Err(abort) = self.abort_upload(bucket, key, &upload_id).await {
                    tracing::warn!(upload_id = %upload_id, error = %abort, "the failed upload could not be aborted");
                }
                Err(error)
            }
        }
    }

    /// Abandons the multipart upload `upload_id`; the server discards its parts.
    ///
    /// # Errors
    /// The server's refusal, or a transport failure after retries.
    pub async fn abort_upload(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        upload_id: &str,
    ) -> Result<()> {
        let mut request = Request::new(Method::DELETE, Some(bucket), Some(key));
        request
            .query
            .push(("uploadId".to_owned(), upload_id.to_owned()));
        self.call(&request).await.map(drop)
    }

    async fn upload_part(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        upload_id: &str,
        path: &Path,
        file_length: u64,
        part: &PartPlan,
    ) -> Result<()> {
        let mut request = Request::new(Method::PUT, Some(bucket), Some(key));
        request
            .query
            .push(("partNumber".to_owned(), part.number.to_string()));
        request
            .query
            .push(("uploadId".to_owned(), upload_id.to_owned()));
        request.headers.push(upload_checksum(part.crc.clone()));
        request.payload = Payload::File {
            path: path.to_path_buf(),
            file_length,
            offset: part.offset,
            length: part.length,
            sha256: part.sha256.clone(),
        };
        let request = &request;
        self.retry
            .run(|_| async move {
                let response = self.transport.send_unbounded(request).await?;
                match response
                    .headers
                    .get("etag")
                    .and_then(|value| value.to_str().ok())
                {
                    Some(etag) if etag == part.etag => Ok(()),
                    _ => Err(Error::ChecksumMismatch { algorithm: "MD5" }),
                }
            })
            .await
    }

    async fn complete(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        upload_id: &str,
        plan: &FilePlan,
    ) -> Result<String> {
        let mut document = String::from(
            "<CompleteMultipartUpload xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">",
        );
        for part in &plan.parts {
            let _ = write!(
                document,
                "<Part><PartNumber>{}</PartNumber><ETag>{}</ETag><ChecksumCRC64NVME>{}</ChecksumCRC64NVME></Part>",
                part.number,
                part.etag.replace('"', "&quot;"),
                part.crc
            );
        }
        document.push_str("</CompleteMultipartUpload>");
        let mut request = Request::new(Method::POST, Some(bucket), Some(key));
        request
            .query
            .push(("uploadId".to_owned(), upload_id.to_owned()));
        request.headers.push(upload_checksum(plan.crc.clone()));
        request
            .headers
            .push(("x-amz-checksum-type".to_owned(), "FULL_OBJECT".to_owned()));
        request.payload = Payload::Memory(document.into());
        let response = self.call(&request).await?;
        xml::complete_result(&response.body, response.status)
    }

    async fn list_all_parts(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        upload_id: &str,
    ) -> Result<Vec<xml::StoredPart>> {
        let mut listed = Vec::new();
        let mut marker: Option<String> = None;
        loop {
            let mut request = Request::new(Method::GET, Some(bucket), Some(key));
            request
                .query
                .push(("uploadId".to_owned(), upload_id.to_owned()));
            if let Some(marker) = marker.take() {
                request
                    .query
                    .push(("part-number-marker".to_owned(), marker));
            }
            let (page, next) = xml::list_parts(&self.call(&request).await?.body)?;
            listed.extend(page);
            match next {
                Some(next) => marker = Some(next),
                None => return Ok(listed),
            }
        }
    }
}

#[cfg(test)]
mod tests;
