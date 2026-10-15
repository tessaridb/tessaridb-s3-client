//! What can go wrong in this crate, as one error type the caller can match on.

/// An error from the client.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A value the caller passed does not satisfy the rule named in `rule`; nothing was sent.
    #[error("invalid {what}: {rule}")]
    InvalidArgument {
        /// Which argument, e.g. "bucket name".
        what: &'static str,
        /// The rule it breaks, e.g. "3 to 63 characters".
        rule: &'static str,
    },
}

/// The result of a client call.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::Error;

    #[test]
    fn an_invalid_argument_names_the_argument_and_the_rule_it_breaks() {
        let error = Error::InvalidArgument {
            what: "bucket name",
            rule: "3 to 63 characters",
        };
        assert_eq!(error.to_string(), "invalid bucket name: 3 to 63 characters");
    }
}
