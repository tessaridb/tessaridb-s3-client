use clap::Parser as _;

use super::transfer::{Endpoints, endpoints};
use super::validate;
use crate::args::Cli;
use crate::error::CliError;
use crate::location::Location;

fn check(line: &[&str]) -> Result<(), CliError> {
    let cli = Cli::try_parse_from(std::iter::once("tessaridb-s3-cli").chain(line.iter().copied()))
        .map_err(|error| CliError::Invalid(error.to_string()))?;
    validate(&cli.command)
}

#[test]
fn a_destructive_command_needs_yes_unless_it_is_a_dry_run() {
    for line in [
        &["rm", "s3://bkt/k"][..],
        &["rb", "s3://bkt"],
        &["mv", "s3://bkt/k", "s3://bkt/l"],
    ] {
        assert!(
            matches!(check(line), Err(CliError::Invalid(message)) if message.contains("--yes")),
            "{line:?}"
        );
        let confirmed = [line, &["--yes"]].concat();
        assert!(check(&confirmed).is_ok(), "{confirmed:?}");
        let dry = [line, &["--dry-run"]].concat();
        assert!(check(&dry).is_ok(), "{dry:?}");
    }
}

#[test]
fn commands_refuse_the_wrong_kind_of_location_before_any_request() {
    let refused: [(&[&str], &str); 9] = [
        (
            &["rm", "s3://bkt", "--yes"],
            "names a bucket, not an object",
        ),
        (&["stat", "s3://bkt"], "names a bucket, not an object"),
        (&["mb", "s3://bkt/k"], "names an object, not a bucket"),
        (&["ls", "./here"], "is not an s3:// location"),
        (&["cp", "a.txt", "b.txt"], "both locations are local"),
        (
            &["sync", "s3://aaa/x", "s3://bkt/y"],
            "one local directory and one s3:// location",
        ),
        (
            &["presign", "s3://bkt/k", "--expires", "0"],
            "a link lives 1 to 604800 seconds",
        ),
        (
            &["presign", "s3://bkt/k", "--expires", "604801"],
            "a link lives 1 to 604800 seconds",
        ),
        (&["stat", "s3://Bad_Bucket/k"], "invalid bucket name"),
    ];
    for (line, expected) in refused {
        let outcome = check(line);
        assert!(
            matches!(&outcome, Err(CliError::Invalid(message)) if message.contains(expected)),
            "{line:?}: {outcome:?}"
        );
    }
    assert!(check(&["rm", "s3://bkt", "--recursive", "--yes"]).is_ok());
    assert!(check(&["presign", "s3://bkt/k", "--expires", "604800"]).is_ok());
}

#[test]
fn a_destination_without_a_name_takes_the_source_name() {
    let parse = |text: &str| Location::parse(text).map_err(|error| error.to_string());
    let upload = |source: &str, destination: &str| -> Result<String, String> {
        match endpoints(&parse(source)?, &parse(destination)?).map_err(|error| error.to_string())? {
            Endpoints::Upload { key, .. } | Endpoints::Copy { key, .. } => {
                Ok(key.as_str().to_owned())
            }
            Endpoints::Download { path, .. } => Ok(path.display().to_string()),
        }
    };
    assert_eq!(
        upload("dir/report.pdf", "s3://bkt"),
        Ok("report.pdf".to_owned())
    );
    assert_eq!(
        upload("dir/report.pdf", "s3://bkt/2026/"),
        Ok("2026/report.pdf".to_owned())
    );
    assert_eq!(
        upload("dir/report.pdf", "s3://bkt/renamed.pdf"),
        Ok("renamed.pdf".to_owned())
    );
    assert_eq!(
        upload("s3://bkt/2026/report.pdf", "s3://cat/"),
        Ok("report.pdf".to_owned())
    );
    assert_eq!(
        upload("s3://bkt/2026/report.pdf", "out/"),
        Ok("out/report.pdf".to_owned())
    );
    assert_eq!(
        upload("s3://bkt/2026/report.pdf", "out/copy.pdf"),
        Ok("out/copy.pdf".to_owned())
    );
    assert!(upload("s3://bkt/2026/", "out/").is_err());
}
