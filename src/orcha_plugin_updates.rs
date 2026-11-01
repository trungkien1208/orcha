//! Update check for plugins installed from GitHub (`orcha plugin install`).
//! Like Orcha's own release check it asks the network at most once a day
//! and keeps the answer in the config folder; no network means no news.
//! It runs plain `git` with the user's own credentials and never prompts.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::api::schema::{InstalledPluginInfo, PluginSourceKind};

const CACHE: &str = ".plugin-updates.json";
const CHECK_EVERY_SECS: u64 = 24 * 60 * 60;
const GIT_TIMEOUT: Duration = Duration::from_secs(15);
const MANIFEST: &str = "herdr-plugin.toml";

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

}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Cache {
    at: u64,
    plugins: Vec<PluginUpdate>,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

fn cache_path() -> std::path::PathBuf {
    crate::config::config_dir().join(CACHE)
}

fn read_cache() -> Cache {
    std::fs::read(cache_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_cache(cache: &Cache) {
    if let Ok(text) = serde_json::to_string_pretty(cache) {
        let _ = std::fs::write(cache_path(), text);
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
        .env("GCM_INTERACTIVE", "never")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut stdout, &mut text);
        text
    });
    let deadline = Instant::now() + GIT_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let text = reader.join().ok()?;
    status.success().then_some(text)
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
        let found = check_plugins(std::slice::from_ref(&plugin), &[], |_| Some(missing.clone()));
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
    fn refs_prefer_the_branch_then_the_peeled_tag() {
        let listed = "aaa\trefs/tags/v1\nbbb\trefs/tags/v1^{}\nccc\trefs/heads/plugin\n";
        assert_eq!(pick_ref(listed, "plugin").as_deref(), Some("ccc"));
        assert_eq!(pick_ref(listed, "v1").as_deref(), Some("bbb"));
        assert_eq!(pick_ref(listed, "0123abc"), None);
    }
}
