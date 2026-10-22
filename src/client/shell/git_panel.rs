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

use super::render::{put_right_text, put_text};
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

/// A section label like every orcha region uses: muted, bold, a note on the right.
fn section(buffer: &mut Buffer, area: Rect, y: u16, title: &str, note: &str, palette: &Palette) {
    let bg = palette.sidebar_bg;
    put_text(
        buffer,
        area.x + 2,
        y,
        area.width.saturating_sub(3),
        title,
        Style::default()
            .fg(palette.overlay0)
            .bg(bg)
            .add_modifier(Modifier::BOLD),
    );
    if !note.is_empty() {
        put_right_text(
            buffer,
            Rect::new(area.x, y, area.width.saturating_sub(2), 1),
            y,
            note,
            Style::default().fg(palette.overlay0).bg(bg),
        );
    }
}

pub(super) fn render_git_panel(
    buffer: &mut Buffer,
    area: Rect,
    data: Option<&GitPanelData>,
    palette: &Palette,
) {
    if area.width < 8 || area.height == 0 {
        return;
    }
    let bg = palette.sidebar_bg;
    buffer.set_style(area, Style::default().bg(bg));
    // the same thin divider the left sidebar has against the panes
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
    let x = area.x + 2;
    let width = area.width.saturating_sub(4);
    let w = usize::from(width);
    let mut y = area.y;
    let fits = |y: u16| y < area.bottom();

    let Some(data) = data else {
        section(buffer, area, y, "TRACKS", "", palette);
        put_text(
            buffer,
            x,
            y + 2,
            width,
            "Not a git repository",
            Style::default().fg(palette.overlay0).bg(bg),
        );
        return;
    };

    // TRACKS: the branch leaves main as a spur, one stop per commit to push
    let sync = match (&data.upstream, data.ahead, data.behind) {
        (None, _, _) => "not pushed".to_owned(),
        (Some(_), 0, 0) => "in sync".to_owned(),
        (Some(_), a, 0) => format!("↑{a} to push"),
        (Some(_), 0, b) => format!("↓{b} to pull"),
        (Some(_), a, b) => format!("↑{a} ↓{b}"),
    };
    section(buffer, area, y, "TRACKS", &sync, palette);
    y += 1;
    let branch = if data.branch.is_empty() {
        "(no branch)"
    } else {
        &data.branch
    };
    let trunk = matches!(branch, "main" | "master" | "develop");
    let muted = Style::default().fg(palette.overlay1).bg(bg);
    let rail = Style::default().fg(palette.accent).bg(bg);
    if trunk {
        if fits(y) {
            let main = format!(
                "{branch} {}●",
                "━".repeat(w.saturating_sub(branch.chars().count() + 2))
            );
            put_text(
                buffer,
                x,
                y,
                width,
                &main,
                rail.add_modifier(Modifier::BOLD),
            );
            y += 1;
        }
    } else {
        if fits(y) {
            let main = format!("main ━━┳{}", "━".repeat(w.saturating_sub(8)));
            put_text(buffer, x, y, width, &main, muted);
            y += 1;
        }
        if fits(y) {
            put_text(buffer, x, y, 7, "       ", muted);
            put_text(buffer, x + 7, y, 1, "┗", muted);
            let stops = data.ahead.min(5);
            let mut spur = String::from("━━");
            for _ in 0..stops {
                spur.push_str("━●");
            }
            spur.push_str(if data.upstream.is_none() || data.ahead > 0 {
                "━▶"
            } else {
                "━●"
            });
            put_text(buffer, x + 8, y, width.saturating_sub(8), &spur, rail);
            y += 1;
        }
        if fits(y) {
            put_text(
                buffer,
                x + 8,
                y,
                width.saturating_sub(8),
                &clip_right(branch, w.saturating_sub(8)),
                Style::default()
                    .fg(palette.accent)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            );
            y += 1;
        }
    }

    // CHANGES
    y += 1;
    let count = data.files.len();
    if fits(y) {
        section(
            buffer,
            area,
            y,
            "CHANGES",
            &if count == 0 {
                "clean".to_owned()
            } else {
                format!("{count} files")
            },
            palette,
        );
        y += 1;
    }
    // leave room for the depots under the changes
    let depot_rows = if data.worktrees.len() > 1 {
        data.worktrees.len().min(6) + 2
    } else {
        0
    };
    let room = usize::from(area.bottom().saturating_sub(y)).saturating_sub(depot_rows);
    for (index, (code, path)) in data.files.iter().enumerate() {
        if index >= room || !fits(y) {
            break;
        }
        if index + 1 == room && count > room {
            put_text(
                buffer,
                x,
                y,
                width,
                &format!("… {} more", count - index),
                Style::default().fg(palette.overlay0).bg(bg),
            );
            y += 1;
            break;
        }
        let (mark, color) = match code.chars().next().unwrap_or(' ') {
            '?' | 'A' => ("A", palette.green),
            'D' => ("D", palette.red),
            'R' => ("R", palette.blue),
            'U' => ("!", palette.red),
            _ => ("M", palette.yellow),
        };
        put_text(
            buffer,
            x,
            y,
            1,
            mark,
            Style::default()
                .fg(color)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        );
        let shown = clip_left(path, w.saturating_sub(2));
        let (dir, name) = match shown.rfind('/') {
            Some(cut) => shown.split_at(cut + 1),
            None => ("", shown.as_str()),
        };
        let dir_width = dir.chars().count() as u16;
        put_text(
            buffer,
            x + 2,
            y,
            dir_width,
            dir,
            Style::default().fg(palette.overlay0).bg(bg),
        );
        put_text(
            buffer,
            x + 2 + dir_width,
            y,
            width.saturating_sub(2 + dir_width),
            name,
            Style::default().fg(palette.subtext0).bg(bg),
        );
        y += 1;
    }

    // DEPOTS: worktrees, the one this pane is in highlighted
    if data.worktrees.len() > 1 {
        y += 1;
        if fits(y) {
            section(
                buffer,
                area,
                y,
                "DEPOTS",
                &data.worktrees.len().to_string(),
                palette,
            );
            y += 1;
        }
        for (name, current) in data.worktrees.iter().take(6) {
            if !fits(y) {
                break;
            }
            let row_bg = if *current { palette.selection_bg } else { bg };
            buffer.set_style(
                Rect::new(area.x + 1, y, area.width.saturating_sub(2), 1),
                Style::default().bg(row_bg),
            );
            let (mark, mark_color) = if *current {
                ("●", palette.accent)
            } else {
                ("○", palette.overlay0)
            };
            put_text(
                buffer,
                x,
                y,
                1,
                mark,
                Style::default().fg(mark_color).bg(row_bg),
            );
            let style = if *current {
                Style::default()
                    .fg(palette.text)
                    .bg(row_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.subtext0).bg(row_bg)
            };
            put_text(
                buffer,
                x + 2,
                y,
                width.saturating_sub(7),
                &clip_right(name, w.saturating_sub(7)),
                style,
            );
            if *current {
                put_right_text(
                    buffer,
                    Rect::new(area.x, y, area.width.saturating_sub(2), 1),
                    y,
                    "here",
                    Style::default().fg(palette.overlay0).bg(row_bg),
                );
            }
            y += 1;
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
