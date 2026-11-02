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

use crate::orcha_app::write_atomic;
use crate::platform::OrchaSecretSlot;

mod secret_cache;

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
    /// When the window refills, in Unix seconds.
    #[serde(default)]
    pub reset_at: Option<u64>,
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

mod registry;
pub(crate) use registry::{agent_kind, AgentKind, AGENTS, HANDOFF_AGENTS};

mod configuration;
use configuration::*;
pub(crate) use configuration::{accounts, remove};

mod claude;
use claude::*;

mod fuel;
use fuel::*;

mod preferences;
use preferences::*;
pub(crate) use preferences::{
    account_of_agent, account_of_pane, claude_folder_of_pane, default_account, find_account, plan,
    prepare_folder, rename, set_default,
};

mod panes;
use panes::*;
pub(crate) use panes::{accounts_with_fuel, accounts_with_fuel_of};

mod actions;
use actions::*;
pub(crate) use actions::{activate, add, login, open, restart_blocker, restart_on, switch};

mod discovery;
use discovery::*;
pub(crate) use discovery::{find_unlisted, import_found, new_agent, FoundAccount};

#[cfg(test)]
mod live;
#[cfg(test)]
mod restart_line_tests;
#[cfg(test)]
mod tests;
