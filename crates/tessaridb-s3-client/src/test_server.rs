//! A loopback HTTP server for fault tests: it reads each request's head and body, records the head, and answers with
//! whatever bytes the test's closure returns — a truncated body, an error, a 206 — then closes the connection. It
//! binds an OS-assigned port, so it never collides with anything listening on a fixed one.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::task::JoinHandle;

use crate::{Error, Result};

/// The request heads seen so far, in arrival order.
pub(crate) type Seen = Arc<Mutex<Vec<String>>>;

/// Serves `respond(index, head)` for every request; returns the base URL, the recorded heads and the task.
pub(crate) async fn serve<F>(respond: F) -> Result<(String, Seen, JoinHandle<()>)>
where
    F: Fn(usize, &str) -> Vec<u8> + Send + Sync + 'static,
{
    let io = |source| Error::Io {
        what: "a loopback listener",
        source,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(io)?;
    let address = listener.local_addr().map_err(io)?;
    let seen: Seen = Arc::default();
    let recorded = Arc::clone(&seen);
    let task = tokio::spawn(async move {
        let mut index = 0;
        while let Ok((mut socket, _)) = listener.accept().await {
            let Some(head) = read_request(&mut socket).await else {
                continue;
            };
            if let Ok(mut heads) = recorded.lock() {
                heads.push(head.clone());
            }
            let _ = socket.write_all(&respond(index, &head)).await;
            let _ = socket.shutdown().await;
            index = index.saturating_add(1);
        }
    });
    Ok((format!("http://{address}"), seen, task))
}

/// Reads one request: its head as text (lowercased), its body read and dropped.
async fn read_request(socket: &mut tokio::net::TcpStream) -> Option<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 8192];
    let end = loop {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position.saturating_add(4);
        }
    };
    let head = String::from_utf8_lossy(&buffer[..end]).to_lowercase();
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    let mut remaining = length.saturating_sub(buffer.len().saturating_sub(end));
    while remaining > 0 {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        remaining = remaining.saturating_sub(read);
    }
    Some(head)
}

/// A response with `status`, `headers` and `body`, its Content-Length set to `declared` (the body's length unless a
/// test cuts it short).
pub(crate) fn response(
    status: &str,
    headers: &[(&str, String)],
    body: &[u8],
    declared: usize,
) -> Vec<u8> {
    let mut out =
        format!("HTTP/1.1 {status}\r\ncontent-length: {declared}\r\nconnection: close\r\n");
    for (name, value) in headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("\r\n");
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}
