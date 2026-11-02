use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClientGlobalMenuAction {
    Binding(crate::input::KeybindAction),
    WhatsNew,
    /// orcha: bring in projects from Orca, VS Code, Cursor, JetBrains.
    ImportProjects,
    /// orcha: start an agent in a new tab of the focused project.
    NewAgent,
    /// orcha: the first-run setup again.
    Setup,
    /// orcha: install the newer orcha.
    UpdateOrcha,
    /// orcha: COMMANDS with the plugin updates.
    UpdatePlugins,
}

/// orcha's own news (herdr's update check is off in orcha): each lights the
/// menu and adds an item to it. Client state, see `OrchaUiState::menu_news`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct OrchaMenuNews {
    /// A newer orcha is out.
    pub(super) update: bool,
    /// A plugin has an update this orcha can run.
    pub(super) plugin_update: bool,
}

pub(super) fn global_menu_attention(snapshot: &ClientShellSnapshot) -> bool {
    snapshot.update_available.is_some() || snapshot.integration_updates_available
}

pub(super) fn global_menu_item_has_badge(
    snapshot: &ClientShellSnapshot,
    action: ClientGlobalMenuAction,
) -> bool {
    matches!(
        action,
        ClientGlobalMenuAction::UpdateOrcha | ClientGlobalMenuAction::UpdatePlugins
    ) || (action == ClientGlobalMenuAction::WhatsNew && snapshot.update_available.is_some())
        || (action == ClientGlobalMenuAction::Binding(crate::input::KeybindAction::Settings)
            && snapshot.integration_updates_available)
}

pub(super) fn global_menu_items(
    snapshot: &ClientShellSnapshot,
    news: OrchaMenuNews,
) -> Vec<(&'static str, ClientGlobalMenuAction)> {
    let mut items = vec![
        (
            "settings",
            ClientGlobalMenuAction::Binding(crate::input::KeybindAction::Settings),
        ),
        (
            "keybinds",
            ClientGlobalMenuAction::Binding(crate::input::KeybindAction::Help),
        ),
        (
            "reload config",
            ClientGlobalMenuAction::Binding(crate::input::KeybindAction::ReloadConfig),
        ),
    ];
    if snapshot.update_available.is_some() || snapshot.latest_release_notes_available {
        items.push((
            if snapshot.update_available.is_some() {
                "update ready"
            } else {
                "what's new"
            },
            ClientGlobalMenuAction::WhatsNew,
        ));
    }
    if news.update {
        items.push(("update orcha", ClientGlobalMenuAction::UpdateOrcha));
    }
    if news.plugin_update {
        items.push(("update plugins", ClientGlobalMenuAction::UpdatePlugins));
    }
    let orcha = crate::orcha_app::is_orcha();
    if orcha {
        items.push(("new agent", ClientGlobalMenuAction::NewAgent));
        items.push(("setup", ClientGlobalMenuAction::Setup));
        items.push(("import projects", ClientGlobalMenuAction::ImportProjects));
    }
    items.push((
        // orcha: plain words; the agents keep running either way
        if orcha { "quit to terminal" } else { "detach" },
        ClientGlobalMenuAction::Binding(crate::input::KeybindAction::Detach),
    ));
    items
}

impl ClientShellState {
    pub(super) fn toggle_global_menu(&mut self) {
        if matches!(self.overlay, Some(ClientShellOverlay::GlobalMenu(_))) {
            self.overlay = None;
        } else {
            self.overlay = Some(ClientShellOverlay::GlobalMenu(ClientGlobalMenuOverlay {
                highlighted: 0,
            }));
        }
    }

    pub(super) fn move_global_menu_selection(&mut self, delta: isize) {
        let news = self.orcha_ui.menu_news();
        let item_count = self
            .snapshot
            .as_deref()
            .map_or(0, |snapshot| global_menu_items(snapshot, news).len());
        let Some(ClientShellOverlay::GlobalMenu(menu)) = self.overlay.as_mut() else {
            return;
        };
        menu.highlighted = (menu.highlighted as isize + delta)
            .clamp(0, item_count.saturating_sub(1) as isize) as usize;
    }

    pub(super) fn activate_global_menu_item(
        &mut self,
        index: usize,
        outcome: &mut ClientShellInput,
    ) {
        let news = self.orcha_ui.menu_news();
        let Some(action) = self.snapshot.as_deref().and_then(|snapshot| {
            global_menu_items(snapshot, news)
                .get(index)
                .map(|(_, action)| *action)
        }) else {
            return;
        };
        self.activate_global_menu_action(action, outcome);
    }

    pub(super) fn activate_global_menu_action(
        &mut self,
        action: ClientGlobalMenuAction,
        outcome: &mut ClientShellInput,
    ) {
        if action == ClientGlobalMenuAction::WhatsNew
            && self
                .snapshot
                .as_deref()
                .and_then(|snapshot| snapshot.release_notes.as_ref())
                .is_none()
        {
            return;
        }
        self.overlay = None;
        match action {
            ClientGlobalMenuAction::Binding(binding) => {
                self.record_binding(crate::input::KeybindMatch::Action(binding), outcome)
            }
            ClientGlobalMenuAction::WhatsNew => self.open_release_notes(),
            ClientGlobalMenuAction::ImportProjects => self.start_import_scan(),
            ClientGlobalMenuAction::NewAgent => self.open_new_agent_popup(),
            ClientGlobalMenuAction::Setup => self.open_welcome(outcome),
            ClientGlobalMenuAction::UpdateOrcha => self.start_update(),
            ClientGlobalMenuAction::UpdatePlugins => self.open_palette_with("update".to_owned()),
        }
        outcome.repaint = true;
    }
}
