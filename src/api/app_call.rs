//! Calls into the app from API threads (account, start, configure, question,
//! plugin and observer work) and decodes the reply in one place.

use serde::de::DeserializeOwned;

use crate::api::schema::{AgentInfo, AgentTarget, Method, Request};
use crate::api::server::{dispatch_to_app_with_timeout, APP_RESPONSE_TIMEOUT};
use crate::api::ApiRequestSender;

/// The app's error reply, with its own code.
#[derive(Debug)]
pub(super) struct AppError {
    pub(super) code: String,
    pub(super) message: String,
}

impl AppError {
    /// The error under the caller's own code, for methods that report one code.
    pub(super) fn with_code(self, code: &'static str) -> (&'static str, String) {
        (code, self.message)
    }

    /// An `agent_get` failure: a reply the app sent means the agent was not
    /// found, anything else is internal.
    pub(super) fn into_agent_lookup(self) -> (&'static str, String) {
        let code = if self.code == "internal_error" {
            "internal_error"
        } else {
            "agent_not_found"
        };
        (code, self.message)
    }
}

/// Sends `method` to the app and returns the whole reply, or its error.
pub(super) fn call(
    id: impl Into<String>,
    method: Method,
    api_tx: &ApiRequestSender,
) -> Result<serde_json::Value, AppError> {
    let response = dispatch_to_app_with_timeout(
        Request {
            id: id.into(),
            method,
        },
        api_tx,
        Some(APP_RESPONSE_TIMEOUT),
    );
    let value: serde_json::Value = serde_json::from_str(&response).map_err(|_| AppError {
        code: "internal_error".into(),
        message: "could not decode the app response".into(),
    })?;
    match value.get("error") {
        Some(error) => Err(AppError {
            code: error["code"]
                .as_str()
                .unwrap_or("internal_error")
                .to_string(),
            message: error["message"]
                .as_str()
                .unwrap_or("request failed")
                .to_string(),
        }),
        None => Ok(value),
    }
}

/// Moves the value at `pointer` out of a reply and decodes it.
pub(super) fn take<T: DeserializeOwned>(value: &mut serde_json::Value, pointer: &str) -> Option<T> {
    serde_json::from_value(value.pointer_mut(pointer)?.take()).ok()
}

pub(super) fn agent_get(
    id: impl Into<String>,
    pane_id: &str,
    api_tx: &ApiRequestSender,
) -> Result<AgentInfo, AppError> {
    let mut value = call(
        id,
        Method::AgentGet(AgentTarget {
            target: pane_id.to_string(),
        }),
        api_tx,
    )?;
    take(&mut value, "/result/agent").ok_or_else(|| AppError {
        code: "internal_error".into(),
        message: "could not decode the agent".into(),
    })
}

pub(super) fn agent_list(
    id: impl Into<String>,
    api_tx: &ApiRequestSender,
) -> Option<Vec<AgentInfo>> {
    let mut value = call(id, Method::AgentList(Default::default()), api_tx).ok()?;
    take(&mut value, "/result/agents")
}
