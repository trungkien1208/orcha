use super::*;
use crate::api::schema::Method;
use crossterm::event::{MouseButton, MouseEventKind};

fn close_state(confirm: bool, tab_count: usize) -> ClientShellState {
    let mut projected = snapshot();
    for number in 2..=tab_count {
        let mut tab = projected.tabs[0].clone();
        tab.tab_id = format!("tab_{number}");
        tab.number = number;
        tab.focused = false;
        projected.tabs.push(tab);
    }
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.config.confirm_close = confirm;
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    state.compose(106, 24).unwrap();
    state
}

fn click(state: &mut ClientShellState, rect: Rect) -> ClientShellInput {
    state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x,
        row: rect.y,
        modifiers: KeyModifiers::empty(),
    })])
}

fn request_close(state: &mut ClientShellState, menu: bool) -> ClientShellInput {
    if menu {
        let pane = state.hits.panes[0].rect;
        state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: pane.x + 1,
            row: pane.y,
            modifiers: KeyModifiers::empty(),
        })]);
        state.compose(106, 24).unwrap();
        let index = match state.overlay.as_ref() {
            Some(ClientShellOverlay::ContextMenu(menu)) => menu
                .items()
                .iter()
                .position(|item| item.action == ClientContextMenuAction::ClosePane)
                .unwrap(),
            _ => panic!("pane menu"),
        };
        let close_row = state.hits.context_menu_rows[index].0;
        click(state, close_row)
    } else {
        let mut outcome = ClientShellInput::default();
        state.record_binding(
            crate::input::KeybindMatch::Action(crate::input::KeybindAction::ClosePane),
            &mut outcome,
        );
        outcome
    }
}

fn assert_pane_close(outcome: &ClientShellInput) {
    let methods = outcome
        .actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => Some(&request.method),
            _ => None,
        })
        .filter(|method| !matches!(method, Method::TabFocus(_) | Method::PaneFocus(_)))
        .collect::<Vec<_>>();
    assert!(
        matches!(methods.as_slice(), [Method::PaneClose(target)] if target.pane_id == "pane_1")
    );
}

/// Closing the last pane keeps the project (the server opens a fresh shell),
/// so the client never asks to close the workspace.
#[test]
fn pane_close_is_immediate_even_for_the_last_pane() {
    for (confirm, tabs) in [(true, 1), (false, 1), (false, 2), (true, 2)] {
        for menu in [false, true] {
            let mut state = close_state(confirm, tabs);
            assert_pane_close(&request_close(&mut state, menu));
            assert!(state.overlay.is_none());
        }
    }
}

#[test]
fn last_pane_close_in_a_worktree_group_is_immediate() {
    for linked in [false, true] {
        let mut state = close_state(true, 1);
        let mut projected = state.snapshot.as_deref().unwrap().clone();
        projected.workspaces[0].worktree = Some(ClientShellWorktree {
            key: "repo".into(),
            label: "repo".into(),
            is_linked_worktree: linked,
        });
        let mut sibling = projected.workspaces[0].clone();
        sibling.workspace_id = "ws_2".into();
        sibling.active_tab_id = "tab_2".into();
        sibling.focused = false;
        sibling.worktree.as_mut().unwrap().is_linked_worktree = !linked;
        projected.workspaces.push(sibling);
        let mut tab = projected.tabs[0].clone();
        tab.tab_id = "tab_2".into();
        tab.workspace_id = "ws_2".into();
        tab.focused = false;
        projected.tabs.push(tab);
        state.set_snapshot(Box::new(projected));

        assert_pane_close(&request_close(&mut state, false));
        assert!(state.overlay.is_none());
    }
}

#[test]
fn ordinary_pane_close_stays_immediate_with_another_pane() {
    for menu in [false, true] {
        let mut state = close_state(true, 1);
        let mut projected = state.snapshot.as_deref().unwrap().clone();
        let mut pane = projected.panes[0].clone();
        pane.pane_id = "pane_2".into();
        pane.focused = false;
        projected.panes.push(pane);
        state.set_snapshot(Box::new(projected));
        assert_pane_close(&request_close(&mut state, menu));
        assert!(state.overlay.is_none());
    }
}
