//! Execution of the choice made in an Orcha popup.

use super::file_paths::open_with_choice;
use super::*;

impl ClientShellState {
    /// The popup's primary action on row `index`; `alternate` is ⌥↵.
    pub(super) fn popup_activate(
        &mut self,
        index: usize,
        alternate: bool,
        outcome: &mut ClientShellInput,
    ) {
        let Some(popup) = self.orcha_ui.popup.clone() else {
            return;
        };
        let orcha = self.config.orcha.clone();
        match popup {
            // the setup and the file view handle their own keys and clicks
            super::super::orcha_popup::OrchaPopup::Welcome(_)
            | super::super::orcha_popup::OrchaPopup::FileView { .. } => return,
            super::super::orcha_popup::OrchaPopup::WhatsNew { .. } => {
                self.orcha_ui.popup = None;
                return;
            }
            super::super::orcha_popup::OrchaPopup::Palette { .. } => {
                self.run_palette_row(index, outcome);
                return;
            }
            super::super::orcha_popup::OrchaPopup::ProjectIcon { .. } => {
                self.choose_project_icon(index);
                return;
            }
            super::super::orcha_popup::OrchaPopup::RemoveAccount { id, pane, .. } => {
                if index == 0 {
                    self.start_account_action(AccountAction::Remove { id });
                }
                self.orcha_ui.popup = Some(self.accounts_popup(pane));
                return;
            }
            super::super::orcha_popup::OrchaPopup::Broadcast { .. } => {
                self.send_broadcast();
                return;
            }
            super::super::orcha_popup::OrchaPopup::SwitchAccount {
                id,
                pane,
                restart_blocked,
                label,
                selected,
            } => {
                match index {
                    0 => self.start_account_action(AccountAction::Switch {
                        id: id.clone(),
                        pane,
                    }),
                    1 => {
                        if let Some(reason) = restart_blocked {
                            self.show_notice(reason.clone());
                            // keep the choice open; the other row still works
                            self.orcha_ui.popup =
                                Some(super::super::orcha_popup::OrchaPopup::SwitchAccount {
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
                self.accounts.mark_current(&id, std::time::Instant::now());
                self.update_low_fuel_hint();
                return;
            }
            super::super::orcha_popup::OrchaPopup::PluginOffer { plugin, .. } => {
                self.orcha_ui.popup = None;
                let rows = super::super::orcha_popup::plugin_offer_rows(&plugin);
                match rows.get(index).map(|row| row.0) {
                    Some(super::super::orcha_popup::PluginOfferChoice::Add { autostart }) => {
                        crate::orcha_app::answer_plugin_offer(plugin.id, false);
                        let working = format!("adding {}…", plugin.name);
                        self.orcha_ui.plugin_add_run =
                            BackgroundJob::spawn("orcha-plugin-add", move || {
                                let result =
                                    crate::orcha_app::add_offered_plugin(&plugin, autostart);
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
                return;
            }
            super::super::orcha_popup::OrchaPopup::RulesOffer { folder, rules, .. } => {
                self.orcha_ui.popup = None;
                match index {
                    0 => self.start_notice_job(
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
                    1 => {
                        self.open_settings_web_at(format!("projects:{}", folder.to_string_lossy()))
                    }
                    3 => {
                        if let Err(message) = crate::orcha_rules::decline(&folder) {
                            tracing::warn!(%message, "orcha: could not remember the declined project");
                            self.show_failure(format!("Could not remember that: {message}"));
                        }
                    }
                    _ => tracing::info!(folder = %folder.display(), "orcha: project rules put off"),
                }
                return;
            }
            super::super::orcha_popup::OrchaPopup::Checklist {
                pane,
                kind,
                file,
                left,
                ..
            } => {
                self.orcha_ui.popup = None;
                if index == 0 {
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
                } else {
                    tracing::info!(left, %kind, "orcha: hand-off checklist declined");
                }
                return;
            }
            super::super::orcha_popup::OrchaPopup::PluginRemove {
                plugin_id, name, ..
            } => {
                self.orcha_ui.popup = None;
                if index == 0 {
                    self.start_plugin_remove(plugin_id, name);
                }
                return;
            }
            super::super::orcha_popup::OrchaPopup::PluginUpdate { update, name, .. } => {
                self.orcha_ui.popup = None;
                if index == 0 {
                    self.start_plugin_update(update.id, name);
                }
                return;
            }
            super::super::orcha_popup::OrchaPopup::Update { release, .. } => {
                self.orcha_ui.popup = None;
                match index {
                    0 => self.start_update(),
                    2 => {
                        crate::orcha_app::skip_release(&release.version);
                        self.orcha_ui.update_available = None;
                        self.orcha_ui.update_note = None;
                    }
                    _ => {}
                }
                return;
            }
            // a choice opens the file; the check past the rows flips `remember`
            super::super::orcha_popup::OrchaPopup::OpenWith {
                path,
                line,
                extension,
                choices,
                remember,
                ..
            } => {
                let Some(choice) = choices.get(index) else {
                    self.toggle_open_with_remember();
                    return;
                };
                self.orcha_ui.popup = None;
                if remember && !extension.is_empty() {
                    self.save_open_with(&extension, choice);
                }
                open_with_choice(choice, &path, line);
                return;
            }
            super::super::orcha_popup::OrchaPopup::NewWorkspace { project, .. } => {
                self.orcha_ui.popup = None;
                if index == 0 && project.is_some() {
                    self.record_binding(
                        crate::input::KeybindMatch::Action(
                            crate::input::KeybindAction::NewWorkspace,
                        ),
                        outcome,
                    );
                } else if index == 1 {
                    self.browse_project_folder();
                } else if index == 2 || (index == 0 && project.is_none()) {
                    self.open_new_project_folder_overlay();
                } else if index == 3 && project.is_some() {
                    self.record_binding(
                        crate::input::KeybindMatch::Action(
                            crate::input::KeybindAction::NewWorktree,
                        ),
                        outcome,
                    );
                }
                return;
            }
            // a check flips; the button past the rows imports
            super::super::orcha_popup::OrchaPopup::FoundAccounts { chosen, .. } => {
                if index >= chosen.len() {
                    self.import_found_accounts();
                } else {
                    self.toggle_found_account(index);
                }
            }
            super::super::orcha_popup::OrchaPopup::Import { .. } => {
                if index >= self.orcha_ui.import_sources.len() {
                    self.import_chosen(outcome);
                } else {
                    self.toggle_import(index);
                }
            }
            super::super::orcha_popup::OrchaPopup::Accounts { pane, .. } => {
                let count = self.accounts.data().map_or(0, <[_]>::len);
                if index >= count {
                    let names = self
                        .accounts
                        .data()
                        .map(super::super::orcha_popup::account_name_choices)
                        .unwrap_or_default();
                    self.orcha_ui.popup = Some(super::super::orcha_popup::OrchaPopup::AddAccount {
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
                if account.current && !alternate && !account.signed_out() {
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
                let current_kind = self
                    .accounts
                    .data()
                    .and_then(|data| data.iter().find(|a| a.current).map(|a| a.kind.clone()));
                if let (false, Some(pane), Some(kind)) = (signed_out, &pane, &current_kind) {
                    if *kind != account.kind {
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
                let (custom, pairs, native): (&str, Vec<(&str, String)>, AccountAction) =
                    if signed_out {
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
                    if let (AccountAction::Switch { id, pane }, "claude") =
                        (&native, account.kind.as_str())
                    {
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
                            self.accounts
                                .mark_current(&account.id, std::time::Instant::now());
                            self.update_low_fuel_hint();
                            return;
                        }
                        self.orcha_ui.popup =
                            Some(super::super::orcha_popup::OrchaPopup::SwitchAccount {
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
                    run_detached(fill(custom, &pairs), &env);
                }
                if signed_out || alternate || pane.is_none() {
                    self.accounts.refresh_soon();
                } else {
                    // the switch shows at once; the data catches up
                    self.accounts
                        .mark_current(&account.id, std::time::Instant::now());
                    self.update_low_fuel_hint();
                }
            }
            super::super::orcha_popup::OrchaPopup::NewAgent { workspace, cwd, .. } => {
                let account = self
                    .accounts
                    .data()
                    .and_then(|data| data.get(index))
                    .cloned();
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
                            workspace: workspace.clone(),
                            cwd: cwd.clone(),
                        });
                    }
                    Some(account) => run_detached(
                        fill(
                            &orcha.new_agent_command,
                            &[
                                ("id", &account.id),
                                ("workspace", &workspace),
                                ("cwd", &cwd),
                            ],
                        ),
                        &[],
                    ),
                    // the last row: a plain shell tab
                    None => self.record_binding(
                        crate::input::KeybindMatch::Action(crate::input::KeybindAction::NewTab),
                        outcome,
                    ),
                }
                return;
            }
            super::super::orcha_popup::OrchaPopup::AddAccount { names, typed, .. } => {
                let name = if index < names.len() {
                    names[index].clone()
                } else {
                    typed
                };
                self.add_claude_account(&name);
            }
            super::super::orcha_popup::OrchaPopup::Handoff { pane, targets, .. } => {
                let Some((agent, mode, _)) = targets
                    .get(index)
                    .and_then(|target| super::super::orcha_popup::HANDOFF_TARGETS.get(*target))
                else {
                    return;
                };
                self.start_handoff(pane, (*agent).to_owned(), mode, None);
            }
        }
        self.orcha_ui.popup = None;
    }
}
