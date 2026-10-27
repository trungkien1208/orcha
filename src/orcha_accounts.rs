//! orcha: agent accounts, their fuel (plan limits) and moving a chat
//! between them (docs/orcha/SPEC.md §3.2–3.3), natively. Ported from
//! orcha-config's `limits.py`, so FUEL, SWITCH, SIGN IN and NEW work
//! without Python, on Windows too. The files it shares with that script
//! (config.json accounts, the limits cache, claude-active, claude-slots,
//! claude-accounts/, selected-account) keep their formats, so both can run
//! side by side.
//!
//! Accounts come from `accounts` in orcha's config.json, plus Claude, Codex
//! and Cursor when they are installed but not listed.
//!
//! - Claude: Anthropic's usage endpoint, with the account's own sign-in
//!   (macOS Keychain, else `.credentials.json` in its config folder).
//! - Codex: the rate limits Codex writes into its newest session logs.
//! - Cursor: the usage summary of cursor.com, with the CLI's sign-in.
//!
//! Tokens only go to those two endpoints, on curl's stdin (never in argv,
//! never logged). Everything here blocks; the client calls it on
//! background threads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::platform::OrchaSecretSlot;

/// Fresh enough to show without asking again.
const CACHE_TTL_SECS: f64 = 120.0;
/// A request that failed (offline, a timeout): ask again after this.
const RETRY_AFTER_SECS: f64 = 300.0;
/// Rate limited (HTTP 429): wait longer.
const RATE_LIMIT_RETRY_AFTER_SECS: f64 = 900.0;
/// Another window holding the lock this long has died.
const STALE_LOCK_SECS: u64 = 60;

const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const CURSOR_USAGE_URL: &str = "https://cursor.com/api/usage-summary";

/// One plan window of an account, as FUEL shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FuelWindow {
    pub key: String,
    /// "5H", "WEEK", "MONTH".
    pub label: String,
    /// Percent left.
    pub left: u8,
    pub reset: Option<String>,
}

/// An account and its fuel, for the FUEL gauges and the accounts popup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Account {
    pub id: String,
    /// "claude", "codex", "cursor".
    pub kind: String,
    pub label: String,
    pub email: Option<String>,
    /// The account the focused agent runs on.
    #[serde(default)]
    pub current: bool,
    pub error: Option<String>,
    #[serde(default)]
    pub windows: Vec<FuelWindow>,
    /// There is a sign-in (its access token may have expired: the agent
    /// refreshes it). Missing from older commands.
    #[serde(default)]
    pub signed_in: Option<bool>,
}

/// An account as orcha's config.json lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AccountConfig {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub email: String,
    /// `CLAUDE_CONFIG_DIR` (expanded); None for the default `~/.claude`.
    pub dir: Option<String>,
}

/// An agent CLI orcha can list as an account. `kind` is herdr's agent id.
pub(crate) struct AgentKind {
    pub kind: &'static str,
    pub command: &'static str,
    pub name: &'static str,
    /// The shell line that signs in.
    login: &'static str,
    /// The shell line that continues the last chat, when the CLI has one.
    resume: Option<&'static str>,
    /// orcha reads its usage numbers (FUEL gauges).
    fuel: bool,
    /// Files under the home folder that exist once it is signed in; none
    /// known means orcha cannot tell.
    sign_in: &'static [&'static str],
}

const fn agent(
    kind: &'static str,
    command: &'static str,
    name: &'static str,
    login: &'static str,
    resume: Option<&'static str>,
    sign_in: &'static [&'static str],
) -> AgentKind {
    AgentKind {
        kind,
        command,
        name,
        login,
        resume,
        fuel: false,
        sign_in,
    }
}

pub(crate) const AGENTS: &[AgentKind] = &[
    AgentKind {
        fuel: true,
        ..agent(
            "claude",
            "claude",
            "Claude",
            "claude auth login",
            Some("claude --continue"),
            &[],
        )
    },
    AgentKind {
        fuel: true,
        ..agent(
            "codex",
            "codex",
            "Codex",
            "codex login",
            Some("codex resume --last"),
            &[".codex/auth.json"],
        )
    },
    AgentKind {
        fuel: true,
        ..agent(
            "cursor",
            "cursor-agent",
            "Cursor",
            "cursor-agent login",
            Some("cursor-agent resume"),
            &[".cursor/cli-config.json"],
        )
    },
    agent(
        "gemini",
        "gemini",
        "Gemini",
        "gemini",
        None,
        &[".gemini/oauth_creds.json", ".gemini/google_accounts.json"],
    ),
    agent(
        "copilot",
        "copilot",
        "Copilot",
        "copilot",
        Some("copilot --continue"),
        &[".copilot/config.json"],
    ),
    agent(
        "opencode",
        "opencode",
        "OpenCode",
        "opencode auth login",
        Some("opencode --continue"),
        &[".local/share/opencode/auth.json"],
    ),
    agent(
        "amp",
        "amp",
        "Amp",
        "amp login",
        None,
        &[".local/share/amp/secrets.json"],
    ),
    agent(
        "qwen",
        "qwen",
        "Qwen",
        "qwen",
        None,
        &[".qwen/oauth_creds.json"],
    ),
    agent("droid", "droid", "Droid", "droid", None, &[]),
    agent("kimi", "kimi", "Kimi", "kimi", None, &[]),
    agent("grok", "grok", "Grok", "grok", None, &[]),
    agent("kiro", "kiro-cli", "Kiro", "kiro-cli login", None, &[]),
    agent("devin", "devin", "Devin", "devin", None, &[]),
    agent("cline", "cline", "Cline", "cline", None, &[]),
    agent("kilo", "kilo", "Kilo", "kilo", None, &[]),
    agent("qodercli", "qodercli", "Qoder", "qodercli", None, &[]),
    agent("agy", "agy", "Antigravity", "agy", None, &[]),
    agent("pi", "pi", "Pi", "pi", None, &[]),
    agent("hermes", "hermes", "Hermes", "hermes", None, &[]),
    agent("letta", "letta", "Letta", "letta", None, &[]),
];

pub(crate) fn agent_kind(kind: &str) -> Option<&'static AgentKind> {
    AGENTS.iter().find(|agent| agent.kind == kind)
}

fn home() -> Option<PathBuf> {
    crate::platform::user_home_dir()
}

/// orcha's config folder (shared with orcha-config).
fn orcha_dir() -> Option<PathBuf> {
    Some(crate::platform::orcha_config_base_dir()?.join("orcha"))
}

fn cache_dir() -> Option<PathBuf> {
    Some(home()?.join(".cache").join("orcha").join("limits"))
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |since| since.as_secs_f64())
}

fn expand(path: &str) -> String {
    let Some(home) = home() else {
        return path.to_owned();
    };
    match path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        Some(rest) => home.join(rest).to_string_lossy().into_owned(),
        None if path == "~" => home.to_string_lossy().into_owned(),
        None => path.to_owned(),
    }
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Writes through a temporary file so a reader never sees half of it.
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("orcha-tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)
}

fn config_path() -> Option<PathBuf> {
    Some(orcha_dir()?.join("config.json"))
}

fn parse_accounts(config: &Value) -> Vec<AccountConfig> {
    config
        .get("accounts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|account| {
            let field = |key: &str| account.get(key).and_then(Value::as_str).unwrap_or("");
            let id = field("id");
            let kind = field("kind");
            if id.is_empty() || agent_kind(kind).is_none() {
                return None;
            }
            Some(AccountConfig {
                id: id.to_owned(),
                kind: kind.to_owned(),
                label: if field("label").is_empty() {
                    id.to_owned()
                } else {
                    field("label").to_owned()
                },
                email: field("email").to_owned(),
                dir: Some(field("dir")).filter(|dir| !dir.is_empty()).map(expand),
            })
        })
        .collect()
}

/// The signed-in email of a Claude config folder (`~/.claude.json` for the
/// default one).
fn claude_folder_email(dir: Option<&str>) -> Option<String> {
    let path = match dir {
        Some(dir) => Path::new(dir).join(".claude.json"),
        None => home()?.join(".claude.json"),
    };
    read_json(&path)?
        .get("oauthAccount")?
        .get("emailAddress")?
        .as_str()
        .map(str::to_owned)
}

/// The configured accounts, plus one for each of Claude, Codex and Cursor
/// installed but not listed yet.
pub(crate) fn accounts() -> Vec<AccountConfig> {
    let config = config_path().and_then(|path| read_json(&path));
    let mut out = config.as_ref().map(parse_accounts).unwrap_or_default();
    let hidden = hidden_accounts(config.as_ref());
    // the agents with fuel show by themselves; the others are offered
    // (`find_unlisted`)
    for AgentKind {
        kind,
        command,
        name,
        ..
    } in AGENTS.iter().filter(|agent| agent.fuel)
    {
        let (kind, command, name) = (*kind, *command, *name);
        let id = if kind == "claude" {
            "claude-main"
        } else {
            kind
        };
        if out.iter().any(|account| account.kind == kind)
            || hidden.iter().any(|known| known == id)
            || !crate::orcha_handoff::command_installed(command)
        {
            continue;
        }
        out.push(AccountConfig {
            id: if kind == "claude" {
                "claude-main".to_owned()
            } else {
                kind.to_owned()
            },
            kind: kind.to_owned(),
            label: name.to_owned(),
            email: if kind == "claude" {
                claude_folder_email(None).unwrap_or_default()
            } else {
                String::new()
            },
            dir: None,
        });
    }
    out
}

/// Detected accounts the user removed (`hidden_accounts` in config.json).
fn hidden_accounts(config: Option<&Value>) -> Vec<String> {
    config
        .and_then(|config| config.get("hidden_accounts"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|id| id.as_str().map(str::to_owned))
        .collect()
}

/// Takes `id` off orcha's list in config.json; a detected account (not in
/// the list) is hidden so it does not come back. Sign-ins and folders stay.
fn remove_from_config(config: &mut Value, id: &str) -> Result<(), String> {
    let object = config
        .as_object_mut()
        .ok_or("orcha's config.json is not an object")?;
    let mut listed = false;
    if let Some(list) = object.get_mut("accounts").and_then(Value::as_array_mut) {
        let before = list.len();
        list.retain(|account| account.get("id").and_then(Value::as_str) != Some(id));
        listed = list.len() != before;
    }
    if !listed {
        let hidden = object
            .entry("hidden_accounts")
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or("hidden_accounts in config.json is not a list")?;
        if !hidden.iter().any(|known| known.as_str() == Some(id)) {
            hidden.push(Value::String(id.to_owned()));
        }
    }
    Ok(())
}

/// Removes an account from orcha (its sign-in and folder are kept, so
/// adding it again later just works).
pub(crate) fn remove(id: &str) -> Result<(), String> {
    let path = config_path().ok_or("no config folder")?;
    let mut config = read_json(&path).unwrap_or_else(|| serde_json::json!({}));
    remove_from_config(&mut config, id)?;
    let text = serde_json::to_string_pretty(&config).map_err(|err| err.to_string())?;
    write_atomic(&path, &format!("{text}\n")).map_err(|err| format!("could not save: {err}"))?;
    // the default sign-in no longer belongs to a removed account
    if active_file()
        .and_then(|file| std::fs::read_to_string(file).ok())
        .is_some_and(|active| active.trim() == id)
    {
        if let Some(file) = active_file() {
            let _ = std::fs::remove_file(file);
        }
    }
    let mut slots = read_string_map(slots_file());
    slots.retain(|_, owner| owner != id);
    write_string_map(slots_file(), &slots);
    forget_limits(id);
    Ok(())
}

fn find<'a>(accounts: &'a [AccountConfig], id: &str) -> Option<&'a AccountConfig> {
    accounts.iter().find(|account| account.id == id)
}

// ---------- where each Claude sign-in lives
//
// Every Claude session reads the sign-in of its config folder. `activate`
// puts an account's sign-in into the default folder's, so running default
// sessions move with it, no restart. The sign-in it replaces (Claude may
// have refreshed it) is kept first under `Kept(<id>)`, so that account still
// works when it comes back. A pane started in an account's own folder reads
// that folder's sign-in; switching such a pane puts the new account's
// sign-in there, and claude-slots.json remembers which account each folder
// holds now ({service: account}).

fn folder_slot(account: &AccountConfig) -> OrchaSecretSlot {
    OrchaSecretSlot::Folder(account.dir.clone())
}

fn kept_slot(id: &str) -> OrchaSecretSlot {
    OrchaSecretSlot::Kept(id.to_owned())
}

fn active_file() -> Option<PathBuf> {
    Some(orcha_dir()?.join("claude-active"))
}

fn profile_dir() -> Option<PathBuf> {
    Some(orcha_dir()?.join("claude-accounts"))
}

fn slots_file() -> Option<PathBuf> {
    Some(orcha_dir()?.join("claude-slots.json"))
}

/// `{service: folder}` for the slots orcha filled, so hosts that keep
/// sign-ins in files can find the folder behind a service name.
fn slot_dirs_file() -> Option<PathBuf> {
    Some(orcha_dir()?.join("claude-slot-dirs.json"))
}

fn selected_file() -> Option<PathBuf> {
    Some(orcha_dir()?.join("selected-account"))
}

fn read_string_map(path: Option<PathBuf>) -> HashMap<String, String> {
    path.and_then(|path| read_json(&path))
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn write_string_map(path: Option<PathBuf>, map: &HashMap<String, String>) {
    let Some(path) = path else {
        return;
    };
    let text = serde_json::to_string(map).unwrap_or_else(|_| "{}".to_owned());
    if let Err(err) = write_atomic(&path, &text) {
        tracing::warn!(err = %err, "orcha: could not save {}", path.display());
    }
}

fn claude_accounts(accounts: &[AccountConfig]) -> impl Iterator<Item = &AccountConfig> {
    accounts.iter().filter(|account| account.kind == "claude")
}

/// The Claude account in the default sign-in: the one `activate` put
/// there, else the account that uses `~/.claude` itself.
fn active_claude(accounts: &[AccountConfig]) -> Option<String> {
    let chosen = active_file()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| text.trim().to_owned())
        .filter(|id| find(accounts, id).is_some());
    chosen.or_else(|| {
        claude_accounts(accounts)
            .find(|account| account.dir.is_none())
            .map(|account| account.id.clone())
    })
}

/// The slots orcha moved accounts into, dropping unknown accounts.
fn read_slots(accounts: &[AccountConfig]) -> HashMap<String, String> {
    let mut slots = read_string_map(slots_file());
    slots.retain(|_, id| find(accounts, id).is_some());
    slots
}

/// The account whose sign-in is in a folder's slot now.
fn slot_owner(accounts: &[AccountConfig], service: &str) -> Option<String> {
    read_slots(accounts).get(service).cloned().or_else(|| {
        claude_accounts(accounts)
            .find(|account| crate::platform::orcha_claude_service(&folder_slot(account)) == service)
            .map(|account| account.id.clone())
    })
}

/// A slot named by its service: the folder behind it when known.
fn service_slot(service: &str) -> OrchaSecretSlot {
    let dirs = read_string_map(slot_dirs_file());
    match dirs.get(service) {
        Some(dir) => OrchaSecretSlot::Folder(Some(dir.clone())),
        None => OrchaSecretSlot::Service(service.to_owned()),
    }
}

/// When a sign-in's access token runs out (ms since the epoch), 0 if unknown.
fn expires_at(secret: &str) -> f64 {
    serde_json::from_str::<Value>(secret)
        .ok()
        .and_then(|value| value.get("claudeAiOauth")?.get("expiresAt")?.as_f64())
        .unwrap_or(0.0)
}

/// Where an account's sign-in is now: the default folder when it is the
/// active account; else, of every place it may sit (folders it was switched
/// into, the copy orcha kept, its own folder), the one whose access token
/// runs out last. Old slot records can point at a stale copy, so the newest
/// wins over the record. The default folder belongs to the active account
/// only, so an inactive account never reads it.
fn claude_slot(accounts: &[AccountConfig], id: &str) -> Option<OrchaSecretSlot> {
    let account = find(accounts, id)?;
    if active_claude(accounts).as_deref() == Some(id) {
        return Some(OrchaSecretSlot::Folder(None));
    }
    let mut candidates: Vec<OrchaSecretSlot> = read_slots(accounts)
        .into_iter()
        .filter(|(_, owner)| owner == id)
        .map(|(service, _)| service_slot(&service))
        .collect();
    candidates.push(kept_slot(id));
    if account.dir.is_some() {
        candidates.push(folder_slot(account));
    }
    candidates
        .into_iter()
        .filter_map(|slot| {
            let secret = crate::platform::orcha_secret_read(&slot)?;
            Some((expires_at(&secret), slot))
        })
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, slot)| slot)
        // nothing anywhere yet: where a sign-in would be kept
        .or_else(|| Some(kept_slot(id)))
}

/// Before `id` signs in where Claude will write its sign-in: an account of
/// the default folder becomes the active one first (the sign-in there now,
/// another account's, is kept for it); an account whose folder another
/// account was switched into gets the folder back the same way.
fn prepare_sign_in(accounts: &[AccountConfig], id: &str) -> Result<(), String> {
    let account = find(accounts, id).ok_or("unknown account")?;
    if account.kind != "claude" {
        return Ok(());
    }
    let slot = folder_slot(account);
    if account.dir.is_none() {
        let current = active_claude(accounts);
        if current.as_deref() == Some(id) {
            return Ok(());
        }
        if let Some(current) = &current {
            keep_for(current, &slot)?;
        }
        if let Some(path) = active_file() {
            let _ = write_atomic(&path, &format!("{id}\n"));
        }
        return Ok(());
    }
    let service = crate::platform::orcha_claude_service(&slot);
    let mut slots = read_slots(accounts);
    if let Some(owner) = slots.get(&service).cloned().filter(|owner| owner != id) {
        keep_for(&owner, &slot)?;
        slots.remove(&service);
        write_string_map(slots_file(), &slots);
    }
    Ok(())
}

/// Keeps the sign-in in `slot` (and, for the default folder, its profile)
/// for `owner`, so that account still works after `slot` changes hands.
fn keep_for(owner: &str, slot: &OrchaSecretSlot) -> Result<(), String> {
    if let Some(live) = crate::platform::orcha_secret_read(slot) {
        crate::platform::orcha_secret_write(&kept_slot(owner), &live, slot)
            .map_err(|err| format!("could not keep {owner}'s sign-in: {err}"))?;
    }
    if *slot == OrchaSecretSlot::Folder(None) {
        let profile = home()
            .and_then(|home| read_json(&home.join(".claude.json")))
            .and_then(|state| state.get("oauthAccount").cloned());
        if let (Some(profile), Some(dir)) = (profile, profile_dir()) {
            let _ = write_atomic(&dir.join(format!("{owner}.json")), &profile.to_string());
        }
    }
    forget_limits(owner);
    Ok(())
}

/// An account's profile (`oauthAccount`: email etc.) as last seen.
fn claude_profile(accounts: &[AccountConfig], id: &str) -> Option<Value> {
    if let Some(profile) = profile_dir().and_then(|dir| read_json(&dir.join(format!("{id}.json"))))
    {
        return Some(profile);
    }
    let dir = find(accounts, id)?.dir.clone()?;
    read_json(&Path::new(&dir).join(".claude.json"))?
        .get("oauthAccount")
        .cloned()
}

// ---------- fuel

/// One account's plan limits: (percent used, reset time) per window.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Limits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session: Option<(f64, Option<f64>)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    weekly: Option<(f64, Option<f64>)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    monthly: Option<(f64, Option<f64>)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Why a fetch failed: worth retrying later, and rate limited.
#[derive(Debug, Default)]
struct Fetched {
    limits: Limits,
    transient: bool,
    rate_limited: bool,
    /// Nothing could be asked (the agent sits idle): the last numbers
    /// still hold, so keep showing them.
    idle: bool,
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

/// curl with its config (headers holding the token) on stdin. Ok is the
/// body; Err is (stderr, rate limited).
fn curl(url: &str, config: &str) -> Result<String, (String, bool)> {
    use std::io::Write;
    let mut command = Command::new("curl");
    command
        .args(["-sS", "-m", "15", "-f", "-K", "-", url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::platform::configure_background_command(&mut command);
    let mut child = command
        .spawn()
        .map_err(|err| (format!("curl: {err}"), false))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(config.as_bytes());
    }
    let output = child
        .wait_with_output()
        .map_err(|err| (format!("curl: {err}"), false))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let rate_limited = stderr.contains("error: 429") || stderr.contains(" 429");
    Err((stderr.chars().take(80).collect(), rate_limited))
}

/// Days since 1970-01-01 of a civil date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
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
fn iso_secs(text: &str) -> Option<f64> {
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
fn jwt_payload(token: &str) -> Option<Value> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn claude_limits(accounts: &[AccountConfig], id: &str) -> Fetched {
    let oauth = claude_slot(accounts, id)
        .and_then(|slot| crate::platform::orcha_secret_read(&slot))
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
        Err((message, rate_limited)) => {
            return Fetched::transient(format!("request failed ({message})"), rate_limited)
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
fn codex_rate_limits(text: &str) -> Option<Value> {
    let mut found = None;
    for line in text.lines().filter(|line| line.contains("\"rate_limits\"")) {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let limits = entry
            .get("payload")
            .and_then(|payload| payload.get("rate_limits"))
            .or_else(|| entry.get("rate_limits"));
        // like limits.py: a null or empty primary window is no data
        if let Some(limits) = limits.filter(|limits| {
            limits
                .get("primary")
                .and_then(Value::as_object)
                .is_some_and(|primary| !primary.is_empty())
        }) {
            found = Some(limits.clone());
        }
    }
    found
}

fn codex_window(limits: &Value, key: &str, now: f64) -> Option<(f64, Option<f64>)> {
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

fn codex_limits() -> Fetched {
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
    files.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    let now = now_secs();
    for (_, path) in files.into_iter().take(10) {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Some(limits) = codex_rate_limits(&String::from_utf8_lossy(&bytes)) else {
            continue;
        };
        return Fetched {
            limits: Limits {
                session: codex_window(&limits, "primary", now),
                weekly: codex_window(&limits, "secondary", now),
                note: Some("as of Codex's last run".to_owned()),
                ..Limits::default()
            },
            ..Fetched::default()
        };
    }
    Fetched::error("no Codex runs found yet")
}

fn cursor_limits() -> Fetched {
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
        Err(_) => return Fetched::error("Cursor sign-in expired: sign in again"),
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

fn fetch(accounts: &[AccountConfig], account: &AccountConfig) -> Fetched {
    match account.kind.as_str() {
        "claude" => claude_limits(accounts, &account.id),
        "codex" => codex_limits(),
        "cursor" => cursor_limits(),
        _ => Fetched::default(),
    }
}

fn modified_age(path: &Path) -> Option<f64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(
        SystemTime::now()
            .duration_since(modified)
            .map_or(0.0, |age| age.as_secs_f64()),
    )
}

/// One request at a time across orcha windows: a lock file, dropped when
/// done (a dead holder's lock goes stale).
struct FetchLock(PathBuf);

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
fn limits(accounts: &[AccountConfig], account: &AccountConfig, fresh: bool) -> Limits {
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
fn forget_limits(id: &str) {
    if let Some(dir) = cache_dir() {
        let _ = std::fs::remove_file(dir.join(format!("{id}.json")));
    }
}

/// "2d 3h", "1h 05m", "12m", "now".
fn until(reset: f64, now: f64) -> String {
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

fn windows(limits: &Limits, now: f64) -> Vec<FuelWindow> {
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
        })
    })
    .collect()
}

/// The signed-in email of an account, or None. Only the email is read.
fn email(accounts: &[AccountConfig], account: &AccountConfig) -> Option<String> {
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
fn agent_email(home: &Path, kind: &str) -> Option<String> {
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
fn agent_signed_in(home: &Path, kind: &str) -> bool {
    agent_kind(kind).is_none_or(|agent| {
        agent.sign_in.is_empty() || agent.sign_in.iter().any(|path| home.join(path).exists())
    })
}

/// There is a sign-in for this account (maybe with an expired token).
fn signed_in(accounts: &[AccountConfig], account: &AccountConfig) -> bool {
    match account.kind.as_str() {
        "claude" => claude_slot(accounts, &account.id)
            .and_then(|slot| crate::platform::orcha_secret_read(&slot))
            .is_some(),
        "cursor" => email(accounts, account).is_some(),
        kind => home().is_some_and(|home| agent_signed_in(&home, kind)),
    }
}

fn auth_needed(error: Option<&str>) -> bool {
    error.is_some_and(|error| {
        let error = error.to_lowercase();
        ["sign-in", "sign in", "signed in"]
            .iter()
            .any(|word| error.contains(word))
    })
}

fn selected(accounts: &[AccountConfig]) -> Option<String> {
    let chosen = selected_file()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| text.trim().to_owned())
        .filter(|id| find(accounts, id).is_some());
    chosen.or_else(|| accounts.first().map(|account| account.id.clone()))
}

fn select(id: &str) {
    if let Some(path) = selected_file() {
        let _ = write_atomic(&path, &format!("{id}\n"));
    }
}

// ---------- panes

/// A pane's `herdr pane get` info.
fn pane_info(pane: &str) -> Option<Value> {
    crate::orcha_handoff::herdr_json(&["pane", "get", pane])?
        .get_mut("pane")
        .map(Value::take)
}

/// `CLAUDE_CONFIG_DIR` of the Claude running in a pane, as Claude sees it;
/// None for the default `~/.claude` (or when the host cannot tell).
fn pane_config_dir(pane: &str) -> Option<String> {
    let info = crate::orcha_handoff::herdr_json(&["pane", "process-info", "--pane", pane])?;
    info.get("process_info")?
        .get("foreground_processes")?
        .as_array()?
        .iter()
        .filter_map(|process| process.get("pid")?.as_u64())
        .find_map(|pid| crate::platform::orcha_process_env_var(pid as u32, "CLAUDE_CONFIG_DIR"))
        .filter(|dir| !dir.is_empty())
}

/// The account the agent in `pane` runs on.
fn pane_account(accounts: &[AccountConfig], pane: &str) -> Option<String> {
    let info = pane_info(pane)?;
    let kind = info.get("agent").and_then(Value::as_str)?;
    if kind != "claude" {
        return accounts
            .iter()
            .find(|account| account.kind == kind)
            .map(|account| account.id.clone());
    }
    let Some(dir) = pane_config_dir(pane) else {
        return active_claude(accounts);
    };
    let service =
        crate::platform::orcha_claude_service(&OrchaSecretSlot::Folder(Some(dir.clone())));
    if let Some(owner) = slot_owner(accounts, &service) {
        return Some(owner);
    }
    let found = std::fs::canonicalize(expand(&dir)).ok();
    claude_accounts(accounts)
        .find(|account| {
            account
                .dir
                .as_deref()
                .and_then(|own| std::fs::canonicalize(own).ok())
                .is_some_and(|own| Some(own) == found)
        })
        .map(|account| account.id.clone())
}

/// Every account and its fuel; `current` is `pane`'s account, else the
/// one picked last.
pub(crate) fn accounts_with_fuel(pane: Option<&str>, fresh: bool) -> Vec<Account> {
    let accounts = accounts();
    let current = pane
        .and_then(|pane| pane_account(&accounts, pane))
        .or_else(|| selected(&accounts));
    let now = now_secs();
    accounts
        .iter()
        .map(|account| {
            let fuel = agent_kind(&account.kind).is_some_and(|agent| agent.fuel);
            let limits = if fuel {
                limits(&accounts, account, fresh)
            } else {
                Limits::default()
            };
            Account {
                id: account.id.clone(),
                kind: account.kind.clone(),
                label: account.label.clone(),
                email: email(&accounts, account),
                current: current.as_deref() == Some(account.id.as_str()),
                windows: windows(&limits, now),
                signed_in: Some(
                    signed_in(&accounts, account) && !auth_needed(limits.error.as_deref()),
                ),
                error: limits.error,
            }
        })
        .collect()
}

// ---------- actions

/// Makes running Claude sessions of a folder (None: `~/.claude`) read
/// their sign-in again. With a `.credentials.json`, Claude only watches
/// that file's modified time, so touching it (content untouched) makes it
/// drop the token it holds and read the sign-in again.
fn nudge_claude(dir: Option<&str>) {
    let folder = match dir {
        Some(dir) => PathBuf::from(expand(dir)),
        None => match home() {
            Some(home) => home.join(".claude"),
            None => return,
        },
    };
    let path = folder.join(".credentials.json");
    if let Ok(file) = std::fs::File::options().append(true).open(&path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

fn set_profile(state_path: &Path, profile: &Value) {
    let Some(mut state) = read_json(state_path) else {
        return;
    };
    let Some(object) = state.as_object_mut() else {
        return;
    };
    object.insert("oauthAccount".to_owned(), profile.clone());
    if let Ok(text) = serde_json::to_string_pretty(&state) {
        let _ = write_atomic(state_path, &text);
    }
}

/// Every default Claude session moves to `id` (no restart).
pub(crate) fn activate(id: &str) -> Result<(), String> {
    let accounts = accounts();
    let account = find(&accounts, id).ok_or("unknown account")?;
    if account.kind != "claude" {
        return Err("not a Claude account".to_owned());
    }
    let current = active_claude(&accounts);
    if current.as_deref() == Some(id) {
        select(id);
        return Ok(());
    }
    let source = crate::platform::orcha_secret_read(&kept_slot(id))
        .or_else(|| crate::platform::orcha_secret_read(&folder_slot(account)))
        .ok_or("no sign-in found: sign in")?;
    let default = OrchaSecretSlot::Folder(None);
    let home = home().ok_or("no home folder")?;
    let claude_json = home.join(".claude.json");
    if let Some(current) = &current {
        // keep the live sign-in (maybe refreshed since) and its profile first
        if let Some(live) = crate::platform::orcha_secret_read(&default) {
            crate::platform::orcha_secret_write(&kept_slot(current), &live, &default)
                .map_err(|err| format!("could not keep the current sign-in: {err}"))?;
        }
        if let (Some(profile), Some(dir)) = (
            read_json(&claude_json).and_then(|state| state.get("oauthAccount").cloned()),
            profile_dir(),
        ) {
            let _ = write_atomic(&dir.join(format!("{current}.json")), &profile.to_string());
        }
    }
    crate::platform::orcha_secret_write(&default, &source, &default)
        .map_err(|err| format!("could not switch the sign-in: {err}"))?;
    if let Some(profile) = claude_profile(&accounts, id) {
        set_profile(&claude_json, &profile);
    }
    if let Some(path) = active_file() {
        let _ = write_atomic(&path, &format!("{id}\n"));
    }
    forget_limits(id);
    select(id);
    nudge_claude(None);
    Ok(())
}

/// Moves a running Claude onto `id` without a restart: the account's
/// sign-in goes into the one that pane's Claude reads. Claude reads it
/// again within about 30 seconds and the chat goes on.
fn switch_claude_pane(accounts: &[AccountConfig], id: &str, pane: &str) -> Result<(), String> {
    let Some(dir) = pane_config_dir(pane) else {
        return activate(id);
    };
    let slot = OrchaSecretSlot::Folder(Some(dir.clone()));
    let service = crate::platform::orcha_claude_service(&slot);
    let owner = slot_owner(accounts, &service);
    if owner.as_deref() == Some(id) {
        select(id);
        return Ok(());
    }
    let source = claude_slot(accounts, id)
        .and_then(|from| crate::platform::orcha_secret_read(&from))
        .ok_or("no sign-in found: sign in")?;
    if let (Some(owner), Some(live)) = (&owner, crate::platform::orcha_secret_read(&slot)) {
        // keep the sign-in being replaced (maybe refreshed since) for its account
        crate::platform::orcha_secret_write(&kept_slot(owner), &live, &slot)
            .map_err(|err| format!("could not keep the current sign-in: {err}"))?;
    }
    crate::platform::orcha_secret_write(&slot, &source, &slot)
        .map_err(|err| format!("could not switch the sign-in: {err}"))?;
    let mut slots = read_slots(accounts);
    let native = claude_accounts(accounts)
        .find(|account| crate::platform::orcha_claude_service(&folder_slot(account)) == service)
        .map(|account| account.id.clone());
    if native.as_deref() == Some(id) {
        slots.remove(&service);
    } else {
        slots.insert(service.clone(), id.to_owned());
    }
    write_string_map(slots_file(), &slots);
    let mut dirs = read_string_map(slot_dirs_file());
    dirs.insert(service, dir.clone());
    write_string_map(slot_dirs_file(), &dirs);
    // the folder's profile shows who is signed in
    if let Some(profile) = claude_profile(accounts, id) {
        set_profile(&Path::new(&expand(&dir)).join(".claude.json"), &profile);
    }
    forget_limits(id);
    nudge_claude(Some(&dir));
    select(id);
    Ok(())
}

fn run_in_pane(pane: &str, line: &str) -> Result<(), String> {
    let ran = crate::orcha_handoff::herdr(&["pane", "run", pane, line])
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if ran {
        Ok(())
    } else {
        Err("could not run it in the pane".to_owned())
    }
}

fn send_keys(pane: &str, keys: &str) {
    let _ = crate::orcha_handoff::herdr(&["pane", "send-keys", pane, keys])
        .stdout(Stdio::null())
        .status();
}

/// Continues `pane`'s conversation on another account. Claude: the running
/// session moves without a restart. Others: stop the agent and resume the
/// same conversation there.
pub(crate) fn switch(id: &str, pane: &str) -> Result<(), String> {
    let accounts = accounts();
    let account = find(&accounts, id).ok_or("unknown account")?;
    if account.kind == "claude" {
        return switch_claude_pane(&accounts, id, pane);
    }
    let session = pane_info(pane).and_then(|info| {
        info.get("agent_session")?
            .get("value")?
            .as_str()
            .map(str::to_owned)
    });
    let line = match (account.kind.as_str(), session) {
        ("codex", Some(session)) => format!("codex resume {session}"),
        (kind, _) => agent_kind(kind).map_or_else(
            || kind.to_owned(),
            |agent| agent.resume.unwrap_or(agent.command).to_owned(),
        ),
    };
    // Ctrl+C twice leaves the agent's CLI; then the shell resumes
    for _ in 0..2 {
        send_keys(pane, "ctrl+c");
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
    std::thread::sleep(std::time::Duration::from_secs(1));
    run_in_pane(pane, &line)?;
    select(id);
    Ok(())
}

/// An extra Claude account's folder: everything of `~/.claude` linked in
/// (history, settings, skills...) except the sign-in files, so accounts
/// share their conversations.
fn link_shared(target: &str) {
    let Some(home) = home() else {
        return;
    };
    let source = home.join(".claude");
    let target = PathBuf::from(expand(target));
    let _ = std::fs::create_dir_all(&target);
    const PRIVATE: [&str; 7] = [
        ".claude.json",
        ".credentials.json",
        "daemon",
        "daemon.log",
        "daemon-auth-cooldown",
        "daemon-auth-status.json",
        "backups",
    ];
    let Ok(entries) = std::fs::read_dir(&source) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if PRIVATE.contains(&name.as_str())
            || name.starts_with("policy-limits.json")
            || name.starts_with("remote-settings.json")
            || name.starts_with("..")
            || name.contains(".tmp")
        {
            continue;
        }
        let link = target.join(&name);
        if std::fs::symlink_metadata(&link).is_err() {
            let _ = crate::platform::orcha_link_path(&entry.path(), &link);
        }
    }
}

/// A new pane right of `pane` (or of the focused one), in `cwd`, with an
/// account's config folder in its environment.
fn split_for(
    account: &AccountConfig,
    pane: Option<&str>,
    cwd: Option<&str>,
) -> Result<String, String> {
    let mut args = vec!["pane", "split", "--direction", "right"];
    match pane {
        Some(pane) => args.extend(["--pane", pane]),
        None => args.push("--current"),
    }
    if let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) {
        args.extend(["--cwd", cwd]);
    }
    let env;
    if let Some(dir) = account.dir.as_deref().filter(|_| account.kind == "claude") {
        link_shared(dir);
        env = format!("CLAUDE_CONFIG_DIR={dir}");
        args.extend(["--env", &env]);
    }
    crate::orcha_handoff::herdr_json(&args)
        .and_then(|result| {
            result
                .get("pane")?
                .get("pane_id")?
                .as_str()
                .map(str::to_owned)
        })
        .ok_or_else(|| "could not open a pane".to_owned())
}

/// The agent's command continuing its last chat, else a new one.
fn continue_line(kind: &str) -> String {
    let Some(agent) = agent_kind(kind) else {
        return kind.to_owned();
    };
    // PowerShell 5 has no `||`; there the agent starts fresh if it must
    match (agent.resume, cfg!(windows)) {
        (Some(resume), false) => format!("{resume} || {}", agent.command),
        (Some(resume), true) => resume.to_owned(),
        (None, _) => agent.command.to_owned(),
    }
}

/// Opens an account in a new pane next to `pane`, continuing its last chat.
pub(crate) fn open(id: &str, pane: Option<&str>, cwd: Option<&str>) -> Result<(), String> {
    let accounts = accounts();
    let account = find(&accounts, id).ok_or("unknown account")?;
    let new_pane = split_for(account, pane, cwd)?;
    run_in_pane(&new_pane, &continue_line(&account.kind))
}

/// An email safe to put on a command line as it is.
fn plain_email(email: &str) -> Option<&str> {
    let email = email.trim();
    (!email.is_empty()
        && email
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "@._+-".contains(ch)))
    .then_some(email)
}

/// Signs an account in, in a new pane next to `pane`.
pub(crate) fn login(id: &str, pane: Option<&str>) -> Result<(), String> {
    let accounts = accounts();
    let account = find(&accounts, id).ok_or("unknown account")?;
    // Claude writes the sign-in where this account will look for it
    prepare_sign_in(&accounts, id)?;
    let new_pane = split_for(account, pane, None)?;
    let line = match account.kind.as_str() {
        "claude" => match plain_email(&account.email) {
            Some(email) => format!("claude auth login --email {email}"),
            None => "claude auth login".to_owned(),
        },
        kind => agent_kind(kind)
            .map_or(kind, |agent| agent.login)
            .to_owned(),
    };
    forget_limits(id);
    run_in_pane(&new_pane, &line)
}

/// "Personal Mac" → "personal-mac".
fn account_slug(name: &str) -> String {
    let mut slug = String::new();
    for ch in name.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            slug.push(ch);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_owned()
}

/// Adds the Claude account `name` to config.json (other settings kept).
fn add_to_config(config: &mut Value, name: &str) -> Result<String, String> {
    let slug = account_slug(name);
    if slug.is_empty() {
        return Err("need a name like personal or work".to_owned());
    }
    let id = format!("claude-{slug}");
    let object = config
        .as_object_mut()
        .ok_or("orcha's config.json is not an object")?;
    let list = object
        .entry("accounts")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or("accounts in config.json is not a list")?;
    let taken = list.iter().any(|account| {
        let known = account.get("id").and_then(Value::as_str).unwrap_or("");
        known == id || known.strip_prefix("claude-") == Some(slug.as_str())
    });
    if taken {
        return Err(format!("{id} is already an account"));
    }
    list.push(serde_json::json!({
        "id": id,
        "kind": "claude",
        "label": format!("Claude · {slug}"),
        "email": "",
        "dir": format!("~/.ai-accounts/claude/{slug}"),
    }));
    Ok(id)
}

/// Adds a Claude account and signs it in, in a new pane next to `pane`.
pub(crate) fn add(name: &str, pane: Option<&str>) -> Result<String, String> {
    let path = config_path().ok_or("no config folder")?;
    let mut config = read_json(&path).unwrap_or_else(|| serde_json::json!({}));
    // a first run lists the detected accounts too, so they stay
    seed_detected(&mut config);
    let id = add_to_config(&mut config, name)?;
    let text = serde_json::to_string_pretty(&config).map_err(|err| err.to_string())?;
    write_atomic(&path, &format!("{text}\n")).map_err(|err| format!("could not save: {err}"))?;
    login(&id, pane)?;
    Ok(id)
}

/// An agent sign-in on this machine that orcha does not list yet: a
/// Claude config folder, or another agent's CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FoundAccount {
    /// herdr's agent id (`AGENTS`).
    pub kind: String,
    /// Claude's `CLAUDE_CONFIG_DIR`; None for the default folder.
    pub dir: Option<String>,
    /// Who it is signed in as; empty when the CLI does not say.
    pub email: String,
    /// The name it gets (Claude: `claude-<name>`).
    pub name: String,
    /// orcha saw its sign-in (false: installed, sign in after importing).
    pub signed_in: bool,
}

impl FoundAccount {
    /// How config.json remembers a dismissed one.
    fn key(&self) -> String {
        if self.kind != "claude" {
            return format!("agent:{}", self.kind);
        }
        self.dir.clone().unwrap_or_else(|| "default".to_owned())
    }
}

/// Folders where Claude sign-ins usually live: orcha's own, `~/.claude-*`
/// and `CLAUDE_CONFIG_DIR`.
fn claude_folder_candidates() -> Vec<PathBuf> {
    let mut folders = Vec::new();
    let Some(home) = home() else {
        return folders;
    };
    let mut push_children = |parent: &Path, prefix: &str| {
        let Ok(read) = std::fs::read_dir(parent) else {
            return;
        };
        let mut found: Vec<PathBuf> = read
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(prefix))
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        found.sort();
        folders.extend(found);
    };
    push_children(&home.join(".ai-accounts").join("claude"), "");
    push_children(&home, ".claude-");
    push_children(&home, ".claude_");
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !dir.trim().is_empty() {
            folders.push(PathBuf::from(expand(dir.trim())));
        }
    }
    folders
}

fn same_folder(a: &str, b: &str) -> bool {
    let canonical =
        |path: &str| std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
    a == b || canonical(a) == canonical(b)
}

/// The name a found folder gets: its folder name without `.claude-`.
fn found_name(dir: Option<&str>, email: &str) -> String {
    let from_folder = dir
        .and_then(|dir| Path::new(dir).file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .map(|name| {
            name.trim_start_matches(".claude-")
                .trim_start_matches(".claude_")
                .trim_start_matches('.')
                .to_owned()
        })
        .filter(|name| !account_slug(name).is_empty() && name != "claude");
    from_folder.unwrap_or_else(|| {
        email
            .split('@')
            .next()
            .filter(|user| !account_slug(user).is_empty())
            .unwrap_or("main")
            .to_owned()
    })
}

/// Signed-in Claude folders and installed agents orcha does not list, in
/// the order found. `agents` are (kind, email, signed in) of the agent CLIs
/// installed. `with_dismissed` also returns the ones the user said no to.
fn find_unlisted_in(
    config: Option<&Value>,
    candidates: &[PathBuf],
    default_email: Option<String>,
    agents: &[(&str, Option<String>, bool)],
    with_dismissed: bool,
) -> Vec<FoundAccount> {
    let listed = config.map(parse_accounts).unwrap_or_default();
    let dismissed: Vec<String> = if with_dismissed {
        Vec::new()
    } else {
        config
            .and_then(|config| config.get("dismissed_account_dirs"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|dir| dir.as_str().map(str::to_owned))
            .collect()
    };
    let mut found: Vec<FoundAccount> = Vec::new();
    // the default folder, when the accounts orcha lists all live elsewhere
    // (with none listed, `accounts` shows it already)
    let default_listed = listed
        .iter()
        .any(|account| account.kind == "claude" && account.dir.is_none());
    let any_claude = listed.iter().any(|account| account.kind == "claude");
    if let Some(email) = default_email.filter(|_| any_claude && !default_listed) {
        found.push(FoundAccount {
            kind: "claude".to_owned(),
            dir: None,
            name: found_name(None, &email),
            email,
            signed_in: true,
        });
    }
    for folder in candidates {
        let dir = folder.to_string_lossy().into_owned();
        let Some(email) = claude_folder_email(Some(&dir)) else {
            continue;
        };
        let known = listed.iter().any(|account| {
            account
                .dir
                .as_deref()
                .is_some_and(|listed_dir| same_folder(listed_dir, &dir))
        }) || found.iter().any(|other| {
            other
                .dir
                .as_deref()
                .is_some_and(|other| same_folder(other, &dir))
        });
        if known {
            continue;
        }
        found.push(FoundAccount {
            kind: "claude".to_owned(),
            name: found_name(Some(&dir), &email),
            dir: Some(dir),
            email,
            signed_in: true,
        });
    }
    // agents with fuel show by themselves (`accounts`); the others once
    // the user says so. Ones taken off the list stay off.
    let hidden = hidden_accounts(config);
    for (kind, email, signed_in) in agents {
        let Some(agent) = agent_kind(kind).filter(|agent| !agent.fuel) else {
            continue;
        };
        if listed.iter().any(|account| account.kind == *kind) || hidden.iter().any(|id| id == kind)
        {
            continue;
        }
        found.push(FoundAccount {
            kind: (*kind).to_owned(),
            dir: None,
            email: email.clone().unwrap_or_default(),
            name: agent.name.to_owned(),
            signed_in: *signed_in,
        });
    }
    found.retain(|account| {
        !dismissed
            .iter()
            .any(|key| *key == account.key() || same_folder(key, &account.key()))
    });
    found
}

/// Signed-in Claude folders and installed agents orcha does not list yet.
/// `asked`: from the
/// menu, which also brings back dismissed ones; the look orcha makes by
/// itself only runs once there is an accounts list to add to (before that,
/// `accounts` shows the default sign-ins).
pub(crate) fn find_unlisted(asked: bool) -> Vec<FoundAccount> {
    let config = config_path().and_then(|path| read_json(&path));
    if !asked
        && config
            .as_ref()
            .is_none_or(|config| config.get("accounts").is_none())
    {
        return Vec::new();
    }
    let agents: Vec<(&str, Option<String>, bool)> = match home() {
        Some(home) => AGENTS
            .iter()
            .filter(|agent| !agent.fuel && crate::orcha_handoff::command_installed(agent.command))
            .map(|agent| {
                (
                    agent.kind,
                    agent_email(&home, agent.kind),
                    agent_signed_in(&home, agent.kind),
                )
            })
            .collect(),
        None => Vec::new(),
    };
    find_unlisted_in(
        config.as_ref(),
        &claude_folder_candidates(),
        claude_folder_email(None),
        &agents,
        asked,
    )
}

/// Lists the detected accounts in config.json the first time it changes,
/// so they stay.
fn seed_detected(config: &mut Value) {
    if config.get("accounts").is_some() {
        return;
    }
    let detected: Vec<Value> = accounts()
        .into_iter()
        .map(|account| {
            serde_json::json!({
                "id": account.id,
                "kind": account.kind,
                "label": account.label,
                "email": account.email,
                "dir": account.dir,
            })
        })
        .collect();
    if let Some(object) = config.as_object_mut() {
        object.insert("accounts".to_owned(), Value::Array(detected));
    }
}

/// Adds found accounts to config.json (their sign-ins are used as they
/// are) and drops the others from asking again. Returns the ids added.
fn import_into_config(
    config: &mut Value,
    chosen: &[FoundAccount],
    dismissed: &[FoundAccount],
) -> Result<Vec<String>, String> {
    let object = config
        .as_object_mut()
        .ok_or("orcha's config.json is not an object")?;
    let list = object
        .entry("accounts")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or("accounts in config.json is not a list")?;
    let mut added = Vec::new();
    for account in chosen {
        let taken = |id: &str| {
            list.iter()
                .any(|known| known.get("id").and_then(Value::as_str) == Some(id))
        };
        // Claude accounts are claude-<name>; another agent is its kind
        let (prefix, base) = if account.kind == "claude" {
            let base = account_slug(&account.name);
            (
                "claude-",
                if base.is_empty() {
                    "main".to_owned()
                } else {
                    base
                },
            )
        } else {
            ("", account.kind.clone())
        };
        let mut slug = base.clone();
        let mut n = 2;
        while taken(&format!("{prefix}{slug}")) {
            slug = format!("{base}{n}");
            n += 1;
        }
        let id = format!("{prefix}{slug}");
        let label = if account.kind == "claude" {
            format!("Claude · {slug}")
        } else {
            account.name.clone()
        };
        list.push(serde_json::json!({
            "id": id,
            "kind": account.kind,
            "label": label,
            "email": account.email,
            "dir": account.dir,
        }));
        added.push(id);
    }
    if !dismissed.is_empty() {
        let keys = object
            .entry("dismissed_account_dirs")
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or("dismissed_account_dirs in config.json is not a list")?;
        for account in dismissed {
            let key = account.key();
            if !keys
                .iter()
                .any(|known| known.as_str() == Some(key.as_str()))
            {
                keys.push(Value::String(key));
            }
        }
    }
    Ok(added)
}

/// Imports the `chosen` found accounts; the `dismissed` ones are not
/// offered again unless asked for.
pub(crate) fn import_found(
    chosen: &[FoundAccount],
    dismissed: &[FoundAccount],
) -> Result<Vec<String>, String> {
    let path = config_path().ok_or("no config folder")?;
    let mut config = read_json(&path).unwrap_or_else(|| serde_json::json!({}));
    seed_detected(&mut config);
    let added = import_into_config(&mut config, chosen, dismissed)?;
    let text = serde_json::to_string_pretty(&config).map_err(|err| err.to_string())?;
    write_atomic(&path, &format!("{text}\n")).map_err(|err| format!("could not save: {err}"))?;
    Ok(added)
}

/// NEW: a tab in `workspace` running the account's agent.
pub(crate) fn new_agent(id: &str, workspace: &str, cwd: &str) -> Result<(), String> {
    let accounts = accounts();
    let account = find(&accounts, id).ok_or("unknown account")?;
    if account.kind == "claude" {
        // one Claude sign-in for every default session: this account becomes it
        activate(id)?;
    }
    let mut args = vec![
        "tab",
        "create",
        "--workspace",
        workspace,
        "--label",
        &account.label,
        "--focus",
    ];
    if !cwd.is_empty() {
        args.extend(["--cwd", cwd]);
    }
    let result = crate::orcha_handoff::herdr_json(&args).ok_or("could not open a tab")?;
    let pane = first_pane_id(&result).ok_or("could not open a tab")?;
    let command = agent_kind(&account.kind).map_or("claude", |agent| agent.command);
    run_in_pane(&pane, command)?;
    select(id);
    Ok(())
}

/// The first pane id anywhere in a CLI result.
fn first_pane_id(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => map
            .get("pane_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| map.values().find_map(first_pane_id)),
        Value::Array(items) => items.iter().find_map(first_pane_id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_in_folder(root: &Path, name: &str, email: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).expect("create folder");
        std::fs::write(
            dir.join(".claude.json"),
            serde_json::json!({"oauthAccount": {"emailAddress": email}}).to_string(),
        )
        .expect("write sign-in");
        dir
    }

    #[test]
    fn found_accounts_skip_listed_dismissed_and_signed_out_folders() {
        let root = std::env::temp_dir().join(format!("orcha-found-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let listed = signed_in_folder(&root, ".claude-listed", "a@x.io");
        let work = signed_in_folder(&root, ".claude-work", "w@x.io");
        let gone = signed_in_folder(&root, ".claude-gone", "g@x.io");
        let empty = root.join(".claude-empty");
        std::fs::create_dir_all(&empty).expect("create empty");
        let config = serde_json::json!({
            "accounts": [{"id": "claude-a", "kind": "claude", "dir": listed.to_string_lossy()}],
            "dismissed_account_dirs": [gone.to_string_lossy()],
        });
        let candidates = [listed, work.clone(), gone.clone(), empty];
        let found = find_unlisted_in(
            Some(&config),
            &candidates,
            Some("me@x.io".into()),
            &[],
            false,
        );
        assert_eq!(
            found,
            vec![
                FoundAccount {
                    kind: "claude".into(),
                    dir: None,
                    email: "me@x.io".into(),
                    name: "me".into(),
                    signed_in: true,
                },
                FoundAccount {
                    kind: "claude".into(),
                    dir: Some(work.to_string_lossy().into_owned()),
                    email: "w@x.io".into(),
                    name: "work".into(),
                    signed_in: true,
                },
            ]
        );
        let all = find_unlisted_in(Some(&config), &candidates, None, &[], true);
        assert_eq!(all.len(), 2, "asking again brings dismissed ones back");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn importing_picks_free_ids_and_remembers_the_rest() {
        let mut config = serde_json::json!({"accounts": [
            {"id": "claude-work", "kind": "claude", "dir": null},
        ]});
        let claude = |dir: &str, name: &str| FoundAccount {
            kind: "claude".into(),
            dir: Some(dir.into()),
            email: format!("{name}@x.io"),
            name: name.into(),
            signed_in: true,
        };
        let gemini = FoundAccount {
            kind: "gemini".into(),
            dir: None,
            email: String::new(),
            name: "Gemini".into(),
            signed_in: false,
        };
        let added = import_into_config(
            &mut config,
            &[claude("/w", "work"), gemini],
            &[claude("/s", "side")],
        )
        .expect("import");
        assert_eq!(added, ["claude-work2", "gemini"]);
        let accounts = parse_accounts(&config);
        assert_eq!(accounts[1].dir.as_deref(), Some("/w"));
        assert_eq!(accounts[1].email, "work@x.io");
        assert_eq!(
            (accounts[2].kind.as_str(), accounts[2].label.as_str()),
            ("gemini", "Gemini")
        );
        assert_eq!(config["dismissed_account_dirs"], serde_json::json!(["/s"]));
    }

    #[test]
    fn other_agents_are_offered_once_unless_listed_hidden_or_dismissed() {
        let config = serde_json::json!({
            "accounts": [{"id": "amp", "kind": "amp"}],
            "hidden_accounts": ["qwen"],
            "dismissed_account_dirs": ["agent:kimi"],
        });
        let agents = [
            ("gemini", Some("g@x.io".to_owned()), true),
            ("amp", None, true),
            ("qwen", None, true),
            ("kimi", None, false),
            ("codex", None, true),
            ("opencode", None, false),
        ];
        let found = find_unlisted_in(Some(&config), &[], None, &agents, false);
        let kinds: Vec<_> = found
            .iter()
            .map(|account| (account.kind.as_str(), account.signed_in))
            .collect();
        assert_eq!(kinds, [("gemini", true), ("opencode", false)]);
        assert_eq!(found[0].email, "g@x.io");
        assert_eq!(found[0].key(), "agent:gemini");
        let again = find_unlisted_in(Some(&config), &[], None, &agents, true);
        assert!(again.iter().any(|account| account.kind == "kimi"));
    }

    #[test]
    fn every_agent_has_a_command_and_a_continue_line() {
        for agent in AGENTS {
            assert!(!agent.command.is_empty() && !agent.login.is_empty());
            assert!(continue_line(agent.kind).contains(agent.command));
        }
    }

    #[test]
    fn config_accounts_keep_known_kinds_and_expand_folders() {
        let config = serde_json::json!({"accounts": [
            {"id": "claude-work", "kind": "claude", "label": "Claude · work", "email": "a@b.c", "dir": null},
            {"id": "claude-me", "kind": "claude", "label": "", "dir": "/x/me"},
            {"id": "odd", "kind": "not-an-agent"},
            {"kind": "codex"},
        ]});
        let accounts = parse_accounts(&config);
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].dir, None);
        assert_eq!(accounts[0].email, "a@b.c");
        assert_eq!(accounts[1].label, "claude-me");
        assert_eq!(accounts[1].dir.as_deref(), Some("/x/me"));
    }

    #[test]
    fn iso_times_with_zones_and_fractions() {
        assert_eq!(iso_secs("1970-01-01T00:00:00Z"), Some(0.0));
        assert_eq!(iso_secs("2026-10-23T12:00:00Z"), Some(1_792_756_800.0));
        assert_eq!(
            iso_secs("2026-10-23T19:00:00+07:00"),
            iso_secs("2026-10-23T12:00:00Z")
        );
        assert_eq!(
            iso_secs("2026-10-23T12:00:00.500000Z"),
            Some(1_792_756_800.5)
        );
        assert_eq!(iso_secs("2026-10-23T12:00:00"), Some(1_792_756_800.0));
        assert_eq!(iso_secs("not a time"), None);
    }

    #[test]
    fn reset_times_read_short() {
        assert_eq!(until(100.0, 200.0), "now");
        assert_eq!(until(200.0 + 12.0 * 60.0, 200.0), "12m");
        assert_eq!(until(3_900.0, 0.0), "1h 05m");
        assert_eq!(until(2.0 * 86_400.0 + 3.0 * 3600.0, 0.0), "2d 3h");
    }

    #[test]
    fn windows_show_percent_left() {
        let limits = Limits {
            session: Some((30.4, Some(3_600.0))),
            weekly: Some((120.0, None)),
            ..Limits::default()
        };
        let shown = windows(&limits, 0.0);
        assert_eq!(shown.len(), 2);
        assert_eq!((shown[0].label.as_str(), shown[0].left), ("5H", 70));
        assert_eq!(shown[0].reset.as_deref(), Some("1h 00m"));
        assert_eq!((shown[1].label.as_str(), shown[1].left), ("WEEK", 0));
    }

    #[test]
    fn windows_past_their_reset_read_full() {
        let limits = Limits {
            session: Some((80.0, Some(100.0))),
            weekly: Some((40.0, Some(9_000.0))),
            ..Limits::default()
        };
        let shown = windows(&limits, 200.0);
        assert_eq!((shown[0].left, shown[0].reset.as_deref()), (100, None));
        assert_eq!(shown[1].left, 60);
    }

    #[test]
    fn cache_entries_read_the_python_format() {
        let python = r#"{"session": [42.0, 1792756800.0], "weekly": [10, null], "note": "x"}"#;
        let limits: Limits = serde_json::from_str(python).expect("python cache");
        assert_eq!(limits.session, Some((42.0, Some(1_792_756_800.0))));
        assert_eq!(limits.weekly, Some((10.0, None)));
        let back = serde_json::to_string(&limits).expect("json");
        assert!(back.contains("\"session\":[42.0,1792756800.0]"));
        assert!(!back.contains("monthly"));
    }

    #[test]
    fn codex_logs_give_their_newest_limits_and_past_resets_count_as_empty() {
        let log = [
            r#"{"type":"event_msg","payload":{"rate_limits":{"primary":{"used_percent":10,"resets_at":100}}}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count"}}"#,
            r#"{"type":"event_msg","payload":{"rate_limits":{"primary":null}}}"#,
            r#"{"type":"event_msg","payload":{"rate_limits":{"primary":{"used_percent":55,"resets_at":5000},"secondary":{"used_percent":20,"resets_in_seconds":60}}}}"#,
        ]
        .join("\n");
        let limits = codex_rate_limits(&log).expect("limits");
        assert_eq!(
            codex_window(&limits, "primary", 1000.0),
            Some((55.0, Some(5000.0)))
        );
        assert_eq!(
            codex_window(&limits, "secondary", 1000.0),
            Some((20.0, Some(1060.0)))
        );
        assert_eq!(
            codex_window(&limits, "primary", 9000.0),
            Some((0.0, Some(5000.0)))
        );
    }

    #[test]
    fn jwt_payloads_decode() {
        // {"sub":"auth0|user_1","email":"me@x.io"}
        let token = "e30.eyJzdWIiOiJhdXRoMHx1c2VyXzEiLCJlbWFpbCI6Im1lQHguaW8ifQ.sig";
        let payload = jwt_payload(token).expect("payload");
        assert_eq!(payload["sub"], "auth0|user_1");
        assert_eq!(payload["email"], "me@x.io");
    }

    #[test]
    fn the_newer_sign_in_is_the_one_expiring_later() {
        let older = r#"{"claudeAiOauth":{"accessToken":"a","expiresAt":1000}}"#;
        let newer = r#"{"claudeAiOauth":{"accessToken":"b","expiresAt":2000}}"#;
        assert!(expires_at(newer) > expires_at(older));
        assert_eq!(expires_at("not json"), 0.0);
    }

    #[test]
    fn sign_in_errors_need_a_sign_in() {
        assert!(auth_needed(Some("no sign-in found: sign in")));
        assert!(auth_needed(Some("Cursor sign-in expired: sign in again")));
        assert!(!auth_needed(Some("request failed (timeout)")));
        assert!(!auth_needed(None));
    }

    #[test]
    fn adding_an_account_keeps_the_config_and_refuses_repeats() {
        let mut config = serde_json::json!({"lang": "en", "accounts": [{"id": "claude-work", "kind": "claude"}]});
        let id = add_to_config(&mut config, "Personal Mac").expect("added");
        assert_eq!(id, "claude-personal-mac");
        assert_eq!(config["lang"], "en");
        assert_eq!(
            config["accounts"][1]["dir"],
            "~/.ai-accounts/claude/personal-mac"
        );
        assert!(add_to_config(&mut config, "personal mac").is_err());
        assert!(add_to_config(&mut config, "work").is_err());
        assert!(add_to_config(&mut config, "  ").is_err());
    }

    #[test]
    fn removing_drops_listed_accounts_and_hides_detected_ones() {
        let mut config = serde_json::json!({"lang": "en", "accounts": [{"id": "claude-work", "kind": "claude"}]});
        remove_from_config(&mut config, "claude-work").expect("removed");
        assert_eq!(config["accounts"], serde_json::json!([]));
        assert!(config.get("hidden_accounts").is_none());
        remove_from_config(&mut config, "codex").expect("hidden");
        remove_from_config(&mut config, "codex").expect("hidden once");
        assert_eq!(config["hidden_accounts"], serde_json::json!(["codex"]));
        assert_eq!(config["lang"], "en");
        assert_eq!(hidden_accounts(Some(&config)), vec!["codex".to_owned()]);
    }

    #[test]
    fn slot_names_match_claude_and_the_python_script() {
        use crate::platform::orcha_claude_service;
        assert_eq!(
            orcha_claude_service(&OrchaSecretSlot::Folder(None)),
            "Claude Code-credentials"
        );
        // sha256("/Users/me/.ai-accounts/claude/personal")[:8]
        let name = orcha_claude_service(&OrchaSecretSlot::Folder(Some(
            "/Users/me/.ai-accounts/claude/personal".to_owned(),
        )));
        assert_eq!(name, "Claude Code-credentials-69098b02");
        assert_eq!(
            orcha_claude_service(&OrchaSecretSlot::Kept("claude-work".to_owned())),
            "orcha-claude-claude-work"
        );
    }

    #[test]
    fn pane_ids_are_found_anywhere_in_a_result() {
        let result = serde_json::json!({"tab": {"panes": [{"pane_id": "w1:p2"}]}});
        assert_eq!(first_pane_id(&result).as_deref(), Some("w1:p2"));
        assert_eq!(first_pane_id(&serde_json::json!({"tab": {}})), None);
    }

    #[test]
    fn only_plain_emails_go_on_the_command_line() {
        assert_eq!(plain_email(" me+x@ex.io "), Some("me+x@ex.io"));
        assert_eq!(plain_email("a@b.c; rm -rf ~"), None);
        assert_eq!(plain_email(""), None);
    }
}

#[cfg(test)]
mod live {
    /// `cargo test --bin herdr orcha_accounts::live -- --ignored --nocapture`
    /// prints this machine's accounts and fuel (no tokens).
    /// Where each Claude account's sign-in is read from and when its access
    /// token runs out (no tokens printed).
    #[test]
    #[ignore]
    fn print_sign_ins() {
        let accounts = super::accounts();
        let now = super::now_secs();
        for account in accounts.iter().filter(|account| account.kind == "claude") {
            let slot = super::claude_slot(&accounts, &account.id);
            let secret = slot.as_ref().and_then(crate::platform::orcha_secret_read);
            let refresh = secret.as_deref().is_some_and(|secret| {
                serde_json::from_str::<serde_json::Value>(secret)
                    .ok()
                    .and_then(|value| value.get("claudeAiOauth")?.get("refreshToken").cloned())
                    .is_some()
            });
            let expires = secret.as_deref().map(super::expires_at).unwrap_or(0.0) / 1000.0;
            eprintln!(
                "signin {} slot={:?} found={} refresh_token={} expires_in_min={:.0}",
                account.id,
                slot.map(|slot| crate::platform::orcha_claude_service(&slot)),
                secret.is_some(),
                refresh,
                (expires - now) / 60.0
            );
        }
    }

    /// Fetches every account once, past the cache (no tokens printed).
    #[test]
    #[ignore]
    fn fetch_fresh() {
        let accounts = super::accounts();
        for account in &accounts {
            let limits = super::limits(&accounts, account, true);
            eprintln!(
                "fresh {} error={:?} session={:?} weekly={:?} monthly={:?}",
                account.id, limits.error, limits.session, limits.weekly, limits.monthly
            );
        }
    }

    #[test]
    #[ignore]
    fn print_accounts() {
        let pane = std::env::var("ORCHA_LIVE_PANE").ok();
        for account in super::accounts_with_fuel(pane.as_deref(), false) {
            eprintln!(
                "{} {} email={:?} signed_in={:?} current={} error={:?} windows={:?}",
                account.id,
                account.kind,
                account.email,
                account.signed_in,
                account.current,
                account.error,
                account
                    .windows
                    .iter()
                    .map(|window| format!("{} {}% {:?}", window.label, window.left, window.reset))
                    .collect::<Vec<_>>()
            );
        }
    }
}
