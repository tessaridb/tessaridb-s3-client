//! What a command can fail with, and the exit code each failure class carries: 2 the input was invalid, 3 a safety
//! check refused to go on, 4 the operation failed, 5 it ran but the result is not what was asked, 130 it was
//! interrupted. A reader closing standard output early (`| head`) ends the command quietly with 0.

use tessaridb_s3_client::Error as ClientError;

/// A failed command.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CliError {
    /// The arguments, a location or the credentials are not valid; nothing was done.
    #[error("{0}")]
    Invalid(String),
    /// A safety check refused: the destination exists, the bucket is not empty, `--yes` is missing.
    #[error("{0}")]
    Refused(String),
    /// The client or the server failed.
    #[error(transparent)]
    Client(#[from] ClientError),
    /// The operation ran but the post-check found another result.
    #[error("{0}")]
    Unconfirmed(String),
    /// A local file or directory could not be read or written.
    #[error("{what}: {source}")]
    Local {
        /// What was being done.
        what: String,
        /// Why it failed.
        source: std::io::Error,
    },
    /// Standard output could not be written.
    #[error("cannot write the output: {0}")]
    Output(String),
    /// Whoever reads standard output stopped reading (`| head`); nothing more is printed.
    #[error("the output was closed")]
    OutputClosed,
    /// Ctrl-C or SIGTERM arrived before the command finished.
    #[error("interrupted")]
    Interrupted,
}

impl CliError {
    /// The process exit code for this failure.
    pub(crate) fn exit_code(&self) -> u8 {
        match self {
            Self::Invalid(_) => 2,
            Self::Refused(_) => 3,
            Self::Client(ClientError::InvalidArgument { .. }) => 2,
            Self::Client(_) | Self::Local { .. } | Self::Output(_) => 4,
            Self::Unconfirmed(_) => 5,
            Self::OutputClosed => 0,
            Self::Interrupted => 130,
        }
    }
}

pub(crate) type Result<T> = std::result::Result<T, CliError>;

#[cfg(test)]
mod tests {
    use super::CliError;
    use tessaridb_s3_client::Error as ClientError;

    #[test]
    fn each_failure_class_has_its_own_exit_code() {
        assert_eq!(CliError::Invalid("x".into()).exit_code(), 2);
        assert_eq!(CliError::Refused("x".into()).exit_code(), 3);
        assert_eq!(CliError::Unconfirmed("x".into()).exit_code(), 5);
        assert_eq!(CliError::Interrupted.exit_code(), 130);
        assert_eq!(CliError::OutputClosed.exit_code(), 0);
        let local = CliError::Local {
            what: "read".into(),
            source: std::io::Error::other("x"),
        };
        assert_eq!(local.exit_code(), 4);
        let invalid = ClientError::InvalidArgument {
            what: "bucket name",
            rule: "3 to 63 characters",
        };
        assert_eq!(CliError::Client(invalid).exit_code(), 2);
        let refused = ClientError::Service {
            status: 403,
            code: Some("AccessDenied".into()),
            message: None,
            request_id: None,
        };
        assert_eq!(CliError::Client(refused).exit_code(), 4);
    }
}
