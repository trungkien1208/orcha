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

fn installed() -> bool {
    crate::persist::plugin_registry::try_load()
        .is_ok_and(|plugins| plugins.iter().any(|plugin| plugin.plugin_id == PLUGIN_ID))
}

fn not_added() -> i32 {
    eprintln!("Easy Job is not added yet. Add it with: orcha office install");
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
    if installed() {
        println!("Easy Job is already added. Open it with: orcha office");
        return 0;
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
    if !installed() {
        println!("Easy Job is not added. Nothing to remove.");
        return 0;
    }
    if !ask("Remove Easy Job? Its server stops.") {
        println!("Nothing changed.");
        return 1;
    }
    stop_panes();
    if !status_of(orcha(&["plugin", "uninstall", PLUGIN_ID]).stdin(Stdio::null())) {
        eprintln!("Easy Job was not removed.");
        return 1;
    }
    let data = crate::platform::user_home_dir().map(|home| home.join(".easy-job"));
    if let Some(data) = data.filter(|data| data.exists()) {
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
    fn help_and_unknown_commands_change_nothing() {
        assert_eq!(run(&["--help".to_owned()]), 0);
        assert_eq!(run(&["fly".to_owned()]), 2);
    }
}
