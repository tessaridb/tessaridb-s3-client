<div align="center">

<img src="assets/logo/tessaridb-s3-mark-128.png" alt="" width="88" height="88">

# TessariDB S3 — Rust client

**An async Rust SDK and a command line for S3-compatible object storage, built for moving files.**

[![status](https://img.shields.io/badge/status-in%20development-D98E33?style=flat-square)](#status)
[![licence](https://img.shields.io/badge/licence-Apache--2.0-6B5FD1?style=flat-square)](LICENSE)

</div>

> [!NOTE]
> Published on [crates.io](https://crates.io/crates/tessaridb-s3-client) as `tessaridb-s3-client`. See
> [**Status**](#status).

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

Versions follow semantic versioning; before 1.0, a minor release (0.x → 0.y) may change the interface, and the
CHANGELOG says how.

| Part | State |
|---|---|
| Workspace, licence, checks | in place |
| Request signing (SigV4: header, presigned URL) | done in `tessaridb_s3_client::sign`, reproducing the AWS published examples; temporary credentials with `Credentials::with_session_token` |
| Buckets (create, delete, exists, list) and objects (put, get, head, delete, copy, list) | done, bodies held in memory; retried on transient failures |
| Errors | one `Error` type: a category to branch on, the S3 code, status and request ID kept |
| Files: upload and download streamed from and to disk | done; files over 16 MiB go up in parts (8 MiB or larger, four at a time) |
| Multipart: resume by upload ID, abort | done |
| Checksums | done: uploads carry their CRC64NVME; downloads are checked against every whole-object checksum the server keeps (CRC32, CRC32C, CRC64NVME, SHA-1, SHA-256) and retried on a mismatch |
| Presigned links (`presign_get`, `presign_put`) | done |
| Download resume (ranged retry) | done |
| Command line (`ls`, `cp`, `mv`, `rm`, `mb`, `rb`, `stat`, `presign`, `sync`) | done: `tessaridb-s3-cli` |

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
- **Files stream.** `upload_file` reads the file once to hash it, then sends it in 1 MiB chunks, signed with that
  hash — the server stores it only if every byte arrived as hashed. `download_file` writes into a hidden `.part`
  file beside the target and renames it into place only when the whole object arrived; on any failure, including a
  cancelled future, the target is left as it was. A transfer fails when nothing arrives for 60 seconds, however long
  the whole transfer takes.
- **Large files go in parts.** A file over 16 MiB is read once to plan its parts (8 MiB, larger when 10 000 parts
  would not hold it) and the whole file's CRC64NVME, then sent four parts at a time, each signed with its own hash.
  If parts keep failing on a transient error, `upload_file` returns `UploadInterrupted` carrying the upload ID and
  leaves the upload open: `resume_upload` with that ID sends only the parts the server does not already hold intact,
  and `abort_upload` discards it. A permanent failure aborts the upload.
- **Downloads continue where they stopped.** When a transfer breaks, the next attempt asks only for the bytes still
  missing (`Range`, with `If-Match` on the object's ETag so a changed object is refused rather than spliced) and
  continues the same `.part` file and the same checksum.
- **Presigned links** (`presign_get`, `presign_put`) let anyone holding the URL make that one request — with any
  plain HTTP client — for 1 second to 7 days. A `PUT` link does not sign the body: it accepts any bytes until it
  expires, so share it only with whoever should write that key.
- **Integrity is checked end to end.** Every upload declares its CRC64NVME, which the server verifies and keeps;
  every download asks for the checksums the server keeps and checks the bytes against each one that covers the whole
  object. A body that does not match is never handed over or written into place — the download is tried again, and
  `ChecksumMismatch` is reported if it keeps failing.

## Using the command line

`tessaridb-s3-cli` does the same work from a shell. It reads the endpoint and credentials from the environment —
`TESSARIDB_S3_ENDPOINT`, `TESSARIDB_S3_ACCESS_KEY`, `TESSARIDB_S3_SECRET_KEY`, and optionally `TESSARIDB_S3_REGION` and
`TESSARIDB_S3_SESSION_TOKEN` (temporary credentials; `session_token` in a profile) —
or, when those are not set, from a profile file at `~/.config/tessaridb-s3/credentials` (`$XDG_CONFIG_HOME` when
set), which must be readable by its owner alone (`chmod 600`):

```ini
[default]
endpoint = https://s3.example.com
access_key = ...
secret_key = ...
```

`--profile <name>` picks another section. There is no flag for a secret: a command-line argument is visible to every
process on the machine and stays in shell history.

```sh
tessaridb-s3-cli mb s3://photos
tessaridb-s3-cli cp holiday.jpg s3://photos/2026/          # keeps the name: s3://photos/2026/holiday.jpg
tessaridb-s3-cli ls s3://photos/ --recursive
tessaridb-s3-cli stat s3://photos/2026/holiday.jpg --json
tessaridb-s3-cli presign s3://photos/2026/holiday.jpg --expires 600
tessaridb-s3-cli sync ./albums s3://photos/albums           # and back: sync s3://photos/albums ./albums
tessaridb-s3-cli rm s3://photos/2026 --recursive --yes      # everything under photos/2026/
```

| Command | What it does | Changes data |
|---|---|---|
| `ls [s3://bucket[/prefix]]` | the buckets, or one level of objects (`--recursive`: all of them) | no |
| `stat s3://bucket/key` | size, ETag, CRC64NVME, last modified | no |
| `presign s3://bucket/key` | a download link (`--put`: an upload link), `--expires` seconds (default 3600, at most 604800) | no |
| `mb s3://bucket` | creates the bucket | yes |
| `cp SOURCE DEST` | file → object, object → file, or object → object; an existing destination is replaced only with `--yes` | yes |
| `sync SOURCE DEST` | a directory → a prefix, or back: copies what is missing or differs, never deletes | yes |
| `rb s3://bucket --yes` | deletes an empty bucket | deletes |
| `rm s3://bucket/key --yes` | deletes an object; `--recursive` everything under a prefix taken as a directory | deletes |
| `mv SOURCE DEST --yes` | copies, checks the copy, then deletes the source | deletes |

Every command that changes data accepts `--dry-run`, which prints what it would do and changes nothing. A command
that deletes refuses to run without `--yes`. After each change the server (or the file system) is asked again, and the
command fails if the result is not what was asked: a copy must have the source's size, a deleted object must be gone.
`sync` decides a file is unchanged only when that is proven — equal size, then the MD5 a single-part ETag carries,
otherwise the CRC64NVME the server keeps — and sends it otherwise. Going down, a key that would land outside the
target directory (a `..` or empty segment) is skipped, reported, and makes the run fail.

What the server sends — keys, messages — is printed with control characters escaped, so a key cannot move the cursor
or retitle the terminal. Deletions are reported on standard error as they happen; `--json` prints one JSON object per
line. Logs go to standard error, filtered by `TESSARIDB_S3_LOG` (`warn` by default).

| Exit code | Meaning |
|---|---|
| 0 | done (also when whoever reads the output stops early, as with `\| head`) |
| 2 | the command, a location or the settings are invalid — nothing was sent |
| 3 | a safety check refused: the destination exists, the bucket is not empty, the object is missing |
| 4 | the server or the file system failed |
| 5 | it ran, but the check afterwards found a different result |
| 130 | interrupted by Ctrl-C or SIGTERM; an interrupted multipart upload stays open on the server until it is aborted |

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
