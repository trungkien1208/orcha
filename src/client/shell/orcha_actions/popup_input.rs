//! Popup selection and keyboard/mouse navigation.

use super::super::orcha_popup::{OrchaPopup, PopupTarget};
use super::*;

impl ClientShellState {
    pub(in crate::client::shell) fn open_new_workspace_popup(&mut self) {
        let project = self
            .snapshot
            .as_deref()
            .and_then(|snapshot| {
                snapshot
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.focused)
            })
            .map(|workspace| workspace.label.clone());
        self.orcha_ui.popup = Some(super::super::orcha_popup::OrchaPopup::NewWorkspace {
            selected: 0,
            project,
        });
    }

    pub(super) fn popup_len(&self) -> usize {
        match &self.orcha_ui.popup {
            Some(super::super::orcha_popup::OrchaPopup::Accounts { .. }) => {
                self.accounts.data().map_or(0, <[_]>::len) + 1
            }
            Some(super::super::orcha_popup::OrchaPopup::AddAccount { names, .. }) => {
                names.len() + 1
            }
            Some(super::super::orcha_popup::OrchaPopup::RemoveAccount { .. }) => 2,
            Some(super::super::orcha_popup::OrchaPopup::ProjectIcon { .. }) => {
                super::super::orcha_icons::ICONS.len() + 1
            }
            Some(super::super::orcha_popup::OrchaPopup::Handoff { targets, .. }) => targets.len(),
            Some(super::super::orcha_popup::OrchaPopup::Palette { query, .. }) => {
                super::super::orcha_palette::matches(query).len()
            }
            Some(super::super::orcha_popup::OrchaPopup::Broadcast { .. }) => 1,
            Some(super::super::orcha_popup::OrchaPopup::NewAgent { .. }) => {
                self.accounts.data().map_or(0, <[_]>::len)
                    + super::super::orcha_popup::NEW_AGENT_EXTRA_ROWS
            }
            Some(super::super::orcha_popup::OrchaPopup::Import { .. }) => {
                self.orcha_ui.import_sources.len()
            }
            Some(super::super::orcha_popup::OrchaPopup::NewWorkspace { .. }) => {
                super::super::orcha_popup::NEW_WORKSPACE_ROWS
            }
            Some(super::super::orcha_popup::OrchaPopup::OpenWith { choices, .. }) => choices.len(),
            Some(super::super::orcha_popup::OrchaPopup::FoundAccounts { chosen, .. }) => {
                chosen.len()
            }
            Some(super::super::orcha_popup::OrchaPopup::Update { .. }) => 3,
            Some(super::super::orcha_popup::OrchaPopup::PluginUpdate { .. }) => 2,
            Some(super::super::orcha_popup::OrchaPopup::PluginRemove { .. }) => 2,
            Some(super::super::orcha_popup::OrchaPopup::FolderAccess { .. }) => 2,
            Some(super::super::orcha_popup::OrchaPopup::Checklist { .. }) => 2,
            Some(super::super::orcha_popup::OrchaPopup::RulesOffer { .. }) => 4,
            Some(super::super::orcha_popup::OrchaPopup::SwitchAccount { .. }) => 3,
            Some(super::super::orcha_popup::OrchaPopup::PluginOffer { plugin, .. }) => {
                if plugin.autostart.is_some() {
                    4
                } else {
                    3
                }
            }
            Some(super::super::orcha_popup::OrchaPopup::WhatsNew { .. }) => 1,
            Some(
                super::super::orcha_popup::OrchaPopup::Welcome(_)
                | super::super::orcha_popup::OrchaPopup::FileView { .. },
            )
            | None => 0,
        }
    }

    pub(super) fn popup_select(&mut self, index: usize) {
        if let Some(selected) = self
            .orcha_ui
            .popup
            .as_mut()
            .and_then(super::super::orcha_popup::OrchaPopup::selected_mut)
        {
            *selected = index;
        }
    }

    pub(super) fn popup_selected(&self) -> usize {
        self.orcha_ui
            .popup
            .as_ref()
            .map_or(0, super::super::orcha_popup::OrchaPopup::selected)
    }

    /// ADD ACCOUNT: arrows move, letters fill the name row, esc goes back.
    pub(super) fn route_add_account_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        use crossterm::event::KeyCode;
        outcome.repaint = true;
        let Some(super::super::orcha_popup::OrchaPopup::AddAccount {
            selected,
            names,
            typed,
            pane,
        }) = self.orcha_ui.popup.clone()
        else {
            return;
        };
        let len = names.len() + 1;
        match key.code {
            KeyCode::Esc => {
                self.orcha_ui.popup = Some(super::super::orcha_popup::OrchaPopup::Accounts {
                    selected: self.accounts.data().map_or(0, <[_]>::len),
                    pane,
                });
            }
            KeyCode::Up | KeyCode::BackTab if len > 0 => {
                self.popup_select((selected + len - 1) % len);
            }
            KeyCode::Down | KeyCode::Tab if len > 0 => {
                self.popup_select((selected + 1) % len);
            }
            KeyCode::Backspace => {
                let mut typed = typed;
                typed.pop();
                self.orcha_ui.popup = Some(super::super::orcha_popup::OrchaPopup::AddAccount {
                    selected: names.len(),
                    names,
                    typed,
                    pane,
                });
            }
            KeyCode::Enter => self.popup_activate(PopupTarget::Row(selected), false, outcome),
            KeyCode::Char(ch) if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' => {
                let mut typed = typed;
                typed.push(ch.to_ascii_lowercase());
                self.orcha_ui.popup = Some(super::super::orcha_popup::OrchaPopup::AddAccount {
                    selected: names.len(),
                    names,
                    typed,
                    pane,
                });
            }
            _ => {}
        }
    }

    /// Keys while an orcha popup is open: arrows/j k move, ↵ acts, ⌥↵ the
    /// alternate action, H hands off, esc closes.
    pub(in crate::client::shell) fn route_orcha_popup_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        use crossterm::event::{KeyCode, KeyModifiers};
        // too small a screen draws no popup: only esc acts on the unseen one
        if key.code != KeyCode::Esc
            && !super::super::orcha_popup::popup_fits_screen(self.last_composed_size)
        {
            return;
        }
        // popups that handle their own keys
        match self.orcha_ui.popup {
            Some(OrchaPopup::Welcome(_)) => return self.route_welcome_key(key, outcome),
            Some(OrchaPopup::AddAccount { .. }) => return self.route_add_account_key(key, outcome),
            Some(OrchaPopup::Palette { .. }) => return self.route_palette_key(key, outcome),
            Some(OrchaPopup::Broadcast { .. }) => return self.route_broadcast_key(key, outcome),
            Some(OrchaPopup::ProjectIcon { .. }) => {
                return self.route_project_icon_key(key, outcome)
            }
            Some(OrchaPopup::FileView { .. }) => return self.route_file_view_key(key, outcome),
            _ => {}
        }
        outcome.repaint = true;
        let len = self.popup_len();
        let selected = self.popup_selected();
        if matches!(
            self.orcha_ui.popup,
            Some(super::super::orcha_popup::OrchaPopup::FoundAccounts { .. })
        ) {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.dismiss_found_accounts();
                    return;
                }
                KeyCode::Char(' ') => {
                    self.toggle_found_account(selected);
                    return;
                }
                KeyCode::Enter => {
                    self.import_found_accounts();
                    return;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.popup_back(),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab if len > 0 => {
                self.popup_select((selected + 1) % len);
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab if len > 0 => {
                self.popup_select((selected + len - 1) % len);
            }
            KeyCode::Enter
                if matches!(
                    self.orcha_ui.popup,
                    Some(super::super::orcha_popup::OrchaPopup::Import { .. })
                ) =>
            {
                self.import_chosen(outcome);
            }
            KeyCode::Char(' ')
                if matches!(
                    self.orcha_ui.popup,
                    Some(super::super::orcha_popup::OrchaPopup::Import { .. })
                ) =>
            {
                self.toggle_import(selected);
            }
            KeyCode::Char(' ')
                if matches!(
                    self.orcha_ui.popup,
                    Some(super::super::orcha_popup::OrchaPopup::OpenWith { .. })
                ) =>
            {
                self.toggle_open_with_remember();
            }
            KeyCode::Enter => {
                let alternate = key.modifiers.contains(KeyModifiers::ALT);
                self.popup_activate(PopupTarget::Row(selected), alternate, outcome);
            }
            KeyCode::Char('p') | KeyCode::Char('P')
                if matches!(
                    self.orcha_ui.popup,
                    Some(super::super::orcha_popup::OrchaPopup::NewAgent { .. })
                ) =>
            {
                self.open_new_workspace_popup();
            }
            KeyCode::Char('h') | KeyCode::Char('H')
                if matches!(
                    self.orcha_ui.popup,
                    Some(super::super::orcha_popup::OrchaPopup::Accounts { .. })
                ) =>
            {
                self.open_handoff_popup();
            }
            KeyCode::Char('r') | KeyCode::Char('R')
                if matches!(
                    self.orcha_ui.popup,
                    Some(super::super::orcha_popup::OrchaPopup::Accounts { .. })
                ) =>
            {
                self.refresh_fuel();
            }
            KeyCode::Char('i') | KeyCode::Char('I')
                if matches!(
                    self.orcha_ui.popup,
                    Some(super::super::orcha_popup::OrchaPopup::Accounts { .. })
                ) =>
            {
                self.start_account_scan(true);
            }
            KeyCode::Char('d') | KeyCode::Delete
                if matches!(
                    self.orcha_ui.popup,
                    Some(super::super::orcha_popup::OrchaPopup::Accounts { .. })
                ) =>
            {
                self.ask_remove_account(selected);
            }
            _ => {}
        }
    }

    /// Esc: a popup opened from the accounts popup goes back to it; the
    /// others close.
    pub(super) fn popup_back(&mut self) {
        use super::super::orcha_popup::OrchaPopup;
        let back = match self.orcha_ui.popup.take() {
            Some(OrchaPopup::RemoveAccount { id, pane, .. }) => Some((pane, Some(id))),
            Some(OrchaPopup::SwitchAccount { id, pane, .. }) => Some((Some(pane), Some(id))),
            Some(OrchaPopup::Handoff { pane, .. }) if self.orcha_ui.handoff_from_accounts => {
                Some((Some(pane), None))
            }
            _ => None,
        };
        let Some((pane, on)) = back else {
            return;
        };
        let mut popup = self.accounts_popup(pane);
        // back on the account the sub-popup was about
        let row = on.and_then(|id| {
            self.accounts
                .data()?
                .iter()
                .position(|account| account.id == id)
        });
        if let (OrchaPopup::Accounts { selected, .. }, Some(row)) = (&mut popup, row) {
            *selected = row;
        }
        self.orcha_ui.popup = Some(popup);
    }

    /// Mouse while an orcha popup is open: rows select, buttons act, a click
    /// outside or on ✕ closes.
    pub(in crate::client::shell) fn handle_orcha_popup_mouse(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        outcome: &mut ClientShellInput,
    ) {
        use crossterm::event::{MouseButton, MouseEventKind};
        if matches!(
            self.orcha_ui.popup,
            Some(super::super::orcha_popup::OrchaPopup::Welcome(_))
        ) {
            self.handle_welcome_mouse(mouse, outcome);
            return;
        }
        if matches!(
            self.orcha_ui.popup,
            Some(super::super::orcha_popup::OrchaPopup::FileView { .. })
        ) {
            let lines = match mouse.kind {
                MouseEventKind::ScrollUp => -3,
                MouseEventKind::ScrollDown => 3,
                _ => 0,
            };
            if lines != 0 {
                if let Some(view) = self.orcha_ui.file_view.as_mut() {
                    view.scroll_by(lines);
                    outcome.repaint = true;
                }
                return;
            }
        }
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let point = (mouse.column, mouse.row);
        let hits = self.hits.orcha.popup.clone();
        outcome.repaint = true;
        if super::super::contains(hits.close, point) || !super::super::contains(hits.area, point) {
            if matches!(
                self.orcha_ui.popup,
                Some(super::super::orcha_popup::OrchaPopup::FoundAccounts { .. })
            ) {
                self.dismiss_found_accounts();
                return;
            }
            self.orcha_ui.popup = None;
            self.orcha_ui.file_view = None;
            return;
        }
        if super::super::contains(hits.extra, point)
            && matches!(
                self.orcha_ui.popup,
                Some(super::super::orcha_popup::OrchaPopup::Accounts { .. })
            )
        {
            self.refresh_fuel();
            return;
        }
        if let Some((_, target)) = hits
            .actions
            .iter()
            .find(|(rect, _)| super::super::contains(*rect, point))
        {
            self.popup_activate(*target, false, outcome);
            return;
        }
        if let Some((_, index)) = hits
            .rows
            .iter()
            .find(|(rect, _)| super::super::contains(*rect, point))
        {
            self.popup_select(*index);
        }
    }
}
