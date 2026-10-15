//! `ls`: buckets, or one level (or with `--recursive` all) of the objects under a prefix, every page of it.

use chrono::{DateTime, Utc};
use serde::Serialize;
use tessaridb_s3_client::{BucketName, Client, ListObjects, ObjectInfo};

use crate::error::Result;
use crate::location::Location;
use crate::output::{json, line, printable};

#[derive(Serialize)]
struct BucketLine<'a> {
    name: &'a str,
    created: Option<String>,
}

#[derive(Serialize)]
struct ObjectLine<'a> {
    key: &'a str,
    size: u64,
    etag: &'a str,
    last_modified: Option<String>,
}

#[derive(Serialize)]
struct PrefixLine<'a> {
    prefix: &'a str,
}

/// A time as `ls` and `stat` print it: UTC, to the second.
pub(super) fn timestamp(time: &DateTime<Utc>) -> String {
    time.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Prints the buckets.
pub(super) async fn buckets(client: &Client, as_json: bool) -> Result<()> {
    for bucket in client.list_buckets().await? {
        let created = bucket.created.as_ref().map(timestamp);
        if as_json {
            json(&BucketLine {
                name: &bucket.name,
                created,
            })?;
        } else {
            line(&format!(
                "{:<20}  s3://{}",
                created.as_deref().unwrap_or("-"),
                printable(&bucket.name)
            ))?;
        }
    }
    Ok(())
}

/// Prints the objects under a remote location: one level, rolled up at `/`, unless `recursive`.
pub(super) async fn objects(
    client: &Client,
    location: &Location,
    recursive: bool,
    as_json: bool,
) -> Result<()> {
    let Location::Remote { bucket, key } = location else {
        return Ok(());
    };
    let mut listing = ListObjects {
        prefix: key.clone(),
        delimiter: (!recursive).then(|| "/".to_owned()),
        ..ListObjects::default()
    };
    loop {
        let page = client.list_objects(bucket, &listing).await?;
        for prefix in &page.common_prefixes {
            if as_json {
                json(&PrefixLine { prefix })?;
            } else {
                line(&format!("{:<20}  {:>14}  {}", "", "PRE", printable(prefix)))?;
            }
        }
        for object in &page.objects {
            let last_modified = object.last_modified.as_ref().map(timestamp);
            if as_json {
                json(&ObjectLine {
                    key: &object.key,
                    size: object.size,
                    etag: &object.etag,
                    last_modified,
                })?;
            } else {
                let when = last_modified.as_deref().unwrap_or("-");
                line(&format!(
                    "{when:<20}  {:>14}  {}",
                    object.size,
                    printable(&object.key)
                ))?;
            }
        }
        match page.next_continuation_token {
            Some(token) => listing.continuation_token = Some(token),
            None => return Ok(()),
        }
    }
}

/// Every object whose key starts with `prefix`, all pages, in key order.
pub(super) async fn all_objects(
    client: &Client,
    bucket: &BucketName,
    prefix: Option<&str>,
) -> Result<Vec<ObjectInfo>> {
    let mut listing = ListObjects {
        prefix: prefix.map(str::to_owned),
        ..ListObjects::default()
    };
    let mut objects = Vec::new();
    loop {
        let page = client.list_objects(bucket, &listing).await?;
        objects.extend(page.objects);
        match page.next_continuation_token {
            Some(token) => listing.continuation_token = Some(token),
            None => return Ok(objects),
        }
    }
}

/// `key` as a prefix that names a directory: a non-empty key gets a trailing `/`, so `photos` never matches
/// `photos-old/…`.
pub(super) fn directory(key: Option<&str>) -> Option<String> {
    key.map(|key| {
        if key.ends_with('/') {
            key.to_owned()
        } else {
            format!("{key}/")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::directory;

    #[test]
    fn a_recursive_prefix_is_a_directory_never_a_name_start() {
        assert_eq!(directory(Some("photos")).as_deref(), Some("photos/"));
        assert_eq!(directory(Some("photos/")).as_deref(), Some("photos/"));
        assert_eq!(directory(None), None);
    }
}
