//! The answers the client reads, each from its own document shape.

use chrono::{DateTime, Utc};

use super::{Node, malformed, tree};
use crate::Error;
use crate::{BucketInfo, ListPage, ObjectInfo};

/// The parts of an S3 `<Error>` document: code, message and request ID; `None` when `body` is not one.
pub(crate) fn error_document(body: &[u8]) -> Option<(String, Option<String>, Option<String>)> {
    let root = tree(body).ok()?;
    if root.name != "Error" {
        return None;
    }
    let code = root
        .text_of("Code")
        .filter(|code| !code.is_empty())?
        .to_owned();
    let owned = |name| root.text_of(name).map(str::to_owned);
    Some((code, owned("Message"), owned("RequestId")))
}

/// The buckets a `ListBuckets` answer names.
pub(crate) fn list_buckets(body: &[u8]) -> Result<Vec<BucketInfo>, Error> {
    let root = rooted(body, "ListAllMyBucketsResult")?;
    root.child("Buckets")
        .map(|buckets| {
            buckets
                .all("Bucket")
                .map(|bucket| {
                    Ok(BucketInfo {
                        name: required(bucket, "Name")?.to_owned(),
                        created: optional_time(bucket, "CreationDate")?,
                    })
                })
                .collect()
        })
        .unwrap_or_else(|| Ok(Vec::new()))
}

/// One page of a `ListObjectsV2` answer; names are URL-decoded when the answer says it encoded them.
pub(crate) fn list_objects_v2(body: &[u8]) -> Result<ListPage, Error> {
    let root = rooted(body, "ListBucketResult")?;
    let url = match root.text_of("EncodingType") {
        None => false,
        Some(kind) if kind.eq_ignore_ascii_case("url") => true,
        Some(_) => return Err(malformed("a listing in an encoding other than url")),
    };
    let name = |text: &str| {
        if url {
            url_decode(text)
        } else {
            Ok(text.to_owned())
        }
    };
    let objects = root
        .all("Contents")
        .map(|entry| {
            Ok(ObjectInfo {
                key: name(required(entry, "Key")?)?,
                size: required(entry, "Size")?
                    .parse()
                    .map_err(|_| malformed("an object size that is not a number"))?,
                etag: required(entry, "ETag")?.to_owned(),
                last_modified: optional_time(entry, "LastModified")?,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let common_prefixes = root
        .all("CommonPrefixes")
        .map(|entry| name(required(entry, "Prefix")?))
        .collect::<Result<Vec<_>, Error>>()?;
    let truncated = root.text_of("IsTruncated") == Some("true");
    let next_continuation_token = match (truncated, root.text_of("NextContinuationToken")) {
        (true, Some(token)) if !token.is_empty() => Some(token.to_owned()),
        (true, _) => {
            return Err(malformed(
                "a truncated listing without a continuation token",
            ));
        }
        (false, _) => None,
    };
    Ok(ListPage {
        objects,
        common_prefixes,
        next_continuation_token,
    })
}

/// The ETag of a `CopyObject` answer. The server may answer 200 and still report an error in the body; that error is
/// returned as the [`Error::Service`] it is.
pub(crate) fn copy_result(body: &[u8], status: u16) -> Result<String, Error> {
    if let Some((code, message, request_id)) = error_document(body) {
        return Err(Error::Service {
            status,
            code: Some(code),
            message,
            request_id,
        });
    }
    let root = rooted(body, "CopyObjectResult")?;
    Ok(required(&root, "ETag")?.to_owned())
}

fn rooted(body: &[u8], name: &'static str) -> Result<Node, Error> {
    let root = tree(body)?;
    if root.name == name {
        Ok(root)
    } else {
        Err(malformed("an answer with an unexpected root element"))
    }
}

fn required<'a>(node: &'a Node, name: &str) -> Result<&'a str, Error> {
    node.text_of(name)
        .ok_or_else(|| malformed("an answer missing a required element"))
}

fn optional_time(node: &Node, name: &str) -> Result<Option<DateTime<Utc>>, Error> {
    node.text_of(name)
        .map(|text| {
            DateTime::parse_from_rfc3339(text)
                .map(|time| time.with_timezone(&Utc))
                .map_err(|_| malformed("a timestamp that is not ISO 8601"))
        })
        .transpose()
}

/// `text` URL-decoded once: `%XX` is a byte, `+` a space, the result UTF-8.
fn url_decode(text: &str) -> Result<String, Error> {
    let bad = || malformed("a URL-encoded name that does not decode");
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.bytes();
    while let Some(byte) = rest.next() {
        match byte {
            b'+' => bytes.push(b' '),
            b'%' => {
                let high = rest.next().and_then(hex_value).ok_or_else(bad)?;
                let low = rest.next().and_then(hex_value).ok_or_else(bad)?;
                bytes.push(high << 4 | low);
            }
            other => bytes.push(other),
        }
    }
    String::from_utf8(bytes).map_err(|_| bad())
}

fn hex_value(digit: u8) -> Option<u8> {
    char::from(digit)
        .to_digit(16)
        .and_then(|value| u8::try_from(value).ok())
}
