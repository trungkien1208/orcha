//! Plugin panes that start with the server, when the user opts in
//! (`plugin.autostart.set`). Each runs as a tab of a background workspace
//! that never takes focus, carries the tokens the plugin asked for (so
//! clients can hide it), and is started again if it exits.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::api::app_call;
use crate::api::schema::{
    Method, PaneInfo, PluginAutostartEntry, PluginAutostartSetParams, PluginPanePlacement,
    ResponseResult, WorkspaceInfo,
};
use crate::api::ApiRequestSender;

const CHECK_INTERVAL: Duration = Duration::from_secs(3);
/// A pane that keeps exiting is started at most this often per minute.
const MAX_STARTS_PER_MINUTE: usize = 3;
/// Token that marks an autostarted pane, `<plugin>/<entrypoint>`.
pub(crate) const PANE_TOKEN: &str = "plugin_autostart";
/// Token that marks the workspace hosting a plugin's autostarted panes.
pub(crate) const HOST_TOKEN: &str = "plugin_autostart_host";
const FILE_NAME: &str = "plugin-autostart.json";

type AutostartError = (&'static str, String);

static WAKE: AtomicBool = AtomicBool::new(true);
static STARTS: LazyLock<Mutex<HashMap<String, Vec<Instant>>>> = LazyLock::new(Default::default);

fn path() -> std::path::PathBuf {
    crate::config::config_dir().join(FILE_NAME)
}

pub(crate) fn load() -> Vec<PluginAutostartEntry> {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save(entries: &[PluginAutostartEntry]) -> Result<(), AutostartError> {
    let path = path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let text = serde_json::to_string_pretty(entries)
        .map_err(|err| ("autostart_failed", err.to_string()))?;
    std::fs::write(&path, format!("{text}\n")).map_err(|err| {
        (
            "autostart_failed",
            format!("could not save {}: {err}", path.display()),
        )
    })
}

/// Stops a running plugin pane. One that starts with the server stops
/// starting too, or the supervisor would open it again within seconds.
pub(super) fn stop_pane(
    pane_id: &str,
    api_tx: &ApiRequestSender,
) -> Result<ResponseResult, AutostartError> {
    let entry = running_entry(pane_id, api_tx)?;
    if entry.autostart {
        let mut entries = load();
        entries.retain(|saved| {
            !(saved.plugin_id == entry.plugin_id && saved.entrypoint == entry.entrypoint)
        });
        save(&entries)?;
    }
    close(pane_id, api_tx)?;
    Ok(ResponseResult::PluginPaneClosed {
        pane_id: pane_id.to_string(),
    })
}

/// Starts a running plugin pane again: closed, then opened in the same
/// workspace without focus (one that starts with the server comes back by
/// itself).
pub(super) fn restart_pane(
    pane_id: &str,
    api_tx: &ApiRequestSender,
) -> Result<ResponseResult, AutostartError> {
    let entry = running_entry(pane_id, api_tx)?;
    close(pane_id, api_tx)?;
    if entry.autostart {
        WAKE.store(true, Ordering::Relaxed);
        return Ok(ResponseResult::Ok {});
    }
    app(
        "restart",
        Method::PluginPaneOpen(crate::api::schema::PluginPaneOpenParams {
            plugin_id: entry.plugin_id,
            entrypoint: entry.entrypoint,
            placement: Some(PluginPanePlacement::Tab),
            width: None,
            height: None,
            workspace_id: Some(entry.workspace_id),
            target_pane_id: None,
            direction: None,
            cwd: None,
            focus: false,
            env: HashMap::new(),
        }),
        api_tx,
    )?;
    Ok(ResponseResult::Ok {})
}

fn running_entry(
    pane_id: &str,
    api_tx: &ApiRequestSender,
) -> Result<crate::api::schema::PluginPaneListEntry, AutostartError> {
    let mut listed = app(
        "plugin-pane",
        Method::PluginPaneList(Default::default()),
        api_tx,
    )?;
    let panes: Vec<crate::api::schema::PluginPaneListEntry> =
        app_call::take(&mut listed, "/result/plugin_panes").unwrap_or_default();
    panes
        .into_iter()
        .find(|pane| pane.pane_id == pane_id)
        .ok_or((
            "plugin_pane_not_found",
            format!("{pane_id} is not a running plugin pane"),
        ))
}

fn close(pane_id: &str, api_tx: &ApiRequestSender) -> Result<(), AutostartError> {
    app(
        "plugin-pane",
        Method::PluginPaneClose(crate::api::schema::PluginPaneCloseParams {
            pane_id: pane_id.to_string(),
        }),
        api_tx,
    )
    .map(|_| ())
}

/// Stops starting every pane of `plugin_id` (it was unlinked).
pub(crate) fn forget(plugin_id: &str) {
    let mut entries = load();
    let before = entries.len();
    entries.retain(|entry| entry.plugin_id != plugin_id);
    if entries.len() != before {
        let _ = save(&entries);
    }
}

pub(super) fn set(
    params: PluginAutostartSetParams,
    api_tx: &ApiRequestSender,
) -> Result<ResponseResult, AutostartError> {
    let plugins = app("autostart", Method::PluginList(Default::default()), api_tx)?;
    let known = plugins
        .pointer("/result/plugins")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .any(|plugin| {
            plugin["plugin_id"].as_str().or(plugin["id"].as_str()) == Some(&params.plugin_id)
                && plugin["panes"].as_array().is_some_and(|panes| {
                    panes
                        .iter()
                        .any(|pane| pane["id"] == params.entrypoint.as_str())
                })
        });
    if params.enabled && !known {
        return Err((
            "plugin_pane_not_found",
            format!("{} has no pane {}", params.plugin_id, params.entrypoint),
        ));
    }
    let mut entries = load();
    entries.retain(|entry| {
        !(entry.plugin_id == params.plugin_id && entry.entrypoint == params.entrypoint)
    });
    if params.enabled {
        entries.push(PluginAutostartEntry {
            plugin_id: params.plugin_id,
            entrypoint: params.entrypoint,
            tokens: params.tokens,
            pane_id: None,
        });
    }
    save(&entries)?;
    WAKE.store(true, Ordering::Relaxed);
    Ok(ResponseResult::PluginAutostart {
        entries: list_with_panes(api_tx),
    })
}

pub(super) fn list(api_tx: &ApiRequestSender) -> ResponseResult {
    ResponseResult::PluginAutostart {
        entries: list_with_panes(api_tx),
    }
}

fn list_with_panes(api_tx: &ApiRequestSender) -> Vec<PluginAutostartEntry> {
    let panes = panes(api_tx).unwrap_or_default();
    load()
        .into_iter()
        .map(|mut entry| {
            entry.pane_id = running_pane(&panes, &entry).map(|pane| pane.pane_id.clone());
            entry
        })
        .collect()
}

fn marker(entry: &PluginAutostartEntry) -> String {
    format!("{}/{}", entry.plugin_id, entry.entrypoint)
}

fn running_pane<'a>(panes: &'a [PaneInfo], entry: &PluginAutostartEntry) -> Option<&'a PaneInfo> {
    let marker = marker(entry);
    panes
        .iter()
        .find(|pane| pane.tokens.get(PANE_TOKEN) == Some(&marker))
}

pub(super) fn spawn_supervisor(api_tx: ApiRequestSender, running: Arc<AtomicBool>) {
    let spawned = crate::thread_spawn::spawn_named("herdr-plugin-autostart", move || {
        let mut next = Instant::now();
        while running.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(250));
            if !WAKE.swap(false, Ordering::Relaxed) && Instant::now() < next {
                continue;
            }
            next = Instant::now() + CHECK_INTERVAL;
            let entries = load();
            if entries.is_empty() {
                continue;
            }
            let Some(panes) = panes(&api_tx) else {
                continue;
            };
            for entry in &entries {
                if running_pane(&panes, entry).is_some() || !may_start(entry) {
                    continue;
                }
                if let Err((_, message)) = start(entry, &api_tx) {
                    tracing::warn!(plugin = %entry.plugin_id, pane = %entry.entrypoint, %message, "plugin autostart failed");
                }
            }
        }
    });
    if let Err(err) = spawned {
        tracing::warn!(err = %err, "plugin autostart unavailable");
    }
}

/// Allows a start unless the pane already started too often in the last minute.
fn may_start(entry: &PluginAutostartEntry) -> bool {
    let Ok(mut starts) = STARTS.lock() else {
        return false;
    };
    let recent = starts.entry(marker(entry)).or_default();
    recent.retain(|at| at.elapsed() < Duration::from_secs(60));
    if recent.len() >= MAX_STARTS_PER_MINUTE {
        return false;
    }
    recent.push(Instant::now());
    true
}

fn start(
    entry: &PluginAutostartEntry,
    api_tx: &ApiRequestSender,
) -> Result<String, AutostartError> {
    let workspace_id = host_workspace(entry, api_tx)?;
    let opened = app(
        "autostart",
        Method::PluginPaneOpen(crate::api::schema::PluginPaneOpenParams {
            plugin_id: entry.plugin_id.clone(),
            entrypoint: entry.entrypoint.clone(),
            placement: Some(PluginPanePlacement::Tab),
            width: None,
            height: None,
            workspace_id: Some(workspace_id),
            target_pane_id: None,
            direction: None,
            cwd: None,
            focus: false,
            env: HashMap::new(),
        }),
        api_tx,
    )?;
    let pane_id = opened
        .pointer("/result/plugin_pane/pane/pane_id")
        .and_then(serde_json::Value::as_str)
        .ok_or((
            "autostart_failed",
            "the plugin pane did not open".to_string(),
        ))?
        .to_string();
    let mut tokens: HashMap<String, Option<String>> = entry
        .tokens
        .iter()
        .map(|(key, value)| (key.clone(), Some(value.clone())))
        .collect();
    tokens.insert(PANE_TOKEN.into(), Some(marker(entry)));
    app(
        "autostart",
        Method::PaneReportMetadata(crate::api::schema::PaneReportMetadataParams {
            pane_id: pane_id.clone(),
            source: "herdr:plugin-autostart".into(),
            agent: None,
            applies_to_source: None,
            title: None,
            display_agent: None,
            state_labels: HashMap::new(),
            tokens,
            clear_title: false,
            clear_display_agent: false,
            clear_state_labels: false,
            seq: None,
            ttl_ms: None,
        }),
        api_tx,
    )?;
    Ok(pane_id)
}

/// The background workspace for a plugin's panes, created without focus.
fn host_workspace(
    entry: &PluginAutostartEntry,
    api_tx: &ApiRequestSender,
) -> Result<String, AutostartError> {
    let mut listed = app(
        "autostart",
        Method::WorkspaceList(Default::default()),
        api_tx,
    )?;
    let workspaces: Vec<WorkspaceInfo> =
        app_call::take(&mut listed, "/result/workspaces").unwrap_or_default();
    if let Some(found) = workspaces
        .iter()
        .find(|workspace| workspace.tokens.get(HOST_TOKEN) == Some(&entry.plugin_id))
    {
        return Ok(found.workspace_id.clone());
    }
    let created = app(
        "autostart",
        Method::WorkspaceCreate(crate::api::schema::WorkspaceCreateParams {
            source_workspace_id: None,
            cwd: None,
            focus: false,
            label: Some(entry.plugin_id.clone()),
            env: HashMap::new(),
        }),
        api_tx,
    )?;
    let workspace_id = created
        .pointer("/result/workspace/workspace_id")
        .and_then(serde_json::Value::as_str)
        .ok_or((
            "autostart_failed",
            "could not create a workspace".to_string(),
        ))?
        .to_string();
    let mut tokens: HashMap<String, Option<String>> = entry
        .tokens
        .iter()
        .map(|(key, value)| (key.clone(), Some(value.clone())))
        .collect();
    tokens.insert(HOST_TOKEN.into(), Some(entry.plugin_id.clone()));
    app(
        "autostart",
        Method::WorkspaceReportMetadata(crate::api::schema::WorkspaceReportMetadataParams {
            workspace_id: workspace_id.clone(),
            source: "herdr:plugin-autostart".into(),
            tokens,
            seq: None,
            ttl_ms: None,
        }),
        api_tx,
    )?;
    Ok(workspace_id)
}

fn panes(api_tx: &ApiRequestSender) -> Option<Vec<PaneInfo>> {
    let mut value = app("autostart", Method::PaneList(Default::default()), api_tx).ok()?;
    app_call::take(&mut value, "/result/panes")
}

fn app(
    request_id: &str,
    method: Method,
    api_tx: &ApiRequestSender,
) -> Result<serde_json::Value, AutostartError> {
    app_call::call(format!("{request_id}:plugin-autostart"), method, api_tx)
        .map_err(|err| err.with_code("autostart_failed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> PluginAutostartEntry {
        PluginAutostartEntry {
            plugin_id: "easy-job".into(),
            entrypoint: "server".into(),
            tokens: HashMap::from([("easy_job".into(), "bridge".into())]),
            pane_id: None,
        }
    }

    #[test]
    fn a_running_pane_is_found_by_its_marker_token() {
        let mut pane: PaneInfo = serde_json::from_value(serde_json::json!({
            "pane_id": "w9:p1", "terminal_id": "t", "workspace_id": "w9", "tab_id": "w9:t1",
            "focused": false, "agent_status": "unknown", "revision": 0
        }))
        .unwrap();
        assert!(running_pane(std::slice::from_ref(&pane), &entry()).is_none());
        pane.tokens
            .insert(PANE_TOKEN.into(), "easy-job/server".into());
        assert_eq!(
            running_pane(std::slice::from_ref(&pane), &entry()).map(|pane| pane.pane_id.as_str()),
            Some("w9:p1")
        );
    }

    #[test]
    fn a_pane_that_keeps_exiting_is_not_restarted_more_than_three_times_a_minute() {
        let mut flapping = entry();
        flapping.entrypoint = "flapping".into();
        assert!((0..MAX_STARTS_PER_MINUTE).all(|_| may_start(&flapping)));
        assert!(!may_start(&flapping));
    }
}
