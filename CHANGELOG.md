# Changelog

All notable changes to this project are recorded here. The format follows Keep a Changelog, and versions follow
Semantic Versioning.

## 0.1.0 — 2026-10-15

The first release: the library and the command line.


- `tessaridb-s3-cli`: `ls`, `stat`, `presign`, `mb`, `rb`, `cp`, `mv`, `rm` and `sync` over the library. Credentials
  come from the environment or an owner-only profile file, never from an argument. Commands that change data take
  `--dry-run`; commands that delete need `--yes`; an existing destination is replaced only with `--yes`. Every change
  is checked against the server afterwards, and exit codes separate invalid input (2), a refused safety check (3), a
  failure (4), an unconfirmed result (5) and an interruption (130). Server text is printed with control characters
  escaped.
- `presign_get` and `presign_put`: path-style presigned URLs for one request, 1 second to 7 days.
- `download_file` resumes a broken transfer: the next attempt requests the missing range with `If-Match` on the
  object's ETag and continues the same part file and checksum.
- Multipart: `upload_file` sends files over 16 MiB in parts (one planning read, four parts in flight, whole-file
  CRC64NVME checked by the server at completion); `begin_upload`, `resume_upload` (sends only the parts the server
  does not hold intact) and `abort_upload`; a transient failure leaves the upload open as `UploadInterrupted`.
- Checksums: `put_object` and `upload_file` declare the body's CRC64NVME; `get_object` and `download_file` check the
  body against every whole-object checksum the server returns (CRC32, CRC32C, CRC64NVME, SHA-1, SHA-256) before
  handing it over, retrying on `ChecksumMismatch`. `head_object` reports the stored CRC64NVME.
- `upload_file` and `download_file`: files up to 5 GiB streamed from and to disk in 1 MiB chunks, the upload signed
  with the file's SHA-256, the download written beside the target and renamed into place only when complete. Large
  transfers are bounded by a 60-second read inactivity timeout instead of a total one.
- `Client` and `ClientBuilder`: bucket calls (create, delete, exists, list) and object calls (put, get, head, delete,
  copy, list one page in byte order) over HTTPS with the platform's trust store, path-style addressing, every request
  signed when it is sent. `put_object` and `get_object` hold the body in memory; files go through
  `upload_file` and `download_file`.
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
