//! Project import, launch and focused-pane navigation.

use super::*;

impl ClientShellState {
    /// A project just added by hand gets one question about working rules,
    /// once nothing else is on screen and only when it has none yet.
    pub(in crate::client::shell) fn offer_project_rules(&mut self) -> bool {
        if self.orcha_ui.popup.is_some() || self.overlay.is_some() {
            return false;
        }
        let Some(folder) = self.orcha_ui.rules_offer.take() else {
            return false;
        };
        // two small reads, once per added project
        if !crate::orcha_rules::should_offer(&folder) {
            tracing::info!(folder = %folder.display(), "orcha: no rules question for this project");
            return false;
        }
        let file = crate::orcha_rules::instruction_file(&folder)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "CLAUDE.md".to_owned());
        tracing::info!(folder = %folder.display(), "orcha: offering working rules");
        self.orcha_ui.popup = Some(super::super::orcha_popup::OrchaPopup::RulesOffer {
            selected: 0,
            folder,
            file,
            rules: crate::orcha_rules::defaults(),
        });
        true
    }

    /// The folder `pane` is in now (its foreground process's, else its own).
    pub(super) fn pane_cwd(&self, pane: &str) -> Option<String> {
        self.snapshot.as_deref().and_then(|snapshot| {
            snapshot
                .panes
                .iter()
                .find(|p| p.pane_id == pane)
                .and_then(|p| p.foreground_cwd.clone().or_else(|| p.cwd.clone()))
        })
    }

    /// Focuses the next agent that needs you, across all projects.
    pub(in crate::client::shell) fn jump_to_needs_you(&mut self, outcome: &mut ClientShellInput) {
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

    pub(super) fn focus_agent_pane(&mut self, pane: &str, outcome: &mut ClientShellInput) {
        self.push_endpoint_method(
            crate::api::schema::Method::PaneFocus(crate::api::schema::PaneTarget {
                pane_id: pane.to_owned(),
            }),
            outcome,
        );
        outcome.repaint = true;
    }

    /// Looks for projects to import off the input path; the popup opens
    /// when the list is in (see `poll_import_scan`).
    pub(in crate::client::shell) fn start_import_scan(&mut self) {
        if self.orcha_ui.import_scan.is_some() {
            return;
        }
        self.orcha_ui.import_scan =
            BackgroundJob::spawn("orcha-import", crate::orcha_import::find_sources);
    }

    /// The import list came in: offer it (asked once; the menu offers it
    /// again any time). Returns true when the popup opened.
    pub(in crate::client::shell) fn poll_import_scan(&mut self) -> bool {
        let Some(Ok(sources)) = BackgroundJob::take_finished(&mut self.orcha_ui.import_scan) else {
            return false;
        };
        let sources = self.without_open_projects(sources);
        // the setup shows the list as one of its steps
        if matches!(
            self.orcha_ui.popup,
            Some(super::super::orcha_popup::OrchaPopup::Welcome(_))
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
        self.orcha_ui.popup = Some(super::super::orcha_popup::OrchaPopup::Import {
            selected: 0,
            chosen,
        });
        true
    }

    /// Drops the folders that are a project already, and sources left empty.
    pub(super) fn without_open_projects(
        &self,
        mut sources: Vec<crate::orcha_import::ImportSource>,
    ) -> Vec<crate::orcha_import::ImportSource> {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return sources;
        };
        for source in &mut sources {
            source.projects.retain(|(_, path)| {
                !self
                    .orcha_ui
                    .imported_folders
                    .contains(&crate::orcha_import::folder_key(path))
                    && workspace_for_folder(snapshot, std::path::Path::new(path)).is_none()
            });
        }
        sources.retain(|source| !source.projects.is_empty());
        sources
    }

    pub(super) fn toggle_import(&mut self, index: usize) {
        if let Some(super::super::orcha_popup::OrchaPopup::Import { chosen, .. }) =
            self.orcha_ui.popup.as_mut()
        {
            if let Some(on) = chosen.get_mut(index) {
                *on = !*on;
            }
        }
    }

    /// Adds the checked sources' projects as workspaces (each folder once,
    /// none that is open already), in the background.
    pub(super) fn import_chosen(&mut self, outcome: &mut ClientShellInput) {
        let Some(super::super::orcha_popup::OrchaPopup::Import { chosen, .. }) =
            self.orcha_ui.popup.take()
        else {
            return;
        };
        self.import_projects(&chosen, outcome);
    }

    /// Adds the projects of the sources checked in `chosen`.
    pub(in crate::client::shell) fn import_projects(
        &mut self,
        chosen: &[bool],
        outcome: &mut ClientShellInput,
    ) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
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
                // once per folder, also across imports the sidebar has not
                // caught up with yet
                let key = crate::orcha_import::folder_key(path);
                if self.orcha_ui.imported_folders.contains(&key)
                    || workspace_for_folder(snapshot, folder).is_some()
                {
                    continue;
                }
                self.orcha_ui.imported_folders.insert(key);
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
        self.orcha_ui.project_icons = super::super::orcha_icons::load();
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
        if !super::super::orcha_welcome::done() {
            // a fresh install: nothing changed for this user
            crate::orcha_app::mark_version_seen();
            self.open_welcome(outcome);
        } else {
            // after an update: what changed, once (looked up off the input path)
            self.orcha_ui.whats_new_check =
                BackgroundJob::spawn("orcha-whats-new", crate::orcha_app::whats_new);
            if !crate::orcha_import::asked() {
                self.start_import_scan();
            }
            self.orcha_ui.plugin_offer_check =
                BackgroundJob::spawn("orcha-plugin-offer", crate::orcha_app::plugin_to_offer);
        }
        self.start_update_check();
        self.start_default_accounts_lookup();
    }
}
