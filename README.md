<div align="center">

<img src="assets/logo/tessaridb-s3-mark-128.png" alt="" width="88" height="88">

# TessariDB S3 — Rust client

**An async Rust SDK and a command line for S3-compatible object storage, built for moving files.**

[![status](https://img.shields.io/badge/status-in%20development-D98E33?style=flat-square)](#status)
[![licence](https://img.shields.io/badge/licence-Apache--2.0-6B5FD1?style=flat-square)](LICENSE)

</div>

> [!WARNING]
> **Under active development, and not usable yet.** Not published to crates.io; the library has no operations and
> the command line answers `--version` only. See [**Status**](#status).

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
| Request signing (SigV4: header, presigned URL) | done in `tessaridb_s3_client::sign`, reproducing the AWS published examples; no operation uses it yet |
| Objects and buckets (put, get, head, delete, copy, list) | not started |
| Files: streaming, multipart with resume | not started |
| Command line (`ls`, `cp`, `mv`, `rm`, `mb`, `rb`, `stat`, `presign`, `sync`) | not started |

## Building

```sh
cargo build
cargo test --workspace --tests
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check
cargo deny check
```

Rust 1.98 or newer (edition 2024).

## Licence

Copyright 2026 boogvar. Licensed under the [Apache License, Version 2.0](LICENSE).
