//! orcha: agent accounts and their fuel (plan limits), for the FUEL gauges
//! and the accounts popup (docs/orcha/SPEC.md §3.2).
//!
//! The data comes from orcha itself (`crate::orcha_accounts`), or from
//! `[orcha] accounts_command` when one is set (it prints the same JSON). It
//! runs on a background thread at most once a minute (or when the focused
//! pane changes, since "current" is that pane's account); rendering only
//! reads the last result.

use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

const REFRESH: Duration = Duration::from_secs(60);
/// Clicking through panes refreshes once they settle, not on every click.
const SETTLE: Duration = Duration::from_millis(800);

pub(super) use crate::orcha_accounts::{Account, FuelWindow};

impl Account {
    /// Percent left of the window that runs out first.
    pub(super) fn lowest(&self) -> Option<u8> {
        self.windows.iter().map(|window| window.left).min()
    }

    /// Needs a sign-in before it can run anything.
    ///
    /// Credentials may still sit in the keychain after the token dies. Fuel
    /// then has an error and no windows; treat that as signed out so SIGN IN
    /// stays on the row (a usage-request failure is not a sign-in error).
    pub(super) fn signed_out(&self) -> bool {
        if matches!(self.signed_in, Some(false)) {
            return true;
        }
        self.windows.is_empty() && self.error.as_deref().is_some_and(sign_in_error)
    }
}

/// "1h 12m" → "resets in 1h 12m". The clock already passed → "resets now".
pub(super) fn resets_in(reset: &str) -> String {
    if reset.eq_ignore_ascii_case("now") {
        "resets now".to_owned()
    } else {
        format!("resets in {reset}")
    }
}

fn sign_in_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("sign-in")
        || error.contains("sign in")
        || error.contains("signed in")
        || error.contains("signin")
        || error.contains("đăng nhập")
        || error.contains("hết hạn")
}

#[derive(Default)]
pub(super) struct AccountsPoller {
    key: Option<(String, Option<String>)>,
    data: Option<Vec<Account>>,
    pending: Option<Receiver<Option<Vec<Account>>>>,
    next_refresh: Option<Instant>,
    /// The next read asks every account again, past the cache.
    fresh: bool,
}

/// A shell running an orcha command against this session's API socket.
pub(super) fn orcha_command(command: &str) -> Command {
    let mut cmd = crate::platform::orcha_shell_command(&expand_home(command));
    cmd.env(
        crate::api::SOCKET_PATH_ENV_VAR,
        crate::session::active_api_socket_path(),
    );
    // the scripts drive this app's CLI, whatever `herdr` on PATH is
    if let Ok(exe) = std::env::current_exe() {
        cmd.env("HERDR_BIN_PATH", exe);
    }
    cmd
}

/// Expands a leading home-relative script path for the host shell.
pub(super) fn expand_home(command: &str) -> String {
    crate::platform::expand_orcha_home_command(command)
}

fn read_accounts(command: &str, pane: Option<&str>, fresh: bool) -> Option<Vec<Account>> {
    if command.trim().is_empty() {
        return Some(crate::orcha_accounts::accounts_with_fuel(pane, fresh));
    }
    let mut cmd = orcha_command(command);
    if let Some(pane) = pane {
        cmd.env("HERDR_ACTIVE_PANE_ID", pane);
    }
    let output = cmd.output().ok()?;
    if !output.status.success() {
        tracing::debug!(
            status = ?output.status,
            "orcha accounts command failed"
        );
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

impl AccountsPoller {
    pub(super) fn data(&self) -> Option<&[Account]> {
        self.data.as_deref()
    }

    /// The account the focused agent runs on, else the first one.
    pub(super) fn current(&self) -> Option<&Account> {
        let data = self.data.as_deref()?;
        data.iter()
            .find(|account| account.current)
            .or_else(|| data.first())
    }

    #[cfg(test)]
    pub(super) fn set_for_test(&mut self, data: Option<Vec<Account>>) {
        self.data = data;
    }

    /// Ask for fresh data on the next tick (after a switch or sign-in).
    pub(super) fn refresh_soon(&mut self) {
        self.next_refresh = None;
    }

    /// REFRESH: ask every account now, past the cache.
    pub(super) fn refresh_now(&mut self) {
        self.fresh = true;
        // a read under way may hold older numbers
        self.pending = None;
        self.next_refresh = None;
    }

    /// A switch shows at once: `id` is the current account now, the real
    /// data follows once the switch had time to land.
    pub(super) fn mark_current(&mut self, id: &str, now: Instant) {
        if let Some(data) = self.data.as_mut() {
            for account in data.iter_mut() {
                account.current = account.id == id;
            }
        }
        // a refresh already under way would bring the old state back
        self.pending = None;
        self.next_refresh = Some(now + Duration::from_millis(1500));
    }

    /// Collects a finished refresh and starts the next one when due. Returns
    /// true when the data changed and needs a repaint.
    pub(super) fn tick(&mut self, command: &str, pane: Option<&str>, now: Instant) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(data) => {
                    self.pending = None;
                    // keep the last good data when one refresh fails
                    if data.is_some() && data != self.data {
                        self.data = data;
                        changed = true;
                    }
                }
                Err(TryRecvError::Disconnected) => self.pending = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        let same = |(c, p): &(String, Option<String>)| c == command && p.as_deref() == pane;
        let key = || (command.to_owned(), pane.map(str::to_owned));
        if self.key.as_ref().is_some_and(|known| !same(known)) {
            // another pane: refresh soon, once the focus stops moving
            self.key = Some(key());
            self.next_refresh = Some(now + SETTLE);
            return changed;
        }
        let due = self.key.is_none() || self.next_refresh.is_none_or(|at| now >= at);
        if due && self.pending.is_none() {
            self.key = Some(key());
            let (command, pane) = (command.to_owned(), pane.map(str::to_owned));
            let fresh = std::mem::take(&mut self.fresh);
            let (tx, rx) = mpsc::channel();
            let spawned = crate::thread_spawn::spawn_named("orcha-accounts", move || {
                let _ = tx.send(read_accounts(&command, pane.as_deref(), fresh));
            });
            if spawned.is_ok() {
                self.pending = Some(rx);
            }
            self.next_refresh = Some(now + REFRESH);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_accounts_json() {
        let json = r#"[{"id":"work","kind":"claude","label":"Claude · company","email":"a@b.c",
            "current":true,"error":null,"windows":[{"key":"session","label":"5H","left":60,"reset":"1h"},
            {"key":"weekly","label":"WEEK","left":18,"reset":null}]}]"#;
        let accounts: Vec<Account> = serde_json::from_str(json).expect("json");
        assert_eq!(accounts[0].lowest(), Some(18));
        assert!(accounts[0].current);
        assert!(!accounts[0].signed_out());
    }

    fn account(signed_in: Option<bool>, error: Option<&str>, windows: bool) -> Account {
        Account {
            id: "work".into(),
            kind: "claude".into(),
            label: "Claude · work".into(),
            email: Some("a@b.c".into()),
            current: true,
            error: error.map(str::to_owned),
            windows: if windows {
                vec![FuelWindow {
                    key: "session".into(),
                    label: "5H".into(),
                    left: 40,
                    reset: None,
                }]
            } else {
                Vec::new()
            },
            signed_in,
        }
    }

    #[test]
    fn expired_sign_in_still_needs_the_sign_in_button() {
        assert!(account(
            Some(true),
            Some("sign-in expired: open Claude with this account once"),
            false
        )
        .signed_out());
        assert!(account(Some(false), Some("not signed in: ctrl-l to sign in"), false).signed_out());
        assert!(account(
            None,
            Some("chưa đăng nhập: nhấn ctrl-l để đăng nhập"),
            false
        )
        .signed_out());
        assert!(
            !account(Some(true), Some("usage request failed (error: 429)"), false).signed_out()
        );
        assert!(!account(Some(true), None, true).signed_out());
    }
}
