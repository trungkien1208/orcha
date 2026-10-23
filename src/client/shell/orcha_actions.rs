//! orcha: what the orcha chrome's buttons do.

use super::*;

/// "src/a.ts:12:4" → ("src/a.ts", Some(12)).
fn split_line(text: &str) -> (&str, Option<u32>) {
    let Some((before_last, last)) = text.rsplit_once(':') else {
        return (text, None);
    };
    let Ok(last_number) = last.parse::<u32>() else {
        return (text, None);
    };
    if let Some((path, line)) = before_last.rsplit_once(':') {
        if let Ok(line) = line.parse::<u32>() {
            return (path, Some(line));
        }
    }
    (before_last, Some(last_number))
}

/// Runs `command` with `$P` (and `{path}`) set to `path`, without waiting.
pub(super) fn spawn_with_path(command: &str, path: &str) {
    spawn_with_env(command, path, 1);
}

fn spawn_with_env(command: &str, path: &str, line: u32) {
    let script = command.replace("{path}", "\"$P\"");
    let child = crate::platform::orcha_shell_command(&script)
        .env("P", path)
        .env("L", line.to_string())
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

/// Files an editor can't show well open in the system's app for their type:
/// pictures, PDFs, media, office documents, web pages, archives. Everything
/// else (code, text, config, unknown types) goes to the editor at its line.
fn opens_in_system_app(path: &std::path::Path) -> bool {
    const SYSTEM_APP_EXTENSIONS: &[&str] = &[
        // pictures
        "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "ico", "heic", "avif",
        // documents
        "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "pages", "numbers",
        "key", "rtf", "epub", // web pages open in the browser
        "html", "htm", // media
        "mp4", "mov", "m4v", "avi", "mkv", "webm", "mp3", "wav", "m4a", "aac", "flac", "ogg",
        // archives and installers
        "zip", "tar", "gz", "tgz", "7z", "rar", "dmg", "pkg", "msi",
    ];
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            SYSTEM_APP_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
        })
}

/// Folders skipped when looking for a bare file name.
const SKIPPED_DIRS: &[&str] = &["node_modules", "target", "dist", "build", "vendor"];

/// Breadth-first search for `name` under `root`, shallowest match first,
/// bounded so a click never stalls on a huge tree.
fn find_file_named(root: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    const MAX_ENTRIES: usize = 20_000;
    let mut seen = 0usize;
    let mut queue = std::collections::VecDeque::from([root.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut subdirs = Vec::new();
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES {
                return None;
            }
            let file_name = entry.file_name();
            let Some(file_name) = file_name.to_str() else {
                continue;
            };
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if !file_name.starts_with('.') && !SKIPPED_DIRS.contains(&file_name) {
                    subdirs.push(entry.path());
                }
            } else if file_name == name {
                return Some(entry.path());
            }
        }
        subdirs.sort();
        queue.extend(subdirs);
    }
    None
}

/// The apps offered for a file, the suggested one first: the system's app
/// for documents and pictures, the editor for code and text.
fn open_choices(path: &std::path::Path, ide: &str) -> Vec<super::orcha_popup::OpenChoice> {
    use super::orcha_popup::OpenChoice;
    let mut editors = vec![OpenChoice::Editor {
        name: String::new(),
        command: if ide.trim().is_empty() {
            String::new()
        } else {
            ide.to_owned()
        },
    }];
    // Installed editors, at most four, besides the configured one.
    for (name, command) in super::settings::installed_apps(super::settings::IDE_APPS)
        .into_iter()
        .filter(|(_, command)| !command.is_empty() && command != ide)
        .take(4)
    {
        editors.push(OpenChoice::Editor { name, command });
    }
    let mut choices = Vec::new();
    if opens_in_system_app(path) {
        choices.push(OpenChoice::System);
        choices.extend(editors);
    } else {
        choices.extend(editors);
        choices.push(OpenChoice::System);
    }
    choices.push(OpenChoice::Reveal);
    choices
}

fn open_with_choice(choice: &super::orcha_popup::OpenChoice, path: &str, line: u32) {
    use super::orcha_popup::OpenChoice;
    match choice {
        OpenChoice::System => {
            open_orcha_target(crate::platform::OrchaOpenTarget::SystemDefault, path, 1);
        }
        OpenChoice::Reveal => {
            let folder = std::path::Path::new(path)
                .parent()
                .map_or_else(|| path.to_owned(), |dir| dir.to_string_lossy().into_owned());
            open_orcha_target(crate::platform::OrchaOpenTarget::SystemDefault, &folder, 1);
        }
        OpenChoice::Editor { command, .. } if command.trim().is_empty() => {
            open_orcha_target(crate::platform::OrchaOpenTarget::File, path, line);
        }
        OpenChoice::Editor { command, .. } => spawn_with_env(command, path, line),
    }
}

fn open_orcha_target(target: crate::platform::OrchaOpenTarget, path: &str, line: u32) {
    let child = crate::platform::open_orcha_target(target, std::path::Path::new(path), line)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match child {
        Ok(mut child) => {
            let _ = crate::thread_spawn::spawn_named("orcha-open", move || {
                let _ = child.wait();
            });
        }
        Err(err) => tracing::warn!(err = %err, "orcha: failed to open {path}"),
    }
}

impl ClientShellState {
    /// The page color while the orcha layout is on, for the host terminal's
    /// default background.
    pub(crate) fn orcha_page_rgb(&self) -> Option<(u8, u8, u8)> {
        if !(self.config.status_bar && self.config.sidebar_tree) {
            return None;
        }
        match super::brutal::Brutal::new(&self.config.palette).bg {
            ratatui::style::Color::Rgb(r, g, b) => Some((r, g, b)),
            _ => None,
        }
    }

    /// FUEL moves between the panel and the status bar; the choice is kept
    /// in the config (`[orcha] fuel`).
    fn move_fuel(&mut self, to: crate::config::FuelPlacement) {
        self.config.orcha.fuel = to;
        let value = match to {
            crate::config::FuelPlacement::Panel => "\"panel\"",
            crate::config::FuelPlacement::StatusBar => "\"status_bar\"",
        };
        let path = crate::config::config_path();
        let _ = crate::thread_spawn::spawn_named("orcha-fuel-pref", move || {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            let updated = crate::config::upsert_section_value(&content, "orcha", "fuel", value);
            if let Err(err) = std::fs::write(&path, updated) {
                tracing::warn!(err = %err, "orcha: failed to save the fuel placement");
            }
        });
    }

    /// Clicks on the orcha chrome. Returns true when one was handled.
    pub(super) fn handle_orcha_click(
        &mut self,
        point: (u16, u16),
        outcome: &mut ClientShellInput,
    ) -> bool {
        if self.config.sidebar_tree && super::contains(self.hits.new_workspace, point) {
            self.open_new_workspace_popup();
            outcome.repaint = true;
            return true;
        }
        let hits = &self.hits.orcha;
        if super::contains(hits.process_toggle, point) {
            self.orcha_ui.process_folded = !self.orcha_ui.process_folded;
            outcome.repaint = true;
            return true;
        }
        if super::contains(hits.update, point) {
            self.start_update();
            outcome.repaint = true;
            return true;
        }
        if super::contains(hits.quit, point) {
            self.record_binding(
                crate::input::KeybindMatch::Action(crate::input::KeybindAction::Detach),
                outcome,
            );
            return true;
        }
        if super::contains(hits.settings, point) {
            self.record_binding(
                crate::input::KeybindMatch::Action(crate::input::KeybindAction::Settings),
                outcome,
            );
            return true;
        }
        let hide = super::contains(hits.fuel_hide, point);
        if hide || super::contains(hits.fuel_show, point) {
            self.move_fuel(if hide {
                crate::config::FuelPlacement::StatusBar
            } else {
                crate::config::FuelPlacement::Panel
            });
            outcome.repaint = true;
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
            let focused = self.snapshot.as_deref().is_some_and(|snapshot| {
                snapshot
                    .workspaces
                    .iter()
                    .any(|workspace| workspace.workspace_id == id && workspace.focused)
            });
            let expanded = self.orcha_ui.open_projects.contains(&id)
                || (focused && !self.orcha_ui.folded_projects.contains(&id));
            if expanded {
                self.orcha_ui.open_projects.remove(&id);
                self.orcha_ui.folded_projects.insert(id);
            } else {
                self.orcha_ui.folded_projects.remove(&id);
                self.orcha_ui.open_projects.insert(id);
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
        let ask = if super::contains(hits.commit, point) {
            Some("Commit the changes with a clear, conventional commit message.")
        } else if super::contains(hits.pull_request, point) {
            Some("Push this branch and open a pull request with a short summary of the changes.")
        } else {
            None
        };
        if let Some(prompt) = ask {
            // the agent that made the changes does the commit / PR
            if let Some(pane) = self
                .snapshot
                .as_deref()
                .and_then(|s| s.focused_pane_id.clone())
            {
                let quoted = |text: &str| format!("'{}'", text.replace('\'', "'\\''"));
                run_detached(
                    format!(
                        "herdr pane send-text {pane} {text} && herdr pane send-keys {pane} enter",
                        pane = quoted(&pane),
                        text = quoted(prompt)
                    ),
                    &[],
                );
            }
            return true;
        }
        let ide = super::contains(hits.ide, point);
        let diff = super::contains(hits.diff, point);
        if ide || diff || super::contains(hits.git_gui, point) {
            if let Some(root) = self.git_panel.data().map(|data| data.root.clone()) {
                let configured = if ide {
                    &self.config.orcha.ide
                } else {
                    &self.config.orcha.git_gui
                };
                if configured.trim().is_empty() {
                    open_orcha_target(
                        if ide {
                            crate::platform::OrchaOpenTarget::Ide
                        } else {
                            crate::platform::OrchaOpenTarget::GitGui
                        },
                        &root,
                        1,
                    );
                } else {
                    spawn_with_path(configured, &root);
                }
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

    pub(super) fn open_new_workspace_popup(&mut self) {
        let project = self
            .snapshot
            .as_deref()
            .and_then(|snapshot| {
                snapshot
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.focused)
            })
            .map(|workspace| workspace.label.clone());
        self.orcha_ui.popup = Some(super::orcha_popup::OrchaPopup::NewWorkspace {
            selected: 0,
            project,
        });
    }

    fn popup_len(&self) -> usize {
        match &self.orcha_ui.popup {
            Some(super::orcha_popup::OrchaPopup::Accounts { .. }) => {
                self.accounts.data().map_or(0, <[_]>::len) + 1
            }
            Some(super::orcha_popup::OrchaPopup::AddAccount { names, .. }) => names.len() + 1,
            Some(super::orcha_popup::OrchaPopup::Handoff { .. }) => {
                super::orcha_popup::HANDOFF_TARGETS.len()
            }
            Some(super::orcha_popup::OrchaPopup::NewAgent { .. }) => {
                self.accounts.data().map_or(0, <[_]>::len) + 1
            }
            Some(super::orcha_popup::OrchaPopup::Import { .. }) => {
                self.orcha_ui.import_sources.len()
            }
            Some(super::orcha_popup::OrchaPopup::NewWorkspace { .. }) => 3,
            Some(super::orcha_popup::OrchaPopup::OpenWith { choices, .. }) => choices.len(),
            Some(super::orcha_popup::OrchaPopup::Update { .. }) => 3,
            Some(super::orcha_popup::OrchaPopup::WhatsNew { .. }) => 1,
            Some(super::orcha_popup::OrchaPopup::Welcome(_)) | None => 0,
        }
    }

    fn popup_select(&mut self, index: usize) {
        match &mut self.orcha_ui.popup {
            Some(
                super::orcha_popup::OrchaPopup::Accounts { selected, .. }
                | super::orcha_popup::OrchaPopup::AddAccount { selected, .. }
                | super::orcha_popup::OrchaPopup::Handoff { selected, .. }
                | super::orcha_popup::OrchaPopup::NewAgent { selected, .. }
                | super::orcha_popup::OrchaPopup::NewWorkspace { selected, .. }
                | super::orcha_popup::OrchaPopup::OpenWith { selected, .. }
                | super::orcha_popup::OrchaPopup::Update { selected, .. }
                | super::orcha_popup::OrchaPopup::Import { selected, .. },
            ) => *selected = index,
            Some(super::orcha_popup::OrchaPopup::WhatsNew { .. }) => {}
            Some(super::orcha_popup::OrchaPopup::Welcome(welcome)) => welcome.selected = index,
            None => {}
        }
    }

    fn popup_selected(&self) -> usize {
        match &self.orcha_ui.popup {
            Some(
                super::orcha_popup::OrchaPopup::Accounts { selected, .. }
                | super::orcha_popup::OrchaPopup::AddAccount { selected, .. }
                | super::orcha_popup::OrchaPopup::Handoff { selected, .. }
                | super::orcha_popup::OrchaPopup::NewAgent { selected, .. }
                | super::orcha_popup::OrchaPopup::NewWorkspace { selected, .. }
                | super::orcha_popup::OrchaPopup::OpenWith { selected, .. }
                | super::orcha_popup::OrchaPopup::Update { selected, .. }
                | super::orcha_popup::OrchaPopup::Import { selected, .. },
            ) => *selected,
            Some(super::orcha_popup::OrchaPopup::WhatsNew { .. }) => 0,
            Some(super::orcha_popup::OrchaPopup::Welcome(welcome)) => welcome.selected,
            None => 0,
        }
    }

    /// NEW: pick the agent and account for a new tab of the selected project.
    pub(super) fn open_new_agent_popup(&mut self) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let Some(workspace) = snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.focused)
        else {
            return;
        };
        let cwd = snapshot
            .focused_pane_id
            .as_deref()
            .and_then(|focused| snapshot.panes.iter().find(|pane| pane.pane_id == focused))
            .and_then(|pane| pane.foreground_cwd.clone().or_else(|| pane.cwd.clone()))
            .unwrap_or_else(|| workspace.new_workspace_cwd.clone());
        // start on the account with the most fuel
        let selected = self
            .accounts
            .data()
            .and_then(|accounts| {
                accounts
                    .iter()
                    .enumerate()
                    .filter(|(_, account)| !account.windows.is_empty())
                    .max_by_key(|(_, account)| account.lowest().unwrap_or(0))
                    .map(|(index, _)| index)
            })
            .unwrap_or(0);
        self.orcha_ui.popup = Some(super::orcha_popup::OrchaPopup::NewAgent {
            selected,
            workspace: workspace.workspace_id.clone(),
            project: workspace.label.clone(),
            cwd,
        });
        self.accounts.refresh_soon();
    }

    /// The popup's primary action on row `index`; `alternate` is ⌥↵.
    fn popup_activate(&mut self, index: usize, alternate: bool, outcome: &mut ClientShellInput) {
        let Some(popup) = self.orcha_ui.popup.clone() else {
            return;
        };
        let orcha = self.config.orcha.clone();
        match popup {
            // the setup handles its own keys and clicks
            super::orcha_popup::OrchaPopup::Welcome(_) => return,
            super::orcha_popup::OrchaPopup::WhatsNew { .. } => {
                self.orcha_ui.popup = None;
                return;
            }
            super::orcha_popup::OrchaPopup::Update { release, .. } => {
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
            super::orcha_popup::OrchaPopup::OpenWith {
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
            super::orcha_popup::OrchaPopup::NewWorkspace { project, .. } => {
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
                }
                return;
            }
            // a check flips; the button past the rows imports
            super::orcha_popup::OrchaPopup::Import { .. } => {
                if index >= self.orcha_ui.import_sources.len() {
                    self.import_chosen(outcome);
                } else {
                    self.toggle_import(index);
                }
            }
            super::orcha_popup::OrchaPopup::Accounts { pane, .. } => {
                let count = self.accounts.data().map_or(0, <[_]>::len);
                if index >= count {
                    let names = self
                        .accounts
                        .data()
                        .map(super::orcha_popup::account_name_choices)
                        .unwrap_or_default();
                    self.orcha_ui.popup = Some(super::orcha_popup::OrchaPopup::AddAccount {
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
                    .and_then(|pane| {
                        self.snapshot.as_deref().and_then(|snapshot| {
                            snapshot
                                .panes
                                .iter()
                                .find(|p| p.pane_id == pane)
                                .and_then(|p| p.foreground_cwd.clone().or_else(|| p.cwd.clone()))
                        })
                    })
                    .unwrap_or_default();
                let mut env = vec![("HERDR_ACTIVE_PANE_CWD", cwd)];
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
                        let command = fill(
                            &orcha.handoff_command,
                            &[
                                ("pane", pane),
                                ("agent", &account.kind),
                                ("mode", if alternate { "summary" } else { "full" }),
                            ],
                        );
                        run_detached(command, &[]);
                        self.orcha_ui.popup = None;
                        return;
                    }
                }
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
                if signed_out || alternate || pane.is_none() {
                    self.accounts.refresh_soon();
                } else {
                    // the switch shows at once; the data catches up
                    self.accounts
                        .mark_current(&account.id, std::time::Instant::now());
                    self.update_low_fuel_hint();
                }
            }
            super::orcha_popup::OrchaPopup::NewAgent { workspace, cwd, .. } => {
                let account = self
                    .accounts
                    .data()
                    .and_then(|data| data.get(index))
                    .cloned();
                self.orcha_ui.popup = None;
                match account {
                    Some(account) if account.signed_out() => {
                        run_detached(fill(&orcha.login_command, &[("id", &account.id)]), &[]);
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
            super::orcha_popup::OrchaPopup::AddAccount { names, typed, .. } => {
                let name = if index < names.len() {
                    names[index].clone()
                } else {
                    typed
                };
                self.add_claude_account(&name);
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

    /// Create `name` as a Claude account and open its sign-in.
    fn add_claude_account(&mut self, name: &str) {
        let name: String = name
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
            .flat_map(|ch| ch.to_lowercase())
            .collect();
        if name.is_empty() {
            return;
        }
        run_detached(
            fill(&self.config.orcha.add_command, &[("name", &name)]),
            &[],
        );
        self.accounts.refresh_soon();
        self.orcha_ui.popup = None;
    }

    /// ADD ACCOUNT: arrows move, letters fill the name row, esc goes back.
    fn route_add_account_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        use crossterm::event::KeyCode;
        outcome.repaint = true;
        let Some(super::orcha_popup::OrchaPopup::AddAccount {
            selected,
            names,
            typed,
            pane,
        }) = self.orcha_ui.popup.clone()
        else {
            return;
        };
        let len = names.len() + 1;
        match key.code {
            KeyCode::Esc => {
                self.orcha_ui.popup = Some(super::orcha_popup::OrchaPopup::Accounts {
                    selected: self.accounts.data().map_or(0, <[_]>::len),
                    pane,
                });
            }
            KeyCode::Up | KeyCode::BackTab if len > 0 => {
                self.popup_select((selected + len - 1) % len);
            }
            KeyCode::Down | KeyCode::Tab if len > 0 => {
                self.popup_select((selected + 1) % len);
            }
            KeyCode::Backspace => {
                let mut typed = typed;
                typed.pop();
                self.orcha_ui.popup = Some(super::orcha_popup::OrchaPopup::AddAccount {
                    selected: names.len(),
                    names,
                    typed,
                    pane,
                });
            }
            KeyCode::Enter => self.popup_activate(selected, false, outcome),
            KeyCode::Char(ch) if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' => {
                let mut typed = typed;
                typed.push(ch.to_ascii_lowercase());
                self.orcha_ui.popup = Some(super::orcha_popup::OrchaPopup::AddAccount {
                    selected: names.len(),
                    names,
                    typed,
                    pane,
                });
            }
            _ => {}
        }
    }

    /// Keys while an orcha popup is open: arrows/j k move, ↵ acts, ⌥↵ the
    /// alternate action, H hands off, esc closes.
    pub(super) fn route_orcha_popup_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        use crossterm::event::{KeyCode, KeyModifiers};
        if matches!(
            self.orcha_ui.popup,
            Some(super::orcha_popup::OrchaPopup::Welcome(_))
        ) {
            self.route_welcome_key(key, outcome);
            return;
        }
        if matches!(
            self.orcha_ui.popup,
            Some(super::orcha_popup::OrchaPopup::AddAccount { .. })
        ) {
            self.route_add_account_key(key, outcome);
            return;
        }
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
            KeyCode::Enter
                if matches!(
                    self.orcha_ui.popup,
                    Some(super::orcha_popup::OrchaPopup::Import { .. })
                ) =>
            {
                self.import_chosen(outcome);
            }
            KeyCode::Char(' ')
                if matches!(
                    self.orcha_ui.popup,
                    Some(super::orcha_popup::OrchaPopup::Import { .. })
                ) =>
            {
                self.toggle_import(selected);
            }
            KeyCode::Char(' ')
                if matches!(
                    self.orcha_ui.popup,
                    Some(super::orcha_popup::OrchaPopup::OpenWith { .. })
                ) =>
            {
                self.toggle_open_with_remember();
            }
            KeyCode::Enter => {
                let alternate = key.modifiers.contains(KeyModifiers::ALT);
                self.popup_activate(selected, alternate, outcome);
            }
            KeyCode::Char('p') | KeyCode::Char('P')
                if matches!(
                    self.orcha_ui.popup,
                    Some(super::orcha_popup::OrchaPopup::NewAgent { .. })
                ) =>
            {
                self.orcha_ui.popup = None;
                self.record_binding(
                    crate::input::KeybindMatch::Action(crate::input::KeybindAction::NewWorkspace),
                    outcome,
                );
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
        if matches!(
            self.orcha_ui.popup,
            Some(super::orcha_popup::OrchaPopup::Welcome(_))
        ) {
            self.handle_welcome_mouse(mouse, outcome);
            return;
        }
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
        if let Some((_, index)) = hits
            .actions
            .iter()
            .find(|(rect, _)| super::contains(*rect, point))
        {
            self.popup_activate(*index, false, outcome);
            return;
        }
        if let Some((_, index)) = hits
            .rows
            .iter()
            .find(|(rect, _)| super::contains(*rect, point))
        {
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

    /// Running low: once per account, when the focused agent is between
    /// turns, ask where to continue (the CONTINUE ON popup, best choice
    /// picked).
    pub(super) fn offer_continue_on_low_fuel(&mut self) -> bool {
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
        self.orcha_ui.low_fuel_offered.insert(id);
        self.open_accounts_overlay();
        true
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

    /// Ctrl+click on a path in a pane: a folder opens in the file manager,
    /// a picture, PDF, web page or other document in the system's app for it,
    /// and code or text in the editor (at its `:line`). Relative paths are the pane's. Returns
    /// false when nothing exists there.
    pub(super) fn open_path_link(&mut self, pane_id: &str, text: &str) -> bool {
        let (raw, line) = split_line(text);
        let home = crate::platform::user_home_dir()
            .map(|home| home.to_string_lossy().into_owned())
            .unwrap_or_default();
        let expanded = match raw.strip_prefix('~') {
            Some(rest) => format!("{home}{rest}"),
            None => raw.to_owned(),
        };
        let mut path = std::path::PathBuf::from(&expanded);
        if path.is_relative() {
            let cwd = self.snapshot.as_deref().and_then(|snapshot| {
                snapshot
                    .panes
                    .iter()
                    .find(|pane| pane.pane_id == pane_id)
                    .and_then(|pane| pane.foreground_cwd.clone().or_else(|| pane.cwd.clone()))
            });
            let Some(cwd) = cwd else {
                return false;
            };
            path = std::path::Path::new(&cwd).join(path);
            // agents print paths from the repository root
            if !path.exists() {
                if let Some(root) = self.git_panel.data().map(|data| data.root.clone()) {
                    path = std::path::Path::new(&root).join(&expanded);
                }
            }
        }
        if !path.exists() {
            // the agent may print a path from its repository root, or a bare name
            match self.find_printed_path(pane_id, &expanded) {
                Some(found) => path = found,
                None => {
                    self.show_notice(format!("can't find {raw}"));
                    return true;
                }
            }
        }
        let shown = path.to_string_lossy().into_owned();
        let line = line.unwrap_or(1);
        if path.is_dir() {
            open_orcha_target(crate::platform::OrchaOpenTarget::SystemDefault, &shown, 1);
            return true;
        }
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .filter(|extension| extension.chars().all(|c| c.is_ascii_alphanumeric()))
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        if let Some(choice) = self
            .config
            .orcha
            .open_with
            .get(&extension)
            .filter(|_| !extension.is_empty())
            .and_then(|value| super::orcha_popup::OpenChoice::from_config_value(value))
        {
            open_with_choice(&choice, &shown, line);
            return true;
        }
        let choices = open_choices(&path, &self.config.orcha.ide);
        self.orcha_ui.popup = Some(super::orcha_popup::OrchaPopup::OpenWith {
            selected: 0,
            path: shown,
            line,
            extension,
            choices,
            remember: true,
        });
        true
    }

    fn toggle_open_with_remember(&mut self) {
        if let Some(super::orcha_popup::OrchaPopup::OpenWith { remember, .. }) =
            self.orcha_ui.popup.as_mut()
        {
            *remember = !*remember;
        }
    }

    /// Keeps the app for `extension`, in memory and in `[orcha.open_with]`.
    fn save_open_with(&mut self, extension: &str, choice: &super::orcha_popup::OpenChoice) {
        let value = choice.config_value();
        self.config
            .orcha
            .open_with
            .insert(extension.to_owned(), value.clone());
        let path = crate::config::config_path();
        let key = extension.to_owned();
        let _ = crate::thread_spawn::spawn_named("orcha-open-with", move || {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            let quoted = format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""));
            let updated =
                crate::config::upsert_section_value(&content, "orcha.open_with", &key, &quoted);
            if let Err(err) = std::fs::write(&path, updated) {
                tracing::warn!(err = %err, "orcha: failed to save how .{key} files open");
            }
        });
    }

    /// A printed path that isn't under the pane's folder: try the pane's
    /// repository root, then (for a bare file name) a short search of it.
    fn find_printed_path(&self, pane_id: &str, printed: &str) -> Option<std::path::PathBuf> {
        let cwd = self.snapshot.as_deref().and_then(|snapshot| {
            snapshot
                .panes
                .iter()
                .find(|pane| pane.pane_id == pane_id)
                .and_then(|pane| pane.foreground_cwd.clone().or_else(|| pane.cwd.clone()))
        })?;
        let root = std::path::Path::new(&cwd)
            .ancestors()
            .find(|dir| dir.join(".git").exists())?
            .to_path_buf();
        let candidate = root.join(printed);
        if candidate.exists() {
            return Some(candidate);
        }
        if printed.contains('/') || printed.contains('\\') {
            return None;
        }
        find_file_named(&root, printed)
    }

    /// Looks for projects to import off the input path; the popup opens
    /// when the list is in (see `poll_import_scan`).
    pub(super) fn start_import_scan(&mut self) {
        if self.orcha_ui.import_scan.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = crate::thread_spawn::spawn_named("orcha-import", move || {
            let _ = tx.send(crate::orcha_import::find_sources());
        });
        if spawned.is_ok() {
            self.orcha_ui.import_scan = Some(rx);
        }
    }

    /// The import list came in: offer it (asked once; the menu offers it
    /// again any time). Returns true when the popup opened.
    pub(super) fn poll_import_scan(&mut self) -> bool {
        let Some(rx) = self.orcha_ui.import_scan.as_ref() else {
            return false;
        };
        let Ok(sources) = rx.try_recv() else {
            if matches!(
                rx.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Disconnected)
            ) {
                self.orcha_ui.import_scan = None;
            }
            return false;
        };
        self.orcha_ui.import_scan = None;
        // the setup shows the list as one of its steps
        if matches!(
            self.orcha_ui.popup,
            Some(super::orcha_popup::OrchaPopup::Welcome(_))
        ) {
            return self.welcome_sources_found(sources);
        }
        let first_time = !crate::orcha_import::asked();
        crate::orcha_import::mark_asked();
        // nothing found on the first run: say nothing
        if sources.is_empty() && first_time {
            return false;
        }
        let chosen = vec![true; sources.len()];
        self.orcha_ui.import_sources = sources;
        self.orcha_ui.popup = Some(super::orcha_popup::OrchaPopup::Import {
            selected: 0,
            chosen,
        });
        true
    }

    fn toggle_import(&mut self, index: usize) {
        if let Some(super::orcha_popup::OrchaPopup::Import { chosen, .. }) =
            self.orcha_ui.popup.as_mut()
        {
            if let Some(on) = chosen.get_mut(index) {
                *on = !*on;
            }
        }
    }

    /// Adds the checked sources' projects as workspaces (each folder once,
    /// none that is open already), in the background.
    fn import_chosen(&mut self, outcome: &mut ClientShellInput) {
        let Some(super::orcha_popup::OrchaPopup::Import { chosen, .. }) =
            self.orcha_ui.popup.take()
        else {
            return;
        };
        self.import_projects(&chosen, outcome);
    }

    /// Adds the projects of the sources checked in `chosen`.
    pub(super) fn import_projects(&mut self, chosen: &[bool], outcome: &mut ClientShellInput) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let mut seen = std::collections::HashSet::new();
        let mut methods = Vec::new();
        for (source, _) in self
            .orcha_ui
            .import_sources
            .iter()
            .zip(chosen)
            .filter(|(_, on)| **on)
        {
            for (name, path) in &source.projects {
                let folder = std::path::Path::new(path);
                if !seen.insert(path.clone()) || workspace_for_folder(snapshot, folder).is_some() {
                    continue;
                }
                methods.push(crate::api::schema::Method::WorkspaceCreate(
                    crate::api::schema::WorkspaceCreateParams {
                        source_workspace_id: None,
                        cwd: Some(path.clone()),
                        focus: false,
                        label: Some(name.clone()),
                        env: Default::default(),
                    },
                ));
            }
        }
        for method in methods {
            self.push_endpoint_method(method, outcome);
        }
        outcome.repaint = true;
    }

    /// On the first snapshot: orcha started inside a project opens that
    /// project (its git root gets a workspace unless one is already there,
    /// then it is focused); the first time ever, the import popup is offered.
    pub(crate) fn open_launch_project(&mut self, outcome: &mut ClientShellInput) {
        if self.orcha_ui.launch_project_done
            || !crate::orcha_app::is_orcha()
            || self.active_endpoint_id != ClientEndpointId::Local
        {
            return;
        }
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        self.orcha_ui.launch_project_done = true;
        let method =
            launch_project().and_then(|project| match workspace_for_folder(snapshot, &project) {
                Some(workspace) if workspace.focused => None,
                Some(workspace) => Some(crate::api::schema::Method::WorkspaceFocus(
                    crate::api::schema::WorkspaceTarget {
                        workspace_id: workspace.workspace_id.clone(),
                    },
                )),
                None => Some(crate::api::schema::Method::WorkspaceCreate(
                    crate::api::schema::WorkspaceCreateParams {
                        source_workspace_id: None,
                        cwd: Some(project.to_string_lossy().into_owned()),
                        focus: true,
                        label: None,
                        env: Default::default(),
                    },
                )),
            });
        if let Some(method) = method {
            self.push_endpoint_method(method, outcome);
            outcome.repaint = true;
        }
        // the first time: the setup, which offers the projects other tools
        // know about too
        if !super::orcha_welcome::done() {
            // a fresh install: nothing changed for this user
            crate::orcha_app::mark_version_seen();
            self.open_welcome(outcome);
        } else {
            // after an update: what changed, once (looked up off the input path)
            let (tx, rx) = std::sync::mpsc::channel();
            if crate::thread_spawn::spawn_named("orcha-whats-new", move || {
                let _ = tx.send(crate::orcha_app::whats_new());
            })
            .is_ok()
            {
                self.orcha_ui.whats_new_check = Some(rx);
            }
            if !crate::orcha_import::asked() {
                self.start_import_scan();
            }
        }
        self.start_update_check();
    }

    /// Asks (at most daily, off the input path) whether a newer orcha is out.
    fn start_update_check(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        if crate::thread_spawn::spawn_named("orcha-update-check", move || {
            let _ = tx.send(crate::orcha_app::newer_release());
        })
        .is_ok()
        {
            self.orcha_ui.update_check = Some(rx);
        }
    }

    /// The header's update button: shows a newer release, then the update
    /// running, then what to do next. Returns true when it changed.
    pub(super) fn poll_update(&mut self) -> bool {
        let ui = &mut self.orcha_ui;
        if let Some(Ok(news)) = ui.whats_new_check.as_ref().map(|rx| rx.try_recv()) {
            ui.whats_new_check = None;
            ui.whats_new = news;
        }
        // shown once nothing else is on screen
        if ui.popup.is_none() {
            if let Some(release) = ui.whats_new.take() {
                ui.popup = Some(super::orcha_popup::OrchaPopup::WhatsNew { release });
                return true;
            }
        }
        if let Some(Ok(newer)) = ui.update_check.as_ref().map(|rx| rx.try_recv()) {
            ui.update_check = None;
            if let Some(release) = newer {
                ui.update_note = Some(format!("⬆ update to {}", release.version));
                ui.update_available = Some(release.version.clone());
                // ask once per version, when nothing else is on screen
                if ui.popup.is_none() && crate::orcha_app::first_offer(&release.version) {
                    ui.popup = Some(super::orcha_popup::OrchaPopup::Update {
                        selected: 0,
                        release,
                    });
                }
                return true;
            }
        }
        if let Some(Ok(ok)) = ui.update_run.as_ref().map(|rx| rx.try_recv()) {
            ui.update_run = None;
            ui.update_note = Some(if ok {
                "✓ updated: quit and reopen".to_owned()
            } else {
                "update failed: run orcha update".to_owned()
            });
            return true;
        }
        false
    }

    /// Click on the update button: installs the newer release behind the UI.
    fn start_update(&mut self) {
        if self.orcha_ui.update_run.is_some() || self.orcha_ui.update_available.is_none() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        if crate::thread_spawn::spawn_named("orcha-update", move || {
            let _ = tx.send(crate::orcha_app::run_installer(true));
        })
        .is_ok()
        {
            self.orcha_ui.update_available = None;
            self.orcha_ui.update_run = Some(rx);
            self.orcha_ui.update_note = Some("⬆ updating…".to_owned());
        }
    }
}

/// The workspace already there for a folder: its own folder, or one of
/// its panes', is inside it.
fn workspace_for_folder<'a>(
    snapshot: &'a ClientShellSnapshot,
    folder: &std::path::Path,
) -> Option<&'a crate::protocol::ClientShellWorkspace> {
    let inside = |path: &str| std::path::Path::new(path).starts_with(folder);
    snapshot.workspaces.iter().find(|workspace| {
        inside(&workspace.new_workspace_cwd)
            || snapshot.panes.iter().any(|pane| {
                pane.workspace_id == workspace.workspace_id
                    && pane.cwd.as_deref().is_some_and(inside)
            })
    })
}

/// The project the client was started in: the git root of the current
/// folder, else the folder. Not the home folder or `/`.
fn launch_project() -> Option<std::path::PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let home = crate::platform::user_home_dir();
    if cwd.parent().is_none() || home.as_deref() == Some(cwd.as_path()) {
        return None;
    }
    let root = std::process::Command::new("git")
        .arg("-C")
        .arg(&cwd)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|root| !root.is_empty())
        .map(std::path::PathBuf::from);
    Some(root.unwrap_or(cwd))
}

#[cfg(test)]
mod tests {
    use super::super::orcha_popup::OpenChoice;

    #[test]
    fn open_choices_suggest_the_system_app_for_documents_and_the_editor_for_code() {
        use std::path::Path;
        let picture = super::open_choices(Path::new("shot.png"), "");
        assert_eq!(picture[0], OpenChoice::System);
        assert_eq!(picture.last(), Some(&OpenChoice::Reveal));
        let code = super::open_choices(Path::new("main.rs"), "zed {path}");
        assert_eq!(
            code[0],
            OpenChoice::Editor {
                name: String::new(),
                command: "zed {path}".into()
            }
        );
        assert!(code.contains(&OpenChoice::System));
    }

    #[test]
    fn open_choices_round_trip_through_the_config() {
        for choice in [
            OpenChoice::System,
            OpenChoice::Reveal,
            OpenChoice::Editor {
                name: String::new(),
                command: String::new(),
            },
            OpenChoice::Editor {
                name: String::new(),
                command: "open -a 'Zed' {path}".into(),
            },
        ] {
            assert_eq!(
                OpenChoice::from_config_value(&choice.config_value()),
                Some(choice)
            );
        }
        assert_eq!(OpenChoice::from_config_value("nonsense"), None);
    }

    #[test]
    fn open_with_section_parses_and_saves() {
        let config: crate::config::Config =
            toml::from_str("[orcha.open_with]\npng = \"system\"\nrs = \"editor\"\n")
                .expect("config");
        assert_eq!(
            config.orcha.open_with.get("png").map(String::as_str),
            Some("system")
        );
        let saved = crate::config::upsert_section_value(
            "[orcha]\nide = \"\"\n",
            "orcha.open_with",
            "pdf",
            "\"folder\"",
        );
        let config: crate::config::Config = toml::from_str(&saved).expect("saved config");
        assert_eq!(
            config.orcha.open_with.get("pdf").map(String::as_str),
            Some("folder")
        );
    }

    #[test]
    fn a_bare_file_name_is_found_in_the_repository() {
        let root = std::env::temp_dir().join(format!("orcha-find-{}", std::process::id()));
        let deep = root.join("src/client");
        std::fs::create_dir_all(&deep).expect("dirs");
        std::fs::create_dir_all(root.join("node_modules/x")).expect("dirs");
        std::fs::write(root.join("node_modules/x/app.rs"), "").expect("file");
        std::fs::write(deep.join("app.rs"), "").expect("file");
        assert_eq!(
            super::find_file_named(&root, "app.rs"),
            Some(deep.join("app.rs"))
        );
        assert_eq!(super::find_file_named(&root, "missing.rs"), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn documents_open_in_their_app_and_code_in_the_editor() {
        use std::path::Path;
        for path in [
            "shot.PNG",
            "spec.pdf",
            "report.docx",
            "index.html",
            "demo.mp4",
        ] {
            assert!(super::opens_in_system_app(Path::new(path)), "{path}");
        }
        for path in [
            "main.rs",
            "README.md",
            "logo.svg",
            "Makefile",
            "config.toml",
        ] {
            assert!(!super::opens_in_system_app(Path::new(path)), "{path}");
        }
    }

    use super::*;

    #[test]
    fn paths_keep_their_line() {
        assert_eq!(split_line("src/a.ts:12:4"), ("src/a.ts", Some(12)));
        assert_eq!(split_line("src/a.ts"), ("src/a.ts", None));
    }
}
