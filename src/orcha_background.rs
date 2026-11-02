//! What keeps running after the orcha window closes: the server, its
//! agents, plugin servers and Easy Job. `orcha status` prints it and the
//! settings page shows it with a Stop for each.

use std::time::Duration;

use serde_json::{json, Value};

use crate::api::client::{ApiClient, ConnectionTarget};
use crate::api::schema::{EmptyParams, Method, PingParams, Request};

/// A stalled server must not hold the settings page's poll for long: the
/// page's one server answers every request in turn.
const CALL_TIMEOUT: Duration = Duration::from_millis(1500);

/// One socket method's `result`, asked over this session's API socket. The
/// settings page polls `snapshot` every few seconds, so this talks to the
/// socket directly instead of starting an `orcha api call` process per method.
fn call(name: &str, method: Method) -> Option<Value> {
    let client = ApiClient::for_target(ConnectionTarget::SocketPath(
        crate::session::active_api_socket_path(),
    ));
    let request = Request {
        id: format!("orcha:background:{name}"),
        method,
    };
    let mut response = client
        .request_value_with_timeout(&request, CALL_TIMEOUT)
        .ok()?;
    Some(response.get_mut("result")?.take())
}

/// Everything running in the background, as the settings page reads it.
/// Blocking (a few API calls, asked at once, so CALL_TIMEOUT at most):
/// call it off the input path.
pub(crate) fn snapshot() -> Value {
    let (server, agents, listed, office) = std::thread::scope(|scope| {
        let server = scope.spawn(|| call("ping", Method::Ping(PingParams::default())));
        let agents = scope.spawn(|| call("agent.list", Method::AgentList(EmptyParams::default())));
        let listed = scope.spawn(|| {
            call(
                "plugin.pane.list",
                Method::PluginPaneList(EmptyParams::default()),
            )
        });
        let office = crate::orcha_office::summary();
        // a call that panicked reads as no answer
        (
            server.join().ok().flatten(),
            agents.join().ok().flatten(),
            listed.join().ok().flatten(),
            office,
        )
    });
    let agents: Vec<Value> = agents
        .and_then(|result| result.get("agents").cloned())
        .and_then(|agents| agents.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|agent| {
            let cwd = agent["cwd"].as_str().unwrap_or_default();
            json!({
                "pane_id": agent["pane_id"],
                "agent": agent["agent"],
                "status": agent["agent_status"],
                "project": std::path::Path::new(cwd)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                "cwd": cwd,
            })
        })
        .collect();
    let plugin_panes: Vec<Value> = listed
        .and_then(|result| result.get("plugin_panes").cloned())
        .and_then(|panes| panes.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|pane| {
            json!({
                "pane_id": pane["pane_id"],
                "plugin_id": pane["plugin_id"],
                "entrypoint": pane["entrypoint"],
                "url": pane["url"],
            })
        })
        .collect();
    // Easy Job running as a plugin pane is listed with the panes already
    let office_in_pane = plugin_panes
        .iter()
        .any(|pane| pane["plugin_id"] == crate::orcha_office::PLUGIN_ID);
    let office = office.filter(|_| !office_in_pane);
    json!({
        "server": {
            "running": server.is_some(),
            "version": server.as_ref().map(|pong| pong["version"].clone()),
            "socket": crate::session::active_api_socket_path().display().to_string(),
        },
        "agents": agents,
        "plugin_panes": plugin_panes,
        "office": office.map(|(running, port)| json!({"running": running, "port": port})),
    })
}

/// Counts agents by state, in the order a person cares about.
pub(crate) fn agent_counts(agents: &[Value]) -> Vec<(&'static str, usize)> {
    let count = |states: &[&str]| {
        agents
            .iter()
            .filter(|agent| states.contains(&agent["status"].as_str().unwrap_or_default()))
            .count()
    };
    [
        ("need you", count(&["blocked"])),
        ("working", count(&["working"])),
        ("done", count(&["done"])),
        ("idle", count(&["idle", "unknown"])),
    ]
    .into_iter()
    .filter(|(_, n)| *n > 0)
    .collect()
}

/// The `background:` block of `orcha status`.
pub(crate) fn print() {
    let snapshot = snapshot();
    println!();
    println!("background (keeps running when you quit orcha):");
    if snapshot["server"]["running"] != true {
        println!("  server: not running");
        return;
    }
    println!("  server: running (orcha server stop ends it and every agent)");
    let agents = snapshot["agents"].as_array().cloned().unwrap_or_default();
    let counts = agent_counts(&agents)
        .into_iter()
        .map(|(state, n)| format!("{n} {state}"))
        .collect::<Vec<_>>()
        .join(", ");
    if agents.is_empty() {
        println!("  agents: none");
    } else {
        println!("  agents: {counts}");
    }
    for pane in snapshot["plugin_panes"].as_array().into_iter().flatten() {
        let plugin = pane["plugin_id"].as_str().unwrap_or_default();
        let pane_id = pane["pane_id"].as_str().unwrap_or_default();
        match pane["url"].as_str() {
            Some(url) => println!("  {plugin}: running in {pane_id}, {}", without_token(url)),
            None => println!("  {plugin}: running in {pane_id}"),
        }
    }
    match snapshot["office"]["running"].as_bool() {
        Some(true) => match snapshot["office"]["port"].as_u64() {
            Some(port) => println!("  Easy Job: running on http://127.0.0.1:{port}"),
            None => println!("  Easy Job: running"),
        },
        Some(false) => println!("  Easy Job: added, not running"),
        None => {}
    }
}

/// A plugin's address without the secret part after `#`.
fn without_token(url: &str) -> &str {
    url.split_once('#').map_or(url, |(address, _)| address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_skip_empty_states_and_lead_with_the_ones_that_need_you() {
        let agents = vec![
            json!({"status": "working"}),
            json!({"status": "blocked"}),
            json!({"status": "working"}),
            json!({"status": "unknown"}),
        ];
        assert_eq!(
            agent_counts(&agents),
            vec![("need you", 1), ("working", 2), ("idle", 1)]
        );
    }

    #[test]
    fn plugin_addresses_lose_their_token() {
        assert_eq!(
            without_token("http://127.0.0.1:4317/#t=secret"),
            "http://127.0.0.1:4317/"
        );
        assert_eq!(without_token("http://x/"), "http://x/");
    }
}
