//! `sync`: makes a prefix hold what a local directory holds, or the reverse. A file moves when it is missing on the
//! other side or its content differs — compared by size, then by MD5 against a single-part ETag, then by the
//! CRC64NVME the server keeps; when neither proves it the same, it is sent. Nothing is ever deleted. Going down, a
//! key that would land outside the target directory (`..`, an empty segment) is skipped and makes the run fail.

use std::path::{Path, PathBuf};

use futures_util::{StreamExt as _, TryStreamExt as _, stream};
use tessaridb_s3_client::{BucketName, Client, ObjectInfo, ObjectKey};

use super::list::{all_objects, directory};
use super::transfer::{download, local, upload};
use super::{CONCURRENCY, shown};
use crate::error::{CliError, Result};
use crate::location::Location;
use crate::output::{line, notice, printable};

mod content;

/// Which way a sync goes.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Endpoints {
    /// A local directory to a prefix.
    Up {
        directory: PathBuf,
        bucket: BucketName,
        prefix: Option<String>,
    },
    /// A prefix to a local directory.
    Down {
        bucket: BucketName,
        prefix: Option<String>,
        directory: PathBuf,
    },
}

/// One side local, the other `s3://`; the remote key is taken as a directory.
pub(super) fn endpoints(source: &Location, destination: &Location) -> Result<Endpoints> {
    match (source, destination) {
        (Location::Local(path), Location::Remote { bucket, key }) => Ok(Endpoints::Up {
            directory: path.clone(),
            bucket: bucket.clone(),
            prefix: directory(key.as_deref()),
        }),
        (Location::Remote { bucket, key }, Location::Local(path)) => Ok(Endpoints::Down {
            bucket: bucket.clone(),
            prefix: directory(key.as_deref()),
            directory: path.clone(),
        }),
        _ => Err(CliError::Invalid(
            "sync needs one local directory and one s3:// location".to_owned(),
        )),
    }
}

/// One file to look at: where it is on each side and what each side holds now.
struct Pair {
    key: ObjectKey,
    path: PathBuf,
    local_size: Option<u64>,
    remote: Option<ObjectInfo>,
}

/// Runs the sync.
pub(super) async fn sync(client: &Client, endpoints: Endpoints, dry_run: bool) -> Result<()> {
    let (bucket, pairs, skipped) = match &endpoints {
        Endpoints::Up {
            directory,
            bucket,
            prefix,
        } => (
            bucket,
            pairs_up(client, directory, bucket, prefix.as_deref()).await?,
            0,
        ),
        Endpoints::Down {
            bucket,
            prefix,
            directory,
        } => {
            let (pairs, skipped) = pairs_down(client, bucket, prefix.as_deref(), directory).await?;
            (bucket, pairs, skipped)
        }
    };
    let up = matches!(endpoints, Endpoints::Up { .. });
    let (sent, same) = stream::iter(pairs)
        .map(|pair| one(client, bucket, pair, up, dry_run))
        .buffer_unordered(CONCURRENCY)
        .try_fold((0_usize, 0_usize), |(sent, same), moved| async move {
            Ok(if moved {
                (sent.saturating_add(1), same)
            } else {
                (sent, same.saturating_add(1))
            })
        })
        .await?;
    let verb = if dry_run { "would send" } else { "sent" };
    line(&format!(
        "{verb} {sent}, unchanged {same}, skipped {skipped}"
    ))?;
    if skipped > 0 {
        return Err(CliError::Refused(format!(
            "{skipped} keys were not synced: they do not map to a path inside the directory"
        )));
    }
    Ok(())
}

/// Moves one file if the two sides differ; `true` when it was (or would be) sent.
async fn one(
    client: &Client,
    bucket: &BucketName,
    pair: Pair,
    up: bool,
    dry_run: bool,
) -> Result<bool> {
    let differs = match (pair.local_size, &pair.remote) {
        (Some(size), Some(remote)) if size == remote.size => {
            !content::same(client, bucket, &pair.key, &pair.path, remote).await?
        }
        _ => true,
    };
    if !differs {
        return Ok(false);
    }
    let (object, file) = (
        shown(bucket, pair.key.as_str()),
        printable(&pair.path.display().to_string()),
    );
    let (from, to) = if up { (file, object) } else { (object, file) };
    if dry_run {
        line(&format!("would copy {from} → {to}"))?;
        return Ok(true);
    }
    match (up, pair.local_size, &pair.remote) {
        (true, Some(size), _) => upload(client, &pair.path, bucket, &pair.key, size).await?,
        (false, _, Some(remote)) => {
            if let Some(parent) = pair.path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|source| local("create", parent, source))?;
            }
            download(client, bucket, &pair.key, &pair.path, remote.size).await?;
        }
        _ => return Ok(false),
    }
    line(&format!("copied {from} → {to}"))?;
    Ok(true)
}

async fn pairs_up(
    client: &Client,
    root: &Path,
    bucket: &BucketName,
    prefix: Option<&str>,
) -> Result<Vec<Pair>> {
    let files = content::walk(root.to_path_buf()).await?;
    let mut remote: std::collections::HashMap<String, ObjectInfo> =
        all_objects(client, bucket, prefix)
            .await?
            .into_iter()
            .map(|object| (object.key.clone(), object))
            .collect();
    files
        .into_iter()
        .map(|(relative, path, size)| {
            let key = format!("{}{relative}", prefix.unwrap_or(""));
            let key = ObjectKey::new(&key).map_err(|error| {
                CliError::Invalid(format!(
                    "{}: {error}",
                    printable(&path.display().to_string())
                ))
            })?;
            let remote = remote.remove(key.as_str());
            Ok(Pair {
                key,
                path,
                local_size: Some(size),
                remote,
            })
        })
        .collect()
}

async fn pairs_down(
    client: &Client,
    bucket: &BucketName,
    prefix: Option<&str>,
    root: &Path,
) -> Result<(Vec<Pair>, usize)> {
    let mut pairs = Vec::new();
    let mut skipped = 0_usize;
    for object in all_objects(client, bucket, prefix).await? {
        let relative = object
            .key
            .strip_prefix(prefix.unwrap_or(""))
            .unwrap_or(&object.key);
        let path = match target_path(root, relative) {
            Target::Path(path) => path,
            Target::Directory => continue,
            Target::Outside => {
                notice(&format!(
                    "skipped {}: it does not map to a path inside the directory",
                    shown(bucket, &object.key)
                ));
                skipped = skipped.saturating_add(1);
                continue;
            }
        };
        let local_size = match tokio::fs::metadata(&path).await {
            Ok(metadata) if metadata.is_file() => Some(metadata.len()),
            Ok(_) => {
                return Err(CliError::Invalid(format!(
                    "{} exists and is not a file",
                    path.display()
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => return Err(local("read", &path, source)),
        };
        let key = ObjectKey::new(&object.key).map_err(CliError::from)?;
        pairs.push(Pair {
            key,
            path,
            local_size,
            remote: Some(object),
        });
    }
    Ok((pairs, skipped))
}

/// Where a key's remainder after the prefix lands under `root`.
#[derive(Debug, PartialEq, Eq)]
enum Target {
    /// A file path inside `root`.
    Path(PathBuf),
    /// A directory marker (`photos/`): nothing to download.
    Directory,
    /// A key that would leave `root` or cannot be a path.
    Outside,
}

fn target_path(root: &Path, relative: &str) -> Target {
    if relative.is_empty() || relative.ends_with('/') {
        return Target::Directory;
    }
    let mut path = root.to_path_buf();
    for segment in relative.split('/') {
        let usable = !segment.is_empty()
            && segment != "."
            && segment != ".."
            && !segment.contains(['\\', '\0'])
            && Path::new(segment).components().count() == 1
            && !Path::new(segment).has_root();
        if !usable {
            return Target::Outside;
        }
        path.push(segment);
    }
    Target::Path(path)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{Target, target_path};

    #[test]
    fn a_key_maps_inside_the_directory_or_not_at_all() {
        let root = Path::new("/tmp/out");
        assert_eq!(
            target_path(root, "a/b.txt"),
            Target::Path(PathBuf::from("/tmp/out/a/b.txt"))
        );
        assert_eq!(target_path(root, "a/"), Target::Directory);
        assert_eq!(target_path(root, ""), Target::Directory);
        for hostile in [
            "../etc/passwd",
            "a/../../x",
            "a//b",
            "./a",
            "a/./b",
            "/abs",
            "a\\..\\b",
            "a/\0",
        ] {
            assert_eq!(target_path(root, hostile), Target::Outside, "{hostile:?}");
        }
    }
}
