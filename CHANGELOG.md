# Changelog

All notable changes to this project are recorded here. The format follows Keep a Changelog, and versions follow
Semantic Versioning.

## Unreleased

- `sign`: AWS Signature Version 4 for S3 — the `Authorization` header and presigned-URL query (1 second to
  seven days), as pure functions reproducing the AWS published examples. Credentials keep their secret out of
  `Debug` output and wipe it on drop.
- The workspace: the `tessaridb-s3-client` library crate and the `tessaridb-s3-cli` command-line crate, under the
  Apache License 2.0. Nothing is usable yet; the CLI answers `--version` only.
