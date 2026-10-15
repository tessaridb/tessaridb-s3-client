//! Whether a local file and an object hold the same bytes, and the local directory walk.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use md5::{Digest as _, Md5};
use tessaridb_s3_client::{BucketName, Client, ObjectInfo, ObjectKey};

use super::super::transfer::local;
use crate::error::{CliError, Result};
use crate::output::notice;

/// Bytes read at a time when hashing a file.
const CHUNK_BYTES: usize = 1 << 20;

/// `true` only when the content is proven equal: MD5 against a single-part ETag, else the server's CRC64NVME
/// against the file's. Sizes are already known to match.
pub(super) async fn same(
    client: &Client,
    bucket: &BucketName,
    key: &ObjectKey,
    path: &Path,
    remote: &ObjectInfo,
) -> Result<bool> {
    if let Some(md5) = single_part_md5(&remote.etag) {
        return Ok(hash(path.to_path_buf(), Hash::Md5).await? == md5);
    }
    match super::super::head(client, bucket, key)
        .await?
        .and_then(|info| info.crc64nvme)
    {
        Some(crc) => Ok(hash(path.to_path_buf(), Hash::Crc64Nvme).await? == crc),
        None => Ok(false),
    }
}

/// The MD5 an ETag carries when it is a single-part upload's: 32 hex digits, no `-N` part count.
pub(super) fn single_part_md5(etag: &str) -> Option<String> {
    let bare = etag.trim_matches('"');
    (bare.len() == 32 && bare.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| bare.to_ascii_lowercase())
}

#[derive(Clone, Copy)]
enum Hash {
    /// Lowercase hex, as in an ETag.
    Md5,
    /// Base64 of the big-endian value, as the server reports it.
    Crc64Nvme,
}

async fn hash(path: PathBuf, kind: Hash) -> Result<String> {
    tokio::task::spawn_blocking(move || {
        let mut file = std::fs::File::open(&path).map_err(|source| local("read", &path, source))?;
        let mut md5 = Md5::new();
        let mut crc = crc_fast::Digest::new(crc_fast::CrcAlgorithm::Crc64Nvme);
        let mut buffer = vec![0_u8; CHUNK_BYTES];
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|source| local("read", &path, source))?;
            let Some(chunk) = buffer.get(..read).filter(|chunk| !chunk.is_empty()) else {
                break;
            };
            match kind {
                Hash::Md5 => md5.update(chunk),
                Hash::Crc64Nvme => crc.update(chunk),
            }
        }
        Ok(match kind {
            Hash::Md5 => md5
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            Hash::Crc64Nvme => STANDARD.encode(crc.finalize().to_be_bytes()),
        })
    })
    .await
    .map_err(|error| CliError::Local {
        what: "hash a file".to_owned(),
        source: std::io::Error::other(error),
    })?
}

/// Every regular file under `root`, as (key suffix with `/` separators, path, size), in key order. Symbolic links and
/// names that are not UTF-8 are skipped and reported.
pub(super) async fn walk(root: PathBuf) -> Result<Vec<(String, PathBuf, u64)>> {
    tokio::task::spawn_blocking(move || {
        let metadata = std::fs::metadata(&root).map_err(|source| local("read", &root, source))?;
        if !metadata.is_dir() {
            return Err(CliError::Invalid(format!(
                "{} is not a directory",
                root.display()
            )));
        }
        let mut files = Vec::new();
        let mut pending = vec![(root, String::new())];
        while let Some((directory, relative)) = pending.pop() {
            let entries = std::fs::read_dir(&directory)
                .map_err(|source| local("read", &directory, source))?;
            for entry in entries {
                let entry = entry.map_err(|source| local("read", &directory, source))?;
                let path = entry.path();
                let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                    notice(&format!(
                        "skipped {}: the name is not UTF-8",
                        path.display()
                    ));
                    continue;
                };
                let kind = entry
                    .file_type()
                    .map_err(|source| local("read", &path, source))?;
                let name = format!("{relative}{name}");
                if kind.is_dir() {
                    pending.push((path, format!("{name}/")));
                } else if kind.is_file() {
                    let size = entry
                        .metadata()
                        .map_err(|source| local("read", &path, source))?
                        .len();
                    files.push((name, path, size));
                } else {
                    notice(&format!("skipped {}: not a regular file", path.display()));
                }
            }
        }
        files.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(files)
    })
    .await
    .map_err(|error| CliError::Local {
        what: "walk the directory".to_owned(),
        source: std::io::Error::other(error),
    })?
}

#[cfg(test)]
mod tests {
    use super::single_part_md5;

    #[test]
    fn only_a_single_part_etag_carries_an_md5() {
        assert_eq!(
            single_part_md5("\"9B2CF535F27731C974343645A3985328\"").as_deref(),
            Some("9b2cf535f27731c974343645a3985328")
        );
        assert_eq!(
            single_part_md5("\"9b2cf535f27731c974343645a3985328-3\""),
            None
        );
        assert_eq!(single_part_md5("\"short\""), None);
    }
}
