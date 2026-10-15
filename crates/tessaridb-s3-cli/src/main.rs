//! `tessaridb-s3-cli` — the command line over `tessaridb-s3-client`. This file only composes; the commands live in
//! the library and in this crate's modules as they arrive.

use std::process::ExitCode;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("--version" | "-V") => {
            println!("tessaridb-s3-cli {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("tessaridb-s3-cli: no commands yet — this build only answers --version");
            ExitCode::from(2)
        }
    }
}
