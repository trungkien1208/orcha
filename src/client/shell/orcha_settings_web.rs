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

use super::settings::{
    installed_apps, ACCENTS, FONT_SIZES, GIT_GUI_APPS, IDE_APPS, SIDEBAR_WIDTHS,
};

const PAGE: &str = include_str!("orcha_settings_web.html");
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
    crate::platform::process_exists(server.pid).then_some(server)
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
         script-src 'unsafe-inline'; connect-src 'self'; img-src 'self' data:\r\n\
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
                PAGE.as_bytes(),
            );
            true
        }
        (_, path) if path.starts_with("/api/") && request.token != token => {
            respond_json(&mut stream, "403 Forbidden", &json!({"error": "bad token"}));
            false
        }
        ("GET", "/api/state") => {
            respond_json(&mut stream, "200 OK", &state());
            true
        }
        ("GET", "/api/background") => {
            respond_json(&mut stream, "200 OK", &crate::orcha_background::snapshot());
            true
        }
        ("POST", "/api/background") => {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
            let quit = body["what"] == "settings";
            match background_action(&body) {
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
        ("POST", "/api/set") => {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
            let key = body["key"].as_str().unwrap_or_default();
            match apply(key, &body["value"]) {
                Ok(note) => respond_json(
                    &mut stream,
                    "200 OK",
                    &json!({"ok": true, "note": note, "state": state()}),
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

// ---- the settings themselves ----

fn option(value: &str, label: &str) -> Value {
    json!({"value": value, "label": label})
}

fn select(key: &str, label: &str, help: &str, value: &str, options: Vec<Value>) -> Value {
    json!({"key": key, "label": label, "help": help, "kind": "select", "value": value, "options": options})
}

fn toggle(key: &str, label: &str, help: &str, value: bool) -> Value {
    json!({"key": key, "label": label, "help": help, "kind": "toggle", "value": value})
}

fn text(key: &str, label: &str, help: &str, value: &str) -> Value {
    json!({"key": key, "label": label, "help": help, "kind": "text", "value": value})
}

/// orcha-config's checkout (palettes, languages), when it is installed.
fn config_tool_root() -> Option<PathBuf> {
    Some(
        crate::orcha_app::config_tool()?
            .parent()?
            .parent()?
            .to_path_buf(),
    )
}

fn orcha_json_path() -> PathBuf {
    crate::config::config_dir().join("config.json")
}

fn orcha_json() -> Value {
    std::fs::read(orcha_json_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| json!({}))
}

fn names_in(dir: PathBuf) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".json").map(str::to_owned)
        })
        .collect();
    names.sort();
    names
}

fn fonts() -> Vec<String> {
    std::process::Command::new("ghostty")
        .arg("+list-fonts")
        .output()
        .ok()
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter(|line| !line.is_empty() && !line.starts_with(char::is_whitespace))
                .filter(|line| *line != "Agent Icons")
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn palettes(root: &std::path::Path) -> Vec<Value> {
    names_in(root.join("palettes"))
        .into_iter()
        .map(|name| {
            let file: Value = std::fs::read(root.join("palettes").join(format!("{name}.json")))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
            json!({
                "value": name,
                "label": file["name"].as_str().unwrap_or(&name),
                "help": file["description"],
                "swatch": [file["bg"], file["fg"], file["ansi"][1], file["ansi"][2], file["ansi"][4], file["ansi"][5]],
            })
        })
        .collect()
}

fn state() -> Value {
    let config = crate::config::Config::load().config;
    let orcha = &config.orcha;
    let ui = &config.ui;
    let json_config = orcha_json();
    let tool = config_tool_root();
    let lang = json_config["lang"].as_str().unwrap_or("en").to_owned();
    let words: Value = tool
        .as_ref()
        .and_then(|root| std::fs::read(root.join("i18n").join(format!("{lang}.json"))).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();

    let accent_value = config
        .theme
        .custom
        .as_ref()
        .and_then(|custom| custom.mauve.clone());
    let accent = ACCENTS
        .iter()
        .find(|(_, color)| color.map(str::to_owned) == accent_value)
        .map_or("theme", |(name, _)| name);
    let width = SIDEBAR_WIDTHS
        .iter()
        .min_by_key(|(_, columns)| columns.abs_diff(ui.sidebar_width))
        .map_or("normal", |(name, _)| name);
    let apps = |names: &[&str], current: &str| {
        let mut options: Vec<Value> = installed_apps(names)
            .into_iter()
            .map(|(label, command)| option(&command, &label))
            .collect();
        if !current.is_empty() && !options.iter().any(|option| option["value"] == current) {
            options.push(option(current, current));
        }
        options
    };

    let mut sections = vec![json!({"id": "look", "title": "Look", "fields": [
        select("theme", "Theme", "orcha's own colors for panes and panels", config.theme.name.as_deref().unwrap_or("orcha"),
            crate::config::THEME_NAMES.iter().map(|name| option(name, name)).collect()),
        json!({"key": "accent", "label": "Accent", "help": "the brand color on buttons and highlights", "kind": "swatches", "value": accent,
            "options": ACCENTS.iter().map(|(name, color)| json!({"value": name, "label": name, "color": color})).collect::<Vec<_>>()}),
        select("sidebar_side", "Sidebar side", "the right panel takes the other side", match orcha.sidebar_side {
            crate::config::SidebarSide::Left => "left", crate::config::SidebarSide::Right => "right" },
            vec![option("left", "left"), option("right", "right")]),
        select("sidebar_width", "Sidebar width", "", width,
            SIDEBAR_WIDTHS.iter().map(|(name, _)| option(name, name)).collect()),
        toggle("right_panel", "Right panel", "accounts, changes and PROCESS beside your panes", ui.right_panel),
        select("fuel", "Fuel gauges", "how much each account has left", match orcha.fuel {
            crate::config::FuelPlacement::Panel => "panel", crate::config::FuelPlacement::StatusBar => "status_bar" },
            vec![option("panel", "right panel"), option("status_bar", "status bar")]),
        select("process", "Process", "the running agents as little trains", match orcha.process {
            crate::config::ProcessPlacement::Panel => "panel", crate::config::ProcessPlacement::Sidebar => "sidebar",
            crate::config::ProcessPlacement::Off => "off" },
            vec![option("sidebar", "sidebar"), option("panel", "right panel"), option("off", "hidden")]),
        select("icons", "Agent logos", "", match orcha.icons {
            crate::config::IconMode::Auto => "auto", crate::config::IconMode::Font => "font",
            crate::config::IconMode::Symbols => "symbols" },
            vec![option("auto", "auto"), option("font", "icon font"), option("symbols", "plain symbols")]),
        toggle("pane_paper", "Pane paper", "paint empty pane cells with the theme's paper", orcha.pane_paper),
    ]})];

    let mut font_fields = Vec::new();
    if let Some(root) = tool.as_ref() {
        font_fields.push(json!({"key": "palette", "label": "Terminal palette", "help": "Ghostty's colors, set by orcha-config", "kind": "palettes",
            "value": json_config["palette"], "options": palettes(root)}));
    }
    let font_names = fonts();
    let family = json_config["font"]["family"].as_str().unwrap_or_default();
    if !font_names.is_empty() {
        let mut options: Vec<Value> = font_names.iter().map(|name| option(name, name)).collect();
        if !family.is_empty() && !font_names.iter().any(|name| name == family) {
            options.insert(0, option(family, family));
        }
        font_fields.push(select(
            "font",
            "Font",
            "Ghostty reloads it with ⌘⇧,",
            family,
            options,
        ));
    }
    font_fields.push(
        json!({"key": "font_size", "label": "Font size", "help": "", "kind": "range",
        "min": FONT_SIZES.start(), "max": FONT_SIZES.end(),
        "value": json_config["font"]["size"].as_u64().unwrap_or(15)}),
    );
    font_fields.push(toggle(
        "ligatures",
        "Ligatures",
        "-> and != drawn as one symbol",
        json_config["font"]["ligatures"].as_bool().unwrap_or(true),
    ));
    sections.push(json!({"id": "font", "title": "Terminal & font", "fields": font_fields}));

    sections.push(json!({"id": "apps", "title": "Apps", "fields": [
        select("ide", "IDE", "opens a project's folder", &orcha.ide, apps(IDE_APPS, &orcha.ide)),
        select("git_gui", "Git app", "opens a project's repository", &orcha.git_gui, apps(GIT_GUI_APPS, &orcha.git_gui)),
    ]}));

    sections.push(json!({"id": "alerts", "title": "Notifications", "fields": [
        select("status_indicators", "Status indicators", "how an agent's state is drawn", ui.status_indicators.as_str(),
            vec![option("dots", "dots"), option("symbols", "symbols")]),
        toggle("sound", "Sound", "a sound when an agent finishes or needs you", ui.sound.enabled),
        select("toast", "Toasts", "where a pop-up note goes", match ui.toast.delivery {
            crate::config::ToastDelivery::Off => "off", crate::config::ToastDelivery::Herdr => "herdr",
            crate::config::ToastDelivery::Terminal => "terminal", crate::config::ToastDelivery::System => "system" },
            vec![option("off", "off"), option("herdr", "in orcha"), option("terminal", "terminal"), option("system", "system")]),
    ]}));

    sections.push(json!({"id": "handoff", "title": "Hand-off", "fields": [
        json!({"key": "checklist_percent", "label": "Ask for a checklist below", "help": "orcha asks you, then the agent writes what is done and what is next while it still can; 0: never",
            "kind": "range", "min": 0, "max": 60, "unit": "%", "value": orcha.checklist_percent}),
        json!({"key": "low_fuel_percent", "label": "Offer to switch below", "help": "orcha asks where to continue",
            "kind": "range", "min": 1, "max": 50, "unit": "%", "value": orcha.low_fuel_percent}),
        json!({"key": "auto_handoff_percent", "label": "Move on by itself below", "help": "the chat goes to the account with the most fuel, no question; 0: never",
            "kind": "range", "min": 0, "max": 50, "unit": "%", "value": orcha.auto_handoff_percent}),
        select("switch_mode", "Switching account", "how a chat moves to another account of the same agent", orcha.switch_mode.as_str(), vec![
            option("ask", "ask each time"), option("keep", "keep the chat running"),
            option("restart", "restart the agent"), option("new_pane", "new pane")]),
        text("checklist_file", "Checklist file", "in the project; a hand-off points the next agent at it", &orcha.checklist_file),
    ]}));

    if let Some(widgets) = json_config["widgets"].as_object() {
        let mut fields: Vec<Value> = widgets
            .iter()
            .map(|(name, on)| {
                let label = words[format!("w_{name}")].as_str().unwrap_or(name);
                let (place, text_label) = label.split_once(": ").unwrap_or(("", label));
                json!({"key": format!("widgets.{name}"), "label": text_label, "help": place, "kind": "toggle",
                    "value": on.as_bool().unwrap_or(false)})
            })
            .collect();
        fields.push(text(
            "weather_city",
            "Weather city",
            "empty: found from your location",
            json_config["weather_city"].as_str().unwrap_or_default(),
        ));
        sections.push(json!({"id": "widgets", "title": "Widgets", "fields": fields}));
    }

    let tracker = &json_config["tracker"];
    let repo = &json_config["repo"];
    let mut project = vec![
        text(
            "tracker.task_pattern",
            "Task number in branch names",
            "a regex; its first group is the task number",
            tracker["task_pattern"].as_str().unwrap_or_default(),
        ),
        text(
            "tracker.azure_pat_file",
            "Azure DevOps token file",
            "where the personal access token is kept",
            tracker["azure_pat_file"].as_str().unwrap_or_default(),
        ),
        text(
            "repo.branch_prefix",
            "Branch prefix",
            "new branches start with it",
            repo["branch_prefix"].as_str().unwrap_or_default(),
        ),
        text(
            "repo.base",
            "Base branch",
            "empty: found from the repo",
            repo["base"].as_str().unwrap_or_default(),
        ),
        text(
            "repo.protected",
            "Protected branches",
            "a regex; agents are warned before changing these",
            repo["protected"].as_str().unwrap_or_default(),
        ),
        text(
            "repo.check",
            "Check command",
            "auto: found from the repo, or a command like npm run typecheck",
            repo["check"].as_str().unwrap_or_default(),
        ),
    ];
    if let Some(root) = tool.as_ref() {
        project.push(select(
            "lang",
            "Language",
            "",
            &lang,
            names_in(root.join("i18n"))
                .iter()
                .map(|name| {
                    option(
                        name,
                        match name.as_str() {
                            "en" => "English",
                            "vi" => "Tiếng Việt",
                            other => other,
                        },
                    )
                })
                .collect(),
        ));
    }
    sections.push(json!({"id": "project", "title": "Project & language", "fields": project}));

    plugins_section(&mut sections, orcha.update_plugins);

    json!({"version": crate::orcha_app::ORCHA_VERSION, "sections": sections, "has_config_tool": tool.is_some()})
}

fn plugins_section(sections: &mut Vec<Value>, update_plugins: bool) {
    sections.push(json!({"id": "plugins", "title": "Plugins", "fields": [
        toggle("update_plugins", "Update plugins by themselves", "off: orcha asks before installing a new version", update_plugins),
    ]}));
}

fn one_of<'a>(value: &'a Value, allowed: &[&str]) -> Result<&'a str, String> {
    let text = value.as_str().ok_or("expected text")?;
    if allowed.contains(&text) {
        Ok(text)
    } else {
        Err(format!("{text} is not one of {}", allowed.join(", ")))
    }
}

fn flag(value: &Value) -> Result<bool, String> {
    value
        .as_bool()
        .ok_or_else(|| "expected on or off".to_owned())
}

/// Saves one setting. Ok(note) carries a hint for the page, if any.
fn apply(key: &str, value: &Value) -> Result<Option<String>, String> {
    use crate::config::ConfigEdit;
    let edit = match key {
        "theme" => {
            let name = one_of(value, crate::config::THEME_NAMES)?;
            Some(ConfigEdit::Theme(name))
        }
        "accent" => {
            let names: Vec<&str> = ACCENTS.iter().map(|(name, _)| *name).collect();
            let name = one_of(value, &names)?;
            let color = ACCENTS
                .iter()
                .find(|(each, _)| *each == name)
                .and_then(|(_, color)| *color);
            Some(ConfigEdit::Accent(color))
        }
        "sidebar_width" => {
            let names: Vec<&str> = SIDEBAR_WIDTHS.iter().map(|(name, _)| *name).collect();
            let name = one_of(value, &names)?;
            let columns = SIDEBAR_WIDTHS
                .iter()
                .find(|(each, _)| *each == name)
                .map_or(28, |(_, columns)| *columns);
            Some(ConfigEdit::SidebarWidth(columns))
        }
        "right_panel" => Some(ConfigEdit::RightPanel(flag(value)?)),
        "pane_paper" => Some(ConfigEdit::PanePaper(flag(value)?)),
        "update_plugins" => Some(ConfigEdit::UpdatePlugins(flag(value)?)),
        "sound" => Some(ConfigEdit::Sound(flag(value)?)),
        "sidebar_side" => Some(ConfigEdit::SidebarSide(
            match one_of(value, &["left", "right"])? {
                "left" => crate::config::SidebarSide::Left,
                _ => crate::config::SidebarSide::Right,
            },
        )),
        "fuel" => Some(ConfigEdit::Fuel(
            match one_of(value, &["panel", "status_bar"])? {
                "panel" => crate::config::FuelPlacement::Panel,
                _ => crate::config::FuelPlacement::StatusBar,
            },
        )),
        "process" => Some(ConfigEdit::Process(
            match one_of(value, &["panel", "sidebar", "off"])? {
                "panel" => crate::config::ProcessPlacement::Panel,
                "sidebar" => crate::config::ProcessPlacement::Sidebar,
                _ => crate::config::ProcessPlacement::Off,
            },
        )),
        "icons" => Some(ConfigEdit::Icons(
            match one_of(value, &["auto", "font", "symbols"])? {
                "auto" => crate::config::IconMode::Auto,
                "font" => crate::config::IconMode::Font,
                _ => crate::config::IconMode::Symbols,
            },
        )),
        "status_indicators" => Some(ConfigEdit::StatusIndicators(
            match one_of(value, &["dots", "symbols"])? {
                "dots" => crate::config::StatusIndicatorStyle::Dots,
                _ => crate::config::StatusIndicatorStyle::Symbols,
            },
        )),
        "toast" => Some(ConfigEdit::ToastDelivery(
            match one_of(value, &["off", "herdr", "terminal", "system"])? {
                "off" => crate::config::ToastDelivery::Off,
                "herdr" => crate::config::ToastDelivery::Herdr,
                "terminal" => crate::config::ToastDelivery::Terminal,
                _ => crate::config::ToastDelivery::System,
            },
        )),
        _ => None,
    };
    if let Some(edit) = edit {
        crate::config::write_edit(edit)?;
        reload_server();
        return Ok(None);
    }
    match key {
        "checklist_percent" | "auto_handoff_percent" => {
            let percent = value
                .as_u64()
                .filter(|percent| *percent <= if key == "checklist_percent" { 60 } else { 50 })
                .ok_or("expected a percentage")?;
            write_orcha_value(key, &percent.to_string())?;
            reload_server();
            Ok(None)
        }
        "switch_mode" => {
            let mode = one_of(value, &["ask", "keep", "restart", "new_pane"])?;
            write_orcha_value(key, &format!("\"{mode}\""))?;
            reload_server();
            Ok(None)
        }
        "checklist_file" => {
            let file = value.as_str().ok_or("expected text")?.trim();
            let path = std::path::Path::new(file);
            if file.is_empty()
                || file.contains('\n')
                || path.is_absolute()
                || path
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir))
            {
                return Err("expected a path inside the project, like .orcha/handoff.md".to_owned());
            }
            write_orcha_value(key, &serde_json::to_string(file).unwrap_or_default())?;
            reload_server();
            Ok(None)
        }
        "low_fuel_percent" => {
            let percent = value
                .as_u64()
                .filter(|percent| (1..=50).contains(percent))
                .ok_or("expected 1 to 50")?;
            write_orcha_value("low_fuel_percent", &percent.to_string())?;
            reload_server();
            Ok(None)
        }
        "ide" | "git_gui" => {
            let command = value.as_str().ok_or("expected text")?;
            let names = if key == "ide" { IDE_APPS } else { GIT_GUI_APPS };
            let known = installed_apps(names)
                .into_iter()
                .any(|(_, each)| each == command);
            let current = crate::config::Config::load().config.orcha;
            let unchanged = command
                == if key == "ide" {
                    current.ide
                } else {
                    current.git_gui
                };
            if !known && !unchanged {
                return Err(format!("{command} is not an app orcha found"));
            }
            write_orcha_value(key, &serde_json::to_string(command).unwrap_or_default())?;
            reload_server();
            Ok(None)
        }
        "palette" | "lang" => {
            let text = value.as_str().ok_or("expected text")?;
            let root = config_tool_root().ok_or("orcha-config is not installed")?;
            let dir = if key == "palette" { "palettes" } else { "i18n" };
            if !names_in(root.join(dir)).iter().any(|name| name == text) {
                return Err(format!("unknown {key}: {text}"));
            }
            let command = if key == "palette" { "theme" } else { "lang" };
            run_config_tool(&[command, text])?;
            Ok(Some(ghostty_note()))
        }
        "font" => {
            let family = value.as_str().ok_or("expected text")?;
            if family.is_empty() || family.contains('\n') {
                return Err("expected a font name".to_owned());
            }
            run_config_tool(&["font", family])?;
            Ok(Some(ghostty_note()))
        }
        "font_size" => {
            let size = value
                .as_u64()
                .and_then(|size| u16::try_from(size).ok())
                .filter(|size| FONT_SIZES.contains(size))
                .ok_or("expected a font size")?;
            let family = orcha_json()["font"]["family"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if family.is_empty() {
                return Err("pick a font first".to_owned());
            }
            run_config_tool(&["font", &family, &size.to_string()])?;
            Ok(Some(ghostty_note()))
        }
        "ligatures" => {
            run_config_tool(&["ligatures", if flag(value)? { "on" } else { "off" }])?;
            Ok(Some(ghostty_note()))
        }
        _ if key.starts_with("widgets.") => {
            let name = &key["widgets.".len()..];
            if !orcha_json()["widgets"]
                .get(name)
                .is_some_and(Value::is_boolean)
            {
                return Err(format!("unknown widget: {name}"));
            }
            run_config_tool(&["widget", name, if flag(value)? { "on" } else { "off" }])?;
            Ok(None)
        }
        "weather_city"
        | "tracker.task_pattern"
        | "tracker.azure_pat_file"
        | "repo.branch_prefix"
        | "repo.base"
        | "repo.protected"
        | "repo.check" => {
            let text = value.as_str().ok_or("expected text")?;
            if text.contains('\n') || text.len() > 400 {
                return Err("expected one short line".to_owned());
            }
            let mut config = orcha_json();
            let slot = match key.split_once('.') {
                Some((table, field)) => {
                    if !config[table].is_object() {
                        config[table] = json!({});
                    }
                    &mut config[table][field]
                }
                None => &mut config[key],
            };
            *slot = Value::String(text.to_owned());
            let pretty = serde_json::to_string_pretty(&config).map_err(|err| err.to_string())?;
            std::fs::write(orcha_json_path(), pretty + "\n")
                .map_err(|err| format!("could not save config.json: {err}"))?;
            run_config_tool(&["apply"])?;
            Ok(None)
        }
        _ => Err(format!("unknown setting: {key}")),
    }
}

/// Stop or go to something from the background list.
fn background_action(body: &Value) -> Result<String, String> {
    let pane = || {
        body["pane_id"]
            .as_str()
            .filter(|pane| !pane.is_empty() && pane.len() < 64)
            .ok_or_else(|| "expected a pane".to_owned())
    };
    let call = |method: &str, params: Value| {
        crate::orcha_handoff::herdr_json(&["api", "call", method, &params.to_string()])
            .map(|_| ())
            .ok_or_else(|| format!("{method} did not work"))
    };
    match body["what"].as_str().unwrap_or_default() {
        "focus" => {
            call("agent.focus", json!({"target": pane()?}))?;
            Ok("focused in orcha".to_owned())
        }
        "plugin_pane" => {
            call("plugin.pane.stop", json!({"pane_id": pane()?}))?;
            Ok("stopped".to_owned())
        }
        "office" => {
            crate::orcha_office::stop_quietly();
            Ok("Easy Job stopped".to_owned())
        }
        "server" => {
            call("server.stop", json!({}))?;
            Ok("the server and every agent stopped".to_owned())
        }
        "settings" => Ok("this page is closing".to_owned()),
        other => Err(format!("unknown action: {other}")),
    }
}

fn ghostty_note() -> String {
    "saved · Ghostty shows it after ⌘⇧, (reload config)".to_owned()
}

fn write_orcha_value(key: &str, value: &str) -> Result<(), String> {
    crate::config::update_file_at(&crate::config::config_path(), key, |content| {
        Ok(crate::config::upsert_section_value(
            content, "orcha", key, value,
        ))
    })
}

fn run_config_tool(args: &[&str]) -> Result<(), String> {
    let tool =
        crate::orcha_app::config_tool().ok_or("orcha-config is not installed: nothing changed")?;
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let output = crate::platform::orcha_tool_command(&tool, &args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|err| format!("could not run orcha-config: {err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let text = String::from_utf8_lossy(&output.stderr);
        Err(text
            .lines()
            .rfind(|line| !line.trim().is_empty())
            .unwrap_or("orcha-config failed")
            .trim()
            .to_owned())
    }
}

/// Tells the running server to read config.toml again; it passes the
/// reload on to every open orcha window.
fn reload_server() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let _ = crate::noninteractive_process::command(exe)
        .args(["server", "reload-config"])
        .env(
            crate::api::SOCKET_PATH_ENV_VAR,
            crate::session::active_api_socket_path(),
        )
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_or_out_of_range_values_are_refused() {
        assert!(apply("nope", &json!(true)).is_err());
        assert!(apply("theme", &json!("not-a-theme")).is_err());
        assert!(apply("sidebar_side", &json!("middle")).is_err());
        assert!(apply("right_panel", &json!("yes")).is_err());
        assert!(apply("low_fuel_percent", &json!(0)).is_err());
        assert!(apply("low_fuel_percent", &json!(90)).is_err());
        assert!(apply("repo.base", &json!("a\nb")).is_err());
        assert!(apply("checklist_percent", &json!(61)).is_err());
        assert!(apply("switch_mode", &json!("teleport")).is_err());
        assert!(apply("checklist_file", &json!("/etc/passwd")).is_err());
        assert!(apply("checklist_file", &json!("../outside.md")).is_err());
        assert!(background_action(&json!({"what": "nope"})).is_err());
        assert!(background_action(&json!({"what": "focus"})).is_err());
    }

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
