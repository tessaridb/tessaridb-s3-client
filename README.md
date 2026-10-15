<div align="center">

<img src="assets/logo/tessaridb-s3-mark-128.png" alt="" width="88" height="88">

# TessariDB S3 — Rust client

**An async Rust SDK and a command line for S3-compatible object storage, built for moving files.**

[![status](https://img.shields.io/badge/status-in%20development-D98E33?style=flat-square)](#status)
[![licence](https://img.shields.io/badge/licence-Apache--2.0-6B5FD1?style=flat-square)](LICENSE)

</div>

> [!WARNING]
> **Under active development.** Not published to crates.io. The library does bucket and object calls on bodies held
> in memory; files, multipart and the command line are still to come. See [**Status**](#status).

Two crates live here:

- **`tessaridb-s3-client`** — the library you embed in a Rust program to put, get, list and delete objects, and above
  all to move **files**: streamed from and to disk without being held in memory, large ones in parts that survive an
  interrupted transfer, and links that grant one object for a bounded time.
- **`tessaridb-s3-cli`** — a command line over that library, for the same work from a shell or a script.

They speak the S3 API with AWS Signature Version 4, so they work with
[TessariDB S3](https://github.com/tessaridb/tessaridb-s3) and with any other S3-compatible store.

Licence: **Apache-2.0**. See [`LICENSE`](LICENSE). TessariDB S3 itself — the server — is licensed separately, under the
Business Source License 1.1. This client depends on the S3 *protocol*, never on the server's code, so nothing about
the server's licence reaches a program that embeds this one.

## Status

| Part | State |
|---|---|
| Workspace, licence, checks | in place |
| Request signing (SigV4: header, presigned URL) | done in `tessaridb_s3_client::sign`, reproducing the AWS published examples |
| Buckets (create, delete, exists, list) and objects (put, get, head, delete, copy, list) | done, bodies held in memory; retried on transient failures |
| Errors | one `Error` type: a category to branch on, the S3 code, status and request ID kept |
| Files: streaming, multipart with resume | not started |
| Command line (`ls`, `cp`, `mv`, `rm`, `mb`, `rb`, `stat`, `presign`, `sync`) | not started |

## Using the library

<!-- non-executable example: the same code, inside an async function, is compiled as the crate's documentation example -->
```rust
use tessaridb_s3_client::sign::Credentials;
use tessaridb_s3_client::{BucketName, ClientBuilder, ListObjects, ObjectKey};

let client = ClientBuilder::new("https://s3.example.com", Credentials::new("ACCESS_KEY", "SECRET_KEY"))
    .region("us-east-1")
    .build()?;
let bucket = BucketName::new("photos")?;
let key = ObjectKey::new("2026/cat.jpg")?;
client.put_object(&bucket, &key, b"...".to_vec(), Some("image/jpeg")).await?;
let page = client.list_objects(&bucket, &ListObjects::default()).await?;
```

- **Names are checked when they are made.** `BucketName::new` and `ObjectKey::new` refuse what S3 would refuse — and
  a key with a `.` or `..` path segment, which S3 accepts but an HTTP URL silently collapses.
- **Buckets are addressed path-style:** `endpoint/bucket/key`.
- **TLS uses the platform's trust store**, with no switch to skip certificate checks. A plain `http://` endpoint is
  accepted for a server on the same machine or a private network: the secret key is never sent, but object data and
  metadata travel unencrypted.
- **Errors carry a category.** Branch on `error.category()` — `NotFound`, `Conflict`, `Unauthorized`, `Unavailable`
  and the rest — never on the message. A transient failure (a timeout, a lost connection, `SlowDown`, a 5xx) is retried
  up to three times with a randomised backoff; when the attempts run out you get `RetriesExhausted` with the last cause.
- **Redirects are not followed**: a signed request sent to another host would be a different request.

## Building

```sh
cargo build
cargo test --workspace --tests
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check
cargo deny check
```

Rust 1.98 or newer (edition 2024).

The live tests run against a real S3 server when these are set, and report that they were skipped otherwise:

```sh
export TESSARIDB_S3_CLIENT_TEST_ENDPOINT=http://127.0.0.1:9000
export TESSARIDB_S3_CLIENT_TEST_ACCESS_KEY=...
export TESSARIDB_S3_CLIENT_TEST_SECRET_KEY=...
cargo test --workspace --tests
```

## Licence

Copyright 2026 boogvar. Licensed under the [Apache License, Version 2.0](LICENSE).
