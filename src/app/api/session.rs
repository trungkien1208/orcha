use crate::api::schema::{
    EventData, EventEnvelope, EventKind, PaneInfo, ResponseResult, SessionSnapshot,
    SessionSnapshotParams,
};
use crate::app::App;

use super::responses::encode_success;

impl App {
    pub(super) fn handle_session_snapshot(
        &mut self,
        id: String,
        params: SessionSnapshotParams,
    ) -> String {
        self.refresh_session_journal();
        let journal = &self.session_journal;
        let same_epoch = params
            .epoch
            .as_deref()
            .is_none_or(|epoch| epoch == journal.epoch());
        if let Some(changes) = params
            .since
            .filter(|_| same_epoch)
            .and_then(|since| journal.changes_since(since))
        {
            return encode_success(
                id,
                ResponseResult::SessionDelta {
                    session_version: journal.version(),
                    session_epoch: journal.epoch().to_string(),
                    changes,
                },
            );
        }
        let mut snapshot = self.session_snapshot();
        snapshot.session_version = Some(self.session_journal.version());
        snapshot.session_epoch = Some(self.session_journal.epoch().to_string());
        encode_success(
            id,
            ResponseResult::SessionSnapshot {
                session_version: snapshot.session_version,
                session_epoch: snapshot.session_epoch.clone(),
                snapshot: Box::new(snapshot),
            },
        )
    }

    /// Publishes `session.changed` while a client watches the session: after
    /// lifecycle events, and on a slow heartbeat for changes without one.
    pub(crate) fn sync_session_journal(&mut self) {
        if !self.event_hub.session_watched() {
            return;
        }
        let events_moved =
            self.event_hub.current_sequence() != self.session_journal.last_event_sequence;
        let heartbeat_due = self.session_journal.last_refresh.is_none_or(|at| {
            at.elapsed() >= super::super::session_journal::SESSION_JOURNAL_HEARTBEAT
        });
        if events_moved || heartbeat_due {
            self.refresh_session_journal();
        }
    }

    fn refresh_session_journal(&mut self) {
        let snapshot = self.session_metadata_snapshot();
        if let Some((version, changes)) = self.session_journal.refresh(&snapshot) {
            // Journal events bypass plugin hooks, like other derived metadata events.
            self.event_hub.push(EventEnvelope {
                event: EventKind::SessionChanged,
                data: EventData::SessionChanged {
                    version,
                    epoch: self.session_journal.epoch().to_string(),
                    changes,
                },
            });
        }
        self.session_journal.last_event_sequence = self.event_hub.current_sequence();
    }

    pub(crate) fn session_snapshot(&self) -> SessionSnapshot {
        self.session_snapshot_with_pane_info(Self::pane_info)
    }

    pub(crate) fn session_metadata_snapshot(&self) -> SessionSnapshot {
        self.session_snapshot_with_pane_info(Self::pane_metadata)
    }

    fn session_snapshot_with_pane_info(
        &self,
        pane_info: fn(&Self, usize, crate::layout::PaneId) -> Option<PaneInfo>,
    ) -> SessionSnapshot {
        let focused_workspace_id = self
            .state
            .active
            .map(|ws_idx| self.public_workspace_id(ws_idx));
        let focused_tab_id = self.state.active.and_then(|ws_idx| {
            let ws = self.state.workspaces.get(ws_idx)?;
            self.public_tab_id(ws_idx, ws.active_tab)
        });
        let focused_pane_id = self.state.active.and_then(|ws_idx| {
            let ws = self.state.workspaces.get(ws_idx)?;
            self.public_pane_id(ws_idx, ws.focused_pane_id()?)
        });

        let mut workspaces = Vec::new();
        let mut tabs = Vec::new();
        let mut layouts = Vec::new();
        let mut panes = Vec::new();
        for (ws_idx, ws) in self.state.workspaces.iter().enumerate() {
            workspaces.push(self.workspace_info(ws_idx));
            for tab_idx in 0..ws.tabs.len() {
                if let Some(tab) = self.tab_info(ws_idx, tab_idx) {
                    tabs.push(tab);
                }
                if let Some(layout) = self.pane_layout_snapshot(ws_idx, tab_idx) {
                    layouts.push(layout);
                }
                panes.extend(
                    ws.tabs[tab_idx]
                        .layout
                        .pane_ids()
                        .into_iter()
                        .filter_map(|pane_id| pane_info(self, ws_idx, pane_id)),
                );
            }
        }

        SessionSnapshot {
            version: crate::build_info::version(),
            protocol: crate::protocol::PROTOCOL_VERSION,
            focused_workspace_id,
            focused_tab_id,
            focused_pane_id,
            workspaces,
            tabs,
            panes,
            layouts,
            agents: self.collect_agent_infos(),
            session_version: None,
            session_epoch: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::api::schema::{Method, ResponseResult, SuccessResponse};
    use crate::{config::Config, workspace::Workspace};

    fn app_with_two_tabs() -> crate::app::App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = crate::app::App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        let mut workspace = Workspace::test_new("snapshot");
        workspace.test_add_tab(None);
        app.state.workspaces = vec![workspace];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app
    }

    #[test]
    fn session_snapshot_bootstraps_runtime_resources() {
        let mut app = app_with_two_tabs();
        let response = app.handle_api_request(crate::api::schema::Request {
            id: "req_snapshot".into(),
            method: Method::SessionSnapshot(Default::default()),
        });

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::SessionSnapshot { snapshot, .. } = success.result else {
            panic!("expected session snapshot response");
        };
        assert_eq!(success.id, "req_snapshot");
        assert_eq!(snapshot.workspaces.len(), 1);
        assert_eq!(snapshot.tabs.len(), 2);
        assert_eq!(snapshot.panes.len(), 2);
        assert_eq!(snapshot.layouts.len(), 2);
        assert_eq!(
            snapshot.focused_workspace_id.as_deref(),
            Some(snapshot.workspaces[0].workspace_id.as_str())
        );
        assert_eq!(
            snapshot.focused_tab_id.as_deref(),
            Some(snapshot.tabs[0].tab_id.as_str())
        );
        assert_eq!(
            snapshot.focused_pane_id.as_deref(),
            Some(snapshot.panes[0].pane_id.as_str())
        );
    }

    fn snapshot_request(
        app: &mut crate::app::App,
        since: Option<u64>,
        epoch: Option<String>,
    ) -> ResponseResult {
        let response = app.handle_api_request(crate::api::schema::Request {
            id: "req".into(),
            method: Method::SessionSnapshot(crate::api::schema::SessionSnapshotParams {
                since,
                epoch,
            }),
        });
        serde_json::from_str::<SuccessResponse>(&response)
            .unwrap()
            .result
    }

    fn rename_first_workspace(app: &mut crate::app::App, label: &str) -> String {
        let workspace_id = app.public_workspace_id(0);
        app.handle_api_request(crate::api::schema::Request {
            id: "rename".into(),
            method: Method::WorkspaceRename(crate::api::schema::WorkspaceRenameParams {
                workspace_id: workspace_id.clone(),
                label: label.into(),
            }),
        });
        workspace_id
    }

    #[test]
    fn session_snapshot_since_returns_only_the_changes_after_that_version() {
        let mut app = app_with_two_tabs();
        let ResponseResult::SessionSnapshot {
            snapshot,
            session_version,
            session_epoch,
        } = snapshot_request(&mut app, None, None)
        else {
            panic!("expected a full snapshot first");
        };
        assert_eq!(session_version, snapshot.session_version);
        assert_eq!(session_epoch, snapshot.session_epoch);
        let version = snapshot.session_version.unwrap();
        let epoch = snapshot.session_epoch.clone().unwrap();

        let ResponseResult::SessionDelta {
            session_version,
            changes,
            ..
        } = snapshot_request(&mut app, Some(version), Some(epoch.clone()))
        else {
            panic!("expected a delta");
        };
        assert_eq!(session_version, version);
        assert!(changes.is_empty());

        let workspace_id = rename_first_workspace(&mut app, "renamed");
        let ResponseResult::SessionDelta {
            session_version,
            changes,
            ..
        } = snapshot_request(&mut app, Some(version), Some(epoch))
        else {
            panic!("expected a delta");
        };
        assert!(session_version > version);
        let workspace = changes
            .iter()
            .find(|change| change.entity == crate::api::schema::SessionEntity::Workspace)
            .expect("workspace change");
        assert_eq!(workspace.op, crate::api::schema::SessionChangeOp::Updated);
        assert_eq!(workspace.id, workspace_id);
        assert_eq!(workspace.record.as_ref().unwrap()["label"], "renamed");
    }

    #[test]
    fn session_snapshot_since_from_another_epoch_or_future_returns_a_full_snapshot() {
        let mut app = app_with_two_tabs();
        assert!(matches!(
            snapshot_request(&mut app, Some(1), Some("other-server".into())),
            ResponseResult::SessionSnapshot { .. }
        ));
        assert!(matches!(
            snapshot_request(&mut app, Some(999), None),
            ResponseResult::SessionSnapshot { .. }
        ));
    }

    #[test]
    fn session_changed_is_published_only_while_watched() {
        let mut app = app_with_two_tabs();
        let session_changed = |app: &crate::app::App| {
            app.event_hub
                .events_after(0)
                .into_iter()
                .filter(|(_, event)| event.event == crate::api::schema::EventKind::SessionChanged)
                .count()
        };

        rename_first_workspace(&mut app, "unwatched");
        app.sync_session_journal();
        assert_eq!(session_changed(&app), 0);

        let watch = app.event_hub.watch_session();
        app.sync_session_journal();
        assert_eq!(
            session_changed(&app),
            1,
            "first refresh records the session"
        );
        app.sync_session_journal();
        assert_eq!(session_changed(&app), 1, "nothing changed");

        rename_first_workspace(&mut app, "watched");
        app.sync_session_journal();
        let events = app.event_hub.events_after(0);
        let (_, last) = events
            .iter()
            .rev()
            .find(|(_, event)| event.event == crate::api::schema::EventKind::SessionChanged)
            .unwrap();
        let crate::api::schema::EventData::SessionChanged {
            version, changes, ..
        } = &last.data
        else {
            panic!("expected session.changed data");
        };
        assert_eq!(*version, 2);
        assert!(changes.iter().any(|change| change
            .record
            .as_ref()
            .is_some_and(|record| record["label"] == "watched")));

        drop(watch);
        rename_first_workspace(&mut app, "dropped");
        app.sync_session_journal();
        assert_eq!(session_changed(&app), 2);
    }
}
