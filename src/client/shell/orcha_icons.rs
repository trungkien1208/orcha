//! orcha: an icon per project, picked once and shown before its name in the
//! sidebar. Kept per project folder in `project-icons.json` in orcha's
//! config folder (presentation only, so the client owns it). Read once at
//! start and after each pick; rendering only reads the map.

use std::collections::HashMap;

use super::brutal::Brutal;
use super::orcha_popup::{popup_frame, PopupHits};
use super::render::put_text;
use super::*;

/// The icons offered, ten a row.
pub(super) const ICONS: [&str; 30] = [
    "🍵", "🐋", "🚀", "🧪", "📦", "🔧", "🎨", "🔥", "⚡", "🌱", //
    "🐛", "📱", "🌐", "🤖", "💳", "📊", "🎮", "🧠", "🗂", "⭐", //
    "🏥", "🛒", "🔒", "📝", "🎵", "📷", "🧾", "🏠", "🌀", "🧩",
];
const PER_ROW: usize = 10;

fn icons_file() -> Option<std::path::PathBuf> {
    Some(
        crate::platform::orcha_config_base_dir()?
            .join("orcha")
            .join("project-icons.json"),
    )
}

/// The saved icons, by project folder.
pub(super) fn load() -> HashMap<String, String> {
    icons_file()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save(icons: &HashMap<String, String>) {
    let Some(path) = icons_file() else {
        return;
    };
    let icons = icons.clone();
    // off the input path
    let _ = crate::thread_spawn::spawn_named("orcha-project-icons", move || {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let text = serde_json::to_string_pretty(&icons).unwrap_or_else(|_| "{}".to_owned());
        if let Err(err) = std::fs::write(&path, text) {
            tracing::warn!(err = %err, "orcha: could not save the project icons");
        }
    });
}

/// The key a project's icon is kept under: its folder, as the server
/// reports it (no filesystem work, so render can use it).
pub(super) fn key(workspace: &crate::protocol::ClientShellWorkspace) -> &str {
    let folder = workspace.new_workspace_cwd.trim_end_matches(['/', '\\']);
    if folder.is_empty() {
        &workspace.workspace_id
    } else {
        folder
    }
}

/// The picker: a grid of icons, then "no icon".
pub(super) fn render(
    buffer: &mut Buffer,
    selected: usize,
    label: &str,
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let rows = ICONS.len().div_ceil(PER_ROW) as u16;
    let frame = popup_frame(
        buffer,
        56,
        rows * 2 + 2,
        "PROJECT ICON",
        label,
        &[("↵", "choose"), ("←↑↓→", "move"), ("esc", "close")],
        "",
        t,
        hits,
    )?;
    let body = frame.body;
    let cell = 4u16;
    let left = body.x + (body.width.saturating_sub(cell * PER_ROW as u16)) / 2;
    for (index, icon) in ICONS.iter().enumerate() {
        let x = left + (index % PER_ROW) as u16 * cell;
        let y = body.y + (index / PER_ROW) as u16 * 2;
        let rect = Rect::new(x, y, cell, 1);
        let chosen = index == selected;
        let fill = if chosen { t.yellow } else { t.bg };
        buffer.set_style(rect, Style::default().bg(fill));
        put_text(buffer, x + 1, y, 2, icon, Style::default().bg(fill));
        hits.rows.push((rect, index));
        hits.actions.push((rect, index));
    }
    let none = ICONS.len();
    let y = body.y + rows * 2;
    let text = " no icon ";
    let rect = Rect::new(left, y, super::render::display_width(text), 1);
    let chosen = selected == none;
    put_text(
        buffer,
        rect.x,
        y,
        rect.width,
        text,
        Style::default()
            .fg(if chosen { t.chip_text } else { t.muted })
            .bg(if chosen { t.yellow } else { t.bg })
            .add_modifier(Modifier::BOLD),
    );
    hits.rows.push((rect, none));
    hits.actions.push((rect, none));
    Some(())
}

impl ClientShellState {
    /// Opens the picker for a project (the focused one when None).
    pub(super) fn open_project_icon_picker(&mut self, workspace_id: Option<String>) {
        let Some(workspace) = self.snapshot.as_deref().and_then(|snapshot| {
            snapshot
                .workspaces
                .iter()
                .find(|workspace| match &workspace_id {
                    Some(id) => workspace.workspace_id == *id,
                    None => workspace.focused,
                })
        }) else {
            return;
        };
        let key = key(workspace).to_owned();
        let selected = self
            .orcha_ui
            .project_icons
            .get(&key)
            .and_then(|icon| ICONS.iter().position(|known| known == icon))
            .unwrap_or(0);
        self.orcha_ui.popup = Some(super::orcha_popup::OrchaPopup::ProjectIcon {
            selected,
            key,
            label: workspace.label.clone(),
        });
    }

    /// Keys in the picker: arrows move on the grid.
    pub(super) fn route_project_icon_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        use crossterm::event::KeyCode;
        outcome.repaint = true;
        let Some(super::orcha_popup::OrchaPopup::ProjectIcon { selected, .. }) =
            self.orcha_ui.popup.as_mut()
        else {
            return;
        };
        let last = ICONS.len();
        match key.code {
            KeyCode::Esc => self.orcha_ui.popup = None,
            KeyCode::Right | KeyCode::Tab => *selected = (*selected + 1).min(last),
            KeyCode::Left | KeyCode::BackTab => *selected = selected.saturating_sub(1),
            KeyCode::Down => *selected = (*selected + PER_ROW).min(last),
            KeyCode::Up => *selected = selected.saturating_sub(PER_ROW),
            KeyCode::Backspace | KeyCode::Delete => {
                let index = last;
                self.choose_project_icon(index);
            }
            KeyCode::Enter => {
                let index = *selected;
                self.choose_project_icon(index);
            }
            _ => {}
        }
    }

    /// Keeps the icon at `index` (past the grid: none) for the project.
    pub(super) fn choose_project_icon(&mut self, index: usize) {
        let Some(super::orcha_popup::OrchaPopup::ProjectIcon { key, .. }) =
            self.orcha_ui.popup.take()
        else {
            return;
        };
        match ICONS.get(index) {
            Some(icon) => {
                self.orcha_ui.project_icons.insert(key, (*icon).to_owned());
            }
            None => {
                self.orcha_ui.project_icons.remove(&key);
            }
        }
        save(&self.orcha_ui.project_icons);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_are_kept_by_folder_without_a_trailing_separator() {
        let workspace = crate::protocol::ClientShellWorkspace {
            workspace_id: "w1".into(),
            active_tab_id: "t1".into(),
            new_workspace_cwd: "/p/app/".into(),
            number: 1,
            label: "app".into(),
            custom_label: false,
            branch: None,
            git_ahead_behind: None,
            tokens: Vec::new(),
            worktree: None,
            focused: true,
            agent_status: crate::api::schema::AgentStatus::Idle,
        };
        assert_eq!(key(&workspace), "/p/app");
    }

    #[test]
    fn every_icon_is_one_symbol() {
        for icon in ICONS {
            assert_eq!(icon.chars().count(), 1, "{icon}");
        }
    }
}
