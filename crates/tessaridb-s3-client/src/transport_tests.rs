use super::{endpoint_url, host_header, request_path};
use crate::Error;
use crate::{BucketName, ObjectKey};

#[test]
fn endpoints_are_scheme_host_and_port_only() {
    for good in [
        "http://127.0.0.1:47920",
        "https://s3.example.com",
        "https://s3.example.com/",
    ] {
        assert!(endpoint_url(good).is_ok(), "{good}");
    }
    let cases = [
        ("ftp://host", "an http:// or https:// URL"),
        ("host:9000", "an http:// or https:// URL"),
        ("not a url", "an http:// or https:// URL"),
        ("https://host/prefix", "scheme, host and port only"),
        ("https://host/?a=1", "scheme, host and port only"),
        ("https://user:pw@host", "scheme, host and port only"),
    ];
    for (bad, expected) in cases {
        assert!(
            matches!(endpoint_url(bad), Err(Error::InvalidArgument { what: "endpoint", rule }) if rule == expected),
            "{bad}"
        );
    }
}

#[test]
fn the_host_header_carries_a_port_only_when_it_is_not_the_default() {
    let host = |url: &str| endpoint_url(url).map(|url| host_header(&url)).ok();
    assert_eq!(
        host("http://127.0.0.1:47920").as_deref(),
        Some("127.0.0.1:47920")
    );
    assert_eq!(
        host("https://s3.example.com:443").as_deref(),
        Some("s3.example.com")
    );
    assert_eq!(
        host("http://s3.example.com:80").as_deref(),
        Some("s3.example.com")
    );
}

#[test]
fn a_path_style_path_keeps_the_key_byte_for_byte() -> crate::Result<()> {
    let bucket = BucketName::new("photos")?;
    let key = ObjectKey::new("2026//a b+é.jpg")?;
    assert_eq!(request_path(None, None), "/");
    assert_eq!(request_path(Some(&bucket), None), "/photos");
    assert_eq!(
        request_path(Some(&bucket), Some(&key)),
        "/photos/2026//a b+é.jpg"
    );
    // The URL path is the SigV4 canonical URI of that path, so what is sent is what was signed.
    let mut url =
        endpoint_url("http://127.0.0.1:47920").map_err(|_| Error::Setup { what: "url" })?;
    url.set_path(&crate::sign::canonical_uri(&request_path(
        Some(&bucket),
        Some(&key),
    )));
    assert_eq!(url.path(), "/photos/2026//a%20b%2B%C3%A9.jpg");
    Ok(())
}

#[test]
fn the_http_client_accepts_the_tls_setup() {
    // The HTTP client checks the type of the TLS setup only when it is built, so building one is the test.
    let signer = crate::sign::Signer::new(crate::sign::Credentials::new("a", "b"), "us-east-1");
    let built = super::Transport::new("https://s3.example.com", signer);
    assert!(built.is_ok(), "{built:?}");
}
