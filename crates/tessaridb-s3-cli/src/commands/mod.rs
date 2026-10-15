//! The operations. `validate` checks everything that needs no server — locations, `--yes`, the link lifetime — so a
//! mistyped command fails before any credential is read or any request is sent; `run` then does the work.

mod buckets;
mod info;
mod list;
mod remove;
mod sync;
mod transfer;

use tessaridb_s3_client::{BucketName, Client, ErrorCategory, ObjectInfo, ObjectKey};

use crate::args::Command;
use crate::error::{CliError, Result};
use crate::location::Location;

/// Transfers or deletions in flight at once, for commands that act on many objects.
const CONCURRENCY: usize = 4;

/// The longest a presigned link may live: seven days, the S3 limit.
const PRESIGN_MAX_SECS: u64 = 604_800;

/// Refuses, before anything is read or sent, a command that cannot be right whatever the server holds.
pub(crate) fn validate(command: &Command) -> Result<()> {
    match command {
        Command::Ls { location, .. } => {
            if let Some(location) = location {
                remote(location)?;
            }
        }
        Command::Mb { location, .. } => {
            remote(location)?.bucket()?;
        }
        Command::Rb { location, safety } => {
            remote(location)?.bucket()?;
            confirmed(safety.yes, safety.dry_run, "rb")?;
        }
        Command::Cp {
            source,
            destination,
            ..
        } => {
            transfer::endpoints(&Location::parse(source)?, &Location::parse(destination)?)?;
        }
        Command::Mv {
            source,
            destination,
            safety,
        } => {
            transfer::endpoints(&Location::parse(source)?, &Location::parse(destination)?)?;
            confirmed(safety.yes, safety.dry_run, "mv")?;
        }
        Command::Rm {
            location,
            recursive,
            safety,
        } => {
            let location = remote(location)?;
            if !recursive {
                location.object()?;
            }
            confirmed(safety.yes, safety.dry_run, "rm")?;
        }
        Command::Stat { location, .. } => {
            remote(location)?.object()?;
        }
        Command::Presign {
            location, expires, ..
        } => {
            remote(location)?.object()?;
            if !(1..=PRESIGN_MAX_SECS).contains(expires) {
                return Err(CliError::Invalid(format!(
                    "--expires {expires}: a link lives 1 to {PRESIGN_MAX_SECS} seconds"
                )));
            }
        }
        Command::Sync {
            source,
            destination,
            ..
        } => {
            sync::endpoints(&Location::parse(source)?, &Location::parse(destination)?)?;
        }
    }
    Ok(())
}

/// Does what `command` asks against the server `client` talks to.
pub(crate) async fn run(command: Command, client: &Client) -> Result<()> {
    match command {
        Command::Ls {
            location,
            recursive,
            json,
        } => match location {
            None => list::buckets(client, json).await,
            Some(location) => list::objects(client, &remote(&location)?, recursive, json).await,
        },
        Command::Mb { location, safety } => {
            buckets::make(client, &remote(&location)?.bucket()?, safety.dry_run).await
        }
        Command::Rb { location, safety } => {
            buckets::remove(client, &remote(&location)?.bucket()?, safety.dry_run).await
        }
        Command::Cp {
            source,
            destination,
            safety,
        } => {
            let endpoints =
                transfer::endpoints(&Location::parse(&source)?, &Location::parse(&destination)?)?;
            transfer::copy(client, &endpoints, safety.yes, safety.dry_run).await
        }
        Command::Mv {
            source,
            destination,
            safety,
        } => {
            let endpoints =
                transfer::endpoints(&Location::parse(&source)?, &Location::parse(&destination)?)?;
            transfer::r#move(client, endpoints, safety.dry_run).await
        }
        Command::Rm {
            location,
            recursive,
            safety,
        } => {
            let location = remote(&location)?;
            if recursive {
                remove::under_prefix(client, &location, safety.dry_run).await
            } else {
                let (bucket, key) = location.object()?;
                remove::object(client, &bucket, &key, safety.dry_run).await
            }
        }
        Command::Stat { location, json } => {
            let (bucket, key) = remote(&location)?.object()?;
            info::stat(client, &bucket, &key, json).await
        }
        Command::Presign {
            location,
            put,
            expires,
        } => {
            let (bucket, key) = remote(&location)?.object()?;
            info::presign(
                client,
                &bucket,
                &key,
                put,
                std::time::Duration::from_secs(expires),
            )
        }
        Command::Sync {
            source,
            destination,
            safety,
        } => {
            let endpoints =
                sync::endpoints(&Location::parse(&source)?, &Location::parse(&destination)?)?;
            sync::sync(client, endpoints, safety.dry_run).await
        }
    }
}

/// `text` as an `s3://` location; a local path is refused.
fn remote(text: &str) -> Result<Location> {
    let location = Location::parse(text)?;
    if let Location::Local(path) = &location {
        return Err(CliError::Invalid(format!(
            "{} is not an s3:// location",
            path.display()
        )));
    }
    Ok(location)
}

/// A destructive command runs only with `--yes`; `--dry-run` needs no confirmation because it deletes nothing.
fn confirmed(yes: bool, dry_run: bool, command: &str) -> Result<()> {
    if yes || dry_run {
        Ok(())
    } else {
        Err(CliError::Invalid(format!(
            "`{command}` deletes data: add --yes to do it, or --dry-run to see what it would do"
        )))
    }
}

/// The object's metadata, or `None` when there is no such object.
async fn head(client: &Client, bucket: &BucketName, key: &ObjectKey) -> Result<Option<ObjectInfo>> {
    match client.head_object(bucket, key).await {
        Ok(info) => Ok(Some(info)),
        Err(error) if error.category() == ErrorCategory::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// `s3://bucket/key` with the key's control characters escaped.
fn shown(bucket: &BucketName, key: &str) -> String {
    crate::output::printable(&format!("s3://{bucket}/{key}"))
}

#[cfg(test)]
mod tests;
