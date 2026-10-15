//! What a command prints. Keys and messages come from the server, so they are printed with every control character
//! escaped: an object key holding terminal escape sequences cannot move the cursor, recolour or retitle the
//! terminal of whoever lists it. JSON lines go through `serde_json`, which escapes them by itself.

use std::io::Write as _;

use serde::Serialize;

use crate::error::{CliError, Result};

/// `text` with every control character written as `\u{…}`.
pub(crate) fn printable(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                character.escape_unicode().to_string()
            } else {
                character.to_string()
            }
        })
        .collect()
}

/// Writes `text` and a newline to standard output.
pub(crate) fn line(text: &str) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{text}").map_err(closed)
}

/// Writes `value` as one line of JSON to standard output.
pub(crate) fn json(value: &impl Serialize) -> Result<()> {
    let text = serde_json::to_string(value).map_err(|error| CliError::Output(error.to_string()))?;
    line(&text)
}

/// Writes `text` to standard error: what a command did to data, so it is on record even when stdout is piped.
pub(crate) fn notice(text: &str) {
    let mut stderr = std::io::stderr().lock();
    // Standard error going away is not a reason to stop deleting what the user asked to delete.
    drop(writeln!(stderr, "{text}"));
}

fn closed(error: std::io::Error) -> CliError {
    if error.kind() == std::io::ErrorKind::BrokenPipe {
        CliError::OutputClosed
    } else {
        CliError::Output(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::printable;

    #[test]
    fn control_characters_from_the_server_cannot_reach_the_terminal() {
        assert_eq!(printable("ok/é.txt"), "ok/é.txt");
        assert_eq!(printable("a\u{1b}[2Jb\u{7}\n"), "a\\u{1b}[2Jb\\u{7}\\u{a}");
    }
}
