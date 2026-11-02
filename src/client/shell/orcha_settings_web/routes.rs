//! Which request gets what: the host and token checks, the page and its
//! scripts, and the API, which hands each request to the settings model.

use std::net::TcpStream;

use serde_json::json;

use super::super::orcha_settings_model as model;
use super::http::{read_request, respond, respond_json, respond_result, respond_with, Request};

const PAGE: &str = include_str!("../orcha_settings_web.html");
/// The page's own code, served as a file so the CSP needs no inline script.
const APP: &str = include_str!("../orcha_settings_web.js");
/// Pico CSS 2.0.6 (MIT, vendor/pico.LICENSE.md) draws the page's controls.
/// Inlined, so the page needs no network and the CSP stays closed.
const PICO: &str = include_str!("../vendor/pico.min.css");
/// Preact with hooks and htm (MIT and Apache-2.0, vendor/*.LICENSE): the
/// page is written as components, served from here so it needs no network.
const PREACT: &str = include_str!("../vendor/htm-preact.mjs");
/// Where the page's API token goes; the page reads it from there.
const TOKEN_SLOT: &str = "__ORCHA_TOKEN__";

fn page() -> &'static str {
    static FULL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    FULL.get_or_init(|| PAGE.replacen("/*PICO*/", PICO, 1))
}

/// Who may talk to this server: the hosts it answers to, the token and
/// the cookie that lets the same browser reload the page.
pub(super) struct Access {
    token: String,
    hosts: [String; 2],
    /// Cookies ignore ports: the port in the name keeps two pages apart.
    cookie: String,
}

impl Access {
    pub(super) fn new(port: u16, token: String) -> Self {
        Self {
            token,
            hosts: [format!("127.0.0.1:{port}"), format!("localhost:{port}")],
            cookie: format!("orcha_settings_{port}"),
        }
    }

    /// The page: the token in the URL (from `orcha settings`), or the
    /// cookie the first visit set (a reload, a bookmark).
    fn may_open_page(&self, request: &Request) -> bool {
        let given = request
            .query
            .split('&')
            .find_map(|pair| pair.strip_prefix("t="));
        given.is_some_and(|given| same_secret(given, &self.token))
            || request
                .cookie(&self.cookie)
                .is_some_and(|given| same_secret(given, &self.token))
    }

    /// The API takes only the header: a page on another local port could
    /// make the browser send the cookie, never the header.
    fn may_call_api(&self, request: &Request) -> bool {
        same_secret(&request.token, &self.token)
    }

    /// HttpOnly keeps it from scripts, SameSite=Strict from other sites;
    /// it lasts until the browser closes.
    fn set_cookie(&self) -> String {
        format!(
            "Set-Cookie: {}={}; HttpOnly; SameSite=Strict; Path=/\r\n",
            self.cookie, self.token
        )
    }
}

/// Compares a token in time that depends only on the lengths, so another
/// local program cannot guess it a character at a time from how fast a
/// wrong one is refused. The length is no secret: it is always 32.
fn same_secret(given: &str, expected: &str) -> bool {
    let (given, expected) = (given.as_bytes(), expected.as_bytes());
    if expected.is_empty() || given.len() != expected.len() {
        return false;
    }
    let difference = given
        .iter()
        .zip(expected)
        .fold(0u8, |difference, (a, b)| difference | (a ^ b));
    // keeps the optimizer from turning the fold back into an early exit
    std::hint::black_box(difference) == 0
}

fn script(stream: &mut TcpStream, code: &str) {
    respond(
        stream,
        "200 OK",
        "text/javascript; charset=utf-8",
        code.as_bytes(),
    );
}

/// Answers one request. True when it came from the page (keeps it alive).
pub(super) fn handle(mut stream: TcpStream, access: &Access) -> bool {
    let Some(request) = read_request(&mut stream) else {
        return false;
    };
    // a page on another site pointing its name at 127.0.0.1 sends its own host
    if !access.hosts.contains(&request.host) {
        respond(&mut stream, "403 Forbidden", "text/plain", b"wrong host");
        return false;
    }
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") => {
            if !access.may_open_page(&request) {
                respond(
                    &mut stream,
                    "403 Forbidden",
                    "text/plain; charset=utf-8",
                    b"Open this page with: orcha settings",
                );
                return false;
            }
            let page = page().replacen(TOKEN_SLOT, &access.token, 1);
            respond_with(
                &mut stream,
                "200 OK",
                "text/html; charset=utf-8",
                &access.set_cookie(),
                page.as_bytes(),
            );
            true
        }
        ("GET", "/app.js") => {
            script(&mut stream, APP);
            true
        }
        ("GET", "/vendor/htm-preact.mjs") => {
            script(&mut stream, PREACT);
            true
        }
        (_, path) if path.starts_with("/api/") && !access.may_call_api(&request) => {
            respond_json(&mut stream, "403 Forbidden", &json!({"error": "bad token"}));
            false
        }
        _ => api(&mut stream, &request),
    }
}

/// The page's API, once the token checked out.
fn api(stream: &mut TcpStream, request: &Request) -> bool {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/api/state") => respond_json(stream, "200 OK", &model::state()),
        ("GET", "/api/background") => {
            respond_json(stream, "200 OK", &crate::orcha_background::snapshot());
        }
        ("POST", "/api/background") => {
            let body = request.json();
            let result = model::background_action(&body);
            respond_result(stream, result.map(|note| json!({"ok": true, "note": note})));
            if body["what"] == "settings" {
                super::process::quit();
            }
        }
        ("GET", "/api/accounts") => respond_json(stream, "200 OK", &model::accounts_state(false)),
        ("POST", path) if path.starts_with("/api/accounts/") => {
            let action = &path["/api/accounts/".len()..];
            respond_result(stream, model::account_action(action, &request.json()));
        }
        ("GET", "/api/projects") => respond_json(stream, "200 OK", &model::projects_state()),
        ("POST", path) if path.starts_with("/api/projects/") => {
            let action = &path["/api/projects/".len()..];
            respond_result(stream, model::project_action(action, &request.json()));
        }
        ("POST", "/api/set") => {
            let body = request.json();
            let key = body["key"].as_str().unwrap_or_default();
            // saved but not fully applied still answers with the new state,
            // so the page shows what is in the file and the warning beside it
            let result = model::apply(key, &body["value"]).map(|saved| {
                json!({
                    "ok": true,
                    "note": saved.note,
                    "warning": saved.warning,
                    "state": model::state(),
                })
            });
            respond_result(stream, result);
        }
        _ => {
            respond(stream, "404 Not Found", "text/plain", b"not found");
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::super::http::MAX_BODY;
    use super::*;

    /// Sends one raw request to `handle` and returns the whole reply.
    /// "{port}" in the request becomes the server's port.
    fn reply_to(raw: &str) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let raw = raw.replace("{port}", &port.to_string());
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream.write_all(raw.as_bytes()).unwrap();
            let mut reply = String::new();
            let _ = stream.read_to_string(&mut reply);
            reply
        });
        let (stream, _) = listener.accept().unwrap();
        handle(stream, &Access::new(port, "secret".to_owned()));
        client.join().unwrap()
    }

    /// Sends one raw request to `handle` and returns the status line.
    fn status_of(raw: &str) -> String {
        reply_to(raw).lines().next().unwrap_or_default().to_owned()
    }

    fn header<'a>(reply: &'a str, name: &str) -> &'a str {
        reply
            .lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix(": "))
            .unwrap_or_default()
    }

    #[test]
    fn the_page_and_the_api_need_the_token_and_this_host() {
        let get = |path: &str, host: &str, token: &str| {
            status_of(&format!(
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
            status_of(&format!(
                "POST /api/set HTTP/1.1\r\nHost: localhost:{{port}}\r\nX-Orcha-Token: secret\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ))
        };
        assert!(set(r#"{"key":"nope","value":1}"#).contains("400"));
    }

    #[test]
    fn a_reload_gets_in_with_the_cookie_but_the_api_does_not() {
        let first = reply_to("GET /?t=secret HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        let cookie = header(&first, "Set-Cookie");
        assert!(cookie.starts_with("orcha_settings_"), "{cookie}");
        assert!(cookie.contains("=secret;"), "{cookie}");
        for attribute in ["HttpOnly", "SameSite=Strict", "Path=/"] {
            assert!(cookie.contains(attribute), "{cookie} lacks {attribute}");
        }
        // the page gets its API token from the server, not from the URL
        assert!(first.contains(r#"<meta name="orcha-token" content="secret" />"#));
        assert!(!first.contains(TOKEN_SLOT));

        let reload = |cookie: &str| {
            status_of(&format!(
                "GET / HTTP/1.1\r\nHost: 127.0.0.1:{{port}}\r\nCookie: theme=dark; {cookie}\r\n\r\n"
            ))
        };
        assert!(reload("orcha_settings_{port}=secret").contains("200"));
        assert!(reload("orcha_settings_{port}=wrong").contains("403"));
        // another port's page
        assert!(reload("orcha_settings_1=secret").contains("403"));
        let api = status_of(
            "GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: orcha_settings_{port}=secret\r\n\r\n",
        );
        assert!(api.contains("403"), "{api}");
    }

    #[test]
    fn tokens_compare_whole() {
        assert!(same_secret("secret", "secret"));
        assert!(!same_secret("secreT", "secret"));
        assert!(!same_secret("Secret", "secret"));
        assert!(!same_secret("secret ", "secret"));
        assert!(!same_secret("", "secret"));
        // a server without a token lets nobody in
        assert!(!same_secret("", ""));
    }

    #[test]
    fn an_oversized_body_gets_no_answer() {
        let reply = status_of(&format!(
            "POST /api/set HTTP/1.1\r\nHost: 127.0.0.1:{{port}}\r\nX-Orcha-Token: secret\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY + 1
        ));
        assert!(reply.is_empty());
    }

    #[test]
    fn a_complete_oversized_header_is_refused() {
        let reply = status_of(&format!(
            "GET /?t=secret HTTP/1.1\r\nHost: 127.0.0.1:{{port}}\r\nX-Padding: {}\r\n\r\n",
            "x".repeat(16 * 1024)
        ));
        assert!(reply.is_empty());
    }

    #[test]
    fn the_page_runs_no_inline_script() {
        let page = reply_to("GET /?t=secret HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        let csp = header(&page, "Content-Security-Policy");
        assert!(csp.contains("script-src 'self';"), "{csp}");
        let scripts = csp
            .split(';')
            .find(|part| part.trim().starts_with("script-src"))
            .unwrap_or_default();
        assert!(!scripts.contains("unsafe-inline"), "{csp}");
        // every script tag on the page loads a file from this server
        assert_eq!(page.matches("<script").count(), 1);
        assert!(page.contains(r#"<script type="module" src="/app.js"></script>"#));
        let app = reply_to("GET /app.js HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        assert!(app.starts_with("HTTP/1.1 200"));
        assert_eq!(
            header(&app, "Content-Type"),
            "text/javascript; charset=utf-8"
        );
        assert!(app.contains(r#"from "/vendor/htm-preact.mjs""#));
    }
}
