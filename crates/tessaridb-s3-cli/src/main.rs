//! `tessaridb-s3-cli` — the command line over `tessaridb-s3-client`. This file only composes: it parses the command,
//! checks it, reads the settings, builds the client and runs the command until it ends or Ctrl-C / SIGTERM arrives.

mod args;
mod commands;
mod config;
mod error;
mod location;
mod output;

use std::process::ExitCode;

use clap::Parser as _;
use tessaridb_s3_client::ClientBuilder;
use tracing_subscriber::EnvFilter;

use crate::error::{CliError, Result};
use crate::output::printable;

/// The variable that sets the log filter (`warn` when unset); logs go to standard error.
const LOG_VARIABLE: &str = "TESSARIDB_S3_LOG";

fn main() -> ExitCode {
    let cli = args::Cli::parse();
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_env(LOG_VARIABLE).unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();
    let outcome = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime.block_on(run(cli)),
        Err(source) => Err(CliError::Local {
            what: "start the runtime".to_owned(),
            source,
        }),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            report(&error);
            ExitCode::from(error.exit_code())
        }
    }
}

async fn run(cli: args::Cli) -> Result<()> {
    commands::validate(&cli.command)?;
    let settings = config::load(&cli.profile, |name| std::env::var(name).ok())?;
    let mut builder = ClientBuilder::new(&settings.endpoint, settings.credentials);
    if let Some(region) = &settings.region {
        builder = builder.region(region);
    }
    let client = builder.build()?;
    tokio::select! {
        biased;
        () = interrupted() => Err(CliError::Interrupted),
        result = commands::run(cli.command, &client) => result,
    }
}

/// Completes on Ctrl-C, or SIGTERM on Unix; never when neither can be listened for.
async fn interrupted() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut terminate) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }
            return;
        }
    }
    if tokio::signal::ctrl_c().await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// The error and its causes on standard error, control characters escaped: they may carry the server's words.
fn report(error: &CliError) {
    if matches!(error, CliError::OutputClosed) {
        return;
    }
    eprintln!("tessaridb-s3-cli: [!] {}", printable(&error.to_string()));
    let mut cause = std::error::Error::source(error);
    while let Some(inner) = cause {
        eprintln!("  caused by: {}", printable(&inner.to_string()));
        cause = inner.source();
    }
}
