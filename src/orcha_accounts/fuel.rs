use super::*;

// ---------- fuel

/// One account's plan limits: (percent used, reset time) per window.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct Limits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) session: Option<(f64, Option<f64>)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) weekly: Option<(f64, Option<f64>)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) monthly: Option<(f64, Option<f64>)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<String>,
}

/// Why a fetch failed: worth retrying later, and rate limited.
#[derive(Debug, Default)]
pub(super) struct Fetched {
    pub(super) limits: Limits,
    pub(super) transient: bool,
    pub(super) rate_limited: bool,
    /// Nothing could be asked (the agent sits idle): the last numbers
    /// still hold, so keep showing them.
    pub(super) idle: bool,
}

impl Fetched {
    fn error(message: &str) -> Self {
        Self {
            limits: Limits {
                error: Some(message.to_owned()),
                ..Limits::default()
            },
            ..Self::default()
        }
    }

    fn idle(message: &str) -> Self {
        Self {
            idle: true,
            ..Self::error(message)
        }
    }

    fn transient(message: String, rate_limited: bool) -> Self {
        Self {
            limits: Limits {
                error: Some(message),
                ..Limits::default()
            },
            transient: true,
            rate_limited,
            idle: false,
        }
    }
}

/// Why a curl request failed.
#[derive(Debug, Default)]
pub(super) struct CurlError {
    /// What curl said, cut short.
    pub(super) message: String,
    pub(super) rate_limited: bool,
    /// The HTTP status when the server answered with an error.
    pub(super) status: Option<u16>,
}

impl CurlError {
    pub(super) fn from_stderr(stderr: &str) -> Self {
        let stderr = stderr.trim();
        Self {
            message: stderr.chars().take(80).collect(),
            rate_limited: stderr.contains("error: 429") || stderr.contains(" 429"),
            status: http_status(stderr),
        }
    }
}

/// The status in curl's "The requested URL returned error: 401".
pub(super) fn http_status(stderr: &str) -> Option<u16> {
    let (_, rest) = stderr.split_once("returned error: ")?;
    rest.split(|ch: char| !ch.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

/// curl with its config (headers holding the token) on stdin. Ok is the
/// body.
pub(super) fn curl(url: &str, config: &str) -> Result<String, CurlError> {
    use std::io::Write;
    let failed = |err: std::io::Error| CurlError {
        message: format!("curl: {err}"),
        ..CurlError::default()
    };
    let mut command = Command::new("curl");
    command
        .args(["-sS", "-m", "15", "-f", "-K", "-", url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::platform::configure_background_command(&mut command);
    let mut child = command.spawn().map_err(failed)?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(config.as_bytes());
    }
    let output = child.wait_with_output().map_err(failed)?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    Err(CurlError::from_stderr(&String::from_utf8_lossy(
        &output.stderr,
    )))
}

/// Days since 1970-01-01 of a civil date (Howard Hinnant's algorithm).
pub(super) fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Seconds since the epoch of an ISO 8601 time ("2026-10-23T12:00:00Z",
/// fractions and "+07:00" offsets too).
pub(super) fn iso_secs(text: &str) -> Option<f64> {
    let text = text.trim();
    let (date, rest) = text.split_once(['T', ' '])?;
    let mut date = date.split('-');
    let year: i64 = date.next()?.parse().ok()?;
    let month: i64 = date.next()?.parse().ok()?;
    let day: i64 = date.next()?.parse().ok()?;
    let (clock, offset) = if let Some(clock) = rest.strip_suffix('Z') {
        (clock, 0)
    } else if let Some(at) = rest.rfind(['+', '-']).filter(|at| *at >= 8) {
        let (clock, zone) = rest.split_at(at);
        let sign = if zone.starts_with('-') { -1 } else { 1 };
        let mut zone = zone[1..].split(':');
        let hours: i64 = zone.next()?.parse().ok()?;
        let minutes: i64 = zone.next().unwrap_or("0").parse().ok()?;
        (clock, sign * (hours * 3600 + minutes * 60))
    } else {
        (rest, 0)
    };
    let mut clock = clock.split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next()?.parse().ok()?;
    let second: f64 = clock.next().unwrap_or("0").parse().ok()?;
    let days = days_from_civil(year, month, day);
    Some((days * 86_400 + hour * 3600 + minute * 60 - offset) as f64 + second)
}

/// The JSON payload of a JWT.
pub(super) fn jwt_payload(token: &str) -> Option<Value> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(super) fn claude_limits(accounts: &[AccountConfig], id: &str) -> Fetched {
    let oauth = claude_slot(accounts, id)
        .and_then(|slot| secret_cache::read(&slot))
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|value| value.get("claudeAiOauth").cloned());
    let Some(oauth) = oauth else {
        return Fetched::error("no sign-in found: sign in");
    };
    let expires = oauth
        .get("expiresAt")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        / 1000.0;
    if expires < now_secs() {
        // Claude renews the access token itself when it next runs on this
        // account; only a missing renewal token needs a new sign-in
        return if oauth.get("refreshToken").and_then(Value::as_str).is_some() {
            Fetched::idle("idle: fuel shows after Claude next runs on it")
        } else {
            Fetched::error("sign-in expired: sign in again")
        };
    }
    let Some(token) = oauth.get("accessToken").and_then(Value::as_str) else {
        return Fetched::error("no sign-in found: sign in");
    };
    let config = format!(
        "header = \"Authorization: Bearer {token}\"\n\
         header = \"anthropic-beta: oauth-2025-04-20\"\n\
         header = \"Content-Type: application/json\"\n"
    );
    let body = match curl(CLAUDE_USAGE_URL, &config) {
        Ok(body) => body,
        Err(err) => {
            return Fetched::transient(
                format!("request failed ({})", err.message),
                err.rate_limited,
            )
        }
    };
    let Ok(data) = serde_json::from_str::<Value>(&body) else {
        return Fetched::transient("request failed (unreadable reply)".to_owned(), false);
    };
    let window = |key: &str| {
        let window = data.get(key)?;
        let used = window.get("utilization")?.as_f64()?;
        let reset = window
            .get("resets_at")
            .and_then(Value::as_str)
            .and_then(iso_secs);
        Some((used, reset))
    };
    let limits = Limits {
        session: window("five_hour"),
        weekly: window("seven_day"),
        ..Limits::default()
    };
    if limits.session.is_none() && limits.weekly.is_none() {
        return Fetched::error("no usage data yet");
    }
    Fetched {
        limits,
        ..Fetched::default()
    }
}

/// The newest `rate_limits` in one Codex session log.
pub(super) fn codex_rate_limits(text: &str) -> Option<Value> {
    text.lines()
        .rev()
        .filter(|line| line.contains("\"rate_limits\""))
        .find_map(|line| {
            let entry: Value = serde_json::from_str(line).ok()?;
            let limits = entry
                .get("payload")
                .and_then(|payload| payload.get("rate_limits"))
                .or_else(|| entry.get("rate_limits"))?;
            limits
                .get("primary")
                .and_then(Value::as_object)
                .filter(|primary| !primary.is_empty())?;
            Some(limits.clone())
        })
}

/// Fuel polls need recent usage records, not an entire conversation. Read at
/// most 512 KiB plus one boundary byte, skipping a cut JSONL record.
pub(super) const CODEX_TAIL_BYTES: usize = 512 * 1024;

pub(super) fn codex_tail<R: std::io::Read + std::io::Seek>(
    reader: &mut R,
) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, SeekFrom};
    let len = reader.seek(SeekFrom::End(0))?;
    let start = len.saturating_sub(CODEX_TAIL_BYTES as u64);
    reader.seek(SeekFrom::Start(start.saturating_sub(1)))?;
    let mut bytes = Vec::with_capacity(CODEX_TAIL_BYTES + 1);
    reader
        .take((CODEX_TAIL_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if start > 0 {
        let boundary = if bytes.first() == Some(&b'\n') {
            1
        } else {
            bytes
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |index| index + 1)
        };
        bytes.drain(..boundary);
    }
    Ok(bytes)
}

pub(super) fn codex_window(limits: &Value, key: &str, now: f64) -> Option<(f64, Option<f64>)> {
    let window = limits.get(key)?;
    let mut used = window
        .get("used_percent")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let reset = window.get("resets_at").and_then(Value::as_f64).or_else(|| {
        window
            .get("resets_in_seconds")
            .and_then(Value::as_f64)
            .map(|seconds| now + seconds)
    });
    if reset.is_some_and(|reset| reset < now) {
        // the window already reset since that run
        used = 0.0;
    }
    Some((used, reset))
}

pub(super) fn codex_limits() -> Fetched {
    let Some(home) = home() else {
        return Fetched::error("no Codex runs found yet");
    };
    // sessions/YYYY/MM/DD/*.jsonl, the newest ten
    let mut files = Vec::new();
    let mut folders = vec![(home.join(".codex").join("sessions"), 0)];
    while let Some((folder, depth)) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if depth < 3 && path.is_dir() {
                folders.push((path, depth + 1));
            } else if depth == 3 && path.extension().is_some_and(|ext| ext == "jsonl") {
                let modified = entry
                    .metadata()
                    .and_then(|meta| meta.modified())
                    .unwrap_or(UNIX_EPOCH);
                files.push((modified, path));
            }
        }
    }
    if files.is_empty() {
        return Fetched::error("no Codex runs found yet");
    }
    files.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    let now = now_secs();
    let found = |limits: Value| Fetched {
        limits: Limits {
            session: codex_window(&limits, "primary", now),
            weekly: codex_window(&limits, "secondary", now),
            note: Some("as of Codex's last run".to_owned()),
            ..Limits::default()
        },
        ..Fetched::default()
    };
    for (_, path) in files.iter().take(10) {
        let Ok(mut file) = std::fs::File::open(path) else {
            continue;
        };
        let Ok(bytes) = codex_tail(&mut file) else {
            continue;
        };
        if let Some(limits) = codex_rate_limits(&String::from_utf8_lossy(&bytes)) {
            return found(limits);
        }
    }
    // No record near the end of the recent runs: a long run can push its
    // last one further back, so read the newest run whole once (bounded).
    if let Some(limits) = files.first().and_then(|(_, path)| {
        let size = std::fs::metadata(path).ok()?.len();
        (size <= CODEX_FULL_READ_MAX).then_some(())?;
        let bytes = std::fs::read(path).ok()?;
        codex_rate_limits(&String::from_utf8_lossy(&bytes))
    }) {
        return found(limits);
    }
    // Runs exist but none says how much is left: keep the last numbers.
    Fetched {
        idle: true,
        ..Fetched::error("no recent Codex usage found yet")
    }
}

/// The largest Codex run read whole when its end has no usage record.
const CODEX_FULL_READ_MAX: u64 = 64 * 1024 * 1024;

pub(super) fn cursor_limits() -> Fetched {
    let token = crate::platform::orcha_cursor_token();
    let user = token
        .as_deref()
        .and_then(jwt_payload)
        .and_then(|payload| payload.get("sub")?.as_str().map(str::to_owned))
        .and_then(|sub| sub.rsplit('|').next().map(str::to_owned));
    let (Some(token), Some(user)) = (token, user) else {
        return Fetched::error("not signed in to the Cursor CLI: sign in");
    };
    // the same session cookie the Cursor dashboard uses
    let config = format!("header = \"Cookie: WorkosCursorSessionToken={user}%3A%3A{token}\"\n");
    let body = match curl(CURSOR_USAGE_URL, &config) {
        Ok(body) => body,
        Err(err) => return cursor_failed(&err),
    };
    let Ok(data) = serde_json::from_str::<Value>(&body) else {
        return Fetched::transient("request failed (unreadable reply)".to_owned(), false);
    };
    let Some(plan) = data
        .get("individualUsage")
        .and_then(|usage| usage.get("plan"))
    else {
        return Fetched::transient("request failed (no plan in the reply)".to_owned(), false);
    };
    let percent = |key: &str| plan.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    let reset = data
        .get("billingCycleEnd")
        .and_then(Value::as_str)
        .and_then(iso_secs);
    Fetched {
        limits: Limits {
            monthly: Some((percent("totalPercentUsed"), reset)),
            note: Some(format!(
                "{} plan · auto {:.0}% · api {:.0}%",
                data.get("membershipType")
                    .and_then(Value::as_str)
                    .unwrap_or("?"),
                percent("autoPercentUsed"),
                percent("apiPercentUsed")
            )),
            ..Limits::default()
        },
        ..Fetched::default()
    }
}

/// A failed Cursor request: only a refused sign-in (401, 403) needs a new
/// one. Offline, a timeout or a server error is asked again later.
pub(super) fn cursor_failed(err: &CurlError) -> Fetched {
    match err.status {
        Some(401 | 403) => Fetched::error("Cursor sign-in expired: sign in again"),
        _ => Fetched::transient(
            format!("request failed ({})", err.message),
            err.rate_limited,
        ),
    }
}

pub(super) fn fetch(accounts: &[AccountConfig], account: &AccountConfig) -> Fetched {
    match account.kind.as_str() {
        "claude" => claude_limits(accounts, &account.id),
        "codex" => codex_limits(),
        "cursor" => cursor_limits(),
        _ => Fetched::default(),
    }
}

pub(super) fn modified_age(path: &Path) -> Option<f64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(
        SystemTime::now()
            .duration_since(modified)
            .map_or(0.0, |age| age.as_secs_f64()),
    )
}

/// One request at a time across orcha windows: a lock file, dropped when
/// done (a dead holder's lock goes stale).
pub(super) struct FetchLock(PathBuf);

impl FetchLock {
    fn take(path: PathBuf) -> Option<Self> {
        for _ in 0..2 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Some(Self(path)),
                Err(_) if modified_age(&path).is_some_and(|age| age > STALE_LOCK_SECS as f64) => {
                    let _ = std::fs::remove_file(&path);
                }
                Err(_) => return None,
            }
        }
        None
    }
}

impl Drop for FetchLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// An account's limits, cached for two minutes. A request that fails for a
/// while (rate limited, offline) keeps the last numbers and backs off, so
/// the gauge does not flip to an error every few minutes.
pub(super) fn limits(accounts: &[AccountConfig], account: &AccountConfig, fresh: bool) -> Limits {
    let Some(dir) = cache_dir() else {
        return fetch(accounts, account).limits;
    };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{}.json", account.id));
    let retry = dir.join(format!("{}.json.retry", account.id));
    let cached = || {
        let limits: Limits = serde_json::from_value(read_json(&path)?).ok()?;
        Some((limits, modified_age(&path).unwrap_or(f64::MAX)))
    };
    let retry_at = || {
        std::fs::read_to_string(&retry)
            .ok()
            .and_then(|text| text.trim().parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    let usable = |found: &Option<(Limits, f64)>| {
        !fresh
            && found
                .as_ref()
                .is_some_and(|(_, age)| *age < CACHE_TTL_SECS || retry_at() > now_secs())
    };
    let found = cached();
    if usable(&found) {
        return found.map(|(limits, _)| limits).unwrap_or_default();
    }
    // not limits.py's `.json.lock`: that one stays on disk for good
    let Some(_lock) = FetchLock::take(dir.join(format!("{}.json.orcha-lock", account.id))) else {
        // another window is asking right now: show what there is
        return found.map(|(limits, _)| limits).unwrap_or_default();
    };
    let found = cached();
    if usable(&found) {
        return found.map(|(limits, _)| limits).unwrap_or_default();
    }
    let fetched = fetch(accounts, account);
    if fetched.idle {
        // keep the last numbers on disk too: an idle account used nothing
        // since, and windows that reset meanwhile read full (`windows`)
        if let Some((old, _)) = found.filter(|(old, _)| old.error.is_none()) {
            return old;
        }
        return fetched.limits;
    }
    if fetched.transient {
        let wait = if fetched.rate_limited {
            RATE_LIMIT_RETRY_AFTER_SECS
        } else {
            RETRY_AFTER_SECS
        };
        let _ = std::fs::write(&retry, (now_secs() + wait).to_string());
        if let Some((old, _)) = found.filter(|(old, _)| old.error.is_none()) {
            return old;
        }
    } else {
        let _ = std::fs::remove_file(&retry);
    }
    if let Ok(text) = serde_json::to_string(&fetched.limits) {
        let _ = write_atomic(&path, &text);
    }
    fetched.limits
}

/// Drops an account's cached limits (after a sign-in or a switch).
pub(super) fn forget_limits(id: &str) {
    secret_cache::invalidate_all();
    if let Some(dir) = cache_dir() {
        let _ = std::fs::remove_file(dir.join(format!("{id}.json")));
    }
}

/// "2d 3h", "1h 05m", "12m", "now".
pub(super) fn until(reset: f64, now: f64) -> String {
    let secs = (reset - now) as i64;
    if secs <= 0 {
        return "now".to_owned();
    }
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (hours, minutes) = (rest / 3600, (rest % 3600) / 60);
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else {
        format!("{minutes}m")
    }
}

pub(super) fn windows(limits: &Limits, now: f64) -> Vec<FuelWindow> {
    [
        ("session", "5H", limits.session),
        ("weekly", "WEEK", limits.weekly),
        ("monthly", "MONTH", limits.monthly),
    ]
    .into_iter()
    .filter_map(|(key, label, window)| {
        let (mut used, mut reset) = window?;
        if reset.is_some_and(|reset| reset <= now) {
            // last numbers from before the reset: the window is full again
            used = 0.0;
            reset = None;
        }
        Some(FuelWindow {
            key: key.to_owned(),
            label: label.to_owned(),
            left: (100.0 - used).round().clamp(0.0, 100.0) as u8,
            reset: reset.map(|reset| until(reset, now)),
            reset_at: reset.map(|reset| reset.max(0.0) as u64),
        })
    })
    .collect()
}

/// The signed-in email of an account, or None. Only the email is read.
pub(super) fn email(accounts: &[AccountConfig], account: &AccountConfig) -> Option<String> {
    let home = home()?;
    match account.kind.as_str() {
        "claude" => {
            if active_claude(accounts).as_deref() == Some(account.id.as_str()) {
                claude_folder_email(None)
            } else {
                claude_profile(accounts, &account.id)?
                    .get("emailAddress")?
                    .as_str()
                    .map(str::to_owned)
            }
        }
        kind => agent_email(&home, kind),
    }
}

/// Who an agent other than Claude is signed in as, where it says.
pub(super) fn agent_email(home: &Path, kind: &str) -> Option<String> {
    match kind {
        "cursor" => read_json(&home.join(".cursor").join("cli-config.json"))?
            .get("authInfo")?
            .get("email")?
            .as_str()
            .map(str::to_owned),
        "codex" => {
            let auth = read_json(&home.join(".codex").join("auth.json"))?;
            let token = auth.get("tokens")?.get("id_token")?.as_str()?;
            jwt_payload(token)?
                .get("email")?
                .as_str()
                .map(str::to_owned)
        }
        "gemini" => read_json(&home.join(".gemini").join("google_accounts.json"))?
            .get("active")?
            .as_str()
            .map(str::to_owned),
        "copilot" => read_json(&home.join(".copilot").join("config.json"))?
            .get("logged_in_users")?
            .get(0)?
            .get("login")?
            .as_str()
            .map(str::to_owned),
        _ => None,
    }
}

/// An agent's sign-in file is there; true when orcha cannot tell.
pub(super) fn agent_signed_in(home: &Path, kind: &str) -> bool {
    agent_kind(kind).is_none_or(|agent| {
        agent.sign_in.is_empty() || agent.sign_in.iter().any(|path| home.join(path).exists())
    })
}

/// There is a sign-in for this account (maybe with an expired token).
pub(super) fn signed_in(accounts: &[AccountConfig], account: &AccountConfig) -> bool {
    match account.kind.as_str() {
        "claude" => claude_slot(accounts, &account.id)
            .and_then(|slot| secret_cache::read(&slot))
            .is_some(),
        "cursor" => email(accounts, account).is_some(),
        kind => home().is_some_and(|home| agent_signed_in(&home, kind)),
    }
}

pub(super) fn auth_needed(error: Option<&str>) -> bool {
    error.is_some_and(|error| {
        let error = error.to_lowercase();
        ["sign-in", "sign in", "signed in"]
            .iter()
            .any(|word| error.contains(word))
    })
}

#[cfg(test)]
mod tail_tests {
    use super::*;
    use std::io::{Cursor, Read, Seek, SeekFrom};

    struct CountingReader {
        cursor: Cursor<Vec<u8>>,
        read: usize,
    }

    impl Read for CountingReader {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            let count = self.cursor.read(bytes)?;
            self.read += count;
            Ok(count)
        }
    }

    impl Seek for CountingReader {
        fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
            self.cursor.seek(position)
        }
    }

    #[test]
    fn large_codex_logs_read_only_the_tail_and_choose_the_newest_valid_limits() {
        let mut bytes = vec![b'x'; CODEX_TAIL_BYTES * 8];
        bytes.extend_from_slice(b"\n{\"rate_limits\":{\"primary\":{\"used_percent\":12}}}\n");
        bytes.extend_from_slice(
            b"{\"payload\":{\"rate_limits\":{\"primary\":{\"used_percent\":37}}}}\n",
        );
        bytes.extend_from_slice(b"{\"rate_limits\":null}\n{broken rate_limits\n");
        let mut reader = CountingReader {
            cursor: Cursor::new(bytes),
            read: 0,
        };
        let tail = codex_tail(&mut reader).expect("read tail");
        assert!(
            reader.read <= CODEX_TAIL_BYTES + 1,
            "never read the conversation prefix"
        );
        assert_eq!(
            codex_rate_limits(&String::from_utf8_lossy(&tail)).unwrap()["primary"]["used_percent"],
            37
        );
    }

    #[test]
    fn tail_keeps_complete_boundary_records_and_discards_cut_records() {
        let record = b"{\"rate_limits\":{\"primary\":{\"used_percent\":12}}}\n";
        let mut bytes = b"prefix\n".to_vec();
        bytes.extend_from_slice(record);
        bytes.resize(7 + CODEX_TAIL_BYTES, b' ');
        let tail = codex_tail(&mut Cursor::new(bytes.clone())).expect("boundary tail");
        assert_eq!(
            &tail[..record.len()],
            record,
            "a record starting at the boundary is whole"
        );
        bytes.push(b' ');
        let tail = codex_tail(&mut Cursor::new(bytes)).expect("cut tail");
        assert!(
            codex_rate_limits(&String::from_utf8_lossy(&tail)).is_none(),
            "a cut record is not accepted"
        );
    }
}
