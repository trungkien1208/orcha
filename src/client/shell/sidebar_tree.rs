//! orcha: the left sidebar as metro lines. Each project (workspace) is a line
//! with its own color; its agents are trains on that line, one block each:
//!
//!   ▌● matcha-house            ⎇ feature/42
//!      LTD.EXP  Dark mode         ✳ Claude
//!        ●━━━●━━━▶┄┄┄○            Testing
//!
//! A train's kind comes from its agent (Claude is a limited express, Codex a
//! rapid…). Its station (Thinking → Coding → Testing → Done) comes from the
//! pane's `stage` token when an integration reports one, else from the agent
//! status.

use crate::api::schema::AgentStatus;
use ratatui::style::Color;

use super::render::{put_right_text, put_text, ShellRenderState};
use super::*;

const STATIONS: [&str; 4] = ["Thinking", "Coding", "Testing", "Done"];

enum TreeItem<'a> {
    Project(&'a WorkspaceEntry, Color),
    Train(Train),
}

struct Train {
    pane_id: String,
    kind: &'static str,
    kind_color: Color,
    title: String,
    agent: String,
    station: usize,
    status: AgentStatus,
    focused: bool,
}

fn line_color(index: usize, palette: &Palette) -> Color {
    let colors = [
        palette.red,
        palette.blue,
        palette.green,
        palette.mauve,
        palette.teal,
        palette.peach,
    ];
    colors[index % colors.len()]
}

fn train_kind(agent: Option<&str>, palette: &Palette) -> (&'static str, Color) {
    match agent.unwrap_or("").to_ascii_lowercase().as_str() {
        "claude" => ("LTD.EXP", palette.peach),
        "codex" => ("RAPID", palette.green),
        "cursor" => ("LOCAL", palette.blue),
        "gemini" => ("SKY", palette.teal),
        "opencode" | "amp" | "droid" => ("EXPRESS", palette.mauve),
        _ => ("TRAIN", palette.overlay1),
    }
}

/// The station a train is at: the `stage` token an integration reports, else
/// a guess from the agent status.
fn station(tokens: &[(String, String)], status: AgentStatus) -> usize {
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

fn trains_of(snapshot: &ClientShellSnapshot, workspace_id: &str, palette: &Palette) -> Vec<Train> {
    snapshot
        .agents
        .iter()
        .filter(|agent| agent.workspace_id == workspace_id)
        .map(|agent| {
            let (kind, kind_color) = train_kind(agent.agent.as_deref(), palette);
            let task = agent
                .tokens
                .iter()
                .find(|(key, value)| key == "task" && !value.trim().is_empty())
                .map(|(_, value)| value.clone());
            let title = task
                .or_else(|| {
                    agent
                        .terminal_title_stripped
                        .clone()
                        .filter(|t| !t.trim().is_empty())
                })
                .or_else(|| agent.title.clone())
                .unwrap_or_default();
            let name = agent
                .display_agent
                .clone()
                .or_else(|| agent.agent.clone())
                .or_else(|| agent.name.clone())
                .unwrap_or_else(|| "agent".to_owned());
            Train {
                pane_id: agent.pane_id.clone(),
                kind,
                kind_color,
                title,
                agent: name,
                station: station(&agent.tokens, agent.agent_status),
                status: agent.agent_status,
                focused: agent.focused,
            }
        })
        .collect()
}

fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn render_project_row(
    buffer: &mut Buffer,
    rect: Rect,
    workspace: &ClientShellWorkspace,
    color: Color,
    focused: bool,
    palette: &Palette,
) {
    let bg = if focused {
        palette.selection_bg
    } else {
        palette.sidebar_bg
    };
    buffer.set_style(rect, Style::default().bg(bg));
    if focused {
        put_text(
            buffer,
            rect.x,
            rect.y,
            1,
            "▌",
            Style::default().fg(palette.accent).bg(bg),
        );
    }
    put_text(
        buffer,
        rect.x + 1,
        rect.y,
        1,
        "●",
        Style::default().fg(color).bg(bg),
    );
    let branch = workspace
        .branch
        .as_deref()
        .map(|branch| format!("⎇ {}", clip(branch, 14)))
        .unwrap_or_default();
    let branch_width = branch.chars().count() as u16;
    let name_width = rect.width.saturating_sub(4 + branch_width + 1);
    put_text(
        buffer,
        rect.x + 3,
        rect.y,
        name_width,
        &clip(&workspace.label, name_width as usize),
        Style::default()
            .fg(palette.text)
            .bg(bg)
            .add_modifier(Modifier::BOLD),
    );
    if !branch.is_empty() {
        put_right_text(
            buffer,
            Rect::new(rect.x, rect.y, rect.width.saturating_sub(1), 1),
            rect.y,
            &branch,
            Style::default().fg(palette.overlay1).bg(bg),
        );
    }
}

fn render_train(buffer: &mut Buffer, rect: Rect, train: &Train, line: Color, palette: &Palette) {
    let bg = if train.focused {
        palette.surface0
    } else {
        palette.sidebar_bg
    };
    buffer.set_style(rect, Style::default().bg(bg));
    if train.focused {
        put_text(
            buffer,
            rect.x,
            rect.y,
            1,
            "▏",
            Style::default().fg(palette.accent).bg(bg),
        );
        put_text(
            buffer,
            rect.x,
            rect.y + 1,
            1,
            "▏",
            Style::default().fg(palette.accent).bg(bg),
        );
    }
    let width = rect.width.saturating_sub(1);

    // line 1: kind badge · train name · agent
    let badge = format!(" {} ", train.kind);
    let badge_width = badge.chars().count() as u16;
    put_text(
        buffer,
        rect.x + 2,
        rect.y,
        badge_width,
        &badge,
        Style::default()
            .fg(palette.sidebar_bg)
            .bg(train.kind_color)
            .add_modifier(Modifier::BOLD),
    );
    let agent = clip(&train.agent, 12);
    let agent_width = agent.chars().count() as u16;
    let title_x = rect.x + 2 + badge_width + 1;
    let title_width = width.saturating_sub(2 + badge_width + 1 + agent_width + 1);
    put_text(
        buffer,
        title_x,
        rect.y,
        title_width,
        &clip(&train.title, title_width as usize),
        Style::default()
            .fg(palette.text)
            .bg(bg)
            .add_modifier(Modifier::BOLD),
    );
    put_right_text(
        buffer,
        Rect::new(rect.x, rect.y, width, 1),
        rect.y,
        &agent,
        Style::default().fg(train.kind_color).bg(bg),
    );

    // line 2: the track with the train at its station, the station's name right
    let arrived = matches!(train.status, AgentStatus::Done)
        || (train.station == STATIONS.len() - 1 && train.status != AgentStatus::Working);
    let (glyph, color, label) = match train.status {
        AgentStatus::Blocked => ("!", palette.red, "needs you".to_owned()),
        _ if arrived => ("✓", palette.green, "arrived".to_owned()),
        AgentStatus::Working => ("▶", palette.yellow, STATIONS[train.station].to_owned()),
        _ => ("■", palette.overlay0, "in depot".to_owned()),
    };
    let label_width = label.chars().count() as u16;
    let track_width = width.saturating_sub(4 + label_width + 1).min(28);
    let segment = usize::from(track_width.saturating_sub(4) / 3).max(1);
    let y = rect.y + 1;
    let mut x = rect.x + 4;
    for stop in 0..STATIONS.len() {
        if stop > 0 {
            let (rail, rail_color) = if stop <= train.station {
                ("━", line)
            } else {
                ("┄", palette.surface1)
            };
            put_text(
                buffer,
                x,
                y,
                segment as u16,
                &rail.repeat(segment),
                Style::default().fg(rail_color).bg(bg),
            );
            x += segment as u16;
        }
        let (mark, style) = if stop == train.station {
            (
                glyph,
                Style::default()
                    .fg(color)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            )
        } else if stop < train.station {
            ("●", Style::default().fg(line).bg(bg))
        } else {
            ("○", Style::default().fg(palette.surface1).bg(bg))
        };
        put_text(buffer, x, y, 1, mark, style);
        x += 1;
    }
    put_right_text(
        buffer,
        Rect::new(rect.x, y, width, 1),
        y,
        &label,
        Style::default().fg(color).bg(bg),
    );
}

/// The orcha wordmark at the left of the full-width header.
pub(crate) fn render_brand(buffer: &mut Buffer, area: Rect, palette: &Palette) {
    buffer.set_style(area, Style::default().bg(palette.sidebar_bg));
    put_text(
        buffer,
        area.x + 2,
        area.y,
        1,
        "◉",
        Style::default().fg(palette.accent).bg(palette.sidebar_bg),
    );
    put_text(
        buffer,
        area.x + 4,
        area.y,
        5,
        "orcha",
        Style::default()
            .fg(palette.accent)
            .bg(palette.sidebar_bg)
            .add_modifier(Modifier::BOLD),
    );
    put_text(
        buffer,
        area.x + 10,
        area.y,
        5,
        "METRO",
        Style::default().fg(palette.overlay0).bg(palette.sidebar_bg),
    );
}

pub(crate) fn render_sidebar_tree(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    state: &mut ShellRenderState<'_>,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    buffer.set_style(area, Style::default().bg(palette.sidebar_bg));
    hits.sidebar_divider = if area.is_empty() {
        Rect::default()
    } else {
        Rect::new(area.right().saturating_sub(1), area.y, 1, area.height)
    };
    for y in area.y..area.bottom() {
        put_text(
            buffer,
            area.right().saturating_sub(1),
            y,
            1,
            "│",
            Style::default().fg(palette.surface1).bg(palette.sidebar_bg),
        );
    }
    hits.sidebar_section_divider = Rect::default();
    hits.agent_body = Rect::default();

    let entries = super::sidebar::workspace_entries(snapshot, state.collapsed_groups);
    let mut items = Vec::new();
    let mut train_count = 0;
    for (index, entry) in entries.iter().enumerate() {
        let Some(workspace) = snapshot.workspaces.get(entry.index) else {
            continue;
        };
        items.push(TreeItem::Project(entry, line_color(index, palette)));
        for train in trains_of(snapshot, &workspace.workspace_id, palette) {
            train_count += 1;
            items.push(TreeItem::Train(train));
        }
    }

    // section label, like every orcha region: muted, bold, count on the right
    let label = Style::default()
        .fg(palette.overlay0)
        .add_modifier(Modifier::BOLD);
    put_text(buffer, area.x + 2, area.y, 6, "LINES", label);
    put_right_text(
        buffer,
        Rect::new(area.x, area.y, area.width.saturating_sub(2), 1),
        area.y,
        &format!("{} · {} trains", entries.len(), train_count),
        Style::default().fg(palette.overlay0),
    );

    let body = Rect::new(
        area.x,
        area.y.saturating_add(2),
        area.width.saturating_sub(1),
        area.height.saturating_sub(5),
    );
    hits.workspace_body = body;
    let heights = items
        .iter()
        .map(|item| match item {
            TreeItem::Project(..) => 1,
            TreeItem::Train(_) => 2,
        })
        .collect::<Vec<u16>>();
    // a blank row before each line but the first, none inside a line
    let gaps = (0..items.len())
        .map(|index| match items.get(index + 1) {
            Some(TreeItem::Project(..)) => 1,
            _ => 0,
        })
        .collect::<Vec<_>>();
    let mut metrics =
        super::scroll::list_scroll_metrics(&heights, &gaps, body.height, *state.workspace_scroll);
    if !body.is_empty() && std::mem::take(state.reveal_focused_workspace) {
        if let Some(target) = items.iter().position(|item| {
            matches!(item, TreeItem::Project(entry, _)
                if snapshot.workspaces.get(entry.index).is_some_and(|workspace| workspace.focused))
        }) {
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
    let show_scrollbar = metrics.max_offset_from_bottom > 0 && body.width > 1;
    let content_width = body.width.saturating_sub(u16::from(show_scrollbar));

    let mut line = palette.accent;
    let mut y = body.y;
    for (index, item) in items.iter().enumerate().skip(*state.workspace_scroll) {
        let height = heights[index].min(body.height);
        if y.saturating_add(height) > body.bottom() {
            break;
        }
        match item {
            TreeItem::Project(entry, color) => {
                let Some(workspace) = snapshot.workspaces.get(entry.index) else {
                    continue;
                };
                line = *color;
                let indent = if entry.indented { 2 } else { 0 };
                let rect = Rect::new(
                    body.x + indent,
                    y,
                    content_width.saturating_sub(indent),
                    height,
                );
                render_project_row(buffer, rect, workspace, *color, workspace.focused, palette);
                let group_toggle = super::sidebar::render_parent_group_toggle(
                    buffer,
                    rect,
                    snapshot,
                    entry.index,
                    state.collapsed_groups,
                    palette,
                );
                hits.workspaces.push(WorkspaceHit {
                    rect,
                    endpoint_id: ClientEndpointId::Local,
                    workspace_id: workspace.workspace_id.clone(),
                    indented: entry.indented,
                    group_toggle,
                });
            }
            TreeItem::Train(train) => {
                let rect = Rect::new(body.x + 1, y, content_width.saturating_sub(1), height);
                render_train(buffer, rect, train, line, palette);
                hits.agents.push((rect, train.pane_id.clone()));
            }
        }
        y = y.saturating_add(height).saturating_add(gaps[index]);
    }

    if show_scrollbar {
        let track = Rect::new(body.right().saturating_sub(1), body.y, 1, body.height);
        hits.workspace_scrollbar = track;
        super::scroll::render_list_scrollbar(buffer, track, metrics, palette);
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
            &"─".repeat(body.width as usize),
            Style::default().fg(palette.accent),
        );
    }

    // footer: two raised buttons, a blank row above them
    let footer_y = area.bottom().saturating_sub(2);
    if config.mouse_capture && area.height > 4 {
        let button = Style::default().fg(palette.text).bg(palette.surface0);
        let new_label = " + New train ";
        let new_width = (new_label.chars().count() as u16).min(area.width);
        hits.new_workspace = Rect::new(area.x.saturating_add(2), footer_y, new_width, 1);
        put_text(
            buffer,
            hits.new_workspace.x,
            footer_y,
            new_width,
            new_label,
            button,
        );
        put_text(
            buffer,
            hits.new_workspace.x + 1,
            footer_y,
            1,
            "+",
            button.fg(palette.accent),
        );
        let attention = super::global_menu::global_menu_attention(snapshot);
        let menu_label = if attention {
            " ● Menu "
        } else {
            " ≡ Menu "
        };
        let menu_width = (menu_label.chars().count() as u16).min(area.width);
        let menu_x = hits.new_workspace.right().saturating_add(1);
        hits.global_launcher = Rect::new(menu_x, footer_y, menu_width, 1);
        let menu_style = if attention {
            button.fg(palette.accent).add_modifier(Modifier::BOLD)
        } else {
            button
        };
        put_text(buffer, menu_x, footer_y, menu_width, menu_label, menu_style);
    }

    hits.sidebar_toggle = Rect::new(
        area.right().saturating_sub(3),
        footer_y,
        u16::from(area.width > 2),
        u16::from(area.height > 0),
    );
    put_text(
        buffer,
        hits.sidebar_toggle.x,
        hits.sidebar_toggle.y,
        hits.sidebar_toggle.width,
        "«",
        Style::default().fg(palette.overlay0),
    );
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
}
