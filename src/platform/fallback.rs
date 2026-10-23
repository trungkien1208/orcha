use std::path::PathBuf;
use std::process::Command;

use super::{ClipboardImage, ForegroundJob, Signal};

#[cfg(unix)]
pub(crate) use super::unix_common::set_default_plugin_pane_pwd;

#[cfg(not(unix))]
pub(crate) fn set_default_plugin_pane_pwd(
    _env: &mut Vec<(String, String)>,
    _cwd: &std::path::Path,
) {
}

#[cfg(unix)]
pub(super) fn socket_peer_pid(_fd: std::os::fd::RawFd) -> Option<u32> {
    None
}

#[cfg(unix)]
pub(super) fn process_name_and_parent(_pid: u32) -> Option<(String, u32)> {
    None
}

#[cfg(unix)]
pub(super) const REMOTE_BRIDGE_CLOCK: libc::clockid_t = libc::CLOCK_MONOTONIC;

pub(crate) fn forward_remote_bridge_stdio(
    stream: crate::ipc::LocalStream,
    _idle_timeout: bool,
) -> std::io::Result<()> {
    use interprocess::TryClone as _;

    let mut stdout = std::io::stdout().lock();
    let mut socket_to_stdout = stream.try_clone()?;
    let mut stdin_to_socket = stream;
    let _upload = std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let _ = std::io::copy(&mut stdin, &mut stdin_to_socket);
    });
    std::io::copy(&mut socket_to_stdout, &mut stdout).map(|_| ())
}

#[cfg(not(unix))]
pub(super) fn read_terminal_grid_size() -> std::io::Result<(u16, u16)> {
    crossterm::terminal::size()
}

pub(crate) fn remote_ssh_config_paths() -> super::RemoteSshConfigPaths {
    super::RemoteSshConfigPaths {
        user_config: std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".ssh").join("config")),
        system_config: None,
        multiplexing: false,
    }
}

pub(crate) fn create_remote_ssh_config_dir(_control_socket_name: &str) -> std::io::Result<PathBuf> {
    for attempt in 0..100 {
        let dir = std::env::temp_dir().join(format!("herdr-ssh-{}-{attempt}", std::process::id()));
        match create_remote_private_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "failed to create private herdr ssh config directory",
    ))
}

pub(crate) fn create_remote_ssh_config_file(
    path: &std::path::Path,
) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

pub(crate) fn create_remote_private_dir(path: &std::path::Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

pub(crate) fn remote_private_temp_base() -> PathBuf {
    std::env::temp_dir()
}

pub(crate) fn remote_bridge_endpoint_path(readable_name: &str, _short_name: &str) -> PathBuf {
    std::env::temp_dir().join(readable_name)
}

pub(crate) fn remote_reattach_program(program: &str) -> String {
    shell_quote(program)
}

pub(crate) fn remote_reattach_argument(value: &str) -> String {
    shell_quote(value)
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value.chars().all(|ch| {
            ch.is_ascii_alphanumeric()
                || matches!(
                    ch,
                    '@' | '%' | '_' | '+' | '=' | ':' | ',' | '.' | '/' | '-'
                )
        })
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Unsupported platform stub.
pub fn raise_server_nofile_limit() {}

pub(crate) fn should_draw_host_cursor_by_default() -> bool {
    false
}

pub(crate) fn should_query_host_terminal_palette() -> bool {
    false
}

pub(crate) fn hostname() -> Option<String> {
    None
}

pub(crate) fn local_datetime() -> Option<time::PrimitiveDateTime> {
    None
}

pub(crate) fn status_commands_supported() -> bool {
    false
}

pub(crate) fn configure_status_command(_process: &mut std::process::Command) {}

pub(crate) struct StatusCommandGuard;

impl StatusCommandGuard {
    pub(crate) fn new(_child: &tokio::process::Child) -> std::io::Result<Self> {
        Ok(Self)
    }

    pub(crate) fn terminate(&mut self) {}
}

fn raw_command_argv(command: &str, flag: &str) -> Vec<std::ffi::OsString> {
    vec!["/bin/sh".into(), flag.into(), command.into()]
}

pub(crate) fn detached_custom_command_process_platform(command: &str) -> std::process::Command {
    let argv = raw_command_argv(command, "-lc");
    let mut command = std::process::Command::new(&argv[0]);
    command.args(&argv[1..]);
    command
}

pub(crate) fn orcha_shell_command_platform(command: &str) -> std::process::Command {
    let mut process = Command::new("sh");
    process.arg("-c").arg(command);
    process
}

pub(crate) fn expand_orcha_home_command_platform(command: &str) -> String {
    let Some(rest) = command.strip_prefix("~/") else {
        return command.to_owned();
    };
    let Some(home) = user_home_dir_platform() else {
        return command.to_owned();
    };
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let path = home.join(&rest[..end]);
    format!("{}{}", shell_quote(&path.to_string_lossy()), &rest[end..])
}

pub(crate) fn orcha_tool_command_platform(program: &std::path::Path, args: &[String]) -> Command {
    let mut process = Command::new(program);
    process.args(args);
    process
}

pub(crate) fn orcha_installer_command_platform(
    unix_command: &str,
    _windows_command: &str,
) -> Command {
    let mut process = Command::new("sh");
    process.arg("-c").arg(unix_command);
    process
}

pub(crate) fn orcha_installer_instruction_platform(
    unix_command: &'static str,
    _windows_command: &'static str,
) -> &'static str {
    unix_command
}

pub(crate) fn open_orcha_target_platform(
    _target: super::OrchaOpenTarget,
    path: &std::path::Path,
    _line: u32,
) -> Command {
    let mut process = Command::new("xdg-open");
    process.arg(path);
    process
}

pub(crate) fn user_home_dir_platform() -> Option<PathBuf> {
    std::env::var_os("HOME").map(Into::into)
}

pub(crate) fn orcha_config_base_dir_platform() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(dir.into());
    }
    user_home_dir_platform().map(|home| home.join(".config"))
}

pub(crate) fn orcha_application_support_dir_platform(_home: &std::path::Path) -> PathBuf {
    orcha_config_base_dir_platform().unwrap_or_else(std::env::temp_dir)
}

pub(crate) fn orcha_secret_read(slot: &super::OrchaSecretSlot) -> Option<String> {
    super::orcha_secret_file_read(slot)
}

/// Claude reads `.credentials.json` in its config folder on this host;
/// there is no account attribute to carry over.
pub(crate) fn orcha_secret_write(
    slot: &super::OrchaSecretSlot,
    secret: &str,
    _account_from: &super::OrchaSecretSlot,
) -> std::io::Result<()> {
    super::orcha_secret_file_write(slot, secret)
}

pub(crate) fn orcha_cursor_token() -> Option<String> {
    super::orcha_cursor_token_file()
}

pub(crate) fn orcha_process_env_var(_pid: u32, _name: &str) -> Option<String> {
    None
}

pub(crate) fn orcha_link_path(
    source: &std::path::Path,
    target: &std::path::Path,
) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, target)
    }
    #[cfg(not(unix))]
    {
        let _ = (source, target);
        Ok(())
    }
}

pub(crate) fn pick_project_folder() -> std::io::Result<Option<std::path::PathBuf>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "folder dialogs are unavailable on this platform",
    ))
}

pub(crate) fn orcha_installed_apps_platform(_names: &[&str]) -> Option<Vec<(String, String)>> {
    None
}

pub(crate) fn orcha_config_tool_candidates_platform() -> &'static [&'static str] {
    &["orcha"]
}

pub(crate) fn remove_orcha_installation_platform() -> std::io::Result<super::OrchaUninstallResult> {
    Ok(super::OrchaUninstallResult::Unsupported)
}

pub(crate) fn orcha_uninstall_intro_platform() -> &'static str {
    "This removes orcha and its settings."
}

pub(crate) fn local_file_uri_path_platform(uri: &str) -> Option<String> {
    uri.strip_prefix("file://")
        .filter(|path| path.starts_with('/'))
        .map(str::to_owned)
}

pub(crate) fn pane_custom_command_pty_builder_platform(
    command: &str,
) -> portable_pty::CommandBuilder {
    portable_pty::CommandBuilder::from_argv(raw_command_argv(command, "-c"))
}

pub(crate) fn interactive_shell_command(_argv: &[String], _shell_name: &str) -> Option<String> {
    None
}

/// Unsupported platform stub.
pub(crate) fn scrollback_editor_argv(_path: &std::path::Path) -> std::io::Result<Vec<String>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "opening scrollback in an editor is not supported on this platform",
    ))
}

/// Unsupported platform stub.
pub fn detach_server_daemon_command(_command: &mut Command) {}

/// Unsupported platform stub.
pub fn current_process_is_detached_server_daemon() -> bool {
    false
}

pub(crate) fn available_pane_shell(_child_pid: u32) -> Option<String> {
    None
}

/// Unsupported platform stub.
pub fn foreground_job(_child_pid: u32) -> Option<ForegroundJob> {
    None
}

/// Unsupported platform stub.
pub fn foreground_group_leader_job(_process_group_id: u32) -> Option<ForegroundJob> {
    None
}

/// Unsupported platform stub.
pub fn foreground_process_group_id(_child_pid: u32) -> Option<u32> {
    None
}

/// Unsupported platform stub.
pub fn process_cwd(_pid: u32) -> Option<PathBuf> {
    None
}

/// Unsupported platform stub.
pub fn session_processes(_child_pid: u32) -> Vec<u32> {
    Vec::new()
}

/// Unsupported platform stub.
pub fn signal_processes(_pids: &[u32], _signal: Signal) {}

/// Unsupported platform stub.
pub fn process_start_token(_pid: u32) -> Option<u64> {
    None
}

/// Unsupported platform stub.
pub fn live_pane_process_group(_shell_pid: u32, _pid: u32, _start_token: u64) -> Option<u32> {
    None
}

/// Unsupported platform stub.
pub fn process_exists(_pid: u32) -> bool {
    false
}

/// Unsupported platform stub.
pub fn write_clipboard(_bytes: &[u8]) -> bool {
    false
}

/// Unsupported platform stub.
pub fn read_clipboard_text() -> Option<String> {
    None
}

/// Unsupported platform stub.
pub fn clipboard_text_matches(_bytes: &[u8]) -> Option<bool> {
    None
}

/// Unsupported platform stub.
pub fn open_url(_url: &str) -> std::io::Result<Option<std::process::Child>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "opening URLs is not supported on this platform",
    ))
}

/// Unsupported platform stub.
pub fn read_clipboard_image() -> Option<ClipboardImage> {
    None
}

/// Unsupported platform stub.
pub fn show_desktop_notification(_title: &str, _body: Option<&str>) -> std::io::Result<bool> {
    Ok(false)
}
