//! Where a command reads or writes: `s3://bucket/key` on the server, or a path on this machine.

use std::path::PathBuf;

use tessaridb_s3_client::{BucketName, ObjectKey};

use crate::error::{CliError, Result};

/// A command's source or destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Location {
    /// A bucket and an optional key (or prefix) on the server.
    Remote {
        bucket: BucketName,
        key: Option<String>,
    },
    /// A path on this machine.
    Local(PathBuf),
}

impl Location {
    /// `text` as a location: `s3://bucket[/key]`, otherwise a local path.
    pub(crate) fn parse(text: &str) -> Result<Self> {
        let Some(rest) = text.strip_prefix("s3://") else {
            if text.is_empty() {
                return Err(CliError::Invalid("a location is empty".to_owned()));
            }
            return Ok(Self::Local(PathBuf::from(text)));
        };
        let (bucket, key) = rest
            .split_once('/')
            .map_or((rest, None), |(bucket, key)| (bucket, Some(key)));
        let bucket = BucketName::new(bucket)
            .map_err(|error| CliError::Invalid(format!("{text}: {error}")))?;
        Ok(Self::Remote {
            bucket,
            key: key.filter(|key| !key.is_empty()).map(str::to_owned),
        })
    }

    /// The bucket and object key of a remote location naming one object.
    pub(crate) fn object(&self) -> Result<(BucketName, ObjectKey)> {
        match self {
            Self::Remote {
                bucket,
                key: Some(key),
            } => Ok((
                bucket.clone(),
                ObjectKey::new(key)
                    .map_err(|error| CliError::Invalid(format!("s3://{bucket}/{key}: {error}")))?,
            )),
            Self::Remote { bucket, key: None } => Err(CliError::Invalid(format!(
                "s3://{bucket} names a bucket, not an object"
            ))),
            Self::Local(path) => Err(CliError::Invalid(format!(
                "{} is not an s3:// location",
                path.display()
            ))),
        }
    }

    /// The bucket of a remote location naming a bucket and nothing else.
    pub(crate) fn bucket(&self) -> Result<BucketName> {
        match self {
            Self::Remote { bucket, key: None } => Ok(bucket.clone()),
            Self::Remote {
                bucket,
                key: Some(key),
            } => Err(CliError::Invalid(format!(
                "s3://{bucket}/{key} names an object, not a bucket"
            ))),
            Self::Local(path) => Err(CliError::Invalid(format!(
                "{} is not an s3:// location",
                path.display()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Location;
    use crate::error::CliError;

    #[test]
    fn s3_locations_name_a_bucket_and_maybe_a_key_and_anything_else_is_local() {
        let remote = Location::parse("s3://photos/2026/a b.jpg");
        assert!(
            matches!(&remote, Ok(Location::Remote { bucket, key: Some(key) }) if bucket.as_str() == "photos" && key == "2026/a b.jpg")
        );
        assert!(matches!(
            Location::parse("s3://photos"),
            Ok(Location::Remote { key: None, .. })
        ));
        assert!(matches!(
            Location::parse("s3://photos/"),
            Ok(Location::Remote { key: None, .. })
        ));
        assert!(matches!(
            Location::parse("./notes.txt"),
            Ok(Location::Local(_))
        ));
        assert!(matches!(
            Location::parse("s3://Bad_Bucket/k"),
            Err(CliError::Invalid(_))
        ));
        assert!(matches!(Location::parse(""), Err(CliError::Invalid(_))));
    }

    #[test]
    fn an_object_location_needs_a_key_and_a_bucket_location_must_not_have_one() {
        let bucket_only = Location::parse("s3://photos").map_err(|e| e.to_string());
        assert!(
            bucket_only
                .as_ref()
                .is_ok_and(|location| location.object().is_err() && location.bucket().is_ok())
        );
        let object = Location::parse("s3://photos/a").map_err(|e| e.to_string());
        assert!(
            object
                .as_ref()
                .is_ok_and(|location| location.object().is_ok() && location.bucket().is_err())
        );
        let dotted = Location::parse("s3://photos/a/../b").map_err(|e| e.to_string());
        assert!(
            dotted
                .as_ref()
                .is_ok_and(|location| matches!(location.object(), Err(CliError::Invalid(_))))
        );
    }
}
