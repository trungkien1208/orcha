//! The page's background server: its state file (pid, port, token), the
//! check that it still runs, starting, stopping and the accept loop.

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::routes::Access;

/// The page polls every few seconds; nobody asking for this long means the
/// tab is gone.
const IDLE: Duration = Duration::from_secs(10 * 60);

pub(super) struct Server {
    pid: u32,
    port: u16,
    token: String,
}

impl Server {
    pub(super) fn url(&self) -> String {
        format!("http://127.0.0.1:{}/?t={}", self.port, self.token)
    }
}

fn state_path() -> PathBuf {
    crate::config::config_dir().join("settings-web.json")
}

/// The server a state file names, running or not.
fn read_state(path: &Path) -> Option<Server> {
    let state: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    Some(Server {
        pid: u32::try_from(state["pid"].as_u64()?).ok()?,
        port: u16::try_from(state["port"].as_u64()?).ok()?,
        token: state["token"].as_str()?.to_owned(),
    })
}

pub(super) fn running() -> Option<Server> {
    let server = read_state(&state_path())?;
    // a pid can be reused after a crash or reboot: the port must answer too
    let alive = crate::platform::process_exists(server.pid)
        && TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], server.port)),
            Duration::from_millis(300),
        )
        .is_ok();
    alive.then_some(server)
}

/// The running page's server, started first when there is none. Two
/// `orcha settings` at once start one server, not two: the second waits
/// on the state file's lock, then finds the first one's server.
pub(super) fn running_or_start() -> Result<Server, String> {
    if let Some(server) = running() {
        return Ok(server);
    }
    let _lock = crate::orcha_app::lock_for_update(&state_path())
        .map_err(|err| format!("could not start the settings page: {err}"))?;
    match running() {
        Some(server) => Ok(server),
        None => start(),
    }
}

/// Starts the server and waits for its address. Call it holding the state
/// file's lock (`running_or_start`); the server writes the file without it.
fn start() -> Result<Server, String> {
    let exe = std::env::current_exe().map_err(|err| format!("could not find orcha: {err}"))?;
    let child = crate::noninteractive_process::command(exe)
        .args(["settings", "serve"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|err| format!("could not start the settings page: {err}"))?;
    reap(child);
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(server) = running() {
            return Ok(server);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("the settings page did not start".to_owned())
}

/// Stops the running page's server. False when none was running.
pub(super) fn stop() -> bool {
    let Some(server) = running() else {
        return false;
    };
    stop_pid(server.pid);
    forget_state(&state_path(), server.pid);
    true
}

/// Ends this server now ("Close now" on the page).
pub(super) fn quit() -> ! {
    forget_state(&state_path(), std::process::id());
    std::process::exit(0);
}

/// Removes the state file only while it still names `pid`: a newer page
/// may have taken it over, and its address must stay.
fn forget_state(path: &Path, pid: u32) {
    let _lock = match crate::orcha_app::lock_for_update(path) {
        Ok(lock) => lock,
        Err(err) => {
            // a stale file is harmless: `running` checks the server answers
            tracing::warn!(%err, "settings page: could not lock its state file");
            return;
        }
    };
    if read_state(path).is_some_and(|server| server.pid == pid) {
        let _ = std::fs::remove_file(path);
    }
}

/// Waits for `child` on a thread of its own, so a long-running caller (the
/// TUI) does not keep it as a zombie once it exits.
pub(super) fn reap(mut child: std::process::Child) {
    let watcher = std::thread::Builder::new()
        .name("orcha-settings-reap".to_owned())
        .spawn(move || {
            let _ = child.wait();
        });
    if let Err(err) = watcher {
        tracing::warn!(%err, "settings page: could not wait for a child process");
    }
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

/// 128 bits from the operating system's random source, as 32 hex digits.
fn new_token() -> std::io::Result<String> {
    crate::platform::random_secret().map(|bytes| format!("{:032x}", u128::from_be_bytes(bytes)))
}

/// `orcha settings serve`: answers the page until nobody asked for a while.
pub(super) fn serve() -> i32 {
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
    let token = match new_token() {
        Ok(token) => token,
        Err(err) => {
            eprintln!("could not make the settings page's token: {err}");
            return 1;
        }
    };
    let state = json!({"pid": std::process::id(), "port": port, "token": token});
    if let Err(err) = write_private(&state_path(), state.to_string().as_bytes()) {
        eprintln!("could not save the settings page's address: {err}");
        return 1;
    }
    if listener.set_nonblocking(true).is_err() {
        return 1;
    }
    let access = Access::new(port, token);
    let mut last_seen = Instant::now();
    while last_seen.elapsed() < IDLE {
        match listener.accept() {
            Ok((stream, _)) => {
                if super::routes::handle(stream, &access) {
                    last_seen = Instant::now();
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(err) => tracing::warn!(%err, "settings page: accept failed"),
        }
    }
    forget_state(&state_path(), std::process::id());
    0
}

/// Writes a file only this user can read: it holds the page's token.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_extension(format!("tmp-{}", new_token()?));
    let mut file = crate::platform::create_private_state_file(&temporary)?;
    let result = file.write_all(bytes);
    drop(file);
    let result = result.and_then(|()| crate::platform::replace_file(&temporary, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_long_and_differ() {
        let one = new_token().unwrap();
        assert_eq!(one.len(), 32);
        assert!(one.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(one, new_token().unwrap());
    }

    #[test]
    fn only_the_server_a_state_file_names_removes_it() {
        let dir =
            std::env::temp_dir().join(format!("orcha-settings-forget-{}", new_token().unwrap()));
        std::fs::create_dir_all(&dir).unwrap();
        let state = dir.join("settings-web.json");
        write_private(&state, br#"{"pid": 41, "port": 9, "token": "t"}"#).unwrap();
        // an older server quitting leaves the newer one's address alone
        forget_state(&state, 40);
        assert_eq!(read_state(&state).map(|server| server.pid), Some(41));
        forget_state(&state, 41);
        assert!(!state.exists());
        // nothing to forget is fine too
        forget_state(&state, 41);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_token_state_write_does_not_follow_an_existing_symlink() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir =
            std::env::temp_dir().join(format!("orcha-settings-state-{}", new_token().unwrap()));
        std::fs::create_dir_all(&dir).unwrap();
        let other = dir.join("other.json");
        let state = dir.join("state.json");
        std::fs::write(&other, b"leave this alone").unwrap();
        symlink(&other, &state).unwrap();
        write_private(&state, b"secret").unwrap();
        assert_eq!(std::fs::read(&other).unwrap(), b"leave this alone");
        assert_eq!(std::fs::read(&state).unwrap(), b"secret");
        assert!(!std::fs::symlink_metadata(&state).unwrap().is_symlink());
        assert_eq!(
            std::fs::metadata(&state).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
