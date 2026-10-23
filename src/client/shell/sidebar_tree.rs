//! orcha: the left sidebar (docs/orcha/SPEC.md §2 "Left sidebar", §3.1).
//!
//!   [ + NEW      ]▌ [ ≡ menu ]
//!   PROJECTS · 6                    «
//!   ┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
//!   ┃▌▾ MATCHA-HOUSE  ⎇ 42-dark  ┃▌      selected: raised, yellow edge
//!   ┃▌  ■ Opus 5.5  dark mode    ┃▌
//!   ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛▌
//!   ┌┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┐      open: dashed, still lists agents
//!   ┆ ▾ ORCA-PAY                 ┆
//!   └┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┘
//!    ▸ TIDE-POOL              ■ ■      folded: one row, a dot per agent
//!   ┏ PROCESS ━━━━━━━━━ 2 RUNNING ┓     running agents as trains
//!
//! A train's station (THINK → CODE → TEST → DONE) comes from the pane's
//! `stage` token when an integration reports one, else from the agent status.

use crate::api::schema::AgentStatus;
use crate::protocol::ClientShellAgent;
use ratatui::style::Color;

use super::brutal::{self, Brutal, Edge};
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
    matches!(agent.agent_status, AgentStatus::Working | AgentStatus::Blocked)
}

fn state_dot(status: AgentStatus, t: &Brutal) -> (&'static str, Color) {
    match t.state(status) {
        Some(color) => ("■", color),
        None => ("□", t.muted),
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
    (if pos > u64::from(span) { period - pos } else { pos }) as u16
}

/// The PROCESS card: running agents only, each a train on four stations.
#[allow(clippy::too_many_arguments)] // one render call, all inputs are distinct facts
fn render_process(
    buffer: &mut Buffer,
    rect: Rect,
    trains: &[(&ClientShellAgent, String)],
    total: usize,
    frame: u64,
    config: &ClientShellConfig,
    t: &Brutal,
    hits: &mut ShellHitMap,
) {
    let card = Rect::new(rect.x, rect.y, rect.width.saturating_sub(1), rect.height.saturating_sub(1));
    brutal::draw_box(buffer, card, Edge::Heavy, t.line, t.card, t.bg);
    brutal::draw_box_shadow(buffer, card, t.shadow, t.bg);
    let x = rect.x + 3;
    let width = rect.width.saturating_sub(6);
    let mut y = rect.y + 1;
    put_text(
        buffer,
        x,
        y,
        7,
        "PROCESS",
        Style::default()
            .fg(t.ink)
            .bg(t.card)
            .add_modifier(Modifier::BOLD),
    );
    let count = format!(" {total} RUNNING ");
    let count_width = display_width(&count);
    put_text(
        buffer,
        (x + width).saturating_sub(count_width),
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
        let style = Style::default().fg(t.muted).bg(t.card).add_modifier(Modifier::BOLD);
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
    for (agent, project) in trains.iter() {
        if y + TRAIN_ROWS > rect.bottom().saturating_sub(1) {
            break;
        }
        let color = t.agent(agent.agent.as_deref());
        let kind = brutal::agent_label(agent).to_uppercase();
        let chip_width = brutal::chip(buffer, x, y, width, &kind, t.chip_text, color);
        let task = brutal::agent_task(agent);
        let detail = if task.is_empty() {
            project.clone()
        } else {
            format!("{project} · {task}")
        };
        let detail_x = x + chip_width + 1;
        let detail_width = (x + width).saturating_sub(detail_x);
        put_text(
            buffer,
            detail_x,
            y,
            detail_width,
            &brutal::clip(&detail, usize::from(detail_width)),
            Style::default().fg(t.ink).bg(t.card),
        );
        hits.agents.push((Rect::new(x, y, width, TRAIN_ROWS), agent.pane_id.clone()));
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
            let (glyph, rail) = if done { ("━", t.rail) } else { ("┄", t.rail_ahead) };
            put_text(
                buffer,
                lx,
                y,
                lw.saturating_sub(1),
                &glyph.repeat(usize::from(lw.saturating_sub(1))),
                Style::default().fg(rail).bg(t.card),
            );
            let stop = if done { "■" } else { "□" };
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

pub(crate) fn render_sidebar_tree(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    state: &mut ShellRenderState<'_>,
    hits: &mut ShellHitMap,
) {
    let t = Brutal::new(&config.palette);
    buffer.set_style(area, Style::default().bg(t.bg).fg(t.ink));
    hits.sidebar_divider = if area.is_empty() {
        Rect::default()
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

    // PROCESS at the foot: running agents only
    let label_of = |workspace_id: &str| {
        snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.workspace_id == workspace_id)
            .map(|workspace| workspace.label.clone())
            .unwrap_or_default()
    };
    let running_agents: Vec<&ClientShellAgent> =
        snapshot.agents.iter().filter(|agent| running(agent)).collect();
    let trains: Vec<(&ClientShellAgent, String)> = running_agents
        .iter()
        .take(MAX_TRAINS)
        .map(|agent| (*agent, label_of(&agent.workspace_id)))
        .collect();
    let rows_for_trains = if trains.is_empty() {
        1
    } else {
        trains.len() as u16 * TRAIN_ROWS
    };
    let process_height = (5 + rows_for_trains).min(area.height.saturating_sub(10));
    let process = Rect::new(x, area.bottom().saturating_sub(process_height), width, process_height);
    if process_height >= 5 {
        render_process(
            buffer,
            process,
            &trains,
            running_agents.len(),
            state.orcha_ui.process_frame,
            config,
            &t,
            hits,
        );
    }

    // PROJECTS: one card, a row per project, its agents under it when open
    let entries = super::sidebar::workspace_entries(snapshot, state.collapsed_groups);
    let mut items = Vec::new();
    for entry in &entries {
        let Some(workspace) = snapshot.workspaces.get(entry.index) else {
            continue;
        };
        let agents: Vec<&ClientShellAgent> = snapshot
            .agents
            .iter()
            .filter(|agent| agent.workspace_id == workspace.workspace_id)
            .collect();
        let open = workspace.focused || state.orcha_ui.open_projects.contains(&workspace.workspace_id);
        items.push((workspace, agents, open));
    }
    let heights: Vec<u16> = items
        .iter()
        .map(|(_, agents, open)| 1 + if *open { agents.len() as u16 } else { 0 })
        .collect();
    let content: u16 = heights.iter().sum();
    let top = area.y + 2;
    let room = process.y.saturating_sub(top + 1);
    // edges, the title row, the rows
    let card = Rect::new(x, top, width.saturating_sub(1), (content + 3).min(room));
    if card.height < 4 {
        return;
    }
    brutal::draw_box(buffer, card, Edge::Light, t.line, t.card, t.bg);
    let x0 = card.x + 1;
    let inner = card.width.saturating_sub(2);
    let title_y = card.y + 1;
    put_text(
        buffer,
        x0 + 1,
        title_y,
        inner,
        &format!("PROJECTS · {}", snapshot.workspaces.len()),
        Style::default()
            .fg(t.muted)
            .bg(t.card)
            .add_modifier(Modifier::BOLD),
    );
    hits.sidebar_toggle = Rect::new(x0 + inner.saturating_sub(2), title_y, 1, 1);
    put_text(
        buffer,
        hits.sidebar_toggle.x,
        title_y,
        1,
        "«",
        Style::default().fg(t.muted).bg(t.card),
    );

    let body = Rect::new(x0, title_y + 1, inner, card.height.saturating_sub(3));
    hits.workspace_body = body;
    let gaps = vec![0; items.len()];
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
    for (index, (workspace, agents, open)) in items.iter().enumerate().skip(*state.workspace_scroll)
    {
        if y.saturating_add(heights[index]) > body.bottom() {
            break;
        }
        render_project_rows(buffer, Rect::new(x0, y, inner, heights[index]), workspace, agents, *open, &t, hits);
        y += heights[index];
    }
    if metrics.max_offset_from_bottom > 0 {
        let track = Rect::new(card.right().saturating_sub(1), body.y, 1, body.height);
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
            Style::default().fg(t.pink).bg(t.card),
        );
    }
}

/// One project in the PROJECTS card: its row, then its agents when open.
/// The selected one is tinted, with a yellow bar down its rows.
fn render_project_rows(
    buffer: &mut Buffer,
    rect: Rect,
    workspace: &ClientShellWorkspace,
    agents: &[&ClientShellAgent],
    open: bool,
    t: &Brutal,
    hits: &mut ShellHitMap,
) {
    let selected = workspace.focused;
    let fill = if selected { t.frame } else { t.card };
    buffer.set_style(rect, Style::default().bg(fill));
    if selected {
        for y in rect.y..rect.bottom() {
            put_text(buffer, rect.x, y, 1, "▌", Style::default().fg(t.yellow).bg(fill));
        }
    }
    let x = rect.x + 1;
    let y = rect.y;
    let right = rect.right().saturating_sub(1);
    put_text(
        buffer,
        x,
        y,
        2,
        if open { "▾ " } else { "▸ " },
        Style::default().fg(t.ink).bg(fill),
    );
    if !selected {
        hits.orcha
            .project_toggles
            .push((Rect::new(x, y, 2, 1), workspace.workspace_id.clone()));
    }
    // folded: a mark per agent on the right; open: the branch
    let note: Vec<(String, ratatui::style::Color)> = if open {
        workspace
            .branch
            .as_deref()
            .map(|branch| {
                let short = branch.rsplit('/').next().unwrap_or(branch);
                vec![(format!("⎇ {}", brutal::clip(short, 12)), t.muted)]
            })
            .unwrap_or_default()
    } else if agents.is_empty() {
        vec![("idle".to_owned(), t.muted)]
    } else {
        agents
            .iter()
            .take(6)
            .map(|agent| {
                let (dot, color) = state_dot(agent.agent_status, t);
                (dot.to_owned(), color)
            })
            .collect()
    };
    let note_width: u16 = note
        .iter()
        .map(|(text, _)| display_width(text) + 1)
        .sum::<u16>()
        .saturating_sub(1);
    let name_width = right.saturating_sub(x + 2 + note_width + 1);
    put_text(
        buffer,
        x + 2,
        y,
        name_width,
        &brutal::clip(&workspace.label.to_uppercase(), usize::from(name_width)),
        Style::default()
            .fg(t.ink)
            .bg(fill)
            .add_modifier(Modifier::BOLD),
    );
    let mut nx = right.saturating_sub(note_width);
    for (text, color) in &note {
        put_text(buffer, nx, y, display_width(text), text, Style::default().fg(*color).bg(fill));
        nx += display_width(text) + 1;
    }
    hits.workspaces.push(WorkspaceHit {
        rect: Rect::new(x + 2, y, right.saturating_sub(x + 2), 1),
        endpoint_id: ClientEndpointId::Local,
        workspace_id: workspace.workspace_id.clone(),
        indented: false,
        group_toggle: None,
    });
    if !open {
        return;
    }
    for (index, agent) in agents.iter().enumerate() {
        let y = rect.y + 1 + index as u16;
        if y >= rect.bottom() {
            break;
        }
        let row = Rect::new(x + 2, y, right.saturating_sub(x + 2), 1);
        let (dot, color) = state_dot(agent.agent_status, t);
        put_text(buffer, x + 2, y, 1, dot, Style::default().fg(color).bg(fill));
        let name = row_name(&brutal::agent_label(agent));
        let name_width = display_width(&name).min(row.width.saturating_sub(3));
        let mut name_style = Style::default().fg(t.ink).bg(fill);
        if agent.focused {
            name_style = name_style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
        } else {
            name_style = name_style.add_modifier(Modifier::BOLD);
        }
        put_text(buffer, x + 4, y, name_width, &name, name_style);
        let blocked = agent.agent_status == AgentStatus::Blocked;
        let note = if blocked { "needs you" } else { "" };
        let task_x = x + 4 + name_width + 1;
        let task_width = right.saturating_sub(task_x + display_width(note) + u16::from(blocked));
        put_text(
            buffer,
            task_x,
            y,
            task_width,
            &brutal::clip(&brutal::agent_task(agent), usize::from(task_width)),
            Style::default().fg(t.muted).bg(fill),
        );
        if blocked {
            put_right_text(
                buffer,
                Rect::new(row.x, y, right.saturating_sub(row.x), 1),
                y,
                note,
                Style::default()
                    .fg(t.red)
                    .bg(fill)
                    .add_modifier(Modifier::BOLD),
            );
        }
        hits.agents.push((row, agent.pane_id.clone()));
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
