//! Just enough HTTP/1.1 for the page: read one request, send one answer,
//! then close.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const MAX_HEADER: usize = 16 * 1024;
pub(super) const MAX_BODY: usize = 64 * 1024;
/// One server answers every request in turn, so a client that stalls
/// mid-request or stops reading its answer must not hold it for long.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// A whole request, however slowly it trickles in.
const REQUEST_DEADLINE: Duration = Duration::from_secs(10);

/// Scripts only from this server, never inline; styles may be inline (the
/// page's own and the theme it paints at runtime).
const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; script-src 'self'; \
     connect-src 'self'; img-src 'self' data:";

pub(super) struct Request {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) query: String,
    pub(super) host: String,
    pub(super) token: String,
    /// Every `Cookie` header, joined with "; ".
    cookies: String,
    pub(super) body: Vec<u8>,
}

impl Request {
    /// The body as JSON; null when it is not JSON.
    pub(super) fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_default()
    }

    /// The value of one cookie, if the browser sent it.
    pub(super) fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies.split(';').find_map(|pair| {
            let (key, value) = pair.trim().split_once('=')?;
            (key == name).then_some(value)
        })
    }
}

pub(super) fn read_request(stream: &mut TcpStream) -> Option<Request> {
    read_request_within(stream, REQUEST_DEADLINE)
}

/// Reads some of the request, giving up once `deadline` has passed. None
/// on a timeout, an error or a closed connection.
fn read_some(stream: &mut TcpStream, chunk: &mut [u8], deadline: Instant) -> Option<usize> {
    let left = deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())?;
    stream.set_read_timeout(Some(left.min(READ_TIMEOUT))).ok()?;
    match stream.read(chunk) {
        Ok(0) | Err(_) => None,
        Ok(read) => Some(read),
    }
}

fn read_request_within(stream: &mut TcpStream, limit: Duration) -> Option<Request> {
    let deadline = Instant::now() + limit;
    let _ = stream.set_nonblocking(false);
    // the answer goes back on this stream: a client that never reads it
    // must not block the accept loop either
    stream.set_write_timeout(Some(WRITE_TIMEOUT)).ok()?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let read = read_some(stream, &mut chunk, deadline)?;
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break end;
        }
        if buffer.len() > MAX_HEADER {
            return None;
        }
    };
    if header_end > MAX_HEADER {
        return None;
    }
    let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_owned();
    let target = first.next()?.to_owned();
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_owned(), query.to_owned()),
        None => (target, String::new()),
    };
    let mut host = String::new();
    let mut token = String::new();
    let mut cookies = Vec::new();
    let mut length = 0usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "host" => host = value.to_owned(),
            "x-orcha-token" => token = value.to_owned(),
            "cookie" => cookies.push(value),
            "content-length" => length = value.parse().ok()?,
            _ => {}
        }
    }
    let cookies = cookies.join("; ");
    if length > MAX_BODY {
        return None;
    }
    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < length {
        let read = read_some(stream, &mut chunk, deadline)?;
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(length);
    Some(Request {
        method,
        path,
        query,
        host,
        token,
        cookies,
        body,
    })
}

/// Sends one answer. `extra` holds more header lines, each ending "\r\n".
pub(super) fn respond_with(
    stream: &mut TcpStream,
    status: &str,
    kind: &str,
    extra: &str,
    body: &[u8],
) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\
         Referrer-Policy: no-referrer\r\nX-Frame-Options: DENY\r\n\
         Content-Security-Policy: {CSP}\r\n{extra}\
         Connection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

pub(super) fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) {
    respond_with(stream, status, kind, "", body);
}

pub(super) fn respond_json(stream: &mut TcpStream, status: &str, value: &Value) {
    respond(
        stream,
        status,
        "application/json",
        value.to_string().as_bytes(),
    );
}

/// An action's answer: its value, or 400 with the reason for the page.
pub(super) fn respond_result(stream: &mut TcpStream, result: Result<Value, String>) {
    match result {
        Ok(value) => respond_json(stream, "200 OK", &value),
        Err(message) => respond_json(
            stream,
            "400 Bad Request",
            &json!({"ok": false, "error": message}),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;

    /// A connected pair: what the client writes, the server reads.
    fn pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        (client, server)
    }

    #[test]
    fn a_request_that_trickles_in_forever_is_dropped_at_the_deadline() {
        let (mut client, mut server) = pair();
        let started = Instant::now();
        let writer = std::thread::spawn(move || {
            let _ = client.write_all(b"GET / HTTP/1.1\r\nX-Slow: ");
            // a byte now and then: no single read ever times out
            while started.elapsed() < Duration::from_secs(3) {
                if client.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        assert!(read_request_within(&mut server, Duration::from_millis(300)).is_none());
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        drop(server);
        writer.join().unwrap();
    }

    #[test]
    fn a_whole_request_is_read_and_its_answer_cannot_block_forever() {
        let (mut client, mut server) = pair();
        client
            .write_all(b"POST /api/set HTTP/1.1\r\nHost: h\r\nContent-Length: 2\r\n\r\n{}")
            .unwrap();
        let request = read_request(&mut server).expect("a whole request");
        assert_eq!(
            (request.method.as_str(), request.path.as_str()),
            ("POST", "/api/set")
        );
        assert_eq!(request.body, b"{}");
        assert_eq!(server.write_timeout().unwrap(), Some(WRITE_TIMEOUT));
    }
}
