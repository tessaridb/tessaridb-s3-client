# Changelog

All notable changes to this project are recorded here. The format follows Keep a Changelog, and versions follow
Semantic Versioning.

## Unreleased

- `upload_file` and `download_file`: files up to 5 GiB streamed from and to disk in 1 MiB chunks, the upload signed
  with the file's SHA-256, the download written beside the target and renamed into place only when complete. Large
  transfers are bounded by a 60-second read inactivity timeout instead of a total one.
- `Client` and `ClientBuilder`: bucket calls (create, delete, exists, list) and object calls (put, get, head, delete,
  copy, list one page in byte order) over HTTPS with the platform's trust store, path-style addressing, every request
  signed when it is sent. Bodies are held in memory for now.
- `BucketName` and `ObjectKey` check the S3 naming rules when they are made; a key with a `.` or `..` path segment is
  refused, because a URL would collapse it.
- `Error` gains a category (`ErrorCategory`), the server's S3 code, status and request ID, and a retry class:
  transient failures are retried up to three times with full-jitter backoff, then reported as `RetriesExhausted`.
- Response documents are read without a DTD or any entity beyond the five predefined ones, and are capped at 8 MiB.
- `sign`: AWS Signature Version 4 for S3 — the `Authorization` header and presigned-URL query (1 second to
  seven days), as pure functions reproducing the AWS published examples. Credentials keep their secret out of
  `Debug` output and wipe it on drop.
- The workspace: the `tessaridb-s3-client` library crate and the `tessaridb-s3-cli` command-line crate, under the
  Apache License 2.0. Nothing is usable yet; the CLI answers `--version` only.
