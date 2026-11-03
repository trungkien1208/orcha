//! orcha: the standalone app identity (docs/orcha/SPEC.md §3.11).
//!
//! The same binary is orcha when it is run as `orcha` (or with `ORCHA_APP=1`):
//! its config and sessions live in its platform config folder instead of herdr's, the
//! first run imports herdr's config and workspaces once, and the orcha-config
//! commands (`orcha setup`, `orcha menu`, …) are handed to that tool.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub(crate) const ORCHA_APP_ENV_VAR: &str = "ORCHA_APP";
/// orcha's own version; its GitHub releases are tagged `orcha-<version>`.
pub(crate) const ORCHA_VERSION: &str = "0.13.0";
const RELEASES_API: &str = "https://api.github.com/repos/trungkien1208/orcha/releases/latest";
/// The first-install line on Unix (see install.sh), shown when an update
/// fails. `orcha update` itself runs the release's own checked installer.
const INSTALL_COMMAND_UNIX: &str =
    "curl -fsSL https://raw.githubusercontent.com/trungkien1208/orcha/HEAD/install.sh | sh";
/// The first-install line on Windows (see install.ps1).
const INSTALL_COMMAND_WINDOWS: &str = "Invoke-Expression ((Invoke-WebRequest -UseBasicParsing -Uri 'https://raw.githubusercontent.com/trungkien1208/orcha/HEAD/install.ps1').Content)";
/// The newest release seen and its notes, checked at most once an hour.
const RELEASE_CACHE: &str = ".latest-release.json";
/// How long the newest release seen holds before GitHub is asked again; a
/// running app asks again this often too. GitHub allows 60 calls an hour.
pub(crate) const RELEASE_CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(60 * 60);
/// A release the user chose to skip: no reminder until a newer one.
const SKIPPED_RELEASE: &str = ".skipped-release";
/// The release the update popup already offered.
const OFFERED_RELEASE: &str = ".offered-release";
/// The version whose "what's new" was shown.
const SEEN_VERSION: &str = ".seen-version";
/// At most this many highlights from a release's notes.
const MAX_NOTES: usize = 4;
const IMPORT_MARKER: &str = ".imported-from-herdr";
/// orcha-config's own commands, handed to its CLI.
const CONFIG_TOOL_COMMANDS: [&str; 6] = ["setup", "menu", "edit", "logos", "plugin", "apply"];

/// Serialize read/modify/write operations across settings and account processes.
/// Lock a stable sibling, since replacing the data file changes its inode.
/// Keep the returned file alive until the atomic replacement has completed.
pub(crate) fn lock_for_update(path: &Path) -> std::io::Result<std::fs::File> {
    let target = crate::persist::resolve_write_target(path)?;
    let name = target.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing config file name")
    })?;
    let folder = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(folder)?;
    let lock = folder.join(format!(".{}.orcha-lock", name.to_string_lossy()));
    let file = match crate::platform::create_private_state_file(&lock) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            // a planted link must not make us open (and lock) some other file
            if !std::fs::symlink_metadata(&lock)?.file_type().is_file() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("{} is not a plain file", lock.display()),
                ));
            }
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(lock)?
        }
        Err(err) => return Err(err),
    };
    file.lock()?;
    Ok(file)
}

/// Writes `text` to `path` through a temporary file in the same folder, so a
/// reader never sees half of it and a failed write leaves the old file whole.
/// A symlink keeps its link: the file it points at gets the new text.
///
/// The file keeps the old one's permissions; a new file is private (0600).
/// Private is the default on purpose: account profiles, fuel and the
/// default-account choice go through here as well as config files, and an
/// owner-only config.toml in the user's own config folder costs nothing.
/// Contents and the rename are flushed to disk before this returns.
pub(crate) fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let target = crate::persist::resolve_write_target(path)?;
    let name = target.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{} is not a file", target.display()),
        )
    })?;
    let folder = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(folder)?;
    let tmp = folder.join(format!(
        ".{}.orcha-tmp-{}-{}",
        name.to_string_lossy(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    // Create exclusively and privately: do not follow a stale temporary
    // symlink or expose an account profile before permissions are copied.
    let mut file = crate::platform::create_private_state_file(&tmp)?;
    use std::io::Write;
    let written = file
        .write_all(text.as_bytes())
        .and_then(|()| match std::fs::metadata(&target) {
            // the new file keeps the old one's permissions
            Ok(old) => file.set_permissions(old.permissions()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        })
        // a rename can reach the disk before the contents do: flush them first
        .and_then(|()| file.sync_all());
    drop(file);
    let written = written.and_then(|()| crate::platform::replace_file(&tmp, &target));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return written;
    }
    // The new text is in place either way; a folder that cannot be flushed
    // only weakens crash safety, so it is logged rather than reported as
    // a failed save.
    if let Err(err) = crate::platform::sync_parent_directory(folder) {
        tracing::warn!(%err, path = %target.display(), "could not flush the folder after saving");
    }
    Ok(())
}

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

/// Subcommands of herdr's own `plugin` command (herdr-plugin.toml plugins).
pub(crate) fn is_herdr_plugin_subcommand(sub: &str) -> bool {
    matches!(
        sub,
        "link"
            | "unlink"
            | "list"
            | "enable"
            | "disable"
            | "install"
            | "uninstall"
            | "update"
            | "config-dir"
            | "action"
            | "log"
            | "logs"
            | "pane"
            | "help"
            | "--help"
            | "-h"
    )
}

/// The platform's user config root, respecting explicit XDG overrides.
fn config_base() -> Option<PathBuf> {
    crate::platform::orcha_config_base_dir()
}

/// orcha-config's CLI: the config folder's `orcha/scripts` links into its
/// checkout. orcha-config shares that folder in a release (a dev build keeps
/// its app config separate as `orcha-dev`).
pub(crate) fn config_tool() -> Option<PathBuf> {
    let scripts = config_base()?.join("orcha").join("scripts");
    let checkout = std::fs::canonicalize(scripts).ok()?.parent()?.to_path_buf();
    let bin = checkout.join("bin");
    crate::platform::orcha_config_tool_candidates()
        .iter()
        .map(|name| bin.join(name))
        .find(|tool| tool.is_file())
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
        // install.sh's Easy Job question (internal)
        "__offer-easy-job" => return Some(installer_offer(args.get(2).map(String::as_str))),
        "uninstall" => return Some(uninstall()),
        "office" => return Some(crate::orcha_office::run(&args[2..])),
        "settings" => return Some(crate::client::orcha_settings_web::run(&args[2..])),
        "channel" => {
            eprintln!("orcha has one channel: `orcha update` installs the latest release.");
            return Some(2);
        }
        _ => {}
    }
    if !CONFIG_TOOL_COMMANDS.contains(&command) {
        return None;
    }
    if command == "plugin"
        && args
            .get(2)
            .is_some_and(|sub| is_herdr_plugin_subcommand(sub))
    {
        // `orcha plugin link <dir>` and friends manage herdr-plugin.toml
        // plugins; bare `orcha plugin` still installs the Claude Code plugin.
        return None;
    }
    let Some(tool) = config_tool() else {
        eprintln!(
            "orcha {command}: orcha-config is not installed (github.com/trungkien1208/orcha-config)"
        );
        return Some(2);
    };
    let status = crate::platform::orcha_tool_command(&tool, &args[1..]).status();
    Some(status.ok().and_then(|status| status.code()).unwrap_or(1))
}

#[cfg(test)]
mod plugin_routing_tests {
    use super::*;

    #[test]
    fn the_offer_asks_until_installed_or_declined() {
        assert!(should_offer("easy-job", &[], None));
        assert!(should_offer("easy-job", &["other"], Some("0.0.1")));
        assert!(!should_offer("easy-job", &["easy-job"], None));
        assert!(!should_offer("easy-job", &[], Some("never")));
        assert!(!should_offer("easy-job", &[], Some(ORCHA_VERSION)));
    }

    #[test]
    fn a_failed_install_reports_its_last_line() {
        assert_eq!(
            last_line(b"Building...\nfetch.sh: release v0.2.0 not found\n\n").as_deref(),
            Some("fetch.sh: release v0.2.0 not found")
        );
        assert_eq!(last_line(b"  \n"), None);
    }

    #[test]
    fn easy_job_installs_from_its_plugin_branch() {
        assert_eq!(
            EASY_JOB_INSTALL.join(" "),
            "plugin install trungkien1208/orcha-plugin-office --ref plugin --yes"
        );
    }

    #[test]
    fn plugin_subcommands_go_to_herdr_and_bare_plugin_to_orcha_config() {
        for sub in ["link", "unlink", "list", "action", "pane"] {
            assert!(super::is_herdr_plugin_subcommand(sub), "{sub}");
        }
        assert!(!super::is_herdr_plugin_subcommand("marketplace"));
    }
}

#[cfg(test)]
mod release_tests {
    #[test]
    fn highlights_take_bold_leads_and_first_sentences() {
        let body = "**Install**\n\n```sh\ncurl x | sh\n```\n\n**New**\n- **Open files your way**: Ctrl+click a folder.\n- Paths that did nothing now open. More words here.\n- See [the docs](https://example.com) for `config`\n- **Four**\n- **Five**\n";
        assert_eq!(
            super::release_highlights(body),
            vec![
                "Open files your way",
                "Paths that did nothing now open",
                "See the docs for config",
                "Four",
            ]
        );
    }
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

/// An orcha release and the highlights of its notes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Release {
    /// "0.1.3"
    pub(crate) version: String,
    pub(crate) notes: Vec<String>,
}

/// The highlights of release notes (markdown): the bullets, each cut to its
/// bold lead or first sentence, links and emphasis removed.
pub(crate) fn release_highlights(body: &str) -> Vec<String> {
    let mut notes = Vec::new();
    for line in body.lines() {
        let Some(item) = line.trim().strip_prefix("- ") else {
            continue;
        };
        let lead = match item
            .strip_prefix("**")
            .and_then(|rest| rest.split_once("**"))
        {
            Some((bold, _)) => bold.to_owned(),
            None => item
                .split_once(". ")
                .map_or(item, |(sentence, _)| sentence)
                .to_owned(),
        };
        let text = strip_markdown(&lead);
        let text = text.trim().trim_end_matches(['.', ':']).trim();
        if !text.is_empty() {
            notes.push(text.to_owned());
        }
        if notes.len() == MAX_NOTES {
            break;
        }
    }
    notes
}

/// `[text](url)` → text; `**`, `` ` `` and `_` emphasis dropped.
fn strip_markdown(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after
            .split_once("](")
            .and_then(|(label, tail)| tail.split_once(')').map(|(_, tail)| (label, tail)))
        {
            Some((label, tail)) => {
                out.push_str(label);
                rest = tail;
            }
            None => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.replace("**", "").replace('`', "")
}

/// The newest release seen, and when (`RELEASE_CACHE`).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct CachedRelease {
    at: u64,
    version: String,
    #[serde(default)]
    notes: Vec<String>,
}

impl CachedRelease {
    fn release(&self) -> Release {
        Release {
            version: self.version.clone(),
            notes: self.notes.clone(),
        }
    }
}

fn cached_release() -> Option<CachedRelease> {
    crate::orcha_state::load_or_default(&crate::config::config_dir().join(RELEASE_CACHE))
}

/// The newest release (from the hourly cache, else GitHub). Blocking.
fn latest_release() -> Option<Release> {
    let cache = crate::config::config_dir().join(RELEASE_CACHE);
    let now = crate::orcha_state::now_secs();
    let cached = cached_release();
    if let Some(cached) = cached.as_ref() {
        if now.saturating_sub(cached.at) < RELEASE_CHECK_EVERY.as_secs() {
            return Some(cached.release());
        }
    }
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
        .filter(|output| output.status.success());
    let Some(output) = output else {
        // offline: the last answer still holds
        return cached.as_ref().map(CachedRelease::release);
    };
    let release: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let tag = release["tag_name"].as_str()?;
    let found = CachedRelease {
        at: now,
        version: tag.trim_start_matches("orcha-").to_owned(),
        notes: release_highlights(release["body"].as_str().unwrap_or_default()),
    };
    if let Err(err) = crate::orcha_state::save(&cache, &found) {
        tracing::warn!(%err, "could not remember the newest release");
    }
    Some(found.release())
}

/// A release newer than this build that the user has not skipped. Asks
/// GitHub at most once an hour. Blocking: call it off the input path.
pub(crate) fn newer_release() -> Option<Release> {
    let release = latest_release()?;
    let skipped = std::fs::read_to_string(crate::config::config_dir().join(SKIPPED_RELEASE))
        .unwrap_or_default();
    (version_parts(&release.version)? > version_parts(ORCHA_VERSION)?
        && skipped.trim() != release.version)
        .then_some(release)
}

/// True the first time a release is offered: the popup asks once per
/// version, after that the header button is the reminder.
pub(crate) fn first_offer(version: &str) -> bool {
    let path = crate::config::config_dir().join(OFFERED_RELEASE);
    let offered = std::fs::read_to_string(&path).unwrap_or_default();
    if offered.trim() == version {
        return false;
    }
    let _ = std::fs::write(&path, format!("{version}\n"));
    true
}

/// A plugin Orcha can offer to add, never without the user's consent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OfferedPlugin {
    pub id: &'static str,
    pub name: &'static str,
    /// Arguments for Orcha's own executable that install the plugin.
    pub install: &'static [&'static str],
    /// The pane that can start with Orcha, and the tokens that hide it.
    pub autostart: Option<(&'static str, &'static [(&'static str, &'static str)])>,
}

/// Installs Easy Job from its GitHub `plugin` branch.
pub(crate) const EASY_JOB_INSTALL: &[&str] = &[
    "plugin",
    "install",
    "trungkien1208/orcha-plugin-office",
    "--ref",
    "plugin",
    "--yes",
];

const OFFERED_PLUGINS: &[OfferedPlugin] = &[OfferedPlugin {
    id: "easy-job",
    name: "Easy Job",
    install: EASY_JOB_INSTALL,
    autostart: Some(("server", &[("easy_job", "bridge")])),
}];

/// Machines the offered plugins publish builds for.
const PLUGIN_OFFER_PLATFORM: bool = cfg!(any(target_os = "macos", target_os = "linux"));

const PLUGIN_OFFERS: &str = ".plugin-offers.json";

type PluginOffers = std::collections::HashMap<String, String>;

fn plugin_offers() -> PluginOffers {
    crate::orcha_state::load_or_default(&crate::config::config_dir().join(PLUGIN_OFFERS))
}

/// Forgets the answer, so the offer asks again next time (the install failed).
pub(crate) fn forget_plugin_offer(id: &str) {
    let path = crate::config::config_dir().join(PLUGIN_OFFERS);
    if let Err(err) = crate::orcha_state::update(&path, |offers: &mut PluginOffers| {
        offers.remove(id);
    }) {
        tracing::warn!(%err, "could not forget the plugin offer's answer");
    }
}

/// Remembers the answer: `never`, or the Orcha version a "not now" was given
/// in (the next update asks again).
pub(crate) fn answer_plugin_offer(id: &str, never: bool) {
    let answer = if never { "never" } else { ORCHA_VERSION };
    let path = crate::config::config_dir().join(PLUGIN_OFFERS);
    if let Err(err) = crate::orcha_state::update(&path, |offers: &mut PluginOffers| {
        offers.insert(id.to_owned(), answer.to_owned());
    }) {
        tracing::warn!(%err, "could not remember the plugin offer's answer");
    }
}

/// A plugin to offer now: not installed yet, not declined (for good, or in
/// this version), on a machine it has builds for. Blocking: run off the
/// input path.
pub(crate) fn plugin_to_offer() -> Option<OfferedPlugin> {
    let offers = plugin_offers();
    let linked = crate::orcha_handoff::herdr_json(&["api", "call", "plugin.list"])?;
    let linked: Vec<&str> = linked
        .get("plugins")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|plugin| plugin.get("plugin_id").or(plugin.get("id"))?.as_str())
        .collect();
    if !PLUGIN_OFFER_PLATFORM {
        return None;
    }
    OFFERED_PLUGINS
        .iter()
        .find(|plugin| {
            should_offer(
                plugin.id,
                &linked,
                offers.get(plugin.id).map(String::as_str),
            )
        })
        .cloned()
}

fn should_offer(id: &str, installed: &[&str], answered: Option<&str>) -> bool {
    !installed.contains(&id) && answered != Some("never") && answered != Some(ORCHA_VERSION)
}

/// Runs Orcha's own executable with `args` and no terminal. On failure,
/// returns the last line it printed (the plugin log line), else a summary.
pub(crate) fn run_self_quietly(args: &[&str]) -> Result<(), String> {
    let output = crate::orcha_cli::OrchaCli::new(args)
        .output()
        .map_err(|err| format!("could not run orcha: {err}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(last_line(&output.stderr)
        .or_else(|| last_line(&output.stdout))
        .unwrap_or_else(|| format!("orcha {} failed", args.join(" "))))
}

fn last_line(bytes: &[u8]) -> Option<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .map(str::to_owned)
}

/// install.sh asks about Easy Job through this: `pending` exits 0 when the
/// question is still open (not answered, not installed), `yes` installs it
/// and `no` declines; both answers keep the in-app offer from asking again.
fn installer_offer(answer: Option<&str>) -> i32 {
    let id = "easy-job";
    match answer {
        Some("pending") => {
            let installed = crate::persist::plugin_registry::try_load()
                .map(|plugins| plugins.iter().any(|plugin| plugin.plugin_id == id))
                .unwrap_or(false);
            let offers = plugin_offers();
            i32::from(!PLUGIN_OFFER_PLATFORM || installed || offers.contains_key(id))
        }
        Some("yes") => {
            answer_plugin_offer(id, false);
            let ok = crate::orcha_cli::succeeded(
                crate::orcha_cli::OrchaCli::new(EASY_JOB_INSTALL).in_terminal(),
            );
            if !ok {
                // the in-app offer asks again
                forget_plugin_offer(id);
                eprintln!("Easy Job was not added. Try later with: orcha office install");
                return 1;
            }
            0
        }
        Some("no") => {
            answer_plugin_offer(id, true);
            0
        }
        _ => 2,
    }
}

/// Installs an offered plugin, then (when asked) starts its pane with
/// Orcha. Blocking: run off the input path.
pub(crate) fn add_offered_plugin(plugin: &OfferedPlugin, autostart: bool) -> Result<(), String> {
    run_self_quietly(plugin.install)
        .map_err(|line| format!("{} was not added: {line}", plugin.name))?;
    if let (true, Some((entrypoint, tokens))) = (autostart, plugin.autostart) {
        let tokens: serde_json::Map<String, serde_json::Value> = tokens
            .iter()
            .map(|(key, value)| ((*key).to_owned(), serde_json::Value::from(*value)))
            .collect();
        let params = serde_json::json!({
            "plugin_id": plugin.id,
            "entrypoint": entrypoint,
            "enabled": true,
            "tokens": tokens,
        })
        .to_string();
        crate::orcha_handoff::herdr_json(&["api", "call", "plugin.autostart.set", &params])
            .ok_or("the plugin was added, but could not start with Orcha")?;
    }
    Ok(())
}

/// No reminder for this release; a newer one asks again.
pub(crate) fn skip_release(version: &str) {
    let _ = std::fs::write(
        crate::config::config_dir().join(SKIPPED_RELEASE),
        format!("{version}\n"),
    );
}

/// This build's version is the one already introduced (or a fresh install,
/// which needs no "what's new").
pub(crate) fn mark_version_seen() {
    let _ = std::fs::write(
        crate::config::config_dir().join(SEEN_VERSION),
        format!("{ORCHA_VERSION}\n"),
    );
}

/// What changed in this build, once: the first start after an update shows
/// the release's highlights. They come from the release cached when the
/// update was offered, else from GitHub. Blocking: call it off the input
/// path. Offline, it asks again on the next start.
pub(crate) fn whats_new() -> Option<Release> {
    let seen_path = crate::config::config_dir().join(SEEN_VERSION);
    let seen = std::fs::read_to_string(&seen_path).ok();
    if seen.as_deref().map(str::trim) == Some(ORCHA_VERSION) {
        return None;
    }
    let cached = cached_release()
        .filter(|cached| cached.version == ORCHA_VERSION)
        .map(|cached| cached.notes);
    let notes = match cached {
        Some(notes) => notes,
        None => {
            let url = format!(
                "https://api.github.com/repos/trungkien1208/orcha/releases/tags/orcha-{ORCHA_VERSION}"
            );
            let output = crate::noninteractive_process::curl_command()
                .args(["-sfL", "--connect-timeout", "5", "--max-time", "10", &url])
                .output()
                .ok()
                .filter(|output| output.status.success())?;
            let release: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
            release_highlights(release["body"].as_str().unwrap_or_default())
        }
    };
    mark_version_seen();
    (!notes.is_empty()).then(|| Release {
        version: ORCHA_VERSION.to_owned(),
        notes,
    })
}

/// Runs the newest release's installer (the app and extras), checked
/// against the release's checksums first. `quiet` keeps its output off the
/// screen (the app runs it behind its own UI).
fn install_latest(quiet: bool) -> Result<(), String> {
    crate::orcha_installer::install_latest(quiet)?;
    // the next check asks GitHub again
    let _ = std::fs::remove_file(crate::config::config_dir().join(RELEASE_CACHE));
    Ok(())
}

/// `install_latest` for the app's update button.
pub(crate) fn run_installer(quiet: bool) -> bool {
    install_latest(quiet)
        .inspect_err(|err| tracing::warn!(%err, "orcha update failed"))
        .is_ok()
}

fn update_from_cli() -> i32 {
    if let Err(err) = install_latest(false) {
        let instruction = crate::platform::orcha_installer_instruction(
            INSTALL_COMMAND_UNIX,
            INSTALL_COMMAND_WINDOWS,
        );
        eprintln!("orcha update failed: {err}\nTry again, or run:\n  {instruction}");
        return 1;
    }
    crate::orcha_plugin_updates::follow_self_update();
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
    let home = crate::platform::user_home_dir().unwrap_or_default();
    println!("{}", crate::platform::orcha_uninstall_intro());
    println!("Agents running inside orcha are stopped.");
    if !ask("Uninstall orcha?") {
        println!("Nothing changed.");
        return 1;
    }
    // what could not be removed, told at the end
    let mut left: Vec<String> = Vec::new();
    let remove = |left: &mut Vec<String>, path: &Path, removed: std::io::Result<()>| match removed {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => left.push(format!("{} ({err})", path.display())),
    };
    let me = std::env::current_exe().ok();
    // fails when no server runs, which is fine
    let _ = crate::orcha_cli::OrchaCli::new(&["server", "stop"]).quiet();
    if let Some(tool) = config_tool() {
        let args = ["uninstall".to_owned(), "--all".to_owned()];
        match crate::platform::orcha_tool_command(&tool, &args).status() {
            Ok(status) if status.success() => {}
            Ok(status) => left.push(format!(
                "what orcha-config set up (its uninstall stopped: {status})"
            )),
            Err(err) => left.push(format!(
                "what orcha-config set up (its uninstall did not run: {err})"
            )),
        }
        // the extras' folder, when the installer made it
        let installed = std::env::var_os("ORCHA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".orcha"));
        if let (Some(checkout), Ok(installed)) = (
            tool.parent().and_then(Path::parent),
            std::fs::canonicalize(&installed),
        ) {
            if checkout == installed {
                remove(&mut left, checkout, std::fs::remove_dir_all(checkout));
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
            remove(&mut left, folder, std::fs::remove_dir_all(folder));
        }
    }
    if let Some(me) = me.filter(|me| me.starts_with(home.join(".local/bin"))) {
        remove(&mut left, &me, std::fs::remove_file(&me));
    }
    match crate::platform::remove_orcha_installation() {
        #[cfg(not(windows))]
        Ok(crate::platform::OrchaUninstallResult::Unsupported) => {}
        #[cfg(windows)]
        Ok(crate::platform::OrchaUninstallResult::NotInstalled) => println!(
            "No managed Windows installation was found; this executable was left in place."
        ),
        #[cfg(windows)]
        Ok(crate::platform::OrchaUninstallResult::Scheduled(path)) => {
            println!("Removing {} after this process exits.", path.display());
        }
        Err(err) => left.push(format!("the managed installation ({err})")),
    }
    if !left.is_empty() {
        eprintln!("orcha could not remove everything. Still there:");
        for item in &left {
            eprintln!("  {item}");
        }
        eprintln!("Delete these by hand, or run orcha uninstall again.");
        return 1;
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
/// over once. Returns what was imported, for the startup notice. When the
/// config cannot be saved nothing is marked done, so the next start tries
/// again.
pub(crate) fn import_from_herdr_once() -> Result<Option<String>, String> {
    if !is_orcha() {
        return Ok(None);
    }
    let dir = crate::config::config_dir();
    let marker = dir.join(IMPORT_MARKER);
    if marker.exists() || dir.join("config.toml").exists() {
        return Ok(None);
    }
    let legacy = legacy_dir();
    let Ok(config) = std::fs::read_to_string(legacy.join("config.toml")) else {
        return Ok(None);
    };
    let target = dir.join("config.toml");
    write_atomic(&target, &import_config(&config))
        .map_err(|err| format!("could not save {}: {err}", target.display()))?;
    let mut imported = vec!["config"];
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
    Ok(Some(summary))
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

    #[cfg(unix)]
    #[test]
    fn atomic_writes_keep_private_permissions_and_preserve_existing_modes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("orcha-atomic-mode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("test folder");
        let path = dir.join("profile.json");
        write_atomic(&path, "first").expect("new private file");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        write_atomic(&path, "second").expect("replace existing file");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn atomic_writes_create_the_folder_and_leave_no_temporary_files() {
        let dir = std::env::temp_dir().join(format!(
            "orcha-atomic-new-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("nested").join("config.toml");
        write_atomic(&path, "a = 1\n").expect("new file in a new folder");
        write_atomic(&path, "a = 2\n").expect("replace it");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = 2\n");
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_lock_file_is_refused_and_its_target_left_alone() {
        let dir = std::env::temp_dir().join(format!("orcha-lock-link-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("test folder");
        let path = dir.join("config.toml");
        // an honest lock works, twice in a row
        drop(lock_for_update(&path).expect("first lock"));
        drop(lock_for_update(&path).expect("the lock file is reused"));
        let lock = dir.join(".config.toml.orcha-lock");
        std::fs::remove_file(&lock).unwrap();
        let elsewhere = dir.join("elsewhere");
        std::fs::write(&elsewhere, "keep").unwrap();
        std::os::unix::fs::symlink(&elsewhere, &lock).unwrap();
        assert!(lock_for_update(&path).is_err());
        assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "keep");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn atomic_writes_refuse_a_symlink_loop() {
        let dir = std::env::temp_dir().join(format!("orcha-atomic-loop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("test folder");
        let (one, two) = (dir.join("one.toml"), dir.join("two.toml"));
        std::os::unix::fs::symlink(&two, &one).unwrap();
        std::os::unix::fs::symlink(&one, &two).unwrap();
        assert!(write_atomic(&one, "x").is_err());
        assert!(lock_for_update(&one).is_err());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
