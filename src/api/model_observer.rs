//! Keeps agent records' model, effort and fast mode current by reading the
//! settings agents record in their own transcripts. Runs on its own thread and
//! re-reads a transcript only after it changed.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::agent_models::ObservedModel;
use crate::api::schema::{AgentInfo, Method, PaneReportAgentModelParams, Request};
use crate::api::server::{dispatch_to_app_with_timeout, APP_RESPONSE_TIMEOUT};
use crate::api::ApiRequestSender;

const OBSERVE_INTERVAL: Duration = Duration::from_secs(2);
const OBSERVER_SOURCE: &str = "herdr:observer";

#[derive(Default)]
struct Seen {
    file: Option<(u64, Option<SystemTime>)>,
    /// Bytes already read; later changes read only what was appended.
    offset: Option<u64>,
    observed: ObservedModel,
}

pub(super) fn spawn(api_tx: ApiRequestSender, running: Arc<AtomicBool>) {
    let spawned = crate::thread_spawn::spawn_named("herdr-model-observer", move || {
        let mut seen: HashMap<(String, String), Seen> = HashMap::new();
        while running.load(Ordering::Relaxed) {
            std::thread::sleep(OBSERVE_INTERVAL);
            let Some(agents) = agent_list(&api_tx) else {
                continue;
            };
            observe(&agents, &mut seen, |report| {
                let _ = dispatch_to_app_with_timeout(
                    Request {
                        id: "herdr:observer".into(),
                        method: Method::PaneReportAgentModel(report),
                    },
                    &api_tx,
                    Some(APP_RESPONSE_TIMEOUT),
                );
            });
        }
    });
    if let Err(err) = spawned {
        tracing::warn!(err = %err, "agent model observer unavailable");
    }
}

fn agent_list(api_tx: &ApiRequestSender) -> Option<Vec<AgentInfo>> {
    let response = dispatch_to_app_with_timeout(
        Request {
            id: "herdr:observer:agents".into(),
            method: Method::AgentList(Default::default()),
        },
        api_tx,
        Some(APP_RESPONSE_TIMEOUT),
    );
    let value: serde_json::Value = serde_json::from_str(&response).ok()?;
    serde_json::from_value(value.pointer("/result/agents")?.clone()).ok()
}

fn observe(
    agents: &[AgentInfo],
    seen: &mut HashMap<(String, String), Seen>,
    mut report: impl FnMut(PaneReportAgentModelParams),
) {
    let mut live = Vec::new();
    for agent in agents {
        let Some(kind) = agent
            .agent
            .as_deref()
            .filter(|kind| matches!(*kind, "claude" | "codex"))
        else {
            continue;
        };
        let Some(path) = agent
            .agent_session
            .as_ref()
            .and_then(|session| session.transcript_path.as_deref())
        else {
            continue;
        };
        let key = (agent.pane_id.clone(), path.to_string());
        live.push(key.clone());
        let Ok(metadata) = std::fs::metadata(path) else {
            continue;
        };
        let file = Some((metadata.len(), metadata.modified().ok()));
        let entry = seen.entry(key).or_default();
        if entry.file != file {
            entry.file = file;
            let from = entry.offset.filter(|offset| *offset <= metadata.len());
            let Some((found, offset)) =
                crate::agent_models::observe_transcript(kind, std::path::Path::new(path), from)
            else {
                continue;
            };
            entry.offset = Some(offset);
            if from.is_none() {
                entry.observed = ObservedModel::default();
            }
            if found.model.is_some() {
                entry.observed.model = found.model;
            }
            if found.effort.is_some() {
                entry.observed.effort = found.effort;
            }
            if found.fast.is_some() {
                entry.observed.fast = found.fast;
            }
        }
        let observed = &entry.observed;
        let current = (agent.model.as_ref(), agent.effort.as_ref(), agent.fast);
        let wanted = (
            observed.model.as_ref().or(agent.model.as_ref()),
            observed.effort.as_ref().or(agent.effort.as_ref()),
            observed.fast.or(agent.fast),
        );
        if current != wanted {
            report(PaneReportAgentModelParams {
                pane_id: agent.pane_id.clone(),
                source: OBSERVER_SOURCE.into(),
                model: observed.model.clone(),
                effort: observed.effort.clone(),
                fast: observed.fast,
            });
        }
    }
    seen.retain(|key, _| live.contains(key));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(kind: &str, path: &str) -> AgentInfo {
        serde_json::from_value(serde_json::json!({
            "terminal_id": "t1",
            "agent": kind,
            "agent_status": "idle",
            "agent_session": {
                "source": format!("herdr:{kind}"), "agent": kind, "kind": "id",
                "value": "s1", "id": "s1", "transcript_path": path
            },
            "workspace_id": "w1", "tab_id": "w1:t1", "pane_id": "w1:p1",
            "focused": false, "revision": 0
        }))
        .unwrap()
    }

    #[test]
    fn observer_reports_transcript_settings_once_per_change() {
        let dir = std::env::temp_dir().join(format!("herdr-observer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"assistant","effort":"high","message":{"model":"claude-opus-5-5","usage":{"speed":"fast"}}}"#,
        )
        .unwrap();
        let agents = vec![agent("claude", path.to_str().unwrap())];
        let mut seen = HashMap::new();
        let mut reports = Vec::new();

        observe(&agents, &mut seen, |report| reports.push(report));
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].effort.as_deref(), Some("high"));
        assert_eq!(reports[0].fast, Some(true));
        assert_eq!(reports[0].source, OBSERVER_SOURCE);

        let mut applied = agents[0].clone();
        applied.model = Some("claude-opus-5-5".into());
        applied.effort = Some("high".into());
        applied.fast = Some(true);
        observe(std::slice::from_ref(&applied), &mut seen, |report| {
            reports.push(report)
        });
        assert_eq!(
            reports.len(),
            1,
            "a record that already matches is not reported"
        );

        applied.effort = Some("low".into());
        observe(std::slice::from_ref(&applied), &mut seen, |report| {
            reports.push(report)
        });
        assert_eq!(reports.len(), 2, "a record that drifted is corrected");

        std::fs::write(
            &path,
            r#"{"type":"assistant","effort":"high","message":{"model":"claude-opus-5-5","usage":{"speed":"fast"}}}
{"type":"assistant","effort":"max","message":{"model":"claude-opus-5-5","usage":{"speed":"fast"}}}"#,
        )
        .unwrap();
        applied.effort = Some("high".into());
        observe(std::slice::from_ref(&applied), &mut seen, |report| {
            reports.push(report)
        });
        assert_eq!(reports.last().unwrap().effort.as_deref(), Some("max"));

        observe(&agents[..0], &mut seen, |report| reports.push(report));
        assert!(seen.is_empty(), "closed panes are forgotten");
        let _ = std::fs::remove_dir_all(dir);
    }
}
