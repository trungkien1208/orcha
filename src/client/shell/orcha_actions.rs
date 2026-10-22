//! orcha: what the orcha chrome's buttons do.

use super::*;

/// Opens a folder in the IDE: the first editor found on PATH, else the
/// system's default handler.
const DETECT_IDE: &str = r#"for c in cursor code zed subl idea; do command -v "$c" >/dev/null 2>&1 && exec "$c" "$P"; done; open "$P" 2>/dev/null || xdg-open "$P""#;
/// Opens a folder in a Git GUI: the first app found, else the folder.
const DETECT_GIT_GUI: &str = r#"for a in Fork SourceTree "GitHub Desktop" Tower GitKraken; do [ -d "/Applications/$a.app" ] && exec open -a "$a" "$P"; done; command -v gitk >/dev/null 2>&1 && cd "$P" && exec gitk --all; open "$P" 2>/dev/null || xdg-open "$P""#;

/// Runs `command` with `$P` (and `{path}`) set to `path`, without waiting.
pub(super) fn spawn_with_path(command: &str, path: &str) {
    let script = command.replace("{path}", "\"$P\"");
    let child = std::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .env("P", path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match child {
        Ok(mut child) => {
            // reap it off the input path
            let _ = crate::thread_spawn::spawn_named("orcha-open", move || {
                let _ = child.wait();
            });
        }
        Err(err) => tracing::warn!(err = %err, "orcha: failed to open {path}"),
    }
}

impl ClientShellState {
    /// Clicks on the orcha chrome. Returns true when one was handled.
    pub(super) fn handle_orcha_click(&mut self, point: (u16, u16), outcome: &mut ClientShellInput) -> bool {
        let hits = &self.hits.orcha;
        if super::contains(hits.settings, point) {
            self.record_binding(
                crate::input::KeybindMatch::Action(crate::input::KeybindAction::Settings),
                outcome,
            );
            return true;
        }
        if super::contains(hits.fuel, point) || super::contains(hits.fuel_hint, point) {
            self.open_accounts_overlay();
            outcome.repaint = true;
            return true;
        }
        if let Some(id) = hits
            .project_toggles
            .iter()
            .find(|(rect, _)| super::contains(*rect, point))
            .map(|(_, id)| id.clone())
        {
            let open = &mut self.orcha_ui.open_projects;
            if !open.remove(&id) {
                open.insert(id);
            }
            outcome.repaint = true;
            return true;
        }
        if super::contains(hits.history_toggle, point) {
            self.orcha_ui.history_open = !self.orcha_ui.history_open;
            outcome.repaint = true;
            return true;
        }
        if super::contains(hits.worktrees_toggle, point) {
            self.orcha_ui.worktrees_open = !self.orcha_ui.worktrees_open;
            outcome.repaint = true;
            return true;
        }
        if super::contains(hits.new_worktree, point) {
            self.record_binding(
                crate::input::KeybindMatch::Action(crate::input::KeybindAction::NewWorktree),
                outcome,
            );
            return true;
        }
        let ide = super::contains(hits.ide, point);
        if ide || super::contains(hits.git_gui, point) {
            if let Some(root) = self.git_panel.data().map(|data| data.root.clone()) {
                let configured = if ide {
                    &self.config.orcha.ide
                } else {
                    &self.config.orcha.git_gui
                };
                let command = if configured.trim().is_empty() {
                    if ide {
                        DETECT_IDE
                    } else {
                        DETECT_GIT_GUI
                    }
                } else {
                    configured.as_str()
                };
                spawn_with_path(command, &root);
            }
            return true;
        }
        false
    }
}

fn fill(command: &str, pairs: &[(&str, &str)]) -> String {
    let mut out = command.to_owned();
    for (key, value) in pairs {
        // values are ids from the server or fixed words; quote them anyway
        let quoted = format!("'{}'", value.replace('\'', ""));
        out = out.replace(&format!("{{{key}}}"), &quoted);
    }
    out
}

/// Runs an orcha command in the background against this session.
fn run_detached(command: String, extra_env: &[(&str, String)]) {
    let mut cmd = super::accounts::orcha_command(&command);
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    match cmd.spawn() {
        Ok(mut child) => {
            let _ = crate::thread_spawn::spawn_named("orcha-command", move || {
                let _ = child.wait();
            });
        }
        Err(err) => tracing::warn!(err = %err, "orcha: failed to run {command}"),
    }
}

impl ClientShellState {
    pub(super) fn open_accounts_overlay(&mut self) {
        let pane = self
            .snapshot
            .as_deref()
            .and_then(super::orcha_popup::focused_agent_pane);
        let selected = self
            .accounts
            .data()
            .and_then(|accounts| {
                super::orcha_popup::best_account(accounts)
                    .or_else(|| accounts.iter().position(|account| account.current))
            })
            .unwrap_or(0);
        self.orcha_ui.popup = Some(super::orcha_popup::OrchaPopup::Accounts { selected, pane });
        self.accounts.refresh_soon();
    }

    pub(super) fn open_handoff_popup(&mut self) {
        let Some(pane) = self
            .snapshot
            .as_deref()
            .and_then(super::orcha_popup::focused_agent_pane)
        else {
            return;
        };
        let from = self.snapshot.as_deref().and_then(|snapshot| {
            snapshot
                .agents
                .iter()
                .find(|agent| agent.pane_id == pane)
                .and_then(|agent| agent.agent.clone())
        });
        // start on the first target that is another agent
        let selected = super::orcha_popup::HANDOFF_TARGETS
            .iter()
            .position(|(kind, _, _)| Some(*kind) != from.as_deref())
            .unwrap_or(0);
        self.orcha_ui.popup = Some(super::orcha_popup::OrchaPopup::Handoff { selected, pane });
    }

    fn popup_len(&self) -> usize {
        match &self.orcha_ui.popup {
            Some(super::orcha_popup::OrchaPopup::Accounts { .. }) => {
                self.accounts.data().map_or(0, <[_]>::len)
            }
            Some(super::orcha_popup::OrchaPopup::Handoff { .. }) => {
                super::orcha_popup::HANDOFF_TARGETS.len()
            }
            None => 0,
        }
    }

    fn popup_select(&mut self, index: usize) {
        match &mut self.orcha_ui.popup {
            Some(
                super::orcha_popup::OrchaPopup::Accounts { selected, .. }
                | super::orcha_popup::OrchaPopup::Handoff { selected, .. },
            ) => *selected = index,
            None => {}
        }
    }

    fn popup_selected(&self) -> usize {
        match &self.orcha_ui.popup {
            Some(
                super::orcha_popup::OrchaPopup::Accounts { selected, .. }
                | super::orcha_popup::OrchaPopup::Handoff { selected, .. },
            ) => *selected,
            None => 0,
        }
    }

    /// The popup's primary action on row `index`; `alternate` is ⌥↵.
    fn popup_activate(&mut self, index: usize, alternate: bool) {
        let Some(popup) = self.orcha_ui.popup.clone() else {
            return;
        };
        let orcha = self.config.orcha.clone();
        match popup {
            super::orcha_popup::OrchaPopup::Accounts { pane, .. } => {
                let Some(account) = self.accounts.data().and_then(|data| data.get(index)).cloned()
                else {
                    return;
                };
                if account.current && !alternate {
                    return;
                }
                let cwd = pane
                    .as_deref()
                    .and_then(|pane| {
                        self.snapshot.as_deref().and_then(|snapshot| {
                            snapshot.panes.iter().find(|p| p.pane_id == pane).and_then(|p| {
                                p.foreground_cwd.clone().or_else(|| p.cwd.clone())
                            })
                        })
                    })
                    .unwrap_or_default();
                let mut env = vec![("HERDR_ACTIVE_PANE_CWD", cwd)];
                if let Some(pane) = &pane {
                    env.push(("HERDR_ACTIVE_PANE_ID", pane.clone()));
                }
                let signed_out = account.error.is_some() && account.windows.is_empty();
                let command = if signed_out {
                    fill(&orcha.login_command, &[("id", &account.id)])
                } else if alternate || pane.is_none() {
                    fill(&orcha.open_command, &[("id", &account.id)])
                } else {
                    fill(
                        &orcha.switch_command,
                        &[("id", &account.id), ("pane", pane.as_deref().unwrap_or(""))],
                    )
                };
                run_detached(command, &env);
                self.accounts.refresh_soon();
            }
            super::orcha_popup::OrchaPopup::Handoff { pane, .. } => {
                let Some((agent, mode, _)) = super::orcha_popup::HANDOFF_TARGETS.get(index) else {
                    return;
                };
                let command = fill(
                    &orcha.handoff_command,
                    &[("pane", &pane), ("agent", agent), ("mode", mode)],
                );
                run_detached(command, &[]);
            }
        }
        self.orcha_ui.popup = None;
    }

    /// Keys while an orcha popup is open: arrows/j k move, ↵ acts, ⌥↵ the
    /// alternate action, H hands off, esc closes.
    pub(super) fn route_orcha_popup_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        use crossterm::event::{KeyCode, KeyModifiers};
        outcome.repaint = true;
        let len = self.popup_len();
        let selected = self.popup_selected();
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.orcha_ui.popup = None,
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab if len > 0 => {
                self.popup_select((selected + 1) % len);
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab if len > 0 => {
                self.popup_select((selected + len - 1) % len);
            }
            KeyCode::Enter => {
                let alternate = key.modifiers.contains(KeyModifiers::ALT);
                self.popup_activate(selected, alternate);
            }
            KeyCode::Char('h') | KeyCode::Char('H')
                if matches!(
                    self.orcha_ui.popup,
                    Some(super::orcha_popup::OrchaPopup::Accounts { .. })
                ) =>
            {
                self.open_handoff_popup();
            }
            _ => {}
        }
    }

    /// Mouse while an orcha popup is open: rows select, buttons act, a click
    /// outside or on ✕ closes.
    pub(super) fn handle_orcha_popup_mouse(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        outcome: &mut ClientShellInput,
    ) {
        use crossterm::event::{MouseButton, MouseEventKind};
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let point = (mouse.column, mouse.row);
        let hits = self.hits.orcha.popup.clone();
        outcome.repaint = true;
        if super::contains(hits.close, point) || !super::contains(hits.area, point) {
            self.orcha_ui.popup = None;
            return;
        }
        if let Some((_, index)) = hits.actions.iter().find(|(rect, _)| super::contains(*rect, point)) {
            self.popup_activate(*index, false);
            return;
        }
        if let Some((_, index)) = hits.rows.iter().find(|(rect, _)| super::contains(*rect, point)) {
            self.popup_select(*index);
        }
    }

    /// Low fuel: when the focused agent's account drops under
    /// `low_fuel_percent`, the header offers a switch to the account with
    /// the most fuel. Returns true when the hint changed.
    pub(super) fn update_low_fuel_hint(&mut self) -> bool {
        let hint = self.accounts.data().and_then(|accounts| {
            let current = accounts.iter().find(|account| account.current)?;
            let left = current.lowest()?;
            if left >= self.config.orcha.low_fuel_percent {
                return None;
            }
            let best = super::orcha_popup::best_account(accounts)
                .and_then(|index| accounts.get(index))
                .filter(|best| best.lowest().unwrap_or(0) > left);
            Some(match best {
                Some(best) => format!(
                    "FUEL {left}% · switch to {} ({}% left)",
                    best.label,
                    best.lowest().unwrap_or(0)
                ),
                None => format!("FUEL {left}% · hand off to another agent"),
            })
        });
        let changed = hint != self.orcha_ui.low_fuel_hint;
        self.orcha_ui.low_fuel_hint = hint;
        changed
    }

    /// Focuses the next agent that needs you, across all projects.
    pub(super) fn jump_to_needs_you(&mut self, outcome: &mut ClientShellInput) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let focused = snapshot.focused_pane_id.clone();
        let waiting: Vec<String> = snapshot
            .agents
            .iter()
            .filter(|agent| agent.agent_status == crate::api::schema::AgentStatus::Blocked)
            .map(|agent| agent.pane_id.clone())
            .collect();
        if waiting.is_empty() {
            return;
        }
        let next = waiting
            .iter()
            .position(|pane| Some(pane) == focused.as_ref())
            .map_or(0, |index| (index + 1) % waiting.len());
        let pane = waiting[next].clone();
        self.focus_agent_pane(&pane, outcome);
    }

    fn focus_agent_pane(&mut self, pane: &str, outcome: &mut ClientShellInput) {
        self.push_endpoint_method(
            crate::api::schema::Method::PaneFocus(crate::api::schema::PaneTarget {
                pane_id: pane.to_owned(),
            }),
            outcome,
        );
        outcome.repaint = true;
    }
}
