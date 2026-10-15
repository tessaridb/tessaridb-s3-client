//! One attempt of one request: build the path-style URL, sign it with SigV4 at the moment it is sent, send it over
//! rustls with the platform's trust store, and read the answer — an error status becomes an [`Error::Service`]
//! carrying the S3 code. Redirects are not followed: a signed request sent to another host would be a different
//! request. There is no switch that skips certificate checks.

use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use reqwest::header::HeaderMap;
use reqwest::{Method, Url};
use rustls::ClientConfig;

use crate::constants::{
    CONNECT_TIMEOUT_SECS, READ_TIMEOUT_SECS, REQUEST_TIMEOUT_SECS, XML_BODY_MAX_BYTES,
};
use crate::error::TransportFailure;
use crate::sign::{Signer, amz_date, canonical_uri, uri_encode};
use crate::{BucketName, ObjectKey};
use crate::{Error, Result, xml};

mod payload;

pub(crate) use payload::Payload;
use payload::body;

/// A request before it is signed.
#[derive(Debug)]
pub(crate) struct Request<'a> {
    pub(crate) method: Method,
    pub(crate) bucket: Option<&'a BucketName>,
    pub(crate) key: Option<&'a ObjectKey>,
    /// Query parameters, unencoded.
    pub(crate) query: Vec<(String, String)>,
    /// Extra headers, every one of them signed.
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) payload: Payload,
}

impl<'a> Request<'a> {
    pub(crate) fn new(
        method: Method,
        bucket: Option<&'a BucketName>,
        key: Option<&'a ObjectKey>,
    ) -> Self {
        Self {
            method,
            bucket,
            key,
            query: Vec::new(),
            headers: Vec::new(),
            payload: Payload::Memory(Bytes::new()),
        }
    }
}

/// A successful answer: its status, headers and body.
#[derive(Debug)]
pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Bytes,
}

/// The connection to one endpoint.
#[derive(Debug)]
pub(crate) struct Transport {
    http: reqwest::Client,
    signer: Signer,
    base: Url,
}

impl Transport {
    /// A transport to `endpoint` (`http://host:port` or `https://host`, nothing after the authority) signing with
    /// `signer`.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] for an endpoint that is not such a URL; [`Error::Setup`] when TLS or the HTTP
    /// client cannot be built.
    pub(crate) fn new(endpoint: &str, signer: Signer) -> Result<Self> {
        let base = endpoint_url(endpoint)?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .read_timeout(Duration::from_secs(READ_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none())
            .tls_backend_preconfigured(tls_config()?)
            .build()
            .map_err(|_| Error::Setup {
                what: "the HTTP client",
            })?;
        Ok(Self { http, signer, base })
    }

    /// Sends `request` once and reads the whole answer within the request time budget; an error status is an
    /// [`Error::Service`].
    ///
    /// # Errors
    /// [`Error::Service`], [`Error::Timeout`], [`Error::Transport`] or [`Error::InvalidResponse`].
    pub(crate) async fn send(&self, request: &Request<'_>) -> Result<Response> {
        let budget = Duration::from_secs(REQUEST_TIMEOUT_SECS);
        tokio::time::timeout(budget, self.send_unbounded(request))
            .await
            .map_err(|_| Error::Timeout(budget))?
    }

    /// Sends `request` once — a body of any size, so no total budget; each read must make progress within the read
    /// timeout — and reads the answer.
    ///
    /// # Errors
    /// As [`Transport::send`], plus [`Error::Io`] when a file payload cannot be read.
    pub(crate) async fn send_unbounded(&self, request: &Request<'_>) -> Result<Response> {
        let answer = self.open(request).await?;
        let status = answer.status().as_u16();
        let headers = answer.headers().clone();
        let body = read_body(answer, request.method == Method::HEAD).await?;
        Ok(Response {
            status,
            headers,
            body,
        })
    }

    /// Sends `request` once and hands back the successful answer unread, for its body to be streamed.
    ///
    /// # Errors
    /// [`Error::Service`] for an error status (its body read up to the cap), [`Error::Timeout`],
    /// [`Error::Transport`], [`Error::Io`] or [`Error::InvalidArgument`] for a file that changed after hashing.
    pub(crate) async fn open(&self, request: &Request<'_>) -> Result<reqwest::Response> {
        let path = request_path(request.bucket, request.key);
        let mut url = self.base.clone();
        url.set_path(&canonical_uri(&path));
        if !request.query.is_empty() {
            let query = request
                .query
                .iter()
                .map(|(name, value)| {
                    format!("{}={}", uri_encode(name, true), uri_encode(value, true))
                })
                .collect::<Vec<_>>()
                .join("&");
            url.set_query(Some(&query));
        }
        let now = chrono::Utc::now();
        let mut headers = request.headers.clone();
        headers.push(("host".to_owned(), host_header(&url)));
        headers.push(("x-amz-date".to_owned(), amz_date(now)));
        let (body, length, payload_hash) = body(&request.payload).await?;
        headers.push(("x-amz-content-sha256".to_owned(), payload_hash.clone()));
        let authorization = self.signer.authorization(
            request.method.as_str(),
            &path,
            &request.query,
            &headers,
            &payload_hash,
            now,
        );
        let mut builder = self.http.request(request.method.clone(), url);
        for (name, value) in headers.iter().filter(|(name, _)| name != "host") {
            builder = builder.header(name, value);
        }
        let answer = builder
            .header("authorization", authorization)
            .header("content-length", length)
            .body(body)
            .send()
            .await
            .map_err(transport_error)?;
        let status = answer.status().as_u16();
        if status >= 300 {
            let headers = answer.headers().clone();
            let body = read_body(answer, true).await?;
            return Err(service_error(status, &headers, &body));
        }
        Ok(answer)
    }
}

/// The unencoded path of a path-style request.
fn request_path(bucket: Option<&BucketName>, key: Option<&ObjectKey>) -> String {
    match (bucket, key) {
        (None, _) => "/".to_owned(),
        (Some(bucket), None) => format!("/{bucket}"),
        (Some(bucket), Some(key)) => format!("/{bucket}/{key}"),
    }
}

fn endpoint_url(endpoint: &str) -> Result<Url> {
    let invalid = |rule| Error::InvalidArgument {
        what: "endpoint",
        rule,
    };
    let url = Url::parse(endpoint).map_err(|_| invalid("an http:// or https:// URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(invalid("an http:// or https:// URL"));
    }
    if url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
    {
        return Err(invalid("scheme, host and port only"));
    }
    Ok(url)
}

fn host_header(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    url.port()
        .map_or_else(|| host.to_owned(), |port| format!("{host}:{port}"))
}

/// The TLS setup, handed to the HTTP client by value: it accepts a `ClientConfig` and refuses anything else (an `Arc`
/// included) only when the client is built.
fn tls_config() -> Result<ClientConfig> {
    use rustls_platform_verifier::BuilderVerifierExt as _;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| Error::Setup {
            what: "TLS protocol versions",
        })?
        .with_platform_verifier()
        .map_err(|_| Error::Setup {
            what: "the platform's trust store",
        })?
        .with_no_client_auth();
    Ok(config)
}

pub(crate) fn transport_error(error: reqwest::Error) -> Error {
    if error.is_timeout() {
        Error::Timeout(Duration::from_secs(READ_TIMEOUT_SECS))
    } else {
        Error::Transport(TransportFailure(Box::new(error)))
    }
}

/// The answer's body; an error answer or a document is read only up to the XML cap.
async fn read_body(mut answer: reqwest::Response, capped: bool) -> Result<Bytes> {
    let mut body = BytesMut::new();
    while let Some(chunk) = answer.chunk().await.map_err(transport_error)? {
        if capped && body.len().saturating_add(chunk.len()) > XML_BODY_MAX_BYTES {
            return Err(Error::InvalidResponse {
                what: "an error answer over 8 MiB",
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}

fn service_error(status: u16, headers: &HeaderMap, body: &[u8]) -> Error {
    let header = |name| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    match xml::error_document(body) {
        Some((code, message, request_id)) => Error::Service {
            status,
            code: Some(code),
            message,
            request_id: request_id.or_else(|| header("x-amz-request-id")),
        },
        None => Error::Service {
            status,
            code: None,
            message: None,
            request_id: header("x-amz-request-id"),
        },
    }
}

#[cfg(test)]
mod tests;
