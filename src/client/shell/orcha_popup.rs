//! orcha: popups. Every popup follows one template (docs/orcha/SPEC.md §1
//! "Popup template"): dimmed page, heavy box with a hard shadow, a header
//! row (title, context, ✕), body rows in thin boxes (the current one heavy
//! with a yellow edge), one raised green primary action, and a footer with
//! the keys.
//!
//! `OrchaPopup` lists every popup. Most draw here; COMMANDS and ASK EVERY
//! AGENT live in `orcha_palette`, the first-run WELCOME in `orcha_welcome`,
//! PROJECT ICON in `orcha_icons` and the file view in `file_viewer`.
//! Lists share one row layout (`choice_row`) and one scroll window
//! (`draw_rows`), so the cursor's row is always on screen.

use crate::api::schema::AgentStatus;

use super::accounts::Account;
use super::brutal::{self, Brutal};
use super::render::{display_width, put_right_text, put_text};
use super::*;

mod accounts;
mod offers;
mod primitives;
mod projects;

pub(super) use accounts::*;
pub(super) use offers::*;
pub(super) use primitives::*;
use projects::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OrchaPopup {
    /// Accounts and their fuel; acts on the agent in `pane`.
    Accounts {
        selected: usize,
        pane: Option<String>,
    },
    /// Pick a project's icon (`key`: its folder).
    ProjectIcon {
        selected: usize,
        key: String,
        label: String,
    },
    /// Confirm taking an account off orcha's list (`selected` 0 removes).
    RemoveAccount {
        selected: usize,
        id: String,
        label: String,
        pane: Option<String>,
    },
    /// Pick a name for a new Claude account, then sign it in.
    AddAccount {
        selected: usize,
        names: Vec<String>,
        typed: String,
        pane: Option<String>,
    },
    /// Continue `pane`'s conversation with another agent.
    /// `targets` index `HANDOFF_TARGETS`: the other agents installed.
    Handoff {
        selected: usize,
        pane: String,
        targets: Vec<usize>,
    },
    /// COMMANDS: every action, filtered by `query`; `selected` indexes the
    /// matches.
    Palette { selected: usize, query: String },
    /// ASK EVERY AGENT: one prompt for the agents in `panes`.
    Broadcast { typed: String, panes: Vec<String> },
    /// Bring in projects from other tools; `chosen` per source.
    Import { selected: usize, chosen: Vec<bool> },
    /// Start an agent on an account in a new tab of `workspace`.
    NewAgent {
        selected: usize,
        workspace: String,
        project: String,
        cwd: String,
    },
    /// Choose the current project or another project folder for a workspace.
    NewWorkspace {
        selected: usize,
        project: Option<String>,
    },
    /// The first-run setup (see `orcha_welcome`).
    Welcome(super::orcha_welcome::Welcome),
    /// A newer orcha is out: update now, later, or skip that version.
    Update {
        selected: usize,
        release: crate::orcha_app::Release,
    },
    /// How a Claude chat moves to another account: keep it running on the
    /// new sign-in, or restart Claude on the account (same chat).
    SwitchAccount {
        selected: usize,
        id: String,
        label: String,
        pane: String,
        /// Why a restart cannot pick this account, when it cannot.
        restart_blocked: Option<String>,
    },
    /// A plugin installed on this machine that Orcha can add (only on consent).
    PluginOffer {
        selected: usize,
        plugin: crate::orcha_app::OfferedPlugin,
    },
    /// A project was just added: write the default working rules into it?
    RulesOffer {
        selected: usize,
        folder: std::path::PathBuf,
        file: String,
        rules: Vec<String>,
    },
    /// Fuel is low: ask the agent in `pane` for its hand-off checklist?
    /// Asked, never sent on its own, so a half-typed prompt is not sent with it.
    Checklist {
        selected: usize,
        pane: String,
        kind: String,
        left: u8,
        file: String,
    },
    /// Remove a plugin? Keep is preselected.
    PluginRemove {
        selected: usize,
        plugin_id: String,
        name: String,
    },
    /// A newer version of a GitHub plugin, shown once per version.
    PluginUpdate {
        selected: usize,
        update: crate::orcha_plugin_updates::PluginUpdate,
        name: String,
    },
    /// What changed in this build, shown once after an update.
    WhatsNew { release: crate::orcha_app::Release },
    /// Claude sign-ins found on this machine (`OrchaUiState::found_accounts`),
    /// each with a check; the primary button adds the checked ones.
    FoundAccounts { selected: usize, chosen: Vec<bool> },
    /// A file shown read-only (`OrchaUiState::file_view` holds its text).
    FileView { path: String },
    /// Ctrl+click on a file whose type has no saved app: choose one, and
    /// keep it for the extension when `remember` is on.
    OpenWith {
        selected: usize,
        path: String,
        line: u32,
        /// Lowercase extension; empty when the choice can't be kept.
        extension: String,
        choices: Vec<OpenChoice>,
        remember: bool,
    },
}

/// Where a clicked file opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OpenChoice {
    /// The app the system registered for the type (viewer, browser, ...).
    System,
    /// The folder holding the file, in the file manager.
    Reveal,
    /// An editor: `command` opens it (`{path}`), empty for the first found.
    Editor { name: String, command: String },
}

impl OpenChoice {
    /// The value kept in `[orcha.open_with]`.
    pub(crate) fn config_value(&self) -> String {
        match self {
            Self::System => "system".to_owned(),
            Self::Reveal => "folder".to_owned(),
            Self::Editor { command, .. } if command.is_empty() => "editor".to_owned(),
            Self::Editor { command, .. } => format!("editor:{command}"),
        }
    }

    pub(crate) fn from_config_value(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "folder" => Some(Self::Reveal),
            "editor" => Some(Self::Editor {
                name: String::new(),
                command: String::new(),
            }),
            other => other.strip_prefix("editor:").map(|command| Self::Editor {
                name: String::new(),
                command: command.to_owned(),
            }),
        }
    }

    fn title(&self) -> String {
        match self {
            Self::System => "Default app".to_owned(),
            Self::Reveal => "Show in folder".to_owned(),
            Self::Editor { name, command } if command.is_empty() || name.is_empty() => {
                "Editor".to_owned()
            }
            Self::Editor { name, .. } => name.clone(),
        }
    }

    fn detail(&self, extension: &str) -> String {
        match self {
            Self::System if extension.is_empty() => {
                "the app this computer uses for the file".to_owned()
            }
            Self::System => format!("the app this computer uses for .{extension} files"),
            Self::Reveal => "open the folder and pick the file yourself".to_owned(),
            Self::Editor { command, .. } if command.is_empty() => {
                "your code editor, at the line the agent printed".to_owned()
            }
            Self::Editor { .. } => "open the file in this editor".to_owned(),
        }
    }
}

/// Click targets of the open popup.
#[derive(Debug, Default, Clone)]
pub(super) struct PopupHits {
    pub area: Rect,
    pub close: Rect,
    /// (row, index)
    pub rows: Vec<(Rect, usize)>,
    /// (button, index)
    pub actions: Vec<(Rect, usize)>,
    /// The footer's extra action on the right (accounts: refresh).
    pub extra: Rect,
}

/// NEW PROJECT: current project, browse, enter a path, new worktree.
pub(super) const NEW_WORKSPACE_ROWS: usize = 4;

/// (agent kind, mode, label)
pub(super) const HANDOFF_TARGETS: [(&str, &str, &str); 6] = [
    ("codex", "full", "Codex · whole conversation"),
    ("codex", "summary", "Codex · summarize first, then continue"),
    ("claude", "full", "Claude · whole conversation"),
    (
        "claude",
        "summary",
        "Claude · summarize first, then continue",
    ),
    ("cursor", "full", "Cursor · whole conversation"),
    (
        "cursor",
        "summary",
        "Cursor · summarize first, then continue",
    ),
];

/// Draws the open popup over the composed frame.
pub(super) fn render_popup(
    buffer: &mut Buffer,
    popup: &OrchaPopup,
    accounts: Option<&[Account]>,
    ui: &OrchaUiState,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
) -> PopupHits {
    let t = Brutal::new(&config.palette);
    let mut hits = PopupHits::default();
    match popup {
        OrchaPopup::Accounts { selected, pane } => {
            let _ = render_accounts(
                buffer,
                *selected,
                pane.as_deref(),
                accounts,
                snapshot,
                config,
                &t,
                &mut hits,
            );
        }
        OrchaPopup::AddAccount {
            selected,
            names,
            typed,
            ..
        } => {
            let _ = render_add_account(buffer, *selected, names, typed, &t, &mut hits);
        }
        OrchaPopup::Handoff {
            selected,
            pane,
            targets,
        } => {
            let _ = render_handoff(
                buffer, *selected, pane, targets, snapshot, config, &t, &mut hits,
            );
        }
        OrchaPopup::ProjectIcon {
            selected, label, ..
        } => {
            let _ = super::orcha_icons::render(buffer, *selected, label, &t, &mut hits);
        }
        OrchaPopup::RemoveAccount {
            selected, label, ..
        } => {
            let _ = render_remove_account(buffer, *selected, label, &t, &mut hits);
        }
        OrchaPopup::Palette { selected, query } => {
            let _ = super::orcha_palette::render(buffer, *selected, query, &t, &mut hits);
        }
        OrchaPopup::Broadcast { typed, panes } => {
            let _ = super::orcha_palette::render_broadcast(
                buffer, typed, panes, snapshot, config, &t, &mut hits,
            );
        }
        OrchaPopup::Import { selected, chosen } => {
            let _ = render_import(buffer, *selected, chosen, &ui.import_sources, &t, &mut hits);
        }
        OrchaPopup::Welcome(welcome) => {
            let _ = super::orcha_welcome::render(buffer, welcome, ui, config, &t, &mut hits);
        }
        OrchaPopup::NewAgent {
            selected, project, ..
        } => {
            let _ = render_new_agent(buffer, *selected, project, accounts, config, &t, &mut hits);
        }
        OrchaPopup::NewWorkspace { selected, project } => {
            let _ = render_new_workspace(buffer, *selected, project.as_deref(), &t, &mut hits);
        }
        OrchaPopup::SwitchAccount {
            selected,
            label,
            restart_blocked,
            ..
        } => {
            let _ = render_switch_account(
                buffer,
                *selected,
                label,
                restart_blocked.as_deref(),
                &t,
                &mut hits,
            );
        }
        OrchaPopup::PluginOffer { selected, plugin } => {
            let rows: Vec<_> = plugin_offer_rows(plugin)
                .into_iter()
                .map(|(_, title, detail, action)| (title, detail, action))
                .collect();
            let _ = render_choice_rows(
                buffer,
                *selected,
                "PLUGIN",
                &format!("Add {} to Orcha? It installs from GitHub.", plugin.name),
                &rows,
                &t,
                &mut hits,
            );
        }
        OrchaPopup::Update { selected, release } => {
            let _ = render_update(buffer, *selected, release, &t, &mut hits);
        }
        OrchaPopup::PluginUpdate {
            selected,
            update,
            name,
        } => {
            let rows = plugin_update_rows(update, name);
            let _ = render_choice_rows(
                buffer,
                *selected,
                "PLUGIN",
                &format!("{} · Update", update.notice(name)),
                &rows,
                &t,
                &mut hits,
            );
        }
        OrchaPopup::RulesOffer {
            selected,
            folder,
            file,
            rules,
        } => {
            let name = folder
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let rows = rules_offer_rows(file, rules);
            let _ = render_choice_rows(
                buffer,
                *selected,
                "WORKING RULES",
                &format!("Working rules for {name}?"),
                &rows,
                &t,
                &mut hits,
            );
        }
        OrchaPopup::Checklist {
            selected,
            kind,
            left,
            file,
            ..
        } => {
            let rows = checklist_rows(kind, file);
            let _ = render_choice_rows(
                buffer,
                *selected,
                "CHECKLIST",
                &format!("{left}% left · ask {kind} for a hand-off checklist?"),
                &rows,
                &t,
                &mut hits,
            );
        }
        OrchaPopup::PluginRemove { selected, name, .. } => {
            let rows = plugin_remove_rows(name);
            let _ = render_choice_rows(
                buffer,
                *selected,
                "PLUGIN",
                &format!("Remove {name}?"),
                &rows,
                &t,
                &mut hits,
            );
        }
        OrchaPopup::WhatsNew { release } => {
            let _ = render_whats_new(buffer, release, &t, &mut hits);
        }
        OrchaPopup::FoundAccounts { selected, chosen } => {
            let _ =
                render_found_accounts(buffer, *selected, chosen, &ui.found_accounts, &t, &mut hits);
        }
        OrchaPopup::FileView { .. } => {
            if let Some(view) = ui.file_view.as_ref() {
                let _ = super::file_viewer::render(buffer, view, &t, &mut hits);
            }
        }
        OrchaPopup::OpenWith {
            selected,
            path,
            extension,
            choices,
            remember,
            ..
        } => {
            let _ = render_open_with(
                buffer, *selected, path, extension, choices, *remember, &t, &mut hits,
            );
        }
    }
    hits
}

/// The agent a popup acts on: the focused pane's.
pub(super) fn focused_agent_pane(snapshot: &ClientShellSnapshot) -> Option<String> {
    let focused = snapshot.focused_pane_id.as_deref()?;
    snapshot
        .agents
        .iter()
        .find(|agent| agent.pane_id == focused && agent.agent_status != AgentStatus::Unknown)
        .map(|agent| agent.pane_id.clone())
        .or_else(|| Some(focused.to_owned()))
}

#[cfg(test)]
mod plugin_offer_tests {
    #[test]
    fn the_offer_adds_only_on_consent_and_offers_autostart_when_possible() {
        let plugin = crate::orcha_app::OfferedPlugin {
            id: "easy-job",
            name: "Easy Job",
            install: crate::orcha_app::EASY_JOB_INSTALL,
            autostart: Some(("server", &[("easy_job", "bridge")])),
        };
        let choices: Vec<_> = super::plugin_offer_rows(&plugin)
            .into_iter()
            .map(|row| row.0)
            .collect();
        assert_eq!(
            choices,
            [
                super::PluginOfferChoice::Add { autostart: false },
                super::PluginOfferChoice::Add { autostart: true },
                super::PluginOfferChoice::NotNow,
                super::PluginOfferChoice::Never,
            ]
        );
        let plain = crate::orcha_app::OfferedPlugin {
            autostart: None,
            ..plugin
        };
        assert_eq!(super::plugin_offer_rows(&plain).len(), 3);
    }
}

#[cfg(test)]
mod switch_account_tests {
    #[test]
    fn restart_is_offered_unless_the_account_cannot_be_picked() {
        let rows = super::switch_account_rows("Claude · work", None);
        assert!(rows[0].0.contains("swap the sign-in"));
        assert!(rows[1].0.contains("Restart Claude on Claude · work"));
        assert!(rows[1].3);
        let blocked =
            super::switch_account_rows("Claude · work", Some("shares the default sign-in"));
        assert!(!blocked[1].3);
        assert_eq!(blocked[1].1, "shares the default sign-in");
    }
}

#[cfg(test)]
mod list_window_tests {
    use super::{list_window, ROW};
    use ratatui::layout::Rect;

    #[test]
    fn a_short_list_shows_whole() {
        let area = Rect::new(0, 0, 40, 11);
        assert_eq!(list_window(area, ROW, 3, 2), (area, 0, 3));
    }

    #[test]
    fn the_cursor_row_is_always_in_the_window() {
        // 15 lines: 3 rows fit with "↓ more", 3 with both hints
        let area = Rect::new(0, 5, 40, 15);
        for cursor in 0..20 {
            let (list, start, end) = list_window(area, ROW, 20, cursor);
            assert!(
                start <= cursor && cursor < end,
                "cursor {cursor}: {start}..{end}"
            );
            let rows = u16::try_from(end - start).expect("rows");
            assert!(rows * (ROW + 1) - 1 <= list.height, "fits");
            assert!(
                list.y >= area.y && list.bottom() < area.bottom(),
                "hint room"
            );
            if start > 0 {
                assert_eq!(list.y, area.y + 1, "room for ↑ more");
            }
        }
        // the end of the list ends the window
        assert_eq!(list_window(area, ROW, 20, 19).2, 20);
        // a cursor past the end (a pinned row below) shows the last rows
        assert_eq!(list_window(area, ROW, 20, 25).2, 20);
    }
}
