//! `cp` and `mv`. Before anything moves, the source is measured and the destination checked: an existing
//! destination is replaced only with `--yes`. Afterwards the destination's size is compared with the source's before
//! the command says it is done, and `mv` deletes its source only after that check.

use std::path::Path;

use tessaridb_s3_client::{BucketName, Client, ObjectKey};

use super::{head, shown};
use crate::error::{CliError, Result};
use crate::output::{line, notice, printable};

mod endpoints;

pub(crate) use endpoints::{Endpoints, endpoints};

/// Copies, checks the copy, then deletes the source. `--yes` was required to get here, and it covers replacing an
/// existing destination.
pub(super) async fn r#move(client: &Client, endpoints: Endpoints, dry_run: bool) -> Result<()> {
    copy(client, &endpoints, true, dry_run).await?;
    let source = describe_source(&endpoints);
    if dry_run {
        return line(&format!("would delete {source}"));
    }
    match &endpoints {
        Endpoints::Upload { path, .. } => {
            tokio::fs::remove_file(path)
                .await
                .map_err(|source| local("delete", path, source))?;
            if tokio::fs::try_exists(path)
                .await
                .map_err(|source| local("check", path, source))?
            {
                return Err(CliError::Unconfirmed(format!(
                    "{source} was deleted but is still there"
                )));
            }
        }
        Endpoints::Download { bucket, key, .. }
        | Endpoints::Copy {
            source_bucket: bucket,
            source_key: key,
            ..
        } => {
            client.delete_object(bucket, key).await?;
            if head(client, bucket, key).await?.is_some() {
                return Err(CliError::Unconfirmed(format!(
                    "{source} was deleted but the server still has it"
                )));
            }
        }
    }
    notice(&format!("deleted {source}"));
    Ok(())
}

/// Copies, replacing an existing destination only when `overwrite`.
pub(super) async fn copy(
    client: &Client,
    endpoints: &Endpoints,
    overwrite: bool,
    dry_run: bool,
) -> Result<()> {
    let source_size = match endpoints {
        Endpoints::Upload { path, .. } => local_size(path).await?,
        Endpoints::Download { bucket, key, .. }
        | Endpoints::Copy {
            source_bucket: bucket,
            source_key: key,
            ..
        } => {
            head(client, bucket, key)
                .await?
                .ok_or_else(|| {
                    CliError::Refused(format!("{} does not exist", shown(bucket, key.as_str())))
                })?
                .size
        }
    };
    let exists = match endpoints {
        Endpoints::Upload { bucket, key, .. } | Endpoints::Copy { bucket, key, .. } => {
            head(client, bucket, key).await?.is_some()
        }
        Endpoints::Download { path, .. } => tokio::fs::try_exists(path)
            .await
            .map_err(|source| local("check", path, source))?,
    };
    let (source, destination) = (describe_source(endpoints), describe_destination(endpoints));
    if exists && !overwrite {
        return Err(CliError::Refused(format!(
            "{destination} exists; add --yes to replace it"
        )));
    }
    if dry_run {
        return line(&format!(
            "would copy {source} → {destination} ({source_size} bytes)"
        ));
    }
    match endpoints {
        Endpoints::Upload { path, bucket, key } => {
            upload(client, path, bucket, key, source_size).await?
        }
        Endpoints::Download { bucket, key, path } => {
            download(client, bucket, key, path, source_size).await?
        }
        Endpoints::Copy {
            source_bucket,
            source_key,
            bucket,
            key,
        } => {
            client
                .copy_object(source_bucket, source_key, bucket, key)
                .await?;
            confirm_remote_size(client, bucket, key, source_size).await?;
        }
    }
    line(&format!(
        "copied {source} → {destination} ({source_size} bytes)"
    ))
}

/// Uploads `path` as `bucket/key` and checks the server holds `size` bytes there.
pub(super) async fn upload(
    client: &Client,
    path: &Path,
    bucket: &BucketName,
    key: &ObjectKey,
    size: u64,
) -> Result<()> {
    client.upload_file(bucket, key, path, None).await?;
    confirm_remote_size(client, bucket, key, size).await
}

/// Downloads `bucket/key` to `path` and checks the file holds `size` bytes.
pub(super) async fn download(
    client: &Client,
    bucket: &BucketName,
    key: &ObjectKey,
    path: &Path,
    size: u64,
) -> Result<()> {
    client.download_file(bucket, key, path).await?;
    let written = local_size(path).await?;
    if written != size {
        return Err(CliError::Unconfirmed(format!(
            "{} holds {written} bytes, the object {size}",
            path.display()
        )));
    }
    Ok(())
}

async fn confirm_remote_size(
    client: &Client,
    bucket: &BucketName,
    key: &ObjectKey,
    size: u64,
) -> Result<()> {
    let shown = shown(bucket, key.as_str());
    match head(client, bucket, key).await? {
        Some(info) if info.size == size => Ok(()),
        Some(info) => Err(CliError::Unconfirmed(format!(
            "{shown} holds {} bytes, the source {size}",
            info.size
        ))),
        None => Err(CliError::Unconfirmed(format!(
            "{shown} was written but the server does not have it"
        ))),
    }
}

/// The size of a local regular file.
pub(super) async fn local_size(path: &Path) -> Result<u64> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|source| local("read", path, source))?;
    if !metadata.is_file() {
        return Err(CliError::Invalid(format!(
            "{} is not a file",
            path.display()
        )));
    }
    Ok(metadata.len())
}

/// A local failure naming the path.
pub(super) fn local(what: &str, path: &Path, source: std::io::Error) -> CliError {
    CliError::Local {
        what: format!("cannot {what} {}", path.display()),
        source,
    }
}

fn describe_source(endpoints: &Endpoints) -> String {
    match endpoints {
        Endpoints::Upload { path, .. } => printable(&path.display().to_string()),
        Endpoints::Download { bucket, key, .. }
        | Endpoints::Copy {
            source_bucket: bucket,
            source_key: key,
            ..
        } => shown(bucket, key.as_str()),
    }
}

fn describe_destination(endpoints: &Endpoints) -> String {
    match endpoints {
        Endpoints::Download { path, .. } => printable(&path.display().to_string()),
        Endpoints::Upload { bucket, key, .. } | Endpoints::Copy { bucket, key, .. } => {
            shown(bucket, key.as_str())
        }
    }
}
