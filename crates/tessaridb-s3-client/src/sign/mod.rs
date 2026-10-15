//! AWS Signature Version 4 for S3, as pure functions over a request's parts: nothing here touches the network, so every
//! byte of a signature is testable against the published examples. The canonical request is built exactly as the
//! server rebuilds it — path segments encoded once with `/` kept, query names and values encoded then sorted, header
//! names lowercased and sorted, values trimmed with inner runs of spaces collapsed.

mod credentials;
mod encode;

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

pub use credentials::Credentials;
pub(crate) use encode::{canonical_uri, uri_encode};

use crate::constants::{PRESIGN_MAX_SECS, PRESIGN_MIN_SECS};
use crate::{Error, Result};

const ALGORITHM: &str = "AWS4-HMAC-SHA256";
const SERVICE: &str = "s3";
const UNSIGNED_PAYLOAD: &str = "UNSIGNED-PAYLOAD";

type HmacSha256 = Hmac<Sha256>;

/// Signs requests with one set of credentials for one region.
#[derive(Debug, Clone)]
pub struct Signer {
    credentials: Credentials,
    region: String,
}

impl Signer {
    /// A signer for `credentials` in `region`.
    #[must_use]
    pub fn new(credentials: Credentials, region: &str) -> Self {
        Self {
            credentials,
            region: region.to_owned(),
        }
    }

    /// The `Authorization` header value for a request whose path is `path` (unencoded), whose query is `query`
    /// (unencoded names and values), and which carries `headers` (every one of them is signed — the caller passes
    /// `host`, `x-amz-date`, `x-amz-content-sha256` and whatever else it sends) and a body hashing to `payload_hash`.
    #[must_use]
    pub fn authorization(
        &self,
        method: &str,
        path: &str,
        query: &[(String, String)],
        headers: &[(String, String)],
        payload_hash: &str,
        now: DateTime<Utc>,
    ) -> String {
        let (signed_headers, canonical_headers) = canonical_headers(headers);
        let canonical = canonical_request(
            method,
            path,
            query,
            &canonical_headers,
            &signed_headers,
            payload_hash,
        );
        let signature = self.signature(&canonical, now);
        format!(
            "{ALGORITHM} Credential={}/{},SignedHeaders={signed_headers},Signature={signature}",
            self.credentials.access_key_id(),
            self.scope(now)
        )
    }

    /// The query parameters that make `method path` on `host` a presigned URL valid for `expires_secs` from `now`,
    /// in the order they are appended; the payload is unsigned and only `host` is signed.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] when `expires_secs` is not 1 to 604 800 (seven days).
    pub fn presign(
        &self,
        method: &str,
        path: &str,
        host: &str,
        expires_secs: u64,
        now: DateTime<Utc>,
    ) -> Result<Vec<(String, String)>> {
        if !(PRESIGN_MIN_SECS..=PRESIGN_MAX_SECS).contains(&expires_secs) {
            return Err(Error::InvalidArgument {
                what: "presigned URL lifetime",
                rule: "1 to 604800 seconds (seven days)",
            });
        }
        let mut query = vec![
            ("X-Amz-Algorithm".to_owned(), ALGORITHM.to_owned()),
            (
                "X-Amz-Credential".to_owned(),
                format!("{}/{}", self.credentials.access_key_id(), self.scope(now)),
            ),
            ("X-Amz-Date".to_owned(), amz_date(now)),
            ("X-Amz-Expires".to_owned(), expires_secs.to_string()),
            ("X-Amz-SignedHeaders".to_owned(), "host".to_owned()),
        ];
        let canonical = canonical_request(
            method,
            path,
            &query,
            &format!("host:{}\n", host.trim()),
            "host",
            UNSIGNED_PAYLOAD,
        );
        query.push((
            "X-Amz-Signature".to_owned(),
            self.signature(&canonical, now),
        ));
        Ok(query)
    }

    fn scope(&self, now: DateTime<Utc>) -> String {
        format!(
            "{}/{}/{SERVICE}/aws4_request",
            now.format("%Y%m%d"),
            self.region
        )
    }

    fn signature(&self, canonical: &str, now: DateTime<Utc>) -> String {
        let string_to_sign = format!(
            "{ALGORITHM}\n{}\n{}\n{}",
            amz_date(now),
            self.scope(now),
            hex(&Sha256::digest(canonical.as_bytes()))
        );
        let date = now.format("%Y%m%d").to_string();
        let key = self.credentials.with_secret(|secret| {
            let mut first = Vec::with_capacity(secret.len().saturating_add(4));
            first.extend_from_slice(b"AWS4");
            first.extend_from_slice(secret.as_bytes());
            let k_date = hmac(&first, date.as_bytes());
            zeroize::Zeroize::zeroize(&mut first);
            k_date
        });
        let k_region = hmac(&key, self.region.as_bytes());
        let k_service = hmac(&k_region, SERVICE.as_bytes());
        let k_signing = hmac(&k_service, b"aws4_request");
        hex(&hmac(&k_signing, string_to_sign.as_bytes()))
    }
}

/// `now` as SigV4 writes it: `YYYYMMDD'T'HHMMSS'Z'`.
#[must_use]
pub fn amz_date(now: DateTime<Utc>) -> String {
    now.format("%Y%m%dT%H%M%SZ").to_string()
}

/// The lowercase hex SHA-256 of `bytes`, as `x-amz-content-sha256` carries it.
#[must_use]
pub fn payload_sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn canonical_request(
    method: &str,
    path: &str,
    query: &[(String, String)],
    canonical_headers: &str,
    signed_headers: &str,
    payload_hash: &str,
) -> String {
    let mut encoded: Vec<(String, String)> = query
        .iter()
        .map(|(name, value)| (uri_encode(name, true), uri_encode(value, true)))
        .collect();
    encoded.sort();
    let query = encoded
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    format!(
        "{method}\n{}\n{query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}",
        canonical_uri(path)
    )
}

/// The `SignedHeaders` list and the canonical header block for `headers`.
fn canonical_headers(headers: &[(String, String)]) -> (String, String) {
    let mut lines: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| {
            (
                name.trim().to_ascii_lowercase(),
                value.split_whitespace().collect::<Vec<_>>().join(" "),
            )
        })
        .collect();
    lines.sort();
    let signed = lines
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let block = lines
        .iter()
        .fold(String::new(), |mut block, (name, value)| {
            block.push_str(name);
            block.push(':');
            block.push_str(value);
            block.push('\n');
            block
        });
    (signed, block)
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let Ok(mut mac) = HmacSha256::new_from_slice(key) else {
        unreachable!("HMAC accepts a key of any length")
    };
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

#[cfg(test)]
mod tests;
