//! orcha: the left sidebar as one tree. Each project (workspace) row is
//! followed by its agents, indented, instead of separate "spaces" and
//! "agents" sections. Rows reuse the spaces/agents row renderers, so the
//! `[ui.sidebar.spaces]` / `[ui.sidebar.agents]` row config still applies.

use super::render::{put_right_text, put_text, ShellRenderState};
use super::*;

/// Indent of agent rows under their project.
const AGENT_INDENT: u16 = 2;

enum TreeItem<'a> {
    Workspace(&'a WorkspaceEntry),
    Agent(super::agent_sidebar::AgentRow),
}

/// One line per agent in the tree: rows made only of custom tokens (e.g. a
/// `$grp` project header a script reports) and the workspace name repeat what
/// the tree already shows, so they are dropped and the rest joined.
fn tree_agent_row(mut row: super::agent_sidebar::AgentRow) -> super::agent_sidebar::AgentRow {
    use crate::ui::ResolvedTokenKind;
    let line = row
        .rows
        .iter()
        .filter(|tokens| {
            !tokens.is_empty()
                && !tokens
                    .iter()
                    .all(|token| matches!(token.kind, ResolvedTokenKind::Custom(_)))
        })
        .flatten()
        .filter(|token| !matches!(token.kind, ResolvedTokenKind::Workspace(_)))
        .cloned()
        .collect::<Vec<_>>();
    row.rows = vec![line];
    row
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
    hits.sidebar_section_divider = Rect::default();
    hits.agent_body = Rect::default();
    put_text(
        buffer,
        area.x,
        area.y,
        area.width,
        " projects",
        Style::default()
            .fg(palette.overlay0)
            .add_modifier(Modifier::BOLD),
    );

    let entries = super::sidebar::workspace_entries(snapshot, state.collapsed_groups);
    let mut items = Vec::new();
    for entry in &entries {
        let Some(workspace) = snapshot.workspaces.get(entry.index) else {
            continue;
        };
        items.push(TreeItem::Workspace(entry));
        for agent in snapshot
            .agents
            .iter()
            .filter(|agent| agent.workspace_id == workspace.workspace_id)
        {
            if let Some(row) =
                super::agent_sidebar::agent_row(snapshot, &agent.pane_id, config, None)
            {
                items.push(TreeItem::Agent(tree_agent_row(row)));
            }
        }
    }

    // header row, a blank row, the list, a blank row, the footer row
    let body = Rect::new(
        area.x,
        area.y.saturating_add(2),
        area.width.saturating_sub(1),
        area.height.saturating_sub(4),
    );
    hits.workspace_body = body;
    let heights = items
        .iter()
        .map(|item| match item {
            TreeItem::Workspace(entry) => snapshot
                .workspaces
                .get(entry.index)
                .map(|workspace| {
                    super::sidebar::workspace_rows(
                        workspace,
                        super::sidebar::displayed_workspace_status(
                            snapshot,
                            workspace,
                            state.collapsed_groups,
                        ),
                        entry.indented,
                        &config.spaces,
                    )
                    .len()
                })
                .unwrap_or(1),
            TreeItem::Agent(row) => row.rows.len(),
        })
        .map(|height| height.max(1).min(u16::MAX as usize) as u16)
        .collect::<Vec<_>>();
    // a gap before each project but the first, none between a project and its agents
    let gaps = (0..items.len())
        .map(|index| match items.get(index + 1) {
            Some(TreeItem::Workspace(_)) => config.spaces.row_gap,
            _ => 0,
        })
        .collect::<Vec<_>>();
    let mut metrics =
        super::scroll::list_scroll_metrics(&heights, &gaps, body.height, *state.workspace_scroll);
    if !body.is_empty() && std::mem::take(state.reveal_focused_workspace) {
        if let Some(target) = items.iter().position(|item| {
            matches!(item, TreeItem::Workspace(entry)
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

    let mut y = body.y;
    for (index, item) in items.iter().enumerate().skip(*state.workspace_scroll) {
        let height = heights[index].min(body.height);
        if y.saturating_add(height) > body.bottom() {
            break;
        }
        match item {
            TreeItem::Workspace(entry) => {
                let Some(workspace) = snapshot.workspaces.get(entry.index) else {
                    continue;
                };
                let rect = Rect::new(body.x, y, content_width, height);
                let status = super::sidebar::displayed_workspace_status(
                    snapshot,
                    workspace,
                    state.collapsed_groups,
                );
                let rows = super::sidebar::workspace_rows(
                    workspace,
                    status,
                    entry.indented,
                    &config.spaces,
                );
                let selected = state.selected_workspace_id.is_some_and(|target| {
                    target.matches(state.active_endpoint_id, &workspace.workspace_id)
                });
                let dragged = state.dragged_workspace_id == Some(workspace.workspace_id.as_str());
                if selected {
                    buffer.set_style(rect, Style::default().bg(palette.selection_bg));
                } else if dragged {
                    buffer.set_style(rect, Style::default().bg(palette.surface1));
                } else if workspace.focused {
                    buffer.set_style(rect, Style::default().bg(palette.active_row_bg));
                }
                super::sidebar::render_workspace_rows(
                    buffer,
                    rect,
                    status,
                    config.status_indicators,
                    entry,
                    rows,
                    workspace.focused,
                    selected,
                    state.selected_workspace_id.is_some(),
                    dragged,
                    palette,
                );
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
            TreeItem::Agent(row) => {
                let rect = Rect::new(
                    body.x.saturating_add(AGENT_INDENT),
                    y,
                    content_width.saturating_sub(AGENT_INDENT),
                    height,
                );
                super::agent_sidebar::render_agent_row(buffer, rect, row, config);
                hits.agents.push((rect, row.pane_id.clone()));
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

    // footer: two real buttons instead of the bare "new" / "menu" words
    let footer_y = area.bottom().saturating_sub(1);
    if config.mouse_capture && area.height > 2 {
        let button = Style::default().fg(palette.text).bg(palette.surface0);
        let new_label = " + New ";
        let new_width = (new_label.chars().count() as u16 + 1).min(area.width);
        hits.new_workspace = Rect::new(area.x.saturating_add(1), footer_y, new_width, 1);
        put_text(
            buffer,
            hits.new_workspace.x,
            footer_y,
            new_width,
            new_label,
            button,
        );
        let attention = super::global_menu::global_menu_attention(snapshot);
        let menu_label = if attention {
            " ● Menu "
        } else {
            " ☰ Menu "
        };
        let menu_width = (menu_label.chars().count() as u16).min(area.width);
        let menu_area = Rect::new(area.x, footer_y, area.width.saturating_sub(3), 1);
        hits.global_launcher = Rect::new(
            menu_area.right().saturating_sub(menu_width),
            footer_y,
            menu_width,
            1,
        );
        let menu_style = if attention {
            button.fg(palette.accent).add_modifier(Modifier::BOLD)
        } else {
            button
        };
        put_right_text(buffer, menu_area, footer_y, menu_label, menu_style);
    }

    hits.sidebar_toggle = Rect::new(
        area.right().saturating_sub(2),
        footer_y,
        u16::from(area.width > 1),
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
