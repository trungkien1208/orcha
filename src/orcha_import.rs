//! orcha: projects to bring in from the tools people used before
//! (docs/orcha/SPEC.md §3.11). Orca's projects, and the recent folders of
//! VS Code, Cursor, Windsurf, VSCodium and the JetBrains IDEs. Reading them
//! touches the disk (and runs `sqlite3`), so the client does it on a
//! background thread, once.

use std::path::{Path, PathBuf};

/// The import was offered already (taken or skipped): don't ask again.
const ASKED_MARKER: &str = ".import-asked";
/// Recent folders per editor: the newest ones, not years of history.
const RECENT_LIMIT: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportSource {
    /// "Orca", "VS Code", …
    pub name: String,
    /// (name, folder), newest or Orca's order first.
    pub projects: Vec<(String, String)>,
}

pub(crate) fn asked() -> bool {
    crate::config::config_dir().join(ASKED_MARKER).exists()
}

pub(crate) fn mark_asked() {
    let dir = crate::config::config_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(ASKED_MARKER), "the import popup was shown\n");
}

/// Every source with at least one folder still on disk.
pub(crate) fn find_sources() -> Vec<ImportSource> {
    let Some(home) = crate::platform::user_home_dir() else {
        return Vec::new();
    };
    let support = crate::platform::orcha_application_support_dir(&home);
    let mut sources = vec![ImportSource {
        name: "Orca".into(),
        projects: orca_projects(&home, &support),
    }];
    for (name, folder) in [
        ("VS Code", "Code"),
        ("Cursor", "Cursor"),
        ("Windsurf", "Windsurf"),
        ("VSCodium", "VSCodium"),
    ] {
        let db = support.join(folder).join("User/globalStorage/state.vscdb");
        sources.push(ImportSource {
            name: name.into(),
            projects: vscode_recent(&db),
        });
    }
    sources.push(ImportSource {
        name: "JetBrains".into(),
        projects: jetbrains_recent(&support.join("JetBrains"), &home),
    });
    let home_key = folder_key(&home.to_string_lossy());
    // each folder once, under the first source that has it
    let mut seen = std::collections::HashSet::new();
    for source in &mut sources {
        source.projects.retain(|(_, path)| {
            let key = folder_key(path);
            Path::new(path).is_dir() && key != home_key && seen.insert(key)
        });
    }
    sources.retain(|source| !source.projects.is_empty());
    sources
}

/// Folders compare case-insensitively where the usual file systems do.
const FOLDERS_IGNORE_CASE: bool = cfg!(any(target_os = "macos", windows));

/// One spelling per folder: links resolved, `/` separators, no trailing
/// separator, and lowercase where folder names ignore case. Two paths of
/// the same folder (`C:\Code\app`, `c:/code/app/`) get the same key.
pub(crate) fn folder_key(path: &str) -> String {
    let resolved = std::fs::canonicalize(path)
        .map(|resolved| resolved.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_owned());
    normalize_folder(&resolved, FOLDERS_IGNORE_CASE)
}

fn normalize_folder(path: &str, ignore_case: bool) -> String {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    let mut key = path.replace('\\', "/");
    while key.len() > 1 && key.ends_with('/') {
        key.pop();
    }
    if ignore_case {
        key = key.to_lowercase();
    }
    key
}

/// `path` is the folder of `folder_key` or inside it (keys from `folder_key`).
pub(crate) fn key_within(path_key: &str, folder_key: &str) -> bool {
    path_key == folder_key
        || path_key
            .strip_prefix(folder_key)
            .is_some_and(|rest| rest.starts_with('/') || folder_key.ends_with('/'))
}

fn folder_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

fn push_unique(out: &mut Vec<(String, String)>, name: Option<&str>, path: &str) {
    let path = path.trim_end_matches(['/', '\\']);
    if path.is_empty() || out.iter().any(|(_, seen)| seen == path) {
        return;
    }
    let name = name
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| folder_name(path));
    out.push((name, path.to_owned()));
}

fn orca_projects(home: &Path, support: &Path) -> Vec<(String, String)> {
    let roots = [support.join("orca"), home.join(".config/orca")];
    let Some(root) = roots.iter().find(|root| root.is_dir()) else {
        return Vec::new();
    };
    let profile = std::fs::read_to_string(root.join("orca-profile-index.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|index| index["activeProfileId"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "local-default".to_owned());
    std::fs::read_to_string(root.join("profiles").join(profile).join("orca-data.json"))
        .map(|text| parse_orca(&text))
        .unwrap_or_default()
}

/// Orca's repos, then their worktrees (keyed "<repo id>::<folder>").
fn parse_orca(text: &str) -> Vec<(String, String)> {
    let Ok(data) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for repo in data["repos"].as_array().into_iter().flatten() {
        push_unique(
            &mut out,
            repo["displayName"].as_str(),
            repo["path"].as_str().unwrap_or_default(),
        );
    }
    if let Some(worktrees) = data["worktreeMeta"].as_object() {
        let mut worktrees: Vec<_> = worktrees.iter().collect();
        worktrees.sort_by_key(|(_, meta)| meta["sortOrder"].as_i64().unwrap_or(i64::MAX));
        for (key, meta) in worktrees {
            if let Some((_, path)) = key.split_once("::") {
                push_unique(&mut out, meta["displayName"].as_str(), path);
            }
        }
    }
    out
}

/// A VS Code-family editor's recent folders, from its state database.
fn vscode_recent(db: &Path) -> Vec<(String, String)> {
    if !db.is_file() {
        return Vec::new();
    }
    let mut command = std::process::Command::new("sqlite3");
    command
        .arg("-readonly")
        .arg(db)
        .arg("select value from ItemTable where key = 'history.recentlyOpenedPathsList'")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // no console window flashing up on Windows
    crate::platform::configure_background_command(&mut command);
    let output = command.output();
    match output {
        Ok(output) if output.status.success() => {
            parse_vscode_recent(&String::from_utf8_lossy(&output.stdout))
        }
        _ => Vec::new(),
    }
}

fn parse_vscode_recent(text: &str) -> Vec<(String, String)> {
    let Ok(data) = serde_json::from_str::<serde_json::Value>(text.trim()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in data["entries"].as_array().into_iter().flatten() {
        // only local folders: not files, remotes or .code-workspace files
        let Some(path) = entry["folderUri"]
            .as_str()
            .map(percent_decode)
            .and_then(|uri| crate::platform::local_file_uri_path(&uri))
        else {
            continue;
        };
        push_unique(&mut out, None, &path);
        if out.len() == RECENT_LIMIT {
            break;
        }
    }
    out
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(&String::from_utf8_lossy(&bytes[index + 1..index + 3]), 16)
            {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Every JetBrains IDE's recent projects (newest IDE version first).
fn jetbrains_recent(root: &Path, home: &Path) -> Vec<(String, String)> {
    let Ok(products) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = products
        .flatten()
        .map(|entry| entry.path().join("options/recentProjects.xml"))
        .filter(|file| file.is_file())
        .collect();
    files.sort();
    files.reverse();
    let mut out = Vec::new();
    for file in files {
        if let Ok(text) = std::fs::read_to_string(file) {
            for (name, path) in parse_jetbrains_recent(&text, home) {
                push_unique(&mut out, Some(&name), &path);
            }
        }
    }
    out.truncate(RECENT_LIMIT);
    out
}

fn parse_jetbrains_recent(text: &str, home: &Path) -> Vec<(String, String)> {
    let home = home.to_string_lossy();
    let mut out = Vec::new();
    let Some(start) = text.find("name=\"additionalInfo\"") else {
        return out;
    };
    for chunk in text[start..].split("<entry key=\"").skip(1) {
        let Some(end) = chunk.find('"') else {
            continue;
        };
        let path = chunk[..end].replace("$USER_HOME$", &home);
        let name = folder_name(&path);
        push_unique(&mut out, Some(&name), &path);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orca_lists_repos_then_worktrees_once_each() {
        let text = r#"{"repos":[{"path":"/p/app","displayName":"App"},{"path":"/p/api","displayName":""}],
            "worktreeMeta":{"r1::/p/app":{"displayName":"app","sortOrder":1},
                            "r1::/w/app/fix":{"displayName":"fix login","sortOrder":0}}}"#;
        assert_eq!(
            parse_orca(text),
            vec![
                ("App".to_owned(), "/p/app".to_owned()),
                ("api".to_owned(), "/p/api".to_owned()),
                ("fix login".to_owned(), "/w/app/fix".to_owned()),
            ]
        );
        assert!(parse_orca("not json").is_empty());
    }

    // a Unix path is not a local folder on Windows
    #[cfg(unix)]
    #[test]
    fn vscode_keeps_local_folders_only() {
        let text = r#"{"entries":[{"folderUri":"file:///Users/me/my%20app"},
            {"fileUri":"file:///Users/me/notes.md"},
            {"folderUri":"vscode-remote://ssh-remote+box/srv"},
            {"workspace":{"configPath":"file:///x.code-workspace"}},
            {"folderUri":"file:///Users/me/api/"}]}"#;
        assert_eq!(
            parse_vscode_recent(text),
            vec![
                ("my app".to_owned(), "/Users/me/my app".to_owned()),
                ("api".to_owned(), "/Users/me/api".to_owned()),
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn vscode_keeps_local_windows_folders_only() {
        let text = r#"{"entries":[{"folderUri":"file:///c%3A/Users/me/my%20app"},
            {"fileUri":"file:///c%3A/Users/me/notes.md"},
            {"folderUri":"vscode-remote://ssh-remote+box/srv"},
            {"folderUri":"file:///Users/me/unix"}]}"#;
        assert_eq!(
            parse_vscode_recent(text),
            vec![("my app".to_owned(), "c:\\Users\\me\\my app".to_owned())]
        );
    }

    #[test]
    fn one_key_per_folder_whatever_the_spelling() {
        assert_eq!(
            normalize_folder(r"\\?\C:\Code\App\", true),
            normalize_folder("c:/code/app", true)
        );
        assert_eq!(normalize_folder("/Users/me/app/", false), "/Users/me/app");
        assert_ne!(
            normalize_folder("/p/App", false),
            normalize_folder("/p/app", false)
        );
        assert_eq!(normalize_folder("/", false), "/");
    }

    #[test]
    fn within_means_the_folder_or_below_it() {
        assert!(key_within("/p/app", "/p/app"));
        assert!(key_within("/p/app/src", "/p/app"));
        assert!(!key_within("/p/application", "/p/app"));
        assert!(key_within("/p/app", "/"));
    }

    #[test]
    fn jetbrains_reads_the_recent_entries() {
        let text = r#"<application><component name="RecentProjectsManager">
            <option name="additionalInfo"><map>
            <entry key="$USER_HOME$/work/shop"><value/></entry>
            <entry key="/opt/tools"><value/></entry>
            </map></option></component></application>"#;
        assert_eq!(
            parse_jetbrains_recent(text, Path::new("/Users/me")),
            vec![
                ("shop".to_owned(), "/Users/me/work/shop".to_owned()),
                ("tools".to_owned(), "/opt/tools".to_owned()),
            ]
        );
    }
}

#[cfg(test)]
mod live {
    /// `cargo test --bin herdr orcha_import::live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn print_sources() {
        for source in super::find_sources() {
            eprintln!("{}: {}", source.name, source.projects.len());
        }
    }
}
