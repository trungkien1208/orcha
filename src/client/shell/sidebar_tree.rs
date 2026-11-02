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

/// Most urgent first: needs you, working, done, the rest.
pub(super) fn status_rank(status: AgentStatus) -> u8 {
    match status {
        AgentStatus::Blocked => 0,
        AgentStatus::Working => 1,
        AgentStatus::Done => 2,
        _ => 3,
    }
}

/// An agent's mark in lists: its state, the same color everywhere (yellow
/// running, red needs you, green done, muted idle), so a glance tells them
/// apart whatever the agent.
pub(super) fn agent_color(agent: &ClientShellAgent, t: &Brutal) -> Color {
    t.state(agent.agent_status).unwrap_or(t.muted)
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
        let detail = match (agent.agent_status == AgentStatus::Blocked, task.is_empty()) {
            // a question waits: say so first, it is what to act on
            (true, true) => Cow::Owned(format!("needs you · {project}")),
            (true, false) => Cow::Owned(format!("needs you · {project} · {task}")),
            (false, true) => Cow::Borrowed(*project),
            (false, false) => Cow::Owned(format!("{project} · {task}")),
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

    /// The state as one word, short when `room` is tight.
    fn word(self, room: u16) -> &'static str {
        let (long, short) = match self {
            Self::Idle => ("idle", "idle"),
            Self::Done => ("done", "done"),
            Self::Running => ("running", "run"),
            Self::NeedsYou => ("needs you", "wait"),
        };
        if display_width(long) <= room {
            long
        } else {
            short
        }
    }

    /// The word beside a project. Idle projects say nothing, so the ones
    /// that are busy stand out; each agent row still names its own state.
    fn label(self, room: u16) -> Option<&'static str> {
        (self != Self::Idle).then(|| self.word(room))
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
    // At most three logos, one per kind of agent. Busier agents win a slot.
    logos: [Option<&'a ClientShellAgent>; 3],
}

impl<'a> ProjectRows<'a> {
    fn push(&mut self, tab: usize, row: ProjectRow<'a>) {
        if let ProjectRow::Agent(agent) = row {
            let activity = ProjectActivity::from_status(agent.agent_status);
            self.activity = self.activity.max(activity);
            let urgency = |seen: &ClientShellAgent| ProjectActivity::from_status(seen.agent_status);
            let kind = agent.agent.as_deref().unwrap_or("");
            // one logo per kind of agent, in its most urgent state; when the
            // slots are full, a busier agent takes the quietest one's place
            if let Some(seen) = self.logos.iter_mut().flatten().find(|seen| {
                seen.agent
                    .as_deref()
                    .unwrap_or("")
                    .eq_ignore_ascii_case(kind)
            }) {
                if urgency(seen) < activity {
                    *seen = agent;
                }
            } else if let Some(slot) = self.logos.iter_mut().find(|slot| slot.is_none()) {
                *slot = Some(agent);
            } else if let Some(slot) = self
                .logos
                .iter_mut()
                .min_by_key(|slot| slot.map(urgency))
                .filter(|slot| slot.is_some_and(|seen| urgency(seen) < activity))
            {
                *slot = Some(agent);
            }
        }
        self.rows.push((tab, row));
    }

    /// The logos, most urgent first.
    fn logos_by_urgency(&self) -> impl Iterator<Item = &'a ClientShellAgent> {
        let mut logos = self.logos;
        logos.sort_by_key(|slot| slot.map_or(u8::MAX, |agent| status_rank(agent.agent_status)));
        logos.into_iter().flatten()
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
    // Agents waiting on a question come first, then the ones working.
    let mut running_agents: Vec<_> = snapshot
        .agents
        .iter()
        .filter(|agent| running(agent))
        .collect();
    running_agents.sort_by_key(|agent| agent.agent_status != AgentStatus::Blocked);
    for agent in running_agents {
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

    // One continuous project list: the label on the left, the count on the
    // right, both lined up with the rows below.
    let label_y = area.y + 2;
    let row_width = width.saturating_sub(1);
    let label_style = Style::default()
        .fg(t.muted)
        .bg(t.bg)
        .add_modifier(Modifier::BOLD);
    put_text(
        buffer,
        x + 1,
        label_y,
        row_width.saturating_sub(1),
        "PROJECTS",
        label_style,
    );
    put_right_text(
        buffer,
        Rect::new(x, label_y, row_width.saturating_sub(1), 1),
        label_y,
        &snapshot.workspaces.len().to_string(),
        label_style,
    );
    // orcha keeps its sidebar open: no fold button

    let focused_git = focused_pane_git(snapshot, state.focused_git);
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
        let expanded = state
            .orcha_ui
            .project_expanded(&workspace.workspace_id, workspace.focused);
        items.push((workspace, project, expanded));
    }
    let heights: Vec<u16> = items
        .iter()
        .map(|(workspace, project, expanded)| {
            if !expanded {
                1
            } else {
                // No blank rows: the indent shows where a project ends.
                let branch = shown_branch(workspace, focused_git).is_some();
                1u16.saturating_add(u16::from(branch))
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
        let icon = state
            .orcha_ui
            .project_icons
            .get(super::orcha_icons::key(workspace))
            .map(String::as_str);
        render_project(
            buffer,
            Rect::new(x, y, row_width, height),
            workspace,
            shown_branch(workspace, focused_git),
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

/// The right panel's read, only when it is about the focused pane's folder.
/// Right after a switch it can still hold the previous project's read;
/// the server's branch is right in the meantime.
fn focused_pane_git<'a>(
    snapshot: &ClientShellSnapshot,
    git_panel: Option<&'a super::git_panel::GitPanelData>,
) -> Option<&'a super::git_panel::GitPanelData> {
    let data = git_panel?;
    let focused = snapshot.focused_pane_id.as_deref()?;
    let pane = snapshot.panes.iter().find(|pane| pane.pane_id == focused)?;
    let folder = pane.foreground_cwd.as_deref().or(pane.cwd.as_deref())?;
    same_or_inside(folder, &data.root).then_some(data)
}

/// `folder` is `root` or inside it, whichever slash each one uses.
fn same_or_inside(folder: &str, root: &str) -> bool {
    let slash = |c: char| if c == '\\' { '/' } else { c };
    let mut folder = folder.chars().map(slash);
    for c in root.trim_end_matches(['/', '\\']).chars().map(slash) {
        if folder.next() != Some(c) {
            return false;
        }
    }
    matches!(folder.next(), None | Some('/'))
}

/// The branch under the focused project: the right panel's read of the
/// focused pane's checkout when it has one, else the workspace's own branch
/// (its first pane's folder), which the server keeps fresh.
fn shown_branch<'a>(
    workspace: &'a ClientShellWorkspace,
    git: Option<&'a super::git_panel::GitPanelData>,
) -> Option<&'a str> {
    if !workspace.focused {
        return None;
    }
    match git {
        Some(data) if data.branch.is_empty() => Some("(no branch)"),
        Some(data) => Some(data.branch.as_str()),
        None => workspace.branch.as_deref(),
    }
}

/// Cells kept for a project's icon, so every name starts in the same column.
const ICON_WIDTH: u16 = 2;

/// One project: its name row, then (when open) its branch and panes,
/// indented under the name. Idle says nothing; busy projects show their
/// agents' logos and one word on the right.
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
    put_text(
        buffer,
        rect.x + 1,
        rect.y,
        1,
        if expanded { "▾" } else { "▸" },
        Style::default().fg(t.muted).bg(fill),
    );
    // the click area is wider than the arrow: the edge column counts too
    hits.orcha.project_toggles.push((
        Rect::new(rect.x, rect.y, 3, 1),
        workspace.workspace_id.clone(),
    ));

    let icon_x = rect.x + 3;
    let name_x = icon_x + ICON_WIDTH + 1;
    let right = rect.right().saturating_sub(1);
    match icon {
        Some(icon) => put_text(
            buffer,
            icon_x,
            rect.y,
            ICON_WIDTH,
            icon,
            Style::default().bg(fill),
        ),
        // a quiet stand-in keeps the name in line with the others
        None => put_text(
            buffer,
            icon_x,
            rect.y,
            1,
            "·",
            Style::default().fg(t.muted).bg(fill),
        ),
    }

    // Right side: logos (folded only) then the word. At most half the row,
    // so the name keeps its room.
    let budget = right.saturating_sub(name_x) / 2;
    let word = project.activity.label(budget);
    let word_width = word.map_or(0, display_width);
    let mut logo_count = 0usize;
    let mut logos_width = 0u16;
    if !expanded {
        for agent in project.logos_by_urgency() {
            let width = display_width(brutal::agent_logo(agent.agent.as_deref(), icon_font));
            let gap = u16::from(word_width > 0);
            if logos_width + width + gap + word_width > budget {
                break;
            }
            logos_width += width;
            logo_count += 1;
        }
    }
    let gap = u16::from(logo_count > 0 && word_width > 0);
    let summary_width = logos_width + gap + word_width;
    let summary_x = right.saturating_sub(summary_width);
    let name_width = summary_x
        .saturating_sub(name_x)
        .saturating_sub(u16::from(summary_width > 0));
    let name_style = Style::default().fg(t.ink).bg(fill);
    put_text(
        buffer,
        name_x,
        rect.y,
        name_width,
        &brutal::clip(&workspace.label, usize::from(name_width)),
        if workspace.focused {
            name_style.add_modifier(Modifier::BOLD)
        } else {
            name_style
        },
    );
    let mut logo_x = summary_x;
    for agent in project.logos_by_urgency().take(logo_count) {
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
    if let Some(word) = word {
        put_text(
            buffer,
            logo_x + gap,
            rect.y,
            word_width,
            word,
            Style::default()
                .fg(t.text(project.activity.color(t), fill))
                .bg(fill),
        );
    }
    hits.workspaces.push(WorkspaceHit {
        rect: Rect::new(icon_x, rect.y, right.saturating_sub(icon_x), 1),
        endpoint_id: ClientEndpointId::Local,
        workspace_id: workspace.workspace_id.clone(),
        indented: false,
        group_toggle: None,
    });
    if !expanded {
        return;
    }

    let content_width = right.saturating_sub(name_x);
    let mut y = rect.y + 1;
    if let Some(branch) = branch {
        if y < rect.bottom() {
            put_text(
                buffer,
                name_x,
                y,
                content_width,
                &brutal::clip(&format!("⎇ {branch}"), usize::from(content_width)),
                Style::default().fg(t.muted).bg(fill),
            );
            y += 1;
        }
    }
    // Tab numbers sit in the icon column, on each tab's first pane, and
    // only when the project has more than one tab.
    let numbered = project.tabs > 1;
    // the open tab's number stands out like the open pane
    let open_tab = project
        .rows
        .iter()
        .find(|(_, row)| match row {
            ProjectRow::Agent(agent) => agent.focused,
            ProjectRow::Shell(pane) => pane.focused,
        })
        .map(|(tab, _)| *tab);
    for (index, (tab, project_row)) in project.rows.iter().enumerate() {
        if y >= rect.bottom() {
            break;
        }
        let first_in_tab = index == 0 || project.rows[index - 1].0 != *tab;
        let (focused, pane_id, name, task, status, logo, color, shell) = match project_row {
            ProjectRow::Agent(agent) => (
                agent.focused,
                agent.pane_id.as_str(),
                Cow::Owned(row_name(&brutal::agent_label(agent))),
                brutal::agent_task(agent),
                agent.agent_status,
                brutal::agent_logo(agent.agent.as_deref(), icon_font),
                agent_color(agent, t),
                false,
            ),
            ProjectRow::Shell(pane) => (
                pane.focused,
                pane.pane_id.as_str(),
                Cow::Borrowed(
                    pane.label
                        .as_deref()
                        .filter(|label| !label.is_empty())
                        .unwrap_or("Shell"),
                ),
                String::new(),
                AgentStatus::Idle,
                "›_",
                t.muted,
                true,
            ),
        };
        // the pane in the open tab: a tinted row, one cell of padding before it
        let row = Rect::new(
            name_x.saturating_sub(1),
            y,
            right.saturating_sub(name_x) + 1,
            1,
        );
        let row_bg = if focused { t.frame } else { fill };
        buffer.set_style(row, Style::default().bg(row_bg));
        if numbered && first_in_tab && *tab != usize::MAX {
            put_text(
                buffer,
                icon_x,
                y,
                ICON_WIDTH,
                &tab.to_string(),
                if open_tab == Some(*tab) {
                    Style::default()
                        .fg(t.ink)
                        .bg(fill)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(t.muted).bg(fill)
                },
            );
        }
        let active = matches!(status, AgentStatus::Working | AgentStatus::Blocked);
        let logo_width = display_width(logo);
        let icon_style = Style::default().fg(t.mark(color, row_bg)).bg(row_bg);
        put_text(
            buffer,
            name_x,
            y,
            logo_width,
            logo,
            if active {
                icon_style.add_modifier(Modifier::BOLD)
            } else {
                icon_style
            },
        );
        let text_x = name_x + logo_width + 1;
        // Every agent names its state on the right; shells have none. The
        // long word unless it would squeeze the name.
        let state = ProjectActivity::from_status(status);
        let note = if shell {
            ""
        } else {
            let room = right
                .saturating_sub(text_x)
                .saturating_sub(display_width(&name).min(8) + 1);
            state.word(room)
        };
        let note_width = display_width(note);
        let text_width = right.saturating_sub(text_x + note_width + u16::from(note_width > 0));
        let name_width = display_width(&name).min(text_width);
        // shells step back so agents stand out, unless one is open
        let name_style = Style::default()
            .fg(if shell && !focused { t.muted } else { t.ink })
            .bg(row_bg);
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
        let task_width = text_width.saturating_sub(name_width + 2);
        if task_width > 0 && !task.is_empty() {
            put_text(
                buffer,
                text_x + name_width + 2,
                y,
                task_width,
                &brutal::clip(&task, usize::from(task_width)),
                Style::default().fg(t.muted).bg(row_bg),
            );
        }
        if note_width > 0 {
            let style = Style::default()
                .fg(t.text(state.color(t), row_bg))
                .bg(row_bg);
            put_right_text(
                buffer,
                Rect::new(row.x, y, right.saturating_sub(row.x), 1),
                y,
                note,
                if state == ProjectActivity::NeedsYou {
                    style.add_modifier(Modifier::BOLD)
                } else {
                    style
                },
            );
        }
        // the open pane: an ink bar on its left edge, like the open tab
        if focused {
            put_text(
                buffer,
                row.x,
                y,
                1,
                "▌",
                Style::default().fg(t.ink).bg(row_bg),
            );
        }
        hits.agents.push((
            Rect::new(icon_x, y, right.saturating_sub(icon_x), 1),
            pane_id.to_owned(),
        ));
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
    fn git_read_counts_only_for_folders_inside_its_repository() {
        assert!(same_or_inside("/repo", "/repo"));
        assert!(same_or_inside("/repo/src/app", "/repo/"));
        assert!(same_or_inside(r"C:\work\repo\src", "C:/work/repo"));
        assert!(!same_or_inside("/repository", "/repo"));
        assert!(!same_or_inside("/other", "/repo"));
        assert!(!same_or_inside("/re", "/repo"));
    }

    #[test]
    fn a_read_of_the_previous_project_gives_way_to_the_servers_branch() {
        let mut snapshot = super::super::tests::snapshot();
        snapshot.workspaces[0].focused = true;
        snapshot.workspaces[0].branch = Some("server-branch".into());
        let read = |root: &str| super::super::git_panel::GitPanelData {
            root: root.into(),
            branch: "panel-branch".into(),
            ..Default::default()
        };
        // the read is about the focused pane's repository: it wins
        let here = read("/repo");
        let git = focused_pane_git(&snapshot, Some(&here));
        assert_eq!(
            shown_branch(&snapshot.workspaces[0], git),
            Some("panel-branch")
        );
        // a read left over from another project: the server's branch shows
        let elsewhere = read("/other-project");
        let git = focused_pane_git(&snapshot, Some(&elsewhere));
        assert_eq!(
            shown_branch(&snapshot.workspaces[0], git),
            Some("server-branch")
        );
    }

    #[test]
    fn trains_chug_forward_and_back() {
        let positions: Vec<u16> = (0..8).map(|frame| chug(frame, 3)).collect();
        assert_eq!(positions, vec![0, 1, 2, 3, 2, 1, 0, 1]);
    }
}

#[cfg(test)]
mod process_station_tests {
    use super::*;

    #[test]
    fn reported_stages_map_to_stations() {
        let tokens = |stage: &str| vec![("stage".to_owned(), stage.to_owned())];
        assert_eq!(station(&tokens("thinking"), AgentStatus::Working), 0);
        assert_eq!(station(&tokens("coding"), AgentStatus::Working), 1);
        assert_eq!(station(&tokens("testing"), AgentStatus::Working), 2);
        assert_eq!(station(&[], AgentStatus::Done), 3);
    }
}
