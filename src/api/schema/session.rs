use serde::{Deserialize, Serialize};

use super::agents::AgentInfo;
use super::panes::{PaneInfo, PaneLayoutSnapshot};
use super::tabs::TabInfo;
use super::workspaces::WorkspaceInfo;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SessionSnapshot {
    pub version: String,
    pub protocol: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused_workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused_tab_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused_pane_id: Option<String>,
    pub workspaces: Vec<WorkspaceInfo>,
    pub tabs: Vec<TabInfo>,
    pub panes: Vec<PaneInfo>,
    pub layouts: Vec<PaneLayoutSnapshot>,
    pub agents: Vec<AgentInfo>,
    /// Journal version this snapshot reflects. Pass it as `since` later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_version: Option<u64>,
    /// Changes when the server restarts; versions from another epoch are void.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_epoch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct SessionSnapshotParams {
    /// Return only the changes after this `session_version` when they are retained.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    /// The `session_epoch` that `since` came from. A different epoch returns a full snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionEntity {
    Workspace,
    Tab,
    Pane,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionChangeOp {
    Added,
    Updated,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SessionChange {
    pub entity: SessionEntity,
    pub op: SessionChangeOp,
    /// Workspace, tab or pane id. Agents use their pane id.
    pub id: String,
    /// The new record, as in `session.snapshot`; absent when removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<serde_json::Value>,
}
