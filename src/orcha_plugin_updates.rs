//! Update check for plugins installed from GitHub (`orcha plugin install`).
//! Like Orcha's own release check it asks the network at most once a day
//! and keeps the answer in the config folder; no network means no news.
//! It runs plain `git` with the user's own credentials and never prompts.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::api::schema::{InstalledPluginInfo, PluginSourceKind};
use crate::orcha_state::now_secs;

const CACHE: &str = ".plugin-updates.json";
const CHECK_EVERY_SECS: u64 = 24 * 60 * 60;
const GIT_TIMEOUT: Duration = Duration::from_secs(15);
const MANIFEST: &str = "herdr-plugin.toml";
/// Versions whose notice was shown, per plugin.
const NOTICES: &str = ".plugin-update-notices.json";
/// Lines of a failed update's output kept for the error.
const LOG_TAIL_LINES: usize = 12;

/// What the last check knows about one GitHub plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PluginUpdate {
    pub id: String,
    pub current_version: String,
    /// The remote's version; the current one when nothing newer was found.
    pub latest_version: String,
    /// False when the newer version needs a newer Orcha.
    pub compatible: bool,
    /// The remote's `min_herdr_version`, when it is newer than this build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_herdr_version: Option<String>,
    /// `owner/repo[/subdir]`
    pub source: String,
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_commit: Option<String>,
}

impl PluginUpdate {
    /// The remote has a different commit than the installed one.
    pub(crate) fn available(&self) -> bool {
        self.latest_commit.is_some() && self.latest_commit != self.installed_commit
    }

    /// A newer commit this Orcha can run.
    pub(crate) fn installable(&self) -> bool {
        self.available() && self.compatible
    }

    /// The notice for this update, in the user's words.
    pub(crate) fn notice(&self, name: &str) -> String {
        match &self.needs_herdr_version {
            Some(required) if !self.compatible => format!(
                "{name} {} needs a newer Orcha (Herdr core {required}); run `orcha update`",
                self.latest_version
            ),
            _ => format!("{name} {} is out", self.latest_version),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Cache {
    at: u64,
    plugins: Vec<PluginUpdate>,
}

fn cache_path() -> std::path::PathBuf {
    crate::config::config_dir().join(CACHE)
}

fn read_cache() -> Cache {
    crate::orcha_state::load_or_default(&cache_path())
}

fn write_cache(cache: &Cache) {
    if let Err(err) = crate::orcha_state::save(&cache_path(), cache) {
        tracing::warn!(%err, "could not save the plugin update check");
    }
}

fn github_plugins() -> Vec<InstalledPluginInfo> {
    crate::persist::plugin_registry::try_load()
        .unwrap_or_default()
        .into_iter()
        .filter(|plugin| plugin.source.kind == PluginSourceKind::Github)
        .collect()
}

fn source_of(plugin: &InstalledPluginInfo) -> Option<String> {
    let owner = plugin.source.owner.as_deref()?;
    let repo = plugin.source.repo.as_deref()?;
    Some(match plugin.source.subdir.as_deref() {
        Some(subdir) => format!("{owner}/{repo}/{subdir}"),
        None => format!("{owner}/{repo}"),
    })
}

fn github_url(plugin: &InstalledPluginInfo) -> Option<String> {
    Some(format!(
        "https://github.com/{}/{}.git",
        plugin.source.owner.as_deref()?,
        plugin.source.repo.as_deref()?
    ))
}

/// What the client shows after a check: one notice at most, the first
/// time a version is seen.
pub(crate) struct PluginNews {
    pub updates: Vec<PluginUpdate>,
    /// An update to announce, with the plugin's name.
    pub notice: Option<(PluginUpdate, String)>,
    /// What the automatic update installed, to show once.
    pub updated: Option<String>,
}

/// Checks (at most daily) and picks the notice to show. With `automatic`
/// (the "update plugins automatically" setting), installs what this Orcha
/// can run instead, and the notice says what was updated. Blocking.
pub(crate) fn news(automatic: bool) -> PluginNews {
    let (mut updates, _) = check(false);
    let names: std::collections::HashMap<String, String> = github_plugins()
        .into_iter()
        .map(|plugin| (plugin.plugin_id, plugin.name))
        .collect();
    if automatic {
        let mut done = Vec::new();
        for update in updates.iter().filter(|update| update.installable()) {
            let name = names
                .get(&update.id)
                .cloned()
                .unwrap_or_else(|| update.id.clone());
            match update_plugin(&update.id) {
                Ok(version) => {
                    restart_running_panes(&update.id);
                    done.push(format!("{name} updated to {version}"));
                }
                Err(log) => tracing::warn!(id = %update.id, %log, "automatic plugin update failed"),
            }
        }
        if !done.is_empty() {
            updates = cached();
            return PluginNews {
                updates,
                notice: None,
                updated: Some(done.join(" · ")),
            };
        }
    }
    let notice = updates
        .iter()
        .filter(|update| update.available())
        .find(|update| first_notice(update))
        .map(|update| {
            let name = names
                .get(&update.id)
                .cloned()
                .unwrap_or_else(|| update.id.clone());
            (update.clone(), name)
        });
    PluginNews {
        updates,
        notice,
        updated: None,
    }
}

/// How one plugin fared in `orcha update`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FollowOutcome {
    Updated(String),
    Current,
    NeedsNewerOrcha,
    Failed(String),
}

impl FollowOutcome {
    pub(crate) fn line(&self, id: &str) -> String {
        match self {
            Self::Updated(version) => format!("  {id}: updated to {version}"),
            Self::Current => format!("  {id}: already current"),
            Self::NeedsNewerOrcha => format!("  {id}: needs a newer Orcha"),
            Self::Failed(reason) => format!("  {id}: failed ({reason})"),
        }
    }
}

/// After `orcha update`: updates every GitHub plugin with the new Orcha
/// (the executable on disk is the new one), printing one line each.
pub(crate) fn follow_self_update() {
    let plugins = github_plugins();
    if plugins.is_empty() {
        return;
    }
    println!("Plugins:");
    for plugin in plugins {
        let id = plugin.plugin_id;
        let outcome = run_update(&id);
        if matches!(outcome, FollowOutcome::Updated(_)) {
            restart_running_panes(&id);
        }
        println!("{}", outcome.line(&id));
    }
}

fn run_update(id: &str) -> FollowOutcome {
    let output = crate::orcha_cli::OrchaCli::new(&["plugin", "update", id, "--yes"]).output();
    let Ok(output) = output else {
        return FollowOutcome::Failed("could not run orcha plugin update".to_owned());
    };
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    let version = || {
        crate::persist::plugin_registry::try_load()
            .ok()
            .and_then(|plugins| plugins.into_iter().find(|plugin| plugin.plugin_id == id))
            .map(|plugin| plugin.version)
    };
    let outcome = classify_update(output.status.success(), &text, version);
    if matches!(outcome, FollowOutcome::Updated(_)) {
        forget(id);
    }
    outcome
}

fn classify_update(
    success: bool,
    output: &str,
    version: impl FnOnce() -> Option<String>,
) -> FollowOutcome {
    if success && output.contains("already up to date") {
        return FollowOutcome::Current;
    }
    if success {
        return FollowOutcome::Updated(version().unwrap_or_else(|| "the newest version".into()));
    }
    if output.contains("requires Herdr") || output.contains("plugin_requires_newer_herdr") {
        return FollowOutcome::NeedsNewerOrcha;
    }
    FollowOutcome::Failed(
        output
            .lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .unwrap_or("no output")
            .to_owned(),
    )
}

/// The last answer, matched to what is installed now; no network.
pub(crate) fn cached() -> Vec<PluginUpdate> {
    reconcile(&github_plugins(), read_cache().plugins)
}

/// True the first time the notice for `update` is shown; after that the
/// plugin list keeps the Update.
pub(crate) fn first_notice(update: &PluginUpdate) -> bool {
    let path = crate::config::config_dir().join(NOTICES);
    crate::orcha_state::update(
        &path,
        |shown: &mut std::collections::HashMap<String, String>| {
            shown.insert(update.id.clone(), update.latest_version.clone())
                != Some(update.latest_version.clone())
        },
    )
    .unwrap_or_else(|err| {
        // shown each time until the file is fixed, rather than never
        tracing::warn!(%err, "could not remember a plugin notice");
        true
    })
}

/// Updates a GitHub plugin with `orcha plugin update <id> --yes` and returns
/// its new version, or the tail of the update's output. Running panes keep
/// the old files until restarted (`restart_running_panes`). Blocking.
pub(crate) fn update_plugin(id: &str) -> Result<String, String> {
    let output = crate::orcha_cli::OrchaCli::new(&["plugin", "update", id, "--yes"])
        .output()
        .map_err(|err| format!("could not run orcha plugin update: {err}"))?;
    if !output.status.success() {
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        return Err(log_tail(&text));
    }
    forget(id);
    crate::persist::plugin_registry::try_load()
        .ok()
        .and_then(|plugins| plugins.into_iter().find(|plugin| plugin.plugin_id == id))
        .map(|plugin| plugin.version)
        .ok_or_else(|| format!("{id} is no longer installed"))
}

fn log_tail(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let tail = lines[lines.len().saturating_sub(LOG_TAIL_LINES)..].join("\n");
    if tail.is_empty() {
        "the update failed".to_owned()
    } else {
        tail
    }
}

/// Restarts `id`'s running panes through the session's API so they run the
/// updated files. Blocking.
pub(crate) fn restart_running_panes(id: &str) {
    let Some(listed) = crate::orcha_handoff::herdr_json(&["api", "call", "plugin.pane.list"])
    else {
        return;
    };
    let panes = plugin_pane_ids(&listed, id);
    for pane_id in &panes {
        let params = serde_json::json!({ "pane_id": pane_id }).to_string();
        let _ = crate::orcha_handoff::herdr_json(&["api", "call", "plugin.pane.restart", &params]);
    }
    if panes.is_empty() && id == crate::orcha_office::PLUGIN_ID {
        crate::orcha_office::restart_detached_office();
    }
}

/// The panes of `id` in a `plugin.pane.list` result.
pub(crate) fn plugin_pane_ids(listed: &serde_json::Value, id: &str) -> Vec<String> {
    listed["plugin_panes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|pane| pane["plugin_id"].as_str() == Some(id))
        .filter_map(|pane| pane["pane_id"].as_str().map(str::to_owned))
        .collect()
}

/// Forgets the cached answer for `id` (it was just updated).
fn forget(id: &str) {
    if let Err(err) = crate::orcha_state::update(&cache_path(), |cache: &mut Cache| {
        cache.plugins.retain(|plugin| plugin.id != id);
    }) {
        tracing::warn!(%err, "could not drop a plugin's update check");
    }
}

/// Asks the remotes when the last check is a day old (or `force`), else
/// returns the cached answer. Returns the answer and whether it has news
/// (an update that was not known before). Blocking: run off the input path.
pub(crate) fn check(force: bool) -> (Vec<PluginUpdate>, bool) {
    let installed = github_plugins();
    let cache = read_cache();
    let fresh = now_secs().saturating_sub(cache.at) < CHECK_EVERY_SECS;
    let known = reconcile(&installed, cache.plugins.clone());
    if !force && fresh && known.len() == installed.len() {
        return (known, false);
    }
    let found = check_plugins(&installed, &cache.plugins, github_url);
    let news = has_news(&cache.plugins, &found);
    write_cache(&Cache {
        at: now_secs(),
        plugins: found.clone(),
    });
    (found, news)
}

/// True when `found` has an update `previous` did not know about.
fn has_news(previous: &[PluginUpdate], found: &[PluginUpdate]) -> bool {
    found
        .iter()
        .filter(|plugin| plugin.available())
        .any(|plugin| {
            !previous
                .iter()
                .any(|old| old.id == plugin.id && old.latest_commit == plugin.latest_commit)
        })
}

/// Cached entries for the plugins installed now. An entry recorded for an
/// older installation is stale: the plugin is shown current until checked.
fn reconcile(installed: &[InstalledPluginInfo], cached: Vec<PluginUpdate>) -> Vec<PluginUpdate> {
    installed
        .iter()
        .filter_map(|plugin| {
            let current = current_entry(plugin)?;
            Some(
                cached
                    .iter()
                    .find(|entry| {
                        entry.id == plugin.plugin_id
                            && entry.installed_commit == current.installed_commit
                    })
                    .cloned()
                    .map(|mut entry| {
                        entry.current_version = current.current_version.clone();
                        entry
                    })
                    .unwrap_or(current),
            )
        })
        .collect()
}

/// A plugin with no news: its latest is what is installed.
fn current_entry(plugin: &InstalledPluginInfo) -> Option<PluginUpdate> {
    Some(PluginUpdate {
        id: plugin.plugin_id.clone(),
        current_version: plugin.version.clone(),
        latest_version: plugin.version.clone(),
        compatible: true,
        needs_herdr_version: None,
        source: source_of(plugin)?,
        reference: plugin.source.requested_ref.clone(),
        installed_commit: plugin.source.resolved_commit.clone(),
        latest_commit: plugin.source.resolved_commit.clone(),
    })
}

fn check_plugins(
    installed: &[InstalledPluginInfo],
    previous: &[PluginUpdate],
    url_of: impl Fn(&InstalledPluginInfo) -> Option<String>,
) -> Vec<PluginUpdate> {
    installed
        .iter()
        .filter_map(|plugin| {
            let mut entry = current_entry(plugin)?;
            let Some(url) = url_of(plugin) else {
                return Some(entry);
            };
            let reference = plugin.source.requested_ref.as_deref();
            let Some(remote) = remote_commit(&url, reference) else {
                // offline, or a pinned commit: keep what was known for this install
                return Some(
                    previous
                        .iter()
                        .find(|old| {
                            old.id == entry.id && old.installed_commit == entry.installed_commit
                        })
                        .cloned()
                        .unwrap_or(entry),
                );
            };
            if Some(&remote) == entry.installed_commit.as_ref() {
                return Some(entry);
            }
            let Some(manifest) = remote_manifest(&url, reference, plugin.source.subdir.as_deref())
            else {
                return Some(entry);
            };
            entry.latest_commit = Some(remote);
            entry.latest_version = manifest.version;
            if let Some(required) = manifest.min_herdr_version {
                let newer = crate::update::Version::parse(&required)
                    .is_some_and(|required| required > crate::update::Version::current());
                if newer {
                    entry.compatible = false;
                    entry.needs_herdr_version = Some(required);
                }
            }
            Some(entry)
        })
        .collect()
}

/// Runs git without a terminal or a credential prompt; its output on success.
fn git(args: &[&str], cwd: Option<&Path>) -> Option<String> {
    let mut command = crate::noninteractive_process::command("git");
    command
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never");
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    run_bounded(command, GIT_TIMEOUT)
}

/// How long the output may take to finish once the command is done.
const READ_GRACE: Duration = Duration::from_secs(2);

/// Runs `command` for at most `timeout` and returns its output when it
/// succeeded. It runs in its own process group (a job on Windows): when it
/// is done or out of time, whatever it left running is killed, so a helper
/// still holding the output cannot keep the read waiting. The read itself
/// gives up after `READ_GRACE`.
fn run_bounded(mut command: std::process::Command, timeout: Duration) -> Option<String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    crate::platform::configure_status_command(&mut command);
    let mut child = command.spawn().ok()?;
    let stop = |child: &mut std::process::Child| {
        let _ = child.kill();
        let _ = child.wait();
    };
    let mut tree = match crate::platform::StatusCommandGuard::for_child(&child) {
        Ok(tree) => tree,
        Err(err) => {
            tracing::warn!(%err, "could not watch a git command");
            stop(&mut child);
            return None;
        }
    };
    let Some(mut stdout) = child.stdout.take() else {
        tree.terminate();
        stop(&mut child);
        return None;
    };
    let (sender, received) = std::sync::mpsc::channel();
    let reader = std::thread::Builder::new()
        .name("orcha-git-output".to_owned())
        .spawn(move || {
            let mut text = String::new();
            let _ = std::io::Read::read_to_string(&mut stdout, &mut text);
            let _ = sender.send(text);
        });
    if reader.is_err() {
        tree.terminate();
        stop(&mut child);
        return None;
    }
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => break None,
        }
    };
    // done or out of time: nothing it started keeps running (or the output open)
    tree.terminate();
    if status.is_none() {
        stop(&mut child);
    }
    let text = received.recv_timeout(READ_GRACE).ok()?;
    status?.success().then_some(text)
}

/// The commit `reference` (a branch or tag; HEAD when none) points at, or
/// None when the remote cannot be reached or the reference is a commit.
fn remote_commit(url: &str, reference: Option<&str>) -> Option<String> {
    let reference = reference.unwrap_or("HEAD");
    let listed = git(&["ls-remote", url, reference], None)?;
    pick_ref(&listed, reference)
}

fn pick_ref(listed: &str, reference: &str) -> Option<String> {
    let lines: Vec<(&str, &str)> = listed
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .collect();
    let wanted = [
        reference.to_owned(),
        format!("refs/heads/{reference}"),
        format!("refs/tags/{reference}^{{}}"),
        format!("refs/tags/{reference}"),
    ];
    wanted
        .iter()
        .find_map(|name| lines.iter().find(|(_, found)| found == name))
        .map(|(commit, _)| (*commit).to_owned())
}

struct RemoteManifest {
    version: String,
    min_herdr_version: Option<String>,
}

/// The manifest at the tip of `reference`, fetched one commit deep.
fn remote_manifest(
    url: &str,
    reference: Option<&str>,
    subdir: Option<&str>,
) -> Option<RemoteManifest> {
    let dir = std::env::temp_dir().join(format!(
        "orcha-plugin-check-{}-{}",
        std::process::id(),
        now_secs()
    ));
    std::fs::create_dir_all(&dir).ok()?;
    let found = (|| {
        git(&["init", "-q"], Some(&dir))?;
        git(
            &[
                "fetch",
                "-q",
                "--depth",
                "1",
                url,
                reference.unwrap_or("HEAD"),
            ],
            Some(&dir),
        )?;
        let path = match subdir {
            Some(subdir) => format!("FETCH_HEAD:{}/{MANIFEST}", subdir.trim_matches('/')),
            None => format!("FETCH_HEAD:{MANIFEST}"),
        };
        parse_manifest(&git(&["show", &path], Some(&dir))?)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    found
}

fn parse_manifest(text: &str) -> Option<RemoteManifest> {
    let value: toml::Value = toml::from_str(text).ok()?;
    Some(RemoteManifest {
        version: value.get("version")?.as_str()?.to_owned(),
        min_herdr_version: value
            .get("min_herdr_version")
            .and_then(toml::Value::as_str)
            .map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder, removed when dropped.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "orcha-plugin-updates-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn run(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    /// A repo with a `plugin` branch holding a manifest at `version`.
    fn remote(dir: &Path, version: &str, min: &str) -> String {
        std::fs::create_dir_all(dir).unwrap();
        if !dir.join(".git").exists() {
            run(dir, &["init", "-q", "-b", "plugin"]);
        }
        std::fs::write(
            dir.join(MANIFEST),
            format!("id = \"easy-job\"\nversion = \"{version}\"\nmin_herdr_version = \"{min}\"\n"),
        )
        .unwrap();
        run(dir, &["add", "."]);
        run(dir, &["commit", "-q", "-m", version]);
        git(&["rev-parse", "HEAD"], Some(dir))
            .unwrap()
            .trim()
            .to_owned()
    }

    fn installed(commit: &str) -> InstalledPluginInfo {
        let mut plugin: InstalledPluginInfo = serde_json::from_value(serde_json::json!({
            "plugin_id": "easy-job",
            "name": "Easy Job",
            "version": "0.1.0",
            "min_herdr_version": "0.9.0",
            "manifest_path": "/x/herdr-plugin.toml",
            "plugin_root": "/x",
            "enabled": true,
        }))
        .unwrap();
        plugin.source.kind = PluginSourceKind::Github;
        plugin.source.owner = Some("trungkien1208".into());
        plugin.source.repo = Some("orcha-plugin-office".into());
        plugin.source.requested_ref = Some("plugin".into());
        plugin.source.resolved_commit = Some(commit.into());
        plugin
    }

    #[test]
    fn a_new_commit_reports_the_remote_version_and_compatibility() {
        let temp = TempDir::new("remote");
        let repo = temp.path().join("repo");
        let first = remote(&repo, "0.1.0", "0.9.0");
        let url = format!("file://{}", repo.display());
        let plugin = installed(&first);

        let same = check_plugins(std::slice::from_ref(&plugin), &[], |_| Some(url.clone()));
        assert!(!same[0].available());
        assert_eq!(same[0].latest_version, "0.1.0");
        assert_eq!(same[0].source, "trungkien1208/orcha-plugin-office");
        assert_eq!(same[0].reference.as_deref(), Some("plugin"));

        remote(&repo, "0.2.0", "0.9.0");
        let found = check_plugins(std::slice::from_ref(&plugin), &[], |_| Some(url.clone()));
        assert!(found[0].available() && found[0].compatible);
        assert_eq!(found[0].latest_version, "0.2.0");
        assert!(has_news(&same, &found));
        assert!(!has_news(&found, &found), "known once, news once");

        remote(&repo, "0.3.0", "99.0.0");
        let blocked = check_plugins(&[plugin], &[], |_| Some(url.clone()));
        assert!(blocked[0].available() && !blocked[0].compatible);
        assert_eq!(blocked[0].needs_herdr_version.as_deref(), Some("99.0.0"));
    }

    #[test]
    fn no_network_is_no_news() {
        let temp = TempDir::new("offline");
        let missing = format!("file://{}", temp.path().join("gone").display());
        let plugin = installed("abc");
        let found = check_plugins(std::slice::from_ref(&plugin), &[], |_| {
            Some(missing.clone())
        });
        assert!(!found[0].available());

        let mut known = found[0].clone();
        known.latest_commit = Some("def".into());
        known.latest_version = "0.2.0".into();
        let kept = check_plugins(&[plugin], &[known.clone()], |_| Some(missing.clone()));
        assert_eq!(kept, vec![known], "offline keeps the last answer");
    }

    #[test]
    fn a_cached_answer_for_an_older_install_is_dropped() {
        let mut stale = current_entry(&installed("old")).unwrap();
        stale.latest_commit = Some("new".into());
        let shown = reconcile(&[installed("new")], vec![stale]);
        assert!(!shown[0].available());
        assert_eq!(shown[0].installed_commit.as_deref(), Some("new"));
    }

    #[test]
    fn notices_point_to_orcha_update_when_orcha_is_too_old() {
        let mut update = current_entry(&installed("old")).unwrap();
        update.latest_commit = Some("new".into());
        update.latest_version = "0.2.0".into();
        assert_eq!(update.notice("Easy Job"), "Easy Job 0.2.0 is out");
        update.compatible = false;
        update.needs_herdr_version = Some("1.2.0".into());
        assert!(!update.installable());
        assert_eq!(
            update.notice("Easy Job"),
            "Easy Job 0.2.0 needs a newer Orcha (Herdr core 1.2.0); run `orcha update`"
        );
    }

    #[test]
    fn a_failed_update_keeps_the_end_of_its_log() {
        let text = (1..=20).map(|n| format!("line {n}\n")).collect::<String>();
        let tail = log_tail(&text);
        assert!(tail.starts_with("line 9") && tail.ends_with("line 20"));
        assert_eq!(log_tail("\n \n"), "the update failed");
    }

    #[test]
    fn restart_picks_only_the_plugins_panes() {
        let listed = serde_json::json!({"plugin_panes": [
            {"plugin_id": "easy-job", "pane_id": "w1:p1"},
            {"plugin_id": "other", "pane_id": "w1:p2"},
        ]});
        assert_eq!(plugin_pane_ids(&listed, "easy-job"), ["w1:p1"]);
    }

    #[test]
    fn orcha_update_reports_each_plugin_in_one_line() {
        let version = || Some("0.2.0".to_owned());
        assert_eq!(
            classify_update(true, "easy-job is already up to date.\n", version),
            FollowOutcome::Current
        );
        assert_eq!(
            classify_update(true, "Updated easy-job from x.\n", version),
            FollowOutcome::Updated("0.2.0".into())
        );
        assert_eq!(
            classify_update(
                false,
                "plugin requires Herdr 1.2.0 or newer; current Herdr is 0.9.5\n",
                version
            ),
            FollowOutcome::NeedsNewerOrcha
        );
        let failed = classify_update(false, "build...\nfetch.sh: no network\n", version);
        assert_eq!(failed, FollowOutcome::Failed("fetch.sh: no network".into()));
        assert_eq!(
            failed.line("easy-job"),
            "  easy-job: failed (fetch.sh: no network)"
        );
        assert_eq!(
            FollowOutcome::NeedsNewerOrcha.line("easy-job"),
            "  easy-job: needs a newer Orcha"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_helper_left_holding_the_output_does_not_hang_the_read() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "sleep 30 & echo hi"]);
        let started = Instant::now();
        assert_eq!(
            run_bounded(command, Duration::from_secs(10)).as_deref(),
            Some("hi\n")
        );
        assert!(started.elapsed() < READ_GRACE + Duration::from_secs(1));
    }

    #[cfg(unix)]
    #[test]
    fn a_command_past_its_deadline_is_stopped() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "echo partial; sleep 30"]);
        let started = Instant::now();
        assert_eq!(run_bounded(command, Duration::from_millis(300)), None);
        assert!(started.elapsed() < READ_GRACE + Duration::from_secs(1));
    }

    #[test]
    fn refs_prefer_the_branch_then_the_peeled_tag() {
        let listed = "aaa\trefs/tags/v1\nbbb\trefs/tags/v1^{}\nccc\trefs/heads/plugin\n";
        assert_eq!(pick_ref(listed, "plugin").as_deref(), Some("ccc"));
        assert_eq!(pick_ref(listed, "v1").as_deref(), Some("bbb"));
        assert_eq!(pick_ref(listed, "0123abc"), None);
    }
}
