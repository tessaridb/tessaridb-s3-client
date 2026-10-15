use std::collections::HashMap;

use super::{from_file, load};
use crate::error::CliError;

fn write_profile(
    directory: &std::path::Path,
    text: &str,
    mode: u32,
) -> std::io::Result<std::path::PathBuf> {
    let path = directory.join("credentials");
    std::fs::write(&path, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    Ok(path)
}

const PROFILE: &str = "[default]\nendpoint = http://127.0.0.1:9000\naccess_key = AK\nsecret_key = s3cr3t-value\n\n[other]\nendpoint = https://s3.example.com\naccess_key = OK\nsecret_key = other-secret\nregion = eu-west-1\n";

#[test]
fn the_environment_wins_and_a_profile_is_read_by_name() {
    let env: HashMap<&str, &str> = [
        ("TESSARIDB_S3_ENDPOINT", "http://env:1"),
        ("TESSARIDB_S3_ACCESS_KEY", "EK"),
        ("TESSARIDB_S3_SECRET_KEY", "env-secret"),
    ]
    .into();
    let settings = load("default", |name| {
        env.get(name).map(|value| (*value).to_owned())
    })
    .expect("env settings");
    assert_eq!(
        (
            settings.endpoint.as_str(),
            settings.credentials.access_key_id()
        ),
        ("http://env:1", "EK")
    );
    let directory = tempfile::tempdir().expect("dir");
    let path = write_profile(directory.path(), PROFILE, 0o600).expect("profile written");
    let other = from_file(&path, "other").expect("other profile");
    assert_eq!(other.endpoint, "https://s3.example.com");
    assert_eq!(other.region.as_deref(), Some("eu-west-1"));
    assert_eq!(other.credentials.access_key_id(), "OK");
}

#[cfg(unix)]
#[test]
fn a_profile_others_can_read_is_refused() {
    let directory = tempfile::tempdir().expect("dir");
    let path = write_profile(directory.path(), PROFILE, 0o644).expect("profile written");
    assert!(
        matches!(from_file(&path, "default"), Err(CliError::Invalid(message)) if message.contains("chmod 600"))
    );
}

#[test]
fn errors_name_the_setting_and_never_echo_a_value() {
    let directory = tempfile::tempdir().expect("dir");
    let path = write_profile(
        directory.path(),
        "[default]\nendpoint = x\nsecret_key = do-not-print-me\n",
        0o600,
    )
    .expect("profile written");
    let message = from_file(&path, "default")
        .map(|_| ())
        .expect_err("missing key")
        .to_string();
    assert!(
        message.contains("access_key") && !message.contains("do-not-print-me"),
        "{message}"
    );
    let bad = write_profile(
        directory.path(),
        "[default]\nsecret = do-not-print-me\n",
        0o600,
    )
    .expect("profile written");
    let message = from_file(&bad, "default")
        .map(|_| ())
        .expect_err("unknown key")
        .to_string();
    assert!(!message.contains("do-not-print-me"), "{message}");
    let none = load("default", |_| None)
        .map(|_| ())
        .expect_err("nothing configured")
        .to_string();
    assert!(none.contains("TESSARIDB_S3_ENDPOINT"), "{none}");
}
