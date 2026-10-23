//! orcha: the five regions (header, left sidebar, main, right git panel,
//! bottom status bar). Run with `--nocapture` to see the composed screen.
use super::*;
use crate::client::shell::git_panel::GitPanelData;
use crate::protocol::ClientShellTabStatusSegment;

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
    // a margin of two columns and a row around the page
    assert_eq!(layout.status_bar, Rect::new(2, 26, 136, 3));
    assert_eq!(layout.header, Rect::new(2, 1, 136, 3));
    assert_eq!(layout.sidebar.y, 4, "right under the header (box edges are the gap)");
    assert_eq!(layout.right_panel.width, 38);
    assert_eq!(layout.main_frame.y, 6, "two rows of tabs over the frame");
    assert_eq!(layout.tab_bar.y + 1, layout.main_frame.y, "tab labels sit on the frame");
    assert!(layout.pane_surface.x > layout.main_frame.x);
    assert!(layout.main_frame.right() < layout.right_panel.x);
    assert!(rows[2].contains("ORCHA") && rows[2].contains("settings"));
    let last = &rows[27];
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
    assert!(all.contains("1 RUNNING"), "only running agents are trains");
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
fn accounts_popup_lists_fuel() {
    use crate::client::shell::accounts::{Account, FuelWindow};
    let mut state = orcha_state(true, true);
    let window = |label: &str, left: u8| FuelWindow {
        key: label.to_lowercase(),
        label: label.into(),
        left,
        reset: None,
    };
    state.accounts.set_for_test(Some(vec![
        Account {
            id: "work".into(),
            kind: "claude".into(),
            label: "Claude · company".into(),
            email: Some("me@example.com".into()),
            current: true,
            error: None,
            windows: vec![window("5H", 8), window("WEEK", 18)],
        },
        Account {
            id: "personal".into(),
            kind: "claude".into(),
            label: "Claude · personal".into(),
            email: Some("you@example.com".into()),
            current: false,
            error: None,
            windows: vec![window("5H", 70), window("WEEK", 64)],
        },
    ]));
    state.open_accounts_overlay();
    let frame = state.compose(140, 40).expect("frame");
    let all = frame_rows(&frame).join("\n");
    for row in frame_rows(&frame) {
        eprintln!("|{row}|");
    }
    assert!(all.contains("ACCOUNTS") && all.contains("IN USE") && all.contains("MOST FUEL"));
    assert!(all.contains("switch this chat"));
    assert_eq!(state.hits.orcha.popup.actions.len(), 1, "only the other account switches");
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
                "› add a dark mode toggle to src/Menu.tsx",
                "",
                "● I'll read the theme tokens first.",
                "  └ Read src/theme.ts (3 lines)",
                "",
                "● Update(src/Menu.tsx)",
                "   + const [dark, setDark] = useState(theme === \"dark\")",
                "   - return <Toggle label=\"Dark mode\" />",
                "",
                "● Bash(npm test -- Menu)",
                "✻ Running tests… 8s",
            ],
        ),
        (
            "pane_2",
            Rect::new(half + 1, 0, size.width.saturating_sub(half + 1), size.height),
            vec![
                "› Explain src/theme.ts in two sentences.",
                "",
                "src/theme.ts exports three theme settings:",
                "a dark theme, a pink accent color and a",
                "corner radius of 12.",
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
                buffer.set_stringn(rect.x + 2, y, line, usize::from(rect.width - 3), Style::default());
            }
        }
        let inner = Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, rect.height - 2);
        surface_panes.push(PaneSurfacePane {
            pane_id: (*id).into(),
            content_revision: 0,
            rect: SurfaceRect { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
            inner_rect: SurfaceRect { x: inner.x, y: inner.y, width: inner.width, height: inner.height },
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
    let theme = std::env::var("ORCHA_PREVIEW_THEME").unwrap_or_else(|_| "brutal".into());
    let parsed: Config =
        toml::from_str(&format!("[theme]\nname = \"{theme}\"\n")).expect("toml");
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
    second_tab.focused = false;
    second_tab.agent_status = AgentStatus::Done;
    let mut third_tab = second_tab.clone();
    third_tab.tab_id = "tab_3".into();
    third_tab.label = "amex fix".into();
    third_tab.agent_status = AgentStatus::Blocked;
    projected.tabs[0].label = "dark mode".into();
    projected.tabs[0].agent_status = AgentStatus::Working;
    projected.tabs.push(second_tab);
    projected.tabs.push(third_tab);
    let mk = |pane: &str, ws: &str, kind: &str, model: &str, task: &str, stage: &str,
              status: AgentStatus, focused: bool| ClientShellAgent {
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
        mk("pane_1", &ws1, "claude", "claude-opus-5-5", "dark mode", "testing", AgentStatus::Working, true),
        mk("pane_2", &ws1, "codex", "gpt-6-luna", "explain theme", "done", AgentStatus::Done, false),
        mk("pane_3", "ws_2", "codex", "gpt-6-luna", "apple pay", "coding", AgentStatus::Working, false),
        mk("pane_4", "ws_2", "cursor", "cursor", "amex", "coding", AgentStatus::Blocked, false),
        mk("pane_5", "ws_3", "claude", "claude-opus-5-5", "search", "done", AgentStatus::Done, false),
        mk("pane_6", "ws_5", "claude", "claude-sonnet-5-5", "flags", "thinking", AgentStatus::Working, false),
    ];
    projected.tab_bar_right = vec![
        ClientShellTabStatusSegment { text: "#42 Dark mode for the menu ● Open".into(), accent: false },
        ClientShellTabStatusSegment { text: "✎ 3 files +26 −4".into(), accent: false },
        ClientShellTabStatusSegment { text: "◷ 18m".into(), accent: false },
        ClientShellTabStatusSegment { text: "17:50".into(), accent: false },
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
        Account { id: "work".into(), kind: "claude".into(), label: "Claude · company".into(),
            email: Some("me@example.com".into()), current: true, error: None,
            windows: vec![window("5H", 60, "1h 12m"), window("WEEK", 18, "Mon")] },
        Account { id: "personal".into(), kind: "claude".into(), label: "Claude · personal".into(),
            email: Some("you@example.com".into()), current: false, error: None,
            windows: vec![window("5H", 70, "3h 40m"), window("WEEK", 64, "Thu")] },
        Account { id: "codex".into(), kind: "codex".into(), label: "Codex".into(),
            email: Some("you@example.com".into()), current: false, error: None,
            windows: vec![window("5H", 90, "4h 05m"), window("WEEK", 52, "Sun")] },
        Account { id: "cursor".into(), kind: "cursor".into(), label: "Cursor".into(),
            email: None, current: false, error: Some("not signed in".into()), windows: vec![] },
    ]));
    if std::env::var("ORCHA_PREVIEW_POPUP").is_ok() {
        state.open_accounts_overlay();
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
