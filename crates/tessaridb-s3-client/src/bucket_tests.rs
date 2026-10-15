use super::BucketName;
use crate::Error;

fn rule(result: crate::Result<impl std::fmt::Debug>) -> &'static str {
    match result {
        Err(Error::InvalidArgument { rule, .. }) => rule,
        other => panic!("expected an invalid argument, got {other:?}"),
    }
}

#[test]
fn valid_bucket_names_are_accepted_as_written() {
    for name in ["abc", "my-bucket.2026", "a1b", &"a".repeat(63), "1.2.3.a"] {
        assert_eq!(
            BucketName::new(name).map(|n| n.to_string()).ok(),
            Some(name.to_owned()),
            "{name}"
        );
    }
}

#[test]
fn each_bucket_naming_rule_refuses_its_own_case() {
    let cases = [
        ("ab", "3 to 63 characters"),
        (&*"a".repeat(64), "3 to 63 characters"),
        ("My-Bucket", "only lowercase letters, digits, `.` and `-`"),
        ("my_bucket", "only lowercase letters, digits, `.` and `-`"),
        ("-bucket", "begins and ends with a letter or a digit"),
        ("bucket.", "begins and ends with a letter or a digit"),
        ("my..bucket", "no two adjacent `.`"),
        ("192.168.5.4", "not an IP address"),
        ("xn--bucket", "no reserved prefix or suffix"),
        ("sthree-bucket", "no reserved prefix or suffix"),
        ("amzn-s3-demo-bucket", "no reserved prefix or suffix"),
        ("bucket-s3alias", "no reserved prefix or suffix"),
        ("bucket--ol-s3", "no reserved prefix or suffix"),
        ("bucket.mrap", "no reserved prefix or suffix"),
        ("bucket--x-s3", "no reserved prefix or suffix"),
        ("bucket--table-s3", "no reserved prefix or suffix"),
    ];
    for (name, expected) in cases {
        assert_eq!(rule(BucketName::new(name)), expected, "{name}");
    }
}
