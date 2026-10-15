//! The client: one endpoint, one region, one set of credentials, shared cheaply between tasks. Every call is one S3
//! operation, retried by the policy when its failure is transient and the operation is idempotent — which every
//! operation here is.

mod buckets;
mod objects;

use std::sync::Arc;

use crate::Result;
use crate::retry::RetryPolicy;
use crate::sign::{Credentials, Signer};
use crate::transport::{Request, Response, Transport};

/// The region a client signs for unless told otherwise.
const DEFAULT_REGION: &str = "us-east-1";

/// Builds a [`Client`].
#[derive(Debug)]
pub struct ClientBuilder {
    endpoint: String,
    region: String,
    credentials: Credentials,
}

impl ClientBuilder {
    /// A client of the server at `endpoint` — `http://host:port` or `https://host`, with nothing after the
    /// authority — signing with `credentials`. Buckets are addressed path-style (`endpoint/bucket/key`).
    #[must_use]
    pub fn new(endpoint: &str, credentials: Credentials) -> Self {
        Self {
            endpoint: endpoint.to_owned(),
            region: DEFAULT_REGION.to_owned(),
            credentials,
        }
    }

    /// The region requests are signed for; `us-east-1` by default.
    #[must_use]
    pub fn region(mut self, region: &str) -> Self {
        region.clone_into(&mut self.region);
        self
    }

    /// The client.
    ///
    /// # Errors
    /// [`crate::Error::InvalidArgument`] for an endpoint that is not an `http`/`https` URL of a host, or a region that
    /// is not lowercase letters, digits and `-`;
    /// [`crate::Error::Setup`] when TLS cannot use the platform's trust store.
    pub fn build(self) -> Result<Client> {
        let valid_region = (1..=64).contains(&self.region.len())
            && self
                .region
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if !valid_region {
            return Err(crate::Error::InvalidArgument {
                what: "region",
                rule: "1 to 64 lowercase letters, digits and `-`",
            });
        }
        let signer = Signer::new(self.credentials, &self.region);
        Ok(Client {
            transport: Arc::new(Transport::new(&self.endpoint, signer)?),
            region: self.region,
            retry: RetryPolicy::default(),
        })
    }
}

/// A client of one S3 endpoint. Cloning it is cheap and shares the connection pool.
#[derive(Debug, Clone)]
pub struct Client {
    transport: Arc<Transport>,
    region: String,
    retry: RetryPolicy,
}

impl Client {
    /// Sends `request`, retrying transient failures by the policy.
    async fn call(&self, request: &Request<'_>) -> Result<Response> {
        self.retry.run(|_| self.transport.send(request)).await
    }
}
