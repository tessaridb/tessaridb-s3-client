//! The binary as a user runs it. The offline tests need nothing; the live test runs against the server named by
//! `TESSARIDB_S3_CLIENT_TEST_ENDPOINT`, `_ACCESS_KEY` and `_SECRET_KEY`, and is skipped without them.

use std::path::Path;
use std::process::{Command, Output};

/// Runs the binary with only the variables in `env`, from `directory`, with `HOME` pointing there too.
fn run(directory: &Path, env: &[(&str, &str)], args: &[&str]) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_tessaridb-s3-cli"))
        .env_clear()
        .env("HOME", directory)
        .envs(env.iter().copied())
        .current_dir(directory)
        .args(args)
        .output()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn without_credentials_a_command_exits_2_and_says_what_to_set() {
    let home = tempfile::tempdir().expect("home");
    let output = run(home.path(), &[], &["ls"]).expect("ran");
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("TESSARIDB_S3_ENDPOINT"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_mistyped_command_exits_2_before_reading_credentials() {
    let home = tempfile::tempdir().expect("home");
    let cases: [(&[&str], &str); 3] = [
        (&["stat", "s3://Bad_Bucket/key"], "invalid bucket name"),
        (&["rm", "s3://photos/key"], "add --yes"),
        (
            &["presign", "s3://photos/key", "--expires", "0"],
            "a link lives",
        ),
    ];
    for (args, expected) in cases {
        let output = run(home.path(), &[], args).expect("ran");
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains(expected),
            "{args:?}: {}",
            stderr(&output)
        );
        assert!(
            !stderr(&output).contains("no credentials"),
            "{args:?}: {}",
            stderr(&output)
        );
    }
    let usage = run(home.path(), &[], &["frobnicate"]).expect("ran");
    assert_eq!(usage.status.code(), Some(2), "{}", stderr(&usage));
}

/// The live server's settings as the CLI reads them, when the test variables are set.
fn live() -> Option<Vec<(&'static str, String)>> {
    let endpoint = std::env::var("TESSARIDB_S3_CLIENT_TEST_ENDPOINT").ok()?;
    let access = std::env::var("TESSARIDB_S3_CLIENT_TEST_ACCESS_KEY").ok()?;
    let secret = std::env::var("TESSARIDB_S3_CLIENT_TEST_SECRET_KEY").ok()?;
    Some(vec![
        ("TESSARIDB_S3_ENDPOINT", endpoint),
        ("TESSARIDB_S3_ACCESS_KEY", access),
        ("TESSARIDB_S3_SECRET_KEY", secret),
    ])
}

/// Bytes that do not repeat, so a misplaced range cannot pass for the right one.
fn noise(length: usize, seed: u64) -> Vec<u8> {
    let mut state = seed | 1;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_le_bytes()[0]
        })
        .collect()
}

#[test]
fn every_command_against_a_live_server() {
    let Some(settings) = live() else {
        eprintln!("skipped: TESSARIDB_S3_CLIENT_TEST_ENDPOINT is not set");
        return;
    };
    let env: Vec<(&str, &str)> = settings
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let work = tempfile::tempdir().expect("work dir");
    let dir = work.path();
    let cli = |args: &[&str]| run(dir, &env, args).expect("ran");
    let ok = |args: &[&str]| {
        let output = cli(args);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{args:?}: {}{}",
            stdout(&output),
            stderr(&output)
        );
        output
    };
    let bucket = format!(
        "cli-live-{:016x}",
        u64::from_le_bytes(
            noise(8, std::process::id().into())
                .try_into()
                .expect("8 bytes")
        )
    );
    let root = format!("s3://{bucket}");

    // mb, and ls of buckets shows it.
    assert!(stdout(&ok(&["mb", &root])).contains("created"));
    assert!(stdout(&ok(&["ls"])).contains(&root));

    // cp up; a second cp onto it is refused without --yes, accepted with it.
    let small = noise(70_001, 7);
    std::fs::write(dir.join("small.bin"), &small).expect("small file");
    ok(&["cp", "small.bin", &format!("{root}/in/")]);
    let again = cli(&["cp", "small.bin", &format!("{root}/in/small.bin")]);
    assert_eq!(again.status.code(), Some(3), "{}", stderr(&again));
    assert!(stderr(&again).contains("--yes"), "{}", stderr(&again));
    ok(&["cp", "small.bin", &format!("{root}/in/small.bin"), "--yes"]);

    // stat --json reports the size; ls lists one level, --recursive everything.
    let stat: serde_json::Value = serde_json::from_str(&stdout(&ok(&[
        "stat",
        &format!("{root}/in/small.bin"),
        "--json",
    ])))
    .expect("json");
    assert_eq!(stat["size"], 70_001);
    assert!(stat["crc64nvme"].is_string());
    let level = stdout(&ok(&["ls", &format!("{root}/")]));
    assert!(
        level.contains("PRE") && level.contains("in/") && !level.contains("small.bin"),
        "{level}"
    );
    assert!(stdout(&ok(&["ls", &root, "--recursive"])).contains("in/small.bin"));

    // presign prints a link to the object on this endpoint.
    let link = stdout(&ok(&[
        "presign",
        &format!("{root}/in/small.bin"),
        "--expires",
        "60",
    ]));
    assert!(
        link.contains(&format!("/{bucket}/in/small.bin?")) && link.contains("X-Amz-Signature="),
        "{link}"
    );

    // cp down into a directory keeps the name and the bytes; cp on the server copies.
    std::fs::create_dir(dir.join("down")).expect("down dir");
    ok(&["cp", &format!("{root}/in/small.bin"), "down/"]);
    assert_eq!(
        std::fs::read(dir.join("down/small.bin")).expect("downloaded"),
        small
    );
    ok(&[
        "cp",
        &format!("{root}/in/small.bin"),
        &format!("{root}/copy/small.bin"),
    ]);

    // mv takes the object away after the copy is checked.
    ok(&[
        "mv",
        &format!("{root}/copy/small.bin"),
        "moved.bin",
        "--yes",
    ]);
    assert_eq!(std::fs::read(dir.join("moved.bin")).expect("moved"), small);
    let gone = cli(&["stat", &format!("{root}/copy/small.bin")]);
    assert_eq!(gone.status.code(), Some(4), "{}", stderr(&gone));

    // sync up a tree, then again: nothing moves; a same-size edit moves exactly that file — once through the MD5 of a
    // single-part ETag, once through the CRC64NVME of a multipart object.
    std::fs::create_dir_all(dir.join("tree/sub")).expect("tree");
    std::fs::write(dir.join("tree/a.txt"), b"alpha").expect("a");
    let mut large = noise(17 << 20, 11);
    std::fs::write(dir.join("tree/sub/large.bin"), &large).expect("large");
    let target = format!("{root}/backup");
    assert!(stdout(&ok(&["sync", "tree", &target])).contains("sent 2, unchanged 0"));
    assert!(stdout(&ok(&["sync", "tree", &target])).contains("sent 0, unchanged 2"));
    std::fs::write(dir.join("tree/a.txt"), b"alphA").expect("a edited");
    large[1_000] ^= 0xff;
    std::fs::write(dir.join("tree/sub/large.bin"), &large).expect("large edited");
    let dry = stdout(&ok(&["sync", "tree", &target, "--dry-run"]));
    assert!(dry.contains("would send 2"), "{dry}");
    assert!(stdout(&ok(&["sync", "tree", &target])).contains("sent 2, unchanged 0"));
    assert!(stdout(&ok(&["sync", "tree", &target])).contains("sent 0, unchanged 2"));

    // sync down rebuilds the tree.
    assert!(stdout(&ok(&["sync", &target, "restored"])).contains("sent 2"));
    assert_eq!(
        std::fs::read(dir.join("restored/a.txt")).expect("a restored"),
        b"alphA"
    );
    assert_eq!(
        std::fs::read(dir.join("restored/sub/large.bin")).expect("large restored"),
        large
    );

    // rb refuses a bucket with objects; rm needs --yes, --dry-run deletes nothing, --recursive empties it.
    let full = cli(&["rb", &root, "--yes"]);
    assert_eq!(full.status.code(), Some(3), "{}", stderr(&full));
    assert_eq!(
        cli(&["rm", &format!("{root}/in/small.bin")]).status.code(),
        Some(2)
    );
    ok(&["rm", &format!("{root}/in/small.bin"), "--dry-run"]);
    ok(&["stat", &format!("{root}/in/small.bin")]);
    let one = ok(&["rm", &format!("{root}/in/small.bin"), "--yes"]);
    assert!(stderr(&one).contains("deleted"), "{}", stderr(&one));
    let missing = cli(&["rm", &format!("{root}/in/small.bin"), "--yes"]);
    assert_eq!(missing.status.code(), Some(3), "{}", stderr(&missing));
    assert!(stdout(&ok(&["rm", &root, "--recursive", "--yes"])).contains("deleted 2 objects"));
    ok(&["rb", &root, "--yes"]);
    assert!(!stdout(&ok(&["ls"])).contains(&root));
}
