//! Account methods on the socket: list with fuel, plan and switch a pane's
//! account, sign in, add, remove, rename and set the default, plus the
//! `accounts.changed` event. They run on API threads, never the UI thread,
//! and return profile fields only, never a token.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::api::schema::{
    AccountInfo, AccountPlan, AccountSwitchMode, AccountsPlanParams, AccountsSwitchParams,
    AgentInfo, AgentStatus, ErrorBody, ErrorResponse, EventData, EventEnvelope, EventKind,
    FuelWindowInfo, Method, Request, ResponseResult, SuccessResponse,
};
use crate::api::server::{dispatch_to_app_with_timeout, APP_RESPONSE_TIMEOUT};
use crate::api::{ApiRequestSender, EventHub};

/// How often watched fuel is re-read; the account code keeps fetched numbers
/// for two minutes, so most checks cost nothing.
const POLL_INTERVAL: Duration = Duration::from_secs(60);
const QUEUE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const QUEUE_POLL: Duration = Duration::from_millis(500);

type AccountError = (&'static str, String);

/// Wakes the poller after a change made through the socket, with its reason.
static CHANGED: LazyLock<Mutex<Vec<&'static str>>> = LazyLock::new(Default::default);

fn note_change(reason: &'static str) {
    if let Ok(mut changed) = CHANGED.lock() {
        changed.push(reason);
    }
}

pub(super) fn handle(request: Request, api_tx: &ApiRequestSender) -> Option<String> {
    let id = request.id.clone();
    let result = match request.method {
        Method::AccountsList(params) => {
            list(params.fresh, api_tx).map(|accounts| ResponseResult::AccountsList { accounts })
        }
        Method::AccountsPlan(params) => {
            plan(&params, api_tx).map(|plan| ResponseResult::AccountPlan { plan })
        }
        Method::AccountsSwitch(params) => switch(&id, params, api_tx),
        Method::AccountsLogin(params) => {
            let pane = match params.pane_id {
                Some(pane) => Ok(pane),
                None => background_pane(&id, api_tx),
            };
            pane.and_then(|pane| {
                crate::orcha_accounts::login(&params.account_id, Some(&pane))
                    .map_err(|message| ("account_failed", message))
            })
            .map(|()| {
                note_change("login_started");
                ResponseResult::Ok {}
            })
        }
        Method::AccountsAdd(params) => {
            if params.kind != "claude" {
                Err((
                    "unsupported_kind",
                    format!(
                        "only claude can have more than one account, not {}",
                        params.kind
                    ),
                ))
            } else {
                let pane = match params.pane_id {
                    Some(pane) => Ok(pane),
                    None => background_pane(&id, api_tx),
                };
                pane.and_then(|pane| {
                    crate::orcha_accounts::add(&params.name, Some(&pane))
                        .map_err(|message| ("account_failed", message))
                })
                .map(|account_id| {
                    note_change("added");
                    ResponseResult::AccountAdded { account_id }
                })
            }
        }
        Method::AccountsRemove(target) => crate::orcha_accounts::remove(&target.account_id)
            .map_err(|message| ("account_failed", message))
            .map(|()| {
                note_change("removed");
                ResponseResult::Ok {}
            }),
        Method::AccountsRename(params) => {
            crate::orcha_accounts::rename(&params.account_id, &params.label)
                .map_err(|message| ("account_failed", message))
                .map(|()| {
                    note_change("renamed");
                    ResponseResult::Ok {}
                })
        }
        Method::AccountsSetDefault(target) => {
            crate::orcha_accounts::set_default(&target.account_id)
                .map_err(|message| ("account_failed", message))
                .map(|()| {
                    note_change("default_changed");
                    ResponseResult::Ok {}
                })
        }
        Method::AccountsEnv(target) => crate::api::start::account_env(&target.account_id, "")
            .map(|env| ResponseResult::AccountEnv {
                env: crate::api::schema::AccountEnv {
                    account_id: target.account_id,
                    env,
                },
            })
            .map_err(|(code, message)| {
                let code = match code.as_str() {
                    "account_not_found" => "account_not_found",
                    "account_not_selectable" => "account_not_selectable",
                    _ => "account_failed",
                };
                (code, message)
            }),
        _ => return None,
    };
    Some(encode(id, result))
}

fn encode(id: String, result: Result<ResponseResult, AccountError>) -> String {
    match result {
        Ok(result) => serde_json::to_string(&SuccessResponse { id, result }).unwrap_or_default(),
        Err((code, message)) => serde_json::to_string(&ErrorResponse {
            id,
            error: ErrorBody {
                code: code.into(),
                message,
            },
        })
        .unwrap_or_default(),
    }
}

pub(crate) fn list(
    fresh: bool,
    api_tx: &ApiRequestSender,
) -> Result<Vec<AccountInfo>, AccountError> {
    let configs = crate::orcha_accounts::accounts();
    let agents = agent_list("accounts", api_tx).unwrap_or_default();
    let mut panes: Vec<(String, String)> = Vec::new();
    for agent in &agents {
        let Some(kind) = agent.agent.as_deref() else {
            continue;
        };
        if !configs.iter().any(|account| account.kind == kind) {
            continue;
        }
        if let Some(account) = crate::orcha_accounts::account_of_pane(&configs, &agent.pane_id) {
            panes.push((account, agent.pane_id.clone()));
        }
    }
    Ok(crate::orcha_accounts::accounts_with_fuel(None, fresh)
        .into_iter()
        .map(|account| {
            let config = crate::orcha_accounts::find_account(&configs, &account.id);
            AccountInfo {
                is_default: crate::orcha_accounts::default_account(&configs, &account.kind)
                    .as_deref()
                    == Some(account.id.as_str()),
                plan: config.and_then(crate::orcha_accounts::plan),
                signed_in: account.signed_in.unwrap_or(false),
                fuel: account
                    .windows
                    .iter()
                    .map(|window| FuelWindowInfo {
                        key: window.key.clone(),
                        label: window.label.clone(),
                        left_percent: window.left,
                        reset_at: window.reset_at,
                    })
                    .collect(),
                pane_ids: panes
                    .iter()
                    .filter(|(owner, _)| *owner == account.id)
                    .map(|(_, pane)| pane.clone())
                    .collect(),
                id: account.id,
                kind: account.kind,
                label: account.label,
                email: account.email,
                error: account.error,
            }
        })
        .collect())
}

/// What switching `pane_id` to `account_id` with `mode` would do.
pub(crate) fn plan(
    params: &AccountsPlanParams,
    api_tx: &ApiRequestSender,
) -> Result<AccountPlan, AccountError> {
    let configs = crate::orcha_accounts::accounts();
    let account = crate::orcha_accounts::find_account(&configs, &params.account_id).ok_or((
        "account_not_found",
        format!("no account {}", params.account_id),
    ))?;
    let agent = agent_get("accounts:plan", &params.pane_id, api_tx)?;
    let kind = agent.agent.clone().unwrap_or_default();
    let refuse = |reason: String, needs_idle: bool| AccountPlan {
        allowed: false,
        reason: Some(reason),
        also_moves: Vec::new(),
        needs_idle,
        session_linked: false,
    };
    if kind != account.kind {
        return Ok(refuse(
            format!(
                "{} is a {} account; the pane runs {kind:?}",
                account.id, account.kind
            ),
            false,
        ));
    }
    if crate::orcha_accounts::account_of_pane(&configs, &params.pane_id).as_deref()
        == Some(account.id.as_str())
    {
        return Ok(AccountPlan {
            allowed: true,
            reason: Some("the pane already runs on this account".into()),
            also_moves: Vec::new(),
            needs_idle: false,
            session_linked: true,
        });
    }
    if kind != "claude" {
        return Ok(refuse(format!("{kind} has only one account"), false));
    }
    let session_id = agent
        .agent_session
        .as_ref()
        .and_then(|session| session.id.clone());
    match params.mode {
        AccountSwitchMode::Keep => {
            // Every Claude reading the same sign-in folder moves with the pane.
            let folder = crate::orcha_accounts::claude_folder_of_pane(&params.pane_id);
            let also_moves = agent_list("accounts:plan", api_tx)
                .unwrap_or_default()
                .into_iter()
                .filter(|other| other.pane_id != params.pane_id)
                .filter(|other| other.agent.as_deref() == Some("claude"))
                .filter(|other| {
                    crate::orcha_accounts::claude_folder_of_pane(&other.pane_id) == folder
                })
                .map(|other| other.pane_id)
                .collect();
            Ok(AccountPlan {
                allowed: true,
                reason: None,
                also_moves,
                needs_idle: false,
                session_linked: true,
            })
        }
        AccountSwitchMode::Restart => {
            if cfg!(windows) {
                return Ok(refuse(
                    "restart is not available on Windows yet; use keep".into(),
                    true,
                ));
            }
            Ok(AccountPlan {
                allowed: true,
                reason: session_id
                    .is_none()
                    .then(|| "no session id is known, so a new conversation starts".into()),
                also_moves: Vec::new(),
                needs_idle: true,
                session_linked: session_id.is_some(),
            })
        }
        AccountSwitchMode::Handoff => Ok(AccountPlan {
            allowed: true,
            reason: Some(
                "a new agent starts on the account with a hand-off of this conversation".into(),
            ),
            also_moves: Vec::new(),
            needs_idle: true,
            session_linked: false,
        }),
    }
}

fn switch(
    request_id: &str,
    params: AccountsSwitchParams,
    api_tx: &ApiRequestSender,
) -> Result<ResponseResult, AccountError> {
    let plan = plan(
        &AccountsPlanParams {
            pane_id: params.pane_id.clone(),
            account_id: params.account_id.clone(),
            mode: params.mode,
        },
        api_tx,
    )?;
    if !plan.allowed {
        return Err((
            "switch_not_allowed",
            plan.reason.clone().unwrap_or_else(|| "not allowed".into()),
        ));
    }
    if !plan.also_moves.is_empty() && !params.move_all {
        return Err((
            "also_moves",
            format!(
                "{} share this sign-in and would switch too; pass move_all: true",
                plan.also_moves.join(", ")
            ),
        ));
    }
    let agent = agent_get(request_id, &params.pane_id, api_tx)?;
    if plan.needs_idle && is_busy(&agent) {
        if !params.when_idle {
            return Err((
                "agent_busy",
                "the agent is working; pass when_idle: true to switch once it is idle".into(),
            ));
        }
        let api_tx = api_tx.clone();
        let request_id = request_id.to_string();
        let queued = params.clone();
        crate::thread_spawn::spawn_named("herdr-account-switch", move || {
            let deadline = Instant::now() + QUEUE_TIMEOUT;
            while Instant::now() < deadline {
                std::thread::sleep(QUEUE_POLL);
                match agent_get(&request_id, &queued.pane_id, &api_tx) {
                    Ok(agent) if !is_busy(&agent) => {
                        if let Err((_, message)) = apply(&request_id, &queued, &agent, &api_tx) {
                            tracing::warn!(%message, "queued account switch failed");
                        }
                        return;
                    }
                    Ok(_) => {}
                    Err(_) => return,
                }
            }
        })
        .map_err(|err| {
            (
                "switch_failed",
                format!("could not queue the switch: {err}"),
            )
        })?;
        return Ok(ResponseResult::AccountSwitched {
            outcome: "queued".into(),
            plan,
            pane_id: None,
        });
    }
    let pane_id = apply(request_id, &params, &agent, api_tx)?;
    Ok(ResponseResult::AccountSwitched {
        outcome: "applied".into(),
        plan,
        pane_id: Some(pane_id),
    })
}

fn apply(
    request_id: &str,
    params: &AccountsSwitchParams,
    agent: &AgentInfo,
    api_tx: &ApiRequestSender,
) -> Result<String, AccountError> {
    let pane_id = match params.mode {
        AccountSwitchMode::Keep => {
            crate::orcha_accounts::switch(&params.account_id, &params.pane_id)
                .map_err(|message| ("switch_failed", message))?;
            params.pane_id.clone()
        }
        AccountSwitchMode::Restart => {
            restart_claude(
                request_id,
                &params.pane_id,
                &params.account_id,
                agent,
                api_tx,
            )?;
            params.pane_id.clone()
        }
        AccountSwitchMode::Handoff => crate::api::handoff::handoff_to_account(
            request_id,
            &params.pane_id,
            &params.account_id,
            api_tx,
        )?,
    };
    note_change("switched");
    Ok(pane_id)
}

/// Stops Claude in the pane and resumes the same conversation in the
/// account's sign-in folder.
fn restart_claude(
    request_id: &str,
    pane_id: &str,
    account_id: &str,
    agent: &AgentInfo,
    api_tx: &ApiRequestSender,
) -> Result<(), AccountError> {
    let configs = crate::orcha_accounts::accounts();
    let account = crate::orcha_accounts::find_account(&configs, account_id)
        .ok_or(("account_not_found", format!("no account {account_id}")))?;
    crate::orcha_accounts::prepare_folder(account);
    let env = match account.dir.as_deref() {
        Some(dir) => format!("CLAUDE_CONFIG_DIR={} ", shell_quote(dir)),
        None => "env -u CLAUDE_CONFIG_DIR ".into(),
    };
    let resume = agent
        .agent_session
        .as_ref()
        .and_then(|session| session.id.as_deref())
        .map(|session| format!(" --resume {}", shell_quote(session)))
        .unwrap_or_default();
    for _ in 0..2 {
        app_request(
            request_id,
            Method::PaneSendKeys(crate::api::schema::PaneSendKeysParams {
                pane_id: pane_id.to_string(),
                keys: vec!["ctrl+c".into()],
            }),
            api_tx,
        )?;
        std::thread::sleep(Duration::from_millis(400));
    }
    std::thread::sleep(Duration::from_secs(1));
    app_request(
        request_id,
        Method::PaneSendInput(crate::api::schema::PaneSendInputParams {
            pane_id: pane_id.to_string(),
            text: format!("{env}claude{resume}"),
            keys: vec!["Enter".into()],
        }),
        api_tx,
    )?;
    Ok(())
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn is_busy(agent: &AgentInfo) -> bool {
    matches!(
        agent.agent_status,
        AgentStatus::Working | AgentStatus::Blocked
    )
}

/// A pane for a sign-in that does not touch the user's layout: the root
/// pane of a new workspace that does not take focus.
fn background_pane(request_id: &str, api_tx: &ApiRequestSender) -> Result<String, AccountError> {
    let value = app_request(
        request_id,
        Method::WorkspaceCreate(crate::api::schema::WorkspaceCreateParams {
            source_workspace_id: None,
            cwd: None,
            focus: false,
            label: Some("sign in".into()),
            env: Default::default(),
        }),
        api_tx,
    )?;
    value
        .pointer("/result/root_pane/pane_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or((
            "account_failed",
            "could not open a pane for the sign-in".into(),
        ))
}

fn app_request(
    request_id: &str,
    method: Method,
    api_tx: &ApiRequestSender,
) -> Result<serde_json::Value, AccountError> {
    let response = dispatch_to_app_with_timeout(
        Request {
            id: format!("{request_id}:accounts"),
            method,
        },
        api_tx,
        Some(APP_RESPONSE_TIMEOUT),
    );
    let value: serde_json::Value = serde_json::from_str(&response).unwrap_or_default();
    if let Some(error) = value.get("error") {
        return Err((
            "account_failed",
            error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("request failed")
                .to_string(),
        ));
    }
    Ok(value)
}

fn agent_get(
    request_id: &str,
    pane_id: &str,
    api_tx: &ApiRequestSender,
) -> Result<AgentInfo, AccountError> {
    let value = app_request(
        request_id,
        Method::AgentGet(crate::api::schema::AgentTarget {
            target: pane_id.to_string(),
        }),
        api_tx,
    )
    .map_err(|(_, message)| ("agent_not_found", message))?;
    serde_json::from_value(value["result"]["agent"].clone())
        .map_err(|_| ("internal_error", "could not decode the agent".to_string()))
}

fn agent_list(request_id: &str, api_tx: &ApiRequestSender) -> Option<Vec<AgentInfo>> {
    let value = app_request(request_id, Method::AgentList(Default::default()), api_tx).ok()?;
    serde_json::from_value(value.pointer("/result/agents")?.clone()).ok()
}

/// Publishes `accounts.changed` while someone subscribes: after a change made
/// through the socket at once, and otherwise when a minute's re-read differs.
pub(super) fn spawn_watcher(
    api_tx: ApiRequestSender,
    event_hub: EventHub,
    running: Arc<AtomicBool>,
) {
    let spawned = crate::thread_spawn::spawn_named("herdr-accounts-watch", move || {
        let mut last: Option<Vec<AccountInfo>> = None;
        let mut next_poll = Instant::now();
        while running.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(250));
            if !event_hub.accounts_watched() {
                last = None;
                continue;
            }
            let reasons: Vec<&'static str> = CHANGED
                .lock()
                .map(|mut changed| std::mem::take(&mut *changed))
                .unwrap_or_default();
            if reasons.is_empty() && Instant::now() < next_poll {
                continue;
            }
            next_poll = Instant::now() + POLL_INTERVAL;
            let Ok(accounts) = list(false, &api_tx) else {
                continue;
            };
            let reason = match (&last, reasons.first()) {
                (_, Some(reason)) => Some(*reason),
                (None, None) => None,
                (Some(previous), None) => change_reason(previous, &accounts),
            };
            last = Some(accounts.clone());
            if let Some(reason) = reason {
                event_hub.push(EventEnvelope {
                    event: EventKind::AccountsChanged,
                    data: EventData::AccountsChanged {
                        reason: reason.into(),
                        accounts,
                    },
                });
            }
        }
    });
    if let Err(err) = spawned {
        tracing::warn!(err = %err, "accounts watcher unavailable");
    }
}

/// Why two account lists differ, or `None` when they do not.
pub(super) fn change_reason(
    previous: &[AccountInfo],
    current: &[AccountInfo],
) -> Option<&'static str> {
    if previous == current {
        return None;
    }
    let ids = |accounts: &[AccountInfo]| accounts.iter().map(|a| a.id.clone()).collect::<Vec<_>>();
    if ids(previous) != ids(current) {
        return Some("list_changed");
    }
    let pairs = previous.iter().zip(current);
    if pairs.clone().any(|(a, b)| !a.signed_in && b.signed_in) {
        return Some("signed_in");
    }
    if pairs.clone().any(|(a, b)| a.is_default != b.is_default) {
        return Some("default_changed");
    }
    if pairs
        .clone()
        .any(|(a, b)| a.fuel != b.fuel || a.error != b.error)
    {
        return Some("fuel_refreshed");
    }
    Some("updated")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str, signed_in: bool, is_default: bool, left: u8) -> AccountInfo {
        AccountInfo {
            id: id.into(),
            kind: "claude".into(),
            label: id.into(),
            email: None,
            plan: None,
            signed_in,
            is_default,
            error: None,
            fuel: vec![FuelWindowInfo {
                key: "session".into(),
                label: "5H".into(),
                left_percent: left,
                reset_at: Some(1),
            }],
            pane_ids: Vec::new(),
        }
    }

    #[test]
    fn change_reasons_name_what_changed() {
        let base = vec![account("a", true, true, 50), account("b", false, false, 50)];
        assert_eq!(change_reason(&base, &base), None);
        assert_eq!(change_reason(&base, &base[..1]), Some("list_changed"));
        let mut signed = base.clone();
        signed[1].signed_in = true;
        assert_eq!(change_reason(&base, &signed), Some("signed_in"));
        let mut default = base.clone();
        default[0].is_default = false;
        default[1].is_default = true;
        assert_eq!(change_reason(&base, &default), Some("default_changed"));
        let mut fuel = base.clone();
        fuel[0].fuel[0].left_percent = 40;
        assert_eq!(change_reason(&base, &fuel), Some("fuel_refreshed"));
    }

    #[test]
    fn shell_quote_keeps_quotes_safe() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    /// Answers `agent.get` with an agent of `kind`.
    fn spawn_agent(kind: &'static str) -> ApiRequestSender {
        let (api_tx, mut api_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::api::ApiRequestMessage>();
        std::thread::spawn(move || {
            while let Some(message) = api_rx.blocking_recv() {
                let agent: AgentInfo = serde_json::from_value(serde_json::json!({
                    "terminal_id": "t1", "agent": kind, "agent_status": "idle",
                    "workspace_id": "w1", "tab_id": "w1:t1", "pane_id": "w1:p1",
                    "focused": false, "revision": 0
                }))
                .unwrap();
                let result = match message.request.method {
                    Method::AgentGet(_) => ResponseResult::AgentInfo { agent },
                    Method::AgentList(_) => ResponseResult::AgentList {
                        agents: vec![agent],
                    },
                    other => panic!("unexpected {other:?}"),
                };
                let _ = message.respond_to.send(
                    serde_json::to_string(&SuccessResponse {
                        id: message.request.id,
                        result,
                    })
                    .unwrap(),
                );
            }
        });
        api_tx
    }

    fn with_accounts_config(test: impl FnOnce()) {
        let base = std::env::temp_dir().join(format!("herdr-accounts-{}", std::process::id()));
        let orcha = base.join("orcha");
        std::fs::create_dir_all(&orcha).unwrap();
        std::fs::write(
            orcha.join("config.json"),
            r#"{"accounts":[{"id":"claude-a","kind":"claude","label":"A","dir":"/tmp/claude-a"},{"id":"codex","kind":"codex","label":"Codex"}]}"#,
        )
        .unwrap();
        std::env::set_var("XDG_CONFIG_HOME", &base);
        test();
        std::env::remove_var("XDG_CONFIG_HOME");
        let _ = std::fs::remove_dir_all(base);
    }

    fn plan_for(kind: &'static str, account_id: &str, mode: AccountSwitchMode) -> AccountPlan {
        plan(
            &AccountsPlanParams {
                pane_id: "w1:p1".into(),
                account_id: account_id.into(),
                mode,
            },
            &spawn_agent(kind),
        )
        .unwrap()
    }

    #[test]
    fn plans_refuse_other_kinds_and_single_account_kinds() {
        with_accounts_config(|| {
            let refused = plan_for("cursor", "claude-a", AccountSwitchMode::Keep);
            assert!(!refused.allowed);
            assert!(refused.reason.unwrap().contains("claude account"));

            let refused = plan_for("codex", "codex", AccountSwitchMode::Restart);
            assert!(!refused.allowed);
            assert_eq!(
                refused.reason.as_deref(),
                Some("codex has only one account")
            );

            let error = plan(
                &AccountsPlanParams {
                    pane_id: "w1:p1".into(),
                    account_id: "nobody".into(),
                    mode: AccountSwitchMode::Keep,
                },
                &spawn_agent("claude"),
            )
            .unwrap_err();
            assert_eq!(error.0, "account_not_found");
        });
    }

    #[test]
    fn claude_plans_describe_each_mode() {
        with_accounts_config(|| {
            let keep = plan_for("claude", "claude-a", AccountSwitchMode::Keep);
            assert!(keep.allowed && keep.session_linked && !keep.needs_idle);
            let handoff = plan_for("claude", "claude-a", AccountSwitchMode::Handoff);
            assert!(handoff.allowed && handoff.needs_idle && !handoff.session_linked);
            if !cfg!(windows) {
                let restart = plan_for("claude", "claude-a", AccountSwitchMode::Restart);
                assert!(restart.allowed && restart.needs_idle);
                assert!(!restart.session_linked, "no session id is known");
            }
        });
    }

    #[test]
    fn rename_changes_only_the_label() {
        with_accounts_config(|| {
            crate::orcha_accounts::rename("codex", "Codex · work").unwrap();
            let accounts = crate::orcha_accounts::accounts();
            let codex = crate::orcha_accounts::find_account(&accounts, "codex").unwrap();
            assert_eq!(codex.label, "Codex · work");
            assert!(crate::orcha_accounts::rename("nobody", "x").is_err());
            assert!(crate::orcha_accounts::rename("codex", "  ").is_err());
        });
    }
}
