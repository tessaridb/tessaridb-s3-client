use md5::{Digest as _, Md5};

use super::plan::{FilePlan, PartPlan, missing, part_size, plan};
use crate::Error;
use crate::checksum::crc64nvme;
use crate::sign::{hex, payload_sha256};
use crate::xml::StoredPart;

const MIB: u64 = 1024 * 1024;

#[test]
fn parts_are_eight_mib_until_ten_thousand_would_not_hold_the_file() {
    assert_eq!(part_size(1).ok(), Some(8 * MIB));
    assert_eq!(part_size(80_000 * MIB).ok(), Some(8 * MIB));
    assert_eq!(part_size(80_000 * MIB + 1).ok(), Some(9 * MIB));
    assert_eq!(
        part_size(10_000 * 5 * 1024 * MIB).ok(),
        Some(5 * 1024 * MIB)
    );
    assert!(matches!(
        part_size(10_000 * 5 * 1024 * MIB + 1),
        Err(Error::InvalidArgument {
            rule: "at most 10 000 parts of 5 GiB",
            ..
        })
    ));
}

fn part(number: u16, etag: &str, length: u64) -> PartPlan {
    PartPlan {
        number,
        offset: u64::from(number.saturating_sub(1)).saturating_mul(8 * MIB),
        length,
        sha256: String::new(),
        etag: etag.to_owned(),
        crc: String::new(),
    }
}

#[test]
fn only_parts_the_server_holds_intact_are_skipped() {
    let plan = FilePlan {
        length: 0,
        crc: String::new(),
        parts: vec![
            part(1, "\"a\"", 8 * MIB),
            part(2, "\"b\"", 8 * MIB),
            part(3, "\"c\"", 5),
        ],
    };
    let stored = |number, etag: &str, size| StoredPart {
        number,
        etag: etag.to_owned(),
        size,
    };
    let listed = vec![
        stored(1, "\"a\"", 8 * MIB),
        stored(2, "\"other\"", 8 * MIB),
        stored(3, "\"c\"", 4),
    ];
    let numbers: Vec<u16> = missing(&plan, &listed)
        .into_iter()
        .map(|part| part.number)
        .collect();
    assert_eq!(numbers, vec![2, 3]);
    assert_eq!(missing(&plan, &[]).len(), 3);
}

#[tokio::test]
async fn one_read_plans_every_part_and_the_whole_file() -> crate::Result<()> {
    let directory = tempfile::tempdir().map_err(|source| Error::Io {
        what: "a test directory",
        source,
    })?;
    let path = directory.path().join("big");
    let bytes: Vec<u8> = (0..(17 * MIB + 3))
        .map(|n| u8::try_from(n % 253).unwrap_or(0))
        .collect();
    std::fs::write(&path, &bytes).map_err(|source| Error::Io {
        what: "the file",
        source,
    })?;
    let length = u64::try_from(bytes.len()).unwrap_or(0);
    let planned = plan(path, length).await?;
    assert_eq!(planned.crc, crc64nvme(&bytes));
    let shape: Vec<(u16, u64, u64)> = planned
        .parts
        .iter()
        .map(|p| (p.number, p.offset, p.length))
        .collect();
    assert_eq!(
        shape,
        vec![
            (1, 0, 8 * MIB),
            (2, 8 * MIB, 8 * MIB),
            (3, 16 * MIB, MIB + 3)
        ]
    );
    for part in &planned.parts {
        let start = usize::try_from(part.offset).unwrap_or(0);
        let end = start.saturating_add(usize::try_from(part.length).unwrap_or(0));
        let range = &bytes[start..end];
        assert_eq!(part.sha256, payload_sha256(range));
        assert_eq!(part.etag, format!("\"{}\"", hex(&Md5::digest(range))));
        assert_eq!(part.crc, crc64nvme(range));
    }
    Ok(())
}

#[tokio::test]
async fn parts_that_keep_failing_transiently_leave_the_upload_open_to_resume() -> crate::Result<()>
{
    use crate::sign::Credentials;
    use crate::test_server::{response, serve};
    use crate::{BucketName, ClientBuilder, ErrorCategory, ObjectKey};
    let (endpoint, seen, server) = serve(|_, head| {
        if head.starts_with("post") && head.contains("?uploads") {
            let body = b"<InitiateMultipartUploadResult><UploadId>fake-id</UploadId></InitiateMultipartUploadResult>";
            response("200 OK", &[], body, body.len())
        } else if head.starts_with("get") {
            let body = b"<ListPartsResult><IsTruncated>false</IsTruncated></ListPartsResult>";
            response("200 OK", &[], body, body.len())
        } else {
            let body = b"<Error><Code>SlowDown</Code><Message>slow</Message></Error>";
            response("503 Slow Down", &[], body, body.len())
        }
    })
    .await?;
    let directory = tempfile::tempdir().map_err(|source| Error::Io {
        what: "a test directory",
        source,
    })?;
    let path = directory.path().join("big");
    std::fs::write(&path, vec![7_u8; usize::try_from(17 * MIB).unwrap_or(0)]).map_err(
        |source| Error::Io {
            what: "the file",
            source,
        },
    )?;
    let client = ClientBuilder::new(&endpoint, Credentials::new("a", "b")).build()?;
    let result = client
        .upload_file(
            &BucketName::new("bucket")?,
            &ObjectKey::new("big")?,
            &path,
            None,
        )
        .await;
    match &result {
        Err(error @ Error::UploadInterrupted { upload_id, .. }) => {
            assert_eq!(upload_id, "fake-id");
            assert_eq!(error.category(), ErrorCategory::Unavailable);
            assert_eq!(error.s3_code(), Some("SlowDown"));
        }
        other => panic!("expected an interrupted upload, got {other:?}"),
    }
    let heads = seen.lock().map(|heads| heads.clone()).unwrap_or_default();
    assert!(
        !heads.iter().any(|head| head.starts_with("delete")),
        "the upload was aborted: {heads:?}"
    );
    server.abort();
    Ok(())
}
