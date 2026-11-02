//! New-project and import list renderers.

use super::*;

/// NEW PROJECT: the current project, another folder, a typed path or a
/// new worktree.
pub(super) fn render_new_workspace(
    buffer: &mut Buffer,
    selected: usize,
    project: Option<&str>,
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let frame = popup_frame(
        buffer,
        72,
        rows_height(NEW_WORKSPACE_ROWS),
        "NEW PROJECT",
        "choose where the project starts",
        &[("↵", "choose"), ("↑↓", "move")],
        "esc close",
        t,
        hits,
    )?;
    let options = [
        (
            "Current project",
            project
                .map(|project| format!("create a workspace in {project}"))
                .unwrap_or_else(|| "no project is selected".to_owned()),
            if project.is_some() {
                "CREATE"
            } else {
                "CHOOSE"
            },
        ),
        (
            "Another folder",
            "browse for a project folder on this computer".to_owned(),
            "BROWSE",
        ),
        (
            "Enter folder path",
            "paste an absolute project folder path".to_owned(),
            "CHOOSE",
        ),
        (
            "New worktree",
            project
                .map(|project| {
                    format!("a branch of {project} in its own folder, for an agent beside it")
                })
                .unwrap_or_else(|| "select a git project first".to_owned()),
            "BRANCH",
        ),
    ];
    draw_rows(
        buffer,
        frame.body,
        ROW,
        options.len(),
        selected,
        t,
        |buffer, rect, index, first| {
            let Some((title, detail, action)) = options.get(index) else {
                return;
            };
            let row = ChoiceRow::new(title, detail, RowButton::Always(action))
                .at(first, index == selected);
            choice_row(buffer, rect, index, row, t, hits);
        },
    );
    Some(())
}

/// HAND OFF: the other agents, each with the whole conversation or a
/// summary first.
pub(super) fn render_handoff(
    buffer: &mut Buffer,
    selected: usize,
    pane: &str,
    targets: &[usize],
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let agent = snapshot.agents.iter().find(|agent| agent.pane_id == pane);
    let from = agent
        .and_then(|agent| agent.agent.clone())
        .unwrap_or_default();
    let context = agent
        .map(|agent| {
            format!(
                "from {} {} · {}",
                brutal::agent_logo(agent.agent.as_deref(), config.icon_font),
                brutal::agent_label(agent),
                brutal::agent_task(agent)
            )
        })
        .unwrap_or_default();
    let frame = popup_frame(
        buffer,
        72,
        rows_height(targets.len()),
        "HAND OFF",
        &context,
        &[("↵", "continue there"), ("↑↓", "choose")],
        "esc close",
        t,
        hits,
    )?;
    let rows: Vec<_> = targets
        .iter()
        .filter_map(|target| HANDOFF_TARGETS.get(*target))
        .collect();
    draw_rows(
        buffer,
        frame.body,
        ROW,
        rows.len(),
        selected,
        t,
        |buffer, rect, index, first| {
            let Some(&&(kind, mode, label)) = rows.get(index) else {
                return;
            };
            let cursor = index == selected;
            let detail = if mode == "summary" {
                "a short summary opens the new chat"
            } else {
                "the whole conversation is replayed"
            };
            let action = if cursor { "CONTINUE ↵" } else { "CONTINUE" };
            let row = ChoiceRow {
                lead: Lead::Logo(
                    brutal::agent_logo(Some(kind), config.icon_font),
                    t.agent(Some(kind)),
                ),
                tone: if kind == from { Tone::Muted } else { Tone::Ink },
                ..ChoiceRow::new(label, detail, RowButton::Always(action))
            };
            choice_row(buffer, rect, index, row.at(first, cursor), t, hits);
        },
    );
    Some(())
}

/// IMPORT: projects from the tools used before, one row per tool, each
/// with a check; the primary button brings the checked ones in.
pub(super) fn render_import(
    buffer: &mut Buffer,
    selected: usize,
    chosen: &[bool],
    sources: &[crate::orcha_import::ImportSource],
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let count: usize = sources
        .iter()
        .zip(chosen)
        .filter(|(_, on)| **on)
        .map(|(source, _)| source.projects.len())
        .sum();
    let frame = popup_frame(
        buffer,
        76,
        rows_height(sources.len()) + 2,
        "IMPORT PROJECTS",
        "not open yet · each becomes a project in the sidebar",
        &[("↵", "import"), ("space", "check"), ("esc", "close")],
        "",
        t,
        hits,
    )?;
    if sources.is_empty() {
        put_text(
            buffer,
            frame.body.x + 2,
            frame.body.y,
            frame.body.width.saturating_sub(4),
            "nothing new to import: every Orca, VS Code, Cursor and JetBrains project is open",
            Style::default().fg(t.muted).bg(t.bg),
        );
        return Some(());
    }
    draw_rows(
        buffer,
        list_above_button(frame.body),
        ROW,
        sources.len(),
        selected,
        t,
        |buffer, rect, index, first| {
            let Some(source) = sources.get(index) else {
                return;
            };
            let title = format!("{} · {} projects", source.name, source.projects.len());
            let names = source
                .projects
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let row = ChoiceRow {
                lead: Lead::Check(chosen.get(index).copied().unwrap_or(false)),
                ..ChoiceRow::new(&title, &names, RowButton::None)
            };
            choice_row(
                buffer,
                rect,
                index,
                row.at(first, index == selected),
                t,
                hits,
            );
        },
    );
    primary_under_list(buffer, frame.body, count, sources.len(), t, hits);
    Some(())
}

/// The list part of a body that ends with a blank line and a button.
pub(super) fn list_above_button(body: Rect) -> Rect {
    Rect::new(body.x, body.y, body.width, body.height.saturating_sub(2))
}

/// IMPORT (n): the primary action under a checked list; plain while
/// nothing is checked. Its index is `index`, one past the rows.
pub(super) fn primary_under_list(
    buffer: &mut Buffer,
    body: Rect,
    count: usize,
    index: usize,
    t: &Brutal,
    hits: &mut PopupHits,
) {
    let label = if count == 0 {
        "IMPORT".to_owned()
    } else {
        format!("IMPORT {count} ↵")
    };
    let action = button(
        buffer,
        body.right() - 1,
        body.bottom().saturating_sub(1),
        &label,
        if count == 0 {
            ButtonKind::Plain
        } else {
            ButtonKind::Primary
        },
        t.bg,
        t,
    );
    hits.actions.push((action, index));
}
