use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct AccountsListParams {
    /// Ask each service for new fuel numbers instead of using ones under two minutes old.
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub fresh: bool,
}

/// An agent account. Profile fields only: never a token or credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AccountInfo {
    pub id: String,
    /// `claude`, `codex`, `cursor`, ...
    pub kind: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Plan name from the sign-in profile, such as `pro`, `max` or `plus`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    pub signed_in: bool,
    /// New agents of this kind use this account.
    pub is_default: bool,
    /// Why fuel could not be read, such as an expired sign-in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub fuel: Vec<FuelWindowInfo>,
    /// Panes whose agent runs on this account.
    #[serde(default)]
    pub pane_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FuelWindowInfo {
    /// `session`, `weekly` or `monthly`.
    pub key: String,
    /// `5H`, `WEEK` or `MONTH`.
    pub label: String,
    pub left_percent: u8,
    /// When the window refills, Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccountSwitchMode {
    /// Swap the sign-in under the running agent; the conversation goes on.
    Keep,
    /// Stop the agent and continue the same conversation on the account.
    Restart,
    /// Start a new agent on the account with a hand-off of the conversation.
    Handoff,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AccountsPlanParams {
    pub pane_id: String,
    pub account_id: String,
    pub mode: AccountSwitchMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AccountsSwitchParams {
    pub pane_id: String,
    pub account_id: String,
    pub mode: AccountSwitchMode,
    /// Required when the plan's `also_moves` is not empty: those panes move too.
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub move_all: bool,
    /// Wait for a busy agent to become idle instead of failing.
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub when_idle: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AccountPlan {
    pub allowed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Other panes that switch too, because they share the sign-in.
    #[serde(default)]
    pub also_moves: Vec<String>,
    /// The agent must be idle (it is stopped and started again).
    pub needs_idle: bool,
    /// The conversation continues on the new account.
    pub session_linked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AccountTarget {
    pub account_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AccountsLoginParams {
    pub account_id: String,
    /// Open the sign-in beside this pane; default: a new workspace in the background.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AccountsAddParams {
    /// Name such as `work`; the account becomes `claude-work`.
    pub name: String,
    /// Only `claude` can have more than one account.
    #[serde(default = "default_claude")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
}

fn default_claude() -> String {
    "claude".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AccountsRenameParams {
    pub account_id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AccountEnv {
    pub account_id: String,
    /// Environment that makes an agent run on this account, for
    /// `workspace.create`, `tab.create` or `pane.split` `env`. Paths only.
    pub env: std::collections::HashMap<String, String>,
}
