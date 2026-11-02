//! Runs this app's own CLI (`orcha plugin install …`, `orcha api call …`)
//! as a child process, against the current session.

use std::path::PathBuf;
use std::process::{Command, ExitStatus, Output, Stdio};

/// This executable with some arguments, pointed at the current session's
/// socket, with no input. How it runs decides where its output goes.
pub(crate) struct OrchaCli(Command);

impl OrchaCli {
    pub(crate) fn new(args: &[&str]) -> Self {
        let exe = std::env::current_exe().unwrap_or_else(|_| {
            PathBuf::from(if crate::orcha_app::is_orcha() {
                "orcha"
            } else {
                "herdr"
            })
        });
        let mut command = Command::new(exe);
        command
            .args(args)
            .env(
                crate::api::SOCKET_PATH_ENV_VAR,
                crate::session::active_api_socket_path(),
            )
            .stdin(Stdio::null());
        Self(command)
    }

    /// No window on Windows: for anything run behind the app's own UI.
    fn background(mut self) -> Self {
        crate::platform::configure_background_command(&mut self.0);
        self
    }

    /// Runs it with its output dropped.
    pub(crate) fn quiet(self) -> std::io::Result<ExitStatus> {
        let mut command = self.background().0;
        command.stdout(Stdio::null()).stderr(Stdio::null());
        command.status()
    }

    /// Runs it and returns what it printed.
    pub(crate) fn output(self) -> std::io::Result<Output> {
        let mut command = self.background().0;
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        command.output()
    }

    /// Runs it in this terminal, so the user sees its progress. Only for
    /// orcha's own commands typed in a terminal.
    pub(crate) fn in_terminal(self) -> std::io::Result<ExitStatus> {
        let mut command = self.0;
        command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        command.status()
    }

    /// The command itself, for callers that run it their own way: no
    /// window, errors dropped.
    pub(crate) fn into_command(self) -> Command {
        let mut command = self.background().0;
        command.stderr(Stdio::null());
        command
    }
}

/// True when it ran and succeeded.
pub(crate) fn succeeded(status: std::io::Result<ExitStatus>) -> bool {
    status.is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_targets_this_session_with_no_input() {
        let command = OrchaCli::new(&["api", "call", "plugin.list"]).into_command();
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["api", "call", "plugin.list"]);
        assert!(command
            .get_envs()
            .any(|(key, value)| key == crate::api::SOCKET_PATH_ENV_VAR && value.is_some()));
    }
}
