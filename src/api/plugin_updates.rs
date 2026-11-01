//! `plugin.updates` and `plugin.update`: clients that must stay offline
//! (Easy Job) ask Orcha whether their plugin has a newer version, and ask it
//! to update. The check is Orcha's daily one; `plugin.update` is the user's
//! click in the client that calls it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::api::app_call;
use crate::api::schema::{
    EventData, EventEnvelope, EventKind, Method, PluginUpdateInfo, PluginUpdatesParams,
    ResponseResult,
};
use crate::api::{ApiRequestSender, EventHub};
use crate::orcha_plugin_updates::PluginUpdate;

type UpdateError = (&'static str, String);

/// How often the server looks at the daily check; the check itself asks
/// the network at most once a day.
const CHECK_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// Let startup settle before the first look.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(60);

fn info(update: &PluginUpdate) -> PluginUpdateInfo {
    PluginUpdateInfo {
        id: update.id.clone(),
        current_version: update.current_version.clone(),
        latest_version: update.latest_version.clone(),
        update_available: update.available(),
        compatible: update.compatible,
        needs_herdr_version: update.needs_herdr_version.clone(),
        source: update.source.clone(),
        reference: update.reference.clone(),
    }
}

fn infos(updates: &[PluginUpdate]) -> Vec<PluginUpdateInfo> {
    updates.iter().map(info).collect()
}

fn publish(event_hub: &EventHub, updates: &[PluginUpdate]) {
    event_hub.push(EventEnvelope {
        event: EventKind::PluginUpdatesChanged,
        data: EventData::PluginUpdatesChanged {
            plugins: infos(updates),
        },
    });
}

/// The last check's answer; with `refresh`, asks the sources now.
pub(super) fn list(params: PluginUpdatesParams, event_hub: &EventHub) -> ResponseResult {
    let updates = if params.refresh {
        let (updates, news) = crate::orcha_plugin_updates::check(true);
        if news {
            publish(event_hub, &updates);
        }
        updates
    } else {
        crate::orcha_plugin_updates::cached()
    };
    ResponseResult::PluginUpdates {
        plugins: infos(&updates),
    }
}

/// Installs the newer version of `id`, then restarts its running panes so
/// they run the new files.
pub(super) fn update(
    id: &str,
    api_tx: &ApiRequestSender,
    event_hub: &EventHub,
) -> Result<ResponseResult, UpdateError> {
    let known = crate::orcha_plugin_updates::cached();
    let Some(entry) = known.iter().find(|update| update.id == id) else {
        return Err((
            "plugin_not_found",
            format!("{id} is not a plugin installed from GitHub"),
        ));
    };
    if entry.available() && !entry.compatible {
        return Err(("plugin_requires_newer_herdr", entry.notice(id)));
    }
    let version = crate::orcha_plugin_updates::update_plugin(id)
        .map_err(|log| ("plugin_update_failed", log))?;
    restart_panes(id, api_tx);
    publish(event_hub, &crate::orcha_plugin_updates::cached());
    Ok(ResponseResult::PluginUpdated {
        id: id.to_owned(),
        version,
    })
}

fn restart_panes(id: &str, api_tx: &ApiRequestSender) {
    let Ok(listed) = app_call::call(
        "herdr:plugin-update",
        Method::PluginPaneList(Default::default()),
        api_tx,
    ) else {
        return;
    };
    let result = listed.get("result").cloned().unwrap_or_default();
    for pane_id in crate::orcha_plugin_updates::plugin_pane_ids(&result, id) {
        if let Err((_, message)) = crate::api::plugin_autostart::restart_pane(&pane_id, api_tx) {
            tracing::warn!(%pane_id, %message, "plugin pane was not restarted after its update");
        }
    }
}

/// Runs the daily check in the background and announces new versions with
/// `plugin.updates_changed`.
pub(super) fn spawn_checker(event_hub: EventHub, running: Arc<AtomicBool>) {
    if cfg!(test) {
        return;
    }
    let spawned = crate::thread_spawn::spawn_named("herdr-plugin-updates", move || {
        let mut next = Instant::now() + FIRST_CHECK_DELAY;
        while running.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_secs(1));
            if Instant::now() < next {
                continue;
            }
            next = Instant::now() + CHECK_INTERVAL;
            let (updates, news) = crate::orcha_plugin_updates::check(false);
            if news {
                publish(&event_hub, &updates);
            }
        }
    });
    if let Err(err) = spawned {
        tracing::warn!(err = %err, "plugin update check unavailable");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn methods_parse_and_results_use_the_documented_shapes() {
        let request: crate::api::schema::Request = serde_json::from_str(
            r#"{"id":"u1","method":"plugin.updates","params":{"refresh":true}}"#,
        )
        .unwrap();
        assert!(matches!(
            request.method,
            Method::PluginUpdates(PluginUpdatesParams { refresh: true })
        ));
        let request: crate::api::schema::Request = serde_json::from_str(
            r#"{"id":"u2","method":"plugin.update","params":{"id":"easy-job"}}"#,
        )
        .unwrap();
        assert!(matches!(request.method, Method::PluginUpdate(params) if params.id == "easy-job"));

        let update = PluginUpdate {
            id: "easy-job".into(),
            current_version: "0.1.0".into(),
            latest_version: "0.2.0".into(),
            compatible: true,
            needs_herdr_version: None,
            source: "trungkien1208/orcha-plugin-office".into(),
            reference: Some("plugin".into()),
            installed_commit: Some("a".into()),
            latest_commit: Some("b".into()),
        };
        let encoded = serde_json::to_value(ResponseResult::PluginUpdates {
            plugins: infos(&[update]),
        })
        .unwrap();
        assert_eq!(
            encoded,
            serde_json::json!({"type": "plugin_updates", "plugins": [{
                "id": "easy-job",
                "current_version": "0.1.0",
                "latest_version": "0.2.0",
                "update_available": true,
                "compatible": true,
                "source": "trungkien1208/orcha-plugin-office",
                "ref": "plugin",
            }]})
        );
        let updated = serde_json::to_value(ResponseResult::PluginUpdated {
            id: "easy-job".into(),
            version: "0.2.0".into(),
        })
        .unwrap();
        assert_eq!(
            updated,
            serde_json::json!({"type": "plugin_updated", "id": "easy-job", "version": "0.2.0"})
        );
    }
}
