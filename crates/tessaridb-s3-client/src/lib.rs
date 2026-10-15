//! An async client for S3-compatible object storage, built around moving files: streaming them to and from disk
//! without holding them in memory, large ones in parts that survive an interrupted transfer, and links that grant one
//! object for a bounded time.
//!
//! The crate is being built. What works today: [`Client`] with bucket and object calls on bodies held in memory,
//! signed with SigV4, retried when the failure is transient. The repository's README tracks the rest.
//!
//! The library starts no runtime of its own: its calls are futures for the caller's tokio runtime.
//!
//! ```no_run
//! use tessaridb_s3_client::sign::Credentials;
//! use tessaridb_s3_client::{BucketName, ClientBuilder, ListObjects, ObjectKey};
//!
//! # async fn example() -> tessaridb_s3_client::Result<()> {
//! let client = ClientBuilder::new("https://s3.example.com", Credentials::new("ACCESS_KEY", "SECRET_KEY"))
//!     .region("us-east-1")
//!     .build()?;
//! let bucket = BucketName::new("photos")?;
//! let key = ObjectKey::new("2026/cat.jpg")?;
//! client.put_object(&bucket, &key, b"...".to_vec(), Some("image/jpeg")).await?;
//! let page = client.list_objects(&bucket, &ListObjects::default()).await?;
//! # Ok(())
//! # }
//! ```

mod bucket;
mod checksum;
mod client;
mod constants;
mod error;
mod object;
mod retry;
pub mod sign;
#[cfg(test)]
mod test_server;
mod transport;
mod xml;

pub use bucket::{BucketInfo, BucketName};
pub use client::{Client, ClientBuilder};
pub use error::{Error, ErrorCategory, Result, TransportFailure};
pub use object::{ListObjects, ListPage, ObjectInfo, ObjectKey};
