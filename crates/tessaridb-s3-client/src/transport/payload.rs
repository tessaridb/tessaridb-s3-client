//! What a request carries and how it goes on the wire: bytes from memory, or a file streamed in chunks.

use std::path::PathBuf;

use bytes::Bytes;

use crate::constants::FILE_CHUNK_BYTES;
use crate::sign::payload_sha256;
use crate::{Error, Result};

/// What a request carries.
#[derive(Debug, Clone)]
pub(crate) enum Payload {
    /// Bytes held in memory.
    Memory(Bytes),
    /// A file of `length` bytes whose SHA-256 is `sha256`, opened afresh for every attempt and streamed.
    File {
        path: PathBuf,
        length: u64,
        sha256: String,
    },
}

/// The body to send for `payload`, its length and its SHA-256 as `x-amz-content-sha256` carries it.
pub(super) async fn body(payload: &Payload) -> Result<(reqwest::Body, u64, String)> {
    match payload {
        Payload::Memory(bytes) => Ok((
            reqwest::Body::from(bytes.clone()),
            u64::try_from(bytes.len()).map_err(|_| Error::InvalidArgument {
                what: "body",
                rule: "a length that fits 64 bits",
            })?,
            payload_sha256(bytes),
        )),
        Payload::File {
            path,
            length,
            sha256,
        } => {
            let file = tokio::fs::File::open(path)
                .await
                .map_err(|source| Error::Io {
                    what: "the file to upload",
                    source,
                })?;
            let found = file
                .metadata()
                .await
                .map_err(|source| Error::Io {
                    what: "the file to upload",
                    source,
                })?
                .len();
            if found != *length {
                return Err(Error::InvalidArgument {
                    what: "file to upload",
                    rule: "unchanged between hashing and sending",
                });
            }
            let stream = tokio_util::io::ReaderStream::with_capacity(file, FILE_CHUNK_BYTES);
            Ok((reqwest::Body::wrap_stream(stream), *length, sha256.clone()))
        }
    }
}
