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

use super::brutal::{self, Brutal, Edge};
use super::render::{display_width, put_right_text, put_text};
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
    /// Lines added and removed against HEAD.
    pub insertions: usize,
    pub deletions: usize,
    /// Subject and relative age of the last commit.
    pub last_commit: Option<(String, String)>,
    /// `git log --graph` of the local branches, newest first.
    pub history: Vec<HistoryLine>,
}

/// One row of the commit graph: the graph glyphs, then (for a commit row)
/// its refs and subject.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct HistoryLine {
    pub graph: String,
    pub refs: Vec<String>,
    pub subject: String,
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

    /// Called from the client timer: refreshes the accounts' fuel and moves
    /// the PROCESS trains while any agent is running.
    pub(crate) fn tick_orcha(&mut self, now: Instant) -> bool {
        if !self.config.sidebar_tree || self.active_endpoint_id != ClientEndpointId::Local {
            return false;
        }
        let pane = self
            .snapshot
            .as_deref()
            .and_then(|snapshot| snapshot.focused_pane_id.clone());
        let mut repaint = self
            .accounts
            .tick(&self.config.orcha.accounts_command, pane.as_deref(), now);
        if repaint {
            self.update_low_fuel_hint();
        }
        let running = self.snapshot.as_deref().is_some_and(|snapshot| {
            snapshot.agents.iter().any(|agent| {
                matches!(agent.agent_status, crate::api::schema::AgentStatus::Working)
            })
        });
        let ui = &mut self.orcha_ui;
        if running && ui.next_process_tick.is_none_or(|at| now >= at) {
            ui.process_frame = ui.process_frame.wrapping_add(1);
            ui.next_process_tick = Some(now + PROCESS_FRAME);
            repaint = true;
        }
        repaint
    }
}

/// How often the PROCESS trains move a cell.
const PROCESS_FRAME: Duration = Duration::from_millis(220);

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
    if let Some(stat) = git(root_path, &["diff", "HEAD", "--shortstat"]) {
        let (insertions, deletions) = parse_shortstat(&stat);
        data.insertions = insertions;
        data.deletions = deletions;
    }
    data.last_commit = git(root_path, &["log", "-1", "--format=%s%x09%cr"]).and_then(|line| {
        let (subject, age) = line.trim().split_once('\t')?;
        Some((subject.to_owned(), age.to_owned()))
    });
    if let Some(log) = git(
        root_path,
        &[
            "log",
            "--graph",
            "--branches",
            "-n",
            "16",
            "--format=%x1f%D%x1f%s",
        ],
    ) {
        data.history = log.lines().take(24).map(parse_history_line).collect();
    }
    Some(data)
}

/// " 3 files changed, 26 insertions(+), 4 deletions(-)"
fn parse_shortstat(stat: &str) -> (usize, usize) {
    let mut out = (0, 0);
    for part in stat.split(',') {
        let part = part.trim();
        let number = part
            .split_whitespace()
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        if part.contains("insertion") {
            out.0 = number;
        } else if part.contains("deletion") {
            out.1 = number;
        }
    }
    out
}

/// "* \x1fHEAD -> main, tag: v1\x1fsubject" or a graph-only line "|\\".
fn parse_history_line(line: &str) -> HistoryLine {
    let mut fields = line.split('\u{1f}');
    let graph = fields.next().unwrap_or("").trim_end().to_owned();
    let refs = fields
        .next()
        .map(|refs| {
            refs.split(", ")
                .filter(|r| !r.is_empty())
                .map(|r| r.trim_start_matches("HEAD -> ").trim_start_matches("tag: ").to_owned())
                .collect()
        })
        .unwrap_or_default();
    let subject = fields.next().unwrap_or("").to_owned();
    HistoryLine {
        graph,
        refs,
        subject,
    }
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

/// A gauge of ten segments: green over half left, yellow under, red under 20%.
pub(super) fn fuel_color(left: u8, t: &Brutal) -> ratatui::style::Color {
    if left < 20 {
        t.red
    } else if left < 50 {
        t.yellow
    } else {
        t.green
    }
}

pub(super) fn draw_gauge(buffer: &mut Buffer, x: u16, y: u16, left: u8, bg: ratatui::style::Color, t: &Brutal) {
    let color = fuel_color(left, t);
    let filled = (u16::from(left) + 5) / 10;
    for index in 0..10u16 {
        let fg = if index < filled { color } else { t.empty };
        put_text(buffer, x + index, y, 1, "▌", Style::default().fg(fg).bg(bg));
    }
}

/// Splits the panel with a light rule that meets its border.
fn rule(buffer: &mut Buffer, rect: Rect, y: u16, t: &Brutal) {
    let style = Style::default().fg(t.line).bg(t.card);
    put_text(
        buffer,
        rect.x + 1,
        y,
        rect.width.saturating_sub(2),
        &"━".repeat(usize::from(rect.width.saturating_sub(2))),
        style,
    );
}

fn title(buffer: &mut Buffer, x: u16, y: u16, width: u16, text: &str, note: &str, t: &Brutal) {
    put_text(
        buffer,
        x,
        y,
        width,
        text,
        Style::default()
            .fg(t.ink)
            .bg(t.card)
            .add_modifier(Modifier::BOLD),
    );
    if !note.is_empty() {
        let room = width.saturating_sub(display_width(text) + 2);
        put_right_text(
            buffer,
            Rect::new(x, y, width, 1),
            y,
            &clip_right(note, usize::from(room)),
            Style::default().fg(t.muted).bg(t.card),
        );
    }
}

fn lane_color(column: usize, t: &Brutal) -> ratatui::style::Color {
    [t.pink, t.blue, t.green, t.yellow, t.red][column % 5]
}

/// The subway-style commit graph: lanes are colored rails, commits stations.
fn history_glyph(ch: char) -> &'static str {
    match ch {
        '*' => "●",
        '|' => "┃",
        '/' => "╱",
        '\\' => "╲",
        '_' => "━",
        '-' => "━",
        '.' => "┄",
        _ => " ",
    }
}

#[allow(clippy::too_many_arguments)] // one render call, all inputs are distinct facts
pub(super) fn render_git_panel(
    buffer: &mut Buffer,
    area: Rect,
    data: Option<&GitPanelData>,
    fuel: Option<&super::accounts::Account>,
    config: &ClientShellConfig,
    ui: &super::OrchaUiState,
    arrived: bool,
    hits: &mut ShellHitMap,
) {
    if area.width < 12 || area.height < 4 {
        return;
    }
    let t = Brutal::new(&config.palette);
    buffer.set_style(area, Style::default().bg(t.bg));
    let fuel = fuel.filter(|_| config.orcha.fuel == crate::config::FuelPlacement::Panel);
    let inner_width = area.width.saturating_sub(6);
    let w = usize::from(inner_width);

    // rows each section needs, so the box hugs its content
    let fuel_rows = fuel.map_or(0, |account| 2 + account.windows.len().clamp(1, 2) as u16);
    let branch_rows = if data.is_some() { 4 } else { 3 };
    let history_rows = 1 + if ui.history_open {
        data.map_or(0, |d| d.history.len().min(14) as u16)
    } else {
        0
    };
    let worktree_rows = if ui.worktrees_open {
        2 + data.map_or(0, |d| d.worktrees.len().min(6) as u16)
    } else {
        1
    };
    let fixed = 2 + fuel_rows + branch_rows + 2 + history_rows + 1 + worktree_rows;
    let file_count = data.map_or(0, |d| d.files.len()) as u16;
    let file_rows = file_count.min(area.height.saturating_sub(1).saturating_sub(fixed));
    // the train arrived: its changes get DIFF / COMMIT / PR
    let arrived = arrived && data.is_some_and(|d| !d.files.is_empty());
    let changes_rows = if data.is_some() {
        1 + file_rows.max(1) + u16::from(arrived)
    } else {
        0
    };
    let height = if data.is_some() {
        fixed + changes_rows
    } else {
        // no repository: FUEL and the BRANCH note only
        2 + fuel_rows + 2
    }
    .min(area.height.saturating_sub(1));
    let rect = Rect::new(area.x + 1, area.y, area.width.saturating_sub(2), height);
    brutal::draw_box(buffer, rect, Edge::Heavy, t.line, t.card, t.bg);
    let x = rect.x + 2;
    let mut y = rect.y + 1;
    let bottom = rect.bottom().saturating_sub(1);

    // FUEL: whose plan, and how much of it is left
    if let Some(account) = fuel {
        let top = y;
        let who = account.email.clone().unwrap_or_else(|| account.label.clone());
        title(buffer, x, y, inner_width, "FUEL", &who, &t);
        y += 1;
        for window in account.windows.iter().take(2) {
            if y >= bottom {
                break;
            }
            put_text(
                buffer,
                x,
                y,
                6,
                &window.label,
                Style::default().fg(t.muted).bg(t.card),
            );
            draw_gauge(buffer, x + 6, y, window.left, t.card, &t);
            put_right_text(
                buffer,
                Rect::new(x, y, inner_width, 1),
                y,
                &format!("{}%", window.left),
                Style::default()
                    .fg(if window.left < 20 { t.red } else { t.ink })
                    .bg(t.card)
                    .add_modifier(Modifier::BOLD),
            );
            y += 1;
        }
        if account.windows.is_empty() && y < bottom {
            // signed out or expired: say so, the popup offers SIGN IN
            let note = account.error.clone().unwrap_or_else(|| "no data".to_owned());
            put_text(
                buffer,
                x,
                y,
                inner_width,
                &clip_right(&note, w),
                Style::default().fg(t.red).bg(t.card),
            );
            y += 1;
        }
        hits.orcha.fuel = Rect::new(rect.x, top, rect.width, y - top);
        rule(buffer, rect, y, &t);
        y += 1;
    }

    // BRANCH
    let Some(data) = data else {
        title(buffer, x, y, inner_width, "BRANCH", "", &t);
        put_text(
            buffer,
            x,
            y + 1,
            inner_width,
            "not a git repository",
            Style::default().fg(t.muted).bg(t.card),
        );
        return;
    };
    let sync = match (&data.upstream, data.ahead, data.behind) {
        (None, _, _) => "not pushed".to_owned(),
        (Some(_), 0, 0) => "in sync".to_owned(),
        (Some(_), a, 0) => format!("↑{a} to push"),
        (Some(_), 0, b) => format!("↓{b} to pull"),
        (Some(_), a, b) => format!("↑{a} ↓{b}"),
    };
    title(buffer, x, y, inner_width, "BRANCH", &sync, &t);
    y += 1;
    let branch = if data.branch.is_empty() {
        "(no branch)"
    } else {
        &data.branch
    };
    put_text(
        buffer,
        x,
        y,
        inner_width,
        &clip_right(&format!("⎇ {branch}"), w),
        Style::default()
            .fg(t.ink)
            .bg(t.card)
            .add_modifier(Modifier::BOLD),
    );
    y += 1;
    // open the project elsewhere
    let mut bx = x;
    for (label, slot) in [(" ⌘E IDE ", 0), (" ⌘G Git ", 1)] {
        let width = display_width(label);
        if bx + width > x + inner_width {
            break;
        }
        put_text(
            buffer,
            bx,
            y,
            width,
            label,
            Style::default()
                .fg(t.ink)
                .bg(t.frame)
                .add_modifier(Modifier::BOLD),
        );
        let hit = Rect::new(bx, y, width, 1);
        if slot == 0 {
            hits.orcha.ide = hit;
        } else {
            hits.orcha.git_gui = hit;
        }
        bx += width + 1;
    }
    y += 1;
    rule(buffer, rect, y, &t);
    y += 1;

    // CHANGES
    let stat = if data.files.is_empty() {
        "clean".to_owned()
    } else {
        format!("+{} −{}", data.insertions, data.deletions)
    };
    title(buffer, x, y, inner_width, "CHANGES", &stat, &t);
    y += 1;
    let shown = usize::from(file_rows);
    for (index, (code, path)) in data.files.iter().enumerate().take(shown) {
        if y >= bottom {
            break;
        }
        if index + 1 == shown && data.files.len() > shown {
            put_text(
                buffer,
                x,
                y,
                inner_width,
                &format!("… {} more", data.files.len() - index),
                Style::default().fg(t.muted).bg(t.card),
            );
            y += 1;
            break;
        }
        let (mark, color) = match code.chars().next().unwrap_or(' ') {
            '?' | 'A' => ("A", t.green),
            'D' => ("D", t.red),
            'R' => ("R", t.blue),
            'U' => ("!", t.red),
            _ => ("M", t.yellow),
        };
        put_text(
            buffer,
            x,
            y,
            1,
            mark,
            Style::default()
                .fg(color)
                .bg(t.card)
                .add_modifier(Modifier::BOLD),
        );
        put_text(
            buffer,
            x + 2,
            y,
            inner_width.saturating_sub(2),
            &clip_left(path, w.saturating_sub(2)),
            Style::default().fg(t.ink).bg(t.card),
        );
        y += 1;
    }
    if data.files.is_empty() {
        y += 1;
    }
    if arrived && y < bottom {
        let mut bx = x;
        for (index, label) in [" ◇ DIFF ", " ✓ COMMIT ", " ↗ PR "].iter().enumerate() {
            let width = display_width(label);
            if bx + width > x + inner_width {
                break;
            }
            let (fg, bg) = if index == 1 {
                (t.chip_text, t.green)
            } else {
                (t.ink, t.frame)
            };
            put_text(
                buffer,
                bx,
                y,
                width,
                label,
                Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
            );
            let hit = Rect::new(bx, y, width, 1);
            match index {
                0 => hits.orcha.diff = hit,
                1 => hits.orcha.commit = hit,
                _ => hits.orcha.pull_request = hit,
            }
            bx += width + 1;
        }
        y += 1;
    }
    if y >= bottom {
        return;
    }
    rule(buffer, rect, y, &t);
    y += 1;

    // HISTORY, folded to its last commit
    let arrow = if ui.history_open { "▾" } else { "▸" };
    let last = data
        .last_commit
        .as_ref()
        .map(|(subject, age)| format!("{subject} · {age}"))
        .unwrap_or_default();
    title(buffer, x, y, inner_width, &format!("{arrow} HISTORY"), &last, &t);
    hits.orcha.history_toggle = Rect::new(rect.x, y, rect.width, 1);
    y += 1;
    if ui.history_open {
        for line in data.history.iter().take(14) {
            if y >= bottom {
                break;
            }
            let mut cx = x;
            for (column, ch) in line.graph.chars().enumerate() {
                if cx >= x + inner_width {
                    break;
                }
                let glyph = history_glyph(ch);
                let style = Style::default()
                    .fg(lane_color(column / 2, &t))
                    .bg(t.card)
                    .add_modifier(Modifier::BOLD);
                put_text(buffer, cx, y, 1, glyph, style);
                cx += 1;
            }
            cx += 1;
            for name in line.refs.iter().take(1) {
                let room = (x + inner_width).saturating_sub(cx);
                let width = brutal::chip(
                    buffer,
                    cx,
                    y,
                    room.min(12),
                    &clip_right(name, 8),
                    t.chip_text,
                    if name == &data.branch { t.pink } else { t.green },
                );
                cx += width + 1;
            }
            let room = (x + inner_width).saturating_sub(cx);
            put_text(
                buffer,
                cx,
                y,
                room,
                &clip_right(&line.subject, usize::from(room)),
                Style::default().fg(t.ink).bg(t.card),
            );
            y += 1;
        }
    }
    if y >= bottom {
        return;
    }
    rule(buffer, rect, y, &t);
    y += 1;

    // WORKTREES, folded to a count
    let arrow = if ui.worktrees_open { "▾" } else { "▸" };
    title(
        buffer,
        x,
        y,
        inner_width,
        &format!("{arrow} WORKTREES"),
        &data.worktrees.len().to_string(),
        &t,
    );
    hits.orcha.worktrees_toggle = Rect::new(rect.x, y, rect.width, 1);
    y += 1;
    if ui.worktrees_open {
        for (name, current) in data.worktrees.iter().take(6) {
            if y >= bottom {
                break;
            }
            let (mark, style) = if *current {
                (
                    "●",
                    Style::default()
                        .fg(t.ink)
                        .bg(t.card)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                ("○", Style::default().fg(t.ink).bg(t.card))
            };
            put_text(
                buffer,
                x,
                y,
                inner_width,
                &clip_right(&format!("{mark} {name}"), w),
                style,
            );
            y += 1;
        }
        if y < bottom {
            let label = "+ NEW WORKTREE";
            let pad = inner_width.saturating_sub(display_width(label)) / 2;
            let button = Rect::new(x, y, inner_width, 1);
            buffer.set_style(button, Style::default().bg(t.bg));
            put_text(
                buffer,
                x + pad,
                y,
                display_width(label),
                label,
                Style::default()
                    .fg(t.ink)
                    .bg(t.bg)
                    .add_modifier(Modifier::BOLD),
            );
            hits.orcha.new_worktree = button;
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
    fn shortstat_counts_lines() {
        assert_eq!(
            parse_shortstat(" 3 files changed, 26 insertions(+), 4 deletions(-)"),
            (26, 4)
        );
        assert_eq!(parse_shortstat(" 1 file changed, 1 deletion(-)"), (0, 1));
    }

    #[test]
    fn history_lines_split_graph_refs_and_subject() {
        let line = parse_history_line("* \u{1f}HEAD -> main, tag: v1\u{1f}release");
        assert_eq!(line.graph, "*");
        assert_eq!(line.refs, vec!["main", "v1"]);
        assert_eq!(line.subject, "release");
        assert_eq!(parse_history_line("|\\").graph, "|\\");
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
