use super::{Error, ErrorCategory};
use std::time::Duration;

fn service(status: u16, code: Option<&str>) -> Error {
    Error::Service {
        status,
        code: code.map(str::to_owned),
        message: None,
        request_id: Some("REQ1".to_owned()),
    }
}

#[test]
fn an_invalid_argument_names_the_argument_and_the_rule_it_breaks() {
    let error = Error::InvalidArgument {
        what: "bucket name",
        rule: "3 to 63 characters",
    };
    assert_eq!(error.to_string(), "invalid bucket name: 3 to 63 characters");
    assert_eq!(error.category(), ErrorCategory::Validation);
}

#[test]
fn a_documented_code_decides_the_category_over_its_status() {
    use ErrorCategory::{
        Conflict, Forbidden, NotFound, Timeout, Unauthorized, Unavailable, Validation,
    };
    let cases = [
        (404, "NoSuchKey", NotFound),
        (404, "NoSuchBucket", NotFound),
        (409, "BucketNotEmpty", Conflict),
        (409, "BucketAlreadyOwnedByYou", Conflict),
        (412, "PreconditionFailed", Conflict),
        (403, "SignatureDoesNotMatch", Unauthorized),
        (403, "InvalidAccessKeyId", Unauthorized),
        (403, "RequestTimeTooSkewed", Unauthorized),
        (403, "AccessDenied", Forbidden),
        (400, "RequestTimeout", Timeout),
        (503, "SlowDown", Unavailable),
        (500, "InternalError", Unavailable),
        (400, "BadDigest", Validation),
        (501, "NotImplemented", Validation),
    ];
    for (status, code, expected) in cases {
        assert_eq!(service(status, Some(code)).category(), expected, "{code}");
    }
}

#[test]
fn without_a_known_code_the_status_decides() {
    use ErrorCategory::{
        Conflict, Forbidden, Internal, NotFound, Timeout, Unauthorized, Unavailable, Validation,
    };
    let cases = [
        (400, Validation),
        (401, Unauthorized),
        (403, Forbidden),
        (404, NotFound),
        (408, Timeout),
        (409, Conflict),
        (412, Conflict),
        (416, Validation),
        (429, Unavailable),
        (502, Unavailable),
        (301, Internal),
    ];
    for (status, expected) in cases {
        assert_eq!(service(status, None).category(), expected, "{status}");
        assert_eq!(
            service(status, Some("SomethingNew")).category(),
            expected,
            "{status}"
        );
    }
}

#[test]
fn only_transient_failures_are_retryable() {
    for retryable in [
        service(503, Some("SlowDown")),
        service(500, Some("InternalError")),
        service(502, None),
        service(504, None),
        service(429, None),
        service(400, Some("RequestTimeout")),
        Error::Timeout(Duration::from_secs(1)),
    ] {
        assert!(retryable.is_retryable(), "{retryable:?}");
    }
    for permanent in [
        service(403, Some("AccessDenied")),
        service(403, Some("SignatureDoesNotMatch")),
        service(404, Some("NoSuchKey")),
        service(400, Some("MalformedXML")),
        service(412, Some("PreconditionFailed")),
        service(501, Some("NotImplemented")),
        Error::InvalidResponse { what: "x" },
    ] {
        assert!(!permanent.is_retryable(), "{permanent:?}");
    }
}

#[test]
fn exhausted_retries_keep_the_last_cause_its_category_and_its_code() {
    let error = Error::RetriesExhausted {
        attempts: 3,
        last: Box::new(service(503, Some("SlowDown"))),
    };
    assert_eq!(error.to_string(), "gave up after 3 attempts");
    assert_eq!(error.category(), ErrorCategory::Unavailable);
    assert_eq!(error.s3_code(), Some("SlowDown"));
    assert!(!error.is_retryable());
    let source = std::error::Error::source(&error).map(ToString::to_string);
    assert_eq!(
        source.as_deref(),
        Some("the server refused the request: HTTP 503 SlowDown")
    );
}
