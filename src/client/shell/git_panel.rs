//! orcha: the right panel. Git facts about the focused pane's folder: branch,
//! ahead/behind, changed files and worktrees.
//!
//! Git runs on a background thread at most every few seconds (or when the
//! focused folder changes); rendering only reads the last result, so nothing
//! here touches the filesystem from view computation. The panel reads the
//! local filesystem, so it is only fed for the Local endpoint.

use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use super::render::put_text;
use super::*;

const REFRESH: Duration = Duration::from_secs(3);
const MAX_FILES: usize = 200;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct GitPanelData {
    pub root: String,
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    /// (porcelain status code, path relative to the root)
    pub files: Vec<(String, String)>,
    /// (branch or detached label, is the focused checkout)
    pub worktrees: Vec<(String, bool)>,
}

#[derive(Default)]
pub(super) struct GitPanelPoller {
    cwd: Option<String>,
    data: Option<GitPanelData>,
    pending: Option<Receiver<Option<GitPanelData>>>,
    next_refresh: Option<Instant>,
}

impl GitPanelPoller {
    pub(super) fn data(&self) -> Option<&GitPanelData> {
        self.data.as_ref()
    }

    #[cfg(test)]
    pub(super) fn set_for_test(&mut self, data: Option<GitPanelData>) {
        self.data = data;
    }

    /// Collects a finished refresh and starts the next one when due. Returns
    /// true when the panel content changed and needs a repaint.
    pub(super) fn tick(&mut self, cwd: Option<&str>, now: Instant) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(data) => {
                    self.pending = None;
                    if data != self.data {
                        self.data = data;
                        changed = true;
                    }
                }
                Err(TryRecvError::Disconnected) => self.pending = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        let moved = cwd != self.cwd.as_deref();
        if moved {
            self.cwd = cwd.map(str::to_owned);
            if self.cwd.is_none() && self.data.take().is_some() {
                changed = true;
            }
        }
        let due = moved || self.next_refresh.is_none_or(|at| now >= at);
        if due && self.pending.is_none() {
            if let Some(cwd) = self.cwd.clone() {
                let (tx, rx) = mpsc::channel();
                let spawned = crate::thread_spawn::spawn_named("orcha-git-panel", move || {
                    let _ = tx.send(read_git(Path::new(&cwd)));
                });
                if spawned.is_ok() {
                    self.pending = Some(rx);
                }
            }
            self.next_refresh = Some(now + REFRESH);
        }
        changed
    }
}

impl ClientShellState {
    /// Called from the client timer: feeds the git panel the focused pane's
    /// folder (Local endpoint only; a remote pane's path is not on this disk).
    pub(crate) fn tick_git_panel(&mut self, now: Instant) -> bool {
        let wanted = self.config.right_panel && self.active_endpoint_id == ClientEndpointId::Local;
        let cwd = wanted
            .then(|| {
                let snapshot = self.snapshot.as_deref()?;
                let focused = snapshot.focused_pane_id.as_deref()?;
                let pane = snapshot.panes.iter().find(|pane| pane.pane_id == focused)?;
                pane.foreground_cwd.clone().or_else(|| pane.cwd.clone())
            })
            .flatten();
        self.git_panel.tick(cwd.as_deref(), now)
    }
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn read_git(cwd: &Path) -> Option<GitPanelData> {
    let root = git(cwd, &["rev-parse", "--show-toplevel"])?
        .trim()
        .to_owned();
    let root_path = Path::new(&root);
    let status = git(
        root_path,
        &[
            "status",
            "--porcelain=v1",
            "--branch",
            "--untracked-files=all",
        ],
    )?;
    let mut data = GitPanelData {
        root: root.clone(),
        ..GitPanelData::default()
    };
    for line in status.lines() {
        if let Some(head) = line.strip_prefix("## ") {
            parse_branch_line(head, &mut data);
        } else if line.len() > 3 && data.files.len() < MAX_FILES {
            let code = line[..2].trim().to_owned();
            let path = line[3..].rsplit(" -> ").next().unwrap_or(&line[3..]);
            data.files.push((code, path.trim_matches('"').to_owned()));
        }
    }
    if let Some(list) = git(root_path, &["worktree", "list", "--porcelain"]) {
        let mut path = String::new();
        for line in list.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = p.to_owned();
            } else if let Some(branch) = line.strip_prefix("branch ") {
                let name = branch.trim_start_matches("refs/heads/").to_owned();
                data.worktrees.push((name, path == root));
            } else if line == "detached" {
                let name = Path::new(&path)
                    .file_name()
                    .map(|n| format!("{} (detached)", n.to_string_lossy()))
                    .unwrap_or_else(|| "(detached)".to_owned());
                data.worktrees.push((name, path == root));
            }
        }
    }
    Some(data)
}

/// "main...origin/main [ahead 1, behind 2]", "No commits yet on main", "HEAD (no branch)"
fn parse_branch_line(head: &str, data: &mut GitPanelData) {
    let head = head.trim_start_matches("No commits yet on ");
    let (names, counts) = match head.split_once(" [") {
        Some((names, counts)) => (names, counts.trim_end_matches(']')),
        None => (head, ""),
    };
    match names.split_once("...") {
        Some((branch, upstream)) => {
            data.branch = branch.to_owned();
            data.upstream = Some(upstream.to_owned());
        }
        None => data.branch = names.to_owned(),
    }
    for part in counts.split(", ") {
        if let Some(n) = part.strip_prefix("ahead ") {
            data.ahead = n.parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            data.behind = n.parse().unwrap_or(0);
        }
    }
}

/// Keeps the end of a path (the file name) visible: "…/components/Menu.tsx".
fn clip_left(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_owned();
    }
    let keep = width.saturating_sub(1);
    let tail: String = text.chars().skip(count - keep).collect();
    format!("…{tail}")
}

fn clip_right(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub(super) fn render_git_panel(
    buffer: &mut Buffer,
    area: Rect,
    data: Option<&GitPanelData>,
    palette: &Palette,
) {
    if area.width < 4 || area.height == 0 {
        return;
    }
    let bg = palette.sidebar_bg;
    buffer.set_style(area, Style::default().bg(bg));
    // a thin divider against the panes
    for y in area.y..area.bottom() {
        put_text(
            buffer,
            area.x,
            y,
            1,
            "│",
            Style::default().fg(palette.surface1).bg(bg),
        );
    }
    let x = area.x.saturating_add(2);
    let width = area.width.saturating_sub(3);
    let w = usize::from(width);
    let mut y = area.y;
    let line = |buffer: &mut Buffer, text: &str, style: Style, y: &mut u16| {
        if *y < area.bottom() {
            put_text(buffer, x, *y, width, text, style.bg(bg));
            *y += 1;
        }
    };
    let muted = Style::default().fg(palette.overlay0);
    let heading = Style::default()
        .fg(palette.overlay1)
        .add_modifier(Modifier::BOLD);

    let Some(data) = data else {
        y += 1;
        line(buffer, "Not a git repository", muted, &mut y);
        return;
    };

    y += 1;
    let branch = if data.branch.is_empty() {
        "(no branch)"
    } else {
        &data.branch
    };
    line(
        buffer,
        &format!("⎇ {}", clip_right(branch, w.saturating_sub(2))),
        Style::default()
            .fg(palette.accent)
            .add_modifier(Modifier::BOLD),
        &mut y,
    );
    let sync = match (&data.upstream, data.ahead, data.behind) {
        (None, _, _) => "not pushed".to_owned(),
        (Some(_), 0, 0) => "✓ in sync".to_owned(),
        (Some(_), a, 0) => format!("↑{a} to push"),
        (Some(_), 0, b) => format!("↓{b} to pull"),
        (Some(_), a, b) => format!("↑{a} ↓{b}"),
    };
    let sync_color = if data.upstream.is_none() || data.ahead > 0 {
        palette.yellow
    } else {
        palette.overlay1
    };
    line(buffer, &sync, Style::default().fg(sync_color), &mut y);

    y += 1;
    let count = data.files.len();
    line(
        buffer,
        &if count == 0 {
            "Changes".to_owned()
        } else {
            format!("Changes  {count}")
        },
        heading,
        &mut y,
    );
    if count == 0 {
        line(buffer, "clean", muted, &mut y);
    }
    // leave room for the worktree list under the changes
    let worktree_rows = data.worktrees.len().min(6) + 2;
    let room = usize::from(area.bottom().saturating_sub(y)).saturating_sub(worktree_rows);
    for (index, (code, path)) in data.files.iter().enumerate() {
        if index >= room.max(1) {
            break;
        }
        if index + 1 == room && count > room {
            line(buffer, &format!("… {} more", count - index), muted, &mut y);
            break;
        }
        let (mark, color) = match code.chars().next().unwrap_or(' ') {
            '?' => ("+", palette.green),
            'A' => ("+", palette.green),
            'D' => ("−", palette.red),
            'R' => ("→", palette.blue),
            'U' => ("!", palette.red),
            _ => ("●", palette.yellow),
        };
        if y >= area.bottom() {
            break;
        }
        put_text(buffer, x, y, 2, mark, Style::default().fg(color).bg(bg));
        put_text(
            buffer,
            x + 2,
            y,
            width.saturating_sub(2),
            &clip_left(path, w.saturating_sub(2)),
            Style::default().fg(palette.subtext0).bg(bg),
        );
        y += 1;
    }

    if data.worktrees.len() > 1 {
        y += 1;
        line(buffer, "Worktrees", heading, &mut y);
        for (name, current) in data.worktrees.iter().take(6) {
            let style = if *current {
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.subtext0)
            };
            let text = format!("{} {}", if *current { "●" } else { "○" }, name);
            line(buffer, &clip_right(&text, w), style, &mut y);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_line_with_upstream_and_counts() {
        let mut data = GitPanelData::default();
        parse_branch_line(
            "feature/42...origin/feature/42 [ahead 2, behind 1]",
            &mut data,
        );
        assert_eq!(data.branch, "feature/42");
        assert_eq!(data.upstream.as_deref(), Some("origin/feature/42"));
        assert_eq!((data.ahead, data.behind), (2, 1));
    }

    #[test]
    fn branch_line_without_upstream() {
        let mut data = GitPanelData::default();
        parse_branch_line("No commits yet on main", &mut data);
        assert_eq!(data.branch, "main");
        assert_eq!(data.upstream, None);
    }

    #[test]
    fn long_paths_keep_the_file_name() {
        assert_eq!(
            clip_left("src/components/settings/Menu.tsx", 12),
            "…gs/Menu.tsx"
        );
        assert_eq!(clip_left("a.ts", 12), "a.ts");
    }
}
