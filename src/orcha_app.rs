//! orcha: the standalone app identity (docs/orcha/SPEC.md §3.11).
//!
//! The same binary is orcha when it is run as `orcha` (or with `ORCHA_APP=1`):
//! its config and sessions live in `~/.config/orcha` instead of herdr's, the
//! first run imports herdr's config and workspaces once, and the orcha-config
//! commands (`orcha setup`, `orcha menu`, …) are handed to that tool.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub(crate) const ORCHA_APP_ENV_VAR: &str = "ORCHA_APP";
const IMPORT_MARKER: &str = ".imported-from-herdr";
/// orcha-config's own commands, handed to its CLI.
const CONFIG_TOOL_COMMANDS: [&str; 6] = ["setup", "menu", "edit", "logos", "plugin", "apply"];

/// Run as orcha. Children (the server, pane shells) inherit it through the
/// environment, so one session never mixes the two identities.
pub(crate) fn is_orcha() -> bool {
    static ORCHA: OnceLock<bool> = OnceLock::new();
    *ORCHA.get_or_init(|| {
        if cfg!(test) {
            return false;
        }
        if std::env::var(ORCHA_APP_ENV_VAR).is_ok_and(|value| value == "1") {
            return true;
        }
        std::env::args_os()
            .next()
            .and_then(|arg| {
                Path::new(&arg)
                    .file_name()
                    .map(|name| name.to_string_lossy().starts_with("orcha"))
            })
            .unwrap_or(false)
    })
}

/// Marks this process tree as orcha before anything is spawned.
pub(crate) fn claim_identity() {
    if is_orcha() && std::env::var(ORCHA_APP_ENV_VAR).is_err() {
        // single-threaded startup, before any thread or child exists
        std::env::set_var(ORCHA_APP_ENV_VAR, "1");
    }
}

/// orcha-config's CLI: `~/.config/orcha/scripts` links into its checkout.
/// orcha-config lives in `~/.config/orcha` whatever this app's own config
/// folder is (a dev build keeps its config in `orcha-dev`).
pub(crate) fn config_tool() -> Option<PathBuf> {
    let base = match std::env::var("XDG_CONFIG_HOME") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    let scripts = base.join("orcha").join("scripts");
    let checkout = std::fs::canonicalize(scripts).ok()?.parent()?.to_path_buf();
    let tool = checkout.join("bin").join("orcha");
    tool.is_file().then_some(tool)
}

/// `orcha setup|menu|edit|logos|plugin|apply` run orcha-config; `orcha update`
/// updates orcha-config (the app itself is built from source for now).
/// Returns the exit code when the command was handled here.
pub(crate) fn maybe_run_config_tool(args: &[String]) -> Option<i32> {
    if !is_orcha() {
        return None;
    }
    let command = args.get(1)?.as_str();
    if command == "channel" {
        eprintln!("orcha has no release channels yet; build it from source to update.");
        return Some(2);
    }
    if !CONFIG_TOOL_COMMANDS.contains(&command) && command != "update" {
        return None;
    }
    let Some(tool) = config_tool() else {
        eprintln!(
            "orcha {command}: orcha-config is not installed (github.com/trungkien1208/orcha-config)"
        );
        return Some(2);
    };
    let status = std::process::Command::new(tool).args(&args[1..]).status();
    if command == "update" {
        eprintln!("orcha app: download the latest release (see the README) to update the app itself.");
    }
    Some(status.ok().and_then(|status| status.code()).unwrap_or(1))
}

fn legacy_dir() -> PathBuf {
    let name = if cfg!(debug_assertions) {
        "herdr-dev"
    } else {
        "herdr"
    };
    match std::env::var("XDG_CONFIG_HOME") {
        Ok(dir) => PathBuf::from(dir).join(name),
        Err(_) => crate::config::config_dir()
            .parent()
            .map(|parent| parent.join(name))
            .unwrap_or_else(|| PathBuf::from(name)),
    }
}

/// herdr's config without its colors (orcha brings its own look) and
/// without the plan-limits popup binding (orcha's accounts popup owns
/// prefix+u).
fn import_config(text: &str) -> String {
    let mut out = Vec::new();
    let mut skipping = false;
    let mut block: Vec<&str> = Vec::new();
    let flush = |block: &mut Vec<&str>, out: &mut Vec<String>| {
        let drop = block.iter().any(|line| line.contains("usage-picker"));
        if !drop {
            out.extend(block.iter().map(|line| (*line).to_owned()));
        }
        block.clear();
    };
    for line in text.lines() {
        let header = line.trim_start().starts_with('[');
        if header {
            flush(&mut block, &mut out);
            let name = line.trim().trim_start_matches('[').trim_end_matches(']');
            skipping = name == "theme" || name.starts_with("theme.");
        }
        if skipping {
            continue;
        }
        if line.trim() == "[[keys.command]]" || !block.is_empty() {
            block.push(line);
        } else {
            out.push(line.to_owned());
        }
    }
    flush(&mut block, &mut out);
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// Workspaces come back, but not their agents: herdr may still run those
/// same conversations, and two copies of one chat would fight.
fn import_session(text: &str) -> Option<String> {
    fn strip(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                for key in ["agent_session", "agent_resume", "launch_argv"] {
                    map.remove(key);
                }
                map.values_mut().for_each(strip);
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut value: serde_json::Value = serde_json::from_str(text).ok()?;
    strip(&mut value);
    serde_json::to_string(&value).ok()
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if kind.is_file() && !target.exists() {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// First run as orcha: bring herdr's config, workspaces, plugins and sounds
/// over once. Returns what was imported, for the startup notice.
pub(crate) fn import_from_herdr_once() -> Option<String> {
    if !is_orcha() {
        return None;
    }
    let dir = crate::config::config_dir();
    let marker = dir.join(IMPORT_MARKER);
    if marker.exists() || dir.join("config.toml").exists() {
        return None;
    }
    let legacy = legacy_dir();
    let config = std::fs::read_to_string(legacy.join("config.toml")).ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    let mut imported = vec!["config"];
    let _ = std::fs::write(dir.join("config.toml"), import_config(&config));
    if let Some(session) = std::fs::read_to_string(legacy.join("session.json"))
        .ok()
        .and_then(|text| import_session(&text))
    {
        if std::fs::write(dir.join("session.json"), session).is_ok() {
            imported.push("workspaces");
        }
    }
    for name in ["plugins", "sounds"] {
        let from = legacy.join(name);
        if from.is_dir() && copy_dir(&from, &dir.join(name)).is_ok() {
            imported.push(name);
        }
    }
    let summary = imported.join(", ");
    let _ = std::fs::write(
        &marker,
        format!("imported {summary} from {}\n", legacy.display()),
    );
    Some(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_config_drops_colors_and_the_limits_popup() {
        let text = "onboarding = false\n\n[theme]\nname = \"rose-pine\"\n\n[theme.custom]\naccent = \"#fff\"\n\n[ui]\nsidebar_width = 48\n\n[[keys.command]]\nkey = \"prefix+u\"\ncommand = \"/x/usage-picker.sh\"\n\n[[keys.command]]\nkey = \"prefix+f\"\ncommand = \"/x/find-file.sh\"\n";
        let out = import_config(text);
        assert!(!out.contains("rose-pine") && !out.contains("[theme"));
        assert!(!out.contains("usage-picker"));
        assert!(out.contains("find-file") && out.contains("sidebar_width = 48"));
        assert!(out.contains("onboarding = false"));
    }

    #[test]
    fn imported_workspaces_do_not_resume_agents() {
        let text = r#"{"workspaces":[{"tabs":[{"panes":{"1":{"cwd":"/x","agent_session":{"value":"a"},"agent_resume":{"argv":["claude"]}}}}]}]}"#;
        let out = import_session(text).expect("json");
        assert!(out.contains("/x"));
        assert!(!out.contains("agent_session") && !out.contains("agent_resume"));
    }
}
