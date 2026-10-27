//! orcha: the left sidebar (docs/orcha/SPEC.md §2 "Left sidebar", §3.1).
//!
//! Flat project rows preserve room for names. Expanded panes use their agent
//! logos; collapsed projects summarize activity with the relevant logos.
//! Only the selected project's branch is shown, below its name.
//!
//! A train's station (THINK → CODE → TEST → DONE) comes from the pane's
//! `stage` token when an integration reports one, else from the agent status.

use crate::api::schema::AgentStatus;
use crate::protocol::{ClientShellAgent, ClientShellPane};
use ratatui::style::Color;
use std::borrow::Cow;
use std::collections::HashMap;

use super::brutal::{self, Brutal};
use super::render::{display_width, put_right_text, put_text, ShellRenderState};
use super::*;

const STATIONS: [&str; 4] = ["THINK", "CODE", "TEST", "DONE"];
/// Rows of the PROCESS card per train (label + rail).
const TRAIN_ROWS: u16 = 2;
const MAX_TRAINS: usize = 4;

/// The station a train is at: the `stage` token an integration reports, else
/// a guess from the agent status.
pub(super) fn station(tokens: &[(String, String)], status: AgentStatus) -> usize {
    let reported = tokens
        .iter()
        .find(|(key, _)| key == "stage")
        .and_then(|(_, value)| match value.to_ascii_lowercase().as_str() {
            "thinking" | "think" | "reading" | "read" | "planning" => Some(0),
            "coding" | "code" | "editing" | "edit" | "writing" => Some(1),
            "testing" | "test" | "checking" | "check" => Some(2),
            "done" => Some(3),
            _ => None,
        });
    match status {
        AgentStatus::Done => 3,
        _ => reported.unwrap_or(match status {
            AgentStatus::Working | AgentStatus::Blocked => 1,
            _ => 0,
        }),
    }
}

/// "OPUS 5.5" → "Opus 5.5", "GPT-6 LUNA" → "GPT-6 Luna": words with digits or
/// dashes stay as they are, the rest are capitalized.
fn row_name(label: &str) -> String {
    label
        .split(' ')
        .map(|word| {
            if word.chars().any(|c| c.is_ascii_digit() || c == '-') {
                word.to_owned()
            } else {
                let mut chars = word.chars();
                chars.next().map_or_else(String::new, |first| {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                })
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn running(agent: &ClientShellAgent) -> bool {
    matches!(
        agent.agent_status,
        AgentStatus::Working | AgentStatus::Blocked
    )
}

fn agent_color(agent: &ClientShellAgent, t: &Brutal) -> Color {
    match agent.agent_status {
        AgentStatus::Blocked => t.red,
        AgentStatus::Working => {
            let color = t.agent(agent.agent.as_deref());
            if color == t.card {
                t.yellow
            } else {
                color
            }
        }
        _ => t.muted,
    }
}

/// The two top buttons, three rows tall: a raised green NEW and an
/// outlined menu.
#[allow(clippy::too_many_arguments)] // one render call, all inputs are distinct facts
fn render_buttons(
    buffer: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    t: &Brutal,
    hits: &mut ShellHitMap,
) {
    if !config.mouse_capture || width < 16 {
        return;
    }
    let menu_width = (width * 2 / 5).clamp(10, 16);
    let new_rect = Rect::new(x, y, width.saturating_sub(menu_width + 3), 1);
    brutal::pill(buffer, new_rect, "+ NEW", t.chip_text, t.green, false, t);
    brutal::draw_shadow(buffer, new_rect, t.shadow, t.bg);
    hits.new_workspace = new_rect;
    let attention = super::global_menu::global_menu_attention(snapshot);
    let menu_rect = Rect::new(x + width - menu_width, y, menu_width, 1);
    let label = if attention { "● menu" } else { "≡ menu" };
    brutal::pill(buffer, menu_rect, label, t.ink, t.card, true, t);
    if attention {
        put_text(
            buffer,
            menu_rect.x + (menu_rect.width.saturating_sub(6)) / 2,
            y,
            1,
            "●",
            Style::default().fg(t.pink).bg(t.card),
        );
    }
    hits.global_launcher = menu_rect;
}

/// How far a train has chugged inside its leg on this animation frame:
/// forward and back, so it looks like it is moving and shaking.
fn chug(frame: u64, span: u16) -> u16 {
    if span == 0 {
        return 0;
    }
    let period = u64::from(span) * 2;
    let pos = frame % period;
    (if pos > u64::from(span) {
        period - pos
    } else {
        pos
    }) as u16
}

/// The PROCESS card: running agents only, each a train on four stations.
#[allow(clippy::too_many_arguments)] // one render call, all inputs are distinct facts
fn render_process(
    buffer: &mut Buffer,
    rect: Rect,
    trains: &[Option<(&ClientShellAgent, &str)>],
    total: usize,
    frame: u64,
    config: &ClientShellConfig,
    t: &Brutal,
    hits: &mut ShellHitMap,
) {
    // its bottom row is its line and shadow, right on the bottom
    let card = Rect::new(rect.x, rect.y, rect.width.saturating_sub(1), rect.height);
    brutal::draw_box(buffer, card, t.line, t.card, t.bg);
    brutal::draw_box_shadow(buffer, card, t.shadow, t.bg);
    let x = rect.x + 2;
    let width = rect.width.saturating_sub(5);
    let mut y = rect.y + 1;
    put_text(
        buffer,
        x,
        y,
        9,
        "▾ PROCESS",
        Style::default()
            .fg(t.ink)
            .bg(t.card)
            .add_modifier(Modifier::BOLD),
    );
    let count = format!(" {total} ACTIVE ");
    let count_width = display_width(&count);
    put_text(
        buffer,
        // in the corner, against the right line
        card.right().saturating_sub(count_width + 1),
        y,
        count_width,
        &count,
        Style::default()
            .fg(t.card)
            .bg(t.ink)
            .add_modifier(Modifier::BOLD),
    );
    y += 1;
    let leg = (width / 4).max(4);
    // a narrow sidebar gets short station names
    let names: [&str; 4] = if leg >= 6 {
        STATIONS
    } else {
        ["THK", "COD", "TST", "END"]
    };
    for (index, name) in names.iter().enumerate() {
        let lx = x + leg * index as u16;
        let style = Style::default()
            .fg(t.muted)
            .bg(t.card)
            .add_modifier(Modifier::BOLD);
        if index == STATIONS.len() - 1 {
            put_right_text(buffer, Rect::new(x, y, width, 1), y, name, style);
        } else {
            put_text(buffer, lx, y, leg, name, style);
        }
    }
    y += 1;
    if trains.is_empty() {
        put_text(
            buffer,
            x,
            y,
            width,
            "all quiet — no train running",
            Style::default().fg(t.muted).bg(t.card),
        );
        return;
    }
    for (agent, project) in trains.iter().flatten() {
        if y + TRAIN_ROWS > rect.bottom().saturating_sub(1) {
            break;
        }
        let color = t.agent(agent.agent.as_deref());
        let kind = brutal::agent_label(agent).to_uppercase();
        let chip_width = brutal::chip(buffer, x, y, width, &kind, t.chip_text, color);
        let task = brutal::agent_task(agent);
        let detail = if task.is_empty() {
            Cow::Borrowed(*project)
        } else {
            Cow::Owned(format!("{project} · {task}"))
        };
        let detail_x = x + chip_width + 1;
        let detail_width = (x + width).saturating_sub(detail_x);
        put_text(
            buffer,
            detail_x,
            y,
            detail_width,
            &brutal::clip(detail.as_ref(), usize::from(detail_width)),
            Style::default().fg(t.ink).bg(t.card),
        );
        hits.agents
            .push((Rect::new(x, y, width, TRAIN_ROWS), agent.pane_id.clone()));
        y += 1;

        // the rail: done legs solid, the rest dashed, a stop at each leg's end
        let at = station(&agent.tokens, agent.agent_status).min(STATIONS.len() - 1);
        for index in 0..STATIONS.len() {
            let lx = x + leg * index as u16;
            let lw = if index == STATIONS.len() - 1 {
                (x + width).saturating_sub(lx)
            } else {
                leg
            };
            let done = index < at;
            let (glyph, rail) = if done {
                ("━", t.rail)
            } else {
                ("┄", t.rail_ahead)
            };
            put_text(
                buffer,
                lx,
                y,
                lw.saturating_sub(1),
                &glyph.repeat(usize::from(lw.saturating_sub(1))),
                Style::default().fg(rail).bg(t.card),
            );
            let stop = if done { "•" } else { "·" };
            put_text(
                buffer,
                lx + lw.saturating_sub(1),
                y,
                1,
                stop,
                Style::default()
                    .fg(if done { t.rail } else { t.line })
                    .bg(t.card),
            );
        }
        let logo = brutal::agent_logo(agent.agent.as_deref(), config.icon_font);
        let blocked = agent.agent_status == AgentStatus::Blocked;
        let body = if blocked {
            format!("{logo}!")
        } else {
            format!("{logo}▶")
        };
        let body_width = display_width(&body);
        let lx = x + leg * at as u16;
        // a blocked train stands still; a running one chugs along its leg
        let span = leg.saturating_sub(body_width + 2);
        let offset = if blocked { 0 } else { chug(frame, span) };
        put_text(
            buffer,
            lx + offset,
            y,
            body_width,
            &body,
            Style::default()
                .fg(t.chip_text)
                .bg(if blocked { t.red } else { color })
                .add_modifier(Modifier::BOLD),
        );
        y += 1;
    }
}

/// A project's rows: every pane, and any agent whose pane is not in the
/// snapshot yet (tests, a moment after a split).
#[derive(Clone, Copy)]
enum ProjectRow<'a> {
    Agent(&'a ClientShellAgent),
    Shell(&'a ClientShellPane),
}

#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
enum ProjectActivity {
    #[default]
    Idle,
    Done,
    Running,
    NeedsYou,
}

impl ProjectActivity {
    fn from_status(status: AgentStatus) -> Self {
        match status {
            AgentStatus::Blocked => Self::NeedsYou,
            AgentStatus::Working => Self::Running,
            AgentStatus::Done => Self::Done,
            _ => Self::Idle,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Done => "done",
            Self::Running => "running",
            Self::NeedsYou => "needs you",
        }
    }

    fn color(self, t: &Brutal) -> Color {
        match self {
            Self::Idle => t.muted,
            Self::Done => t.green,
            Self::Running => t.yellow,
            Self::NeedsYou => t.red,
        }
    }
}

#[derive(Default)]
struct ProjectRows<'a> {
    // Each pane with its tab number, in tab order.
    rows: Vec<(usize, ProjectRow<'a>)>,
    tabs: usize,
    activity: ProjectActivity,
    // At most three distinct agent logos. Active agents replace idle ones.
    logos: [Option<&'a ClientShellAgent>; 3],
}

impl<'a> ProjectRows<'a> {
    fn push(&mut self, tab: usize, row: ProjectRow<'a>) {
        if let ProjectRow::Agent(agent) = row {
            let activity = ProjectActivity::from_status(agent.agent_status);
            if activity >= ProjectActivity::Running && self.activity < ProjectActivity::Running {
                self.logos = [None; 3];
            }
            self.activity = self.activity.max(activity);
            if activity >= ProjectActivity::Running || self.activity < ProjectActivity::Running {
                let kind = agent.agent.as_deref().unwrap_or("");
                if let Some(slot) = self.logos.iter_mut().find(|slot| {
                    slot.is_some_and(|seen| {
                        seen.agent
                            .as_deref()
                            .unwrap_or("")
                            .eq_ignore_ascii_case(kind)
                    })
                }) {
                    if slot.is_none_or(|seen| {
                        ProjectActivity::from_status(seen.agent_status) < activity
                    }) {
                        *slot = Some(agent);
                    }
                } else if let Some(slot) = self.logos.iter_mut().find(|slot| slot.is_none()) {
                    *slot = Some(agent);
                }
            }
        }
        self.rows.push((tab, row));
    }
}

fn project_rows_by_workspace<'a>(
    snapshot: &'a ClientShellSnapshot,
) -> HashMap<&'a str, ProjectRows<'a>> {
    let mut agents_by_pane = HashMap::with_capacity(snapshot.agents.len());
    for agent in &snapshot.agents {
        agents_by_pane
            .entry(agent.pane_id.as_str())
            .or_insert(agent);
    }

    let tab_numbers: HashMap<&str, usize> = snapshot
        .tabs
        .iter()
        .map(|tab| (tab.tab_id.as_str(), tab.number))
        .collect();
    let tab_of = |tab_id: &str| tab_numbers.get(tab_id).copied().unwrap_or(usize::MAX);

    let mut rows_by_workspace: HashMap<&str, ProjectRows<'_>> = HashMap::new();
    for pane in &snapshot.panes {
        let row = agents_by_pane
            .remove(pane.pane_id.as_str())
            .map(ProjectRow::Agent)
            .unwrap_or(ProjectRow::Shell(pane));
        rows_by_workspace
            .entry(pane.workspace_id.as_str())
            .or_default()
            .push(tab_of(&pane.tab_id), row);
    }

    for agent in &snapshot.agents {
        if let Some(agent) = agents_by_pane.remove(agent.pane_id.as_str()) {
            rows_by_workspace
                .entry(agent.workspace_id.as_str())
                .or_default()
                .push(tab_of(&agent.tab_id), ProjectRow::Agent(agent));
        }
    }
    for project in rows_by_workspace.values_mut() {
        // Stable: panes keep their order inside a tab.
        project.rows.sort_by_key(|(tab, _)| *tab);
        project.tabs = project
            .rows
            .iter()
            .enumerate()
            .filter(|(index, (tab, _))| *index == 0 || project.rows[index - 1].0 != *tab)
            .count();
    }
    rows_by_workspace
}

/// What PROCESS shows: up to `MAX_TRAINS` running agents with their
/// projects, and how many run and wait in all.
pub(super) struct ProcessView<'a> {
    trains: [Option<(&'a ClientShellAgent, &'a str)>; MAX_TRAINS],
    visible: usize,
    total: usize,
    blocked: usize,
}

pub(super) fn process_view(snapshot: &ClientShellSnapshot, folded: bool) -> ProcessView<'_> {
    let mut view = ProcessView {
        trains: [None; MAX_TRAINS],
        visible: 0,
        total: 0,
        blocked: 0,
    };
    for agent in snapshot.agents.iter().filter(|agent| running(agent)) {
        view.blocked += usize::from(agent.agent_status == AgentStatus::Blocked);
        if !folded && view.total < MAX_TRAINS {
            let project = snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.workspace_id == agent.workspace_id)
                .map(|workspace| workspace.label.as_str())
                .unwrap_or("");
            view.trains[view.total] = Some((agent, project));
        }
        view.total += 1;
    }
    view.visible = view.total.min(MAX_TRAINS);
    view
}

/// Rows PROCESS takes: one quiet row when idle or folded, else the card.
pub(super) fn process_height(view: &ProcessView<'_>, folded: bool, max: u16) -> u16 {
    if view.visible == 0 || folded {
        1
    } else {
        (4 + view.visible as u16 * TRAIN_ROWS).min(max).max(1)
    }
}

/// Draws PROCESS into `rect`: the card when it fits, else a one-row
/// summary that opens it.
pub(super) fn draw_process_area(
    buffer: &mut Buffer,
    rect: Rect,
    view: &ProcessView<'_>,
    frame: u64,
    config: &ClientShellConfig,
    t: &Brutal,
    hits: &mut ShellHitMap,
) {
    if rect.height >= 5 {
        hits.orcha.process = rect;
        hits.orcha.process_toggle =
            Rect::new(rect.x + 1, rect.y + 1, rect.width.saturating_sub(2), 1);
        render_process(
            buffer,
            rect,
            &view.trains[..view.visible],
            view.total,
            frame,
            config,
            t,
            hits,
        );
        return;
    }
    if rect.height == 0 {
        return;
    }
    let (total, blocked) = (view.total, view.blocked);
    let note = if total == 0 {
        "No agents running".to_owned()
    } else if blocked == 0 {
        format!("▸ {total} running")
    } else if blocked == total {
        format!("▸ {blocked} need you")
    } else {
        format!("▸ {} running · {blocked} needs you", total - blocked)
    };
    if total > 0 {
        hits.orcha.process_toggle = rect;
    }
    put_text(
        buffer,
        rect.x + 2,
        rect.y,
        rect.width.saturating_sub(3),
        &note,
        Style::default()
            .fg(if blocked > 0 {
                t.text(t.red, t.bg)
            } else {
                t.muted
            })
            .bg(t.bg),
    );
}

/// `process_here`: PROCESS sits at the sidebar's foot (not in the right
/// panel, not turned off).
pub(crate) fn render_sidebar_tree(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    state: &mut ShellRenderState<'_>,
    process_here: bool,
    hits: &mut ShellHitMap,
) {
    let t = Brutal::new(&config.palette);
    buffer.set_style(area, Style::default().bg(t.bg).fg(t.ink));
    // the divider faces the panes
    let right_side = config.orcha.sidebar_side == crate::config::SidebarSide::Right;
    hits.sidebar_divider = if area.is_empty() {
        Rect::default()
    } else if right_side {
        Rect::new(area.x, area.y, 1, area.height)
    } else {
        Rect::new(area.right().saturating_sub(1), area.y, 1, area.height)
    };
    hits.sidebar_section_divider = Rect::default();
    hits.agent_body = Rect::default();
    if area.width < 12 || area.height < 6 {
        return;
    }
    // a column of paper on the window's edge: the terminal stretches edge
    // cells' colors into its padding
    let x = area.x + 1;
    let width = area.width.saturating_sub(1);

    render_buttons(buffer, x, area.y, width, snapshot, config, &t, hits);

    // PROCESS at the foot (unless it lives in the right panel or is off)
    let process = if process_here {
        let view = process_view(snapshot, state.orcha_ui.process_folded);
        let height = process_height(
            &view,
            state.orcha_ui.process_folded,
            area.height.saturating_sub(10),
        );
        let rect = Rect::new(x, area.bottom().saturating_sub(height), width, height);
        draw_process_area(
            buffer,
            rect,
            &view,
            state.orcha_ui.process_frame,
            config,
            &t,
            hits,
        );
        rect
    } else {
        Rect::new(x, area.bottom(), width, 0)
    };

    // One continuous project list. Details belong below the selected name.
    let label_y = area.y + 2;
    put_text(
        buffer,
        x,
        label_y,
        width,
        &format!("PROJECTS · {}", snapshot.workspaces.len()),
        Style::default()
            .fg(t.muted)
            .bg(t.bg)
            .add_modifier(Modifier::BOLD),
    );
    // orcha keeps its sidebar open: no fold button

    let entries = super::sidebar::workspace_entries(snapshot, state.collapsed_groups);
    let rows_by_workspace = project_rows_by_workspace(snapshot);
    let empty = ProjectRows::default();
    let mut items = Vec::new();
    for entry in &entries {
        let Some(workspace) = snapshot.workspaces.get(entry.index) else {
            continue;
        };
        let project = rows_by_workspace
            .get(workspace.workspace_id.as_str())
            .unwrap_or(&empty);
        let expanded = !state
            .orcha_ui
            .folded_projects
            .contains(&workspace.workspace_id)
            && (workspace.focused
                || state
                    .orcha_ui
                    .open_projects
                    .contains(&workspace.workspace_id));
        items.push((workspace, project, expanded));
    }
    let heights: Vec<u16> = items
        .iter()
        .map(|(workspace, project, expanded)| {
            if !expanded {
                1
            } else {
                let branch = shown_branch(workspace, state.focused_git).is_some();
                // The last row stays blank to separate the next project.
                2u16.saturating_add(u16::from(branch))
                    .saturating_add(u16::try_from(project.rows.len()).unwrap_or(u16::MAX))
            }
        })
        .collect();
    let gaps: [u16; 0] = [];
    let body = Rect::new(x, label_y + 1, width, process.y.saturating_sub(label_y + 1));
    hits.workspace_body = body;
    let mut metrics =
        super::scroll::list_scroll_metrics(&heights, &gaps, body.height, *state.workspace_scroll);
    if !body.is_empty() && std::mem::take(state.reveal_focused_workspace) {
        if let Some(target) = items.iter().position(|(workspace, _, _)| workspace.focused) {
            *state.workspace_scroll = super::scroll::list_scroll_start_to_reveal(
                &heights,
                &gaps,
                body.height,
                *state.workspace_scroll,
                target,
            );
            metrics = super::scroll::list_scroll_metrics(
                &heights,
                &gaps,
                body.height,
                *state.workspace_scroll,
            );
        }
    }
    hits.workspace_max_scroll = metrics.max_offset_from_bottom;
    hits.workspace_scroll_metrics = Some(metrics);
    *state.workspace_scroll = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);

    let row_width = width.saturating_sub(1);
    let mut y = body.y;
    for (index, (workspace, project, expanded)) in
        items.iter().enumerate().skip(*state.workspace_scroll)
    {
        // A project with many panes must still expose its header and first panes.
        // Match list_scroll_metrics' clamping of an oversized first item.
        let height = heights[index].min(body.height);
        if y.saturating_add(height) > body.bottom() {
            break;
        }
        let content = if *expanded {
            height.saturating_sub(1).max(1)
        } else {
            height
        };
        let icon = state
            .orcha_ui
            .project_icons
            .get(super::orcha_icons::key(workspace))
            .map(String::as_str);
        render_project(
            buffer,
            Rect::new(x, y, row_width, content),
            workspace,
            shown_branch(workspace, state.focused_git).as_deref(),
            icon,
            project,
            *expanded,
            config.icon_font,
            &t,
            hits,
        );
        y += height;
    }
    if metrics.max_offset_from_bottom > 0 {
        let track = Rect::new(area.right().saturating_sub(1), body.y, 1, body.height);
        hits.workspace_scrollbar = track;
        super::scroll::render_list_scrollbar(buffer, track, metrics, &config.palette);
    }
    if let Some(row) = state
        .workspace_drop_indicator_row
        .filter(|row| *row >= body.y && *row < body.bottom())
    {
        put_text(
            buffer,
            body.x,
            row,
            body.width,
            &"━".repeat(usize::from(body.width)),
            Style::default().fg(t.pink).bg(t.bg),
        );
    }
}

fn tab_number_width(tab: usize) -> u16 {
    if tab == usize::MAX {
        1
    } else {
        u16::try_from(tab.to_string().len()).unwrap_or(1)
    }
}

/// The branch under the focused project. The right panel's read wins when
/// there is one: it follows the focused pane, so both sides name the same
/// checkout. The workspace's own branch (its first pane's folder) is the
/// fallback while the panel is off or has not read yet.
fn shown_branch(
    workspace: &ClientShellWorkspace,
    git_panel: Option<&super::git_panel::GitPanelData>,
) -> Option<String> {
    if !workspace.focused {
        return None;
    }
    match git_panel {
        Some(data) if data.branch.is_empty() => Some("(no branch)".to_owned()),
        Some(data) => Some(data.branch.clone()),
        None => workspace.branch.clone(),
    }
}

/// Project row with an optional branch and its panes as a tree grouped by tab.
#[allow(clippy::too_many_arguments)] // Render inputs are independent view facts.
fn render_project(
    buffer: &mut Buffer,
    rect: Rect,
    workspace: &ClientShellWorkspace,
    branch: Option<&str>,
    icon: Option<&str>,
    project: &ProjectRows<'_>,
    expanded: bool,
    icon_font: bool,
    t: &Brutal,
    hits: &mut ShellHitMap,
) {
    let fill = if workspace.focused { t.card } else { t.bg };
    buffer.set_style(rect, Style::default().bg(fill));
    if workspace.focused {
        for y in rect.y..rect.bottom() {
            put_text(
                buffer,
                rect.x,
                y,
                1,
                "▌",
                Style::default().fg(t.mark(t.yellow, fill)).bg(fill),
            );
        }
    }
    let arrow = Rect::new(rect.x + 1, rect.y, 2, 1);
    put_text(
        buffer,
        arrow.x,
        arrow.y,
        arrow.width,
        if expanded { "▾" } else { "▸" },
        Style::default().fg(t.muted).bg(fill),
    );
    hits.orcha
        .project_toggles
        .push((arrow, workspace.workspace_id.clone()));
    let name_x = rect.x + 3;
    let right = rect.right().saturating_sub(1);
    let available = right.saturating_sub(name_x);
    // Reserve at least half the row for the name. Narrow sidebars show fewer logos.
    let summary_budget = available / 2;
    let label = project.activity.label();
    let label = if display_width(label) <= summary_budget {
        label
    } else {
        match project.activity {
            ProjectActivity::NeedsYou => "wait",
            ProjectActivity::Running => "run",
            ProjectActivity::Done => "done",
            ProjectActivity::Idle => "idle",
        }
    };
    let status_width = display_width(label).min(summary_budget);
    let mut logo_count = 0usize;
    let mut logos_width = 0u16;
    if !expanded {
        for agent in project.logos.iter().flatten() {
            let width = display_width(brutal::agent_logo(agent.agent.as_deref(), icon_font));
            if status_width + 1 + logos_width + width > summary_budget {
                break;
            }
            logos_width += width;
            logo_count += 1;
        }
    }
    let summary_width = status_width + logos_width + u16::from(logo_count > 0);
    let summary_x = right.saturating_sub(summary_width);
    let name_width = summary_x.saturating_sub(name_x + 1);
    put_text(
        buffer,
        name_x,
        rect.y,
        name_width,
        &brutal::clip(
            &match icon {
                Some(icon) => Cow::Owned(format!("{icon} {}", workspace.label)),
                None => Cow::Borrowed(workspace.label.as_str()),
            },
            usize::from(name_width),
        ),
        if workspace.focused {
            Style::default()
                .fg(t.ink)
                .bg(fill)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.ink).bg(fill)
        },
    );
    put_text(
        buffer,
        summary_x,
        rect.y,
        status_width,
        label,
        Style::default()
            .fg(t.text(project.activity.color(t), fill))
            .bg(fill),
    );
    let mut logo_x = summary_x + status_width + 1;
    for agent in project.logos.iter().flatten().take(logo_count) {
        let logo = brutal::agent_logo(agent.agent.as_deref(), icon_font);
        let width = display_width(logo);
        put_text(
            buffer,
            logo_x,
            rect.y,
            width,
            logo,
            Style::default()
                .fg(t.mark(agent_color(agent, t), fill))
                .bg(fill),
        );
        logo_x += width;
    }
    hits.workspaces.push(WorkspaceHit {
        rect: Rect::new(name_x, rect.y, right.saturating_sub(name_x), 1),
        endpoint_id: ClientEndpointId::Local,
        workspace_id: workspace.workspace_id.clone(),
        indented: false,
        group_toggle: None,
    });
    if !expanded {
        return;
    }
    let mut y = rect.y + 1;
    if let Some(branch) = branch {
        if y < rect.bottom() {
            put_text(
                buffer,
                name_x,
                y,
                available,
                &brutal::clip(&format!("git · {branch}"), usize::from(available)),
                Style::default().fg(t.muted).bg(fill),
            );
            y += 1;
        }
    }
    // Tab numbers only help when there is more than one tab.
    let number_width = if project.tabs > 1 {
        project
            .rows
            .iter()
            .map(|(tab, _)| tab_number_width(*tab))
            .max()
            .unwrap_or(1)
            + 1
    } else {
        0
    };
    let last_tab = project.rows.last().map(|(tab, _)| *tab);
    let guide_x = rect.x + 2;
    let content_x = guide_x + 2 + number_width;
    let content_width = right.saturating_sub(content_x);
    for (index, (tab, project_row)) in project.rows.iter().enumerate() {
        if y >= rect.bottom() {
            break;
        }
        let first_in_tab = index == 0 || project.rows[index - 1].0 != *tab;
        let in_last_tab = Some(*tab) == last_tab;
        let guide = match (first_in_tab, in_last_tab) {
            (true, true) => "└",
            (true, false) => "├",
            (false, true) => " ",
            (false, false) => "│",
        };
        put_text(
            buffer,
            guide_x,
            y,
            1,
            guide,
            Style::default().fg(t.muted).bg(fill),
        );
        if number_width > 0 && first_in_tab && *tab != usize::MAX {
            let number = tab.to_string();
            put_text(
                buffer,
                guide_x + 2,
                y,
                number_width,
                &number,
                Style::default().fg(t.muted).bg(fill),
            );
        }
        let (focused, pane_id, name, task, status, logo, color) = match project_row {
            ProjectRow::Agent(agent) => (
                agent.focused,
                agent.pane_id.as_str(),
                row_name(&brutal::agent_label(agent)),
                brutal::agent_task(agent),
                agent.agent_status,
                brutal::agent_logo(agent.agent.as_deref(), icon_font),
                agent_color(agent, t),
            ),
            ProjectRow::Shell(pane) => (
                pane.focused,
                pane.pane_id.as_str(),
                pane.label
                    .as_deref()
                    .filter(|label| !label.is_empty())
                    .unwrap_or("Shell")
                    .to_owned(),
                String::new(),
                AgentStatus::Idle,
                "›_",
                t.muted,
            ),
        };
        let row = Rect::new(content_x, y, content_width, 1);
        let row_bg = if focused { t.frame } else { fill };
        buffer.set_style(row, Style::default().bg(row_bg));
        let logo_width = display_width(logo);
        let active = matches!(status, AgentStatus::Working | AgentStatus::Blocked);
        let icon_style = Style::default().fg(t.mark(color, row_bg)).bg(row_bg);
        put_text(
            buffer,
            row.x,
            y,
            logo_width,
            logo,
            if active {
                icon_style.add_modifier(Modifier::BOLD)
            } else {
                icon_style
            },
        );
        let text_x = row.x + logo_width + 1;
        let note = if status == AgentStatus::Blocked {
            "needs you"
        } else {
            ""
        };
        let note_width = display_width(note);
        let text_width = right.saturating_sub(text_x + note_width + u16::from(!note.is_empty()));
        let name_width = display_width(&name).min(text_width);
        let name_style = Style::default().fg(t.ink).bg(row_bg);
        put_text(
            buffer,
            text_x,
            y,
            name_width,
            &brutal::clip(&name, usize::from(name_width)),
            if active || focused {
                name_style.add_modifier(Modifier::BOLD)
            } else {
                name_style
            },
        );
        // Two spaces keep the agent's name apart from its task.
        let task_x = text_x + name_width + 2;
        let task_width = text_width.saturating_sub(name_width + 2);
        put_text(
            buffer,
            task_x,
            y,
            task_width,
            &brutal::clip(&task, usize::from(task_width)),
            Style::default().fg(t.muted).bg(row_bg),
        );
        if !note.is_empty() {
            put_right_text(
                buffer,
                row,
                y,
                note,
                Style::default().fg(t.text(t.red, row_bg)).bg(row_bg),
            );
        }
        hits.agents.push((row, pane_id.to_owned()));
        y += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reported_stage_wins_while_working() {
        let tokens = vec![("stage".to_owned(), "testing".to_owned())];
        assert_eq!(station(&tokens, AgentStatus::Working), 2);
        assert_eq!(station(&[], AgentStatus::Working), 1);
        assert_eq!(
            station(&tokens, AgentStatus::Done),
            3,
            "done arrives whatever the token says"
        );
    }

    #[test]
    fn row_names_read_naturally() {
        assert_eq!(row_name("OPUS 5.5"), "Opus 5.5");
        assert_eq!(row_name("GPT-6 LUNA"), "GPT-6 Luna");
        assert_eq!(row_name("Claude"), "Claude");
    }

    #[test]
    fn trains_chug_forward_and_back() {
        let positions: Vec<u16> = (0..8).map(|frame| chug(frame, 3)).collect();
        assert_eq!(positions, vec![0, 1, 2, 3, 2, 1, 0, 1]);
    }
}
