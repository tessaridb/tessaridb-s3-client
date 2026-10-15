//! `mb` and `rb`: each checks afterwards that the bucket is (or is no longer) there before saying so.

use tessaridb_s3_client::{BucketName, Client, ListObjects};

use crate::error::{CliError, Result};
use crate::output::{line, notice};

/// Creates `bucket`.
pub(super) async fn make(client: &Client, bucket: &BucketName, dry_run: bool) -> Result<()> {
    if dry_run {
        return line(&format!("would create s3://{bucket}"));
    }
    client.create_bucket(bucket).await?;
    if !client.bucket_exists(bucket).await? {
        return Err(CliError::Unconfirmed(format!(
            "s3://{bucket} was created but the server does not show it"
        )));
    }
    line(&format!("created s3://{bucket}"))
}

/// Deletes `bucket`, which must be empty.
pub(super) async fn remove(client: &Client, bucket: &BucketName, dry_run: bool) -> Result<()> {
    let listing = ListObjects {
        max_keys: Some(1),
        ..ListObjects::default()
    };
    if !client
        .list_objects(bucket, &listing)
        .await?
        .objects
        .is_empty()
    {
        return Err(CliError::Refused(format!(
            "s3://{bucket} is not empty; delete its objects first (rm --recursive)"
        )));
    }
    if dry_run {
        return line(&format!("would delete s3://{bucket}"));
    }
    client.delete_bucket(bucket).await?;
    if client.bucket_exists(bucket).await? {
        return Err(CliError::Unconfirmed(format!(
            "s3://{bucket} was deleted but the server still shows it"
        )));
    }
    notice(&format!("deleted s3://{bucket}"));
    Ok(())
}
