//! Execution of the choice made in an Orcha popup: one dispatch on the
//! open popup, one function per popup.

use super::super::orcha_popup::{OpenChoice, OrchaPopup, PopupButton, PopupTarget};
use super::file_paths::open_with_choice;
use super::*;

impl ClientShellState {
    /// The popup's action on `target` (a row's action for ↵ on the cursor's
    /// row); `alternate` is ⌥↵.
    pub(super) fn popup_activate(
        &mut self,
        target: PopupTarget,
        alternate: bool,
        outcome: &mut ClientShellInput,
    ) {
        let Some(popup) = self.orcha_ui.popup.clone() else {
            return;
        };
        let row = target.row();
        match popup {
            // the setup and the file view handle their own keys and clicks
            OrchaPopup::Welcome(_) | OrchaPopup::FileView { .. } => {}
            OrchaPopup::WhatsNew { .. } => self.orcha_ui.popup = None,
            OrchaPopup::Broadcast { .. } => self.send_broadcast(),
            OrchaPopup::Palette { .. } => {
                if let Some(row) = row {
                    self.run_palette_row(row, outcome);
                }
            }
            OrchaPopup::ProjectIcon { .. } => {
                if let Some(row) = row {
                    self.choose_project_icon(row);
                }
            }
            OrchaPopup::RemoveAccount { id, pane, .. } => {
                self.activate_remove_account(row, id, pane)
            }
            OrchaPopup::SwitchAccount {
                selected,
                id,
                label,
                pane,
                restart_blocked,
            } => self.activate_switch_account(row, selected, id, label, pane, restart_blocked),
            OrchaPopup::PluginOffer { plugin, .. } => self.activate_plugin_offer(row, plugin),
            OrchaPopup::RulesOffer { folder, rules, .. } => {
                self.activate_rules_offer(row, folder, rules)
            }
            OrchaPopup::Checklist {
                pane,
                kind,
                file,
                left,
                ..
            } => self.activate_checklist(row, pane, kind, file, left),
            OrchaPopup::FolderAccess { .. } => self.activate_folder_access(row),
            OrchaPopup::PluginRemove {
                plugin_id, name, ..
            } => {
                self.orcha_ui.popup = None;
                if row == Some(0) {
                    self.start_plugin_remove(plugin_id, name);
                }
            }
            OrchaPopup::PluginUpdate { update, name, .. } => {
                self.orcha_ui.popup = None;
                if row == Some(0) {
                    self.start_plugin_update(update.id, name);
                }
            }
            OrchaPopup::Update { release, .. } => self.activate_update(row, &release),
            OrchaPopup::OpenWith {
                path,
                line,
                extension,
                choices,
                remember,
                ..
            } => self.activate_open_with(target, &path, line, &extension, &choices, remember),
            OrchaPopup::NewWorkspace { project, .. } => {
                self.activate_new_workspace(row, project.is_some(), outcome)
            }
            // a check flips (the list stays open); the button under it imports
            OrchaPopup::FoundAccounts { .. } => match target {
                PopupTarget::Button(PopupButton::Primary) => self.import_found_accounts(),
                PopupTarget::Row(index) => self.toggle_found_account(index),
                _ => {}
            },
            OrchaPopup::Import { .. } => match target {
                PopupTarget::Button(PopupButton::Primary) => self.import_chosen(outcome),
                PopupTarget::Row(index) => self.toggle_import(index),
                _ => {}
            },
            OrchaPopup::Accounts { pane, .. } => {
                if let Some(row) = row {
                    self.activate_accounts(row, pane, alternate);
                }
            }
            OrchaPopup::NewAgent { workspace, cwd, .. } => {
                self.activate_new_agent(row, &workspace, &cwd, outcome)
            }
            OrchaPopup::AddAccount { names, typed, .. } => {
                let name = match row.and_then(|row| names.get(row)) {
                    Some(name) => name.clone(),
                    None => typed,
                };
                self.add_claude_account(&name);
                self.orcha_ui.popup = None;
            }
            OrchaPopup::Handoff { pane, targets, .. } => {
                let all = super::super::orcha_popup::handoff_targets();
                let Some((agent, mode)) = row
                    .and_then(|row| targets.get(row))
                    .and_then(|target| all.get(*target))
                else {
                    return;
                };
                self.start_handoff(pane, agent.id.to_owned(), mode.as_str(), None);
                self.orcha_ui.popup = None;
            }
        }
    }

    /// REMOVE ACCOUNT: the first row removes it; either way back to the
    /// accounts.
    fn activate_remove_account(&mut self, row: Option<usize>, id: String, pane: Option<String>) {
        if row == Some(0) {
            self.start_account_action(AccountAction::Remove { id });
        }
        self.orcha_ui.popup = Some(self.accounts_popup(pane));
    }

    /// How a Claude chat moves to another account: keep it running on the
    /// new sign-in (row 0) or restart Claude on it (row 1).
    fn activate_switch_account(
        &mut self,
        row: Option<usize>,
        selected: usize,
        id: String,
        label: String,
        pane: String,
        restart_blocked: Option<String>,
    ) {
        match row {
            Some(0) => self.start_account_action(AccountAction::Switch {
                id: id.clone(),
                pane,
            }),
            Some(1) => {
                if let Some(reason) = restart_blocked {
                    self.show_notice(reason.clone());
                    // keep the choice open; the other row still works
                    self.orcha_ui.popup = Some(OrchaPopup::SwitchAccount {
                        selected,
                        id,
                        label,
                        pane,
                        restart_blocked: Some(reason),
                    });
                    return;
                }
                self.start_account_action(AccountAction::Restart {
                    id: id.clone(),
                    pane,
                });
            }
            _ => {
                self.orcha_ui.popup = None;
                return;
            }
        }
        self.orcha_ui.popup = None;
    }

    /// PLUGIN: add it (maybe started with orcha), not now, or never.
    fn activate_plugin_offer(
        &mut self,
        row: Option<usize>,
        plugin: crate::orcha_app::OfferedPlugin,
    ) {
        self.orcha_ui.popup = None;
        let rows = super::super::orcha_popup::plugin_offer_rows(&plugin);
        match row.and_then(|row| rows.get(row)).map(|row| row.0) {
            Some(super::super::orcha_popup::PluginOfferChoice::Add { autostart }) => {
                crate::orcha_app::answer_plugin_offer(plugin.id, false);
                let working = format!("adding {}…", plugin.name);
                self.orcha_ui.plugin_add_run =
                    BackgroundJob::spawn("orcha-plugin-add", move || {
                        let result = crate::orcha_app::add_offered_plugin(&plugin, autostart);
                        (plugin.id, plugin.name, result)
                    });
                if self.orcha_ui.plugin_add_run.is_some() {
                    self.start_working_notice("orcha-plugin-add", working);
                }
            }
            Some(super::super::orcha_popup::PluginOfferChoice::NotNow) => {
                crate::orcha_app::answer_plugin_offer(plugin.id, false);
            }
            Some(super::super::orcha_popup::PluginOfferChoice::Never) => {
                crate::orcha_app::answer_plugin_offer(plugin.id, true);
            }
            None => {}
        }
    }

    /// WORKING RULES: add them, open the project's settings, later, or
    /// never for this project.
    fn activate_rules_offer(
        &mut self,
        row: Option<usize>,
        folder: std::path::PathBuf,
        rules: Vec<String>,
    ) {
        self.orcha_ui.popup = None;
        match row {
            Some(0) => self.start_notice_job(
                "orcha-rules",
                Some("adding the working rules…".to_owned()),
                move || match crate::orcha_rules::apply(&folder, &rules) {
                    Ok(file) => Ok(format!(
                        "working rules added to {}",
                        file.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "the project".to_owned())
                    )),
                    Err(message) => {
                        tracing::warn!(%message, "orcha: project rules not written");
                        Err(format!("Working rules not added: {message}"))
                    }
                },
            ),
            Some(1) => self.open_settings_web_at(format!("projects:{}", folder.to_string_lossy())),
            Some(3) => {
                if let Err(message) = crate::orcha_rules::decline(&folder) {
                    tracing::warn!(%message, "orcha: could not remember the declined project");
                    self.show_failure(format!("Could not remember that: {message}"));
                }
            }
            _ => tracing::info!(folder = %folder.display(), "orcha: project rules put off"),
        }
    }

    /// CHECKLIST: the first row asks the agent to write its hand-off
    /// checklist.
    fn activate_checklist(
        &mut self,
        row: Option<usize>,
        pane: String,
        kind: String,
        file: String,
        left: u8,
    ) {
        self.orcha_ui.popup = None;
        if row != Some(0) {
            tracing::info!(left, %kind, "orcha: hand-off checklist declined");
            return;
        }
        let prompt = checklist_prompt(left, &file);
        tracing::info!(left, %file, %kind, "orcha: asked the agent for a hand-off checklist");
        self.show_notice(format!("asked {kind} to write its checklist to {file}"));
        self.start_notice_job("orcha-checklist", None, move || {
            if crate::orcha_handoff::send_prompt(&pane, &prompt) {
                Ok(String::new())
            } else {
                Err("Could not type into the agent".to_owned())
            }
        });
    }

    /// UPDATE: now, later, or skip this version.
    fn activate_update(&mut self, row: Option<usize>, release: &crate::orcha_app::Release) {
        self.orcha_ui.popup = None;
        match row {
            Some(0) => self.start_update(),
            Some(2) => {
                crate::orcha_app::skip_release(&release.version);
                self.orcha_ui.update_available = None;
                self.orcha_ui.update_note = None;
            }
            _ => {}
        }
    }

    /// OPEN WITH: a choice opens the file; the check flips `remember`.
    fn activate_open_with(
        &mut self,
        target: PopupTarget,
        path: &str,
        line: u32,
        extension: &str,
        choices: &[OpenChoice],
        remember: bool,
    ) {
        let Some(choice) = target.row().and_then(|row| choices.get(row)) else {
            self.toggle_open_with_remember();
            return;
        };
        self.orcha_ui.popup = None;
        if remember && !extension.is_empty() {
            self.save_open_with(extension, choice);
        }
        open_with_choice(choice, path, line);
    }

    /// NEW PROJECT: the current project, another folder, a typed path, or
    /// a new worktree.
    fn activate_new_workspace(
        &mut self,
        row: Option<usize>,
        has_project: bool,
        outcome: &mut ClientShellInput,
    ) {
        self.orcha_ui.popup = None;
        match row {
            Some(0) if has_project => self.record_binding(
                crate::input::KeybindMatch::Action(crate::input::KeybindAction::NewWorkspace),
                outcome,
            ),
            Some(1) => self.browse_project_folder(),
            Some(0 | 2) => self.open_new_project_folder_overlay(),
            Some(3) if has_project => self.record_binding(
                crate::input::KeybindMatch::Action(crate::input::KeybindAction::NewWorktree),
                outcome,
            ),
            _ => {}
        }
    }

    /// NEW AGENT: an account starts an agent (or signs in first); then a
    /// plain shell tab, then NEW PROJECT.
    fn activate_new_agent(
        &mut self,
        row: Option<usize>,
        workspace: &str,
        cwd: &str,
        outcome: &mut ClientShellInput,
    ) {
        let Some(row) = row else {
            return;
        };
        let orcha = self.config.orcha.clone();
        let count = self.accounts.data().map_or(0, <[_]>::len);
        if row > count {
            self.open_new_workspace_popup();
            return;
        }
        let account = self.accounts.data().and_then(|data| data.get(row)).cloned();
        self.orcha_ui.popup = None;
        match account {
            Some(account) if account.signed_out() => {
                if orcha.login_command.trim().is_empty() {
                    self.start_account_action(AccountAction::Login {
                        id: account.id.clone(),
                        pane: None,
                    });
                } else {
                    run_detached(fill(&orcha.login_command, &[("id", &account.id)]), &[]);
                }
            }
            Some(account) if orcha.new_agent_command.trim().is_empty() => {
                self.start_account_action(AccountAction::NewAgent {
                    id: account.id.clone(),
                    workspace: workspace.to_owned(),
                    cwd: cwd.to_owned(),
                });
            }
            Some(account) => run_detached(
                fill(
                    &orcha.new_agent_command,
                    &[("id", &account.id), ("workspace", workspace), ("cwd", cwd)],
                ),
                &[],
            ),
            // past the accounts: a plain shell tab
            None => self.record_binding(
                crate::input::KeybindMatch::Action(crate::input::KeybindAction::NewTab),
                outcome,
            ),
        }
    }

    /// ACCOUNTS, row `index`: an account continues the chat in `pane`
    /// (switched, opened, handed off or signed in first); the row past the
    /// accounts is ADD ACCOUNT.
    fn activate_accounts(&mut self, index: usize, pane: Option<String>, alternate: bool) {
        let orcha = self.config.orcha.clone();
        let count = self.accounts.data().map_or(0, <[_]>::len);
        if index >= count {
            let names = self
                .accounts
                .data()
                .map(super::super::orcha_popup::account_name_choices)
                .unwrap_or_default();
            self.orcha_ui.popup = Some(OrchaPopup::AddAccount {
                selected: 0,
                names,
                typed: String::new(),
                pane,
            });
            return;
        }
        let Some(account) = self
            .accounts
            .data()
            .and_then(|data| data.get(index))
            .cloned()
        else {
            return;
        };
        let current_kind = pane.as_deref().and_then(|pane| {
            self.snapshot
                .as_deref()?
                .agents
                .iter()
                .find(|agent| agent.pane_id == pane)?
                .agent
                .clone()
        });
        let same_provider = current_kind
            .as_deref()
            .is_none_or(|kind| super::super::brutal::same_agent(kind, &account.kind));
        if account.current && same_provider && !alternate && !account.signed_out() {
            return;
        }
        let cwd = pane
            .as_deref()
            .and_then(|pane| self.pane_cwd(pane))
            .unwrap_or_default();
        let mut env = vec![("HERDR_ACTIVE_PANE_CWD", cwd.clone())];
        if let Some(pane) = &pane {
            env.push(("HERDR_ACTIVE_PANE_ID", pane.clone()));
        }
        let signed_out = account.signed_out();
        // another agent than the chat's: hand the chat off to it
        if let (false, Some(pane), Some(kind)) = (signed_out, &pane, &current_kind) {
            if !super::super::brutal::same_agent(kind, &account.kind) {
                let mode = if alternate { "summary" } else { "full" };
                self.start_handoff(
                    pane.clone(),
                    account.kind.clone(),
                    mode,
                    Some(account.id.clone()),
                );
                self.orcha_ui.popup = None;
                return;
            }
        }
        let (custom, pairs, native): (&str, Vec<(&str, String)>, AccountAction) = if signed_out {
            (
                &orcha.login_command,
                vec![("id", account.id.clone())],
                AccountAction::Login {
                    id: account.id.clone(),
                    pane: pane.clone(),
                },
            )
        } else if alternate
            || pane.is_none()
            || orcha.switch_mode == crate::config::SwitchMode::NewPane
        {
            (
                &orcha.open_command,
                vec![("id", account.id.clone())],
                AccountAction::Open {
                    id: account.id.clone(),
                    pane: pane.clone(),
                    cwd: cwd.clone(),
                },
            )
        } else {
            (
                &orcha.switch_command,
                vec![
                    ("id", account.id.clone()),
                    ("pane", pane.clone().unwrap_or_default()),
                ],
                AccountAction::Switch {
                    id: account.id.clone(),
                    pane: pane.clone().unwrap_or_default(),
                },
            )
        };
        if custom.trim().is_empty() {
            // A Claude chat can keep running on the new sign-in or
            // restart on it: ask which.
            if let (AccountAction::Switch { id, pane }, true) = (
                &native,
                super::super::brutal::same_agent(&account.kind, "claude"),
            ) {
                let blocked = crate::orcha_accounts::restart_blocker(id);
                let chosen = match orcha.switch_mode {
                    crate::config::SwitchMode::Keep => Some(native.clone()),
                    crate::config::SwitchMode::Restart if blocked.is_none() => {
                        Some(AccountAction::Restart {
                            id: id.clone(),
                            pane: pane.clone(),
                        })
                    }
                    _ => None,
                };
                if let Some(action) = chosen {
                    self.orcha_ui.popup = None;
                    self.start_account_action(action);
                    return;
                }
                self.orcha_ui.popup = Some(OrchaPopup::SwitchAccount {
                    selected: 0,
                    id: id.clone(),
                    label: account.label.clone(),
                    pane: pane.clone(),
                    restart_blocked: blocked,
                });
                return;
            }
            self.start_account_action(native);
        } else {
            let pairs: Vec<(&str, &str)> = pairs
                .iter()
                .map(|(key, value)| (*key, value.as_str()))
                .collect();
            self.accounts.forget_pane_owners();
            run_detached(fill(custom, &pairs), &env);
        }
        // Custom commands do not report their completion. Re-read the real
        // account instead of claiming a switch succeeded when it only started.
        self.accounts.refresh_soon();
        self.orcha_ui.popup = None;
    }
}
