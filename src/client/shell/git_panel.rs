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
    draw_gauge_wide(buffer, x, y, 10, left, bg, t);
}

/// Ten flat segments across `width` cells, a gap between them.
pub(super) fn draw_gauge_wide(
    buffer: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    left: u8,
    bg: ratatui::style::Color,
    t: &Brutal,
) {
    let color = fuel_color(left, t);
    let filled = (u16::from(left) + 5) / 10;
    let segment = (width / 10).max(1);
    for index in 0..10u16 {
        let fg = if index < filled { color } else { t.empty };
        let sx = x + index * segment;
        // a cell of gap between segments once they are wide enough
        let solid = if segment >= 2 { segment - 1 } else { 1 };
        put_text(
            buffer,
            sx,
            y,
            solid,
            &"▬".repeat(usize::from(solid)),
            Style::default().fg(fg).bg(bg),
        );
    }
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

/// One row of the right panel, built first so the box hugs its content.
enum Row<'a> {
    Blank,
    Rule,
    Title(String, String, Option<Slot>),
    Gauge(&'a super::accounts::FuelWindow),
    Note(String, ratatui::style::Color),
    Strong(String),
    Buttons(Vec<(&'static str, bool, Slot)>),
    File(&'a str, &'a str),
    History(&'a HistoryLine),
    Worktree(&'a str, bool),
    NewWorktree,
}

#[derive(Clone, Copy)]
enum Slot {
    Fuel,
    Ide,
    GitGui,
    Diff,
    Commit,
    PullRequest,
    History,
    Worktrees,
}

fn hit_slot(hits: &mut ShellHitMap, slot: Slot, rect: Rect) {
    let target = match slot {
        Slot::Fuel => &mut hits.orcha.fuel,
        Slot::Ide => &mut hits.orcha.ide,
        Slot::GitGui => &mut hits.orcha.git_gui,
        Slot::Diff => &mut hits.orcha.diff,
        Slot::Commit => &mut hits.orcha.commit,
        Slot::PullRequest => &mut hits.orcha.pull_request,
        Slot::History => &mut hits.orcha.history_toggle,
        Slot::Worktrees => &mut hits.orcha.worktrees_toggle,
    };
    *target = rect;
}

/// The right panel (docs/orcha/SPEC.md §2 "Right panel"): one heavy box,
/// sections split by rules with a blank row of padding on each side.
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
    if area.width < 16 || area.height < 6 {
        return;
    }
    let t = Brutal::new(&config.palette);
    buffer.set_style(area, Style::default().bg(t.bg));
    let fuel = fuel.filter(|_| config.orcha.fuel == crate::config::FuelPlacement::Panel);
    let mut rows: Vec<Row<'_>> = Vec::new();
    // a rule between sections; its line sits mid-row, half a row of air each side
    let section = |rows: &mut Vec<Row<'_>>| {
        if !rows.is_empty() {
            rows.push(Row::Rule);
        }
    };

    if let Some(account) = fuel {
        section(&mut rows);
        let who = account.email.clone().unwrap_or_else(|| account.label.clone());
        rows.push(Row::Title("FUEL".into(), who, Some(Slot::Fuel)));
        for window in account.windows.iter().take(2) {
            rows.push(Row::Gauge(window));
        }
        if account.windows.is_empty() {
            let note = account.error.clone().unwrap_or_else(|| "no data".into());
            rows.push(Row::Note(note, t.red));
        }
    }
    section(&mut rows);
    let Some(data) = data else {
        rows.push(Row::Title("BRANCH".into(), String::new(), None));
        rows.push(Row::Note("not a git repository".into(), t.muted));
        rows.push(Row::Blank);
        draw_rows(buffer, area, &rows, config, &t, hits);
        return;
    };
    let sync = match (&data.upstream, data.ahead, data.behind) {
        (None, _, _) => "not pushed".to_owned(),
        (Some(_), 0, 0) => "in sync".to_owned(),
        (Some(_), a, 0) => format!("↑{a} to push"),
        (Some(_), 0, b) => format!("↓{b} to pull"),
        (Some(_), a, b) => format!("↑{a} ↓{b}"),
    };
    rows.push(Row::Title("BRANCH".into(), sync, None));
    let branch = if data.branch.is_empty() {
        "(no branch)"
    } else {
        &data.branch
    };
    rows.push(Row::Strong(format!("⎇ {branch}")));
    rows.push(Row::Buttons(vec![
        ("⌘E IDE", false, Slot::Ide),
        ("⌘G Git", false, Slot::GitGui),
    ]));

    section(&mut rows);
    let stat = if data.files.is_empty() {
        "clean".to_owned()
    } else {
        format!("+{} −{}", data.insertions, data.deletions)
    };
    rows.push(Row::Title("CHANGES".into(), stat, None));
    // what is left of the box after the other sections decides how many files show
    let tail = 5
        + if ui.history_open { data.history.len().min(14) } else { 0 }
        + if ui.worktrees_open { data.worktrees.len().min(6) + 1 } else { 0 };
    let room = usize::from(area.height)
        .saturating_sub(rows.len() + tail + 2 + usize::from(arrived) * 2);
    let shown = data.files.len().min(room.max(1));
    for (index, (code, path)) in data.files.iter().enumerate().take(shown) {
        if index + 1 == shown && data.files.len() > shown {
            rows.push(Row::Note(format!("… {} more", data.files.len() - index), t.muted));
        } else {
            rows.push(Row::File(code, path));
        }
    }
    let arrived = arrived && !data.files.is_empty();
    if arrived {
        rows.push(Row::Blank);
        rows.push(Row::Buttons(vec![
            ("◇ DIFF", false, Slot::Diff),
            ("✓ COMMIT", true, Slot::Commit),
            ("↗ PR", false, Slot::PullRequest),
        ]));
    }

    section(&mut rows);
    let arrow = if ui.history_open { "▾" } else { "▸" };
    let last = data
        .last_commit
        .as_ref()
        .map(|(subject, age)| format!("{subject} · {age}"))
        .unwrap_or_default();
    rows.push(Row::Title(format!("{arrow} HISTORY"), last, Some(Slot::History)));
    if ui.history_open {
        rows.extend(data.history.iter().take(14).map(Row::History));
    }
    section(&mut rows);
    let arrow = if ui.worktrees_open { "▾" } else { "▸" };
    rows.push(Row::Title(
        format!("{arrow} WORKTREES"),
        data.worktrees.len().to_string(),
        Some(Slot::Worktrees),
    ));
    if ui.worktrees_open {
        rows.extend(
            data.worktrees
                .iter()
                .take(6)
                .map(|(name, current)| Row::Worktree(name, *current)),
        );
        rows.push(Row::NewWorktree);
    }
    draw_rows(buffer, area, &rows, config, &t, hits);
}

fn draw_rows(
    buffer: &mut Buffer,
    area: Rect,
    rows: &[Row<'_>],
    _config: &ClientShellConfig,
    t: &Brutal,
    hits: &mut ShellHitMap,
) {
    let height = (rows.len() as u16 + 2).min(area.height);
    let rect = Rect::new(area.x, area.y, area.width, height);
    brutal::draw_box(buffer, rect, Edge::Heavy, t.line, t.card, t.bg);
    let x = rect.x + 3;
    let inner_width = rect.width.saturating_sub(6);
    let w = usize::from(inner_width);
    let bottom = rect.bottom().saturating_sub(1);
    let mut y = rect.y + 1;
    // a title's hit covers its whole section, up to the next rule
    let mut open_slot: Option<(Slot, u16)> = None;
    let close = |hits: &mut ShellHitMap, open: &mut Option<(Slot, u16)>, y: u16| {
        if let Some((slot, top)) = open.take() {
            hit_slot(hits, slot, Rect::new(rect.x, top, rect.width, y.saturating_sub(top)));
        }
    };
    for row in rows {
        if y >= bottom {
            break;
        }
        match row {
            Row::Blank => {}
            Row::Rule => {
                close(hits, &mut open_slot, y);
                put_text(
                    buffer,
                    rect.x + 1,
                    y,
                    rect.width.saturating_sub(2),
                    &"━".repeat(usize::from(rect.width.saturating_sub(2))),
                    Style::default().fg(t.line).bg(t.card),
                );
            }
            Row::Title(text, note, slot) => {
                title(buffer, x, y, inner_width, text, note, t);
                if let Some(slot) = slot {
                    open_slot = Some((*slot, y.saturating_sub(1)));
                }
            }
            Row::Gauge(window) => {
                put_text(buffer, x, y, 6, &window.label, Style::default().fg(t.muted).bg(t.card));
                // the marks run the width of the panel, the % right of them
                draw_gauge_wide(buffer, x + 6, y, inner_width.saturating_sub(12), window.left, t.card, t);
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
            }
            Row::Note(text, color) => put_text(
                buffer,
                x,
                y,
                inner_width,
                &clip_right(text, w),
                Style::default().fg(*color).bg(t.card),
            ),
            Row::Strong(text) => put_text(
                buffer,
                x,
                y,
                inner_width,
                &clip_right(text, w),
                Style::default()
                    .fg(t.ink)
                    .bg(t.card)
                    .add_modifier(Modifier::BOLD),
            ),
            Row::Buttons(buttons) => {
                let mut bx = x;
                for (label, primary, slot) in buttons {
                    let text = format!(" {label} ");
                    let width = display_width(&text);
                    if bx + width > x + inner_width {
                        break;
                    }
                    let (fg, bg) = if *primary {
                        (t.chip_text, t.green)
                    } else {
                        (t.ink, t.frame)
                    };
                    put_text(
                        buffer,
                        bx,
                        y,
                        width,
                        &text,
                        Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
                    );
                    hit_slot(hits, *slot, Rect::new(bx, y, width, 1));
                    bx += width + 1;
                }
            }
            Row::File(code, path) => {
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
                    x + 3,
                    y,
                    inner_width.saturating_sub(3),
                    &clip_left(path, w.saturating_sub(3)),
                    Style::default().fg(t.ink).bg(t.card),
                );
            }
            Row::History(line) => {
                let mut cx = x;
                for (column, ch) in line.graph.chars().enumerate() {
                    if cx >= x + inner_width {
                        break;
                    }
                    put_text(
                        buffer,
                        cx,
                        y,
                        1,
                        history_glyph(ch),
                        Style::default()
                            .fg(lane_color(column / 2, t))
                            .bg(t.card)
                            .add_modifier(Modifier::BOLD),
                    );
                    cx += 1;
                }
                cx += 1;
                if let Some(name) = line.refs.first() {
                    let room = (x + inner_width).saturating_sub(cx);
                    cx += brutal::chip(buffer, cx, y, room.min(12), &clip_right(name, 8), t.chip_text, t.green) + 1;
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
            }
            Row::Worktree(name, current) => {
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
                put_text(buffer, x, y, inner_width, &clip_right(&format!("{mark} {name}"), w), style);
            }
            Row::NewWorktree => {
                let label = "+ NEW WORKTREE";
                let button = Rect::new(x, y, inner_width, 1);
                buffer.set_style(button, Style::default().bg(t.bg));
                let pad = inner_width.saturating_sub(display_width(label)) / 2;
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
        y += 1;
    }
    close(hits, &mut open_slot, y);
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
