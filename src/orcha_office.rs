//! `orcha office`: opens, adds, updates and removes Easy Job, the pixel-art
//! office plugin. Nothing is installed, updated or removed without the
//! command (or a yes) from the user.

use std::path::PathBuf;
use std::process::{Command, Stdio};

pub(crate) const PLUGIN_ID: &str = "easy-job";
const USAGE: &str = "usage: orcha office [install|update|remove]";

pub(crate) fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        None | Some("open") => open(),
        Some("install") => install(),
        Some("update") => update(),
        Some("remove") | Some("uninstall") => remove(),
        Some("--help") | Some("-h") | Some("help") => {
            println!("{USAGE}");
            println!("  orcha office          open Easy Job");
            println!("  orcha office install  add Easy Job from GitHub");
            println!("  orcha office update   get the newest Easy Job");
            println!("  orcha office remove   remove Easy Job (asks first)");
            0
        }
        Some(other) => {
            eprintln!("unknown office command: {other}\n{USAGE}");
            2
        }
    }
}

/// This app's executable against the current session.
fn orcha(args: &[&str]) -> Command {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("orcha"));
    let mut command = Command::new(exe);
    command.args(args).env(
        crate::api::SOCKET_PATH_ENV_VAR,
        crate::session::active_api_socket_path(),
    );
    command
}

fn status_of(command: &mut Command) -> bool {
    command.status().is_ok_and(|status| status.success())
}

#[derive(Debug, PartialEq, Eq)]
enum Install {
    Missing,
    /// Recorded, but its files are gone (say, a link to a deleted folder).
    Broken,
    Ready,
}

fn install_state() -> Install {
    let plugins = crate::persist::plugin_registry::try_load().unwrap_or_default();
    state_of(&plugins)
}

fn state_of(plugins: &[crate::api::schema::InstalledPluginInfo]) -> Install {
    match plugins.iter().find(|plugin| plugin.plugin_id == PLUGIN_ID) {
        None => Install::Missing,
        Some(plugin)
            if plugin.warnings.iter().any(|warning| {
                warning.starts_with(
                    crate::persist::plugin_registry::MANIFEST_UNAVAILABLE_WARNING_PREFIX,
                )
            }) =>
        {
            Install::Broken
        }
        Some(_) => Install::Ready,
    }
}

fn installed() -> bool {
    install_state() == Install::Ready
}

/// The office as the header shows it: None when it is not added, else
/// whether its server runs. Blocking (two small file reads): call it off
/// the input path.
pub(crate) fn office_state() -> Option<bool> {
    installed().then(|| bridge_pid().is_some())
}

/// Easy Job's own data folder, found the way Easy Job finds it: an env
/// override, else ~/.easy-job, else the pre-rename ~/.orcha-office when only
/// that one exists. Guessing only ~/.easy-job left the header chip dark.
fn data_home() -> Option<PathBuf> {
    let env = |name| std::env::var_os(name).filter(|value| !value.is_empty());
    if let Some(dir) = env("EASY_JOB_HOME").or_else(|| env("ORCHA_OFFICE_HOME")) {
        return Some(PathBuf::from(dir));
    }
    let home = crate::platform::user_home_dir()?;
    Some(pick_data_home(
        home.join(".easy-job"),
        home.join(".orcha-office"),
    ))
}

fn pick_data_home(next: PathBuf, previous: PathBuf) -> PathBuf {
    if !next.exists() && previous.exists() {
        previous
    } else {
        next
    }
}

/// The server's pid from the state file Easy Job keeps, when it still runs.
fn bridge_pid() -> Option<u32> {
    let path = data_home()?.join("bridge.json");
    let state: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let pid = u32::try_from(state.get("pid")?.as_u64()?).ok()?;
    crate::platform::process_exists(pid).then_some(pid)
}

fn plugin_root() -> Option<PathBuf> {
    crate::persist::plugin_registry::try_load()
        .ok()?
        .into_iter()
        .find(|plugin| plugin.plugin_id == PLUGIN_ID)
        .map(|plugin| PathBuf::from(plugin.plugin_root))
}

/// Opens the office in the browser, starting it when it is not running.
pub(crate) fn open_quietly() -> Result<(), String> {
    let output = orcha(&["plugin", "action", "invoke", "open", "--plugin", PLUGIN_ID])
        .stdin(Stdio::null())
        .output()
        .map_err(|err| format!("could not open Easy Job: {err}"))?;
    if output.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&output.stderr);
    Err(text
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .unwrap_or("could not open Easy Job")
        .trim()
        .to_owned())
}

/// Stops the server Easy Job started by itself (`open`), which is not a pane.
fn stop_bridge() {
    if bridge_pid().is_none() {
        return;
    }
    if let Some(root) = plugin_root() {
        let _ = Command::new(root.join("bin").join("easy-job"))
            .arg("stop")
            .current_dir(&root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// After an update: a server Easy Job started by itself (not a pane, so
/// pane restarts miss it) is started again from the new files, in the
/// background. Its port and token stay, so open pages reconnect.
pub(crate) fn restart_detached_office() {
    if bridge_pid().is_none() {
        return;
    }
    stop_bridge();
    let Some(root) = plugin_root() else {
        return;
    };
    let mut command = crate::noninteractive_process::command(root.join("bin").join("easy-job"));
    command
        .args(["serve", "--no-open"])
        .current_dir(&root)
        .env(
            crate::api::SOCKET_PATH_ENV_VAR,
            crate::session::active_api_socket_path(),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Err(err) = command.spawn() {
        tracing::warn!(%err, "Easy Job did not start again after its update");
    }
}

/// Removes Easy Job without questions (the caller asked): stops its server
/// and panes, then uninstalls it. Its data in ~/.easy-job stays.
pub(crate) fn remove_quietly() -> Result<(), String> {
    stop_bridge();
    stop_panes();
    let output = orcha(&["plugin", "uninstall", PLUGIN_ID])
        .stdin(Stdio::null())
        .output()
        .map_err(|err| format!("could not remove Easy Job: {err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

fn not_added() -> i32 {
    if install_state() == Install::Broken {
        eprintln!("Easy Job's files are gone. Add it again with: orcha office install");
    } else {
        eprintln!("Easy Job is not added yet. Add it with: orcha office install");
    }
    1
}

fn open() -> i32 {
    if !installed() {
        return not_added();
    }
    let params = serde_json::json!({"plugin_id": PLUGIN_ID, "action_id": "open"}).to_string();
    let output = orcha(&["api", "call", "plugin.run_action", &params])
        .stdin(Stdio::null())
        .output();
    match output {
        Ok(output) if output.status.success() => 0,
        Ok(output) => {
            let message = serde_json::from_slice::<serde_json::Value>(&output.stdout)
                .ok()
                .and_then(|value| value["error"]["message"].as_str().map(str::to_owned))
                .unwrap_or_else(|| String::from_utf8_lossy(&output.stderr).trim().to_owned());
            eprintln!("could not open Easy Job: {message}");
            1
        }
        Err(err) => {
            eprintln!("could not open Easy Job: {err}");
            1
        }
    }
}

fn install() -> i32 {
    match install_state() {
        Install::Ready => {
            println!("Easy Job is already added. Open it with: orcha office");
            return 0;
        }
        Install::Broken => {
            // its files are gone: drop the record so the GitHub install can take its place
            if !status_of(orcha(&["plugin", "unlink", PLUGIN_ID]).stdout(Stdio::null())) {
                eprintln!(
                    "Easy Job's old entry could not be removed. Run: orcha plugin unlink easy-job"
                );
                return 1;
            }
        }
        Install::Missing => {}
    }
    // asked for by name: the in-app offer has its answer
    crate::orcha_app::answer_plugin_offer(PLUGIN_ID, false);
    if !status_of(&mut orcha(crate::orcha_app::EASY_JOB_INSTALL)) {
        crate::orcha_app::forget_plugin_offer(PLUGIN_ID);
        eprintln!("Easy Job was not added.");
        return 1;
    }
    println!("Easy Job added. Open it with: orcha office");
    0
}

fn update() -> i32 {
    if !installed() {
        return not_added();
    }
    match crate::orcha_plugin_updates::update_plugin(PLUGIN_ID) {
        Ok(version) => {
            crate::orcha_plugin_updates::restart_running_panes(PLUGIN_ID);
            println!("Easy Job is at {version}.");
            0
        }
        Err(log) => {
            eprintln!("{log}\nEasy Job was not updated.");
            1
        }
    }
}

fn ask(question: &str) -> bool {
    use std::io::Write;
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).is_ok()
        && matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
}

fn remove() -> i32 {
    if install_state() == Install::Missing {
        println!("Easy Job is not added. Nothing to remove.");
        return 0;
    }
    if !ask("Remove Easy Job? Its server stops.") {
        println!("Nothing changed.");
        return 1;
    }
    stop_bridge();
    stop_panes();
    if !status_of(orcha(&["plugin", "uninstall", PLUGIN_ID]).stdin(Stdio::null())) {
        eprintln!("Easy Job was not removed.");
        return 1;
    }
    if let Some(data) = data_home().filter(|data| data.exists()) {
        if ask(&format!(
            "Also delete Easy Job's own data in {}?",
            data.display()
        )) {
            if let Err(err) = std::fs::remove_dir_all(&data) {
                eprintln!("could not delete {}: {err}", data.display());
            }
        } else {
            println!("Kept {}.", data.display());
        }
    }
    println!("Easy Job removed.");
    0
}

/// Stops Easy Job's running panes (its server), which also stops starting
/// them with Orcha. A server that is not running has nothing to stop.
fn stop_panes() {
    let Some(listed) = crate::orcha_handoff::herdr_json(&["api", "call", "plugin.pane.list"])
    else {
        return;
    };
    for pane_id in crate::orcha_plugin_updates::plugin_pane_ids(&listed, PLUGIN_ID) {
        let params = serde_json::json!({ "pane_id": pane_id }).to_string();
        let _ = crate::orcha_handoff::herdr_json(&["api", "call", "plugin.pane.stop", &params]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_whose_files_are_gone_counts_as_broken() {
        let plugin = |warnings: Vec<String>| -> crate::api::schema::InstalledPluginInfo {
            serde_json::from_value(serde_json::json!({
                "plugin_id": PLUGIN_ID,
                "name": "Easy Job",
                "version": "0.1.0",
                "min_herdr_version": "0.9.0",
                "manifest_path": "/gone/herdr-plugin.toml",
                "plugin_root": "/gone",
                "enabled": true,
                "warnings": warnings,
            }))
            .unwrap()
        };
        assert_eq!(state_of(&[]), Install::Missing);
        assert_eq!(state_of(&[plugin(Vec::new())]), Install::Ready);
        let gone = format!(
            "{}no such file",
            crate::persist::plugin_registry::MANIFEST_UNAVAILABLE_WARNING_PREFIX
        );
        assert_eq!(state_of(&[plugin(vec![gone])]), Install::Broken);
    }

    #[test]
    fn the_pre_rename_data_folder_counts_until_the_new_one_exists() {
        let root = std::env::temp_dir().join(format!("orcha-office-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let next = root.join(".easy-job");
        let previous = root.join(".orcha-office");
        assert_eq!(pick_data_home(next.clone(), previous.clone()), next);
        std::fs::create_dir(&previous).unwrap();
        assert_eq!(pick_data_home(next.clone(), previous.clone()), previous);
        std::fs::create_dir(&next).unwrap();
        assert_eq!(pick_data_home(next.clone(), previous), next);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn help_and_unknown_commands_change_nothing() {
        assert_eq!(run(&["--help".to_owned()]), 0);
        assert_eq!(run(&["fly".to_owned()]), 2);
    }
}
