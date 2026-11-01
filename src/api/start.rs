//! `agent.start` with `workspace_id`: opens its own background tab (never
//! focused), optionally on an account, starts the agent there, then applies
//! model settings and the first prompt. A failure closes the tab again, so
//! nothing is left half created.

use crate::api::app_call;
use crate::api::schema::{
    AgentConfigureParams, AgentInfo, AgentStartParams, ErrorBody, ErrorResponse, Method, Request,
    ResponseResult, SuccessResponse, TabCreateParams,
};
use crate::api::server::{dispatch_to_app_with_timeout, ApiConnection};
use crate::api::ApiRequestSender;

/// Whether this request needs the server-side path (the app handles a plain
/// start in an existing pane).
pub(super) fn wants_server_start(params: &AgentStartParams) -> bool {
    params.pane_id.is_empty()
        || params.workspace_id.is_some()
        || params.account_id.is_some()
        || params.prompt.is_some()
        || params.model.is_some()
        || params.effort.is_some()
        || params.fast.is_some()
}

type StartError = (String, String);

pub(super) fn start(
    request_id: &str,
    params: AgentStartParams,
    connection: &mut dyn ApiConnection,
    api_tx: &ApiRequestSender,
    running: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> String {
    match run(request_id, params, connection, api_tx, running) {
        Ok(result) => serde_json::to_string(&SuccessResponse {
            id: request_id.to_string(),
            result,
        })
        .unwrap_or_default(),
        Err((code, message)) => serde_json::to_string(&ErrorResponse {
            id: request_id.to_string(),
            error: ErrorBody { code, message },
        })
        .unwrap_or_default(),
    }
}

fn run(
    request_id: &str,
    mut params: AgentStartParams,
    connection: &mut dyn ApiConnection,
    api_tx: &ApiRequestSender,
    running: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<ResponseResult, StartError> {
    let mut created_tab = None;
    if params.pane_id.is_empty() {
        let workspace_id = params.workspace_id.clone().ok_or_else(|| {
            (
                "invalid_params".to_string(),
                "give pane_id, or workspace_id for a new pane".to_string(),
            )
        })?;
        let env = match params.account_id.as_deref() {
            Some(account_id) => account_env(account_id, &params.kind)?,
            None => Default::default(),
        };
        let created = app(
            request_id,
            Method::TabCreate(TabCreateParams {
                workspace_id: Some(workspace_id),
                cwd: params.cwd.clone(),
                focus: false,
                label: None,
                env,
            }),
            api_tx,
        )?;
        params.pane_id = text(&created, "/result/root_pane/pane_id")?;
        created_tab = Some(text(&created, "/result/tab/tab_id")?);
    } else if params.account_id.is_some() {
        return Err((
            "invalid_params".into(),
            "account_id needs workspace_id: the account's sign-in is set when the pane is created"
                .into(),
        ));
    }

    let started = start_in_pane(request_id, &mut params, api_tx);
    let (agent, argv) = match started {
        Ok(started) => started,
        Err(error) => {
            if let Some(tab_id) = created_tab {
                let _ = app(
                    request_id,
                    Method::TabClose(crate::api::schema::TabTarget { tab_id }),
                    api_tx,
                );
            }
            return Err(error);
        }
    };

    if params.model.is_some() || params.effort.is_some() || params.fast.is_some() {
        let response = crate::api::configure::configure_agent(
            format!("{request_id}:configure"),
            AgentConfigureParams {
                pane_id: agent.pane_id.clone(),
                model: params.model.clone(),
                effort: params.effort.clone(),
                fast: params.fast,
            },
            api_tx,
        );
        fail_on_error(&response, "agent started, but its model settings failed")?;
        // Let the agent take the commands before a prompt follows them.
        if params.prompt.is_some() {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
    if let Some(prompt) = params.prompt.clone() {
        let response = crate::api::wait::prompt_agent_and_confirm(
            format!("{request_id}:prompt"),
            crate::api::schema::AgentPromptParams {
                target: agent.pane_id.clone(),
                text: prompt,
                wait: None,
            },
            connection,
            api_tx,
            running,
        )
        .map_err(|err| ("agent_prompt_failed".to_string(), err.to_string()))?
        .unwrap_or_default();
        fail_on_error(&response, "agent started, but its first prompt failed")?;
    }
    let agent = agent_get(request_id, &agent.pane_id, api_tx).unwrap_or(agent);
    Ok(ResponseResult::AgentStarted { agent, argv })
}

/// Starts in `params.pane_id`, choosing a free `<kind>-<n>` name when none is given.
fn start_in_pane(
    request_id: &str,
    params: &mut AgentStartParams,
    api_tx: &ApiRequestSender,
) -> Result<(AgentInfo, Vec<String>), StartError> {
    let generated = params.name.is_empty();
    for attempt in 1..=20 {
        if generated {
            params.name = format!("{}-{attempt}", params.kind);
        }
        let response = dispatch_to_app_with_timeout(
            Request {
                id: format!("{request_id}:start"),
                method: Method::AgentStart(params.clone()),
            },
            api_tx,
            None,
        );
        let value: serde_json::Value = serde_json::from_str(&response).unwrap_or_default();
        if let Some(error) = value.get("error") {
            let code = error["code"]
                .as_str()
                .unwrap_or("agent_start_failed")
                .to_string();
            if generated && code == "agent_name_taken" {
                continue;
            }
            return Err((code, error["message"].as_str().unwrap_or("").to_string()));
        }
        let agent = serde_json::from_value(value["result"]["agent"].clone()).map_err(|_| {
            (
                "internal_error".to_string(),
                "could not decode the agent".to_string(),
            )
        })?;
        let argv = serde_json::from_value(value["result"]["argv"].clone()).unwrap_or_default();
        return Ok((agent, argv));
    }
    Err((
        "agent_name_conflict".into(),
        format!("no free {}-<n> name", params.kind),
    ))
}

/// Environment that runs an agent of `kind` on `account_id`.
pub(super) fn account_env(
    account_id: &str,
    kind: &str,
) -> Result<std::collections::HashMap<String, String>, StartError> {
    let accounts = crate::orcha_accounts::accounts();
    let account = crate::orcha_accounts::find_account(&accounts, account_id).ok_or_else(|| {
        (
            "account_not_found".to_string(),
            format!("no account {account_id}"),
        )
    })?;
    if !kind.is_empty() && account.kind != kind {
        return Err((
            "invalid_params".into(),
            format!("{account_id} is a {} account, not {kind}", account.kind),
        ));
    }
    if account.kind == "claude" && account.dir.is_none() {
        let default = crate::orcha_accounts::default_account(&accounts, "claude");
        if default.as_deref() != Some(account_id) {
            return Err((
                "account_not_selectable".into(),
                format!(
                    "{account_id} shares the default ~/.claude sign-in, which is on {} now; \
                     make it the default with accounts.set_default or give it its own folder",
                    default.as_deref().unwrap_or("another account")
                ),
            ));
        }
    }
    crate::orcha_accounts::prepare_folder(account);
    let mut env = std::collections::HashMap::new();
    if let Some(dir) = account.dir.clone().filter(|_| account.kind == "claude") {
        env.insert("CLAUDE_CONFIG_DIR".to_string(), dir);
    }
    Ok(env)
}

fn fail_on_error(response: &str, context: &str) -> Result<(), StartError> {
    let value: serde_json::Value = serde_json::from_str(response).unwrap_or_default();
    match value.get("error") {
        Some(error) => Err((
            error["code"]
                .as_str()
                .unwrap_or("agent_start_failed")
                .to_string(),
            format!("{context}: {}", error["message"].as_str().unwrap_or("")),
        )),
        None => Ok(()),
    }
}

fn app(
    request_id: &str,
    method: Method,
    api_tx: &ApiRequestSender,
) -> Result<serde_json::Value, StartError> {
    app_call::call(format!("{request_id}:start"), method, api_tx)
        .map_err(|err| (err.code, err.message))
}

fn text(value: &serde_json::Value, pointer: &str) -> Result<String, StartError> {
    value
        .pointer(pointer)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ("internal_error".into(), format!("missing {pointer}")))
}

fn agent_get(request_id: &str, pane_id: &str, api_tx: &ApiRequestSender) -> Option<AgentInfo> {
    app_call::agent_get(format!("{request_id}:start"), pane_id, api_tx).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(pane_id: &str, workspace_id: Option<&str>) -> AgentStartParams {
        AgentStartParams {
            name: String::new(),
            kind: "claude".into(),
            pane_id: pane_id.into(),
            workspace_id: workspace_id.map(str::to_string),
            account_id: None,
            cwd: None,
            prompt: None,
            model: None,
            effort: None,
            fast: None,
            args: Vec::new(),
            timeout_ms: None,
        }
    }

    struct OpenConnection;

    impl ApiConnection for OpenConnection {
        fn peer_closed(&mut self) -> std::io::Result<bool> {
            Ok(false)
        }

        fn send_line(&mut self, _line: &str) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn plain_starts_in_an_existing_pane_stay_with_the_app() {
        assert!(!wants_server_start(&params("w1:p1", None)));
        assert!(wants_server_start(&params("", Some("w1"))));
        let mut prompted = params("w1:p1", None);
        prompted.prompt = Some("hi".into());
        assert!(wants_server_start(&prompted));
    }

    #[test]
    fn a_failed_start_closes_the_tab_it_created() {
        let (api_tx, mut api_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::api::ApiRequestMessage>();
        let app = std::thread::spawn(move || {
            let mut seen = Vec::new();
            while let Some(message) = api_rx.blocking_recv() {
                let response = match &message.request.method {
                    Method::TabCreate(create) => {
                        assert!(!create.focus, "the new tab must not take focus");
                        seen.push("tab.create");
                        serde_json::json!({"id": message.request.id, "result": {
                            "type": "tab_created",
                            "tab": {"tab_id": "w1:t9"},
                            "root_pane": {"pane_id": "w1:p9"}
                        }})
                    }
                    Method::AgentStart(start) => {
                        seen.push("agent.start");
                        assert_eq!(start.pane_id, "w1:p9");
                        assert_eq!(start.name, "claude-1");
                        serde_json::json!({"id": message.request.id, "error": {
                            "code": "agent_start_timeout", "message": "never became ready"
                        }})
                    }
                    Method::TabClose(target) => {
                        seen.push("tab.close");
                        assert_eq!(target.tab_id, "w1:t9");
                        serde_json::json!({"id": message.request.id, "result": {"type": "ok"}})
                    }
                    other => panic!("unexpected {other:?}"),
                };
                let _ = message.respond_to.send(response.to_string());
            }
            seen
        });
        let response = start(
            "s",
            params("", Some("w1")),
            &mut OpenConnection,
            &api_tx,
            &std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        );
        let value: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(value["error"]["code"], "agent_start_timeout");
        drop(api_tx);
        assert_eq!(
            app.join().unwrap(),
            ["tab.create", "agent.start", "tab.close"]
        );
    }

    #[test]
    fn an_account_needs_a_new_pane() {
        let mut existing = params("w1:p1", None);
        existing.account_id = Some("claude-a".into());
        let (api_tx, _api_rx) = tokio::sync::mpsc::unbounded_channel();
        let response = start(
            "s",
            existing,
            &mut OpenConnection,
            &api_tx,
            &std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        );
        assert!(response.contains("account_id needs workspace_id"));
    }
}
