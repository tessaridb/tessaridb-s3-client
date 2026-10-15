//! The AWS published SigV4 examples (S3 API reference, sig-v4-header-based-auth and sigv4-query-string-auth) are the
//! first tests: a signer that does not reproduce them byte for byte is wrong. Each has a negative variant, so a signer
//! that ignored the varied input would fail it.

use chrono::{TimeZone, Utc};

use super::{Credentials, Signer, canonical_uri};

const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
const SECRET: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

fn signer(region: &str) -> Signer {
    Signer::new(Credentials::new(ACCESS_KEY, SECRET), region)
}

fn example_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0)
        .single()
        .expect("a valid instant")
}

fn get_headers(range: &str) -> Vec<(String, String)> {
    vec![
        (
            "Host".to_owned(),
            "examplebucket.s3.amazonaws.com".to_owned(),
        ),
        ("Range".to_owned(), range.to_owned()),
        ("x-amz-content-sha256".to_owned(), EMPTY_SHA256.to_owned()),
        ("x-amz-date".to_owned(), "20130524T000000Z".to_owned()),
    ]
}

#[test]
fn the_published_header_get_example_is_reproduced() {
    let authorization = signer("us-east-1").authorization(
        "GET",
        "/test.txt",
        &[],
        &get_headers("bytes=0-9"),
        EMPTY_SHA256,
        example_time(),
    );
    assert_eq!(
        authorization,
        "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,\
         SignedHeaders=host;range;x-amz-content-sha256;x-amz-date,\
         Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
    );
    // One header value changed, and another region: each must change the signature.
    let other_range = signer("us-east-1").authorization(
        "GET",
        "/test.txt",
        &[],
        &get_headers("bytes=0-8"),
        EMPTY_SHA256,
        example_time(),
    );
    assert!(
        !other_range.ends_with("f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41")
    );
    let other_region = signer("eu-west-1").authorization(
        "GET",
        "/test.txt",
        &[],
        &get_headers("bytes=0-9"),
        EMPTY_SHA256,
        example_time(),
    );
    assert!(
        !other_region.ends_with("f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41")
    );
}

#[test]
fn the_published_presigned_get_example_is_reproduced() {
    let query = signer("us-east-1")
        .presign(
            "GET",
            "/test.txt",
            "examplebucket.s3.amazonaws.com",
            86_400,
            example_time(),
        )
        .expect("a lifetime within seven days");
    let signature = query
        .iter()
        .find(|(name, _)| name == "X-Amz-Signature")
        .map(|(_, value)| value.as_str());
    assert_eq!(
        signature,
        Some("aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404")
    );
    let names: Vec<&str> = query.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        [
            "X-Amz-Algorithm",
            "X-Amz-Credential",
            "X-Amz-Date",
            "X-Amz-Expires",
            "X-Amz-SignedHeaders",
            "X-Amz-Signature"
        ]
    );
    let shorter = signer("us-east-1")
        .presign(
            "GET",
            "/test.txt",
            "examplebucket.s3.amazonaws.com",
            3_600,
            example_time(),
        )
        .expect("a lifetime within seven days");
    assert_ne!(shorter.last(), query.last(), "the lifetime is signed");
}

#[test]
fn a_presigned_lifetime_is_one_second_to_seven_days() {
    let sign = |seconds| {
        signer("us-east-1").presign(
            "GET",
            "/test.txt",
            "examplebucket.s3.amazonaws.com",
            seconds,
            example_time(),
        )
    };
    assert!(sign(1).is_ok());
    assert!(sign(604_800).is_ok());
    assert!(matches!(
        sign(0),
        Err(crate::Error::InvalidArgument {
            what: "presigned URL lifetime",
            ..
        })
    ));
    assert!(matches!(
        sign(604_801),
        Err(crate::Error::InvalidArgument {
            what: "presigned URL lifetime",
            ..
        })
    ));
}

#[test]
fn a_path_is_encoded_once_with_slashes_kept_and_uppercase_hex() {
    assert_eq!(
        canonical_uri("/bucket/photos/a b+c~d*é.jpg"),
        "/bucket/photos/a%20b%2Bc~d%2A%C3%A9.jpg"
    );
    assert_eq!(canonical_uri(""), "/");
}

#[test]
fn the_secret_never_appears_in_debug_output() {
    let shown = format!("{:?}", Credentials::new(ACCESS_KEY, SECRET));
    assert!(shown.contains(ACCESS_KEY));
    assert!(!shown.contains("wJalrXUtnFEMI"), "{shown}");
}

#[test]
fn query_order_and_header_spacing_do_not_change_the_signature() {
    let sign = |query: &[(String, String)], range: &str| {
        signer("us-east-1").authorization(
            "GET",
            "/test.txt",
            query,
            &get_headers(range),
            EMPTY_SHA256,
            example_time(),
        )
    };
    let pair = |name: &str, value: &str| (name.to_owned(), value.to_owned());
    let sorted = sign(&[pair("acl", ""), pair("versionId", "3")], "bytes=0-9");
    assert_eq!(
        sign(&[pair("versionId", "3"), pair("acl", "")], "bytes=0-9"),
        sorted
    );
    // The server trims a header value and collapses inner runs of spaces; the signer must sign what it rebuilds.
    assert!(
        sign(&[], "  bytes=0-9  ")
            .ends_with("f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41")
    );
    assert_ne!(
        sign(&[pair("acl", "")], "bytes=0-9"),
        sign(&[pair("acl", "x")], "bytes=0-9"),
        "values are signed"
    );
}
