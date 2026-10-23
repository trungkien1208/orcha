//! orcha: the standalone app identity (docs/orcha/SPEC.md §3.11).
//!
//! The same binary is orcha when it is run as `orcha` (or with `ORCHA_APP=1`):
//! its config and sessions live in `~/.config/orcha` instead of herdr's, the
//! first run imports herdr's config and workspaces once, and the orcha-config
//! commands (`orcha setup`, `orcha menu`, …) are handed to that tool.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub(crate) const ORCHA_APP_ENV_VAR: &str = "ORCHA_APP";
/// orcha's own version; its GitHub releases are tagged `orcha-<version>`.
pub(crate) const ORCHA_VERSION: &str = "0.1.2";
const RELEASES_API: &str = "https://api.github.com/repos/trungkien1208/orcha/releases/latest";
/// Installs or updates the app and its extras (see install.sh).
const INSTALL_COMMAND: &str =
    "curl -fsSL https://raw.githubusercontent.com/trungkien1208/orcha/HEAD/install.sh | sh";
/// The newest release seen, checked at most once a day.
const RELEASE_CACHE: &str = ".latest-release";
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

/// `$XDG_CONFIG_HOME`, else `~/.config`.
fn config_base() -> Option<PathBuf> {
    match std::env::var("XDG_CONFIG_HOME") {
        Ok(dir) => Some(PathBuf::from(dir)),
        Err(_) => Some(PathBuf::from(std::env::var_os("HOME")?).join(".config")),
    }
}

/// orcha-config's CLI: `~/.config/orcha/scripts` links into its checkout.
/// orcha-config lives in `~/.config/orcha` whatever this app's own config
/// folder is (a dev build keeps its config in `orcha-dev`).
pub(crate) fn config_tool() -> Option<PathBuf> {
    let scripts = config_base()?.join("orcha").join("scripts");
    let checkout = std::fs::canonicalize(scripts).ok()?.parent()?.to_path_buf();
    let tool = checkout.join("bin").join("orcha");
    tool.is_file().then_some(tool)
}

/// orcha's own commands: `--version`, `update`, `uninstall`; and
/// `setup|menu|edit|logos|plugin|apply`, handed to orcha-config.
/// Returns the exit code when the command was handled here.
pub(crate) fn maybe_run_config_tool(args: &[String]) -> Option<i32> {
    if !is_orcha() {
        return None;
    }
    let command = args.get(1)?.as_str();
    match command {
        "--version" | "-V" | "version" => {
            println!(
                "orcha {ORCHA_VERSION} (herdr {})",
                env!("CARGO_PKG_VERSION")
            );
            return Some(0);
        }
        "update" => return Some(update_from_cli()),
        "uninstall" => return Some(uninstall()),
        "channel" => {
            eprintln!("orcha has one channel: `orcha update` installs the latest release.");
            return Some(2);
        }
        _ => {}
    }
    if !CONFIG_TOOL_COMMANDS.contains(&command) {
        return None;
    }
    let Some(tool) = config_tool() else {
        eprintln!(
            "orcha {command}: orcha-config is not installed (github.com/trungkien1208/orcha-config)"
        );
        return Some(2);
    };
    let status = std::process::Command::new(tool).args(&args[1..]).status();
    Some(status.ok().and_then(|status| status.code()).unwrap_or(1))
}

/// "orcha-0.1.3" / "0.1.3" → (0, 1, 3).
fn version_parts(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version
        .trim()
        .trim_start_matches("orcha-")
        .trim_start_matches('v')
        .split('.')
        .map(|part| part.parse::<u32>().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

/// A release newer than this build, e.g. "0.1.3". Asks GitHub at most once
/// a day (the answer is kept in the config folder). Blocking: call it off
/// the input path.
pub(crate) fn newer_release() -> Option<String> {
    let cache = crate::config::config_dir().join(RELEASE_CACHE);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let cached = std::fs::read_to_string(&cache).ok().and_then(|text| {
        let (at, tag) = text.trim().split_once(' ')?;
        let at: u64 = at.parse().ok()?;
        (now.saturating_sub(at) < 24 * 60 * 60).then(|| tag.to_owned())
    });
    let tag = match cached {
        Some(tag) => tag,
        None => {
            let output = crate::noninteractive_process::curl_command()
                .args([
                    "-sfL",
                    "--connect-timeout",
                    "5",
                    "--max-time",
                    "10",
                    RELEASES_API,
                ])
                .output()
                .ok()
                .filter(|output| output.status.success())?;
            let release: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
            let tag = release["tag_name"].as_str()?.to_owned();
            let _ = std::fs::write(&cache, format!("{now} {tag}\n"));
            tag
        }
    };
    (version_parts(&tag)? > version_parts(ORCHA_VERSION)?)
        .then(|| tag.trim_start_matches("orcha-").to_owned())
}

/// Runs the installer: the latest app and extras. `quiet` keeps its output
/// off the screen (the app runs it behind its own UI).
pub(crate) fn run_installer(quiet: bool) -> bool {
    let mut command = std::process::Command::new("sh");
    command.arg("-c").arg(INSTALL_COMMAND);
    if quiet {
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    }
    let ok = command.status().is_ok_and(|status| status.success());
    if ok {
        // the next check asks GitHub again
        let _ = std::fs::remove_file(crate::config::config_dir().join(RELEASE_CACHE));
    }
    ok
}

fn update_from_cli() -> i32 {
    if !run_installer(false) {
        eprintln!("orcha update failed. Try again, or run:\n  {INSTALL_COMMAND}");
        return 1;
    }
    println!("Quit orcha and open it again to use the new version. Your agents keep running.");
    0
}

fn ask(question: &str) -> bool {
    use std::io::Write;
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).is_ok()
        && matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
}

/// `orcha uninstall`: stops orcha (and the agents in it), takes out
/// everything the installer put in, and, if asked, the settings and saved
/// projects too. A checkout someone cloned by hand is left alone.
fn uninstall() -> i32 {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    println!("This removes orcha: the app, its extras, its hooks in Claude Code and Codex, the logo font and the lines it added to ~/.zshrc.");
    println!("Agents running inside orcha are stopped.");
    if !ask("Uninstall orcha?") {
        println!("Nothing changed.");
        return 1;
    }
    let me = std::env::current_exe().ok();
    if let Some(me) = &me {
        let _ = std::process::Command::new(me)
            .args(["server", "stop"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    if let Some(tool) = config_tool() {
        let _ = std::process::Command::new(&tool)
            .args(["uninstall", "--all"])
            .status();
        // the extras' folder, when the installer made it
        let installed = std::env::var_os("ORCHA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".orcha"));
        if let (Some(checkout), Ok(installed)) = (
            tool.parent().and_then(Path::parent),
            std::fs::canonicalize(&installed),
        ) {
            if checkout == installed {
                let _ = std::fs::remove_dir_all(checkout);
            } else {
                println!("kept {} (not made by the installer)", checkout.display());
            }
        }
    }
    // the app's folder, and orcha-config's (the same one in a release)
    let mut folders = vec![crate::config::config_dir()];
    if let Some(extras) = config_base().map(|base| base.join("orcha")) {
        if !folders.contains(&extras) {
            folders.push(extras);
        }
    }
    folders.retain(|folder| folder.exists());
    let shown: Vec<String> = folders.iter().map(|f| f.display().to_string()).collect();
    if !folders.is_empty()
        && ask(&format!(
            "Also delete your settings and saved projects ({})?",
            shown.join(", ")
        ))
    {
        for folder in &folders {
            let _ = std::fs::remove_dir_all(folder);
        }
    }
    if let Some(me) = me.filter(|me| me.starts_with(home.join(".local/bin"))) {
        let _ = std::fs::remove_file(me);
    }
    println!("orcha is uninstalled. Thanks for trying it 🐋🍵");
    0
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
    fn release_versions_compare_by_number() {
        assert_eq!(version_parts("orcha-0.1.10"), Some((0, 1, 10)));
        assert!(version_parts("orcha-0.1.10") > version_parts("0.1.9"));
        assert_eq!(version_parts("preview-x"), None);
    }

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
