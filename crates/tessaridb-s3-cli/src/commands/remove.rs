//! `rm`: one object, or with `--recursive` every object under a prefix. Each deletion is reported on standard error
//! as it happens, and afterwards the server is asked again: a key still there makes the command fail.

use std::collections::HashSet;

use futures_util::{TryStreamExt as _, stream};
use tessaridb_s3_client::{BucketName, Client, ObjectKey};

use super::list::{all_objects, directory};
use super::{CONCURRENCY, head, shown};
use crate::error::{CliError, Result};
use crate::location::Location;
use crate::output::{line, notice};

/// Deletes `bucket/key`, which must exist.
pub(super) async fn object(
    client: &Client,
    bucket: &BucketName,
    key: &ObjectKey,
    dry_run: bool,
) -> Result<()> {
    let target = shown(bucket, key.as_str());
    if head(client, bucket, key).await?.is_none() {
        return Err(CliError::Refused(format!("{target} does not exist")));
    }
    if dry_run {
        return line(&format!("would delete {target}"));
    }
    client.delete_object(bucket, key).await?;
    if head(client, bucket, key).await?.is_some() {
        return Err(CliError::Unconfirmed(format!(
            "{target} was deleted but the server still has it"
        )));
    }
    notice(&format!("deleted {target}"));
    Ok(())
}

/// Deletes every object under the location's key taken as a directory (`s3://b/photos` → `photos/…`), or every
/// object in the bucket when there is no key.
pub(super) async fn under_prefix(
    client: &Client,
    location: &Location,
    dry_run: bool,
) -> Result<()> {
    let Location::Remote { bucket, key } = location else {
        return Ok(());
    };
    let prefix = directory(key.as_deref());
    let keys = all_objects(client, bucket, prefix.as_deref())
        .await?
        .into_iter()
        .map(|object| ObjectKey::new(&object.key).map_err(CliError::from))
        .collect::<Result<Vec<_>>>()?;
    if dry_run {
        for key in &keys {
            line(&format!("would delete {}", shown(bucket, key.as_str())))?;
        }
        return Ok(());
    }
    stream::iter(keys.iter().map(Ok::<_, CliError>))
        .try_for_each_concurrent(CONCURRENCY, |key| async move {
            client.delete_object(bucket, key).await?;
            notice(&format!("deleted {}", shown(bucket, key.as_str())));
            Ok(())
        })
        .await?;
    let deleted: HashSet<&str> = keys.iter().map(ObjectKey::as_str).collect();
    let remaining = all_objects(client, bucket, prefix.as_deref())
        .await?
        .into_iter()
        .filter(|object| deleted.contains(object.key.as_str()))
        .count();
    if remaining > 0 {
        return Err(CliError::Unconfirmed(format!(
            "{remaining} deleted objects are still listed"
        )));
    }
    line(&format!("deleted {} objects", keys.len()))
}
