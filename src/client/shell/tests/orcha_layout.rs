//! orcha: the five regions (header, left sidebar, main, right git panel,
//! bottom status bar). Run with `--nocapture` to see the composed screen.
use super::*;
use crate::client::shell::git_panel::GitPanelData;
use crate::protocol::{ClientShellPane, ClientShellTabStatusSegment};

fn orcha_state(right_panel: bool, status_bar: bool) -> ClientShellState {
    let mut projected = snapshot();
    projected.tab_bar_right = vec![
        ClientShellTabStatusSegment {
            text: "#42 Dark mode for the menu ● Open".into(),
            accent: false,
        },
        ClientShellTabStatusSegment {
            text: "5h ███░░ 40%".into(),
            accent: false,
        },
        ClientShellTabStatusSegment {
            text: "17:50".into(),
            accent: false,
        },
    ];
    let ws = projected.workspaces[0].workspace_id.clone();
    let tab = projected.tabs[0].tab_id.clone();
    let agent = |pane: &str, name: &str, status: AgentStatus, focused: bool| ClientShellAgent {
        pane_id: pane.into(),
        workspace_id: ws.clone(),
        tab_id: tab.clone(),
        name: None,
        display_agent: Some(name.into()),
        agent: Some(name.to_lowercase()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: status,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused,
    };
    projected.agents = vec![
        agent("pane_1", "Claude", AgentStatus::Working, true),
        agent("pane_2", "Codex", AgentStatus::Done, false),
    ];
    let mut config = ClientShellConfig::from_config(&Config::default());
    config.right_panel = right_panel;
    config.status_bar = status_bar;
    config.sidebar_tree = right_panel;
    let mut state = ClientShellState::new(config);
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    state.git_panel.set_for_test(Some(GitPanelData {
        root: "/tmp/matcha-house".into(),
        branch: "feature/42-dark-menu".into(),
        upstream: Some("origin/feature/42-dark-menu".into()),
        ahead: 2,
        behind: 0,
        files: vec![
            ("M".into(), "src/theme.ts".into()),
            ("??".into(), "src/components/settings/Menu.tsx".into()),
            ("D".into(), "old.js".into()),
        ],
        worktrees: vec![
            ("main".into(), false),
            ("feature/42-dark-menu".into(), true),
        ],
        insertions: 26,
        deletions: 4,
        last_commit: Some(("dark mode toggle".into(), "2 minutes ago".into())),
        history: demo_history(),
    }));
    state
}

fn demo_history() -> Vec<crate::client::shell::git_panel::HistoryLine> {
    use crate::client::shell::git_panel::HistoryLine;
    let line = |graph: &str, refs: &[&str], subject: &str| HistoryLine {
        graph: graph.into(),
        refs: refs.iter().map(|r| (*r).to_owned()).collect(),
        subject: subject.into(),
    };
    vec![
        line("*", &["feature/42-dark-menu"], "dark mode toggle"),
        line("*", &[], "theme tokens"),
        line("| *", &["orca"], "apple pay wip"),
        line("|/", &[], ""),
        line("*", &[], "fix menu spacing"),
        line("*", &["main"], "release 1.4"),
    ]
}

#[test]
fn five_regions_compose() {
    let mut state = orcha_state(true, true);
    let frame = state.compose(140, 30).expect("frame");
    let rows = frame_rows(&frame);
    for row in &rows {
        eprintln!("|{row}|");
    }
    let layout = state.layout(140, 30);
    // one-row header and status bar at the very top and bottom, a column of margin
    assert_eq!(layout.status_bar, Rect::new(0, 29, 140, 1));
    assert_eq!(layout.header, Rect::new(0, 0, 140, 1));
    assert_eq!(layout.sidebar.y, 2, "a blank row under the header");
    assert_eq!(layout.right_panel.width, 34);
    assert_eq!(
        layout.tab_bar.y + 1,
        layout.main_frame.y,
        "tabs sit on the frame"
    );
    assert_eq!(
        layout.pane_surface.x, layout.main_frame.x,
        "panes fill the frame"
    );
    assert!(layout.main_frame.right() < layout.right_panel.x);
    assert!(rows[0].contains("ORCHA") && rows[0].contains("settings"));
    let last = &rows[29];
    assert!(
        last.contains("#42") && last.contains("Dark mode for the menu"),
        "status bar holds the right entries"
    );
    let all: String = rows.join("\n");
    assert!(all.contains("feature/42-dark-menu"));
    assert!(all.contains("↑2 to push"));
    assert!(all.contains("BRANCH") && all.contains("CHANGES") && all.contains("HISTORY"));
    assert!(all.contains("WORKTREES"));
    assert!(all.contains("PROJECTS") && all.contains("PROCESS"));
    assert!(all.contains("+ NEW") && all.contains("menu"));
    assert!(all.contains("1 ACTIVE"), "only active agents are trains");
    assert!(
        state.hits.agents.iter().any(|(_, pane)| pane == "pane_2"),
        "agent rows are clickable"
    );
    assert!(!state.hits.orcha.settings.is_empty());
    assert!(!state.hits.orcha.history_toggle.is_empty());
}

#[test]
fn folds_open_from_the_panel() {
    let mut state = orcha_state(true, true);
    state.orcha_ui.history_open = true;
    state.orcha_ui.worktrees_open = true;
    let frame = state.compose(140, 40).expect("frame");
    let all = frame_rows(&frame).join("\n");
    assert!(all.contains("NEW WORKTREE"));
    assert!(!state.hits.orcha.new_worktree.is_empty());
}

#[test]
fn new_button_offers_current_project_or_another_folder() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

    let mut current = orcha_state(true, true);
    current.compose(140, 40).expect("shell frame");
    let button = current.hits.new_workspace;
    let mut outcome = ClientShellInput::default();
    assert!(current.handle_orcha_click((button.x, button.y), &mut outcome));
    assert!(matches!(
        current.orcha_ui.popup.as_ref(),
        Some(crate::client::shell::orcha_popup::OrchaPopup::NewWorkspace {
            selected: 0,
            project: Some(project),
        }) if project == "client-shell"
    ));
    current.compose(140, 40).expect("new workspace choices");
    assert_eq!(current.hits.orcha.popup.actions.len(), 3);
    let create = current.hits.orcha.popup.actions[0].0;
    current.handle_orcha_popup_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: create.x,
            row: create.y,
            modifiers: KeyModifiers::empty(),
        },
        &mut outcome,
    );
    assert!(outcome.actions.iter().any(|action| matches!(
        action,
        ClientShellAction::Endpoint { request, .. }
            if matches!(&request.method, crate::api::schema::Method::WorkspaceCreate(_))
    )));

    let mut folder = orcha_state(true, true);
    folder.compose(140, 40).expect("shell frame");
    let button = folder.hits.new_workspace;
    folder.handle_orcha_click((button.x, button.y), &mut outcome);
    folder.compose(140, 40).expect("new workspace choices");
    let choose = folder.hits.orcha.popup.actions[2].0;
    folder.handle_orcha_popup_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: choose.x,
            row: choose.y,
            modifiers: KeyModifiers::empty(),
        },
        &mut outcome,
    );
    let Some(ClientShellOverlay::Rename(rename)) = folder.overlay.as_mut() else {
        panic!("folder choice should open a path prompt");
    };
    assert!(matches!(
        &rename.target,
        ClientRenameTarget::NewProjectFolder
    ));
    rename.input.insert("/tmp/another-project");
    folder.save_rename_overlay(&mut outcome);
    assert!(outcome.actions.iter().any(|action| matches!(
        action,
        ClientShellAction::Endpoint { request, .. }
            if matches!(
                &request.method,
                crate::api::schema::Method::WorkspaceCreate(params)
                    if params.source_workspace_id.is_none()
                        && params.cwd.as_deref() == Some("/tmp/another-project")
            )
    )));
}

#[test]
fn project_labels_disambiguate_home_without_overwriting_custom_names() {
    let mut snapshot = snapshot();
    for (path, expected) in [
        ("/Users/hibari/", "hibari #1"),
        (r"C:\Users\Hibari\", "Hibari #1"),
    ] {
        snapshot.workspaces[0].label = "~".into();
        snapshot.workspaces[0].new_workspace_cwd = path.into();
        super::super::orcha_projects::normalize_project_labels(&mut snapshot);
        assert_eq!(snapshot.workspaces[0].label, expected);
    }
    snapshot.workspaces[0].label = "~".into();
    snapshot.workspaces[0].custom_label = true;
    super::super::orcha_projects::normalize_project_labels(&mut snapshot);
    assert_eq!(snapshot.workspaces[0].label, "~");
}

#[test]
fn project_folder_picker_confirms_selection_and_handles_cancel_or_failure() {
    let mut state = orcha_state(true, true);
    let complete = |state: &mut ClientShellState, result| {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(result).expect("picker result");
        state.orcha_ui.project_folder_picker = Some(rx);
        state.poll_project_folder_picker()
    };
    assert!(!complete(&mut state, Ok(None)));
    assert!(state.overlay.is_none());
    assert!(complete(
        &mut state,
        Ok(Some("/tmp/project with spaces".into()))
    ));
    let Some(ClientShellOverlay::Rename(rename)) = state.overlay.as_ref() else {
        panic!("the selected path needs confirmation");
    };
    assert_eq!(rename.input.trim(), "/tmp/project with spaces");
    let mut outcome = ClientShellInput::default();
    state.save_rename_overlay(&mut outcome);
    assert!(outcome.actions.iter().any(|action| matches!(
        action,
        ClientShellAction::Endpoint { request, .. }
            if matches!(&request.method, crate::api::schema::Method::WorkspaceCreate(params)
                if params.cwd.as_deref() == Some("/tmp/project with spaces"))
    )));
    assert!(complete(
        &mut state,
        Err(std::io::ErrorKind::NotFound.into())
    ));
    assert!(state.overlay.is_some(), "manual entry remains available");
    state.overlay = None;
    state.active_endpoint_id =
        ClientEndpointId::Ssh(crate::client::endpoint::ProfileId::generate());
    assert!(!complete(&mut state, Ok(Some("/tmp/local-project".into()))));
    assert!(
        state.overlay.is_none(),
        "local paths cannot reach a remote endpoint"
    );
}

#[test]
fn accounts_popup_lists_fuel() {
    use crate::client::shell::accounts::{Account, FuelWindow};
    let mut state = orcha_state(true, true);
    let window = |label: &str, left: u8, reset: &str| FuelWindow {
        key: label.to_lowercase(),
        label: label.into(),
        left,
        reset: Some(reset.into()),
    };
    state.accounts.set_for_test(Some(vec![
        Account {
            id: "work".into(),
            kind: "claude".into(),
            label: "Claude · company".into(),
            email: Some("me@example.com".into()),
            current: true,
            error: None,
            windows: vec![window("5H", 8, "1h 12m"), window("WEEK", 18, "3d 5h")],
            signed_in: None,
        },
        Account {
            id: "personal".into(),
            kind: "claude".into(),
            label: "Claude · personal".into(),
            email: Some("you@example.com".into()),
            current: false,
            error: None,
            windows: vec![window("5H", 70, "4h 02m"), window("WEEK", 64, "6d 1h")],
            signed_in: None,
        },
    ]));
    let panel = state.compose(140, 40).expect("fuel panel");
    let panel_text = frame_rows(&panel).join("\n");
    assert!(
        panel_text.contains("resets in 1h 12m") && panel_text.contains("resets in 3d 5h"),
        "the fuel panel says when 5H and WEEK reset:\n{panel_text}"
    );
    state.open_accounts_overlay();
    let frame = state.compose(140, 40).expect("frame");
    let all = frame_rows(&frame).join("\n");
    for row in frame_rows(&frame) {
        eprintln!("|{row}|");
    }
    assert!(all.contains("CONTINUE ON") && all.contains("IN USE") && all.contains("BEST"));
    assert!(
        all.contains("5H resets in 1h 12m") && all.contains("WEEK resets in 3d 5h"),
        "each account says when its windows reset:\n{all}"
    );
    assert!(
        all.contains("continue") && all.contains("8% left"),
        "says how low"
    );
    assert!(
        all.contains("ADD ACCOUNT"),
        "there is a place to add one:\n{all}"
    );
    assert_eq!(
        state.hits.orcha.popup.actions.len(),
        2,
        "the other account switches, and ADD"
    );
    assert!(state.update_low_fuel_hint(), "8% left offers a switch");

    // NEW: the accounts plus a plain shell, the one with most fuel picked
    state.orcha_ui.popup = None;
    state.open_new_agent_popup();
    let frame = state.compose(140, 40).expect("frame");
    let all = frame_rows(&frame).join("\n");
    assert!(all.contains("NEW AGENT") && all.contains("Shell") && all.contains("START"));
    assert_eq!(state.hits.orcha.popup.rows.len(), 3);
    assert!(matches!(
        state.orcha_ui.popup,
        Some(crate::client::shell::orcha_popup::OrchaPopup::NewAgent { selected: 1, .. })
    ));
}

#[test]
fn accounts_popup_offers_sign_in_on_the_current_expired_row() {
    use crate::client::shell::accounts::Account;
    let mut state = orcha_state(true, true);
    state.accounts.set_for_test(Some(vec![Account {
        id: "work".into(),
        kind: "claude".into(),
        label: "Claude · work".into(),
        email: Some("me@example.com".into()),
        current: true,
        error: Some("sign-in expired: open Claude with this account once".into()),
        windows: Vec::new(),
        signed_in: Some(true),
    }]));
    state.open_accounts_overlay();
    let frame = state.compose(140, 40).expect("frame");
    let all = frame_rows(&frame).join("\n");
    assert!(
        all.contains("SIGN IN"),
        "expired current account still has SIGN IN:\n{all}"
    );
    assert!(
        all.contains("sign-in expired"),
        "shows why fuel is missing, not only the email:\n{all}"
    );
    assert!(all.contains("ADD ACCOUNT"));
    assert_eq!(state.hits.orcha.popup.actions.len(), 2);
}

#[test]
fn add_account_offers_a_free_name_and_what_you_type() {
    use crate::client::shell::accounts::Account;
    let mut state = orcha_state(true, true);
    state.accounts.set_for_test(Some(vec![Account {
        id: "claude-work".into(),
        kind: "claude".into(),
        label: "Claude · work".into(),
        email: None,
        current: true,
        error: None,
        windows: Vec::new(),
        signed_in: Some(true),
    }]));
    state.open_accounts_overlay();
    // down onto ADD, then open the name list
    state.handle_input_bytes(b"j\r");
    let frame = state.compose(140, 40).expect("add account");
    let all = frame_rows(&frame).join("\n");
    assert!(all.contains("ADD ACCOUNT"), "{all}");
    assert!(all.contains("oc personal"), "{all}");
    assert!(
        !all.contains("oc work"),
        "work is already an account:\n{all}"
    );
    state.handle_input_bytes(b"jo");
    let frame = state.compose(140, 40).expect("typed name");
    let all = frame_rows(&frame).join("\n");
    assert!(all.contains("oc jo"), "{all}");
}

#[test]
fn regions_off_is_plain_herdr() {
    let mut state = orcha_state(false, false);
    let frame = state.compose(140, 22).expect("frame");
    let layout = state.layout(140, 22);
    assert_eq!(layout.status_bar.height, 0);
    assert_eq!(layout.right_panel.width, 0);
    assert!(frame_rows(&frame)[0].contains("17:50"));
    assert_eq!(layout.main_frame, Rect::default());
}

#[test]
fn narrow_screens_drop_the_panel() {
    let state = orcha_state(true, true);
    assert_eq!(state.layout(100, 22).right_panel.width, 0);
}

/// Two bordered panes filling `size`, with demo content, as the server would send.
fn preview_surface(size: Rect) -> PaneSurfaceFrame {
    let mut buffer = Buffer::empty(Rect::new(0, 0, size.width, size.height));
    let half = size.width / 2;
    let panes = [
        (
            "pane_1",
            Rect::new(0, 0, half.saturating_sub(1), size.height),
            vec![
                "› add a dark mode toggle to the menu",
                "",
                "● I'll read the theme tokens first.",
                "  └ Read src/theme.ts (24 lines)",
                "",
                "● Update(src/Menu.tsx)",
                "  └ Added 3 lines, removed 1 line",
                "     12 + const [dark, setDark] = useTheme()",
                "     13 + const label = dark ? \"Light\" : \"Dark\"",
                "     14 - return <Toggle label=\"Dark mode\" />",
                "     14 + return <Toggle label={label} />",
                "",
                "● Update(src/theme.ts)",
                "  └ Added 1 line",
                "     31 + export const dark = { bg: \"#1a1916\" }",
                "",
                "● Bash(npm test -- Menu)",
                "  └ PASS  src/Menu.test.tsx (4 tests)",
                "",
                "✻ Checking contrast in both themes… 8s",
            ],
        ),
        (
            "pane_2",
            Rect::new(
                half + 1,
                0,
                size.width.saturating_sub(half + 1),
                size.height,
            ),
            vec![
                "› Explain src/theme.ts in two sentences.",
                "",
                "• src/theme.ts exports the design tokens: two",
                "  palettes (light and dark), a pink accent and",
                "  a corner radius of 12.",
                "",
                "• Components read them through useTheme(), so",
                "  switching palettes repaints the whole app",
                "  without a reload.",
                "",
                "  Worth knowing: the dark palette reuses the",
                "  light accent, so check pink-on-dark contrast.",
                "",
                "─ Worked for 14s ─────────────────────",
            ],
        ),
    ];
    let mut surface_panes = Vec::new();
    for (id, rect, lines) in &panes {
        let block = ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::ALL);
        ratatui::widgets::Widget::render(block, *rect, &mut buffer);
        for (row, line) in lines.iter().enumerate() {
            let y = rect.y + 1 + row as u16;
            if y + 1 < rect.bottom() {
                buffer.set_stringn(
                    rect.x + 2,
                    y,
                    line,
                    usize::from(rect.width - 3),
                    Style::default(),
                );
            }
        }
        let inner = Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, rect.height - 2);
        surface_panes.push(PaneSurfacePane {
            pane_id: (*id).into(),
            content_revision: 0,
            rect: SurfaceRect {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
            },
            inner_rect: SurfaceRect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: inner.height,
            },
            scrollbar_rect: None,
            scroll: None,
            focused: *id == "pane_1",
            mouse_reporting: false,
            sgr_pixel_mouse: false,
            alternate_screen_active: false,
            pixel_width: 0,
            pixel_height: 0,
        });
    }
    let mut base = surface();
    base.frame = FrameData::from_ratatui_buffer_with_hyperlinks(&buffer, None, &[]);
    base.panes = surface_panes;
    base
}

/// Writes a composed orcha frame as JSON cells for a PNG preview:
/// `ORCHA_PREVIEW_OUT=/tmp/f.json [ORCHA_PREVIEW_THEME=brutal-dark] [ORCHA_PREVIEW_POPUP=1]
///  cargo test --bin herdr orcha_preview -- --ignored`
#[test]
#[ignore]
fn orcha_preview() {
    use crate::client::shell::accounts::{Account, FuelWindow};
    let Ok(out) = std::env::var("ORCHA_PREVIEW_OUT") else {
        return;
    };
    let theme = std::env::var("ORCHA_PREVIEW_THEME").unwrap_or_else(|_| "orcha".into());
    let parsed: Config = toml::from_str(&format!("[theme]\nname = \"{theme}\"\n")).expect("toml");
    let (cols, rows) = (180u16, 48u16);
    let mut projected = snapshot();
    let first = projected.workspaces[0].clone();
    projected.workspaces[0].label = "matcha-house".into();
    projected.workspaces[0].branch = Some("feature/42-dark-menu".into());
    for (index, (name, branch)) in [
        ("orca-pay", "feature/108-apple-pay"),
        ("tide-pool", "main"),
        ("koi-notes", "main"),
        ("bento-cli", "feature/7-flags"),
        ("lantern-api", "main"),
    ]
    .iter()
    .enumerate()
    {
        let mut ws = first.clone();
        ws.workspace_id = format!("ws_{}", index + 2);
        ws.label = (*name).into();
        ws.branch = Some((*branch).into());
        ws.focused = false;
        projected.workspaces.push(ws);
    }
    let ws1 = projected.workspaces[0].workspace_id.clone();
    let tab = projected.tabs[0].tab_id.clone();
    let mut second_tab = projected.tabs[0].clone();
    second_tab.tab_id = "tab_2".into();
    second_tab.label = "explain theme".into();
    second_tab.number = 2;
    second_tab.focused = false;
    second_tab.agent_status = AgentStatus::Done;
    let mut third_tab = second_tab.clone();
    third_tab.tab_id = "tab_3".into();
    third_tab.label = "amex fix".into();
    third_tab.number = 3;
    third_tab.agent_status = AgentStatus::Blocked;
    projected.tabs[0].label = "dark mode".into();
    projected.tabs[0].agent_status = AgentStatus::Working;
    projected.tabs.push(second_tab);
    projected.tabs.push(third_tab);
    let mk = |pane: &str,
              ws: &str,
              kind: &str,
              model: &str,
              task: &str,
              stage: &str,
              status: AgentStatus,
              focused: bool| ClientShellAgent {
        pane_id: pane.into(),
        workspace_id: ws.into(),
        tab_id: tab.clone(),
        name: None,
        display_agent: Some(kind.into()),
        agent: Some(kind.into()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: status,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: vec![
            ("task".into(), task.into()),
            ("stage".into(), stage.into()),
            ("model".into(), model.into()),
        ],
        focused,
    };
    projected.agents = vec![
        mk(
            "pane_1",
            &ws1,
            "claude",
            "claude-opus-5-5",
            "dark mode",
            "testing",
            AgentStatus::Working,
            true,
        ),
        mk(
            "pane_2",
            &ws1,
            "codex",
            "gpt-6-luna",
            "explain theme",
            "done",
            AgentStatus::Done,
            false,
        ),
        mk(
            "pane_3",
            "ws_2",
            "codex",
            "gpt-6-luna",
            "apple pay",
            "coding",
            AgentStatus::Working,
            false,
        ),
        mk(
            "pane_4",
            "ws_2",
            "cursor",
            "cursor",
            "amex",
            "coding",
            AgentStatus::Blocked,
            false,
        ),
        mk(
            "pane_5",
            "ws_3",
            "claude",
            "claude-opus-5-5",
            "search",
            "done",
            AgentStatus::Done,
            false,
        ),
        mk(
            "pane_6",
            "ws_5",
            "claude",
            "claude-sonnet-5-5",
            "flags",
            "thinking",
            AgentStatus::Working,
            false,
        ),
    ];
    // the explain chat runs in the project's second tab
    projected.agents[1].tab_id = "tab_2".into();
    for pane in projected
        .panes
        .iter_mut()
        .filter(|pane| pane.pane_id == "pane_2")
    {
        pane.tab_id = "tab_2".into();
    }
    projected.tab_bar_right = vec![
        ClientShellTabStatusSegment {
            text: "#42 Dark mode for the menu ● Open".into(),
            accent: false,
        },
        ClientShellTabStatusSegment {
            text: "✎ 3 files +26 −4".into(),
            accent: false,
        },
        ClientShellTabStatusSegment {
            text: "◷ 18m".into(),
            accent: false,
        },
        ClientShellTabStatusSegment {
            text: "17:50".into(),
            accent: false,
        },
    ];
    let mut config = ClientShellConfig::from_config(&parsed);
    config.status_bar = true;
    config.right_panel = true;
    config.sidebar_tree = true;
    let mut state = ClientShellState::new(config);
    state.sidebar_width = 40;
    state.set_snapshot(Box::new(projected));
    let size = state.layout(cols, rows).pane_surface;
    state.set_pane_surface(preview_surface(size));
    state.orcha_ui.open_projects.insert("ws_2".into());
    state.orcha_ui.process_frame = 2;
    if std::env::var("ORCHA_PREVIEW_OPEN").is_ok() {
        state.orcha_ui.history_open = true;
        state.orcha_ui.worktrees_open = true;
    }
    let window = |label: &str, left: u8, reset: &str| FuelWindow {
        key: label.to_lowercase(),
        label: label.into(),
        left,
        reset: Some(reset.into()),
    };
    state.accounts.set_for_test(Some(vec![
        Account {
            id: "work".into(),
            kind: "claude".into(),
            label: "Claude · company".into(),
            email: Some("me@example.com".into()),
            current: true,
            error: None,
            windows: vec![window("5H", 60, "1h 12m"), window("WEEK", 18, "Mon")],
            signed_in: None,
        },
        Account {
            id: "personal".into(),
            kind: "claude".into(),
            label: "Claude · personal".into(),
            email: Some("you@example.com".into()),
            current: false,
            error: None,
            windows: vec![window("5H", 70, "3h 40m"), window("WEEK", 64, "Thu")],
            signed_in: None,
        },
        Account {
            id: "codex".into(),
            kind: "codex".into(),
            label: "Codex".into(),
            email: Some("you@example.com".into()),
            current: false,
            error: None,
            windows: vec![window("5H", 90, "4h 05m"), window("WEEK", 52, "Sun")],
            signed_in: None,
        },
        Account {
            id: "cursor".into(),
            kind: "cursor".into(),
            label: "Cursor".into(),
            email: None,
            current: false,
            error: Some("not signed in".into()),
            windows: vec![],
            signed_in: None,
        },
    ]));
    if std::env::var("ORCHA_PREVIEW_POPUP").is_ok() {
        state.open_accounts_overlay();
    }
    let demo_release = || crate::orcha_app::Release {
        version: "0.1.6".into(),
        notes: vec![
            "Switching Claude accounts works in the running chat".into(),
            "Choose the app for each file type".into(),
            "The sidebar shows tabs as a tree".into(),
        ],
    };
    if std::env::var("ORCHA_PREVIEW_UPDATE").is_ok() {
        state.orcha_ui.popup = Some(crate::client::shell::orcha_popup::OrchaPopup::Update {
            selected: 0,
            release: demo_release(),
        });
    }
    if std::env::var("ORCHA_PREVIEW_WHATS_NEW").is_ok() {
        state.orcha_ui.popup = Some(crate::client::shell::orcha_popup::OrchaPopup::WhatsNew {
            release: demo_release(),
        });
    }
    if std::env::var("ORCHA_PREVIEW_OPEN_WITH").is_ok() {
        use crate::client::shell::orcha_popup::{OpenChoice, OrchaPopup};
        let editor = |name: &str| OpenChoice::Editor {
            name: name.into(),
            command: format!("open -a '{name}' {{path}}"),
        };
        state.orcha_ui.popup = Some(OrchaPopup::OpenWith {
            selected: 0,
            path: "/tmp/matcha-house/docs/dark-menu.png".into(),
            line: 1,
            extension: "png".into(),
            choices: vec![
                OpenChoice::System,
                editor("Cursor"),
                editor("Zed"),
                OpenChoice::Reveal,
            ],
            remember: true,
        });
    }
    state.git_panel.set_for_test(Some(GitPanelData {
        root: "/tmp/matcha-house".into(),
        branch: "feature/42-dark-menu".into(),
        upstream: Some("origin/feature/42-dark-menu".into()),
        ahead: 2,
        behind: 0,
        files: vec![
            ("M".into(), "src/theme.ts".into()),
            ("??".into(), "src/Menu.tsx".into()),
            ("D".into(), "old.js".into()),
        ],
        worktrees: vec![
            ("main".into(), false),
            ("feature/42-dark-menu".into(), true),
        ],
        insertions: 26,
        deletions: 4,
        last_commit: Some(("dark mode toggle".into(), "2 minutes ago".into())),
        history: demo_history(),
    }));
    let frame = state.compose(cols, rows).expect("frame");
    let cells: Vec<serde_json::Value> = frame
        .cells
        .iter()
        .map(|c| serde_json::json!([c.symbol, c.fg, c.bg, c.modifier]))
        .collect();
    let json = serde_json::json!({"width": frame.width, "height": frame.height, "cells": cells});
    std::fs::write(out, json.to_string()).expect("write");
}

#[test]
fn welcome_walks_the_setup_and_skip_puts_the_theme_back() {
    use crate::api::schema::{IntegrationInfo, IntegrationState, IntegrationTarget};
    use crate::client::shell::orcha_popup::OrchaPopup;
    use crate::client::shell::orcha_welcome::Step;
    use crate::input::TerminalKey;
    use crate::raw_input::RawInputEvent;
    use crossterm::event::{KeyCode, KeyModifiers};

    fn show(state: &mut ClientShellState) -> String {
        let frame = state.compose(140, 40).expect("frame");
        let rows = frame_rows(&frame);
        for row in &rows {
            eprintln!("|{row}|");
        }
        rows.join("\n")
    }
    fn press(state: &mut ClientShellState, code: KeyCode) {
        state.handle_raw_events(vec![RawInputEvent::Key(TerminalKey::new(
            code,
            KeyModifiers::NONE,
        ))]);
    }
    fn go(state: &mut ClientShellState, step: Step) {
        if let Some(OrchaPopup::Welcome(welcome)) = state.orcha_ui.popup.as_mut() {
            welcome.step = step;
            welcome.selected = 0;
        }
    }

    let mut state = orcha_state(true, true);
    let theme = state.config.theme_name.clone();
    let mut outcome = ClientShellInput::default();
    state.open_welcome(&mut outcome);
    let all = show(&mut state);
    assert!(all.contains("WELCOME TO ORCHA") && all.contains("step 1 of 5"));
    assert!(all.contains("START SETUP") && all.contains("skip setup"));

    // the theme step previews the theme under the cursor
    press(&mut state, KeyCode::Enter);
    let all = show(&mut state);
    assert!(all.contains("pick a look") && all.contains("matcha-dark"));
    press(&mut state, KeyCode::Down);
    assert_ne!(
        state.config.theme_name, theme,
        "the page follows the cursor"
    );

    // apps: the IDE and Git groups
    go(&mut state, Step::Apps);
    let all = show(&mut state);
    assert!(all.contains("IDE ·") && all.contains("GIT ·"));

    // agents: missing hooks start checked, installed and absent ones can't be
    let info = |target, label: &str, available, state| IntegrationInfo {
        target,
        label: label.into(),
        command: String::new(),
        available,
        state,
    };
    assert!(state.welcome_integrations_listed(vec![
        info(
            IntegrationTarget::Claude,
            "Claude",
            true,
            IntegrationState::NotInstalled
        ),
        info(
            IntegrationTarget::Codex,
            "Codex",
            true,
            IntegrationState::Current
        ),
        info(
            IntegrationTarget::Pi,
            "Pi",
            false,
            IntegrationState::NotInstalled
        ),
    ]));
    go(&mut state, Step::Agents);
    let all = show(&mut state);
    assert!(all.contains("INSTALL 1 · NEXT") && all.contains("not available on this device"));
    press(&mut state, KeyCode::Down);
    press(&mut state, KeyCode::Char(' '));
    assert!(
        show(&mut state).contains("INSTALL 1 · NEXT"),
        "installed rows don't check"
    );

    // projects: what the scan found
    assert!(
        state.welcome_sources_found(vec![crate::orcha_import::ImportSource {
            name: "VS Code".into(),
            projects: vec![("matcha-house".into(), "/tmp/matcha-house".into())],
        }])
    );
    go(&mut state, Step::Projects);
    let all = show(&mut state);
    assert!(all.contains("VS Code · 1") && all.contains("IMPORT 1 · FINISH"));
    press(&mut state, KeyCode::Char(' '));
    assert!(show(&mut state).contains("FINISH ↵"));

    // esc: the tried theme goes back, the setup closes
    press(&mut state, KeyCode::Esc);
    assert!(state.orcha_ui.popup.is_none());
    assert_eq!(state.config.theme_name, theme);
}

#[test]
fn project_card_lists_a_shell_pane() {
    let mut state = orcha_state(true, true);
    let snapshot = state.snapshot.as_mut().expect("snapshot");
    let workspace_id = snapshot.workspaces[0].workspace_id.clone();
    let tab_id = snapshot.tabs[0].tab_id.clone();
    snapshot.panes.push(ClientShellPane {
        pane_id: "pane_shell".into(),
        workspace_id,
        tab_id,
        label: None,
        cwd: Some("/tmp/matcha-house".into()),
        foreground_cwd: None,
        focused: false,
        right_click_passthrough: false,
    });
    let frame = state.compose(140, 40).expect("frame");
    let all = frame_rows(&frame).join("\n");
    assert!(
        all.contains("Shell"),
        "a plain terminal shows under the project: {all}"
    );
    assert!(
        state
            .hits
            .agents
            .iter()
            .any(|(_, pane)| pane == "pane_shell"),
        "the shell row is clickable"
    );
}

#[test]
fn open_with_popup_suggests_an_app_and_the_check_flips_remember() {
    use crate::client::shell::orcha_popup::{OpenChoice, OrchaPopup};
    let mut state = orcha_state(true, true);
    state.orcha_ui.popup = Some(OrchaPopup::OpenWith {
        selected: 0,
        path: "/tmp/shot.png".into(),
        line: 1,
        extension: "png".into(),
        choices: vec![
            OpenChoice::System,
            OpenChoice::Editor {
                name: String::new(),
                command: String::new(),
            },
            OpenChoice::Reveal,
        ],
        remember: true,
    });
    let frame = state.compose(140, 40).expect("open with");
    if let Ok(path) = std::env::var("ORCHA_POPUP_PREVIEW_OUT") {
        let cells: Vec<_> = frame
            .cells
            .iter()
            .map(|cell| serde_json::json!([cell.symbol, cell.fg, cell.bg, cell.modifier]))
            .collect();
        let json =
            serde_json::json!({"width": frame.width, "height": frame.height, "cells": cells});
        std::fs::write(path, json.to_string()).expect("preview");
    }
    let text = frame_rows(&frame).join("\n");
    assert!(text.contains("OPEN WITH"));
    assert!(text.contains("shot.png"));
    assert!(text.contains("Default app  · suggested"));
    assert!(text.contains("always open .png files this way"));
    // three OPEN buttons and the remember check
    assert_eq!(state.hits.orcha.popup.actions.len(), 4);
    let (check, index) = state.hits.orcha.popup.actions[3];
    assert_eq!(index, 3);
    let mut outcome = ClientShellInput::default();
    state.handle_orcha_popup_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: check.x,
            row: check.y,
            modifiers: KeyModifiers::empty(),
        },
        &mut outcome,
    );
    assert!(matches!(
        state.orcha_ui.popup,
        Some(OrchaPopup::OpenWith {
            remember: false,
            ..
        })
    ));
}

#[test]
fn update_popup_lists_highlights_and_later_keeps_the_button() {
    use crate::client::shell::orcha_popup::OrchaPopup;
    let mut state = orcha_state(true, true);
    state.orcha_ui.update_available = Some("0.1.6".into());
    state.orcha_ui.popup = Some(OrchaPopup::Update {
        selected: 0,
        release: crate::orcha_app::Release {
            version: "0.1.6".into(),
            notes: vec!["Faster sidebar".into(), "Open files your way".into()],
        },
    });
    let frame = state.compose(140, 40).expect("update popup");
    let text = frame_rows(&frame).join("\n");
    assert!(text.contains("orcha 0.1.6 is out"));
    assert!(text.contains("• Faster sidebar"));
    assert!(text.contains("Skip 0.1.6"));
    assert_eq!(state.hits.orcha.popup.actions.len(), 3);
    let (later, index) = state.hits.orcha.popup.actions[1];
    assert_eq!(index, 1);
    let mut outcome = ClientShellInput::default();
    state.handle_orcha_popup_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: later.x,
            row: later.y,
            modifiers: KeyModifiers::empty(),
        },
        &mut outcome,
    );
    assert!(state.orcha_ui.popup.is_none(), "later closes the popup");
    assert_eq!(
        state.orcha_ui.update_available.as_deref(),
        Some("0.1.6"),
        "the header button stays"
    );
}

#[test]
fn whats_new_shows_once_and_got_it_closes() {
    use crate::client::shell::orcha_popup::OrchaPopup;
    let mut state = orcha_state(true, true);
    state.orcha_ui.popup = Some(OrchaPopup::WhatsNew {
        release: crate::orcha_app::Release {
            version: "0.1.6".into(),
            notes: vec!["Open files your way".into()],
        },
    });
    let frame = state.compose(140, 40).expect("what's new");
    let text = frame_rows(&frame).join("\n");
    assert!(text.contains("WHAT'S NEW"));
    assert!(text.contains("• Open files your way"));
    let (got_it, _) = state.hits.orcha.popup.actions[0];
    let mut outcome = ClientShellInput::default();
    state.handle_orcha_popup_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: got_it.x,
            row: got_it.y,
            modifiers: KeyModifiers::empty(),
        },
        &mut outcome,
    );
    assert!(state.orcha_ui.popup.is_none());
}
