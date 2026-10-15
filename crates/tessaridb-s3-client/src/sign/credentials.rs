//! An access key and its secret. The secret is wiped from memory when the credentials are dropped and never appears
//! in `Debug` output, so a logged client or a panic message does not carry it.

use zeroize::{Zeroize, ZeroizeOnDrop};

/// An access key ID and its secret access key.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Credentials {
    access_key_id: String,
    secret_access_key: String,
}

impl Credentials {
    /// Credentials for `access_key_id` with `secret_access_key`.
    #[must_use]
    pub fn new(access_key_id: &str, secret_access_key: &str) -> Self {
        Self {
            access_key_id: access_key_id.to_owned(),
            secret_access_key: secret_access_key.to_owned(),
        }
    }

    /// The access key ID, which is not secret.
    #[must_use]
    pub fn access_key_id(&self) -> &str {
        &self.access_key_id
    }

    /// Runs `use_secret` with the secret, so it is borrowed for one computation and never handed out.
    pub(crate) fn with_secret<T>(&self, use_secret: impl FnOnce(&str) -> T) -> T {
        use_secret(&self.secret_access_key)
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &"<redacted>")
            .finish()
    }
}
