use super::ObjectKey;
use crate::Error;

fn rule(result: crate::Result<impl std::fmt::Debug>) -> &'static str {
    match result {
        Err(Error::InvalidArgument { rule, .. }) => rule,
        other => panic!("expected an invalid argument, got {other:?}"),
    }
}

#[test]
fn object_keys_keep_every_byte_and_refuse_what_cannot_travel() {
    for key in [
        "a",
        "photos//2026/ é.jpg",
        "trailing/",
        "a+b%20c",
        ".hidden",
        "a/..b/c.",
        &"k".repeat(1024),
    ] {
        assert_eq!(
            ObjectKey::new(key).map(|k| k.to_string()).ok(),
            Some(key.to_owned()),
            "{key}"
        );
    }
    assert_eq!(rule(ObjectKey::new("")), "at least one byte");
    assert_eq!(
        rule(ObjectKey::new(&"é".repeat(513))),
        "at most 1024 bytes of UTF-8"
    );
    for key in ["a/./b", "../a", "a/..", ".", "x/../../y"] {
        assert_eq!(
            rule(ObjectKey::new(key)),
            "no `.` or `..` path segment (a URL would collapse it)",
            "{key}"
        );
    }
}
