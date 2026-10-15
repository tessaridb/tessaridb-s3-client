use std::path::Path;

use super::{PartFile, digests_of};
use crate::checksum::crc64nvme;
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
async fn the_file_digests_are_the_digests_of_its_bytes() -> crate::Result<()> {
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
    assert_eq!(
        digests_of(path).await?,
        (payload_sha256(&bytes), crc64nvme(&bytes))
    );
    Ok(())
}

#[tokio::test]
async fn a_missing_file_is_reported_before_anything_is_sent() -> crate::Result<()> {
    let directory = tempfile::tempdir().map_err(|source| Error::Io {
        what: "a test directory",
        source,
    })?;
    // Nothing listens on 47925: a request would fail differently from the refusal asserted here.
    let client =
        ClientBuilder::new("http://127.0.0.1:47925", Credentials::new("a", "b")).build()?;
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

/// A loopback server answering every request with `body` and a CRC64NVME that belongs to other bytes.
async fn damaging_server(
    body: &'static [u8],
) -> crate::Result<(String, tokio::task::JoinHandle<()>)> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|source| Error::Io {
            what: "a loopback listener",
            source,
        })?;
    let address = listener.local_addr().map_err(|source| Error::Io {
        what: "a loopback listener",
        source,
    })?;
    let wrong = crc64nvme(b"not the body");
    let server = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut request = [0_u8; 4096];
            let _ = socket.read(&mut request).await;
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nx-amz-checksum-crc64nvme: {wrong}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(body).await;
        }
    });
    Ok((format!("http://{address}"), server))
}

#[tokio::test]
async fn a_body_that_does_not_match_its_checksum_is_retried_and_never_handed_over()
-> crate::Result<()> {
    let (endpoint, server) = damaging_server(b"damaged").await?;
    let client = ClientBuilder::new(&endpoint, Credentials::new("a", "b")).build()?;
    let (bucket, key) = (BucketName::new("bucket")?, ObjectKey::new("key")?);
    let got = client.get_object(&bucket, &key).await;
    assert!(
        matches!(&got, Err(Error::RetriesExhausted { attempts: 3, last }) if matches!(**last, Error::ChecksumMismatch { algorithm: "CRC64NVME" })),
        "{got:?}"
    );
    let directory = tempfile::tempdir().map_err(|source| Error::Io {
        what: "a test directory",
        source,
    })?;
    let target = directory.path().join("target");
    std::fs::write(&target, b"untouched").map_err(|source| Error::Io {
        what: "the target",
        source,
    })?;
    let downloaded = client.download_file(&bucket, &key, &target).await;
    assert!(
        matches!(&downloaded, Err(Error::RetriesExhausted { .. })),
        "{downloaded:?}"
    );
    assert_eq!(
        std::fs::read(&target).ok().as_deref(),
        Some(&b"untouched"[..])
    );
    assert_eq!(names(directory.path()), vec!["target".to_owned()]);
    server.abort();
    Ok(())
}
