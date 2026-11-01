//! Versioned change log of the public session records, so API clients can
//! patch a cached copy instead of re-reading `session.snapshot` on every event.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::api::schema::{SessionChange, SessionChangeOp, SessionEntity, SessionSnapshot};

/// Changes kept for `session.snapshot { since }`. Older requests get a full snapshot.
const MAX_RETAINED_CHANGES: usize = 4096;
/// Catches changes that reach the records without a lifecycle event, such as a
/// new working directory, while a client watches the session.
pub(crate) const SESSION_JOURNAL_HEARTBEAT: Duration = Duration::from_secs(2);
/// Shortest gap between two rebuilds after events; later events wait for the
/// next loop turn past it, so none is lost.
#[cfg(not(test))]
pub(crate) const SESSION_JOURNAL_MIN_GAP: Duration = Duration::from_millis(200);
#[cfg(test)]
pub(crate) const SESSION_JOURNAL_MIN_GAP: Duration = Duration::ZERO;

pub(crate) struct SessionJournal {
    epoch: String,
    version: u64,
    /// Versions at or below this one are no longer retained.
    floor: u64,
    records: HashMap<(SessionEntity, String), serde_json::Value>,
    batches: VecDeque<(u64, Vec<SessionChange>)>,
    retained_changes: usize,
    pub(crate) last_event_sequence: u64,
    pub(crate) last_refresh: Option<Instant>,
}

impl SessionJournal {
    pub(crate) fn new() -> Self {
        let started = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        Self {
            epoch: format!("{:x}-{:x}", started, std::process::id()),
            version: 0,
            floor: 0,
            records: HashMap::new(),
            batches: VecDeque::new(),
            retained_changes: 0,
            last_event_sequence: 0,
            last_refresh: None,
        }
    }

    pub(crate) fn epoch(&self) -> &str {
        &self.epoch
    }

    pub(crate) fn version(&self) -> u64 {
        self.version
    }

    /// Compares `snapshot` with the last recorded state. Returns the new
    /// version and its changes when anything differs.
    pub(crate) fn refresh(
        &mut self,
        snapshot: &SessionSnapshot,
    ) -> Option<(u64, Vec<SessionChange>)> {
        self.last_refresh = Some(Instant::now());
        let mut current = Vec::new();
        let mut push = |entity, id: &str, record: serde_json::Result<serde_json::Value>| {
            if let Ok(record) = record {
                current.push(((entity, id.to_string()), record));
            }
        };
        for workspace in &snapshot.workspaces {
            push(
                SessionEntity::Workspace,
                &workspace.workspace_id,
                serde_json::to_value(workspace),
            );
        }
        for tab in &snapshot.tabs {
            push(SessionEntity::Tab, &tab.tab_id, serde_json::to_value(tab));
        }
        for pane in &snapshot.panes {
            push(
                SessionEntity::Pane,
                &pane.pane_id,
                serde_json::to_value(pane),
            );
        }
        for agent in &snapshot.agents {
            push(
                SessionEntity::Agent,
                &agent.pane_id,
                serde_json::to_value(agent),
            );
        }

        // Records still in `self.records` after this pass were removed.
        let mut changes = Vec::new();
        let mut next = HashMap::with_capacity(current.len());
        for (key, record) in current {
            let op = match self.records.remove(&key) {
                Some(previous) if previous == record => None,
                Some(_) => Some(SessionChangeOp::Updated),
                None => Some(SessionChangeOp::Added),
            };
            if let Some(op) = op {
                changes.push(SessionChange {
                    entity: key.0,
                    op,
                    id: key.1.clone(),
                    record: Some(record.clone()),
                });
            }
            next.insert(key, record);
        }
        let mut removed: Vec<_> = std::mem::replace(&mut self.records, next)
            .into_keys()
            .collect();
        removed.sort_by(|a, b| (a.0 as u8, &a.1).cmp(&(b.0 as u8, &b.1)));
        for key in removed {
            changes.push(SessionChange {
                entity: key.0,
                op: SessionChangeOp::Removed,
                id: key.1,
                record: None,
            });
        }
        if changes.is_empty() {
            return None;
        }

        self.version += 1;
        self.retained_changes += changes.len();
        self.batches.push_back((self.version, changes.clone()));
        while self.retained_changes > MAX_RETAINED_CHANGES && self.batches.len() > 1 {
            if let Some((version, dropped)) = self.batches.pop_front() {
                self.retained_changes -= dropped.len();
                self.floor = version;
            }
        }
        Some((self.version, changes))
    }

    /// Every change after `since`, oldest first, or `None` when `since` is not
    /// from this journal's retained history and the caller needs a full snapshot.
    pub(crate) fn changes_since(&self, since: u64) -> Option<Vec<SessionChange>> {
        if since < self.floor || since > self.version {
            return None;
        }
        Some(
            self.batches
                .iter()
                .filter(|(version, _)| *version > since)
                .flat_map(|(_, changes)| changes.iter().cloned())
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::api::schema::{AgentStatus, PaneInfo};

    fn pane(pane_id: &str, status: AgentStatus) -> PaneInfo {
        PaneInfo {
            pane_id: pane_id.into(),
            terminal_id: format!("t-{pane_id}"),
            workspace_id: "w1".into(),
            tab_id: "w1:t1".into(),
            focused: false,
            cwd: None,
            foreground_cwd: None,
            restore_error: None,
            label: None,
            agent: None,
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            display_agent: None,
            agent_status: status,
            state_labels: HashMap::new(),
            tokens: HashMap::new(),
            agent_session: None,
            scroll: None,
            revision: 0,
        }
    }

    fn snapshot(panes: Vec<PaneInfo>) -> SessionSnapshot {
        SessionSnapshot {
            version: "test".into(),
            protocol: crate::protocol::PROTOCOL_VERSION,
            focused_workspace_id: None,
            focused_tab_id: None,
            focused_pane_id: None,
            workspaces: Vec::new(),
            tabs: Vec::new(),
            panes,
            layouts: Vec::new(),
            agents: Vec::new(),
            session_version: None,
            session_epoch: None,
        }
    }

    fn ops(changes: &[SessionChange]) -> Vec<(SessionChangeOp, &str)> {
        changes
            .iter()
            .map(|change| (change.op, change.id.as_str()))
            .collect()
    }

    #[test]
    fn refresh_records_added_updated_and_removed_records_with_increasing_versions() {
        let mut journal = SessionJournal::new();
        let (first, changes) = journal
            .refresh(&snapshot(vec![
                pane("w1:p1", AgentStatus::Idle),
                pane("w1:p2", AgentStatus::Idle),
            ]))
            .unwrap();
        assert_eq!(first, 1);
        assert_eq!(
            ops(&changes),
            vec![
                (SessionChangeOp::Added, "w1:p1"),
                (SessionChangeOp::Added, "w1:p2")
            ]
        );

        assert!(journal
            .refresh(&snapshot(vec![
                pane("w1:p1", AgentStatus::Idle),
                pane("w1:p2", AgentStatus::Idle),
            ]))
            .is_none());

        let (second, changes) = journal
            .refresh(&snapshot(vec![pane("w1:p1", AgentStatus::Working)]))
            .unwrap();
        assert_eq!(second, 2);
        assert_eq!(
            ops(&changes),
            vec![
                (SessionChangeOp::Updated, "w1:p1"),
                (SessionChangeOp::Removed, "w1:p2")
            ]
        );
        assert_eq!(
            changes[0].record.as_ref().unwrap()["agent_status"],
            "working"
        );
        assert!(changes[1].record.is_none());
    }

    #[test]
    fn changes_since_returns_only_later_changes_or_none_outside_history() {
        let mut journal = SessionJournal::new();
        journal.refresh(&snapshot(vec![pane("w1:p1", AgentStatus::Idle)]));
        journal.refresh(&snapshot(vec![pane("w1:p1", AgentStatus::Working)]));
        journal.refresh(&snapshot(vec![pane("w1:p1", AgentStatus::Done)]));

        let changes = journal.changes_since(1).unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[1].record.as_ref().unwrap()["agent_status"], "done");
        assert!(journal.changes_since(3).unwrap().is_empty());
        assert!(
            journal.changes_since(4).is_none(),
            "a newer version is from another server"
        );
    }

    #[test]
    fn dropped_history_forces_a_full_snapshot() {
        let mut journal = SessionJournal::new();
        let statuses = [AgentStatus::Idle, AgentStatus::Working];
        for round in 0..=MAX_RETAINED_CHANGES {
            journal.refresh(&snapshot(vec![pane("w1:p1", statuses[round % 2])]));
        }
        assert!(journal.changes_since(0).is_none());
        assert!(journal.changes_since(journal.version() - 1).is_some());
    }
}
