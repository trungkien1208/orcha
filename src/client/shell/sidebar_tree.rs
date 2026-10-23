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
    // its bottom row is its line and shadow, right on the bottom
    let card = Rect::new(rect.x, rect.y, rect.width.saturating_sub(1), rect.height);
    brutal::draw_box(buffer, card, Edge::Heavy, t.line, t.card, t.bg);
    brutal::draw_box_shadow(buffer, card, t.shadow, t.bg);
    let x = rect.x + 2;
    let width = rect.width.saturating_sub(5);
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
    // line row, title, stations, the trains, bottom line row
    let process_height = (4 + rows_for_trains).min(area.height.saturating_sub(10));
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

    // PROJECTS: a card per project; the selected one raised, an open one
    // outlined dim, folded ones stacked a hair apart
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
    hits.sidebar_toggle = Rect::new(x + width.saturating_sub(2), label_y, 1, 1);
    put_text(buffer, hits.sidebar_toggle.x, label_y, 1, "«", Style::default().fg(t.muted).bg(t.bg));

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
        let kind = if workspace.focused {
            Card::Selected
        } else if state.orcha_ui.open_projects.contains(&workspace.workspace_id) {
            Card::Open
        } else {
            Card::Folded
        };
        items.push((workspace, agents, kind));
    }
    // a card is its top line's row, its rows, and (when nothing follows it,
    // or it is raised) a bottom row; stacked cards share the row between
    let content = |agents: &Vec<&ClientShellAgent>, kind: Card| {
        1 + if kind == Card::Folded { 0 } else { agents.len() as u16 }
    };
    // every card: the line row over it (shared with the card above), its
    // rows, and a bottom line row after the last; the same gap everywhere
    let heights: Vec<u16> = items
        .iter()
        .enumerate()
        .map(|(index, (_, agents, kind))| 1 + content(agents, *kind) + u16::from(index + 1 == items.len()))
        .collect();
    let gaps = vec![0; items.len()];
    let body = Rect::new(x, label_y + 1, width, process.y.saturating_sub(label_y + 1));
    hits.workspace_body = body;
    let mut metrics =
        super::scroll::list_scroll_metrics(&heights, &gaps, body.height, *state.workspace_scroll);
    if !body.is_empty() && std::mem::take(state.reveal_focused_workspace) {
        if let Some(target) = items.iter().position(|(_, _, kind)| *kind == Card::Selected) {
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

    let card_width = width.saturating_sub(1);
    let mut y = body.y;
    for (index, (workspace, agents, kind)) in items.iter().enumerate().skip(*state.workspace_scroll) {
        let height = heights[index];
        if y.saturating_add(height) > body.bottom() {
            break;
        }
        let rows = content(agents, *kind);
        let first = index == *state.workspace_scroll;
        let selected = *kind == Card::Selected;
        let above_selected = !first && items[index - 1].2 == Card::Selected;
        // the shared row: the card above's bottom line and this one's top line;
        // yellow when it edges the selected card
        let glyph = if first { "▁" } else { "\u{1FB80}" };
        let color = if selected || above_selected { t.yellow } else { t.line };
        put_text(
            buffer,
            x,
            y,
            card_width,
            &glyph.repeat(usize::from(card_width)),
            Style::default().fg(color).bg(t.bg),
        );
        let body_rect = Rect::new(x, y + 1, card_width, rows);
        match kind {
            Card::Folded => render_folded_row(buffer, body_rect, workspace, agents, &t, hits),
            _ => render_project_card(buffer, body_rect, workspace, agents, *kind, &t, hits),
        }
        if selected {
            // raised: a shadow down its right side, no extra row
            for row in body_rect.y..body_rect.bottom() {
                put_text(buffer, x + card_width, row, 1, "▌", Style::default().fg(t.shadow).bg(t.bg));
            }
        }
        if index + 1 == items.len() {
            put_text(
                buffer,
                x,
                body_rect.bottom(),
                card_width,
                &"▔".repeat(usize::from(card_width)),
                Style::default().fg(if selected { t.yellow } else { t.line }).bg(t.bg),
            );
        }
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Card {
    Selected,
    Open,
    Folded,
}

/// A selected (raised, yellow edge) or open (dim outline) project card:
/// its name row, then a row per agent.
fn render_project_card(
    buffer: &mut Buffer,
    rect: Rect,
    workspace: &ClientShellWorkspace,
    agents: &[&ClientShellAgent],
    kind: Card,
    t: &Brutal,
    hits: &mut ShellHitMap,
) {
    // `rect` is the card's rows; its top and bottom lines are drawn by the list
    let selected = kind == Card::Selected;
    let fill = if selected {
        t.card
    } else if brutal::is_light(t.bg) {
        t.bg
    } else {
        t.frame
    };
    for row in rect.y..rect.bottom() {
        brutal::pill(buffer, Rect::new(rect.x, row, rect.width, 1), "", t.ink, fill, true, t);
        if selected {
            // highlighted: yellow lines and a yellow bar inside the left one
            let edge = Style::default().fg(t.yellow).bg(fill);
            put_text(buffer, rect.x, row, 1, "▏", edge);
            put_text(buffer, rect.right().saturating_sub(1), row, 1, "▕", edge);
            put_text(buffer, rect.x + 1, row, 1, "▌", Style::default().fg(t.yellow).bg(fill));
        }
    }
    let card = rect;
    let x = card.x + 2;
    let right = card.right().saturating_sub(2);
    let y = card.y;
    put_text(buffer, x, y, 2, "▾ ", Style::default().fg(t.ink).bg(fill));
    if !selected {
        hits.orcha
            .project_toggles
            .push((Rect::new(x, y, 2, 1), workspace.workspace_id.clone()));
    }
    let branch = workspace
        .branch
        .as_deref()
        .map(|branch| format!("⎇ {}", brutal::clip(branch.rsplit('/').next().unwrap_or(branch), 12)))
        .unwrap_or_default();
    let branch_width = display_width(&branch);
    let name_width = right.saturating_sub(x + 2 + branch_width + 1);
    put_text(
        buffer,
        x + 2,
        y,
        name_width,
        &brutal::clip(&workspace.label.to_uppercase(), usize::from(name_width)),
        Style::default()
            .fg(if selected { t.ink } else { t.muted })
            .bg(fill)
            .add_modifier(Modifier::BOLD),
    );
    put_text(
        buffer,
        right.saturating_sub(branch_width),
        y,
        branch_width,
        &branch,
        Style::default().fg(t.muted).bg(fill),
    );
    hits.workspaces.push(WorkspaceHit {
        rect: Rect::new(x + 2, y, right.saturating_sub(x + 2), 1),
        endpoint_id: ClientEndpointId::Local,
        workspace_id: workspace.workspace_id.clone(),
        indented: false,
        group_toggle: None,
    });
    for (index, agent) in agents.iter().enumerate() {
        let y = y + 1 + index as u16;
        if y >= card.bottom() {
            break;
        }
        let row = Rect::new(x + 1, y, right.saturating_sub(x + 1), 1);
        let row_bg = if agent.focused { t.frame } else { fill };
        buffer.set_style(row, Style::default().bg(row_bg));
        let (dot, color) = state_dot(agent.agent_status, t);
        put_text(buffer, x + 2, y, 1, dot, Style::default().fg(color).bg(row_bg));
        let name = row_name(&brutal::agent_label(agent));
        let name_width = display_width(&name).min(row.width.saturating_sub(3));
        put_text(
            buffer,
            x + 4,
            y,
            name_width,
            &name,
            Style::default().fg(t.ink).bg(row_bg).add_modifier(Modifier::BOLD),
        );
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
            Style::default().fg(t.muted).bg(row_bg),
        );
        if blocked {
            put_right_text(
                buffer,
                Rect::new(row.x, y, right.saturating_sub(row.x), 1),
                y,
                note,
                Style::default().fg(t.red).bg(row_bg).add_modifier(Modifier::BOLD),
            );
        }
        hits.agents.push((row, agent.pane_id.clone()));
    }
}

/// A folded project's row: its card's sides, the name and a mark per agent.
fn render_folded_row(
    buffer: &mut Buffer,
    row: Rect,
    workspace: &ClientShellWorkspace,
    agents: &[&ClientShellAgent],
    t: &Brutal,
    hits: &mut ShellHitMap,
) {
    let fill = if brutal::is_light(t.bg) { t.bg } else { t.card };
    let row = Rect::new(row.x, row.y, row.width, 1);
    brutal::pill(buffer, row, "", t.ink, fill, true, t);
    let (x, y) = (row.x + 2, row.y);
    put_text(buffer, x, y, 2, "▸ ", Style::default().fg(t.ink).bg(fill));
    hits.orcha
        .project_toggles
        .push((Rect::new(x, y, 2, 1), workspace.workspace_id.clone()));
    let marks_width = if agents.is_empty() {
        4
    } else {
        (agents.len().min(6) * 2 - 1) as u16
    };
    let right = row.right().saturating_sub(2);
    let name_width = right.saturating_sub(x + 3 + marks_width + 1);
    put_text(
        buffer,
        x + 2,
        y,
        name_width,
        &brutal::clip(&workspace.label.to_uppercase(), usize::from(name_width)),
        Style::default().fg(t.ink).bg(fill).add_modifier(Modifier::BOLD),
    );
    let mut mx = right.saturating_sub(marks_width);
    if agents.is_empty() {
        put_text(buffer, mx, y, 4, "idle", Style::default().fg(t.muted).bg(fill));
    }
    for agent in agents.iter().take(6) {
        let (dot, color) = state_dot(agent.agent_status, t);
        put_text(buffer, mx, y, 1, dot, Style::default().fg(color).bg(fill));
        mx += 2;
    }
    hits.workspaces.push(WorkspaceHit {
        rect: Rect::new(x + 2, y, row.width.saturating_sub(5), 1),
        endpoint_id: ClientEndpointId::Local,
        workspace_id: workspace.workspace_id.clone(),
        indented: false,
        group_toggle: None,
    });
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
