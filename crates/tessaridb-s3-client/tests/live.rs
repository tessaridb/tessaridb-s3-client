//! Calls against a running S3 server. They run when `TESSARIDB_S3_CLIENT_TEST_ENDPOINT`,
//! `TESSARIDB_S3_CLIENT_TEST_ACCESS_KEY` and `TESSARIDB_S3_CLIENT_TEST_SECRET_KEY` are set, and report that they were
//! skipped otherwise; with `TESSARIDB_S3_CLIENT_TEST_SESSION_TOKEN` too they run as those temporary credentials. Each
//! test makes its own bucket and removes it.

use std::time::{SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};
use tessaridb_s3_client::sign::Credentials;
use tessaridb_s3_client::{BucketName, ListObjects, ObjectKey};
use tessaridb_s3_client::{Client, ClientBuilder, Error, ErrorCategory, Result};

fn settings() -> Option<(String, String, String)> {
    let var = |name: &str| std::env::var(format!("TESSARIDB_S3_CLIENT_TEST_{name}")).ok();
    match (var("ENDPOINT"), var("ACCESS_KEY"), var("SECRET_KEY")) {
        (Some(endpoint), Some(access), Some(secret)) => Some((endpoint, access, secret)),
        _ => {
            eprintln!(
                "skipped: TESSARIDB_S3_CLIENT_TEST_ENDPOINT/_ACCESS_KEY/_SECRET_KEY are not set"
            );
            None
        }
    }
}

/// The client under test; `Ok(None)` only when the settings are absent — a client that cannot be built is an error.
fn client() -> Result<Option<Client>> {
    let Some((endpoint, access, secret)) = settings() else {
        return Ok(None);
    };
    ClientBuilder::new(&endpoint, credentials(&access, &secret))
        .build()
        .map(Some)
}

/// `access` with `secret` — temporary credentials (an STS answer) when a session token is set beside them.
fn credentials(access: &str, secret: &str) -> Credentials {
    let credentials = Credentials::new(access, secret);
    match std::env::var("TESSARIDB_S3_CLIENT_TEST_SESSION_TOKEN") {
        Ok(token) => credentials.with_session_token(&token),
        Err(_) => credentials,
    }
}

fn unique_bucket(tag: &str) -> Result<BucketName> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    BucketName::new(&format!("client-{tag}-{nanos}"))
}

fn quoted_md5(body: &[u8]) -> String {
    let digest = Md5::digest(body);
    format!(
        "\"{}\"",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

async fn empty_and_delete(client: &Client, bucket: &BucketName) -> Result<()> {
    let page = client.list_objects(bucket, &ListObjects::default()).await?;
    for object in page.objects {
        client
            .delete_object(bucket, &ObjectKey::new(&object.key)?)
            .await?;
    }
    client.delete_bucket(bucket).await
}

#[tokio::test]
async fn a_bucket_is_created_listed_and_deleted() -> Result<()> {
    let Some(client) = client()? else {
        return Ok(());
    };
    let bucket = unique_bucket("bucket")?;
    client.create_bucket(&bucket).await?;
    assert!(client.bucket_exists(&bucket).await?);
    let again = client.create_bucket(&bucket).await;
    assert!(
        matches!(&again, Err(error) if error.category() == ErrorCategory::Conflict),
        "{again:?}"
    );
    let names: Vec<String> = client
        .list_buckets()
        .await?
        .into_iter()
        .map(|b| b.name)
        .collect();
    assert!(names.contains(&bucket.to_string()), "{names:?}");
    client.delete_bucket(&bucket).await?;
    assert!(!client.bucket_exists(&bucket).await?);
    let gone = client.delete_bucket(&bucket).await;
    assert!(
        matches!(&gone, Err(error) if error.s3_code() == Some("NoSuchBucket")),
        "{gone:?}"
    );
    Ok(())
}

#[tokio::test]
async fn an_object_round_trips_byte_for_byte_under_an_awkward_key() -> Result<()> {
    let Some(client) = client()? else {
        return Ok(());
    };
    let bucket = unique_bucket("object")?;
    client.create_bucket(&bucket).await?;
    let key = ObjectKey::new("dir//a b+c%d é~.txt")?;
    let body = b"hello, object\n\x00\xff".to_vec();
    let etag = client
        .put_object(&bucket, &key, body.clone(), Some("text/plain"))
        .await?;
    assert_eq!(etag, quoted_md5(&body));
    assert_eq!(
        client.get_object(&bucket, &key).await?.as_ref(),
        body.as_slice()
    );
    let info = client.head_object(&bucket, &key).await?;
    assert_eq!(
        (info.size, info.etag.as_str()),
        (u64::try_from(body.len()).unwrap_or(0), etag.as_str())
    );
    assert!(info.last_modified.is_some());
    // The server keeps the checksum the client declared, so downloads have something to check against.
    let crc = crc_fast::checksum(crc_fast::CrcAlgorithm::Crc64Nvme, &body).to_be_bytes();
    let expected_crc = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, crc);
    assert_eq!(info.crc64nvme.as_deref(), Some(expected_crc.as_str()));
    client.delete_object(&bucket, &key).await?;
    let missing = client.get_object(&bucket, &key).await;
    assert!(
        matches!(&missing, Err(error) if error.s3_code() == Some("NoSuchKey")),
        "{missing:?}"
    );
    let missing_head = client.head_object(&bucket, &key).await;
    assert!(
        matches!(
            &missing_head,
            Err(Error::Service {
                status: 404,
                code: None,
                ..
            })
        ),
        "{missing_head:?}"
    );
    client.delete_object(&bucket, &key).await?;
    client.delete_bucket(&bucket).await
}

#[tokio::test]
async fn a_listing_pages_in_byte_order_and_rolls_up_prefixes_once() -> Result<()> {
    let Some(client) = client()? else {
        return Ok(());
    };
    let bucket = unique_bucket("list")?;
    client.create_bucket(&bucket).await?;
    let keys = ["a/1", "a/2", "b c", "b+c", "Z", "é", "€"];
    for key in keys {
        client
            .put_object(
                &bucket,
                &ObjectKey::new(key)?,
                key.as_bytes().to_vec(),
                None,
            )
            .await?;
    }
    let mut expected: Vec<&str> = keys.to_vec();
    expected.sort_unstable_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    let mut seen = Vec::new();
    let mut listing = ListObjects {
        max_keys: Some(2),
        ..ListObjects::default()
    };
    loop {
        let page = client.list_objects(&bucket, &listing).await?;
        assert!(page.objects.len() <= 2);
        seen.extend(page.objects.into_iter().map(|object| object.key));
        match page.next_continuation_token {
            Some(token) => listing.continuation_token = Some(token),
            None => break,
        }
    }
    assert_eq!(seen, expected);
    let rolled = ListObjects {
        delimiter: Some("/".to_owned()),
        ..ListObjects::default()
    };
    let page = client.list_objects(&bucket, &rolled).await?;
    assert_eq!(page.common_prefixes, vec!["a/".to_owned()]);
    assert!(
        page.objects
            .iter()
            .all(|object| !object.key.starts_with("a/"))
    );
    let prefixed = ListObjects {
        prefix: Some("b".to_owned()),
        ..ListObjects::default()
    };
    let names: Vec<String> = client
        .list_objects(&bucket, &prefixed)
        .await?
        .objects
        .into_iter()
        .map(|o| o.key)
        .collect();
    assert_eq!(names, vec!["b c".to_owned(), "b+c".to_owned()]);
    let zero = client
        .list_objects(
            &bucket,
            &ListObjects {
                max_keys: Some(0),
                ..ListObjects::default()
            },
        )
        .await;
    assert!(matches!(
        zero,
        Err(Error::InvalidArgument {
            what: "max keys",
            ..
        })
    ));
    empty_and_delete(&client, &bucket).await
}

#[tokio::test]
async fn a_copy_carries_the_bytes_and_their_etag() -> Result<()> {
    let Some(client) = client()? else {
        return Ok(());
    };
    let bucket = unique_bucket("copy")?;
    client.create_bucket(&bucket).await?;
    let (source, target) = (
        ObjectKey::new("src/a b.bin")?,
        ObjectKey::new("dst/c+d.bin")?,
    );
    let body = vec![7_u8; 4096];
    client
        .put_object(&bucket, &source, body.clone(), None)
        .await?;
    let etag = client
        .copy_object(&bucket, &source, &bucket, &target)
        .await?;
    assert_eq!(etag, quoted_md5(&body));
    assert_eq!(
        client.get_object(&bucket, &target).await?.as_ref(),
        body.as_slice()
    );
    let missing = client
        .copy_object(&bucket, &ObjectKey::new("nope")?, &bucket, &target)
        .await;
    assert!(
        matches!(&missing, Err(error) if error.category() == ErrorCategory::NotFound),
        "{missing:?}"
    );
    empty_and_delete(&client, &bucket).await
}

#[tokio::test]
async fn a_wrong_secret_is_refused_as_unauthorized_and_not_retried() -> Result<()> {
    let Some((endpoint, access, _)) = settings() else {
        return Ok(());
    };
    let client = ClientBuilder::new(&endpoint, credentials(&access, "not-the-secret")).build()?;
    let refused = client.list_buckets().await;
    assert!(
        matches!(&refused, Err(error @ Error::Service { .. }) if error.category() == ErrorCategory::Unauthorized),
        "{refused:?}"
    );
    Ok(())
}

#[tokio::test]
async fn an_unreachable_server_is_retried_then_reported_unavailable() -> Result<()> {
    if settings().is_none() {
        return Ok(());
    }
    // 47925 is in the band these tests own and nothing listens there.
    let client =
        ClientBuilder::new("http://127.0.0.1:47925", Credentials::new("a", "b")).build()?;
    let refused = client.list_buckets().await;
    assert!(
        matches!(&refused, Err(error @ Error::RetriesExhausted { attempts: 3, .. }) if error.category() == ErrorCategory::Unavailable),
        "{refused:?}"
    );
    Ok(())
}

fn io(what: &'static str) -> impl FnOnce(std::io::Error) -> Error {
    move |source| Error::Io { what, source }
}

#[tokio::test]
async fn a_file_of_several_mebibytes_streams_up_and_down_unchanged() -> Result<()> {
    let Some(client) = client()? else {
        return Ok(());
    };
    let bucket = unique_bucket("file")?;
    client.create_bucket(&bucket).await?;
    let directory = tempfile::tempdir().map_err(io("a test directory"))?;
    let source = directory.path().join("source.bin");
    let bytes: Vec<u8> = (0..5_242_883_u32)
        .map(|n| u8::try_from(n.wrapping_mul(2_654_435_761) >> 24).unwrap_or(0))
        .collect();
    std::fs::write(&source, &bytes).map_err(io("the source file"))?;
    let key = ObjectKey::new("files/source copy.bin")?;
    let etag = client
        .upload_file(&bucket, &key, &source, Some("application/octet-stream"))
        .await?;
    assert_eq!(etag, quoted_md5(&bytes));
    let target = directory.path().join("target.bin");
    std::fs::write(&target, b"stale").map_err(io("the target file"))?;
    let written = client.download_file(&bucket, &key, &target).await?;
    assert_eq!(written, u64::try_from(bytes.len()).unwrap_or(0));
    assert!(std::fs::read(&target).map_err(io("the target file"))? == bytes);
    let mut names: Vec<String> = std::fs::read_dir(directory.path())
        .map_err(io("the test directory"))?
        .filter_map(|entry| {
            entry
                .ok()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec!["source.bin".to_owned(), "target.bin".to_owned()]
    );
    empty_and_delete(&client, &bucket).await
}

#[tokio::test]
async fn a_missing_object_leaves_the_target_file_untouched() -> Result<()> {
    let Some(client) = client()? else {
        return Ok(());
    };
    let bucket = unique_bucket("nofile")?;
    client.create_bucket(&bucket).await?;
    let directory = tempfile::tempdir().map_err(io("a test directory"))?;
    let target = directory.path().join("keep.txt");
    std::fs::write(&target, b"keep me").map_err(io("the target file"))?;
    let missing = client
        .download_file(&bucket, &ObjectKey::new("absent")?, &target)
        .await;
    assert!(
        matches!(&missing, Err(error) if error.s3_code() == Some("NoSuchKey")),
        "{missing:?}"
    );
    assert_eq!(
        std::fs::read(&target).ok().as_deref(),
        Some(&b"keep me"[..])
    );
    assert_eq!(
        std::fs::read_dir(directory.path())
            .map_err(io("the test directory"))?
            .count(),
        1
    );
    client.delete_bucket(&bucket).await
}

/// `length` bytes from xorshift64: no two 8 MiB windows of it are equal.
fn pseudo_random(length: usize) -> Vec<u8> {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_be_bytes()[0]
        })
        .collect()
}

fn crc64nvme(bytes: &[u8]) -> String {
    let crc = crc_fast::checksum(crc_fast::CrcAlgorithm::Crc64Nvme, bytes).to_be_bytes();
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, crc)
}

#[tokio::test]
async fn a_large_file_goes_up_in_parts_and_comes_back_whole() -> Result<()> {
    let Some(client) = client()? else {
        return Ok(());
    };
    let bucket = unique_bucket("multipart")?;
    client.create_bucket(&bucket).await?;
    let directory = tempfile::tempdir().map_err(io("a test directory"))?;
    let source = directory.path().join("large.bin");
    // No period: parts must differ from each other, or a part sent from the wrong offset would go unnoticed.
    let bytes = pseudo_random(21 * 1024 * 1024 + 5);
    std::fs::write(&source, &bytes).map_err(io("the source file"))?;
    let key = ObjectKey::new("big/large.bin")?;
    let etag = client.upload_file(&bucket, &key, &source, None).await?;
    assert!(etag.ends_with("-3\""), "{etag}");
    let info = client.head_object(&bucket, &key).await?;
    assert_eq!(info.size, u64::try_from(bytes.len()).unwrap_or(0));
    assert_eq!(info.crc64nvme, Some(crc64nvme(&bytes)));
    let target = directory.path().join("back.bin");
    client.download_file(&bucket, &key, &target).await?;
    assert!(std::fs::read(&target).map_err(io("the target file"))? == bytes);
    empty_and_delete(&client, &bucket).await
}

#[tokio::test]
async fn an_upload_is_resumed_by_its_id_and_an_aborted_one_is_gone() -> Result<()> {
    let Some(client) = client()? else {
        return Ok(());
    };
    let bucket = unique_bucket("resume")?;
    client.create_bucket(&bucket).await?;
    let directory = tempfile::tempdir().map_err(io("a test directory"))?;
    let source = directory.path().join("resume.bin");
    let bytes = pseudo_random(9 * 1024 * 1024);
    std::fs::write(&source, &bytes).map_err(io("the source file"))?;
    let key = ObjectKey::new("resumed.bin")?;
    let upload = client.begin_upload(&bucket, &key, None).await?;
    let etag = client
        .resume_upload(&bucket, &key, &source, &upload)
        .await?;
    assert!(etag.ends_with("-2\""), "{etag}");
    assert_eq!(client.get_object(&bucket, &key).await?.len(), bytes.len());
    let again = client.resume_upload(&bucket, &key, &source, &upload).await;
    assert!(
        matches!(&again, Err(error) if error.s3_code() == Some("NoSuchUpload")),
        "{again:?}"
    );
    let aborted = client
        .begin_upload(&bucket, &ObjectKey::new("aborted.bin")?, None)
        .await?;
    client
        .abort_upload(&bucket, &ObjectKey::new("aborted.bin")?, &aborted)
        .await?;
    let gone = client
        .resume_upload(&bucket, &ObjectKey::new("aborted.bin")?, &source, &aborted)
        .await;
    assert!(
        matches!(&gone, Err(error) if error.s3_code() == Some("NoSuchUpload")),
        "{gone:?}"
    );
    empty_and_delete(&client, &bucket).await
}

#[tokio::test]
async fn presigned_links_carry_one_request_and_a_tampered_one_is_refused() -> Result<()> {
    let Some(client) = client()? else {
        return Ok(());
    };
    let bucket = unique_bucket("presign")?;
    client.create_bucket(&bucket).await?;
    let key = ObjectKey::new("shared/a b+c.txt")?;
    let ten_minutes = std::time::Duration::from_secs(600);
    // A plain HTTP client, as a browser or a script holding the link would be; it needs a TLS provider chosen.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let plain = reqwest::Client::new();
    let transport = |source: reqwest::Error| Error::Io {
        what: "a plain HTTP request",
        source: std::io::Error::other(source),
    };
    let put_url = client.presign_put(&bucket, &key, ten_minutes)?;
    let put = plain
        .put(&put_url)
        .body("posted through a link")
        .send()
        .await
        .map_err(transport)?;
    assert_eq!(put.status().as_u16(), 200);
    let get_url = client.presign_get(&bucket, &key, ten_minutes)?;
    let got = plain.get(&get_url).send().await.map_err(transport)?;
    assert_eq!(got.status().as_u16(), 200);
    assert_eq!(
        got.bytes().await.map_err(transport)?.as_ref(),
        b"posted through a link"
    );
    let mut tampered = get_url.clone();
    let last = tampered
        .pop()
        .map_or('0', |c| if c == '0' { '1' } else { '0' });
    tampered.push(last);
    let refused = plain.get(&tampered).send().await.map_err(transport)?;
    assert_eq!(refused.status().as_u16(), 403);
    let as_put = plain
        .put(&get_url)
        .body("not allowed")
        .send()
        .await
        .map_err(transport)?;
    assert_eq!(as_put.status().as_u16(), 403);
    assert!(matches!(
        client.presign_get(&bucket, &key, std::time::Duration::ZERO),
        Err(Error::InvalidArgument { .. })
    ));
    empty_and_delete(&client, &bucket).await
}

#[tokio::test]
async fn temporary_credentials_without_their_session_token_are_refused() -> Result<()> {
    let (Some((endpoint, access, secret)), Ok(_)) = (
        settings(),
        std::env::var("TESSARIDB_S3_CLIENT_TEST_SESSION_TOKEN"),
    ) else {
        eprintln!("skipped: TESSARIDB_S3_CLIENT_TEST_SESSION_TOKEN is not set");
        return Ok(());
    };
    let bare = ClientBuilder::new(&endpoint, Credentials::new(&access, &secret)).build()?;
    let refused = bare.list_buckets().await;
    assert!(
        matches!(&refused, Err(Error::Service { status: 400, code: Some(code), .. }) if code == "InvalidToken"),
        "{refused:?}"
    );
    Ok(())
}
