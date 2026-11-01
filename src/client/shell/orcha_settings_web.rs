//! `orcha settings`: every setting on a page in the browser, served from
//! 127.0.0.1 by a small background process. The page reads and writes the
//! same files the terminal Settings does, through the same edits, and asks
//! the running server to reload so a change shows at once.
//!
//! Only this machine can reach it, and only with the token in the URL: the
//! page sends it back in a header, which a page from another origin cannot.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::orcha_settings_model as model;

const PAGE: &str = include_str!("orcha_settings_web.html");
/// Pico CSS 2.0.6 (MIT, vendor/pico.LICENSE.md) draws the page's controls.
/// Inlined, so the page needs no network and the CSP stays closed.
const PICO: &str = include_str!("vendor/pico.min.css");
/// Preact with hooks and htm (MIT and Apache-2.0, vendor/*.LICENSE): the
/// page is written as components, served from here so it needs no network.
const PREACT: &str = include_str!("vendor/htm-preact.mjs");

fn page() -> &'static str {
    static FULL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    FULL.get_or_init(|| PAGE.replacen("/*PICO*/", PICO, 1))
}
const USAGE: &str = "usage: orcha settings [stop]";
/// The page polls every few seconds; nobody asking for this long means the
/// tab is gone.
const IDLE: Duration = Duration::from_secs(10 * 60);
const MAX_HEADER: usize = 16 * 1024;
const MAX_BODY: usize = 64 * 1024;

pub(crate) fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        None | Some("open") => match open_quietly("") {
            Ok(Opened::Browser(url)) => {
                println!("Settings are open in your browser: {url}");
                0
            }
            Ok(Opened::Remote(url)) => {
                println!("{}", remote_hint(&url));
                0
            }
            Err(message) => {
                eprintln!("{message}");
                1
            }
        },
        Some("serve") => serve(),
        Some("stop") => {
            if let Some(server) = running() {
                stop_pid(server.pid);
                let _ = std::fs::remove_file(state_path());
                println!("Settings page stopped.");
            } else {
                println!("The settings page is not running.");
            }
            0
        }
        Some("--help") | Some("-h") | Some("help") => {
            println!("{USAGE}");
            println!("  orcha settings       open every setting in the browser");
            println!("  orcha settings stop  stop the page's background server");
            0
        }
        Some(other) => {
            eprintln!("unknown settings command: {other}\n{USAGE}");
            2
        }
    }
}

/// Opens the page, starting its background server first when needed.
/// Blocking for up to a few seconds: call it off the input path.
pub(crate) enum Opened {
    Browser(String),
    /// Over SSH: the browser is on another machine, so only the address.
    Remote(String),
}

pub(crate) fn over_ssh() -> bool {
    ["SSH_CONNECTION", "SSH_TTY"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
}

/// What to do with the address when orcha runs on another machine.
pub(crate) fn remote_hint(url: &str) -> String {
    let port = url
        .strip_prefix("http://127.0.0.1:")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("PORT");
    format!(
        "orcha runs over SSH: forward the page first, then open it on your own machine.\n  ssh -L {port}:127.0.0.1:{port} <this host>\n  {url}"
    )
}

/// `section` (say "background") scrolls the page to it.
pub(crate) fn open_quietly(section: &str) -> Result<Opened, String> {
    let server = match running() {
        Some(server) => server,
        None => start()?,
    };
    let mut url = server.url();
    if !section.is_empty() {
        url = format!("{url}#{section}");
    }
    if over_ssh() {
        return Ok(Opened::Remote(url));
    }
    crate::platform::open_url(&url).map_err(|err| format!("could not open {url}: {err}"))?;
    Ok(Opened::Browser(url))
}

struct Server {
    pid: u32,
    port: u16,
    token: String,
}

impl Server {
    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/?t={}", self.port, self.token)
    }
}

fn state_path() -> PathBuf {
    crate::config::config_dir().join("settings-web.json")
}

fn running() -> Option<Server> {
    let state: Value = serde_json::from_slice(&std::fs::read(state_path()).ok()?).ok()?;
    let server = Server {
        pid: u32::try_from(state["pid"].as_u64()?).ok()?,
        port: u16::try_from(state["port"].as_u64()?).ok()?,
        token: state["token"].as_str()?.to_owned(),
    };
    // a pid can be reused after a crash or reboot: the port must answer too
    let alive = crate::platform::process_exists(server.pid)
        && TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], server.port)),
            Duration::from_millis(300),
        )
        .is_ok();
    alive.then_some(server)
}

fn start() -> Result<Server, String> {
    let exe = std::env::current_exe().map_err(|err| format!("could not find orcha: {err}"))?;
    crate::noninteractive_process::command(exe)
        .args(["settings", "serve"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|err| format!("could not start the settings page: {err}"))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(server) = running() {
            return Ok(server);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("the settings page did not start".to_owned())
}

fn stop_pid(pid: u32) {
    #[cfg(windows)]
    let mut command = {
        let mut command = std::process::Command::new("taskkill");
        command.args(["/PID", &pid.to_string(), "/F"]);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = std::process::Command::new("kill");
        command.arg(pid.to_string());
        command
    };
    let _ = command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// 128 random bits from the std hasher's per-process random keys.
fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let half = || {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default(),
        );
        hasher.finish()
    };
    format!("{:016x}{:016x}", half(), half())
}

fn serve() -> i32 {
    let listener = match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("could not open a local port: {err}");
            return 1;
        }
    };
    let Ok(port) = listener.local_addr().map(|addr| addr.port()) else {
        return 1;
    };
    let token = new_token();
    let state = json!({"pid": std::process::id(), "port": port, "token": token});
    if let Err(err) = write_private(&state_path(), state.to_string().as_bytes()) {
        eprintln!("could not save the settings page's address: {err}");
        return 1;
    }
    if listener.set_nonblocking(true).is_err() {
        return 1;
    }
    let host_ok = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    let mut last_seen = Instant::now();
    while last_seen.elapsed() < IDLE {
        match listener.accept() {
            Ok((stream, _)) => {
                if handle(stream, &token, &host_ok) {
                    last_seen = Instant::now();
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(err) => tracing::warn!(%err, "settings page: accept failed"),
        }
    }
    // a newer page may have taken the file over
    if running().is_some_and(|server| server.pid == std::process::id()) {
        let _ = std::fs::remove_file(state_path());
    }
    0
}

/// Writes a file only this user can read: it holds the page's token.
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    // an older file keeps its old mode through open: set it again
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)
}

struct Request {
    method: String,
    path: String,
    query: String,
    host: String,
    token: String,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break end;
        }
        if buffer.len() > MAX_HEADER {
            return None;
        }
    };
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
    let mut length = 0usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "host" => host = value.to_owned(),
            "x-orcha-token" => token = value.to_owned(),
            "content-length" => length = value.parse().ok()?,
            _ => {}
        }
    }
    if length > MAX_BODY {
        return None;
    }
    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < length {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(length);
    Some(Request {
        method,
        path,
        query,
        host,
        token,
        body,
    })
}

fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\
         Referrer-Policy: no-referrer\r\nX-Frame-Options: DENY\r\n\
         Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; \
         script-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn respond_json(stream: &mut TcpStream, status: &str, value: &Value) {
    respond(
        stream,
        status,
        "application/json",
        value.to_string().as_bytes(),
    );
}

/// Answers one request. True when it came from the page (keeps it alive).
fn handle(mut stream: TcpStream, token: &str, host_ok: &[String]) -> bool {
    let Some(request) = read_request(&mut stream) else {
        return false;
    };
    // a page on another site pointing its name at 127.0.0.1 sends its own host
    if !host_ok.contains(&request.host) {
        respond(&mut stream, "403 Forbidden", "text/plain", b"wrong host");
        return false;
    }
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") => {
            let given = request
                .query
                .split('&')
                .find_map(|pair| pair.strip_prefix("t="))
                .unwrap_or_default();
            if given != token {
                respond(
                    &mut stream,
                    "403 Forbidden",
                    "text/plain; charset=utf-8",
                    b"Open this page with: orcha settings",
                );
                return false;
            }
            respond(
                &mut stream,
                "200 OK",
                "text/html; charset=utf-8",
                page().as_bytes(),
            );
            true
        }
        ("GET", "/vendor/htm-preact.mjs") => {
            respond(
                &mut stream,
                "200 OK",
                "text/javascript; charset=utf-8",
                PREACT.as_bytes(),
            );
            true
        }
        (_, path) if path.starts_with("/api/") && request.token != token => {
            respond_json(&mut stream, "403 Forbidden", &json!({"error": "bad token"}));
            false
        }
        ("GET", "/api/state") => {
            respond_json(&mut stream, "200 OK", &model::state());
            true
        }
        ("GET", "/api/background") => {
            respond_json(&mut stream, "200 OK", &crate::orcha_background::snapshot());
            true
        }
        ("POST", "/api/background") => {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
            let quit = body["what"] == "settings";
            match model::background_action(&body) {
                Ok(note) => respond_json(&mut stream, "200 OK", &json!({"ok": true, "note": note})),
                Err(message) => respond_json(
                    &mut stream,
                    "400 Bad Request",
                    &json!({"ok": false, "error": message}),
                ),
            }
            if quit {
                let _ = std::fs::remove_file(state_path());
                std::process::exit(0);
            }
            true
        }
        ("GET", "/api/accounts") => {
            respond_json(&mut stream, "200 OK", &model::accounts_state(false));
            true
        }
        ("POST", path) if path.starts_with("/api/accounts/") => {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
            match model::account_action(&path["/api/accounts/".len()..], &body) {
                Ok(value) => respond_json(&mut stream, "200 OK", &value),
                Err(message) => respond_json(
                    &mut stream,
                    "400 Bad Request",
                    &json!({"ok": false, "error": message}),
                ),
            }
            true
        }
        ("GET", "/api/projects") => {
            respond_json(&mut stream, "200 OK", &model::projects_state());
            true
        }
        ("POST", path) if path.starts_with("/api/projects/") => {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
            match model::project_action(&path["/api/projects/".len()..], &body) {
                Ok(value) => respond_json(&mut stream, "200 OK", &value),
                Err(message) => respond_json(
                    &mut stream,
                    "400 Bad Request",
                    &json!({"ok": false, "error": message}),
                ),
            }
            true
        }
        ("POST", "/api/set") => {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
            let key = body["key"].as_str().unwrap_or_default();
            match model::apply(key, &body["value"]) {
                Ok(note) => respond_json(
                    &mut stream,
                    "200 OK",
                    &json!({"ok": true, "note": note, "state": model::state()}),
                ),
                Err(message) => respond_json(
                    &mut stream,
                    "400 Bad Request",
                    &json!({"ok": false, "error": message}),
                ),
            }
            true
        }
        _ => {
            respond(&mut stream, "404 Not Found", "text/plain", b"not found");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sends one raw request to `handle` and returns the status line.
    fn status_of(raw: String) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream
                .write_all(raw.replace("{port}", &port.to_string()).as_bytes())
                .unwrap();
            let mut reply = String::new();
            let _ = stream.read_to_string(&mut reply);
            reply
        });
        let (stream, _) = listener.accept().unwrap();
        let hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
        handle(stream, "secret", &hosts);
        client
            .join()
            .unwrap()
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned()
    }

    #[test]
    fn the_page_and_the_api_need_the_token_and_this_host() {
        let get = |path: &str, host: &str, token: &str| {
            status_of(format!(
                "GET {path} HTTP/1.1\r\nHost: {host}\r\nX-Orcha-Token: {token}\r\n\r\n"
            ))
        };
        assert!(get("/?t=secret", "127.0.0.1:{port}", "").contains("200"));
        assert!(get("/?t=wrong", "127.0.0.1:{port}", "").contains("403"));
        assert!(get("/", "127.0.0.1:{port}", "").contains("403"));
        // a site that points its name at 127.0.0.1 sends its own host
        assert!(get("/?t=secret", "evil.example:{port}", "").contains("403"));
        assert!(get("/api/background", "127.0.0.1:{port}", "").contains("403"));
        assert!(get("/api/background", "127.0.0.1:{port}", "wrong").contains("403"));
        assert!(get("/nope", "127.0.0.1:{port}", "secret").contains("404"));
        let set = |body: &str| {
            status_of(format!(
                "POST /api/set HTTP/1.1\r\nHost: localhost:{{port}}\r\nX-Orcha-Token: secret\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ))
        };
        assert!(set(r#"{"key":"nope","value":1}"#).contains("400"));
    }

    #[test]
    fn an_oversized_body_gets_no_answer() {
        let reply = status_of(format!(
            "POST /api/set HTTP/1.1\r\nHost: 127.0.0.1:{{port}}\r\nX-Orcha-Token: secret\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY + 1
        ));
        assert!(reply.is_empty());
    }

    #[test]
    fn the_remote_hint_names_the_port_to_forward() {
        let hint = remote_hint("http://127.0.0.1:64344/?t=abc");
        assert!(hint.contains("ssh -L 64344:127.0.0.1:64344"));
        assert!(hint.contains("http://127.0.0.1:64344/?t=abc"));
    }

    #[test]
    fn tokens_are_long_and_differ() {
        let one = new_token();
        assert_eq!(one.len(), 32);
        assert_ne!(one, new_token());
    }

    #[test]
    fn help_and_unknown_commands_change_nothing() {
        assert_eq!(run(&["--help".to_owned()]), 0);
        assert_eq!(run(&["fly".to_owned()]), 2);
    }
}
