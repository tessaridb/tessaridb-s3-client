//! Planning a file's parts: their size, their ranges and digests, and which of them the server already holds.

use std::io::Read as _;
use std::path::PathBuf;

use md5::Md5;
use sha2::{Digest as _, Sha256};

use crate::checksum::Crc64Nvme;
use crate::constants::{
    FILE_CHUNK_BYTES, MAX_PARTS, PART_SIZE_MAX_BYTES, PART_SIZE_MIN_BYTES, PART_SIZE_STEP_BYTES,
};
use crate::sign::hex;
use crate::xml::StoredPart;
use crate::{Error, Result};

/// One part: its number, where it lies in the file, and its digests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PartPlan {
    pub(crate) number: u16,
    pub(crate) offset: u64,
    pub(crate) length: u64,
    pub(crate) sha256: String,
    /// The ETag the server returns for these bytes: their MD5 in hex, quoted.
    pub(crate) etag: String,
    pub(crate) crc: String,
}

/// A file cut into parts.
#[derive(Debug)]
pub(crate) struct FilePlan {
    pub(crate) length: u64,
    pub(crate) crc: String,
    pub(crate) parts: Vec<PartPlan>,
}

/// The part size for a file of `length` bytes: at least 8 MiB, large enough that 10 000 parts hold the file, rounded up
/// to whole MiB.
pub(crate) fn part_size(length: u64) -> Result<u64> {
    let needed = length
        .div_ceil(u64::from(MAX_PARTS))
        .div_ceil(PART_SIZE_STEP_BYTES);
    let size = needed
        .checked_mul(PART_SIZE_STEP_BYTES)
        .map_or(u64::MAX, |size| size.max(PART_SIZE_MIN_BYTES));
    if size > PART_SIZE_MAX_BYTES {
        return Err(Error::InvalidArgument {
            what: "file to upload",
            rule: "at most 10 000 parts of 5 GiB",
        });
    }
    Ok(size)
}

/// The parts the server does not hold intact: listed parts are kept when their size and ETag match the plan.
pub(crate) fn missing<'a>(plan: &'a FilePlan, listed: &[StoredPart]) -> Vec<&'a PartPlan> {
    plan.parts
        .iter()
        .filter(|part| {
            !listed.iter().any(|stored| {
                stored.number == part.number
                    && stored.size == part.length
                    && stored.etag == part.etag
            })
        })
        .collect()
}

/// Plans `path`, `length` bytes long, reading it once on a blocking thread.
pub(crate) async fn plan(path: PathBuf, length: u64) -> Result<FilePlan> {
    let size = part_size(length)?;
    tokio::task::spawn_blocking(move || {
        let io = |source| Error::Io {
            what: "the file to upload",
            source,
        };
        let mut file = std::fs::File::open(&path).map_err(io)?;
        let mut whole = Crc64Nvme::new();
        let mut parts = Vec::new();
        let mut buffer = vec![0_u8; FILE_CHUNK_BYTES];
        let mut offset: u64 = 0;
        let mut number: u16 = 0;
        while offset < length {
            number = number.checked_add(1).ok_or(Error::InvalidArgument {
                what: "file to upload",
                rule: "at most 10 000 parts of 5 GiB",
            })?;
            let part_length = size.min(length.saturating_sub(offset));
            let (mut sha, mut md5, mut crc) = (Sha256::new(), Md5::new(), Crc64Nvme::new());
            let mut remaining = part_length;
            while remaining > 0 {
                let want =
                    usize::try_from(remaining).map_or(buffer.len(), |left| left.min(buffer.len()));
                let chunk = buffer.get_mut(..want).ok_or(Error::Setup {
                    what: "the hashing buffer",
                })?;
                file.read_exact(chunk).map_err(io)?;
                sha.update(&*chunk);
                md5.update(&*chunk);
                crc.update(chunk);
                whole.update(chunk);
                remaining = remaining.saturating_sub(u64::try_from(want).unwrap_or(0));
            }
            parts.push(PartPlan {
                number,
                offset,
                length: part_length,
                sha256: hex(&sha.finalize()),
                etag: format!("\"{}\"", hex(&md5.finalize())),
                crc: crc.finish(),
            });
            offset = offset.saturating_add(part_length);
        }
        Ok(FilePlan {
            length,
            crc: whole.finish(),
            parts,
        })
    })
    .await
    .map_err(|_| Error::Setup {
        what: "the hashing task",
    })?
}
