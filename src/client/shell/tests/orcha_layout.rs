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
    }));
    state
}

#[test]
fn five_regions_compose() {
    let mut state = orcha_state(true, true);
    let frame = state.compose(140, 22).expect("frame");
    let rows = frame_rows(&frame);
    for row in &rows {
        eprintln!("|{row}|");
    }
    let layout = state.layout(140, 22);
    assert_eq!(layout.status_bar, Rect::new(0, 21, 140, 1));
    assert_eq!(layout.tab_bar.y, 0, "one header row across the top");
    assert_eq!(layout.sidebar.y, 1);
    assert_eq!(layout.right_panel.width, 34);
    assert_eq!(layout.pane_surface.right(), layout.right_panel.x);
    let last = rows.last().expect("status row");
    assert!(
        last.contains("#42") && last.contains("Dark mode for the menu"),
        "status bar holds the right entries"
    );
    assert!(
        !rows[0].contains("17:50"),
        "the tab row no longer holds them"
    );
    let panel: String = rows.join("\n");
    assert!(panel.contains("feature/42-dark-menu"));
    assert!(panel.contains("↑2 to push"));
    assert!(panel.contains("…ings/Menu.tsx") || panel.contains("Menu.tsx"));
    assert!(panel.contains("DEPOTS") && panel.contains("TRACKS") && panel.contains("CHANGES"));
    assert!(
        rows[0].contains("orcha") && rows[1].contains("LINES"),
        "one tree of lines, no spaces/agents split"
    );
    assert!(!panel.contains(" agents "));
    assert!(panel.contains("Claude") && panel.contains("Codex"));
    assert!(panel.contains("+ New train") && panel.contains("Menu"));
    assert!(
        state.hits.agents.iter().any(|(_, pane)| pane == "pane_2"),
        "agent rows are clickable"
    );
}

#[test]
fn regions_off_is_plain_herdr() {
    let mut state = orcha_state(false, false);
    let frame = state.compose(140, 22).expect("frame");
    let layout = state.layout(140, 22);
    assert_eq!(layout.status_bar.height, 0);
    assert_eq!(layout.right_panel.width, 0);
    assert!(frame_rows(&frame)[0].contains("17:50"));
}

#[test]
fn narrow_screens_drop_the_panel() {
    let state = orcha_state(true, true);
    assert_eq!(state.layout(100, 22).right_panel.width, 0);
}

/// Writes a composed orcha frame as JSON cells for a PNG preview:
/// `ORCHA_PREVIEW_CONFIG=~/.config/herdr/config.toml ORCHA_PREVIEW_OUT=/tmp/f.json
///  cargo test --bin herdr orcha_preview -- --ignored`
#[test]
#[ignore]
fn orcha_preview() {
    let (Ok(config_path), Ok(out)) = (
        std::env::var("ORCHA_PREVIEW_CONFIG"),
        std::env::var("ORCHA_PREVIEW_OUT"),
    ) else {
        return;
    };
    let text = std::fs::read_to_string(config_path).expect("config");
    let parsed: Config = toml::from_str(&text).expect("toml");
    let (cols, rows) = (180u16, 46u16);
    let mut projected = snapshot();
    let mut second = projected.workspaces[0].clone();
    projected.workspaces[0].label = "matcha-house".into();
    projected.workspaces[0].branch = Some("feature/42-dark-menu".into());
    second.workspace_id = "ws_2".into();
    second.label = "orca-pay".into();
    second.branch = Some("feature/108-apple-pay".into());
    second.focused = false;
    projected.workspaces.push(second);
    let ws1 = projected.workspaces[0].workspace_id.clone();
    let tab = projected.tabs[0].tab_id.clone();
    let mk = |pane: &str,
              ws: &str,
              kind: &str,
              display: &str,
              task: &str,
              stage: &str,
              status: AgentStatus,
              focused: bool| {
        ClientShellAgent {
            pane_id: pane.into(),
            workspace_id: ws.into(),
            tab_id: tab.clone(),
            name: None,
            display_agent: Some(display.into()),
            agent: Some(kind.into()),
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: status,
            state_change_seq: 1,
            state_labels: Vec::new(),
            tokens: vec![("task".into(), task.into()), ("stage".into(), stage.into())],
            focused,
        }
    };
    projected.agents = vec![
        mk(
            "pane_1",
            &ws1,
            "claude",
            "Claude",
            "Menu.tsx dark mode",
            "testing",
            AgentStatus::Working,
            true,
        ),
        mk(
            "pane_2",
            &ws1,
            "codex",
            "Codex",
            "Explain theme",
            "done",
            AgentStatus::Done,
            false,
        ),
        mk(
            "pane_3",
            "ws_2",
            "claude",
            "Claude",
            "Apple Pay amex",
            "coding",
            AgentStatus::Blocked,
            false,
        ),
    ];
    projected.tab_bar_right = vec![
        ClientShellTabStatusSegment {
            text: "󰄬 #42 Dark mode for the menu ● Open".into(),
            accent: false,
        },
        ClientShellTabStatusSegment {
            text: "✎ 3 files +26/−4".into(),
            accent: false,
        },
        ClientShellTabStatusSegment {
            text: "✳ kien@… · 5h ██░░░ 40%".into(),
            accent: false,
        },
        ClientShellTabStatusSegment {
            text: "17:50".into(),
            accent: false,
        },
    ];
    projected.tab_bar_right_separator = "  ·  ".into();
    let mut config = ClientShellConfig::from_config(&parsed);
    config.status_bar = true;
    config.right_panel = true;
    config.sidebar_tree = true;
    let mut state = ClientShellState::new(config);
    state.sidebar_width = 40;
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
            ("??".into(), "src/Menu.tsx".into()),
            ("D".into(), "old.js".into()),
        ],
        worktrees: vec![
            ("main".into(), false),
            ("feature/42-dark-menu".into(), true),
        ],
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
