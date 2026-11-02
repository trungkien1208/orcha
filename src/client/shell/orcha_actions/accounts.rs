//! Account operations, hand-offs and low-fuel policy.

use super::*;

impl ClientShellState {
    /// Runs an account action off the input thread; the result shows as a
    /// notice and the accounts refresh.
    pub(super) fn start_account_action(&mut self, action: AccountAction) {
        if self.orcha_ui.account_run.is_some() {
            self.show_notice("accounts: one change is on its way already".to_owned());
            return;
        }
        let change = match &action {
            AccountAction::Switch { id, pane } | AccountAction::Restart { id, pane } => {
                self.snapshot.as_deref().and_then(|snapshot| {
                    snapshot
                        .agents
                        .iter()
                        .find(|agent| agent.pane_id == *pane)
                        .and_then(|agent| agent.agent.as_ref())
                        .map(|provider| (id.clone(), pane.clone(), provider.clone()))
                })
            }
            _ => None,
        };
        let working = action.working();
        self.orcha_ui.account_run = BackgroundJob::spawn("orcha-account", move || action.run());
        if self.orcha_ui.account_run.is_some() {
            self.orcha_ui.account_change = change;
            self.start_working_notice("orcha-account", working);
        } else {
            self.show_failure("Could not start the account change, try again".to_owned());
        }
    }

    /// An account action finished: say how it went. True when the screen
    /// changed.
    pub(in crate::client::shell) fn poll_account_action(&mut self) -> bool {
        let Some(result) = BackgroundJob::take_finished(&mut self.orcha_ui.account_run) else {
            return false;
        };
        self.end_working_notice("orcha-account");
        self.accounts.refresh_soon();
        let change = self.orcha_ui.account_change.take();
        match result.unwrap_or_else(|lost| Err(lost.to_string())) {
            Ok(message) => {
                if let Some((id, pane, provider)) = change {
                    self.accounts.forget_pane_owners();
                    if self.active_endpoint_id == ClientEndpointId::Local
                        && self.snapshot.as_deref().is_some_and(|snapshot| {
                            snapshot.focused_pane_id.as_deref() == Some(pane.as_str())
                                && snapshot.agents.iter().any(|agent| {
                                    agent.pane_id == pane
                                        && agent.agent.as_deref().is_some_and(|actual| {
                                            super::super::brutal::same_agent(actual, &provider)
                                        })
                                })
                        })
                    {
                        self.accounts.mark_current(&id, std::time::Instant::now());
                    }
                }
                self.show_notice(message);
            }
            Err(error) => self.show_failure(format!("accounts: {error}")),
        }
        true
    }

    /// Asks before taking the `index`th account off orcha's list.
    pub(super) fn ask_remove_account(&mut self, index: usize) {
        let Some(super::super::orcha_popup::OrchaPopup::Accounts { pane, .. }) =
            self.orcha_ui.popup.clone()
        else {
            return;
        };
        let Some(account) = self
            .accounts
            .data()
            .and_then(|data| data.get(index))
            .cloned()
        else {
            return;
        };
        let who = account
            .email
            .as_deref()
            .map(|email| format!("{} · {email}", account.label))
            .unwrap_or_else(|| account.label.clone());
        self.orcha_ui.popup = Some(super::super::orcha_popup::OrchaPopup::RemoveAccount {
            selected: 1,
            id: account.id,
            label: who,
            pane,
        });
    }

    /// REFRESH: every account's fuel again, past the cache.
    pub(in crate::client::shell) fn refresh_fuel(&mut self) {
        self.accounts.refresh_now();
        self.show_notice("refreshing fuel…".to_owned());
    }

    pub(in crate::client::shell) fn open_accounts_overlay(&mut self) {
        let pane = self
            .snapshot
            .as_deref()
            .and_then(super::super::orcha_popup::focused_agent_pane);
        self.orcha_ui.popup = Some(self.accounts_popup(pane));
        self.accounts.refresh_soon();
    }

    /// The accounts popup for the chat in `pane`, the best place to
    /// continue picked.
    pub(super) fn accounts_popup(
        &self,
        pane: Option<String>,
    ) -> super::super::orcha_popup::OrchaPopup {
        let selected = self
            .accounts
            .data()
            .and_then(|accounts| {
                super::super::orcha_popup::best_account(accounts)
                    .or_else(|| accounts.iter().position(|account| account.current))
            })
            .unwrap_or(0);
        super::super::orcha_popup::OrchaPopup::Accounts { selected, pane }
    }

    pub(in crate::client::shell) fn open_handoff_popup(&mut self) {
        let agent = self.snapshot.as_deref().and_then(focused_agent);
        let Some((pane, from)) = agent.map(|agent| (agent.pane_id.clone(), agent.agent.clone()))
        else {
            self.show_notice("hand off: open an agent first".to_owned());
            return;
        };
        // only the other agents that are installed
        let installed = crate::orcha_handoff::targets(from.as_deref());
        let targets: Vec<usize> = super::super::orcha_popup::handoff_targets()
            .iter()
            .enumerate()
            .filter(|(_, (kind, _))| {
                installed
                    .iter()
                    .any(|installed| super::super::brutal::same_agent(installed, kind.id))
            })
            .map(|(index, _)| index)
            .collect();
        if targets.is_empty() {
            self.show_notice("hand off: no other agent is installed".to_owned());
            return;
        }
        // H in the accounts popup: esc goes back to it
        self.orcha_ui.handoff_from_accounts = matches!(
            self.orcha_ui.popup,
            Some(super::super::orcha_popup::OrchaPopup::Accounts { .. })
        );
        self.orcha_ui.popup = Some(super::super::orcha_popup::OrchaPopup::Handoff {
            selected: 0,
            pane,
            targets,
        });
    }

    /// Hands `pane`'s chat to `agent` off the input thread: orcha's own
    /// hand-off, or `[orcha] handoff_command` when one is set. `account`:
    /// the account picked for the new agent (its sign-in folder), else the
    /// agent's default.
    pub(in crate::client::shell) fn start_handoff(
        &mut self,
        pane: String,
        agent: String,
        mode: &str,
        account: Option<String>,
    ) {
        let custom = self.config.orcha.handoff_command.trim().to_owned();
        if !custom.is_empty() {
            let command = fill(
                &custom,
                &[
                    ("pane", &pane),
                    ("agent", &agent),
                    ("mode", mode),
                    ("account", account.as_deref().unwrap_or_default()),
                ],
            );
            run_detached(command, &[]);
            return;
        }
        if self.orcha_ui.handoff_run.is_some() {
            self.show_notice("hand off: one is on its way already".to_owned());
            return;
        }
        let mode = crate::orcha_handoff::Mode::parse(mode);
        let source = pane.clone();
        self.orcha_ui.handoff_run = BackgroundJob::spawn("orcha-handoff", move || {
            let accounts = account
                .as_deref()
                .map(|_| crate::orcha_accounts::accounts())
                .unwrap_or_default();
            let chosen = account
                .as_deref()
                .and_then(|id| crate::orcha_accounts::find_account(&accounts, id));
            // a sign-in folder that can't share history would start the
            // new chat without it: say so instead of handing off half-done
            if let Some(account) = chosen {
                crate::orcha_accounts::prepare_folder(account)?;
            }
            let folder = chosen.and_then(|account| account.dir.clone());
            crate::orcha_handoff::hand_off_to(&pane, &agent, mode, folder.as_deref())
        });
        if self.orcha_ui.handoff_run.is_some() {
            self.orcha_ui.handoff_source = Some(source);
            self.start_working_notice("orcha-handoff", "hand off: writing the notes…".to_owned());
        }
    }

    /// The hand-off finished: say how it went, and mark the old pane's row
    /// "→ continued". True when the screen changed.
    pub(in crate::client::shell) fn poll_handoff(&mut self) -> bool {
        let Some(result) = BackgroundJob::take_finished(&mut self.orcha_ui.handoff_run) else {
            return false;
        };
        self.end_working_notice("orcha-handoff");
        let source = self.orcha_ui.handoff_source.take();
        match result.unwrap_or_else(|lost| Err(lost.to_string())) {
            Ok((message, pane)) => {
                self.show_notice(format!("hand off: {message}"));
                if let (Some(source), false) = (source, pane.is_empty()) {
                    self.mark_continued(source, pane);
                }
            }
            Err(error) => self.show_failure(format!("hand off failed: {error}")),
        }
        true
    }

    /// `old`'s chat goes on in `pane`: its row says so until it is focused
    /// (after the focus left it once) or closed.
    pub(in crate::client::shell) fn mark_continued(&mut self, old: String, pane: String) {
        let focused = self
            .snapshot
            .as_deref()
            .and_then(|snapshot| snapshot.focused_pane_id.as_deref())
            == Some(old.as_str());
        self.orcha_ui.continued.insert(
            old,
            super::super::ContinuedPane {
                pane,
                armed: !focused,
            },
        );
    }

    /// Drops "→ continued" marks whose old pane was focused (after the focus
    /// left it once) or closed, or whose new pane is gone. True when one
    /// went.
    pub(in crate::client::shell) fn reconcile_continued(&mut self) -> bool {
        if self.orcha_ui.continued.is_empty() {
            return false;
        }
        let Some(snapshot) = self.snapshot.as_deref() else {
            return false;
        };
        let open: std::collections::HashSet<_> = snapshot
            .panes
            .iter()
            .map(|pane| pane.pane_id.as_str())
            .chain(snapshot.agents.iter().map(|agent| agent.pane_id.as_str()))
            .collect();
        let focused = snapshot.focused_pane_id.as_deref();
        let before = self.orcha_ui.continued.len();
        self.orcha_ui.continued.retain(|old, next| {
            if !open.contains(old.as_str()) || !open.contains(next.pane.as_str()) {
                return false;
            }
            if focused == Some(old.as_str()) {
                return !next.armed;
            }
            next.armed = true;
            true
        });
        before != self.orcha_ui.continued.len()
    }

    /// NEW: pick the agent and account for a new tab of the selected
    /// project, or start a new project. With no project yet, NEW PROJECT.
    pub(in crate::client::shell) fn open_new_agent_popup(&mut self) {
        let Some(workspace) = self.snapshot.as_deref().and_then(|snapshot| {
            snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.focused)
        }) else {
            self.open_new_workspace_popup();
            return;
        };
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let cwd = snapshot
            .focused_pane_id
            .as_deref()
            .and_then(|focused| self.pane_cwd(focused))
            .unwrap_or_else(|| workspace.new_workspace_cwd.clone());
        let popup = super::super::orcha_popup::OrchaPopup::NewAgent {
            selected: self.new_agent_selected(),
            workspace: workspace.workspace_id.clone(),
            project: workspace.label.clone(),
            cwd,
        };
        self.orcha_ui.popup = Some(popup);
        self.accounts.refresh_soon();
        // the defaults may have changed in the settings
        self.start_default_accounts_lookup();
    }

    /// NEW AGENT's starting row with the defaults known so far.
    pub(super) fn new_agent_selected(&self) -> usize {
        self.accounts.data().map_or(0, |accounts| {
            new_agent_choice(
                accounts,
                &self.orcha_ui.default_accounts,
                self.config.orcha.low_fuel_percent,
            )
        })
    }

    /// Looks up which accounts new agents start on (small file reads), off
    /// the input path.
    pub(super) fn start_default_accounts_lookup(&mut self) {
        if self.orcha_ui.default_accounts_job.is_some() {
            return;
        }
        self.orcha_ui.default_accounts_job = BackgroundJob::spawn("orcha-default-accounts", || {
            let accounts = crate::orcha_accounts::accounts();
            let mut kinds: Vec<&str> = accounts
                .iter()
                .map(|account| account.kind.as_str())
                .collect();
            kinds.sort_unstable();
            kinds.dedup();
            kinds
                .into_iter()
                .filter_map(|kind| crate::orcha_accounts::default_account(&accounts, kind))
                .collect()
        });
    }

    /// The defaults came in: an open NEW AGENT whose row was not moved yet
    /// starts on the default now. True when the screen changed.
    pub(super) fn poll_default_accounts(&mut self) -> bool {
        let Some(Ok(defaults)) =
            BackgroundJob::take_finished(&mut self.orcha_ui.default_accounts_job)
        else {
            return false;
        };
        let before = self.new_agent_selected();
        self.orcha_ui.default_accounts = defaults;
        let after = self.new_agent_selected();
        match &mut self.orcha_ui.popup {
            Some(super::super::orcha_popup::OrchaPopup::NewAgent { selected, .. })
                if *selected == before && before != after =>
            {
                *selected = after;
                true
            }
            _ => false,
        }
    }

    /// Create `name` as a Claude account and open its sign-in.
    pub(super) fn add_claude_account(&mut self, name: &str) {
        let name: String = name
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
            .flat_map(|ch| ch.to_lowercase())
            .collect();
        if name.is_empty() {
            return;
        }
        if self.config.orcha.add_command.trim().is_empty() {
            let pane = self
                .snapshot
                .as_deref()
                .and_then(super::super::orcha_popup::focused_agent_pane);
            self.start_account_action(AccountAction::Add { name, pane });
        } else {
            run_detached(
                fill(&self.config.orcha.add_command, &[("name", &name)]),
                &[],
            );
        }
        self.accounts.refresh_soon();
        self.orcha_ui.popup = None;
    }

    /// Low fuel: when the focused agent's account drops under
    /// `low_fuel_percent`, the header offers a switch to the account with
    /// the most fuel; other agents whose account is that low are counted
    /// in, so none runs dry unseen. Returns true when the hint changed.
    pub(in crate::client::shell) fn update_low_fuel_hint(&mut self) -> bool {
        let orcha = &self.config.orcha;
        let hint = self.accounts.data().and_then(|accounts| {
            // agents other than the focused one, on an account under the line
            let others = self.snapshot.as_deref().map_or(0, |snapshot| {
                let focused = snapshot.focused_pane_id.as_deref();
                self.accounts
                    .low_fuel_agents(snapshot, orcha)
                    .into_iter()
                    .filter(|(pane, mark)| mark.empty && Some(*pane) != focused)
                    .count()
            });
            let more = match others {
                0 => String::new(),
                1 => " · 1 more agent low".to_owned(),
                n => format!(" · {n} more agents low"),
            };
            let current = accounts.iter().find(|account| account.current);
            let left = current
                .and_then(|current| current.lowest())
                .filter(|left| *left < orcha.low_fuel_percent);
            let Some(left) = left else {
                return match others {
                    0 => None,
                    1 => Some("1 agent low on fuel".to_owned()),
                    n => Some(format!("{n} agents low on fuel")),
                };
            };
            let best = super::super::orcha_popup::best_account(accounts)
                .and_then(|index| accounts.get(index))
                .filter(|best| best.lowest().unwrap_or(0) > left);
            Some(match best {
                Some(best) => format!(
                    "FUEL {left}% · switch to {} ({}% left){more}",
                    best.label,
                    best.lowest().unwrap_or(0)
                ),
                None => format!("FUEL {left}% · hand off to another agent{more}"),
            })
        });
        let changed = hint != self.orcha_ui.low_fuel_hint;
        self.orcha_ui.low_fuel_hint = hint;
        changed
    }

    /// Running low: once per account, when the focused agent is between
    /// turns, offer where to continue. A notice, not a popup: the keys stay
    /// with the pane, so a prompt being typed cannot switch the account. A
    /// click on it opens ACCOUNTS.
    pub(in crate::client::shell) fn offer_continue_on_low_fuel(&mut self) -> bool {
        if self.orcha_ui.popup.is_some() || self.overlay.is_some() {
            return false;
        }
        let Some(current) = self.accounts.current().filter(|account| account.current) else {
            return false;
        };
        let low = current
            .lowest()
            .is_some_and(|left| left < self.config.orcha.low_fuel_percent);
        if !low || self.orcha_ui.low_fuel_offered.contains(&current.id) {
            return false;
        }
        let busy = self.snapshot.as_deref().is_some_and(|snapshot| {
            let focused = snapshot.focused_pane_id.as_deref();
            snapshot.agents.iter().any(|agent| {
                Some(agent.pane_id.as_str()) == focused
                    && agent.agent_status == crate::api::schema::AgentStatus::Working
            })
        });
        if busy {
            return false;
        }
        let id = current.id.clone();
        let left = current.lowest().unwrap_or(0);
        let message = match self.accounts.data().and_then(|accounts| {
            super::super::orcha_popup::best_account(accounts).and_then(|index| accounts.get(index))
        }) {
            Some(best) if best.lowest().unwrap_or(0) > left => format!(
                "{left}% fuel left. Click here to continue on {}",
                best.label
            ),
            _ => format!("{left}% fuel left. Click here to hand off"),
        };
        self.orcha_ui.low_fuel_offered.insert(id);
        self.show_lasting_notice(message, NoticeClick::OpenAccounts);
        true
    }

    /// An account that filled up again (a new plan window) is offered the
    /// checklist, the switch and the move again the next time it runs low.
    pub(in crate::client::shell) fn forget_refilled_accounts(&mut self) {
        let orcha = &self.config.orcha;
        let Some(accounts) = self.accounts.data() else {
            return;
        };
        let ui = &mut self.orcha_ui;
        for account in accounts {
            let Some(left) = account.lowest() else {
                continue;
            };
            if left >= orcha.low_fuel_percent {
                ui.low_fuel_offered.remove(&account.id);
            }
            if left >= orcha.checklist_percent {
                ui.checklist_asked.remove(&account.id);
            }
            if left >= orcha.auto_handoff_percent {
                ui.auto_handed_off.remove(&account.id);
            }
        }
    }

    /// Running low, with the focused agent between turns: first (under
    /// `checklist_percent`) ask it to write a hand-off checklist, then
    /// (under `auto_handoff_percent`) move the chat to the best other
    /// account without asking. Each happens once per account; both are off
    /// unless set.
    pub(in crate::client::shell) fn act_on_low_fuel(&mut self) -> bool {
        if self.orcha_ui.popup.is_some() || self.overlay.is_some() {
            return false;
        }
        let orcha = &self.config.orcha;
        if orcha.checklist_percent == 0 && orcha.auto_handoff_percent == 0 {
            return false;
        }
        let Some(current) = self.accounts.current().filter(|account| account.current) else {
            return false;
        };
        let Some(left) = current.lowest() else {
            return false;
        };
        let Some((pane, kind)) = self.snapshot.as_deref().and_then(|snapshot| {
            let focused = snapshot.focused_pane_id.as_deref()?;
            snapshot
                .agents
                .iter()
                .find(|agent| agent.pane_id == focused)
                .filter(|agent| {
                    matches!(
                        agent.agent_status,
                        crate::api::schema::AgentStatus::Idle
                            | crate::api::schema::AgentStatus::Done
                    )
                })
                .map(|agent| {
                    (
                        agent.pane_id.clone(),
                        agent.agent.clone().unwrap_or_default(),
                    )
                })
        }) else {
            return false;
        };
        let id = current.id.clone();
        let current_kind = current.kind.clone();
        if left < orcha.checklist_percent && !self.orcha_ui.checklist_asked.contains(&id) {
            self.orcha_ui.checklist_asked.insert(id);
            // asked, not typed in: a prompt half-written in the pane would go along
            let file = orcha.checklist_file.clone();
            tracing::info!(left, %file, %kind, "orcha: offering a hand-off checklist");
            self.orcha_ui.popup = Some(super::super::orcha_popup::OrchaPopup::Checklist {
                selected: 0,
                pane,
                kind,
                left,
                file,
            });
            return true;
        }
        if left >= orcha.auto_handoff_percent
            || self.orcha_ui.auto_handed_off.contains(&id)
            || self.orcha_ui.account_run.is_some()
        {
            return false;
        }
        self.orcha_ui.auto_handed_off.insert(id.clone());
        // it is moving on by itself: the offer would only get in the way
        self.orcha_ui.low_fuel_offered.insert(id.clone());
        let target = self.accounts.data().and_then(|accounts| {
            super::super::orcha_popup::best_account(accounts)
                .and_then(|index| accounts.get(index).cloned())
        });
        let Some(target) = target else {
            tracing::info!(left, "orcha: low on fuel, but no other account has any");
            self.show_notice(format!(
                "{left}% left · no other account has fuel to move to"
            ));
            return true;
        };
        tracing::info!(left, to = %target.id, mode = orcha.switch_mode.as_str(), "orcha: moving the chat on by itself");
        if !super::super::brutal::same_agent(&target.kind, &current_kind) {
            self.start_handoff(
                pane,
                target.kind.clone(),
                "summary",
                Some(target.id.clone()),
            );
            self.show_notice(format!(
                "{left}% left · handing this chat to {}",
                target.label
            ));
            return true;
        }
        let action = match orcha.switch_mode {
            crate::config::SwitchMode::NewPane => AccountAction::Open {
                id: target.id.clone(),
                cwd: self.pane_cwd(&pane).unwrap_or_default(),
                pane: Some(pane),
            },
            crate::config::SwitchMode::Restart
                if super::super::brutal::same_agent(&target.kind, "claude")
                    && crate::orcha_accounts::restart_blocker(&target.id).is_none() =>
            {
                AccountAction::Restart {
                    id: target.id.clone(),
                    pane,
                }
            }
            _ => AccountAction::Switch {
                id: target.id.clone(),
                pane,
            },
        };
        self.start_account_action(action);
        // Keep the current account until the action completes. The job's
        // working notice and result report whether the move actually happened.
        if self.orcha_ui.account_run.is_none() {
            self.orcha_ui.auto_handed_off.remove(&id);
        }
        true
    }

    /// Looks for Claude sign-ins orcha does not list, off the input path.
    /// `asked`: from the menu (says when there is none, and offers the ones
    /// dismissed before too); otherwise the quiet look once per start.
    pub(in crate::client::shell) fn start_account_scan(&mut self, asked: bool) {
        self.orcha_ui.account_scan_started = true;
        if self.orcha_ui.account_scan.is_some() {
            return;
        }
        self.orcha_ui.account_scan = BackgroundJob::spawn("orcha-account-scan", move || {
            (crate::orcha_accounts::find_unlisted(asked), asked)
        });
    }

    /// Collects the account look and offers what it found once the screen
    /// is free of other popups. Returns true when the screen changed.
    pub(in crate::client::shell) fn poll_account_scan(&mut self) -> bool {
        let mut changed = false;
        if let Some(Ok((found, asked))) =
            BackgroundJob::take_finished(&mut self.orcha_ui.account_scan)
        {
            if found.is_empty() && asked {
                self.show_notice("no other signed-in accounts found".to_owned());
                changed = true;
            }
            if asked && !found.is_empty() {
                // asked from the accounts popup: this replaces it
                self.orcha_ui.popup = None;
            }
            self.orcha_ui.found_accounts = found;
        }
        if !self.orcha_ui.found_accounts.is_empty()
            && self.orcha_ui.popup.is_none()
            && self.overlay.is_none()
        {
            self.orcha_ui.popup = Some(super::super::orcha_popup::OrchaPopup::FoundAccounts {
                selected: 0,
                chosen: vec![true; self.orcha_ui.found_accounts.len()],
            });
            changed = true;
        }
        changed
    }

    pub(super) fn toggle_found_account(&mut self, index: usize) {
        if let Some(super::super::orcha_popup::OrchaPopup::FoundAccounts { chosen, .. }) =
            self.orcha_ui.popup.as_mut()
        {
            if let Some(on) = chosen.get_mut(index) {
                *on = !*on;
            }
        }
    }

    /// IMPORT: adds the checked accounts; the unchecked ones are not
    /// offered again.
    pub(super) fn import_found_accounts(&mut self) {
        let Some(super::super::orcha_popup::OrchaPopup::FoundAccounts { chosen, .. }) =
            self.orcha_ui.popup.take()
        else {
            return;
        };
        let found = std::mem::take(&mut self.orcha_ui.found_accounts);
        let (chosen, dismissed): (Vec<_>, Vec<_>) =
            found.into_iter().zip(chosen).partition(|(_, on)| *on);
        self.start_account_action(AccountAction::Import {
            chosen: chosen.into_iter().map(|(account, _)| account).collect(),
            dismissed: dismissed.into_iter().map(|(account, _)| account).collect(),
        });
    }

    /// Not now: none of the found accounts is offered again by itself.
    pub(super) fn dismiss_found_accounts(&mut self) {
        self.orcha_ui.popup = None;
        let dismissed = std::mem::take(&mut self.orcha_ui.found_accounts);
        self.start_account_action(AccountAction::Import {
            chosen: Vec::new(),
            dismissed,
        });
    }
}
