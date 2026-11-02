//! orcha: what the orcha chrome's buttons do.

use super::background_job::BackgroundJob;
use super::*;

mod accounts;
mod file_paths;
mod popup_choices;
mod popup_input;
mod projects;
mod services;

pub(super) use file_paths::spawn_with_path;
#[cfg(test)]
use file_paths::{find_file_named, open_choices, opens_in_system_app, split_line};
use file_paths::{open_orcha_target, spawn_quietly};

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
        // + NEW and the tab strip's + both offer NEW AGENT (a new project is
        // one of its rows)
        if self.config.sidebar_tree
            && (super::contains(self.hits.new_workspace, point)
                || super::contains(self.hits.new_tab, point))
        {
            self.open_new_agent_popup();
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
            // the page shows every setting at once; on another machine (SSH or a
            // remote endpoint) the browser is not here, so the terminal Settings
            if self.active_endpoint_id == ClientEndpointId::Local
                && !super::orcha_settings_web::over_ssh()
            {
                self.open_settings_web();
                outcome.repaint = true;
            } else {
                self.record_binding(
                    crate::input::KeybindMatch::Action(crate::input::KeybindAction::Settings),
                    outcome,
                );
            }
            return true;
        }
        if super::contains(hits.commands, point) {
            self.open_palette();
            outcome.repaint = true;
            return true;
        }
        if super::contains(hits.office, point) {
            self.open_office();
            outcome.repaint = true;
            return true;
        }
        if super::contains(hits.needs_you, point) {
            self.jump_to_needs_you(outcome);
            outcome.repaint = true;
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
            let expanded = self.orcha_ui.project_expanded(&id, focused);
            self.orcha_ui.set_project_expanded(&id, !expanded);
            self.persist_chrome_preferences(outcome);
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
        if let Some(index) = hits
            .file_rows
            .iter()
            .find(|(rect, _)| super::contains(*rect, point))
            .map(|(_, index)| *index)
        {
            self.click_file_row(index);
            outcome.repaint = true;
            return true;
        }
        if super::contains(hits.files_toggle, point) {
            self.orcha_ui.files_open = !self.orcha_ui.files_open;
            self.orcha_ui.files_scroll = 0;
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
            self.ask_focused_agent(prompt);
            outcome.repaint = true;
            return true;
        }
        let ide = super::contains(hits.ide, point);
        let diff = super::contains(hits.diff, point);
        if ide || diff || super::contains(hits.git_gui, point) {
            self.open_project_in(ide);
            return true;
        }
        if super::contains(hits.branch_toggle, point) {
            self.orcha_ui.branch_folded = !self.orcha_ui.branch_folded;
            outcome.repaint = true;
            return true;
        }
        if super::contains(hits.changes_toggle, point) {
            self.orcha_ui.changes_folded = !self.orcha_ui.changes_folded;
            outcome.repaint = true;
            return true;
        }
        false
    }

    /// The agent that made the changes does the commit / PR: the prompt is
    /// typed into the focused pane, off the input thread. Only an agent that
    /// is waiting for its next prompt gets it: a shell would run it, a busy
    /// agent would get it mid-turn, and one asking you something would take
    /// the enter as its answer.
    pub(super) fn ask_focused_agent(&mut self, prompt: &'static str) {
        let agent = self.snapshot.as_deref().and_then(focused_agent);
        let Some(agent) = agent else {
            self.show_notice("Open an agent first".to_owned());
            return;
        };
        if !matches!(
            agent.agent_status,
            crate::api::schema::AgentStatus::Idle | crate::api::schema::AgentStatus::Done
        ) {
            self.show_notice("The agent is busy. Try again when it is done".to_owned());
            return;
        }
        let pane = agent.pane_id.clone();
        self.show_notice("sent to the agent".to_owned());
        self.start_notice_job("orcha-ask-agent", None, move || {
            if crate::orcha_handoff::send_prompt(&pane, prompt) {
                Ok(String::new())
            } else {
                Err("Could not type into the agent".to_owned())
            }
        });
    }

    /// Runs `work` off the input thread; its Ok message (when not empty)
    /// shows as a notice, its Err as a failure that stays. `working` stays up
    /// while it runs.
    pub(super) fn start_notice_job(
        &mut self,
        job: &'static str,
        working: Option<String>,
        work: impl FnOnce() -> Result<String, String> + Send + 'static,
    ) {
        let Some(handle) = BackgroundJob::spawn(job, work) else {
            self.show_failure("Could not start that, try again".to_owned());
            return;
        };
        if let Some(message) = working {
            self.start_working_notice(job, message);
        }
        self.orcha_ui.notice_jobs.push((job, handle));
    }

    /// Says how the finished notice jobs went. True when one finished.
    fn poll_notice_jobs(&mut self) -> bool {
        let mut finished = Vec::new();
        self.orcha_ui
            .notice_jobs
            .retain(|(job, handle)| match handle.poll() {
                Some(result) => {
                    finished.push((*job, result));
                    false
                }
                None => true,
            });
        let changed = !finished.is_empty();
        for (job, result) in finished {
            self.end_working_notice(job);
            match result.unwrap_or_else(|lost| Err(format!("Not done: {lost}"))) {
                Ok(message) if message.is_empty() => {}
                Ok(message) => self.show_notice(message),
                Err(message) => self.show_failure(message),
            }
        }
        changed
    }

    /// Opens the focused project's repository in the IDE (`ide`) or the Git
    /// GUI: the configured command, else the platform's discovery.
    pub(super) fn open_project_in(&mut self, ide: bool) {
        let Some(root) = self.git_panel.data().map(|data| data.root.clone()) else {
            self.show_notice("open: the focused folder is not a git repository".to_owned());
            return;
        };
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
    let mut process = super::accounts::orcha_command(&command);
    for (key, value) in extra_env {
        process.env(key, value);
    }
    spawn_quietly(process, &command);
}

/// An account action orcha runs itself (`crate::orcha_accounts`).
#[derive(Debug, Clone)]
enum AccountAction {
    Switch {
        id: String,
        pane: String,
    },
    /// Stops Claude in `pane` and starts it on `id`, same chat.
    Restart {
        id: String,
        pane: String,
    },
    Login {
        id: String,
        pane: Option<String>,
    },
    Open {
        id: String,
        pane: Option<String>,
        cwd: String,
    },
    Add {
        name: String,
        pane: Option<String>,
    },
    Remove {
        id: String,
    },
    /// Adds the found accounts in `chosen`; `dismissed` are not asked again.
    Import {
        chosen: Vec<crate::orcha_accounts::FoundAccount>,
        dismissed: Vec<crate::orcha_accounts::FoundAccount>,
    },
    NewAgent {
        id: String,
        workspace: String,
        cwd: String,
    },
}

impl AccountAction {
    /// What the notice says while it runs.
    fn working(&self) -> String {
        match self {
            Self::Switch { id, .. } => format!("moving this chat to {id}…"),
            Self::Restart { id, .. } => format!("restarting Claude on {id}…"),
            Self::Login { id, .. } => format!("opening the sign in for {id}…"),
            Self::Open { id, .. } => format!("opening {id}…"),
            Self::Add { name, .. } => format!("adding {name}…"),
            Self::Remove { id } => format!("removing {id}…"),
            Self::Import { .. } => "adding the accounts…".to_owned(),
            Self::NewAgent { id, .. } => format!("starting an agent on {id}…"),
        }
    }

    /// Runs it (blocking); Ok is what to tell the user.
    fn run(self) -> Result<String, String> {
        use crate::orcha_accounts as accounts;
        match self {
            Self::Switch { id, pane } => {
                accounts::switch(&id, &pane).map(|()| format!("this chat continues on {id}"))
            }
            Self::Restart { id, pane } => accounts::restart_on(&id, &pane)
                .map(|()| format!("Claude restarted on {id}, same chat")),
            Self::Login { id, pane } => {
                accounts::login(&id, pane.as_deref()).map(|()| "sign in in the new pane".to_owned())
            }
            Self::Open { id, pane, cwd } => {
                let cwd = Some(cwd.as_str()).filter(|cwd| !cwd.is_empty());
                accounts::open(&id, pane.as_deref(), cwd).map(|()| format!("{id} opened"))
            }
            Self::Add { name, pane } => accounts::add(&name, pane.as_deref())
                .map(|id| format!("{id} added: sign in in the new pane")),
            Self::Remove { id } => accounts::remove(&id).map(|()| format!("{id} removed")),
            Self::Import { chosen, dismissed } => {
                accounts::import_found(&chosen, &dismissed).map(|added| match added.len() {
                    0 => "found accounts: not asking again".to_owned(),
                    1 => format!("{} added to Accounts", added[0]),
                    n => format!("{n} accounts added to Accounts"),
                })
            }
            Self::NewAgent { id, workspace, cwd } => {
                accounts::new_agent(&id, &workspace, &cwd).map(|()| format!("{id} started"))
            }
        }
    }
}

/// The agent in the focused pane, when one runs there.
fn focused_agent(snapshot: &ClientShellSnapshot) -> Option<&crate::protocol::ClientShellAgent> {
    let focused = snapshot.focused_pane_id.as_deref()?;
    snapshot.agents.iter().find(|agent| {
        agent.pane_id == focused
            && agent.agent.is_some()
            && agent.agent_status != crate::api::schema::AgentStatus::Unknown
    })
}

/// Where NEW AGENT starts: the default account of the first kind whose
/// default is signed in and not low on fuel, else the account with the
/// most fuel.
fn new_agent_choice(
    accounts: &[super::accounts::Account],
    defaults: &[String],
    low_fuel_percent: u8,
) -> usize {
    accounts
        .iter()
        .position(|account| {
            defaults.contains(&account.id)
                && !account.signed_out()
                && account.lowest().is_none_or(|left| left >= low_fuel_percent)
        })
        .or_else(|| {
            accounts
                .iter()
                .enumerate()
                .filter(|(_, account)| !account.windows.is_empty())
                .max_by_key(|(_, account)| account.lowest().unwrap_or(0))
                .map(|(index, _)| index)
        })
        .unwrap_or(0)
}

/// The workspace already there for a folder: its own folder, or one of
/// its panes', is inside it.
fn workspace_for_folder<'a>(
    snapshot: &'a ClientShellSnapshot,
    folder: &std::path::Path,
) -> Option<&'a crate::protocol::ClientShellWorkspace> {
    let folder = crate::orcha_import::folder_key(&folder.to_string_lossy());
    let inside = |path: &str| {
        crate::orcha_import::key_within(&crate::orcha_import::folder_key(path), &folder)
    };
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

/// What the agent is asked to write before its account runs dry.
fn checklist_prompt(left: u8, file: &str) -> String {
    format!(
        "Your account has about {left}% of its plan left. Before going on, write a short \
         hand-off checklist to {file} (create the folder if needed): what is done, what is \
         next as - [ ] items, open questions, and the files you touched. Keep it under 40 \
         lines, then stop and wait."
    )
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
