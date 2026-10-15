//! Where a copy reads and writes: the two locations resolved to an upload, a download or a server-side copy, with a
//! nameless destination taking the source's file name.

use std::path::{Path, PathBuf};

use tessaridb_s3_client::{BucketName, ObjectKey};

use super::super::shown;
use crate::error::{CliError, Result};
use crate::location::Location;
use crate::output::printable;

/// What a copy reads and writes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Endpoints {
    /// A local file to an object.
    Upload {
        path: PathBuf,
        bucket: BucketName,
        key: ObjectKey,
    },
    /// An object to a local file.
    Download {
        bucket: BucketName,
        key: ObjectKey,
        path: PathBuf,
    },
    /// An object to another object, on the server.
    Copy {
        source_bucket: BucketName,
        source_key: ObjectKey,
        bucket: BucketName,
        key: ObjectKey,
    },
}

/// The source and destination of a copy; a destination that is a bucket, or ends in `/`, or is a local directory,
/// takes the source's file name.
pub(crate) fn endpoints(source: &Location, destination: &Location) -> Result<Endpoints> {
    match (source, destination) {
        (Location::Local(path), Location::Remote { bucket, key }) => {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    CliError::Invalid(format!("{} does not name a file", path.display()))
                })?;
            Ok(Endpoints::Upload {
                path: path.clone(),
                bucket: bucket.clone(),
                key: object_key(bucket, key.as_deref(), name)?,
            })
        }
        (Location::Remote { .. }, Location::Local(path)) => {
            let (bucket, key) = source.object()?;
            let path = if path.is_dir() || ends_with_separator(path) {
                path.join(file_name(&key)?)
            } else {
                path.clone()
            };
            Ok(Endpoints::Download { bucket, key, path })
        }
        (Location::Remote { .. }, Location::Remote { bucket, key }) => {
            let (source_bucket, source_key) = source.object()?;
            let key = object_key(bucket, key.as_deref(), file_name(&source_key)?)?;
            Ok(Endpoints::Copy {
                source_bucket,
                source_key,
                bucket: bucket.clone(),
                key,
            })
        }
        (Location::Local(_), Location::Local(_)) => Err(CliError::Invalid(
            "both locations are local; one of them must be s3://".to_owned(),
        )),
    }
}

fn object_key(bucket: &BucketName, key: Option<&str>, name: &str) -> Result<ObjectKey> {
    let key = match key {
        None => name.to_owned(),
        Some(prefix) if prefix.ends_with('/') => format!("{prefix}{name}"),
        Some(key) => key.to_owned(),
    };
    ObjectKey::new(&key)
        .map_err(|error| CliError::Invalid(format!("{}: {error}", shown(bucket, &key))))
}

/// The last segment of `key`, which must be usable as a file name.
fn file_name(key: &ObjectKey) -> Result<&str> {
    match key.as_str().rsplit('/').next() {
        Some(name) if !name.is_empty() && name != "." && name != ".." => Ok(name),
        _ => Err(CliError::Invalid(format!(
            "{} does not end in a file name",
            printable(key.as_str())
        ))),
    }
}

fn ends_with_separator(path: &Path) -> bool {
    path.as_os_str()
        .to_string_lossy()
        .ends_with(std::path::MAIN_SEPARATOR)
        || path.as_os_str().to_string_lossy().ends_with('/')
}
