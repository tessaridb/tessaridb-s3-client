use std::path::Path;

use super::{PartFile, sha256_of};
use crate::sign::{Credentials, payload_sha256};
use crate::{BucketName, ClientBuilder, Error, ObjectKey};

fn names(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[tokio::test]
async fn an_unfinished_part_file_disappears_and_a_committed_one_replaces_the_target()
-> crate::Result<()> {
    let directory = tempfile::tempdir().map_err(|source| Error::Io {
        what: "a test directory",
        source,
    })?;
    let target = directory.path().join("object.bin");
    std::fs::write(&target, b"old").map_err(|source| Error::Io {
        what: "the target",
        source,
    })?;
    {
        let _abandoned = PartFile::create(&target).await?;
        assert_eq!(names(directory.path()).len(), 2);
    }
    assert_eq!(names(directory.path()), vec!["object.bin".to_owned()]);
    let mut part = PartFile::create(&target).await?;
    tokio::io::AsyncWriteExt::write_all(&mut part.file, b"new")
        .await
        .map_err(|source| Error::Io {
            what: "the part",
            source,
        })?;
    part.commit().await?;
    assert_eq!(names(directory.path()), vec!["object.bin".to_owned()]);
    assert_eq!(std::fs::read(&target).ok().as_deref(), Some(&b"new"[..]));
    Ok(())
}

#[tokio::test]
async fn the_file_hash_is_the_hash_of_its_bytes() -> crate::Result<()> {
    let directory = tempfile::tempdir().map_err(|source| Error::Io {
        what: "a test directory",
        source,
    })?;
    let path = directory.path().join("data");
    let bytes: Vec<u8> = (0..3_000_000_u32)
        .map(|n| u8::try_from(n % 251).unwrap_or(0))
        .collect();
    std::fs::write(&path, &bytes).map_err(|source| Error::Io {
        what: "the data",
        source,
    })?;
    assert_eq!(sha256_of(path).await?, payload_sha256(&bytes));
    Ok(())
}

#[tokio::test]
async fn a_file_over_five_gib_is_refused_before_anything_is_read_or_sent() -> crate::Result<()> {
    let directory = tempfile::tempdir().map_err(|source| Error::Io {
        what: "a test directory",
        source,
    })?;
    let path = directory.path().join("huge");
    let file = std::fs::File::create(&path).map_err(|source| Error::Io {
        what: "the file",
        source,
    })?;
    file.set_len(5 * 1024 * 1024 * 1024 + 1)
        .map_err(|source| Error::Io {
            what: "the file",
            source,
        })?;
    // Nothing listens on 47925: a request would fail differently from the refusal asserted here.
    let client =
        ClientBuilder::new("http://127.0.0.1:47925", Credentials::new("a", "b")).build()?;
    let refused = client
        .upload_file(
            &BucketName::new("bucket")?,
            &ObjectKey::new("huge")?,
            &path,
            None,
        )
        .await;
    assert!(
        matches!(
            refused,
            Err(Error::InvalidArgument {
                rule: "at most 5 GiB in one request",
                ..
            })
        ),
        "{refused:?}"
    );
    let missing = client
        .upload_file(
            &BucketName::new("bucket")?,
            &ObjectKey::new("k")?,
            directory.path().join("absent"),
            None,
        )
        .await;
    assert!(
        matches!(&missing, Err(error @ Error::Io { .. }) if error.category() == crate::ErrorCategory::NotFound),
        "{missing:?}"
    );
    Ok(())
}
