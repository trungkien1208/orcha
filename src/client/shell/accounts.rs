//! orcha: agent accounts and their fuel (plan limits), for the FUEL gauges
//! and the accounts popup (docs/orcha/SPEC.md §3.2).
//!
//! The data comes from orcha itself (`crate::orcha_accounts`), or from
//! `[orcha] accounts_command` when one is set (it prints the same JSON). It
//! runs on a background thread at most once a minute; rendering only reads
//! the last result. "Current" is the focused pane's account: when the focus
//! moves, orcha's own data only asks which account the new pane runs on (no
//! fuel, no keychain) and moves the mark; a command runs again.

use std::collections::HashMap;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use crate::protocol::ClientShellSnapshot;

const REFRESH: Duration = Duration::from_secs(60);
/// Clicking through panes looks the account up once they settle, not on
/// every click.
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
    /// The command and the focused pane the data follows.
    key: Option<(String, Option<String>)>,
    data: Option<Vec<Account>>,
    pending: Option<Receiver<AccountsRead>>,
    next_refresh: Option<Instant>,
    /// The next read asks every account again, past the cache.
    fresh: bool,
    /// The account orcha marks current when the focused pane's own is
    /// unknown (the one picked last), as of the last read.
    fallback: Option<String>,
    /// Finding out which account a newly focused pane runs on, and when to
    /// start that (once the focus settles).
    resolving: Option<Receiver<Option<String>>>,
    resolve_at: Option<Instant>,
    /// Last observed owner of an ambiguous local pane, keyed by stable pane
    /// id and provider. Never infer an owner from how much fuel is left.
    pane_owners: HashMap<String, (String, String)>,
    pane_owners_pending: Option<Receiver<PaneOwnersRead>>,
    pane_owners_refresh: Option<Instant>,
    pane_owners_revision: Option<u64>,
}

type PaneOwnersRead = Vec<(String, String, Option<String>)>;

fn provider_id(kind: &str) -> &str {
    super::brutal::AgentKind::of(Some(kind)).map_or(kind, |provider| provider.id)
}

/// A read's result: the accounts (None when it failed), the pane it was
/// read for, and the account orcha falls back on for an unknown pane.
struct AccountsRead {
    accounts: Option<Vec<Account>>,
    pane: Option<String>,
    fallback: Option<String>,
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

fn read_accounts(command: &str, pane: Option<&str>, fresh: bool) -> AccountsRead {
    let read = |accounts, fallback| AccountsRead {
        accounts,
        pane: pane.map(str::to_owned),
        fallback,
    };
    if command.trim().is_empty() {
        // `accounts_with_fuel(pane)`, with the pane's account found apart
        // from the rest, so a focus change can move the mark on its own
        let configs = crate::orcha_accounts::accounts();
        let owner = pane.and_then(|pane| crate::orcha_accounts::account_of_pane(&configs, pane));
        let mut accounts = crate::orcha_accounts::accounts_with_fuel_of(&configs, None, fresh);
        let fallback = marked(&accounts);
        if owner.is_some() {
            mark(&mut accounts, owner.as_deref());
        }
        return read(Some(accounts), fallback);
    }
    let mut cmd = orcha_command(command);
    if let Some(pane) = pane {
        cmd.env("HERDR_ACTIVE_PANE_ID", pane);
    }
    let Ok(output) = cmd.output() else {
        return read(None, None);
    };
    if !output.status.success() {
        tracing::debug!(
            status = ?output.status,
            "orcha accounts command failed"
        );
        return read(None, None);
    }
    read(serde_json::from_slice(&output.stdout).ok(), None)
}

/// The account marked current.
fn marked(accounts: &[Account]) -> Option<String> {
    accounts
        .iter()
        .find(|account| account.current)
        .map(|account| account.id.clone())
}

/// Marks `id` current, and only it. Returns true when a mark moved.
fn mark(accounts: &mut [Account], id: Option<&str>) -> bool {
    let mut moved = false;
    for account in accounts {
        let current = Some(account.id.as_str()) == id;
        moved |= account.current != current;
        account.current = current;
    }
    moved
}

impl AccountsPoller {
    pub(super) fn has_pending(&self) -> bool {
        self.pending.is_some() || self.resolving.is_some() || self.pane_owners_pending.is_some()
    }

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

    #[cfg(test)]
    pub(super) fn set_pane_owner_for_test(&mut self, pane: &str, provider: &str, owner: &str) {
        self.pane_owners
            .insert(pane.into(), (provider.into(), owner.into()));
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
            mark(data, Some(id));
        }
        // a refresh or a lookup already under way would bring the old
        // state back
        self.pending = None;
        self.resolving = None;
        self.resolve_at = None;
        self.next_refresh = Some(now + Duration::from_millis(1500));
    }

    /// Collects a finished refresh and starts the next one when due. Returns
    /// true when the data changed and needs a repaint.
    pub(super) fn tick(&mut self, command: &str, pane: Option<&str>, now: Instant) -> bool {
        let mut changed = false;
        let own = command.trim().is_empty();
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(read) => {
                    self.pending = None;
                    self.fallback = read.fallback;
                    // keep the last good data when one refresh fails
                    if read.accounts.is_some() && read.accounts != self.data {
                        self.data = read.accounts;
                        changed = true;
                    }
                    // orcha's own read for a pane the focus has left since:
                    // the mark may sit on that pane's account
                    if own
                        && read.pane.as_deref() != pane
                        && self.resolving.is_none()
                        && self.resolve_at.is_none()
                    {
                        self.resolve_at = Some(now);
                    }
                }
                Err(TryRecvError::Disconnected) => self.pending = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = &self.resolving {
            match rx.try_recv() {
                Ok(owner) => {
                    self.resolving = None;
                    let id = owner.or_else(|| self.fallback.clone());
                    if let Some(data) = self.data.as_mut() {
                        changed |= mark(data, id.as_deref());
                    }
                }
                Err(TryRecvError::Disconnected) => self.resolving = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        let same = |(c, p): &(String, Option<String>)| c == command && p.as_deref() == pane;
        let key = || (command.to_owned(), pane.map(str::to_owned));
        if let Some((known, _)) = self.key.as_ref().filter(|known| !same(known)) {
            // another pane, once the focus stops moving: orcha's own data
            // only moves the mark (the fuel is the same); a command, or a
            // new one, reads again
            self.resolving = None;
            if own && known == command {
                self.resolve_at = Some(now + SETTLE);
            } else {
                self.resolve_at = None;
                self.next_refresh = Some(now + SETTLE);
            }
            self.key = Some(key());
            return changed;
        }
        if self.resolving.is_none() && self.resolve_at.is_some_and(|at| now >= at) {
            self.resolve_at = None;
            match pane {
                Some(pane) => {
                    let pane = pane.to_owned();
                    let (tx, rx) = mpsc::channel();
                    let spawned =
                        crate::thread_spawn::spawn_named("orcha-account-of-pane", move || {
                            let configs = crate::orcha_accounts::accounts();
                            let owner = crate::orcha_accounts::account_of_pane(&configs, &pane);
                            let _ = tx.send(owner);
                        });
                    if spawned.is_ok() {
                        self.resolving = Some(rx);
                    }
                }
                // no pane focused: the account picked last
                None => {
                    if let Some(data) = self.data.as_mut() {
                        changed |= mark(data, self.fallback.as_deref());
                    }
                }
            }
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

/// An agent whose account runs low on fuel, for its sidebar row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FuelMark {
    /// Percent left of the window that runs out first.
    pub left: u8,
    /// Under `low_fuel_percent`, where orcha offers a switch (red); else
    /// only low (yellow).
    pub empty: bool,
}

impl AccountsPoller {
    pub(super) fn forget_pane_owners(&mut self) {
        self.pane_owners.clear();
        self.pane_owners_pending = None;
        self.pane_owners_refresh = None;
        self.pane_owners_revision = None;
    }

    /// Resolves ambiguous pane owners off the UI thread, at most once per
    /// minute. The caller only uses this for the local endpoint and native
    /// accounts. Provider changes and closed panes invalidate old results.
    pub(super) fn tick_pane_owners(
        &mut self,
        snapshot: &ClientShellSnapshot,
        now: Instant,
    ) -> bool {
        let owners = self
            .pane_owners_pending
            .as_ref()
            .and_then(|rx| match rx.try_recv() {
                Ok(owners) => Some(Ok(owners)),
                Err(TryRecvError::Disconnected) => Some(Err(())),
                Err(TryRecvError::Empty) => None,
            });
        if owners.is_some() {
            self.pane_owners_pending = None;
        }
        if owners.is_none()
            && self.pane_owners_revision == Some(snapshot.revision)
            && (self.pane_owners_pending.is_some()
                || self.pane_owners_refresh.is_some_and(|at| now < at))
        {
            return false;
        }
        self.pane_owners_revision = Some(snapshot.revision);
        let providers: HashMap<_, _> = snapshot
            .agents
            .iter()
            .filter_map(|agent| {
                Some((agent.pane_id.as_str(), provider_id(agent.agent.as_deref()?)))
            })
            .collect();
        let known = |pane: &str, kind: &str| {
            providers
                .get(pane)
                .is_some_and(|actual| super::brutal::same_agent(actual, kind))
        };
        let mut changed = false;
        self.pane_owners.retain(|pane, (kind, _)| {
            let keep = known(pane, kind);
            changed |= !keep;
            keep
        });
        if let Some(Ok(owners)) = owners {
            for (pane, kind, owner) in owners {
                if known(&pane, &kind) {
                    let next = owner.map(|owner| (kind, owner));
                    if self.pane_owners.get(&pane) != next.as_ref() {
                        changed = true;
                        match next {
                            Some(next) => {
                                self.pane_owners.insert(pane, next);
                            }
                            None => {
                                self.pane_owners.remove(&pane);
                            }
                        }
                    }
                }
            }
        }
        if self.pane_owners_pending.is_some() || self.pane_owners_refresh.is_some_and(|at| now < at)
        {
            return changed;
        }
        let Some(accounts) = self.data.as_deref() else {
            return changed;
        };
        let panes: Vec<_> = snapshot
            .agents
            .iter()
            .filter_map(|agent| {
                let kind = agent.agent.as_deref()?;
                let ambiguous = accounts
                    .iter()
                    .filter(|account| super::brutal::same_agent(&account.kind, kind))
                    .take(2)
                    .count()
                    > 1;
                ambiguous.then(|| (agent.pane_id.clone(), kind.to_ascii_lowercase()))
            })
            .collect();
        self.pane_owners_refresh = Some(now + REFRESH);
        if panes.is_empty() {
            return changed;
        }
        let (tx, rx) = mpsc::channel();
        if crate::thread_spawn::spawn_named("orcha-pane-accounts", move || {
            let configs = crate::orcha_accounts::accounts();
            let owners = panes
                .into_iter()
                .map(|(pane, kind)| {
                    let owner =
                        crate::orcha_accounts::observed_account_of_agent(&configs, &kind, &pane);
                    (pane, kind, owner)
                })
                .collect();
            let _ = tx.send(owners);
        })
        .is_ok()
        {
            self.pane_owners_pending = Some(rx);
        }
        changed
    }

    /// Low fuel only for an observed owner, the focused account, or an
    /// agent kind with one account. Ambiguous unknown panes have no mark.
    pub(super) fn low_fuel_agents<'a>(
        &self,
        snapshot: &'a ClientShellSnapshot,
        orcha: &crate::config::OrchaConfig,
    ) -> HashMap<&'a str, FuelMark> {
        let mut marks = HashMap::new();
        let Some(accounts) = self.data.as_deref() else {
            return marks;
        };
        let mut by_provider: HashMap<&str, Vec<&Account>> = HashMap::new();
        let mut by_id = HashMap::new();
        for account in accounts {
            by_provider
                .entry(provider_id(&account.kind))
                .or_default()
                .push(account);
            by_id.insert(account.id.as_str(), account);
        }
        for agent in &snapshot.agents {
            let Some(kind) = agent.agent.as_deref().map(provider_id) else {
                continue;
            };
            let owner = self
                .pane_owners
                .get(&agent.pane_id)
                .filter(|(provider, _)| super::brutal::same_agent(provider, kind))
                .and_then(|(_, owner)| by_id.get(owner.as_str()).copied())
                .filter(|account| super::brutal::same_agent(&account.kind, kind));
            let same_kind = by_provider.get(kind).map_or(&[][..], Vec::as_slice);
            let focused = snapshot.focused_pane_id.as_deref() == Some(agent.pane_id.as_str());
            let account = focused
                .then(|| same_kind.iter().copied().find(|account| account.current))
                .flatten()
                .or(owner)
                .or_else(|| (same_kind.len() == 1).then(|| same_kind[0]));
            let Some(left) = account.and_then(Account::lowest) else {
                continue;
            };
            if super::orcha_popup::fuel_is_low(left, orcha) {
                marks.insert(
                    agent.pane_id.as_str(),
                    FuelMark {
                        left,
                        empty: left < orcha.low_fuel_percent,
                    },
                );
            }
        }
        marks
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
                    reset_at: None,
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

    fn poller_on(command: &str, now: Instant) -> AccountsPoller {
        let named = |id: &str, current: bool| Account {
            id: id.into(),
            current,
            ..account(Some(true), None, true)
        };
        AccountsPoller {
            key: Some((command.into(), Some("pane_1".into()))),
            data: Some(vec![named("a", true), named("b", false)]),
            next_refresh: Some(now + REFRESH),
            fallback: Some("a".into()),
            ..Default::default()
        }
    }

    #[test]
    fn a_focus_change_moves_the_mark_without_reading_every_account() {
        let now = Instant::now();
        let mut poller = poller_on("", now);
        assert!(!poller.tick("", Some("pane_2"), now));
        assert!(poller.pending.is_none(), "no fuel read");
        assert_eq!(
            poller.next_refresh,
            Some(now + REFRESH),
            "the cadence holds"
        );
        assert_eq!(poller.resolve_at, Some(now + SETTLE));
        // the lookup lands: pane_2 runs on b
        let (tx, rx) = mpsc::channel();
        poller.resolving = Some(rx);
        poller.resolve_at = None;
        tx.send(Some("b".into())).expect("send");
        assert!(poller.tick("", Some("pane_2"), now));
        assert_eq!(poller.current().map(|a| a.id.as_str()), Some("b"));
        // a pane whose account is unknown: the account picked last
        let (tx, rx) = mpsc::channel();
        poller.resolving = Some(rx);
        tx.send(None).expect("send");
        assert!(poller.tick("", Some("pane_2"), now));
        assert_eq!(poller.current().map(|a| a.id.as_str()), Some("a"));
        assert!(poller.pending.is_none());
    }

    #[test]
    fn a_focus_change_runs_an_accounts_command_again() {
        let now = Instant::now();
        let mut poller = poller_on("./accounts.sh", now);
        assert!(!poller.tick("./accounts.sh", Some("pane_2"), now));
        assert_eq!(poller.next_refresh, Some(now + SETTLE));
        assert!(poller.resolve_at.is_none());
    }
}
