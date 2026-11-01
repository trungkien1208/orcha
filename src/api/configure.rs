//! `agent.configure`: changes an agent's model, effort and fast mode with the
//! agent's own commands, as one action, waiting for the agent to be idle.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::api::app_call;
use crate::api::schema::{
    AgentConfigureParams, AgentInfo, AgentModelInfo, AgentStatus, ErrorBody, ErrorResponse, Method,
    Request, ResponseResult, SuccessResponse,
};
use crate::api::server::dispatch_to_app_with_timeout;
use crate::api::ApiRequestSender;

/// Gap between commands, so each one is handled before the next is typed.
const COMMAND_GAP: Duration = Duration::from_millis(800);
/// A queued change gives up if the agent stays busy this long.
const QUEUE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const QUEUE_POLL: Duration = Duration::from_millis(500);

/// The latest queued change per pane; an older queued change stops when a
/// newer one replaces it.
static QUEUED: LazyLock<Mutex<HashMap<String, u64>>> = LazyLock::new(Default::default);
static NEXT_TICKET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub(super) fn configure_agent(
    request_id: String,
    params: AgentConfigureParams,
    api_tx: &ApiRequestSender,
) -> String {
    match plan_and_run(&request_id, params, api_tx) {
        Ok(result) => serde_json::to_string(&SuccessResponse {
            id: request_id,
            result,
        })
        .unwrap_or_default(),
        Err((code, message)) => serde_json::to_string(&ErrorResponse {
            id: request_id,
            error: ErrorBody {
                code: code.into(),
                message,
            },
        })
        .unwrap_or_default(),
    }
}

type ConfigureError = (&'static str, String);

fn plan_and_run(
    request_id: &str,
    params: AgentConfigureParams,
    api_tx: &ApiRequestSender,
) -> Result<ResponseResult, ConfigureError> {
    if params.model.is_none() && params.effort.is_none() && params.fast.is_none() {
        return Err((
            "invalid_params",
            "give at least one of model, effort or fast".into(),
        ));
    }
    let agent = agent_get(request_id, &params.pane_id, api_tx)?;
    let kind = agent.agent.clone().unwrap_or_default();
    if kind == "cursor" {
        return Err((
            "unsupported_agent",
            "Cursor's /model picker filters by display name and cannot be driven reliably; \
             start a new Cursor agent with the model instead"
                .into(),
        ));
    }
    if !matches!(kind.as_str(), "claude" | "codex") {
        return Err((
            "unsupported_agent",
            format!("agent.configure supports claude and codex, not {kind:?}"),
        ));
    }
    let catalog =
        crate::agent_models::catalog(&kind).map_err(|message| ("models_unavailable", message))?;
    let commands = plan_commands(&kind, &agent, &params, &catalog.models)?;
    if commands.is_empty() {
        return Ok(ResponseResult::AgentConfigured {
            outcome: "applied".into(),
            commands,
        });
    }

    let ticket = NEXT_TICKET.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut queued) = QUEUED.lock() {
        queued.insert(params.pane_id.clone(), ticket);
    }
    if is_busy(&agent) {
        let api_tx = api_tx.clone();
        let pane_id = params.pane_id.clone();
        let queued_commands = commands.clone();
        let request_id = request_id.to_string();
        crate::thread_spawn::spawn_named("herdr-agent-configure", move || {
            let deadline = Instant::now() + QUEUE_TIMEOUT;
            while Instant::now() < deadline && still_current(&pane_id, ticket) {
                std::thread::sleep(QUEUE_POLL);
                match agent_get(&request_id, &pane_id, &api_tx) {
                    Ok(agent) if !is_busy(&agent) => {
                        if still_current(&pane_id, ticket) {
                            let _ = send_commands(&request_id, &pane_id, &queued_commands, &api_tx);
                        }
                        break;
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            forget(&pane_id, ticket);
        })
        .map_err(|err| {
            (
                "configure_failed",
                format!("could not queue the change: {err}"),
            )
        })?;
        return Ok(ResponseResult::AgentConfigured {
            outcome: "queued".into(),
            commands,
        });
    }
    let sent = send_commands(request_id, &params.pane_id, &commands, api_tx);
    forget(&params.pane_id, ticket);
    sent?;
    Ok(ResponseResult::AgentConfigured {
        outcome: "applied".into(),
        commands,
    })
}

fn still_current(pane_id: &str, ticket: u64) -> bool {
    QUEUED
        .lock()
        .map(|queued| queued.get(pane_id) == Some(&ticket))
        .unwrap_or(false)
}

fn forget(pane_id: &str, ticket: u64) {
    if let Ok(mut queued) = QUEUED.lock() {
        if queued.get(pane_id) == Some(&ticket) {
            queued.remove(pane_id);
        }
    }
}

fn is_busy(agent: &AgentInfo) -> bool {
    matches!(
        agent.agent_status,
        AgentStatus::Working | AgentStatus::Blocked
    ) || agent.launch_pending
}

/// The agent commands that turn the current settings into the requested ones.
pub(super) fn plan_commands(
    kind: &str,
    agent: &AgentInfo,
    params: &AgentConfigureParams,
    models: &[AgentModelInfo],
) -> Result<Vec<String>, ConfigureError> {
    let model_id = params
        .model
        .clone()
        .or_else(|| agent.model.clone())
        .ok_or((
            "model_unknown",
            "the agent's current model is not known yet; pass model as well".to_string(),
        ))?;
    let model = models
        .iter()
        .find(|model| model.id == model_id)
        .ok_or_else(|| ("unknown_model", format!("{kind} has no model {model_id}")))?;
    if let Some(effort) = params.effort.as_deref() {
        if !model.efforts.iter().any(|known| known == effort) {
            return Err((
                "invalid_effort",
                format!(
                    "{model_id} accepts efforts {}, not {effort}",
                    model.efforts.join(", ")
                ),
            ));
        }
    }
    if params.fast == Some(true) {
        let effort = params.effort.clone().or_else(|| agent.effort.clone());
        let effort_allows = model.fast_efforts.is_empty()
            || effort.is_some_and(|effort| model.fast_efforts.contains(&effort));
        if !model.fast_available || !effort_allows {
            return Err((
                "fast_unavailable",
                format!("{model_id} has no fast mode here"),
            ));
        }
    }

    let model_changes = params
        .model
        .as_ref()
        .is_some_and(|model| agent.model.as_ref() != Some(model));
    let effort_changes = params
        .effort
        .as_ref()
        .is_some_and(|effort| agent.effort.as_ref() != Some(effort));
    let fast_changes = params.fast.is_some_and(|fast| agent.fast != Some(fast));
    match kind {
        "claude" => {
            let mut commands = Vec::new();
            if model_changes {
                commands.push(format!("/model {model_id}"));
            }
            if effort_changes {
                commands.push(format!(
                    "/effort {}",
                    params.effort.as_deref().unwrap_or_default()
                ));
            }
            if fast_changes {
                // Claude's /fast toggles, so the current state must be known.
                if agent.fast.is_none() {
                    return Err((
                        "fast_state_unknown",
                        "Claude's /fast toggles and its current state is not known until its first reply"
                            .into(),
                    ));
                }
                commands.push("/fast".into());
            }
            Ok(commands)
        }
        "codex" => {
            if !(model_changes || effort_changes || fast_changes) {
                return Ok(Vec::new());
            }
            let mut command = format!("/model {model_id}");
            if let Some(effort) = params.effort.as_ref().or(agent.effort.as_ref()) {
                command.push_str(&format!(" --reasoning={effort}"));
            }
            if let Some(fast) = params.fast {
                command.push_str(if fast {
                    " --speed=fast"
                } else {
                    " --speed=standard"
                });
            }
            Ok(vec![command])
        }
        _ => Err(("unsupported_agent", format!("cannot configure {kind}"))),
    }
}

fn send_commands(
    request_id: &str,
    pane_id: &str,
    commands: &[String],
    api_tx: &ApiRequestSender,
) -> Result<(), ConfigureError> {
    for (index, command) in commands.iter().enumerate() {
        if index > 0 {
            std::thread::sleep(COMMAND_GAP);
        }
        let response = dispatch_to_app_with_timeout(
            Request {
                id: format!("{request_id}:configure:{index}"),
                method: Method::AgentPrompt(crate::api::schema::AgentPromptParams {
                    target: pane_id.to_string(),
                    text: command.clone(),
                    wait: None,
                }),
            },
            api_tx,
            None,
        );
        let value: serde_json::Value = serde_json::from_str(&response).unwrap_or_default();
        if let Some(error) = value.get("error") {
            return Err((
                "configure_failed",
                format!(
                    "{command} was not sent: {}",
                    error
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown error")
                ),
            ));
        }
    }
    Ok(())
}

fn agent_get(
    request_id: &str,
    pane_id: &str,
    api_tx: &ApiRequestSender,
) -> Result<AgentInfo, ConfigureError> {
    app_call::agent_get(format!("{request_id}:agent"), pane_id, api_tx)
        .map_err(app_call::AppError::into_agent_lookup)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(
        kind: &str,
        model: Option<&str>,
        effort: Option<&str>,
        fast: Option<bool>,
    ) -> AgentInfo {
        let mut agent: AgentInfo = serde_json::from_value(serde_json::json!({
            "terminal_id": "t1", "agent": kind, "agent_status": "idle",
            "workspace_id": "w1", "tab_id": "w1:t1", "pane_id": "w1:p1",
            "focused": false, "revision": 0
        }))
        .unwrap();
        agent.model = model.map(str::to_string);
        agent.effort = effort.map(str::to_string);
        agent.fast = fast;
        agent
    }

    fn models() -> Vec<AgentModelInfo> {
        vec![AgentModelInfo {
            id: "m1".into(),
            name: "M1".into(),
            efforts: vec!["low".into(), "high".into()],
            default_effort: Some("low".into()),
            fast_available: true,
            fast_efforts: Vec::new(),
            variants: Vec::new(),
            default: false,
        }]
    }

    fn params(
        model: Option<&str>,
        effort: Option<&str>,
        fast: Option<bool>,
    ) -> AgentConfigureParams {
        AgentConfigureParams {
            pane_id: "w1:p1".into(),
            model: model.map(str::to_string),
            effort: effort.map(str::to_string),
            fast,
        }
    }

    #[test]
    fn claude_gets_one_command_per_changed_setting() {
        let current = agent("claude", Some("m0"), Some("low"), Some(false));
        let commands = plan_commands(
            "claude",
            &current,
            &params(Some("m1"), Some("high"), Some(true)),
            &models(),
        )
        .unwrap();
        assert_eq!(commands, ["/model m1", "/effort high", "/fast"]);

        let same = agent("claude", Some("m1"), Some("high"), Some(true));
        assert!(plan_commands(
            "claude",
            &same,
            &params(None, Some("high"), Some(true)),
            &models()
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn claude_fast_toggle_needs_a_known_state() {
        let current = agent("claude", Some("m1"), Some("low"), None);
        let error = plan_commands(
            "claude",
            &current,
            &params(None, None, Some(true)),
            &models(),
        )
        .unwrap_err();
        assert_eq!(error.0, "fast_state_unknown");
    }

    #[test]
    fn codex_applies_everything_in_one_model_command() {
        let current = agent("codex", Some("m1"), Some("low"), Some(false));
        assert_eq!(
            plan_commands(
                "codex",
                &current,
                &params(None, Some("high"), Some(true)),
                &models()
            )
            .unwrap(),
            ["/model m1 --reasoning=high --speed=fast"]
        );
        assert_eq!(
            plan_commands(
                "codex",
                &current,
                &params(None, None, Some(false)),
                &models()
            )
            .unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn invalid_settings_are_rejected_before_anything_is_typed() {
        let current = agent("codex", Some("m1"), None, None);
        assert_eq!(
            plan_commands(
                "codex",
                &current,
                &params(Some("nope"), None, None),
                &models()
            )
            .unwrap_err()
            .0,
            "unknown_model"
        );
        assert_eq!(
            plan_commands(
                "codex",
                &current,
                &params(None, Some("ultra"), None),
                &models()
            )
            .unwrap_err()
            .0,
            "invalid_effort"
        );
        let unknown = agent("codex", None, None, None);
        assert_eq!(
            plan_commands(
                "codex",
                &unknown,
                &params(None, Some("low"), None),
                &models()
            )
            .unwrap_err()
            .0,
            "model_unknown"
        );
    }
}
