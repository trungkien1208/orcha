use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use regex::Regex;

use crate::api::schema::{
    ErrorBody, ErrorResponse, EventData, EventEnvelope, EventKind, EventMatch, EventsWaitParams,
    Method, Request, ResponseResult, Subscription, SubscriptionEventData,
    SubscriptionEventEnvelope, SuccessResponse,
};
use crate::api::server::{
    dispatch_to_app_with_caller_timeout, dispatch_to_app_with_timeout, should_stop_connection,
    ApiConnection, APP_RESPONSE_TIMEOUT, CONNECTION_POLL_INTERVAL,
};
use crate::api::subscriptions::ActiveSubscription;
use crate::api::subscriptions::{match_output, output_match_read_source};
use crate::api::{ApiRequestSender, EventHub};

const AGENT_PROMPT_EFFECT_TIMEOUT_MS: u64 = 5_000;

pub(super) fn wait_for_output(
    request_id: String,
    params: crate::api::schema::PaneWaitForOutputParams,
    stream: &mut dyn ApiConnection,
    api_tx: &ApiRequestSender,
    running: &Arc<AtomicBool>,
) -> std::io::Result<Option<String>> {
    crate::logging::api_wait_started(&request_id, &params.pane_id, params.timeout_ms);
    let deadline = params
        .timeout_ms
        .map(|ms| std::time::Instant::now() + std::time::Duration::from_millis(ms));

    let regex = match &params.r#match {
        crate::api::schema::OutputMatch::Regex { value } => match Regex::new(value) {
            Ok(regex) => Some(regex),
            Err(err) => {
                return Ok(Some(
                    serde_json::to_string(&ErrorResponse {
                        id: request_id,
                        error: ErrorBody {
                            code: "invalid_regex".into(),
                            message: err.to_string(),
                        },
                    })
                    .unwrap(),
                ));
            }
        },
        crate::api::schema::OutputMatch::Substring { .. } => None,
    };

    loop {
        if should_stop_connection(stream, running)? {
            crate::logging::api_wait_completed(&request_id, &params.pane_id, "client_disconnected");
            return Ok(None);
        }

        let read_request = Request {
            id: format!("{request_id}:read"),
            method: Method::PaneRead(crate::api::schema::PaneReadParams {
                pane_id: params.pane_id.clone(),
                source: output_match_read_source(&params.source),
                lines: params.lines,
                format: crate::api::schema::ReadFormat::Text,
                strip_ansi: params.strip_ansi,
                intent: crate::api::schema::ReadIntent::Passive,
            }),
        };
        let response =
            dispatch_to_app_with_timeout(read_request, api_tx, Some(APP_RESPONSE_TIMEOUT));
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&response) else {
            return Ok(Some(response));
        };
        if value.get("error").is_some() {
            let mut value = value;
            value["id"] = serde_json::Value::String(request_id.clone());
            return Ok(Some(serde_json::to_string(&value).unwrap()));
        }

        let read_value = value["result"]["read"].clone();
        let Ok(read) = serde_json::from_value::<crate::api::schema::PaneReadResult>(read_value)
        else {
            return Ok(Some(
                serde_json::to_string(&ErrorResponse {
                    id: request_id,
                    error: ErrorBody {
                        code: "internal_error".into(),
                        message: "failed to decode pane read result".into(),
                    },
                })
                .unwrap(),
            ));
        };

        let matched_line = match_output(&read.text, &params.r#match, regex.as_ref());
        if matched_line.is_some() {
            let revision = read.revision;
            crate::logging::api_wait_completed(&request_id, &params.pane_id, "matched");
            return Ok(Some(
                serde_json::to_string(&SuccessResponse {
                    id: request_id,
                    result: ResponseResult::OutputMatched {
                        pane_id: read.pane_id.clone(),
                        revision,
                        matched_line,
                        read,
                    },
                })
                .unwrap(),
            ));
        }

        if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
            crate::logging::api_wait_timed_out(&request_id, &params.pane_id);
            return Ok(Some(
                serde_json::to_string(&ErrorResponse {
                    id: request_id,
                    error: ErrorBody {
                        code: "timeout".into(),
                        message: "timed out waiting for output match".into(),
                    },
                })
                .unwrap(),
            ));
        }

        std::thread::sleep(CONNECTION_POLL_INTERVAL);
    }
}

pub(super) fn wait_for_agent(
    request_id: String,
    params: crate::api::schema::AgentWaitParams,
    stream: &mut dyn ApiConnection,
    api_tx: &ApiRequestSender,
    event_hub: &EventHub,
    running: &Arc<AtomicBool>,
) -> std::io::Result<Option<String>> {
    let last_event_sequence = event_hub.current_sequence();
    let initial = match agent_get(&request_id, &params.target, api_tx) {
        Ok(agent) => agent,
        Err(response) => {
            return serde_json::to_string(&response)
                .map(Some)
                .map_err(std::io::Error::other);
        }
    };
    let until = agent_wait_statuses(params.until);
    if agent_wait_matches(&initial, &until, None) {
        return agent_wait_success(request_id, initial).map(Some);
    }

    match wait_for_resolved_agent(
        request_id.clone(),
        ResolvedAgentWait {
            target: params.target,
            until,
            timeout_ms: params.timeout_ms,
            initial,
            last_event_sequence,
            after_state_change_seq: None,
            accept_transient_status: true,
            timeout_kind: AgentWaitTimeoutKind::Status,
        },
        stream,
        api_tx,
        event_hub,
        running,
    )? {
        Some(AgentWaitOutcome::Matched(agent)) => agent_wait_success(request_id, *agent).map(Some),
        Some(AgentWaitOutcome::Response(response)) => Ok(Some(response)),
        None => Ok(None),
    }
}

pub(super) fn prompt_agent(
    request_id: String,
    mut params: crate::api::schema::AgentPromptParams,
    stream: &mut dyn ApiConnection,
    api_tx: &ApiRequestSender,
    event_hub: &EventHub,
    running: &Arc<AtomicBool>,
) -> std::io::Result<Option<String>> {
    let Some(wait) = params.wait.clone() else {
        return prompt_agent_and_confirm(request_id, params, stream, api_tx, running);
    };

    let wait_started = std::time::Instant::now();
    let before_prompt = match agent_get_for_prompt(
        &request_id,
        &params.target,
        api_tx,
        wait.timeout_ms,
        wait_started,
    ) {
        Ok(agent) => agent,
        Err(response) => {
            return serde_json::to_string(&response)
                .map(Some)
                .map_err(std::io::Error::other);
        }
    };
    let prompt_started_working =
        before_prompt.agent_status == crate::api::schema::AgentStatus::Working;
    let target = params.target.clone();
    if let Some(prompt_wait) = params.wait.as_mut() {
        prompt_wait.submission_deadline = wait
            .timeout_ms
            .map(|timeout_ms| wait_started + std::time::Duration::from_millis(timeout_ms));
    }
    let last_event_sequence = event_hub.current_sequence();
    let prompt_request = Request {
        id: request_id.clone(),
        method: Method::AgentPrompt(params),
    };
    #[cfg(windows)]
    let prompt_response = dispatch_to_app_with_caller_timeout(
        prompt_request,
        api_tx,
        remaining_timeout_ms(wait.timeout_ms, wait_started).map(std::time::Duration::from_millis),
    );
    #[cfg(not(windows))]
    let prompt_response = dispatch_to_app_with_timeout(prompt_request, api_tx, None);
    let Ok(prompted) = agent_from_response(&request_id, &prompt_response) else {
        return Ok(Some(prompt_response));
    };
    if !agent_wait_identity_matches(
        &prompted,
        &before_prompt.terminal_id,
        before_prompt.name.as_deref().filter(|name| *name == target),
        before_prompt.agent.as_deref(),
    ) {
        return agent_wait_not_running(request_id).map(Some);
    }

    let prompt_activity_observed = prompt_started_working
        || matches!(
            prompted.agent_status,
            crate::api::schema::AgentStatus::Working | crate::api::schema::AgentStatus::Blocked
        );
    let prompt_state_change_seq = prompted.state_change_seq;
    let until = agent_wait_statuses(wait.until);
    let mut initial = prompted;

    if !prompt_activity_observed {
        let remaining_timeout_ms = remaining_timeout_ms(wait.timeout_ms, wait_started);
        let (effect_timeout_ms, timeout_kind) = match remaining_timeout_ms {
            Some(timeout_ms) if timeout_ms <= AGENT_PROMPT_EFFECT_TIMEOUT_MS => {
                (timeout_ms, AgentWaitTimeoutKind::Status)
            }
            _ => (
                AGENT_PROMPT_EFFECT_TIMEOUT_MS,
                AgentWaitTimeoutKind::PromptStalled {
                    timeout_ms: AGENT_PROMPT_EFFECT_TIMEOUT_MS,
                },
            ),
        };
        let Some(outcome) = wait_for_resolved_agent(
            request_id.clone(),
            ResolvedAgentWait {
                target: target.clone(),
                until: prompt_activity_statuses(),
                timeout_ms: Some(effect_timeout_ms),
                initial,
                last_event_sequence,
                after_state_change_seq: Some(prompt_state_change_seq),
                accept_transient_status: true,
                timeout_kind,
            },
            stream,
            api_tx,
            event_hub,
            running,
        )?
        else {
            return Ok(None);
        };
        initial = match outcome {
            AgentWaitOutcome::Matched(agent) => *agent,
            AgentWaitOutcome::Response(response) => return Ok(Some(response)),
        };
    }
    if agent_wait_matches(&initial, &until, None) {
        return agent_prompt_success(request_id, initial).map(Some);
    }

    let Some(outcome) = wait_for_resolved_agent(
        request_id.clone(),
        ResolvedAgentWait {
            target,
            until,
            timeout_ms: remaining_timeout_ms(wait.timeout_ms, wait_started),
            initial,
            // Replay from before submission so terminal lifecycle events consumed by
            // the activity gate still terminate this settled-state wait.
            last_event_sequence,
            after_state_change_seq: None,
            accept_transient_status: false,
            timeout_kind: AgentWaitTimeoutKind::Status,
        },
        stream,
        api_tx,
        event_hub,
        running,
    )?
    else {
        return Ok(None);
    };
    let agent = match outcome {
        AgentWaitOutcome::Matched(agent) => *agent,
        AgentWaitOutcome::Response(response) => return Ok(Some(response)),
    };
    agent_prompt_success(request_id, agent).map(Some)
}

fn remaining_timeout_ms(total_ms: Option<u64>, started: std::time::Instant) -> Option<u64> {
    total_ms.map(|total_ms| {
        let elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        total_ms.saturating_sub(elapsed_ms)
    })
}

fn agent_prompt_success(
    request_id: String,
    agent: crate::api::schema::AgentInfo,
) -> std::io::Result<String> {
    serde_json::to_string(&SuccessResponse {
        id: request_id,
        result: ResponseResult::AgentPrompted { agent },
    })
    .map_err(std::io::Error::other)
}

struct ResolvedAgentWait {
    target: String,
    until: Vec<crate::api::schema::AgentStatus>,
    timeout_ms: Option<u64>,
    initial: crate::api::schema::AgentInfo,
    last_event_sequence: u64,
    after_state_change_seq: Option<u64>,
    accept_transient_status: bool,
    timeout_kind: AgentWaitTimeoutKind,
}

#[derive(Clone, Copy)]
enum AgentWaitTimeoutKind {
    Status,
    PromptStalled { timeout_ms: u64 },
}

enum AgentWaitOutcome {
    Matched(Box<crate::api::schema::AgentInfo>),
    Response(String),
}

fn wait_for_resolved_agent(
    request_id: String,
    wait: ResolvedAgentWait,
    stream: &mut dyn ApiConnection,
    api_tx: &ApiRequestSender,
    event_hub: &EventHub,
    running: &Arc<AtomicBool>,
) -> std::io::Result<Option<AgentWaitOutcome>> {
    let deadline = wait
        .timeout_ms
        .map(|ms| std::time::Instant::now() + std::time::Duration::from_millis(ms));
    let expected_terminal_id = wait.initial.terminal_id.clone();
    let expected_name = wait
        .initial
        .name
        .as_ref()
        .filter(|name| name.as_str() == wait.target)
        .cloned();
    let expected_agent = wait.initial.agent.clone();
    let pane_id = wait.initial.pane_id.clone();
    let mut last_event_sequence = wait.last_event_sequence;

    loop {
        if should_stop_connection(stream, running)? {
            return Ok(None);
        }

        let mut should_probe = false;
        let mut matched_event_status = None;
        for (sequence, event) in event_hub.events_after(last_event_sequence) {
            last_event_sequence = sequence;
            match event.data {
                EventData::PaneAgentDetected {
                    pane_id: event_pane,
                    agent,
                    released,
                    final_status,
                    ..
                } if event_pane == pane_id => {
                    if released {
                        if let Some(status) = final_status
                            .filter(|status| wait.until.contains(status))
                            .or(matched_event_status)
                        {
                            let mut matched = wait.initial.clone();
                            matched.agent_status = status;
                            return Ok(Some(AgentWaitOutcome::Matched(Box::new(matched))));
                        }
                        return agent_wait_not_running(request_id)
                            .map(AgentWaitOutcome::Response)
                            .map(Some);
                    }
                    if agent.is_some() && expected_agent.is_some() && agent != expected_agent {
                        return agent_wait_not_running(request_id)
                            .map(AgentWaitOutcome::Response)
                            .map(Some);
                    }
                    should_probe = true;
                }
                EventData::PaneAgentStatusChanged {
                    pane_id: event_pane,
                    agent_status,
                    ..
                } if event_pane == pane_id => {
                    if wait.accept_transient_status && wait.until.contains(&agent_status) {
                        matched_event_status = Some(agent_status);
                    }
                    should_probe = true;
                }
                EventData::PaneUpdated { pane } if pane.pane_id == pane_id => should_probe = true,
                EventData::PaneMoved {
                    previous_pane_id, ..
                } if previous_pane_id == pane_id => {
                    return agent_wait_not_running(request_id)
                        .map(AgentWaitOutcome::Response)
                        .map(Some);
                }
                EventData::PaneClosed {
                    pane_id: event_pane,
                    ..
                }
                | EventData::PaneExited {
                    pane_id: event_pane,
                    ..
                } if event_pane == pane_id => {
                    return agent_wait_not_running(request_id)
                        .map(AgentWaitOutcome::Response)
                        .map(Some);
                }
                _ => {}
            }
        }

        if should_probe {
            let current = match agent_get(&request_id, &wait.target, api_tx) {
                Ok(agent) => agent,
                Err(response) => {
                    return agent_wait_probe_error(response)
                        .map(AgentWaitOutcome::Response)
                        .map(Some);
                }
            };
            if !agent_wait_identity_matches(
                &current,
                &expected_terminal_id,
                expected_name.as_deref(),
                expected_agent.as_deref(),
            ) {
                return agent_wait_not_running(request_id)
                    .map(AgentWaitOutcome::Response)
                    .map(Some);
            }
            if let Some(status) = matched_event_status {
                let mut matched = current;
                matched.agent_status = status;
                return Ok(Some(AgentWaitOutcome::Matched(Box::new(matched))));
            }
            if agent_wait_matches(&current, &wait.until, wait.after_state_change_seq) {
                return Ok(Some(AgentWaitOutcome::Matched(Box::new(current))));
            }
        }

        if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
            let current = match agent_get(&request_id, &wait.target, api_tx) {
                Ok(agent) => agent,
                Err(response) => {
                    return agent_wait_probe_error(response)
                        .map(AgentWaitOutcome::Response)
                        .map(Some);
                }
            };
            if !agent_wait_identity_matches(
                &current,
                &expected_terminal_id,
                expected_name.as_deref(),
                expected_agent.as_deref(),
            ) {
                return agent_wait_not_running(request_id)
                    .map(AgentWaitOutcome::Response)
                    .map(Some);
            }
            if agent_wait_matches(&current, &wait.until, wait.after_state_change_seq) {
                return Ok(Some(AgentWaitOutcome::Matched(Box::new(current))));
            }
            return agent_wait_timeout(request_id, wait.timeout_kind, &current)
                .map(AgentWaitOutcome::Response)
                .map(Some);
        }
        std::thread::sleep(CONNECTION_POLL_INTERVAL);
    }
}

fn prompt_activity_statuses() -> Vec<crate::api::schema::AgentStatus> {
    vec![
        crate::api::schema::AgentStatus::Working,
        crate::api::schema::AgentStatus::Blocked,
    ]
}

fn agent_wait_statuses(
    until: Vec<crate::api::schema::AgentStatus>,
) -> Vec<crate::api::schema::AgentStatus> {
    if until.is_empty() {
        vec![
            crate::api::schema::AgentStatus::Idle,
            crate::api::schema::AgentStatus::Done,
            crate::api::schema::AgentStatus::Blocked,
        ]
    } else {
        until
    }
}

fn agent_wait_identity_matches(
    agent: &crate::api::schema::AgentInfo,
    expected_terminal_id: &str,
    expected_name: Option<&str>,
    expected_agent: Option<&str>,
) -> bool {
    agent.terminal_id == expected_terminal_id
        && expected_name.is_none_or(|name| agent.name.as_deref() == Some(name))
        && match (expected_agent, agent.agent.as_deref()) {
            (Some(expected), Some(current)) => expected == current,
            (Some(_), None) => agent.name.is_some(),
            (None, _) => true,
        }
}

fn agent_wait_matches(
    agent: &crate::api::schema::AgentInfo,
    until: &[crate::api::schema::AgentStatus],
    after_state_change_seq: Option<u64>,
) -> bool {
    until.contains(&agent.agent_status)
        && after_state_change_seq.is_none_or(|baseline| agent.state_change_seq > baseline)
}

/// How long a prompt may take to leave the composer before Enter is sent again.
#[cfg(not(test))]
const AGENT_PROMPT_SUBMIT_CONFIRM: std::time::Duration = std::time::Duration::from_secs(3);
#[cfg(test)]
const AGENT_PROMPT_SUBMIT_CONFIRM: std::time::Duration = std::time::Duration::from_millis(400);
/// How long `agent.prompt` waits for a just-answered question to clear.
const AGENT_PROMPT_BLOCKED_GRACE: std::time::Duration = std::time::Duration::from_secs(3);
/// How long `agent.prompt` waits for a starting agent to accept input.
const AGENT_PROMPT_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// `agent.prompt` without `wait`: waits for a starting agent to be ready,
/// submits, and succeeds only once the agent reacts. If the text stayed in the
/// composer (an agent still mounting its input after a trust prompt drops the
/// first Enter), Enter is sent once more; if that also has no effect the call
/// fails with `agent_prompt_not_submitted` instead of reporting success.
pub(super) fn prompt_agent_and_confirm(
    request_id: String,
    params: crate::api::schema::AgentPromptParams,
    stream: &mut dyn ApiConnection,
    api_tx: &ApiRequestSender,
    running: &Arc<AtomicBool>,
) -> std::io::Result<Option<String>> {
    let target = params.target.clone();
    let encode = |response: &ErrorResponse| {
        serde_json::to_string(response)
            .map(Some)
            .map_err(std::io::Error::other)
    };
    let started = std::time::Instant::now();
    let ready_deadline = started + AGENT_PROMPT_READY_TIMEOUT;
    let before = loop {
        let agent = match agent_get(&request_id, &target, api_tx) {
            Ok(agent) => agent,
            Err(response) => return encode(&response),
        };
        let now = std::time::Instant::now();
        // A question that was just answered can still read as blocked, and an
        // agent loading after it reads as unknown; give both a moment.
        let settling = match agent.agent_status {
            crate::api::schema::AgentStatus::Blocked => now < started + AGENT_PROMPT_BLOCKED_GRACE,
            crate::api::schema::AgentStatus::Unknown => {
                agent.agent.is_some() && now < ready_deadline
            }
            _ => false,
        };
        if !agent.launch_pending && !settling {
            break agent;
        }
        if !agent.launch_pending {
            if should_stop_connection(stream, running)? {
                return Ok(None);
            }
            std::thread::sleep(CONNECTION_POLL_INTERVAL);
            continue;
        }
        if now >= ready_deadline {
            let message = if agent.agent_status == crate::api::schema::AgentStatus::Blocked {
                format!(
                    "agent {target} is waiting on a question; see agent.question and agent.answer"
                )
            } else {
                format!("agent {target} did not become ready for input")
            };
            return encode(&ErrorResponse {
                id: request_id,
                error: ErrorBody {
                    code: "agent_not_ready".into(),
                    message,
                },
            });
        }
        if should_stop_connection(stream, running)? {
            return Ok(None);
        }
        std::thread::sleep(CONNECTION_POLL_INTERVAL);
    };

    let response = dispatch_to_app_with_timeout(
        Request {
            id: request_id.clone(),
            method: Method::AgentPrompt(params),
        },
        api_tx,
        None,
    );
    let Ok(prompted) = agent_from_response(&request_id, &response) else {
        return Ok(Some(response));
    };
    // Without status detection there is nothing to confirm against, and a
    // working agent queues the prompt without a visible change.
    if matches!(
        before.agent_status,
        crate::api::schema::AgentStatus::Unknown | crate::api::schema::AgentStatus::Working
    ) {
        return Ok(Some(response));
    }
    let reacted = |agent: &crate::api::schema::AgentInfo| {
        agent.state_change_seq > before.state_change_seq
            || matches!(
                agent.agent_status,
                crate::api::schema::AgentStatus::Working | crate::api::schema::AgentStatus::Blocked
            ) && before.agent_status != agent.agent_status
    };
    if reacted(&prompted) {
        return Ok(Some(response));
    }
    for attempt in 0..2 {
        if attempt == 1 {
            let _ = dispatch_to_app_with_timeout(
                Request {
                    id: format!("{request_id}:enter"),
                    method: Method::AgentSendKeys(crate::api::schema::AgentSendKeysParams {
                        target: target.clone(),
                        keys: vec!["enter".into()],
                    }),
                },
                api_tx,
                Some(APP_RESPONSE_TIMEOUT),
            );
        }
        let deadline = std::time::Instant::now() + AGENT_PROMPT_SUBMIT_CONFIRM;
        while std::time::Instant::now() < deadline {
            if should_stop_connection(stream, running)? {
                return Ok(None);
            }
            std::thread::sleep(CONNECTION_POLL_INTERVAL);
            match agent_get(&request_id, &target, api_tx) {
                Ok(agent) if reacted(&agent) => {
                    return serde_json::to_string(&SuccessResponse {
                        id: request_id,
                        result: ResponseResult::AgentPrompted { agent },
                    })
                    .map(Some)
                    .map_err(std::io::Error::other);
                }
                Ok(_) => {}
                Err(response) => return encode(&response),
            }
        }
    }
    encode(&ErrorResponse {
        id: request_id,
        error: ErrorBody {
            code: "agent_prompt_not_submitted".into(),
            message: format!(
                "agent {target} did not react to the prompt; the text may still be in its input"
            ),
        },
    })
}

fn agent_get(
    request_id: &str,
    target: &str,
    api_tx: &ApiRequestSender,
) -> Result<crate::api::schema::AgentInfo, ErrorResponse> {
    let response = dispatch_to_app_with_timeout(
        Request {
            id: format!("{request_id}:agent"),
            method: Method::AgentGet(crate::api::schema::AgentTarget {
                target: target.to_string(),
            }),
        },
        api_tx,
        Some(APP_RESPONSE_TIMEOUT),
    );
    agent_from_response(request_id, &response)
}

fn agent_get_for_prompt(
    request_id: &str,
    target: &str,
    api_tx: &ApiRequestSender,
    total_timeout_ms: Option<u64>,
    started: std::time::Instant,
) -> Result<crate::api::schema::AgentInfo, ErrorResponse> {
    let request = Request {
        id: format!("{request_id}:agent"),
        method: Method::AgentGet(crate::api::schema::AgentTarget {
            target: target.to_string(),
        }),
    };
    let remaining_ms = remaining_timeout_ms(total_timeout_ms, started);
    let response = match remaining_ms {
        Some(timeout_ms) if timeout_ms <= APP_RESPONSE_TIMEOUT.as_millis() as u64 => {
            dispatch_to_app_with_caller_timeout(
                request,
                api_tx,
                Some(std::time::Duration::from_millis(timeout_ms)),
            )
        }
        _ => dispatch_to_app_with_timeout(request, api_tx, Some(APP_RESPONSE_TIMEOUT)),
    };
    agent_from_response(request_id, &response)
}

fn agent_from_response(
    request_id: &str,
    response: &str,
) -> Result<crate::api::schema::AgentInfo, ErrorResponse> {
    let value: serde_json::Value = serde_json::from_str(response).map_err(|_| ErrorResponse {
        id: request_id.into(),
        error: ErrorBody {
            code: "internal_error".into(),
            message: "failed to decode agent response".into(),
        },
    })?;
    if value.get("error").is_some() {
        let error = serde_json::from_value(value["error"].clone()).map_err(|_| ErrorResponse {
            id: request_id.into(),
            error: ErrorBody {
                code: "internal_error".into(),
                message: "failed to decode agent error".into(),
            },
        })?;
        return Err(ErrorResponse {
            id: request_id.into(),
            error,
        });
    }
    serde_json::from_value(value["result"]["agent"].clone()).map_err(|_| ErrorResponse {
        id: request_id.into(),
        error: ErrorBody {
            code: "internal_error".into(),
            message: "failed to decode agent result".into(),
        },
    })
}

fn agent_wait_success(
    request_id: String,
    agent: crate::api::schema::AgentInfo,
) -> std::io::Result<String> {
    serde_json::to_string(&SuccessResponse {
        id: request_id,
        result: ResponseResult::AgentInfo { agent },
    })
    .map_err(std::io::Error::other)
}

fn agent_wait_timeout(
    request_id: String,
    kind: AgentWaitTimeoutKind,
    current: &crate::api::schema::AgentInfo,
) -> std::io::Result<String> {
    let (code, message) = match kind {
        AgentWaitTimeoutKind::Status => {
            ("timeout", "timed out waiting for agent status".to_string())
        }
        AgentWaitTimeoutKind::PromptStalled { timeout_ms } => {
            let status = format!("{:?}", current.agent_status).to_ascii_lowercase();
            (
                "agent_prompt_stalled",
                format!(
                    "agent prompt produced no observed working or blocked state within {timeout_ms} ms; current status is {status}"
                ),
            )
        }
    };
    serde_json::to_string(&ErrorResponse {
        id: request_id,
        error: ErrorBody {
            code: code.into(),
            message,
        },
    })
    .map_err(std::io::Error::other)
}

fn agent_wait_not_running(request_id: String) -> std::io::Result<String> {
    serde_json::to_string(&ErrorResponse {
        id: request_id,
        error: ErrorBody {
            code: "agent_not_running".into(),
            message: "agent is no longer running in the target pane".into(),
        },
    })
    .map_err(std::io::Error::other)
}

fn agent_wait_probe_error(response: ErrorResponse) -> std::io::Result<String> {
    if response.error.code == "agent_not_found" {
        return agent_wait_not_running(response.id);
    }
    serde_json::to_string(&response).map_err(std::io::Error::other)
}

pub(super) fn wait_for_event(
    request_id: String,
    params: EventsWaitParams,
    stream: &mut dyn ApiConnection,
    api_tx: &ApiRequestSender,
    event_hub: &EventHub,
    running: &Arc<AtomicBool>,
) -> std::io::Result<Option<String>> {
    let deadline = params
        .timeout_ms
        .map(|ms| std::time::Instant::now() + std::time::Duration::from_millis(ms));

    let subscription = match event_match_subscription(&request_id, params.match_event) {
        Ok(subscription) => subscription,
        Err(response) => return Ok(Some(serde_json::to_string(&response).unwrap())),
    };
    let mut active = match ActiveSubscription::new(
        subscription,
        &request_id,
        0,
        api_tx,
        event_hub,
        event_hub.current_sequence(),
    ) {
        Ok(active) => active,
        Err(response) => return Ok(Some(serde_json::to_string(&response).unwrap())),
    };

    loop {
        if should_stop_connection(stream, running)? {
            return Ok(None);
        }

        match active.poll_for_wait(api_tx, event_hub) {
            Ok(Some(event)) => return Ok(Some(wait_matched_response(&request_id, event))),
            Ok(None) => {}
            Err(mut response) if response.error.code == "pane_not_found" => {
                response.id = request_id;
                return serde_json::to_string(&response)
                    .map(Some)
                    .map_err(std::io::Error::other);
            }
            Err(_) => {}
        }

        if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
            return Ok(Some(
                serde_json::to_string(&ErrorResponse {
                    id: request_id,
                    error: ErrorBody {
                        code: "timeout".into(),
                        message: "timed out waiting for event match".into(),
                    },
                })
                .unwrap(),
            ));
        }

        std::thread::sleep(CONNECTION_POLL_INTERVAL);
    }
}

fn event_match_subscription(
    request_id: &str,
    match_event: EventMatch,
) -> Result<Subscription, ErrorResponse> {
    match match_event {
        EventMatch::PaneAgentStatusChanged {
            pane_id,
            agent_status,
        } => Ok(Subscription::PaneAgentStatusChanged {
            pane_id: Some(pane_id),
            workspace_id: None,
            pane_ids: Vec::new(),
            agent_status: Some(agent_status),
        }),
        _ => Err(ErrorResponse {
            id: request_id.into(),
            error: ErrorBody {
                code: "unsupported_event_wait_match".into(),
                message: "events.wait currently supports pane agent status matches".into(),
            },
        }),
    }
}

fn wait_matched_response(request_id: &str, event: serde_json::Value) -> String {
    let Ok(event) = serde_json::from_value::<SubscriptionEventEnvelope>(event) else {
        return serde_json::to_string(&ErrorResponse {
            id: request_id.into(),
            error: ErrorBody {
                code: "internal_error".into(),
                message: "failed to decode matched event".into(),
            },
        })
        .unwrap();
    };

    let SubscriptionEventData::PaneAgentStatusChanged(data) = event.data else {
        return serde_json::to_string(&ErrorResponse {
            id: request_id.into(),
            error: ErrorBody {
                code: "unsupported_event_wait_match".into(),
                message: "events.wait currently supports pane agent status matches".into(),
            },
        })
        .unwrap();
    };

    serde_json::to_string(&SuccessResponse {
        id: request_id.into(),
        result: ResponseResult::WaitMatched {
            event: EventEnvelope {
                event: EventKind::PaneAgentStatusChanged,
                data: EventData::PaneAgentStatusChanged {
                    pane_id: data.pane_id,
                    workspace_id: data.workspace_id,
                    agent_status: data.agent_status,
                    agent: data.agent,
                    title: data.title,
                    display_agent: data.display_agent,
                    state_labels: data.state_labels,
                    state_change_seq: data.state_change_seq,
                    session_version: data.session_version,
                    stage: data.stage,
                    tool: data.tool,
                },
            },
        },
    })
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_wait_probe_only_translates_agent_disappearance() {
        let disappeared = agent_wait_probe_error(ErrorResponse {
            id: "wait".into(),
            error: ErrorBody {
                code: "agent_not_found".into(),
                message: "missing".into(),
            },
        })
        .unwrap();
        let disappeared: ErrorResponse = serde_json::from_str(&disappeared).unwrap();
        assert_eq!(disappeared.id, "wait");
        assert_eq!(disappeared.error.code, "agent_not_running");

        let unavailable = agent_wait_probe_error(ErrorResponse {
            id: "wait".into(),
            error: ErrorBody {
                code: "server_unavailable".into(),
                message: "timed out waiting for app response".into(),
            },
        })
        .unwrap();
        let unavailable: ErrorResponse = serde_json::from_str(&unavailable).unwrap();
        assert_eq!(unavailable.id, "wait");
        assert_eq!(unavailable.error.code, "server_unavailable");
    }
}

#[cfg(test)]
mod prompt_confirm_tests {
    use super::*;

    fn agent(status: crate::api::schema::AgentStatus, seq: u64) -> crate::api::schema::AgentInfo {
        crate::api::schema::AgentInfo {
            terminal_id: "t1".into(),
            name: None,
            agent: Some("claude".into()),
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            display_agent: None,
            agent_status: status,
            screen_detection_skipped: false,
            state_labels: Default::default(),
            tokens: Default::default(),
            agent_session: None,
            workspace_id: "w1".into(),
            tab_id: "w1:t1".into(),
            pane_id: "w1:p1".into(),
            focused: false,
            launch_pending: false,
            interactive_ready: true,
            state_change_seq: seq,
            completion_seq: None,
            seen_seq: None,
            stage: None,
            tool: None,
            cwd: None,
            foreground_cwd: None,
            revision: 0,
            model: None,
            effort: None,
            fast: None,
        }
    }

    /// Answers like an agent whose first Enter is lost: it reacts only after
    /// `reacts_after_enters` extra Enter keys. Returns how many Enters it saw.
    fn spawn_agent(
        reacts_after_enters: usize,
    ) -> (ApiRequestSender, std::thread::JoinHandle<usize>) {
        use crate::api::schema::{AgentStatus, ResponseResult, SuccessResponse};
        let (api_tx, mut api_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::api::ApiRequestMessage>();
        let thread = std::thread::spawn(move || {
            let mut enters = 0;
            while let Some(message) = api_rx.blocking_recv() {
                let reacted = enters >= reacts_after_enters && reacts_after_enters > 0;
                let current = if reacted {
                    agent(AgentStatus::Working, 6)
                } else {
                    agent(AgentStatus::Idle, 5)
                };
                let result = match message.request.method {
                    Method::AgentGet(_) => ResponseResult::AgentInfo { agent: current },
                    Method::AgentPrompt(_) => ResponseResult::AgentPrompted { agent: current },
                    Method::AgentSendKeys(_) => {
                        enters += 1;
                        ResponseResult::Ok {}
                    }
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
            enters
        });
        (api_tx, thread)
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

    fn prompt(api_tx: &ApiRequestSender) -> serde_json::Value {
        let response = prompt_agent_and_confirm(
            "p".into(),
            crate::api::schema::AgentPromptParams {
                target: "w1:p1".into(),
                text: "hello".into(),
                wait: None,
            },
            &mut OpenConnection,
            api_tx,
            &Arc::new(AtomicBool::new(true)),
        )
        .unwrap()
        .unwrap();
        serde_json::from_str(&response).unwrap()
    }

    #[test]
    fn prompt_that_stays_in_the_composer_gets_one_more_enter() {
        let (api_tx, agent) = spawn_agent(1);
        let response = prompt(&api_tx);
        assert_eq!(response["result"]["type"], "agent_prompted");
        assert_eq!(response["result"]["agent"]["agent_status"], "working");
        drop(api_tx);
        assert_eq!(agent.join().unwrap(), 1);
    }

    #[test]
    fn prompt_without_any_reaction_fails_instead_of_reporting_success() {
        let (api_tx, agent) = spawn_agent(0);
        let response = prompt(&api_tx);
        assert_eq!(response["error"]["code"], "agent_prompt_not_submitted");
        drop(api_tx);
        assert_eq!(agent.join().unwrap(), 1, "Enter is retried only once");
    }
}
