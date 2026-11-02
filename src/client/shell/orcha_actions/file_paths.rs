//! File links, editor choices and the file viewer.

use super::*;

impl ClientShellState {
    /// Ctrl+click on a path in a pane: a folder opens in the file manager,
    /// a picture, PDF, web page or other document in the system's app for it,
    /// and code or text in the editor (at its `:line`). Relative paths are the pane's. Returns
    /// false when nothing exists there.
    pub(in crate::client::shell) fn open_path_link(&mut self, pane_id: &str, text: &str) -> bool {
        let (raw, line) = split_line(text);
        let home = crate::platform::user_home_dir()
            .map(|home| home.to_string_lossy().into_owned())
            .unwrap_or_default();
        let expanded = match raw.strip_prefix('~') {
            Some(rest) => format!("{home}{rest}"),
            None => raw.to_owned(),
        };
        let mut path = std::path::PathBuf::from(&expanded);
        if path.is_relative() {
            let Some(cwd) = self.pane_cwd(pane_id) else {
                return false;
            };
            path = std::path::Path::new(&cwd).join(path);
            // agents print paths from the repository root
            if !path.exists() {
                if let Some(root) = self.git_panel.data().map(|data| data.root.clone()) {
                    path = std::path::Path::new(&root).join(&expanded);
                }
            }
        }
        if !path.exists() {
            // the agent may print a path from its repository root, or a bare name
            match self.find_printed_path(pane_id, &expanded) {
                Some(found) => path = found,
                None => {
                    self.show_notice(format!("can't find {raw}"));
                    return true;
                }
            }
        }
        self.open_existing_path(&path, line);
        true
    }

    /// Opens a file or folder that exists: a folder in the file manager, a
    /// file in the app kept for its type, else the OPEN WITH choices.
    pub(in crate::client::shell) fn open_existing_path(
        &mut self,
        path: &std::path::Path,
        line: Option<u32>,
    ) {
        let shown = path.to_string_lossy().into_owned();
        let line = line.unwrap_or(1);
        if path.is_dir() {
            open_orcha_target(crate::platform::OrchaOpenTarget::SystemDefault, &shown, 1);
            return;
        }
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .filter(|extension| extension.chars().all(|c| c.is_ascii_alphanumeric()))
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        if let Some(choice) = self
            .config
            .orcha
            .open_with
            .get(&extension)
            .filter(|_| !extension.is_empty())
            .and_then(|value| super::super::orcha_popup::OpenChoice::from_config_value(value))
        {
            open_with_choice(&choice, &shown, line);
            return;
        }
        let choices = open_choices(path, &self.config.orcha.ide);
        self.orcha_ui.popup = Some(super::super::orcha_popup::OrchaPopup::OpenWith {
            selected: 0,
            path: shown,
            line,
            extension,
            choices,
            remember: true,
        });
    }

    /// FILES: a folder opens or closes, a file shows in the FILE popup.
    pub(super) fn click_file_row(&mut self, index: usize) {
        let Some(row) = self.file_tree.rows().get(index).cloned() else {
            return;
        };
        if row.is_dir {
            self.file_tree.toggle(&row.path);
            return;
        }
        let root = self.file_tree.root().map(std::path::Path::to_path_buf);
        self.orcha_ui.file_view = Some(super::super::file_viewer::FileView::open(
            row.path.clone(),
            root,
        ));
        self.orcha_ui.popup = Some(super::super::orcha_popup::OrchaPopup::FileView {
            path: row.path.to_string_lossy().into_owned(),
        });
    }

    /// Keys in the FILE popup: scroll, o opens the file in another app,
    /// esc closes.
    pub(super) fn route_file_view_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        use crossterm::event::KeyCode;
        outcome.repaint = true;
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
            self.orcha_ui.popup = None;
            self.orcha_ui.file_view = None;
            return;
        }
        if matches!(key.code, KeyCode::Char('o') | KeyCode::Char('O')) {
            let Some(view) = self.orcha_ui.file_view.take() else {
                return;
            };
            self.orcha_ui.popup = None;
            self.open_existing_path(&view.path, None);
            return;
        }
        let Some(view) = self.orcha_ui.file_view.as_mut() else {
            return;
        };
        let page = view.page();
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => view.scroll_by(1),
            KeyCode::Up | KeyCode::Char('k') => view.scroll_by(-1),
            KeyCode::PageDown | KeyCode::Char(' ') | KeyCode::Char('f') => view.scroll_by(page),
            KeyCode::PageUp | KeyCode::Char('b') => view.scroll_by(-page),
            KeyCode::Home | KeyCode::Char('g') => view.scroll = 0,
            KeyCode::End | KeyCode::Char('G') => view.scroll_to_end(),
            _ => {}
        }
    }

    pub(super) fn toggle_open_with_remember(&mut self) {
        if let Some(super::super::orcha_popup::OrchaPopup::OpenWith { remember, .. }) =
            self.orcha_ui.popup.as_mut()
        {
            *remember = !*remember;
        }
    }

    /// Keeps the app for `extension`, in memory and in `[orcha.open_with]`.
    pub(super) fn save_open_with(
        &mut self,
        extension: &str,
        choice: &super::super::orcha_popup::OpenChoice,
    ) {
        let value = choice.config_value();
        self.config
            .orcha
            .open_with
            .insert(extension.to_owned(), value.clone());
        let path = crate::config::config_path();
        let key = extension.to_owned();
        let _ = crate::thread_spawn::spawn_named("orcha-open-with", move || {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            let quoted = format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""));
            let updated =
                crate::config::upsert_section_value(&content, "orcha.open_with", &key, &quoted);
            if let Err(err) = std::fs::write(&path, updated) {
                tracing::warn!(err = %err, "orcha: failed to save how .{key} files open");
            }
        });
    }

    /// A printed path that isn't under the pane's folder: try the pane's
    /// repository root, then (for a bare file name) a short search of it.
    pub(super) fn find_printed_path(
        &self,
        pane_id: &str,
        printed: &str,
    ) -> Option<std::path::PathBuf> {
        let cwd = self.pane_cwd(pane_id)?;
        let root = std::path::Path::new(&cwd)
            .ancestors()
            .find(|dir| dir.join(".git").exists())?
            .to_path_buf();
        let candidate = root.join(printed);
        if candidate.exists() {
            return Some(candidate);
        }
        if printed.contains('/') || printed.contains('\\') {
            return None;
        }
        find_file_named(&root, printed)
    }
}

/// "src/a.ts:12:4" → ("src/a.ts", Some(12)).
pub(super) fn split_line(text: &str) -> (&str, Option<u32>) {
    let Some((before_last, last)) = text.rsplit_once(':') else {
        return (text, None);
    };
    let Ok(last_number) = last.parse::<u32>() else {
        return (text, None);
    };
    if let Some((path, line)) = before_last.rsplit_once(':') {
        if let Ok(line) = line.parse::<u32>() {
            return (path, Some(line));
        }
    }
    (before_last, Some(last_number))
}

/// Runs `command` with `$P` (and `{path}`) set to `path`, without waiting.
pub(in crate::client::shell) fn spawn_with_path(command: &str, path: &str) {
    spawn_with_env(command, path, 1);
}

pub(super) fn spawn_with_env(command: &str, path: &str, line: u32) {
    let script = command.replace("{path}", "\"$P\"");
    let mut process = crate::platform::orcha_shell_command(&script);
    process.env("P", path).env("L", line.to_string());
    spawn_quietly(process, path);
}

/// Starts `process` with no input or output and reaps it off the input
/// path; `what` names it in the log when it does not start.
pub(super) fn spawn_quietly(mut process: std::process::Command, what: &str) {
    process
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    match process.spawn() {
        Ok(mut child) => {
            let _ = crate::thread_spawn::spawn_named("orcha-reap", move || {
                let _ = child.wait();
            });
        }
        Err(err) => tracing::warn!(err = %err, "orcha: failed to start {what}"),
    }
}

/// Files an editor can't show well open in the system's app for their type:
/// pictures, PDFs, media, office documents, web pages, archives. Everything
/// else (code, text, config, unknown types) goes to the editor at its line.
pub(super) fn opens_in_system_app(path: &std::path::Path) -> bool {
    const SYSTEM_APP_EXTENSIONS: &[&str] = &[
        // pictures
        "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "ico", "heic", "avif",
        // documents
        "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "pages", "numbers",
        "key", "rtf", "epub", // web pages open in the browser
        "html", "htm", // media
        "mp4", "mov", "m4v", "avi", "mkv", "webm", "mp3", "wav", "m4a", "aac", "flac", "ogg",
        // archives and installers
        "zip", "tar", "gz", "tgz", "7z", "rar", "dmg", "pkg", "msi",
    ];
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            SYSTEM_APP_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
        })
}

/// Folders skipped when looking for a bare file name.
const SKIPPED_DIRS: &[&str] = &["node_modules", "target", "dist", "build", "vendor"];

/// Breadth-first search for `name` under `root`, shallowest match first,
/// bounded so a click never stalls on a huge tree.
pub(super) fn find_file_named(root: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    const MAX_ENTRIES: usize = 20_000;
    let mut seen = 0usize;
    let mut queue = std::collections::VecDeque::from([root.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut subdirs = Vec::new();
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES {
                return None;
            }
            let file_name = entry.file_name();
            let Some(file_name) = file_name.to_str() else {
                continue;
            };
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if !file_name.starts_with('.') && !SKIPPED_DIRS.contains(&file_name) {
                    subdirs.push(entry.path());
                }
            } else if file_name == name {
                return Some(entry.path());
            }
        }
        subdirs.sort();
        queue.extend(subdirs);
    }
    None
}

/// The apps offered for a file, the suggested one first: the system's app
/// for documents and pictures, the editor for code and text.
pub(super) fn open_choices(
    path: &std::path::Path,
    ide: &str,
) -> Vec<super::super::orcha_popup::OpenChoice> {
    use super::super::orcha_popup::OpenChoice;
    let mut editors = vec![OpenChoice::Editor {
        name: String::new(),
        command: if ide.trim().is_empty() {
            String::new()
        } else {
            ide.to_owned()
        },
    }];
    // Installed editors, at most four, besides the configured one.
    for (name, command) in super::super::settings::installed_apps(super::super::settings::IDE_APPS)
        .into_iter()
        .filter(|(_, command)| !command.is_empty() && command != ide)
        .take(4)
    {
        editors.push(OpenChoice::Editor { name, command });
    }
    let mut choices = Vec::new();
    if opens_in_system_app(path) {
        choices.push(OpenChoice::System);
        choices.extend(editors);
    } else {
        choices.extend(editors);
        choices.push(OpenChoice::System);
    }
    choices.push(OpenChoice::Reveal);
    choices
}

pub(super) fn open_with_choice(
    choice: &super::super::orcha_popup::OpenChoice,
    path: &str,
    line: u32,
) {
    use super::super::orcha_popup::OpenChoice;
    match choice {
        OpenChoice::System => {
            open_orcha_target(crate::platform::OrchaOpenTarget::SystemDefault, path, 1);
        }
        OpenChoice::Reveal => {
            let folder = std::path::Path::new(path)
                .parent()
                .map_or_else(|| path.to_owned(), |dir| dir.to_string_lossy().into_owned());
            open_orcha_target(crate::platform::OrchaOpenTarget::SystemDefault, &folder, 1);
        }
        OpenChoice::Editor { command, .. } if command.trim().is_empty() => {
            open_orcha_target(crate::platform::OrchaOpenTarget::File, path, line);
        }
        OpenChoice::Editor { command, .. } => spawn_with_env(command, path, line),
    }
}

pub(super) fn open_orcha_target(target: crate::platform::OrchaOpenTarget, path: &str, line: u32) {
    spawn_quietly(
        crate::platform::open_orcha_target(target, std::path::Path::new(path), line),
        path,
    );
}
