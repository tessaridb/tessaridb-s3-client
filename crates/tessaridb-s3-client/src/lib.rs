//! An async client for S3-compatible object storage, built around moving files: streaming them to and from disk
//! without holding them in memory, large ones in parts that survive an interrupted transfer, and links that grant one
//! object for a bounded time.
//!
//! The crate is being built; nothing here is usable yet. The repository's README tracks what works.

mod error;

pub use error::{Error, Result};
