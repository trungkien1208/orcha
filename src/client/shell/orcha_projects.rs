//! Project presentation and local folder selection; runtime creation uses the
//! existing WorkspaceCreate API after the user confirms the chosen path.

use super::*;

/// Project names are projected once per snapshot, never looked up on disk or
/// recomputed in pane rendering. Explicit names remain authoritative.
pub(super) fn normalize_project_labels(snapshot: &mut ClientShellSnapshot) {
    for workspace in &mut snapshot.workspaces {
        if workspace.custom_label || workspace.label != "~" {
            continue;
        }
        let path = workspace.new_workspace_cwd.trim_end_matches(['/', '\\']);
        let folder = path
            .rsplit(['/', '\\'])
            .next()
            .filter(|name| !name.is_empty() && *name != "~");
        workspace.label = format!("{} #{}", folder.unwrap_or("Home"), workspace.number);
    }
}

impl ClientShellState {
    pub(super) fn browse_project_folder(&mut self) {
        if self.active_endpoint_id != ClientEndpointId::Local {
            self.open_new_project_folder_overlay();
            self.set_endpoint_error("Paste the folder path on the remote machine.");
            return;
        }
        if self.orcha_ui.project_folder_picker.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        match crate::thread_spawn::spawn_named("orcha-folder-dialog", move || {
            let _ = tx.send(crate::platform::pick_project_folder());
        }) {
            Ok(_) => self.orcha_ui.project_folder_picker = Some(rx),
            Err(error) => {
                self.open_new_project_folder_overlay();
                self.set_endpoint_error(format!(
                    "Could not open folder dialog: {error}. Paste a path instead."
                ));
            }
        }
    }

    pub(super) fn poll_project_folder_picker(&mut self) -> bool {
        // Do not replace a dialog the user opened while the native picker was up.
        if self.active_endpoint_id == ClientEndpointId::Local
            && (self.overlay.is_some() || self.orcha_ui.popup.is_some())
        {
            return false;
        }
        let Some(receiver) = self.orcha_ui.project_folder_picker.take() else {
            return false;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                self.orcha_ui.project_folder_picker = Some(receiver);
                return false;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err(std::io::Error::other("folder dialog closed unexpectedly"))
            }
        };
        // A local folder must never be submitted to another endpoint.
        if self.active_endpoint_id != ClientEndpointId::Local {
            return false;
        }
        match result {
            Ok(None) => false,
            Ok(Some(path)) => {
                self.overlay = Some(ClientShellOverlay::Rename(ClientRenameOverlay {
                    title: "confirm project folder",
                    input: TextEditor::new(&path.to_string_lossy(), false),
                    target: ClientRenameTarget::NewProjectFolder,
                }));
                true
            }
            Err(error) => {
                self.open_new_project_folder_overlay();
                self.set_endpoint_error(format!(
                    "Could not open folder dialog: {error}. Paste a path instead."
                ));
                true
            }
        }
    }
}
