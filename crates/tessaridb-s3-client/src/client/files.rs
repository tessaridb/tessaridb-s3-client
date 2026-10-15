//! Files moved without being held in memory. An upload hashes the file once (SHA-256, on a blocking thread) and then
//! streams it, opened afresh for every attempt; a download streams into a temporary file beside the target that is
//! synced and renamed into place only when every byte arrived, and removed otherwise — so the target is either the
//! whole object or untouched.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use reqwest::Method;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt as _;

use super::Client;
use super::objects::{checksum_mode, etag, upload_checksum};
use crate::checksum::{Crc64Nvme, UPLOAD_ALGORITHM, Verifier};
use crate::constants::{FILE_CHUNK_BYTES, MULTIPART_THRESHOLD_BYTES};
use crate::sign::hex;
use crate::transport::{Payload, Request, transport_error};
use crate::{BucketName, Error, ObjectKey, Result};

impl Client {
    /// Uploads the file at `path` as `bucket/key` and returns its ETag. The body is signed with the file's SHA-256
    /// and carries its CRC64NVME, so the server stores it only if every byte arrived as hashed. A file over 16 MiB
    /// goes in parts (see `resume_upload`).
    ///
    /// # Errors
    /// [`Error::Io`] when the file cannot be read; [`Error::InvalidArgument`] for a file over 10 000 parts of 5 GiB
    /// or one that changed while being sent; [`Error::UploadInterrupted`] when a multipart upload stopped on a
    /// transient failure and can be resumed; the server's refusal otherwise.
    pub async fn upload_file(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        path: impl AsRef<Path>,
        content_type: Option<&str>,
    ) -> Result<String> {
        let path = path.as_ref().to_path_buf();
        let length = tokio::fs::metadata(&path).await.map_err(to_upload)?.len();
        if length > MULTIPART_THRESHOLD_BYTES {
            return self.upload_in_parts(bucket, key, &path, content_type).await;
        }
        let (sha256, crc) = digests_of(path.clone()).await?;
        let mut request = Request::new(Method::PUT, Some(bucket), Some(key));
        request.headers.push(upload_checksum(crc));
        request.headers.push((
            "x-amz-sdk-checksum-algorithm".to_owned(),
            UPLOAD_ALGORITHM.to_owned(),
        ));
        request.payload = Payload::File {
            path,
            file_length: length,
            offset: 0,
            length,
            sha256,
        };
        if let Some(content_type) = content_type {
            request
                .headers
                .push(("content-type".to_owned(), content_type.to_owned()));
        }
        let request = &request;
        let response = self
            .retry
            .run(|_| self.transport.send_unbounded(request))
            .await?;
        etag(&response.headers)
    }

    /// Downloads `bucket/key` to the file at `path`, replacing it, and returns the number of bytes written. Nothing
    /// is written to `path` unless the whole object arrived and matched every whole-object checksum the server keeps.
    ///
    /// # Errors
    /// `NotFound` for a missing object; [`Error::Io`] when the file cannot be written; [`Error::InvalidResponse`]
    /// when the body is not as long as the server said.
    pub async fn download_file(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        path: impl AsRef<Path>,
    ) -> Result<u64> {
        let path = path.as_ref();
        // Held for the whole of one attempt, across its awaits; attempts run one after another, so it is never
        // contended — it carries the part file and what has been received from one attempt to the next.
        let progress = tokio::sync::Mutex::new(None);
        let progress = &progress;
        self.retry
            .run(|_| async move {
                let mut progress = progress.lock().await;
                self.download_attempt(bucket, key, path, &mut progress)
                    .await
            })
            .await
    }

    /// One attempt: a fresh download, or — when an earlier attempt left bytes in the part file — the rest of the same
    /// object (`Range` from the first missing byte, `If-Match` its ETag).
    async fn download_attempt(
        &self,
        bucket: &BucketName,
        key: &ObjectKey,
        path: &Path,
        progress: &mut Option<Progress>,
    ) -> Result<u64> {
        let mut request = Request::new(Method::GET, Some(bucket), Some(key));
        request.headers.push(checksum_mode());
        // Continuing needs the ETag, so the server can refuse if the object changed in between.
        let resume_from = progress
            .as_ref()
            .filter(|so_far| so_far.written > 0)
            .and_then(|so_far| so_far.etag.clone().map(|etag| (so_far.written, etag)));
        if let Some((from, etag)) = &resume_from {
            request
                .headers
                .push(("range".to_owned(), format!("bytes={from}-")));
            request.headers.push(("if-match".to_owned(), etag.clone()));
        }
        let mut answer = self.transport.open(&request).await?;
        match (&resume_from, answer.status().as_u16()) {
            (Some((from, _)), 206) => {
                let range = answer
                    .headers()
                    .get("content-range")
                    .and_then(|value| value.to_str().ok());
                if !range.is_some_and(|range| range.starts_with(&format!("bytes {from}-"))) {
                    return Err(Error::InvalidResponse {
                        what: "a resumed body that does not start where the last one stopped",
                    });
                }
            }
            _ => {
                let etag = answer
                    .headers()
                    .get("etag")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned);
                *progress = Some(Progress {
                    total: answer.content_length(),
                    verifier: Verifier::from_headers(answer.headers()),
                    part: PartFile::create(path).await?,
                    written: 0,
                    etag,
                });
            }
        }
        let so_far = progress.as_mut().ok_or(Error::InvalidResponse {
            what: "a download with nothing to continue",
        })?;
        while let Some(chunk) = answer.chunk().await.map_err(transport_error)? {
            so_far.verifier.update(&chunk);
            so_far
                .part
                .file
                .write_all(&chunk)
                .await
                .map_err(to_download)?;
            so_far.written = u64::try_from(chunk.len())
                .ok()
                .and_then(|length| so_far.written.checked_add(length))
                .ok_or(Error::InvalidResponse {
                    what: "a body over 2^64 bytes",
                })?;
        }
        // Whatever happens next, this download does not continue: a mismatch starts the next attempt afresh.
        let done = progress.take().ok_or(Error::InvalidResponse {
            what: "a download with nothing to continue",
        })?;
        if done.total.is_some_and(|total| total != done.written) {
            return Err(Error::InvalidResponse {
                what: "a body shorter or longer than its Content-Length",
            });
        }
        done.verifier.finish()?;
        done.part.commit().await?;
        Ok(done.written)
    }
}

fn to_upload(source: std::io::Error) -> Error {
    Error::Io {
        what: "the file to upload",
        source,
    }
}

fn to_download(source: std::io::Error) -> Error {
    Error::Io {
        what: "the file to download into",
        source,
    }
}

/// The SHA-256 (lowercase hex) and CRC64NVME (base64) of the file at `path`, read once on a blocking thread.
async fn digests_of(path: PathBuf) -> Result<(String, String)> {
    tokio::task::spawn_blocking(move || {
        let mut file = std::fs::File::open(&path).map_err(to_upload)?;
        let mut hasher = Sha256::new();
        let mut crc = Crc64Nvme::new();
        let mut buffer = vec![0_u8; FILE_CHUNK_BYTES];
        loop {
            let read = file.read(&mut buffer).map_err(to_upload)?;
            match buffer.get(..read) {
                Some([]) | None => break,
                Some(bytes) => {
                    hasher.update(bytes);
                    crc.update(bytes);
                }
            }
        }
        Ok((hex(&hasher.finalize()), crc.finish()))
    })
    .await
    .map_err(|_| Error::Setup {
        what: "the hashing task",
    })?
}

/// What a download has received so far, kept across attempts so a broken transfer continues where it stopped.
struct Progress {
    part: PartFile,
    written: u64,
    /// The object's length, from the first answer.
    total: Option<u64>,
    /// The object's ETag; without one a broken transfer starts again instead of continuing.
    etag: Option<String>,
    verifier: Verifier,
}

/// A temporary file beside its target: renamed onto the target by `commit`, removed if dropped before that — also
/// when the download future is cancelled.
struct PartFile {
    file: tokio::fs::File,
    temporary: PathBuf,
    target: PathBuf,
    committed: bool,
}

impl PartFile {
    async fn create(target: &Path) -> Result<Self> {
        let name = target.file_name().ok_or(Error::InvalidArgument {
            what: "file to download into",
            rule: "a path ending in a file name",
        })?;
        let temporary = target.with_file_name(format!(
            ".{}.{:016x}.part",
            name.to_string_lossy(),
            fastrand::u64(..)
        ));
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .await
            .map_err(to_download)?;
        Ok(Self {
            file,
            temporary,
            target: target.to_path_buf(),
            committed: false,
        })
    }

    async fn commit(mut self) -> Result<()> {
        self.file.flush().await.map_err(to_download)?;
        self.file.sync_all().await.map_err(to_download)?;
        tokio::fs::rename(&self.temporary, &self.target)
            .await
            .map_err(to_download)?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for PartFile {
    fn drop(&mut self) {
        if !self.committed {
            // A single unlink; it runs on drop because a cancelled future gets no other chance to clean up.
            let _ = std::fs::remove_file(&self.temporary);
        }
    }
}

#[cfg(test)]
#[path = "files_tests.rs"]
mod tests;
