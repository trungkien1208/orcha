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
    SettingsWeb,
    Background,
    Keybinds,
    ToggleRightPanel,
    OpenAllProjects,
    FoldAllProjects,
    Setup,
    Quit,
}

/// (command, title, what it does)
const COMMANDS: [(PaletteCommand, &str, &str); 27] = [
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
        "Settings in the terminal",
        "theme, look, apps, notifications, without leaving orcha",
    ),
    (
        PaletteCommand::SettingsWeb,
        "Settings in the browser",
        "every setting on one page, applied as you change it",
    ),
    (
        PaletteCommand::Background,
        "Running in background",
        "agents, plugins and Easy Job that keep going after you quit",
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
    (
        PaletteCommand::OpenAllProjects,
        "Open all projects",
        "show every project's agents in the sidebar",
    ),
    (
        PaletteCommand::FoldAllProjects,
        "Fold all projects",
        "one line per project in the sidebar",
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

/// A linked plugin's row, listed after the built-in commands: one of its
/// actions, or a control for one of its running panes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PluginPaletteAction {
    pub title: String,
    pub detail: String,
    pub run: PluginRun,
}

/// What a plugin row does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PluginRun {
    Invoke {
        plugin_id: String,
        action_id: String,
    },
    /// Close a running pane (and stop starting it with Orcha).
    Stop {
        pane_id: String,
    },
    Restart {
        pane_id: String,
    },
    /// Open the address the pane printed in the browser.
    OpenUrl {
        url: String,
    },
    /// Install a newer version of a GitHub plugin.
    Update {
        plugin_id: String,
        name: String,
    },
    /// Remove an installed plugin (asks first).
    Uninstall {
        plugin_id: String,
        name: String,
    },
}

/// Plugin rows from the last refresh; read when COMMANDS opens.
static PLUGIN_ACTIONS: std::sync::Mutex<Vec<PluginPaletteAction>> =
    std::sync::Mutex::new(Vec::new());

fn plugin_actions() -> Vec<PluginPaletteAction> {
    PLUGIN_ACTIONS
        .lock()
        .map(|actions| actions.clone())
        .unwrap_or_default()
}

/// Asks the server for plugin actions and running plugin panes off the
/// input thread.
fn refresh_plugin_actions() {
    let _ = crate::thread_spawn::spawn_named("orcha-palette-plugins", || {
        let call = |method: &str| crate::orcha_handoff::herdr_json(&["api", "call", method]);
        let names: std::collections::HashMap<String, String> = call("plugin.list")
            .and_then(|result| result.get("plugins").cloned())
            .and_then(|plugins| plugins.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|plugin| {
                let id = plugin
                    .get("plugin_id")
                    .or(plugin.get("id"))?
                    .as_str()?
                    .to_owned();
                let name = plugin
                    .get("name")
                    .and_then(|name| name.as_str())
                    .unwrap_or(&id)
                    .to_owned();
                Some((id, name))
            })
            .collect();
        let mut rows = plugin_action_rows(call("plugin.action.list").as_ref());
        rows.extend(plugin_pane_rows(call("plugin.pane.list").as_ref(), &names));
        rows.extend(plugin_update_rows(
            &crate::orcha_plugin_updates::cached(),
            &names,
        ));
        rows.extend(plugin_uninstall_rows(&names));
        if let Ok(mut known) = PLUGIN_ACTIONS.lock() {
            *known = rows;
        }
    });
}

fn plugin_action_rows(result: Option<&serde_json::Value>) -> Vec<PluginPaletteAction> {
    result
        .and_then(|result| result.get("actions"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|action| {
            let text = |key: &str| action.get(key)?.as_str().map(str::to_owned);
            let plugin_id = text("plugin_id")?;
            Some(PluginPaletteAction {
                title: text("title")?,
                detail: text("description").unwrap_or_else(|| format!("{plugin_id} plugin")),
                run: PluginRun::Invoke {
                    action_id: text("action_id")?,
                    plugin_id,
                },
            })
        })
        .collect()
}

/// An Update row for each GitHub plugin with a newer version this Orcha runs.
fn plugin_update_rows(
    updates: &[crate::orcha_plugin_updates::PluginUpdate],
    names: &std::collections::HashMap<String, String>,
) -> Vec<PluginPaletteAction> {
    updates
        .iter()
        .filter(|update| update.installable())
        .map(|update| {
            let name = names
                .get(&update.id)
                .cloned()
                .unwrap_or_else(|| update.id.clone());
            PluginPaletteAction {
                title: format!("Update {name} to {}", update.latest_version),
                detail: format!(
                    "{} is installed; restarts its running panes",
                    update.current_version
                ),
                run: PluginRun::Update {
                    plugin_id: update.id.clone(),
                    name,
                },
            }
        })
        .collect()
}

/// An Uninstall row for each installed plugin, Easy Job first.
fn plugin_uninstall_rows(
    names: &std::collections::HashMap<String, String>,
) -> Vec<PluginPaletteAction> {
    let mut ids: Vec<&String> = names.keys().collect();
    ids.sort_by_key(|id| (id.as_str() != crate::orcha_office::PLUGIN_ID, id.to_owned()));
    ids.into_iter()
        .map(|id| {
            let name = names[id].clone();
            PluginPaletteAction {
                title: format!("Uninstall {name}"),
                detail: "stops it and removes it from Orcha (asks first)".to_owned(),
                run: PluginRun::Uninstall {
                    plugin_id: id.clone(),
                    name,
                },
            }
        })
        .collect()
}

/// Open link, restart and stop for each running plugin pane.
pub(super) fn plugin_pane_rows(
    result: Option<&serde_json::Value>,
    names: &std::collections::HashMap<String, String>,
) -> Vec<PluginPaletteAction> {
    let mut rows = Vec::new();
    for pane in result
        .and_then(|result| result.get("plugin_panes"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = |key: &str| {
            pane.get(key)
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        };
        let (Some(plugin_id), Some(pane_id)) = (text("plugin_id"), text("pane_id")) else {
            continue;
        };
        let name = names
            .get(&plugin_id)
            .cloned()
            .unwrap_or_else(|| plugin_id.clone());
        let entrypoint = text("entrypoint").unwrap_or_default();
        let autostart = pane.get("autostart").and_then(|value| value.as_bool()) == Some(true);
        if let Some(url) = text("url") {
            rows.push(PluginPaletteAction {
                title: format!("Open {name}"),
                detail: url.clone(),
                run: PluginRun::OpenUrl { url },
            });
        }
        rows.push(PluginPaletteAction {
            title: format!("Restart {name}"),
            detail: format!("its {entrypoint} pane starts again in the background"),
            run: PluginRun::Restart {
                pane_id: pane_id.clone(),
            },
        });
        rows.push(PluginPaletteAction {
            title: format!("Stop {name}"),
            detail: if autostart {
                format!("closes its {entrypoint} pane; it no longer starts with Orcha")
            } else {
                format!("closes its {entrypoint} pane")
            },
            run: PluginRun::Stop { pane_id },
        });
    }
    rows
}

/// A row's title and detail: built-in commands first, then plugin actions.
fn row(plugins: &[PluginPaletteAction], index: usize) -> Option<(&str, &str)> {
    if let Some((_, title, detail)) = COMMANDS.get(index) {
        return Some((title, detail));
    }
    plugins
        .get(index.checked_sub(COMMANDS.len())?)
        .map(|action| (action.title.as_str(), action.detail.as_str()))
}

/// The rows for `query`, best match first; all of them, in order, when the
/// query is empty.
pub(super) fn matches(query: &str) -> Vec<usize> {
    matches_in(&plugin_actions(), query)
}

fn matches_in(plugins: &[PluginPaletteAction], query: &str) -> Vec<usize> {
    let total = COMMANDS.len() + plugins.len();
    if query.trim().is_empty() {
        return (0..total).collect();
    }
    let mut scored: Vec<(usize, usize)> = (0..total)
        .filter_map(|index| {
            let (title, detail) = row(plugins, index)?;
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

fn plugin_action(index: usize) -> Option<PluginPaletteAction> {
    plugin_actions()
        .get(index.checked_sub(COMMANDS.len())?)
        .cloned()
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
    let plugins = plugin_actions();
    let rows = matches_in(&plugins, query);
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
        let Some((title, detail)) = row(&plugins, *index) else {
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
        refresh_plugin_actions();
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
        let Some(index) = matches(query).get(position).copied() else {
            return;
        };
        if let Some(action) = plugin_action(index) {
            self.orcha_ui.popup = None;
            match action.run {
                PluginRun::Update { plugin_id, name } => {
                    self.start_plugin_update(plugin_id, name);
                    return;
                }
                PluginRun::Uninstall { plugin_id, name } => {
                    self.orcha_ui.popup = Some(super::orcha_popup::OrchaPopup::PluginRemove {
                        selected: 1,
                        plugin_id,
                        name,
                    });
                    return;
                }
                _ => {}
            }
            self.show_notice(match &action.run {
                PluginRun::Invoke { .. } => format!("running {}", action.title),
                PluginRun::OpenUrl { url } => format!("opening {url}"),
                PluginRun::Restart { .. }
                | PluginRun::Stop { .. }
                | PluginRun::Update { .. }
                | PluginRun::Uninstall { .. } => {
                    format!("{}…", action.title.to_lowercase())
                }
            });
            // Through the server like any plugin request; never blocks input.
            let _ = crate::thread_spawn::spawn_named("orcha-palette-plugin-run", move || {
                let params = |pane_id: &str| serde_json::json!({ "pane_id": pane_id }).to_string();
                match action.run {
                    PluginRun::Invoke {
                        plugin_id,
                        action_id,
                    } => {
                        let _ = crate::orcha_handoff::herdr(&[
                            "plugin", "action", "invoke", &action_id, "--plugin", &plugin_id,
                        ])
                        .stdout(std::process::Stdio::null())
                        .status();
                    }
                    PluginRun::Stop { pane_id } => {
                        let _ = crate::orcha_handoff::herdr_json(&[
                            "api",
                            "call",
                            "plugin.pane.stop",
                            &params(&pane_id),
                        ]);
                    }
                    PluginRun::Restart { pane_id } => {
                        let _ = crate::orcha_handoff::herdr_json(&[
                            "api",
                            "call",
                            "plugin.pane.restart",
                            &params(&pane_id),
                        ]);
                    }
                    PluginRun::OpenUrl { url } => {
                        let _ = crate::platform::open_url(&url);
                    }
                    PluginRun::Update { .. } | PluginRun::Uninstall { .. } => {}
                }
            });
            return;
        }
        let Some(command) = command(index) else {
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
            PaletteCommand::SettingsWeb => self.open_settings_web(),
            PaletteCommand::Background => self.open_settings_web_at("background".to_owned()),
            PaletteCommand::Keybinds => self.record_binding(binding(KeybindAction::Help), outcome),
            PaletteCommand::ToggleRightPanel => {
                let on = !self.config.right_panel;
                self.save_settings_edit(crate::config::ConfigEdit::RightPanel(on), outcome);
            }
            PaletteCommand::OpenAllProjects | PaletteCommand::FoldAllProjects => {
                let open = command == PaletteCommand::OpenAllProjects;
                let ids: Vec<String> = self
                    .snapshot
                    .as_deref()
                    .map(|snapshot| {
                        snapshot
                            .workspaces
                            .iter()
                            .map(|workspace| workspace.workspace_id.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                for id in ids {
                    if open {
                        self.orcha_ui.folded_projects.remove(&id);
                        self.orcha_ui.open_projects.insert(id);
                    } else {
                        self.orcha_ui.open_projects.remove(&id);
                        self.orcha_ui.folded_projects.insert(id);
                    }
                }
                self.persist_chrome_preferences(outcome);
                outcome.repaint = true;
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

#[cfg(test)]
mod plugin_palette_tests {
    use super::*;

    #[test]
    fn every_installed_plugin_can_be_uninstalled_easy_job_first() {
        let names = std::collections::HashMap::from([
            ("zeta".to_owned(), "Zeta".to_owned()),
            ("easy-job".to_owned(), "Easy Job".to_owned()),
        ]);
        let rows = plugin_uninstall_rows(&names);
        let titles: Vec<&str> = rows.iter().map(|row| row.title.as_str()).collect();
        assert_eq!(titles, ["Uninstall Easy Job", "Uninstall Zeta"]);
        assert_eq!(
            rows[0].run,
            PluginRun::Uninstall {
                plugin_id: "easy-job".into(),
                name: "Easy Job".into()
            }
        );
    }

    #[test]
    fn only_updates_this_orcha_can_run_get_an_update_row() {
        let update = |id: &str, compatible: bool| crate::orcha_plugin_updates::PluginUpdate {
            id: id.into(),
            current_version: "0.1.0".into(),
            latest_version: "0.2.0".into(),
            compatible,
            needs_herdr_version: None,
            source: format!("me/{id}"),
            reference: Some("plugin".into()),
            installed_commit: Some("a".into()),
            latest_commit: Some("b".into()),
        };
        let mut current = update("current", true);
        current.latest_commit = current.installed_commit.clone();
        let names = std::collections::HashMap::from([("easy-job".into(), "Easy Job".into())]);
        let rows = plugin_update_rows(
            &[update("easy-job", true), update("blocked", false), current],
            &names,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Update Easy Job to 0.2.0");
        assert_eq!(
            rows[0].run,
            PluginRun::Update {
                plugin_id: "easy-job".into(),
                name: "Easy Job".into()
            }
        );
    }

    #[test]
    fn plugin_actions_follow_the_built_in_commands_and_match_queries() {
        *PLUGIN_ACTIONS.lock().unwrap() = vec![PluginPaletteAction {
            title: "Open Easy Job".into(),
            detail: "Open the Easy Job office".into(),
            run: PluginRun::Invoke {
                plugin_id: "easy-job".into(),
                action_id: "open".into(),
            },
        }];
        assert_eq!(matches("").len(), COMMANDS.len() + 1);
        let found = matches("easy job");
        assert_eq!(found.first(), Some(&COMMANDS.len()));
        assert!(matches!(
            plugin_action(COMMANDS.len()).unwrap().run,
            PluginRun::Invoke { ref action_id, .. } if action_id == "open"
        ));
        assert!(command(COMMANDS.len()).is_none());
        PLUGIN_ACTIONS.lock().unwrap().clear();
    }

    #[test]
    fn running_plugin_panes_get_open_restart_and_stop_rows() {
        let panes = serde_json::json!({"plugin_panes": [
            {"plugin_id": "easy-job", "entrypoint": "server", "pane_id": "w7:p2",
             "workspace_id": "w7", "autostart": true, "url": "http://127.0.0.1:4317/"}
        ]});
        let names =
            std::collections::HashMap::from([("easy-job".to_owned(), "Easy Job".to_owned())]);
        let rows = plugin_pane_rows(Some(&panes), &names);
        let titles: Vec<_> = rows.iter().map(|row| row.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Open Easy Job", "Restart Easy Job", "Stop Easy Job"]
        );
        assert!(rows[2].detail.contains("no longer starts with Orcha"));
        assert_eq!(
            rows[0].run,
            PluginRun::OpenUrl {
                url: "http://127.0.0.1:4317/".into()
            }
        );
    }
}
