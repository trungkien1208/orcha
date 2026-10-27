//! orcha: COMMANDS, the command palette. Every orcha action in one search
//! popup (popup template, docs/orcha/SPEC.md §1): type to filter, ↵ runs
//! the highlighted one. Opened by the header's commands button or
//! `prefix` + `:`.

use super::brutal::Brutal;
use super::orcha_popup::{button, popup_frame, row_box, ButtonKind, PopupHits};
use super::render::put_text;
use super::*;

/// What a palette row does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PaletteCommand {
    NewAgent,
    NewWorkspace,
    NewWorktree,
    NewTab,
    SplitRight,
    HandOff,
    Broadcast,
    ProjectIcon,
    Accounts,
    RefreshFuel,
    FindAccounts,
    NeedsYou,
    OpenIde,
    OpenGit,
    Commit,
    PullRequest,
    Search,
    ImportProjects,
    Settings,
    Keybinds,
    ToggleRightPanel,
    Setup,
    Quit,
}

/// (command, title, what it does)
const COMMANDS: [(PaletteCommand, &str, &str); 23] = [
    (
        PaletteCommand::NewAgent,
        "New agent",
        "start Claude, Codex or Cursor in a new tab",
    ),
    (
        PaletteCommand::NewWorkspace,
        "New project",
        "a workspace for a folder",
    ),
    (
        PaletteCommand::NewWorktree,
        "New worktree",
        "a branch in its own folder, for an agent beside this one",
    ),
    (PaletteCommand::NewTab, "New tab", "a shell in this project"),
    (
        PaletteCommand::SplitRight,
        "Split right",
        "a second pane next to this one",
    ),
    (
        PaletteCommand::HandOff,
        "Hand off this chat",
        "continue the conversation with another agent",
    ),
    (
        PaletteCommand::Broadcast,
        "Ask every agent here",
        "type one prompt for all agents of this project",
    ),
    (
        PaletteCommand::ProjectIcon,
        "Set this project's icon",
        "an emoji before its name in the sidebar",
    ),
    (
        PaletteCommand::Accounts,
        "Accounts and fuel",
        "switch account, sign in, see what is left",
    ),
    (
        PaletteCommand::RefreshFuel,
        "Refresh fuel",
        "ask every account for its limits now",
    ),
    (
        PaletteCommand::FindAccounts,
        "Find accounts",
        "agents and accounts on this machine, to add to FUEL",
    ),
    (
        PaletteCommand::NeedsYou,
        "Go to the agent that needs you",
        "the one waiting longest, in any project",
    ),
    (
        PaletteCommand::OpenIde,
        "Open in IDE",
        "this repository in your editor",
    ),
    (
        PaletteCommand::OpenGit,
        "Open in Git app",
        "this repository in your Git GUI",
    ),
    (
        PaletteCommand::Commit,
        "Ask the agent to commit",
        "a clear, conventional commit of the changes",
    ),
    (
        PaletteCommand::PullRequest,
        "Ask the agent for a pull request",
        "push the branch and open a PR",
    ),
    (
        PaletteCommand::Search,
        "Go to…",
        "search projects, tabs and panes",
    ),
    (
        PaletteCommand::ImportProjects,
        "Import projects",
        "from Orca, VS Code, Cursor and JetBrains",
    ),
    (
        PaletteCommand::Settings,
        "Settings",
        "theme, look, apps, notifications",
    ),
    (
        PaletteCommand::Keybinds,
        "Keybinds",
        "every key, in one list",
    ),
    (
        PaletteCommand::ToggleRightPanel,
        "Show or hide the right panel",
        "fuel and git beside the panes",
    ),
    (PaletteCommand::Setup, "Setup", "the first-run setup again"),
    (
        PaletteCommand::Quit,
        "Quit to terminal",
        "agents keep running; orcha brings you back",
    ),
];

/// `query`'s letters appear in `text` in order (case ignored). A word or
/// substring hit beats scattered letters; earlier and tighter matches score
/// lower. None when it does not match.
fn fuzzy_score(text: &str, query: &str) -> Option<usize> {
    let lower = text.to_lowercase();
    let wanted = query.trim().to_lowercase();
    if let Some(at) = lower.find(&wanted) {
        let word_start = at == 0 || lower[..at].ends_with([' ', '-', '…']);
        return Some(if word_start { at / 4 } else { 10 + at / 4 });
    }
    let text: Vec<char> = lower.chars().collect();
    let mut score = 30;
    let mut at = 0;
    for wanted in query
        .to_lowercase()
        .chars()
        .filter(|ch| !ch.is_whitespace())
    {
        let found = text[at..].iter().position(|ch| *ch == wanted)?;
        score += found + if at == 0 { found } else { 0 };
        at += found + 1;
    }
    Some(score)
}

/// The rows for `query`, best match first; all of them, in order, when the
/// query is empty.
pub(super) fn matches(query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..COMMANDS.len()).collect();
    }
    let mut scored: Vec<(usize, usize)> = COMMANDS
        .iter()
        .enumerate()
        .filter_map(|(index, (_, title, detail))| {
            let title_score = fuzzy_score(title, query);
            let detail_score = fuzzy_score(detail, query).map(|score| score + 100);
            title_score.or(detail_score).map(|score| (score, index))
        })
        .collect();
    scored.sort();
    scored.into_iter().map(|(_, index)| index).collect()
}

pub(super) fn command(index: usize) -> Option<PaletteCommand> {
    COMMANDS.get(index).map(|(command, _, _)| *command)
}

/// Rows shown at once; the list scrolls past them.
const VISIBLE: usize = 10;

pub(super) fn render(
    buffer: &mut Buffer,
    selected: usize,
    query: &str,
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let rows = matches(query);
    let frame = popup_frame(
        buffer,
        92,
        (VISIBLE as u16) * 2 + 2,
        "COMMANDS",
        "type to find an action",
        &[("↵", "run"), ("↑↓", "move"), ("esc", "close")],
        "",
        t,
        hits,
    )?;
    let body = frame.body;
    // the search input, the first body row
    let input = Rect::new(body.x, body.y, body.width, 1);
    let fill = row_box(buffer, input, true, true, t);
    let shown = if query.is_empty() {
        "⌕ search commands".to_owned()
    } else {
        format!("⌕ {query}▏")
    };
    put_text(
        buffer,
        input.x + 2,
        input.y,
        input.width.saturating_sub(4),
        &shown,
        Style::default()
            .fg(if query.is_empty() { t.muted } else { t.ink })
            .bg(fill)
            .add_modifier(Modifier::BOLD),
    );
    if rows.is_empty() {
        put_text(
            buffer,
            body.x + 2,
            body.y + 2,
            body.width.saturating_sub(4),
            "no command matches",
            Style::default().fg(t.muted).bg(t.bg),
        );
        return Some(());
    }
    let selected = selected.min(rows.len() - 1);
    // keep the highlighted row in view
    let first = selected.saturating_sub(VISIBLE - 1);
    let mut y = body.y + 2;
    for (position, index) in rows.iter().enumerate().skip(first).take(VISIBLE) {
        if y >= body.bottom() {
            break;
        }
        let Some((_, title, detail)) = COMMANDS.get(*index) else {
            continue;
        };
        let rect = Rect::new(body.x, y, body.width, 1);
        let cursor = position == selected;
        let fill = row_box(buffer, rect, false, cursor, t);
        let title_width = 33u16.min(rect.width.saturating_sub(4));
        put_text(
            buffer,
            rect.x + 2,
            y,
            title_width,
            title,
            Style::default()
                .fg(t.ink)
                .bg(fill)
                .add_modifier(Modifier::BOLD),
        );
        let detail_x = rect.x + 2 + title_width + 1;
        put_text(
            buffer,
            detail_x,
            y,
            rect.right().saturating_sub(detail_x + 9),
            detail,
            Style::default().fg(t.muted).bg(fill),
        );
        if cursor {
            let action = button(
                buffer,
                rect.right() - 1,
                y,
                "RUN ↵",
                ButtonKind::Primary,
                fill,
                t,
            );
            hits.actions.push((action, position));
        }
        hits.rows.push((rect, position));
        y += 2;
    }
    Some(())
}

/// ASK EVERY AGENT: the prompt being typed and the agents it goes to.
pub(super) fn render_broadcast(
    buffer: &mut Buffer,
    typed: &str,
    panes: &[String],
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let shown = panes.len().min(8);
    let context = format!(
        "{} agent{} in this project",
        panes.len(),
        if panes.len() == 1 { "" } else { "s" }
    );
    let frame = popup_frame(
        buffer,
        80,
        (shown as u16).max(1) + 4,
        "ASK EVERY AGENT",
        &context,
        &[
            ("↵", "send to all"),
            ("type", "the prompt"),
            ("esc", "close"),
        ],
        "",
        t,
        hits,
    )?;
    let body = frame.body;
    let input = Rect::new(body.x, body.y, body.width, 1);
    let fill = row_box(buffer, input, true, true, t);
    let text = if typed.is_empty() {
        "› what should every agent do?".to_owned()
    } else {
        format!("› {typed}▏")
    };
    let room = usize::from(input.width.saturating_sub(16));
    // the end of a long prompt stays in view
    let clipped: String = {
        let chars: Vec<char> = text.chars().collect();
        chars[chars.len().saturating_sub(room)..].iter().collect()
    };
    put_text(
        buffer,
        input.x + 2,
        input.y,
        input.width.saturating_sub(16),
        &clipped,
        Style::default()
            .fg(if typed.is_empty() { t.muted } else { t.ink })
            .bg(fill)
            .add_modifier(Modifier::BOLD),
    );
    if !typed.trim().is_empty() {
        let send = button(
            buffer,
            input.right() - 1,
            input.y,
            "SEND ↵",
            ButtonKind::Primary,
            fill,
            t,
        );
        hits.actions.push((send, 0));
    }
    let mut y = body.y + 2;
    for pane in panes.iter().take(shown) {
        if y >= body.bottom() {
            break;
        }
        let Some(agent) = snapshot.agents.iter().find(|agent| agent.pane_id == *pane) else {
            continue;
        };
        let line = format!(
            "{} {} · {}",
            super::brutal::agent_logo(agent.agent.as_deref(), config.icon_font),
            super::brutal::agent_label(agent),
            super::brutal::agent_task(agent)
        );
        put_text(
            buffer,
            body.x + 2,
            y,
            body.width.saturating_sub(4),
            &line,
            Style::default().fg(t.ink).bg(t.bg),
        );
        y += 1;
    }
    Some(())
}

/// The agents of the focused project, in sidebar order.
fn project_agents(snapshot: &ClientShellSnapshot) -> Vec<String> {
    let Some(workspace) = snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.focused)
        .map(|workspace| workspace.workspace_id.clone())
    else {
        return Vec::new();
    };
    snapshot
        .agents
        .iter()
        .filter(|agent| {
            agent.workspace_id == workspace
                && agent.agent.is_some()
                && agent.agent_status != crate::api::schema::AgentStatus::Unknown
        })
        .map(|agent| agent.pane_id.clone())
        .collect()
}

impl ClientShellState {
    pub(super) fn open_broadcast(&mut self) {
        let panes = self
            .snapshot
            .as_deref()
            .map(project_agents)
            .unwrap_or_default();
        if panes.is_empty() {
            self.show_notice("ask every agent: no agent runs in this project".to_owned());
            return;
        }
        self.orcha_ui.popup = Some(super::orcha_popup::OrchaPopup::Broadcast {
            typed: String::new(),
            panes,
        });
    }

    pub(super) fn route_broadcast_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        use crossterm::event::{KeyCode, KeyModifiers};
        outcome.repaint = true;
        let Some(super::orcha_popup::OrchaPopup::Broadcast { typed, .. }) =
            self.orcha_ui.popup.as_mut()
        else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.orcha_ui.popup = None,
            KeyCode::Backspace => {
                typed.pop();
            }
            KeyCode::Enter => self.send_broadcast(),
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => typed.push(ch),
            _ => {}
        }
    }

    /// Types the prompt into every listed agent, off the input thread.
    pub(super) fn send_broadcast(&mut self) {
        let Some(super::orcha_popup::OrchaPopup::Broadcast { typed, panes }) =
            self.orcha_ui.popup.clone()
        else {
            return;
        };
        let prompt = typed.trim().to_owned();
        if prompt.is_empty() {
            return;
        }
        self.orcha_ui.popup = None;
        let count = panes.len();
        let _ = crate::thread_spawn::spawn_named("orcha-broadcast", move || {
            for pane in panes {
                if !crate::orcha_handoff::send_prompt(&pane, &prompt) {
                    tracing::warn!("orcha: could not send the prompt to {pane}");
                }
            }
        });
        self.show_notice(format!(
            "sent to {count} agent{}",
            if count == 1 { "" } else { "s" }
        ));
    }

    pub(super) fn open_palette(&mut self) {
        self.orcha_ui.popup = Some(super::orcha_popup::OrchaPopup::Palette {
            selected: 0,
            query: String::new(),
        });
    }

    /// Typing, moving and running inside COMMANDS.
    pub(super) fn route_palette_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        use crossterm::event::{KeyCode, KeyModifiers};
        outcome.repaint = true;
        let Some(super::orcha_popup::OrchaPopup::Palette { selected, query }) =
            self.orcha_ui.popup.as_mut()
        else {
            return;
        };
        let count = matches(query).len();
        match key.code {
            KeyCode::Esc => self.orcha_ui.popup = None,
            KeyCode::Down | KeyCode::Tab if count > 0 => *selected = (*selected + 1) % count,
            KeyCode::Up | KeyCode::BackTab if count > 0 => {
                *selected = (*selected + count - 1) % count;
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) && count > 0 => {
                *selected = (*selected + 1) % count;
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) && count > 0 => {
                *selected = (*selected + count - 1) % count;
            }
            KeyCode::Backspace => {
                query.pop();
                *selected = 0;
            }
            KeyCode::Enter => {
                let position = *selected;
                self.run_palette_row(position, outcome);
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                query.push(ch);
                *selected = 0;
            }
            _ => {}
        }
    }

    /// Runs the `position`th row of the current matches.
    pub(super) fn run_palette_row(&mut self, position: usize, outcome: &mut ClientShellInput) {
        let Some(super::orcha_popup::OrchaPopup::Palette { query, .. }) = &self.orcha_ui.popup
        else {
            return;
        };
        let Some(command) = matches(query)
            .get(position)
            .and_then(|index| command(*index))
        else {
            return;
        };
        self.orcha_ui.popup = None;
        self.run_palette_command(command, outcome);
    }

    fn run_palette_command(&mut self, command: PaletteCommand, outcome: &mut ClientShellInput) {
        use crate::input::{KeybindAction, KeybindMatch};
        let binding = |action| KeybindMatch::Action(action);
        match command {
            PaletteCommand::NewAgent => self.open_new_agent_popup(),
            PaletteCommand::NewWorkspace => self.open_new_workspace_popup(),
            PaletteCommand::NewWorktree => {
                self.record_binding(binding(KeybindAction::NewWorktree), outcome)
            }
            PaletteCommand::NewTab => self.record_binding(binding(KeybindAction::NewTab), outcome),
            PaletteCommand::SplitRight => {
                self.record_binding(binding(KeybindAction::SplitVertical), outcome)
            }
            PaletteCommand::HandOff => self.open_handoff_popup(),
            PaletteCommand::Broadcast => self.open_broadcast(),
            PaletteCommand::ProjectIcon => self.open_project_icon_picker(None),
            PaletteCommand::Accounts => self.open_accounts_overlay(),
            PaletteCommand::RefreshFuel => self.refresh_fuel(),
            PaletteCommand::FindAccounts => self.start_account_scan(true),
            PaletteCommand::NeedsYou => self.jump_to_needs_you(outcome),
            PaletteCommand::OpenIde => self.open_project_in(true),
            PaletteCommand::OpenGit => self.open_project_in(false),
            PaletteCommand::Commit => self
                .ask_focused_agent("Commit the changes with a clear, conventional commit message."),
            PaletteCommand::PullRequest => self.ask_focused_agent(
                "Push this branch and open a pull request with a short summary of the changes.",
            ),
            PaletteCommand::Search => {
                self.record_binding(binding(KeybindAction::OpenNavigator), outcome)
            }
            PaletteCommand::ImportProjects => self.start_import_scan(),
            PaletteCommand::Settings => {
                self.record_binding(binding(KeybindAction::Settings), outcome)
            }
            PaletteCommand::Keybinds => self.record_binding(binding(KeybindAction::Help), outcome),
            PaletteCommand::ToggleRightPanel => {
                let on = !self.config.right_panel;
                self.save_settings_edit(crate::config::ConfigEdit::RightPanel(on), outcome);
            }
            PaletteCommand::Setup => self.open_welcome(outcome),
            PaletteCommand::Quit => self.record_binding(binding(KeybindAction::Detach), outcome),
        }
        outcome.repaint = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_in_order_match_and_earlier_tighter_matches_rank_first() {
        assert!(fuzzy_score("Hand off this chat", "hoff").is_some());
        assert!(fuzzy_score("Hand off this chat", "zz").is_none());
        let rows = matches("acc");
        assert_eq!(command(rows[0]), Some(PaletteCommand::Accounts));
        let rows = matches("worktree");
        assert_eq!(command(rows[0]), Some(PaletteCommand::NewWorktree));
        // a word hit outranks scattered letters
        let rows = matches("git");
        assert_eq!(command(rows[0]), Some(PaletteCommand::OpenGit));
        let rows = matches("commit");
        assert_eq!(command(rows[0]), Some(PaletteCommand::Commit));
    }

    #[test]
    fn an_empty_query_lists_every_command_in_order() {
        let rows = matches("  ");
        assert_eq!(rows.len(), COMMANDS.len());
        assert_eq!(command(rows[0]), Some(PaletteCommand::NewAgent));
    }

    #[test]
    fn descriptions_match_when_titles_do_not() {
        let rows = matches("fuel");
        assert!(rows
            .iter()
            .any(|row| command(*row) == Some(PaletteCommand::Accounts)));
    }
}
