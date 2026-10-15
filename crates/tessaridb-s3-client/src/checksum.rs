//! End-to-end integrity: the checksum an upload declares, and the check a download makes against every whole-object
//! checksum the server returns (`x-amz-checksum-*` with `ChecksumMode=ENABLED`). A composite checksum — one taken
//! over part checksums, written with a `-N` suffix — covers no single byte stream the client sees, so it is not
//! checked; neither is an algorithm this client does not compute.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use crc_fast::{CrcAlgorithm, Digest as CrcDigest};
use reqwest::header::HeaderMap;
use sha1::Sha1;
use sha2::{Digest as _, Sha256};

use crate::{Error, Result};

/// The algorithm this client declares on uploads.
pub(crate) const UPLOAD_ALGORITHM: &str = "CRC64NVME";

/// The header carrying the upload checksum.
pub(crate) const UPLOAD_HEADER: &str = "x-amz-checksum-crc64nvme";

/// The CRC64NVME of `bytes` as the header carries it: base64 of the big-endian value.
pub(crate) fn crc64nvme(bytes: &[u8]) -> String {
    STANDARD.encode(crc_fast::checksum(CrcAlgorithm::Crc64Nvme, bytes).to_be_bytes())
}

/// A running CRC64NVME, for content hashed in pieces.
pub(crate) struct Crc64Nvme(CrcDigest);

impl Crc64Nvme {
    pub(crate) fn new() -> Self {
        Self(CrcDigest::new(CrcAlgorithm::Crc64Nvme))
    }

    pub(crate) fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    pub(crate) fn finish(&self) -> String {
        STANDARD.encode(self.0.finalize().to_be_bytes())
    }
}

enum Running {
    Crc32(CrcDigest),
    Crc64(CrcDigest),
    Sha1(Sha1),
    Sha256(Sha256),
}

/// Checks a body against the whole-object checksums its answer declared.
pub(crate) struct Verifier {
    checks: Vec<(&'static str, String, Running)>,
}

impl Verifier {
    /// A verifier for the checksums in `headers` this client can compute over the whole body.
    pub(crate) fn from_headers(headers: &HeaderMap) -> Self {
        let declared = |name: &str| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .filter(|value| !value.contains('-'))
                .map(str::to_owned)
        };
        let candidates = [
            (
                "CRC32",
                "x-amz-checksum-crc32",
                Running::Crc32(CrcDigest::new(CrcAlgorithm::Crc32IsoHdlc)),
            ),
            (
                "CRC32C",
                "x-amz-checksum-crc32c",
                Running::Crc32(CrcDigest::new(CrcAlgorithm::Crc32Iscsi)),
            ),
            (
                "CRC64NVME",
                UPLOAD_HEADER,
                Running::Crc64(CrcDigest::new(CrcAlgorithm::Crc64Nvme)),
            ),
            ("SHA1", "x-amz-checksum-sha1", Running::Sha1(Sha1::new())),
            (
                "SHA256",
                "x-amz-checksum-sha256",
                Running::Sha256(Sha256::new()),
            ),
        ];
        let checks = candidates
            .into_iter()
            .filter_map(|(algorithm, header, running)| {
                declared(header).map(|value| (algorithm, value, running))
            })
            .collect();
        Self { checks }
    }

    pub(crate) fn update(&mut self, bytes: &[u8]) {
        for (_, _, running) in &mut self.checks {
            match running {
                Running::Crc32(digest) | Running::Crc64(digest) => digest.update(bytes),
                Running::Sha1(digest) => digest.update(bytes),
                Running::Sha256(digest) => digest.update(bytes),
            }
        }
    }

    /// Whether every declared checksum matched.
    ///
    /// # Errors
    /// [`Error::ChecksumMismatch`] naming the first algorithm whose value differs.
    pub(crate) fn finish(self) -> Result<()> {
        for (algorithm, expected, running) in self.checks {
            let actual = match running {
                Running::Crc32(digest) => u32::try_from(digest.finalize())
                    .map(|value| STANDARD.encode(value.to_be_bytes()))
                    .unwrap_or_default(),
                Running::Crc64(digest) => STANDARD.encode(digest.finalize().to_be_bytes()),
                Running::Sha1(digest) => STANDARD.encode(digest.finalize()),
                Running::Sha256(digest) => STANDARD.encode(digest.finalize()),
            };
            if actual != expected {
                return Err(Error::ChecksumMismatch { algorithm });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "checksum_tests.rs"]
mod tests;
