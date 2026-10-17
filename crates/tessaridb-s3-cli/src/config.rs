//! Where the endpoint and credentials come from: the environment (`TESSARIDB_S3_ENDPOINT`, `_ACCESS_KEY`,
//! `_SECRET_KEY`, `_REGION`, and `_SESSION_TOKEN` for temporary credentials) or, when it names none, a profile file —
//! `$XDG_CONFIG_HOME/tessaridb-s3/credentials` (`~/.config/…` without it) — readable by its owner alone. There is no command-line flag for a secret: an argument
//! is visible to every process on the machine and stays in shell history. Errors name the setting, never its value.

use std::path::{Path, PathBuf};

use tessaridb_s3_client::sign::Credentials;
use zeroize::Zeroizing;

use crate::error::{CliError, Result};

/// What a command connects with.
#[derive(Debug)]
pub(crate) struct Settings {
    pub(crate) endpoint: String,
    pub(crate) region: Option<String>,
    pub(crate) credentials: Credentials,
}

/// The settings from `variable` (an environment lookup) or, failing that, from `profile` in the profile file.
pub(crate) fn load(profile: &str, variable: impl Fn(&str) -> Option<String>) -> Result<Settings> {
    let endpoint = variable("TESSARIDB_S3_ENDPOINT");
    let access = variable("TESSARIDB_S3_ACCESS_KEY");
    let secret = variable("TESSARIDB_S3_SECRET_KEY").map(Zeroizing::new);
    if let (Some(endpoint), Some(access), Some(secret)) = (&endpoint, &access, &secret) {
        let credentials = Credentials::new(access, secret);
        let token = variable("TESSARIDB_S3_SESSION_TOKEN").map(Zeroizing::new);
        return Ok(Settings {
            endpoint: endpoint.clone(),
            region: variable("TESSARIDB_S3_REGION"),
            credentials: match &token {
                Some(token) => credentials.with_session_token(token),
                None => credentials,
            },
        });
    }
    let path = profile_path(&variable).ok_or_else(|| {
        CliError::Invalid("no credentials: set TESSARIDB_S3_ENDPOINT, _ACCESS_KEY and _SECRET_KEY, or a profile file".to_owned())
    })?;
    from_file(&path, profile)
}

fn profile_path(variable: &impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let base = variable("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| variable("HOME").map(|home| Path::new(&home).join(".config")))?;
    Some(base.join("tessaridb-s3").join("credentials"))
}

/// Reads `profile` from the file at `path`.
pub(crate) fn from_file(path: &Path, profile: &str) -> Result<Settings> {
    let shown = path.display();
    let metadata = std::fs::metadata(path).map_err(|_| {
        CliError::Invalid(format!(
            "no credentials: set TESSARIDB_S3_ENDPOINT, _ACCESS_KEY and _SECRET_KEY, or write a profile file at {shown}"
        ))
    })?;
    owner_only(&metadata, path)?;
    let text = Zeroizing::new(
        std::fs::read_to_string(path)
            .map_err(|_| CliError::Invalid(format!("the profile file {shown} cannot be read")))?,
    );
    let section = format!("[{profile}]");
    let mut inside = false;
    let (mut endpoint, mut access, mut secret, mut region) = (None, None, None, None);
    let mut token: Option<Zeroizing<String>> = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line == section;
            continue;
        }
        if !inside || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            return Err(CliError::Invalid(format!(
                "the profile file {shown} has a line without `=` in [{profile}]"
            )));
        };
        let value = value.trim().to_owned();
        match name.trim() {
            "endpoint" => endpoint = Some(value),
            "access_key" => access = Some(value),
            "secret_key" => secret = Some(Zeroizing::new(value)),
            "region" => region = Some(value),
            "session_token" => token = Some(Zeroizing::new(value)),
            other => {
                return Err(CliError::Invalid(format!(
                    "the profile file {shown} has an unknown setting `{other}`"
                )));
            }
        }
    }
    let missing =
        |name: &str| CliError::Invalid(format!("profile [{profile}] in {shown} has no `{name}`"));
    let endpoint = endpoint.ok_or_else(|| missing("endpoint"))?;
    let access = access.ok_or_else(|| missing("access_key"))?;
    let secret = secret.ok_or_else(|| missing("secret_key"))?;
    let credentials = Credentials::new(&access, &secret);
    Ok(Settings {
        endpoint,
        region,
        credentials: match &token {
            Some(token) => credentials.with_session_token(token),
            None => credentials,
        },
    })
}

#[cfg(unix)]
fn owner_only(metadata: &std::fs::Metadata, path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(CliError::Invalid(format!(
            "the profile file {} can be read by others; make it owner-only (chmod 600)",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn owner_only(_metadata: &std::fs::Metadata, _path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
