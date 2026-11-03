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
    /// An agent is working or waiting for you: its project opens by itself.
    fn busy(self) -> bool {
        matches!(self, Self::Running | Self::NeedsYou)
    }

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

/// A project with an agent working or waiting for you opens by itself.
pub(super) fn project_busy(snapshot: &ClientShellSnapshot, workspace_id: &str) -> bool {
    snapshot.agents.iter().any(|agent| {
        agent.workspace_id == workspace_id
            && ProjectActivity::from_status(agent.agent_status).busy()
    })
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

/// The sidebar laid out before it is drawn: where PROCESS and the project
/// list go, each project's rows and height, and the list's scroll, already
/// moved to reveal the focused project and clamped to the list. Drawing
/// only reads it.
pub(super) struct SidebarTree<'a> {
    /// PROCESS at the sidebar's foot, with its rect; None when it lives
    /// elsewhere.
    process: Option<(ProcessView<'a>, Rect)>,
    /// None when the sidebar is too small for anything but its paper.
    list: Option<ProjectList<'a>>,
}

struct ProjectList<'a> {
    x: u16,
    width: u16,
    label_y: u16,
    body: Rect,
    focused_git: Option<&'a super::git_panel::GitPanelData>,
    rows_by_workspace: HashMap<&'a str, ProjectRows<'a>>,
    /// Each shown project and whether it is open.
    items: Vec<(&'a ClientShellWorkspace, bool)>,
    heights: Vec<u16>,
    /// The first project drawn.
    scroll: usize,
    metrics: crate::pane::ScrollMetrics,
    /// Agents whose account runs low on fuel, by pane.
    low_fuel: HashMap<&'a str, super::accounts::FuelMark>,
}

/// Lays the sidebar out in `area` (see `SidebarTree`). The only state it
/// changes is the client's scroll: `scroll` comes back clamped, and moved
/// to the focused project when `reveal` asks for it (which it clears).
#[allow(clippy::too_many_arguments)] // one layout step, all inputs are distinct facts
pub(super) fn layout_sidebar_tree<'a>(
    area: Rect,
    snapshot: &'a ClientShellSnapshot,
    ui: &super::OrchaUiState,
    collapsed_groups: &HashSet<String>,
    focused_git: Option<&'a super::git_panel::GitPanelData>,
    accounts: &super::accounts::AccountsPoller,
    orcha: &crate::config::OrchaConfig,
    process_here: bool,
    scroll: &mut usize,
    reveal: &mut bool,
) -> SidebarTree<'a> {
    if area.width < 12 || area.height < 6 {
        return SidebarTree {
            process: None,
            list: None,
        };
    }
    // a column of paper on the window's edge: the terminal stretches edge
    // cells' colors into its padding
    let x = area.x + 1;
    let width = area.width.saturating_sub(1);

    // PROCESS at the foot (unless it lives in the right panel or is off)
    let process = process_here.then(|| {
        let view = process_view(snapshot, ui.process_folded);
        let height = process_height(&view, ui.process_folded, area.height.saturating_sub(10));
        let rect = Rect::new(x, area.bottom().saturating_sub(height), width, height);
        (view, rect)
    });
    let process_y = process.as_ref().map_or(area.bottom(), |(_, rect)| rect.y);

    let label_y = area.y + 2;
    let focused_git = focused_pane_git(snapshot, focused_git);
    let entries = super::sidebar::workspace_entries(snapshot, collapsed_groups);
    let rows_by_workspace = project_rows_by_workspace(snapshot);
    let empty = ProjectRows::default();
    let mut items = Vec::with_capacity(entries.len());
    let mut heights = Vec::with_capacity(entries.len());
    for entry in &entries {
        let Some(workspace) = snapshot.workspaces.get(entry.index) else {
            continue;
        };
        let project = rows_by_workspace
            .get(workspace.workspace_id.as_str())
            .unwrap_or(&empty);
        let expanded = ui.project_expanded(
            &workspace.workspace_id,
            workspace.focused || project.activity.busy(),
        );
        heights.push(if !expanded {
            1
        } else {
            // No blank rows: the indent shows where a project ends.
            project.rows.iter().fold(1u16, |height, (_, row)| {
                height
                    .saturating_add(1)
                    .saturating_add(sub_lines(row, ui.usage.as_ref()))
            })
        });
        items.push((workspace, expanded));
    }
    let gaps: [u16; 0] = [];
    let body = Rect::new(x, label_y + 1, width, process_y.saturating_sub(label_y + 1));
    if !body.is_empty() && std::mem::take(reveal) {
        if let Some(target) = items.iter().position(|(workspace, _)| workspace.focused) {
            *scroll = super::scroll::list_scroll_start_to_reveal(
                &heights,
                &gaps,
                body.height,
                *scroll,
                target,
            );
        }
    }
    let metrics = super::scroll::list_scroll_metrics(&heights, &gaps, body.height, *scroll);
    *scroll = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    SidebarTree {
        process,
        list: Some(ProjectList {
            x,
            width,
            label_y,
            body,
            focused_git,
            rows_by_workspace,
            items,
            heights,
            scroll: *scroll,
            metrics,
            low_fuel: accounts.low_fuel_agents(snapshot, orcha),
        }),
    }
}

/// Draws the sidebar `layout_sidebar_tree` laid out.
pub(crate) fn render_sidebar_tree(
    buffer: &mut Buffer,
    area: Rect,
    tree: &SidebarTree<'_>,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    state: &ShellRenderState<'_>,
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
    let Some(list) = &tree.list else {
        return;
    };
    let (x, width) = (list.x, list.width);

    render_buttons(buffer, x, area.y, width, snapshot, config, &t, hits);

    if let Some((view, rect)) = &tree.process {
        draw_process_area(
            buffer,
            *rect,
            view,
            state.orcha_ui.process_frame,
            config,
            &t,
            hits,
        );
    }

    // One continuous project list: the label on the left, the count on the
    // right, both lined up with the rows below.
    let label_y = list.label_y;
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

    let body = list.body;
    let metrics = list.metrics;
    hits.workspace_body = body;
    hits.workspace_max_scroll = metrics.max_offset_from_bottom;
    hits.workspace_scroll_metrics = Some(metrics);

    let empty = ProjectRows::default();
    let marks = RowMarks {
        low_fuel: &list.low_fuel,
        handing_off: state.orcha_ui.handoff_source.as_deref(),
        continued: &state.orcha_ui.continued,
        usage: state.orcha_ui.usage.as_ref(),
    };
    let mut y = body.y;
    for (index, (workspace, expanded)) in list.items.iter().enumerate().skip(list.scroll) {
        // A project with many panes must still expose its header and first panes.
        // Match list_scroll_metrics' clamping of an oversized first item.
        let height = list.heights[index].min(body.height);
        if y.saturating_add(height) > body.bottom() {
            break;
        }
        let project = list
            .rows_by_workspace
            .get(workspace.workspace_id.as_str())
            .unwrap_or(&empty);
        let icon = state
            .orcha_ui
            .project_icons
            .get(super::orcha_icons::key(workspace))
            .map(String::as_str);
        render_project(
            buffer,
            Rect::new(x, y, row_width, height),
            workspace,
            project_git(workspace, list.focused_git),
            icon,
            project,
            *expanded,
            &marks,
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
/// What orcha read about an agent's pane (`orcha_usage`), if anything.
fn pane_usage<'u>(
    agent: &ClientShellAgent,
    usage: Option<&'u crate::orcha_usage::Usage>,
) -> Option<&'u crate::orcha_usage::PaneUsage> {
    usage?.panes.get(agent.pane_id.as_str())
}

/// The lines under a pane row. An agent has its model and branch, then,
/// once read, its context and memory; a shell has none. Layout and drawing
/// both ask here, so the heights always match.
fn sub_lines(row: &ProjectRow<'_>, usage: Option<&crate::orcha_usage::Usage>) -> u16 {
    match row {
        ProjectRow::Agent(agent) => {
            let cost = pane_usage(agent, usage)
                .is_some_and(|cost| cost.context.is_some() || cost.memory.is_some());
            1 + u16::from(cost)
        }
        ProjectRow::Shell(_) => 0,
    }
}

/// Picks the long form of a line when it fits `room`, else the short one.
fn fitting(
    long: Vec<(String, Style)>,
    room: u16,
    short: impl FnOnce() -> Vec<(String, Style)>,
) -> Vec<(String, Style)> {
    let width: u16 = long.iter().map(|(text, _)| display_width(text)).sum();
    if width <= room {
        long
    } else {
        short()
    }
}

/// An agent's first line: its model, then the branch of the folder it is
/// in (`◇` in a linked worktree). The project's counts (commits to push
/// and pull, files changed) join only when the agent is on the project's
/// own branch, so they never describe a checkout they weren't read from.
fn who_parts(
    agent: &ClientShellAgent,
    cost: Option<&crate::orcha_usage::PaneUsage>,
    git: &ProjectGit<'_>,
    room: u16,
    t: &Brutal,
    bg: Color,
) -> Vec<(String, Style)> {
    let quiet = Style::default().fg(t.muted).bg(bg);
    let count = |color: Color| {
        Style::default()
            .fg(t.text(color, bg))
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    };
    let model = row_name(&brutal::agent_label(agent));
    let model_width = display_width(&model);
    let mut parts = vec![(model, Style::default().fg(t.ink).bg(bg))];
    // the agent's own folder when read, else the project's
    let (name, worktree) = match cost.and_then(|cost| cost.branch.as_ref()) {
        Some(branch) => (Some(branch.name.as_str()), branch.worktree),
        None => (git.branch, git.worktree),
    };
    let Some(name) = name else {
        return parts;
    };
    // the counts go after the branch; a long branch name gives way first
    let mut counts: Vec<(String, Style)> = Vec::new();
    if git.branch == Some(name) {
        if git.ahead > 0 || git.behind > 0 {
            counts.push((" ".to_owned(), quiet));
        }
        if git.ahead > 0 {
            counts.push((format!("↑{}", git.ahead), count(t.green)));
        }
        if git.behind > 0 {
            counts.push((format!("↓{}", git.behind), count(t.blue)));
        }
        if let Some(dirty) = git.dirty.filter(|dirty| *dirty > 0) {
            counts.push((format!(" ±{dirty}"), count(t.yellow)));
        }
    }
    let counts_width: u16 = counts.iter().map(|(text, _)| display_width(text)).sum();
    // "model · ⎇ " before the name
    let name_room = room.saturating_sub(model_width + 5 + counts_width).max(4);
    let mark = if worktree { "◇" } else { "⎇" };
    parts.push((" · ".to_owned(), quiet));
    parts.push((
        format!("{mark} {}", brutal::clip(name, usize::from(name_room))),
        Style::default().fg(t.text(t.pink, bg)).bg(bg),
    ));
    parts.extend(counts);
    parts
}

/// An agent's second line: `context 636k · RAM 1.3G`. The numbers are
/// bold: green while fine, yellow when worth a look, red when too much.
/// `ctx` for `context` when the long word doesn't fit.
fn cost_parts(
    cost: &crate::orcha_usage::PaneUsage,
    room: u16,
    t: &Brutal,
    bg: Color,
) -> Vec<(String, Style)> {
    let quiet = Style::default().fg(t.muted).bg(bg);
    let count = |color: Color| {
        Style::default()
            .fg(t.text(color, bg))
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    };
    let build = |long: bool| {
        let mut parts = Vec::new();
        if let Some(context) = cost.context {
            let color = match context.percent() {
                Some(percent) => t.context(percent),
                None if context.heavy() => t.yellow,
                None => t.green,
            };
            parts.push((if long { "context " } else { "ctx " }.to_owned(), quiet));
            parts.push((context.short(), count(color)));
        }
        if let Some(memory) = cost.memory {
            if cost.context.is_some() {
                parts.push((" · ".to_owned(), quiet));
            }
            parts.push(("RAM ".to_owned(), quiet));
            parts.push((
                crate::orcha_usage::short_bytes(memory),
                count(t.memory(memory).unwrap_or(t.green)),
            ));
        }
        parts
    };
    fitting(build(true), room, || build(false))
}

/// A project's git, as its agents' lines tell it.
#[derive(Debug, Clone, Copy, Default)]
struct ProjectGit<'a> {
    branch: Option<&'a str>,
    worktree: bool,
    ahead: usize,
    behind: usize,
    /// Files changed: only read for the open project.
    dirty: Option<usize>,
}

/// The open project's git comes from the git panel's read of its pane
/// (when that read is about this project), the others' from the server.
fn project_git<'a>(
    workspace: &'a ClientShellWorkspace,
    git: Option<&'a super::git_panel::GitPanelData>,
) -> ProjectGit<'a> {
    let worktree = workspace
        .worktree
        .as_ref()
        .is_some_and(|worktree| worktree.is_linked_worktree);
    match git.filter(|_| workspace.focused) {
        Some(data) => ProjectGit {
            branch: Some(if data.branch.is_empty() {
                "(no branch)"
            } else {
                data.branch.as_str()
            }),
            worktree,
            ahead: data.ahead,
            behind: data.behind,
            dirty: Some(data.files.len()),
        },
        None => {
            let (ahead, behind) = workspace.git_ahead_behind.unwrap_or((0, 0));
            ProjectGit {
                branch: workspace.branch.as_deref(),
                worktree,
                ahead,
                behind,
                dirty: None,
            }
        }
    }
}

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
/// Cells kept for a project's icon, so every name starts in the same column.
const ICON_WIDTH: u16 = 2;

/// A hand-off on an agent's row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandoffMark<'a> {
    /// Its chat is being handed off.
    Running,
    /// Its chat goes on in this pane.
    Continued(&'a str),
}

/// What agent rows show besides their state: fuel running low on their
/// account, hand-offs, and what each pane costs (context, memory).
struct RowMarks<'a> {
    low_fuel: &'a HashMap<&'a str, super::accounts::FuelMark>,
    handing_off: Option<&'a str>,
    continued: &'a HashMap<String, super::ContinuedPane>,
    usage: Option<&'a crate::orcha_usage::Usage>,
}

impl<'a> RowMarks<'a> {
    fn handoff(&self, pane: &str) -> Option<HandoffMark<'a>> {
        if self.handing_off == Some(pane) {
            return Some(HandoffMark::Running);
        }
        self.continued
            .get(pane)
            .map(|next| HandoffMark::Continued(next.pane.as_str()))
    }
}

/// One project: its name row, then (when open) its branch and panes,
/// indented under the name. Idle says nothing; busy projects show their
/// agents' logos and one word on the right.
#[allow(clippy::too_many_arguments)] // Render inputs are independent view facts.
fn render_project(
    buffer: &mut Buffer,
    rect: Rect,
    workspace: &ClientShellWorkspace,
    git: ProjectGit<'_>,
    icon: Option<&str>,
    project: &ProjectRows<'_>,
    expanded: bool,
    marks: &RowMarks<'_>,
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
        // the full-size triangles: the small ones read as dots
        if expanded { "▼" } else { "▶" },
        Style::default().fg(t.muted).bg(fill),
    );
    // the click area is wider than the arrow: the edge column counts too
    hits.orcha.project_toggles.push((
        Rect::new(rect.x, rect.y, 3, 1),
        workspace.workspace_id.clone(),
    ));

    let icon_x = rect.x + 3;
    // no icon: the name moves up to where it would be
    let name_x = if icon.is_some() {
        icon_x + ICON_WIDTH + 1
    } else {
        icon_x
    };
    let right = rect.right().saturating_sub(1);
    if let Some(icon) = icon {
        put_text(
            buffer,
            icon_x,
            rect.y,
            ICON_WIDTH,
            icon,
            Style::default().bg(fill),
        );
    }

    // Right side: logos (folded only) then the word. At most half the row,
    // so the name keeps its room.
    let budget = right.saturating_sub(name_x) / 2;
    // open, its agents say how they are doing; folded, the project does
    let word = if expanded {
        None
    } else {
        project.activity.label(budget)
    };
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

    // Panes hang off the project like a tree: `├`/`└` in the connector
    // column, the open pane's branch heavy and yellow (`┣━`). Under each
    // agent, its model and branch, then what it costs.
    let tree_x = icon_x;
    let logo_x = tree_x + 2;
    let mut y = rect.y + 1;
    // Tab numbers sit left of the tree, on each tab's first pane, and only
    // when the project has more than one tab.
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
    let last = project.rows.len().saturating_sub(1);
    let quiet = Style::default().fg(t.muted).bg(fill);
    let lit = Style::default()
        .fg(t.mark(t.yellow, fill))
        .bg(fill)
        .add_modifier(Modifier::BOLD);
    for (index, (tab, project_row)) in project.rows.iter().enumerate() {
        if y >= rect.bottom() {
            break;
        }
        let is_last = index == last;
        let first_in_tab = index == 0 || project.rows[index - 1].0 != *tab;
        let (focused, pane_id, title, status, logo, color, shell) = match project_row {
            ProjectRow::Agent(agent) => {
                // what it works on, else which model it is
                let task = brutal::agent_task(agent);
                let title = if task.trim().is_empty() {
                    row_name(&brutal::agent_label(agent))
                } else {
                    task
                };
                (
                    agent.focused,
                    agent.pane_id.as_str(),
                    Cow::Owned(title),
                    agent.agent_status,
                    brutal::agent_logo(agent.agent.as_deref(), icon_font),
                    agent_color(agent, t),
                    false,
                )
            }
            ProjectRow::Shell(pane) => (
                pane.focused,
                pane.pane_id.as_str(),
                Cow::Borrowed(
                    pane.label
                        .as_deref()
                        .filter(|label| !label.is_empty())
                        .unwrap_or("Shell"),
                ),
                AgentStatus::Idle,
                "›_",
                t.muted,
                true,
            ),
        };
        if numbered && first_in_tab && *tab != usize::MAX {
            put_text(
                buffer,
                rect.x + 1,
                y,
                2,
                &tab.to_string(),
                if open_tab == Some(*tab) {
                    Style::default()
                        .fg(t.ink)
                        .bg(fill)
                        .add_modifier(Modifier::BOLD)
                } else {
                    quiet
                },
            );
        }
        let branch = match (focused, is_last) {
            (true, _) => "┣━",
            (false, true) => "└",
            (false, false) => "├",
        };
        put_text(
            buffer,
            tree_x,
            y,
            2,
            branch,
            if focused { lit } else { quiet },
        );
        let active = matches!(status, AgentStatus::Working | AgentStatus::Blocked);
        let logo_width = display_width(logo);
        let icon_style = Style::default().fg(t.mark(color, fill)).bg(fill);
        put_text(
            buffer,
            logo_x,
            y,
            logo_width,
            logo,
            if active {
                icon_style.add_modifier(Modifier::BOLD)
            } else {
                icon_style
            },
        );
        let text_x = logo_x + logo_width + 1;
        let cost = match project_row {
            ProjectRow::Agent(agent) => pane_usage(agent, marks.usage),
            ProjectRow::Shell(_) => None,
        };
        // Every agent names its state on the right (and, when it isn't
        // working, for how long); shells have none. The long word unless
        // it would squeeze the title. A hand-off under way or done says so
        // in its place.
        let state = ProjectActivity::from_status(status);
        let room = right
            .saturating_sub(text_x)
            .saturating_sub(display_width(&title).min(8) + 1);
        let handoff = if shell { None } else { marks.handoff(pane_id) };
        let (note, note_color, note_bold): (Cow<'_, str>, Color, bool) = match handoff {
            Some(HandoffMark::Running) => (
                Cow::Borrowed(if room >= 12 {
                    "handing off…"
                } else if room >= 7 {
                    "handoff"
                } else {
                    ""
                }),
                t.blue,
                false,
            ),
            Some(HandoffMark::Continued(_)) => (
                Cow::Borrowed(if room >= 11 {
                    "→ continued"
                } else if room >= 7 {
                    "→ moved"
                } else {
                    ""
                }),
                t.blue,
                true,
            ),
            None if shell => (Cow::Borrowed(""), t.muted, false),
            None => {
                let word = state.word(room);
                let since = cost
                    .and_then(|cost| cost.quiet_for)
                    .filter(|_| status != AgentStatus::Working)
                    .and_then(crate::orcha_usage::age)
                    .map(|age| format!("{word} {age}"))
                    .filter(|long| !word.is_empty() && display_width(long) <= room);
                (
                    since.map_or(Cow::Borrowed(word), Cow::Owned),
                    state.color(t),
                    state == ProjectActivity::NeedsYou,
                )
            }
        };
        let note_width = display_width(&note);
        // left of the note, while it fits: fuel running low on the account
        // (red when orcha would offer a switch)
        let mut fuel: Option<(String, Color)> = None;
        if let Some(mark) = marks.low_fuel.get(pane_id).filter(|_| !shell) {
            let color = if mark.empty { t.red } else { t.yellow };
            let fits = |text: &str| note_width + display_width(text) < room;
            let long = format!("fuel {}%", mark.left);
            let short = format!("{}%", mark.left);
            if fits(&long) {
                fuel = Some((long, color));
            } else if fits(&short) {
                fuel = Some((short, color));
            }
        }
        let fuel_width = fuel.as_ref().map_or(0, |(text, _)| display_width(text) + 1);
        let marks_width = note_width + u16::from(note_width > 0) + fuel_width;
        let title_width = display_width(&title).min(right.saturating_sub(text_x + marks_width));
        // shells step back so agents stand out, unless one is open
        let title_style = Style::default()
            .fg(if shell && !focused { t.muted } else { t.ink })
            .bg(fill);
        put_text(
            buffer,
            text_x,
            y,
            title_width,
            &brutal::clip(&title, usize::from(title_width)),
            if active || focused {
                title_style.add_modifier(Modifier::BOLD)
            } else {
                title_style
            },
        );
        if note_width > 0 {
            let style = Style::default().fg(t.text(note_color, fill)).bg(fill);
            put_right_text(
                buffer,
                Rect::new(text_x, y, right.saturating_sub(text_x), 1),
                y,
                &note,
                if note_bold {
                    style.add_modifier(Modifier::BOLD)
                } else {
                    style
                },
            );
        }
        if let Some((text, color)) = &fuel {
            let end = right.saturating_sub(note_width + u16::from(note_width > 0));
            put_right_text(
                buffer,
                Rect::new(text_x, y, end.saturating_sub(text_x), 1),
                y,
                text,
                Style::default()
                    .fg(t.text(*color, fill))
                    .bg(fill)
                    .add_modifier(Modifier::BOLD),
            );
        }
        // "→ continued" goes to the new pane; listed first, so it wins
        // over the row's own click
        if let Some(HandoffMark::Continued(next)) = handoff.filter(|_| note_width > 0) {
            hits.agents.push((
                Rect::new(right.saturating_sub(note_width), y, note_width, 1),
                next.to_owned(),
            ));
        }
        hits.agents.push((
            Rect::new(tree_x, y, right.saturating_sub(tree_x), 1),
            pane_id.to_owned(),
        ));
        y += 1;

        // the agent's own lines: part of the same pane (same click)
        let ProjectRow::Agent(agent) = project_row else {
            continue;
        };
        let room = right.saturating_sub(logo_x + 2);
        let who = who_parts(agent, cost, &git, room, t, fill);
        let costs = cost
            .filter(|cost| cost.context.is_some() || cost.memory.is_some())
            .map(|cost| cost_parts(cost, room, t, fill));
        let lines = [Some(who), costs];
        let count = lines.iter().flatten().count();
        for (line, parts) in lines.into_iter().flatten().enumerate() {
            if y >= rect.bottom() {
                break;
            }
            if !is_last {
                put_text(buffer, tree_x, y, 1, "│", quiet);
            }
            let end = line + 1 == count;
            let (glyph, style) = match (focused, end) {
                (true, true) => ("┗", lit),
                (true, false) => ("┣", lit),
                (false, true) => ("└", quiet),
                (false, false) => ("├", quiet),
            };
            put_text(buffer, logo_x, y, 1, glyph, style);
            let mut x = logo_x + 2;
            for (text, style) in parts {
                let width = display_width(&text).min(right.saturating_sub(x));
                if width == 0 {
                    break;
                }
                put_text(
                    buffer,
                    x,
                    y,
                    width,
                    &brutal::clip(&text, usize::from(width)),
                    style,
                );
                x += width;
            }
            hits.agents.push((
                Rect::new(tree_x, y, right.saturating_sub(tree_x), 1),
                pane_id.to_owned(),
            ));
            y += 1;
        }
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
            project_git(&snapshot.workspaces[0], git).branch,
            Some("panel-branch")
        );
        // a read left over from another project: the server's branch shows
        let elsewhere = read("/other-project");
        let git = focused_pane_git(&snapshot, Some(&elsewhere));
        assert_eq!(
            project_git(&snapshot.workspaces[0], git).branch,
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
