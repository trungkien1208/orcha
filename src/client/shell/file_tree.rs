//! orcha: the right panel's FILES section. The focused project's folders as
//! a tree; a click opens a folder or shows a file (`file_viewer`).
//!
//! Folders are listed on a background thread when the section opens, when a
//! folder is opened and every few seconds while the section stays open;
//! rendering only reads the last listing. Files git ignores are left out.
//! A folder whose modified time stands still is not listed again until
//! `RELIST` passes (what git ignores can change without touching it).

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant, SystemTime};

const REFRESH: Duration = Duration::from_secs(5);
/// The longest a listing stands on an unchanged folder.
const RELIST: Duration = Duration::from_secs(30);
/// A folder modified this close before its listing may have changed again
/// within the same tick of a coarse clock: it is listed again.
const RACY: Duration = Duration::from_secs(2);
/// Entries listed per folder; the rest are summed up in one row.
const MAX_ENTRIES: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    pub name: String,
    pub is_dir: bool,
}

/// One visible row of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TreeRow {
    pub depth: usize,
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub open: bool,
}

type Listings = HashMap<PathBuf, Vec<Entry>>;

/// A folder's modified time when it was listed, and when that was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    modified: SystemTime,
    listed: SystemTime,
}

impl Stamp {
    /// The listing still holds at `now` for a folder modified at `modified`.
    fn holds(&self, modified: SystemTime, now: SystemTime) -> bool {
        self.modified == modified
            && self
                .listed
                .duration_since(modified)
                .is_ok_and(|gap| gap >= RACY)
            && now
                .duration_since(self.listed)
                .is_ok_and(|age| age < RELIST)
    }
}

/// A finished listing: each folder read again, with its new stamp.
/// Folders left out still hold.
type Relisted = Vec<(PathBuf, Vec<Entry>, Option<Stamp>)>;

#[derive(Default)]
pub(super) struct FileTree {
    root: Option<PathBuf>,
    /// Opened folders under `root`.
    open: BTreeSet<PathBuf>,
    listings: Listings,
    stamps: HashMap<PathBuf, Stamp>,
    rows: Vec<TreeRow>,
    pending: Option<(PathBuf, Receiver<Relisted>)>,
    next_refresh: Option<Instant>,
}

impl FileTree {
    pub(super) fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub(super) fn rows(&self) -> &[TreeRow] {
        &self.rows
    }

    pub(super) fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// Follows `root` while `wanted` (the section is open), collects a
    /// finished listing and starts the next one when due. Returns true when
    /// the rows changed.
    pub(super) fn tick(&mut self, root: Option<&Path>, wanted: bool, now: Instant) -> bool {
        let mut changed = false;
        if root != self.root.as_deref() {
            self.root = root.map(Path::to_path_buf);
            self.open.clear();
            self.listings.clear();
            self.stamps.clear();
            self.pending = None;
            self.next_refresh = None;
            changed |= self.rebuild();
        }
        if let Some((for_root, rx)) = &self.pending {
            match rx.try_recv() {
                Ok(relisted) => {
                    if Some(for_root) == self.root.as_ref() && !relisted.is_empty() {
                        for (folder, entries, stamp) in relisted {
                            match stamp {
                                Some(stamp) => self.stamps.insert(folder.clone(), stamp),
                                None => self.stamps.remove(&folder),
                            };
                            self.listings.insert(folder, entries);
                        }
                        changed |= self.rebuild();
                    }
                    self.pending = None;
                    self.next_refresh = Some(now + REFRESH);
                }
                Err(TryRecvError::Disconnected) => self.pending = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        let due = self.next_refresh.is_none_or(|at| now >= at);
        if wanted && due && self.pending.is_none() {
            if let Some(root) = self.root.clone() {
                // a folder not listed yet has no stamp and is read for sure
                let folders: Vec<(PathBuf, Option<Stamp>)> = std::iter::once(&root)
                    .chain(self.open.iter())
                    .map(|folder| {
                        let stamp = self
                            .listings
                            .contains_key(folder)
                            .then(|| self.stamps.get(folder).copied())
                            .flatten();
                        (folder.clone(), stamp)
                    })
                    .collect();
                let (tx, rx) = mpsc::channel();
                let spawned = crate::thread_spawn::spawn_named("orcha-file-tree", move || {
                    let _ = tx.send(relist(folders));
                });
                if spawned.is_ok() {
                    self.pending = Some((root, rx));
                }
            }
            self.next_refresh = Some(now + REFRESH);
        }
        changed
    }

    /// Opens or closes a folder; an opened folder not listed yet is read on
    /// the next tick.
    pub(super) fn toggle(&mut self, path: &Path) {
        if !self.open.remove(path) {
            self.open.insert(path.to_path_buf());
            if !self.listings.contains_key(path) {
                self.next_refresh = None;
            }
        }
        self.rebuild();
    }

    fn rebuild(&mut self) -> bool {
        let mut rows = Vec::new();
        if let Some(root) = &self.root {
            push_rows(&mut rows, root, 0, &self.listings, &self.open);
        }
        let changed = rows != self.rows;
        self.rows = rows;
        changed
    }

    #[cfg(test)]
    pub(super) fn set_for_test(&mut self, root: &Path, listings: Listings, open: &[&Path]) {
        self.root = Some(root.to_path_buf());
        self.listings = listings;
        self.open = open.iter().map(|path| path.to_path_buf()).collect();
        self.rebuild();
    }
}

fn push_rows(
    rows: &mut Vec<TreeRow>,
    folder: &Path,
    depth: usize,
    listings: &Listings,
    open: &BTreeSet<PathBuf>,
) {
    let Some(entries) = listings.get(folder) else {
        return;
    };
    for entry in entries {
        let path = folder.join(&entry.name);
        let is_open = entry.is_dir && open.contains(&path);
        rows.push(TreeRow {
            depth,
            name: entry.name.clone(),
            path: path.clone(),
            is_dir: entry.is_dir,
            open: is_open,
        });
        if is_open {
            push_rows(rows, &path, depth + 1, listings, open);
        }
    }
}

/// Lists the folders again, but not one whose stamp still holds.
fn relist(folders: Vec<(PathBuf, Option<Stamp>)>) -> Relisted {
    folders
        .into_iter()
        .filter_map(|(folder, stamp)| {
            let now = SystemTime::now();
            let modified = std::fs::metadata(&folder)
                .and_then(|meta| meta.modified())
                .ok();
            if let (Some(stamp), Some(modified)) = (stamp, modified) {
                if stamp.holds(modified, now) {
                    return None;
                }
            }
            let entries = list_folder(&folder);
            let stamp = modified.map(|modified| Stamp {
                modified,
                listed: now,
            });
            Some((folder, entries, stamp))
        })
        .collect()
}

/// The folder's entries, folders first, without `.git` and what git ignores.
fn list_folder(folder: &Path) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut entries: Vec<Entry> = read
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".git" || name == ".DS_Store" {
                return None;
            }
            // the listing already knows the type; a link to a folder opens
            // like a folder, so only a link is followed
            let is_dir = match entry.file_type() {
                Ok(kind) if !kind.is_symlink() => kind.is_dir(),
                _ => std::fs::metadata(entry.path()).is_ok_and(|meta| meta.is_dir()),
            };
            Some(Entry { name, is_dir })
        })
        .collect();
    let ignored = git_ignored(folder, &entries);
    entries.retain(|entry| !ignored.contains(&entry.name));
    sort_entries(&mut entries);
    entries.truncate(MAX_ENTRIES);
    entries
}

fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

/// Names in `folder` that git ignores; none outside a repository.
fn git_ignored(folder: &Path, entries: &[Entry]) -> std::collections::HashSet<String> {
    let mut ignored = std::collections::HashSet::new();
    if entries.is_empty() {
        return ignored;
    }
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(folder)
        .args(["check-ignore", "--stdin", "-z"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    crate::platform::configure_background_command(&mut command);
    let Ok(mut child) = command.spawn() else {
        return ignored;
    };
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let mut input = Vec::new();
        for entry in entries {
            input.extend_from_slice(entry.name.as_bytes());
            input.push(0);
        }
        let _ = stdin.write_all(&input);
    }
    let Ok(output) = child.wait_with_output() else {
        return ignored;
    };
    for name in output.stdout.split(|byte| *byte == 0) {
        if !name.is_empty() {
            ignored.insert(String::from_utf8_lossy(name).into_owned());
        }
    }
    ignored
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool) -> Entry {
        Entry {
            name: name.into(),
            is_dir,
        }
    }

    #[test]
    fn folders_come_first_then_names_ignoring_case() {
        let mut entries = vec![
            entry("b.rs", false),
            entry("Zeta", true),
            entry("a.rs", false),
            entry("alpha", true),
        ];
        sort_entries(&mut entries);
        let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["alpha", "Zeta", "a.rs", "b.rs"]);
    }

    #[test]
    fn only_opened_folders_show_their_entries() {
        let root = PathBuf::from("/repo");
        let src = root.join("src");
        let docs = root.join("docs");
        let mut listings = Listings::new();
        listings.insert(
            root.clone(),
            vec![
                entry("docs", true),
                entry("src", true),
                entry("Cargo.toml", false),
            ],
        );
        listings.insert(src.clone(), vec![entry("main.rs", false)]);
        listings.insert(docs.clone(), vec![entry("guide.md", false)]);
        let mut tree = FileTree::default();
        tree.set_for_test(&root, listings, &[&src]);
        let shown: Vec<_> = tree
            .rows()
            .iter()
            .map(|row| (row.depth, row.name.as_str(), row.open))
            .collect();
        assert_eq!(
            shown,
            [
                (0, "docs", false),
                (0, "src", true),
                (1, "main.rs", false),
                (0, "Cargo.toml", false),
            ]
        );
        tree.toggle(&src);
        assert_eq!(tree.rows().len(), 3);
    }

    #[test]
    fn a_listing_holds_while_the_folder_stands_still() {
        let changed = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let stamp = Stamp {
            modified: changed,
            listed: changed + RACY,
        };
        let later = |secs| stamp.listed + Duration::from_secs(secs);
        assert!(stamp.holds(changed, later(1)));
        assert!(
            !stamp.holds(changed + Duration::from_secs(1), later(1)),
            "modified since"
        );
        assert!(
            !stamp.holds(changed, later(RELIST.as_secs())),
            "listed too long ago"
        );
        let racy = Stamp {
            modified: changed,
            listed: changed + Duration::from_millis(500),
        };
        assert!(
            !racy.holds(changed, later(1)),
            "listed right after a change: a coarse clock may hide the next one"
        );
    }

    #[test]
    fn a_folder_not_listed_yet_is_listed() {
        let root = std::env::temp_dir().join(format!("orcha-relist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("create src");
        std::fs::write(root.join("a.txt"), "a").expect("write file");
        let relisted = relist(vec![(root.clone(), None)]);
        assert_eq!(relisted.len(), 1);
        let (folder, entries, stamp) = &relisted[0];
        assert_eq!(folder, &root);
        assert_eq!(entries, &[entry("src", true), entry("a.txt", false)]);
        assert!(stamp.is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn listing_skips_git_and_ignored_files() {
        let root = std::env::temp_dir().join(format!("orcha-file-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("create src");
        std::fs::create_dir_all(root.join("target")).expect("create target");
        std::fs::write(root.join("README.md"), "hi").expect("write readme");
        std::fs::write(root.join(".gitignore"), "target/\n").expect("write gitignore");
        let init = Command::new("git")
            .arg("-C")
            .arg(&root)
            .arg("init")
            .output()
            .expect("run git init");
        assert!(init.status.success());
        let names: Vec<_> = list_folder(&root)
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, ["src", ".gitignore", "README.md"]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
