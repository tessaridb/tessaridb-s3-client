//! What a request carries and how it goes on the wire: bytes from memory, or a range of a file streamed in chunks.

use std::io::SeekFrom;
use std::path::PathBuf;

use bytes::Bytes;
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};

use crate::constants::FILE_CHUNK_BYTES;
use crate::sign::payload_sha256;
use crate::{Error, Result};

/// What a request carries.
#[derive(Debug, Clone)]
pub(crate) enum Payload {
    /// Bytes held in memory.
    Memory(Bytes),
    /// `length` bytes from `offset` of a file that was `file_length` bytes long when hashed, the range's SHA-256
    /// being `sha256`; opened afresh for every attempt and streamed.
    File {
        path: PathBuf,
        file_length: u64,
        offset: u64,
        length: u64,
        sha256: String,
    },
}

fn to_upload(source: std::io::Error) -> Error {
    Error::Io {
        what: "the file to upload",
        source,
    }
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
            file_length,
            offset,
            length,
            sha256,
        } => {
            let mut file = tokio::fs::File::open(path).await.map_err(to_upload)?;
            let found = file.metadata().await.map_err(to_upload)?.len();
            if found != *file_length {
                return Err(Error::InvalidArgument {
                    what: "file to upload",
                    rule: "unchanged between hashing and sending",
                });
            }
            file.seek(SeekFrom::Start(*offset))
                .await
                .map_err(to_upload)?;
            let range = file.take(*length);
            let stream = tokio_util::io::ReaderStream::with_capacity(range, FILE_CHUNK_BYTES);
            Ok((reqwest::Body::wrap_stream(stream), *length, sha256.clone()))
        }
    }
}
