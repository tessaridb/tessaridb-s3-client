//! The command line: one subcommand per operation, each with only the flags it acts on. There is deliberately no flag
//! for a secret (see `config`).

use clap::{Args, Parser, Subcommand};

/// Move files to and from an S3-compatible object store.
#[derive(Debug, Parser)]
#[command(name = "tessaridb-s3-cli", version, about)]
pub(crate) struct Cli {
    /// The profile to read from the profile file when the environment names no endpoint.
    #[arg(long, global = true, default_value = "default")]
    pub(crate) profile: String,
    /// What to do.
    #[command(subcommand)]
    pub(crate) command: Command,
}

/// The operations.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// List buckets, or the objects under `s3://bucket[/prefix]`.
    Ls {
        /// `s3://bucket[/prefix]`; without it, the buckets.
        location: Option<String>,
        /// Every key under the prefix, instead of one level.
        #[arg(long)]
        recursive: bool,
        /// One JSON object per line.
        #[arg(long)]
        json: bool,
    },
    /// Create a bucket.
    Mb {
        /// `s3://bucket`.
        location: String,
        #[command(flatten)]
        safety: DryRun,
    },
    /// Delete an empty bucket.
    Rb {
        /// `s3://bucket`.
        location: String,
        #[command(flatten)]
        safety: Confirm,
    },
    /// Copy a file or an object: local → `s3://`, `s3://` → local, or `s3://` → `s3://`.
    Cp {
        /// Where from.
        source: String,
        /// Where to; ending in `/` (or naming a directory) keeps the source's name.
        destination: String,
        #[command(flatten)]
        safety: Overwrite,
    },
    /// Move a file or an object: copy it, check the copy, then delete the source.
    Mv {
        /// Where from.
        source: String,
        /// Where to.
        destination: String,
        #[command(flatten)]
        safety: Confirm,
    },
    /// Delete an object, or with `--recursive` every object under a prefix.
    Rm {
        /// `s3://bucket/key`, or `s3://bucket[/prefix]` with `--recursive`.
        location: String,
        /// Everything under the prefix.
        #[arg(long)]
        recursive: bool,
        #[command(flatten)]
        safety: Confirm,
    },
    /// Show an object's size, ETag, checksum and modification time.
    Stat {
        /// `s3://bucket/key`.
        location: String,
        /// One JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Print a link that lets anyone holding it download (or with `--put`, upload) one object.
    Presign {
        /// `s3://bucket/key`.
        location: String,
        /// A link for uploading instead of downloading.
        #[arg(long)]
        put: bool,
        /// How long the link works, in seconds (1 to 604800).
        #[arg(long, default_value_t = 3600)]
        expires: u64,
    },
    /// Make a destination hold what the source holds: a directory → `s3://bucket/prefix`, or back. Never deletes.
    Sync {
        /// Where from.
        source: String,
        /// Where to.
        destination: String,
        #[command(flatten)]
        safety: DryRun,
    },
}

/// A mutating command that can show what it would do.
#[derive(Debug, Clone, Copy, Args)]
pub(crate) struct DryRun {
    /// Show what would change, change nothing.
    #[arg(long)]
    pub(crate) dry_run: bool,
}

/// A destructive command: it needs `--yes`.
#[derive(Debug, Clone, Copy, Args)]
pub(crate) struct Confirm {
    /// Do it: the command deletes data.
    #[arg(long)]
    pub(crate) yes: bool,
    /// Show what would be deleted, delete nothing.
    #[arg(long)]
    pub(crate) dry_run: bool,
}

/// A copy, which needs `--yes` only to replace what is already there.
#[derive(Debug, Clone, Copy, Args)]
pub(crate) struct Overwrite {
    /// Replace the destination if it exists.
    #[arg(long)]
    pub(crate) yes: bool,
    /// Show what would change, change nothing.
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    use super::Cli;

    #[test]
    fn the_command_line_definition_is_consistent() {
        Cli::command().debug_assert();
    }
}
