//! Minimal single-threaded HTTP server for the local reader site.
//! Sequential accept loop; one response per connection. Good enough for a
//! local single-user reader.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

impl Response {
    pub fn json(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            content_type: "application/json".to_string(),
            body,
        }
    }

    pub fn text(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8".to_string(),
            body: body.into(),
        }
    }

    pub fn html(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            content_type: "text/html; charset=utf-8".to_string(),
            body: body.into(),
        }
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        413 => "Content Too Large",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

/// Bind 127.0.0.1 on `port`; if busy, try the next free port (up to +20).
pub fn listen(port: u16) -> std::io::Result<(TcpListener, u16)> {
    for candidate in port..port.saturating_add(20) {
        match TcpListener::bind(("127.0.0.1", candidate)) {
            Ok(listener) => return Ok((listener, candidate)),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AddrInUse,
        format!("no free port found from {port}"),
    ))
}

/// Serve until the listener fails. `handler(method, path)` returns the
/// response; the caller resolves the full path including query stripping.
pub fn run(
    listener: TcpListener,
    handler: impl Fn(&str, &str) -> Response + Send + Sync + 'static,
) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let response = handle_one(stream, &handler);
        if response.is_err() {
            continue;
        }
    }
}

fn handle_one(
    mut stream: TcpStream,
    handler: &(impl Fn(&str, &str) -> Response + Send + Sync),
) -> std::io::Result<()> {
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .ok();
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() > 16 * 1024 {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    if method != "GET" {
        write_response(&mut stream, Response::text(400, "only GET is supported"))?;
        return Ok(());
    }
    let response = handler(&method, &path);
    write_response(&mut stream, response)?;
    Ok(())
}

fn write_response(stream: &mut TcpStream, response: Response) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        reason(response.status),
        response.content_type,
        response.body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()
}
