//! `stat` and `presign`: they read, and change nothing.

use std::time::Duration;

use serde::Serialize;
use tessaridb_s3_client::{BucketName, Client, ObjectKey};

use super::shown;
use crate::error::Result;
use crate::output::{json, line, printable};

/// One object's metadata as `stat --json` prints it.
#[derive(Serialize)]
struct Stat<'a> {
    key: &'a str,
    size: u64,
    etag: &'a str,
    crc64nvme: Option<&'a str>,
    last_modified: Option<String>,
}

/// Prints the metadata of `bucket/key`.
pub(super) async fn stat(
    client: &Client,
    bucket: &BucketName,
    key: &ObjectKey,
    as_json: bool,
) -> Result<()> {
    let info = client.head_object(bucket, key).await?;
    let last_modified = info.last_modified.map(|time| super::list::timestamp(&time));
    if as_json {
        return json(&Stat {
            key: key.as_str(),
            size: info.size,
            etag: &info.etag,
            crc64nvme: info.crc64nvme.as_deref(),
            last_modified,
        });
    }
    line(&format!("object         {}", shown(bucket, key.as_str())))?;
    line(&format!("size           {}", info.size))?;
    line(&format!("etag           {}", printable(&info.etag)))?;
    line(&format!(
        "crc64nvme      {}",
        info.crc64nvme
            .as_deref()
            .map_or_else(|| "-".to_owned(), printable)
    ))?;
    line(&format!(
        "last modified  {}",
        last_modified.as_deref().unwrap_or("-")
    ))
}

/// Prints a link to download (or upload) `bucket/key` for `expires`.
pub(super) fn presign(
    client: &Client,
    bucket: &BucketName,
    key: &ObjectKey,
    put: bool,
    expires: Duration,
) -> Result<()> {
    let url = if put {
        client.presign_put(bucket, key, expires)?
    } else {
        client.presign_get(bucket, key, expires)?
    };
    line(&url)
}
