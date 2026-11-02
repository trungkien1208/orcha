use super::*;

const PROJECTS: [&str; 16] = [
    "ORCHA",
    "TAGGLE-SUPERAPP-V2",
    "THKCM RPM",
    "WABI",
    "NCA-PSSB",
    "LARK AUTOMATION",
    "HIENTRANGVISA-MARKETING",
    "CANVA",
    "SEARCH JOBS",
    "COGIGNON",
    "STROKE360",
    "MYPORTFOLIO",
    "GAME-IOS",
    "LARK_AUTOMATION",
    "USE AI JOB SEARCH",
    "DOLPHIN",
];

fn sidebar_state(count: usize, one_workspace: bool) -> ClientShellState {
    let mut data = snapshot();
    let workspace = data.workspaces[0].clone();
    let tab = data.tabs[0].clone();
    let pane = data.panes[0].clone();
    data.workspaces.clear();
    data.tabs.clear();
    data.panes.clear();
    data.agents.clear();
    for index in 0..count {
        let number = index + 1;
        let workspace_id = if one_workspace {
            "ws_1".to_owned()
        } else {
            format!("ws_{number}")
        };
        let tab_id = format!("tab_{number}");
        let pane_id = format!("pane_{number}");
        if !one_workspace || index == 0 {
            let mut next = workspace.clone();
            next.workspace_id = workspace_id.clone();
            next.active_tab_id = tab_id.clone();
            next.number = number;
            next.label = PROJECTS[index % PROJECTS.len()].into();
            next.branch = Some(
                if index == 0 {
                    "orcha"
                } else {
                    "feature/a-long-branch-name"
                }
                .into(),
            );
            next.focused = index == 0;
            data.workspaces.push(next);
        }
        let mut next_tab = tab.clone();
        next_tab.workspace_id = workspace_id.clone();
        next_tab.tab_id = tab_id.clone();
        next_tab.focused = index == 0;
        data.tabs.push(next_tab);
        let mut next_pane = pane.clone();
        next_pane.workspace_id = workspace_id;
        next_pane.tab_id = tab_id;
        next_pane.pane_id = pane_id;
        next_pane.cwd = Some(format!("/projects/{}", PROJECTS[index % PROJECTS.len()]));
        next_pane.foreground_cwd = None;
        next_pane.focused = index == 0;
        data.panes.push(next_pane);
    }
    let config: Config = toml::from_str("[theme]\nname = 'orcha'\n").expect("theme");
    let mut config = ClientShellConfig::from_config(&config);
    config.sidebar_tree = true;
    config.right_panel = false;
    config.status_bar = false;
    let mut state = ClientShellState::new(config);
    state.sidebar_width = 34;
    state.set_snapshot(Box::new(data));
    state.set_pane_surface(surface());
    state
}

fn add_agent(state: &mut ClientShellState, pane_index: usize, kind: &str, status: AgentStatus) {
    let data = state.snapshot.as_mut().expect("snapshot");
    let pane = &data.panes[pane_index];
    data.agents.push(ClientShellAgent {
        pane_id: pane.pane_id.clone(),
        workspace_id: pane.workspace_id.clone(),
        tab_id: pane.tab_id.clone(),
        name: None,
        display_agent: Some(kind.into()),
        agent: Some(kind.to_lowercase()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: status,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: pane.focused,
    });
}

fn click_toggle(state: &mut ClientShellState) -> ClientShellInput {
    let (rect, _) = state
        .hits
        .orcha
        .project_toggles
        .iter()
        .find(|(_, id)| id == "ws_1")
        .expect("selected project's toggle");
    let mut outcome = ClientShellInput::default();
    assert!(state.handle_orcha_click((rect.x, rect.y), &mut outcome));
    outcome
}

#[test]
fn idle_projects_fit_without_boxes_or_repeated_shell_paths() {
    let mut state = sidebar_state(16, false);
    let frame = state.compose(120, 42).expect("frame");
    assert_eq!(state.hits.workspaces.len(), 16);
    assert!(
        state.hits.orcha.process.is_empty(),
        "idle footer has no animation target"
    );
    let text = frame_rows(&frame).join("\n");
    assert!(text.contains("HIENTRANGVISA-MARKETING"));
    assert!(text.contains("No agents running"));
    let body = state.hits.workspace_body;
    let sidebar: String = frame
        .cells
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            let x = (*index % frame.width as usize) as u16;
            let y = (*index / frame.width as usize) as u16;
            super::super::contains(body, (x, y))
        })
        .map(|(_, cell)| cell.symbol.as_str())
        .collect();
    assert!(!sidebar.contains('□') && !sidebar.contains('■'));
    assert!(!sidebar.contains("Shell ORCHA"));
    let header = state.hits.workspaces[0].rect.y as usize;
    // selected project: header, branch, one pane; no blank rows between projects
    assert_eq!(state.hits.workspaces[15].rect.y as usize, header + 17);
    let rows = frame_rows(&frame);
    assert!(
        rows[header + 3].contains("TAGGLE-SUPERAPP-V2"),
        "the next project follows right away"
    );
    assert!(
        !rows[header + 3].contains("idle"),
        "idle projects say nothing: {}",
        rows[header + 3]
    );
}

#[test]
fn collapsed_project_shows_active_agent_logos_and_selected_project_can_fold() {
    let mut state = sidebar_state(3, true);
    add_agent(&mut state, 0, "Codex", AgentStatus::Working);
    add_agent(&mut state, 1, "Cursor", AgentStatus::Idle);
    add_agent(&mut state, 2, "Claude", AgentStatus::Blocked);
    let frame = state.compose(120, 42).expect("expanded frame");
    let palette = super::super::brutal::Brutal::new(&state.config.palette);
    let body = state.hits.workspace_body;
    let idle_icon = frame
        .cells
        .iter()
        .enumerate()
        .find(|(index, cell)| {
            cell.symbol == "▲" && (*index / frame.width as usize) < body.bottom() as usize
        })
        .expect("idle Cursor logo")
        .1;
    assert_eq!(idle_icon.fg, crate::protocol::color_to_u32(palette.muted));
    let outcome = click_toggle(&mut state);
    assert!(
        outcome.actions.is_empty(),
        "folding is client presentation only"
    );
    let frame = state.compose(120, 42).expect("collapsed frame");
    let y = state.hits.workspaces[0].rect.y;
    let line = &frame_rows(&frame)[y as usize];
    assert!(
        line.contains("needs you") && line.contains('◎') && line.contains('✳'),
        "{line}"
    );
    // most urgent first; a narrow row drops the quietest logo, never a busy one
    let blocked = line.find('✳').expect("needs-you logo");
    let running = line.find('◎').expect("running logo");
    assert!(blocked < running, "needs you leads: {line}");
    assert!(
        line.find('▲').is_none_or(|idle| running < idle),
        "idle comes last: {line}"
    );
    let cells =
        &frame.cells[y as usize * frame.width as usize..(y as usize + 1) * frame.width as usize];
    assert_eq!(
        cells
            .iter()
            .find(|cell| cell.symbol == "◎")
            .expect("Codex")
            .fg,
        crate::protocol::color_to_u32(palette.mark(palette.yellow, palette.card)),
        "a running agent is yellow, whatever its brand"
    );
    assert_eq!(
        cells
            .iter()
            .find(|cell| cell.symbol == "✳")
            .expect("Claude")
            .fg,
        crate::protocol::color_to_u32(palette.mark(palette.red, palette.card))
    );
    assert!(state.snapshot.as_ref().expect("snapshot").workspaces[0].focused);
    state.snapshot.as_mut().expect("snapshot").agents[2].agent_status = AgentStatus::Idle;
    let running_frame = state.compose(120, 42).expect("working state changed");
    let running_line = &frame_rows(&running_frame)[y as usize];
    assert!(running_line.contains("running") && running_line.contains('◎'));
    assert!(
        running_line.find('◎') < running_line.find('✳'),
        "the running agent leads, idle ones follow: {running_line}"
    );
    click_toggle(&mut state);
    state.compose(120, 42).expect("expanded again");
    assert!(state
        .hits
        .agents
        .iter()
        .any(|(rect, id)| id == "pane_2" && rect.y < body.bottom()));
}

#[test]
fn branch_does_not_compete_with_project_name_and_shell_row_keeps_its_target() {
    let mut state = sidebar_state(1, false);
    state.snapshot.as_mut().expect("snapshot").workspaces[0].label = "TAGGLE-SUPERAPP-V2".into();
    state.snapshot.as_mut().expect("snapshot").workspaces[0].branch =
        Some("feature/a-long-branch-name".into());
    let frame = state.compose(120, 42).expect("frame");
    let y = state.hits.workspaces[0].rect.y as usize;
    let rows = frame_rows(&frame);
    assert!(rows[y].contains("TAGGLE-SUPERAPP-V2"));
    assert!(!rows[y].contains("feature/"));
    assert!(rows[y + 1].contains("⎇ feature/"));
    let (rect, id) = state
        .hits
        .agents
        .iter()
        .find(|(_, id)| id == "pane_1")
        .expect("shell hit");
    assert_eq!(id, "pane_1");
    assert_eq!(rect.y as usize, y + 2);
}

#[test]
fn activity_footer_starts_open_and_stops_rendering_animation_when_folded() {
    let mut state = sidebar_state(2, true);
    add_agent(&mut state, 0, "Codex", AgentStatus::Working);
    add_agent(&mut state, 1, "Cursor", AgentStatus::Blocked);
    let frame = state.compose(120, 42).expect("open activity");
    assert!(frame_rows(&frame).join("\n").contains("PROCESS"));
    assert!(!state.hits.orcha.process.is_empty());
    let toggle = state.hits.orcha.process_toggle;
    let mut outcome = ClientShellInput::default();
    assert!(state.handle_orcha_click((toggle.x, toggle.y), &mut outcome));
    let frame = state.compose(120, 42).expect("folded activity");
    assert!(frame_rows(&frame)
        .join("\n")
        .contains("1 running · 1 needs you"));
    assert!(state.hits.orcha.process.is_empty());
    let toggle = state.hits.orcha.process_toggle;
    assert!(state.handle_orcha_click((toggle.x, toggle.y), &mut outcome));
    state.compose(120, 42).expect("reopened activity");
    assert!(!state.hits.orcha.process.is_empty());
    assert!(
        outcome.actions.is_empty(),
        "activity layout never changes the runtime"
    );
}

#[test]
#[ignore = "manual fixed-geometry sidebar profile"]
fn orcha_sidebar_render_scale_profile() {
    let with_agents = std::env::var("ORCHA_SIDEBAR_PROFILE_AGENTS").is_ok();
    for one_workspace in [false, true] {
        for count in [1, 15] {
            let mut state = sidebar_state(count, one_workspace);
            if with_agents {
                for index in 0..count {
                    add_agent(
                        &mut state,
                        index,
                        ["Claude", "Codex", "Cursor"][index % 3],
                        AgentStatus::Working,
                    );
                }
            }
            // Exercise every populated project when the viewport can show it.
            for workspace in &state.snapshot.as_ref().expect("snapshot").workspaces {
                state
                    .orcha_ui
                    .open_projects
                    .insert(workspace.workspace_id.clone());
            }
            let mut samples = Vec::new();
            for index in 0..300 {
                let start = std::time::Instant::now();
                std::hint::black_box(state.compose(120, 42).expect("frame"));
                if index >= 50 {
                    samples.push(start.elapsed().as_nanos());
                }
            }
            samples.sort_unstable();
            eprintln!(
                "orcha_sidebar agents={with_agents} layout={} populated={} geometry=120x42 median_ns={} p95_ns={}",
                if one_workspace {
                    "active_panes"
                } else {
                    "workspaces"
                },
                count,
                samples[samples.len() / 2],
                samples[samples.len() * 95 / 100]
            );
        }
    }
}

#[test]
#[ignore = "exports the real sidebar renderer for visual review"]
fn orcha_sidebar_preview() {
    let Ok(path) = std::env::var("ORCHA_SIDEBAR_PREVIEW_OUT") else {
        return;
    };
    let mut state = sidebar_state(16, false);
    let dark: Config = toml::from_str("[theme]\nname = 'orcha-dark'\n").expect("dark theme");
    state.config.palette = ClientShellConfig::from_config(&dark).palette;
    state.config.theme_name = "orcha-dark".into();
    if std::env::var("ORCHA_SIDEBAR_PREVIEW_ACTIVE").is_ok() {
        let mut extra = state.snapshot.as_ref().expect("snapshot").panes[0].clone();
        extra.pane_id = "pane_extra".into();
        extra.tab_id = "tab_extra".into();
        extra.focused = false;
        let mut tab = state.snapshot.as_ref().expect("snapshot").tabs[0].clone();
        tab.tab_id = "tab_extra".into();
        tab.number = 2;
        tab.focused = false;
        let data = state.snapshot.as_mut().expect("snapshot");
        data.tabs.push(tab);
        data.panes.push(extra);
        add_agent(&mut state, 0, "Cursor", AgentStatus::Working);
        add_agent(&mut state, 16, "Codex", AgentStatus::Idle);
        add_agent(&mut state, 1, "Codex", AgentStatus::Working);
        add_agent(&mut state, 4, "Claude", AgentStatus::Blocked);
        add_agent(&mut state, 9, "Cursor", AgentStatus::Idle);
    }
    state.config.icon_font = std::env::var("ORCHA_SIDEBAR_PREVIEW_LOGOS").is_ok();
    let frame = state.compose(120, 42).expect("frame");
    let cells: Vec<_> = frame
        .cells
        .iter()
        .map(|cell| serde_json::json!([cell.symbol, cell.fg, cell.bg, cell.modifier]))
        .collect();
    std::fs::write(
        path,
        serde_json::json!({"width":frame.width,"height":frame.height,"cells":cells}).to_string(),
    )
    .expect("preview");
}

#[test]
fn project_panes_show_as_a_tree_numbered_by_tab() {
    let mut state = sidebar_state(1, false);
    {
        let data = state.snapshot.as_mut().expect("snapshot");
        let mut tab = data.tabs[0].clone();
        tab.tab_id = "tab_2".into();
        tab.number = 2;
        tab.focused = false;
        data.tabs.push(tab);
        let mut pane = data.panes[0].clone();
        pane.pane_id = "pane_2".into();
        pane.tab_id = "tab_2".into();
        pane.focused = false;
        data.panes.push(pane);
    }
    add_agent(&mut state, 0, "Claude", AgentStatus::Working);
    let frame = state.compose(120, 42).expect("frame");
    let rows = frame_rows(&frame);
    let first = rows
        .iter()
        .position(|row| row.contains("1 ▌✳"))
        .expect("the first tab's number sits before its agent, the open pane's bar between");
    assert!(
        rows[first + 1].contains("2  ›_ Shell"),
        "the second tab follows: {}",
        rows[first + 1]
    );
}

#[test]
fn single_tab_project_hides_tab_numbers() {
    let mut state = sidebar_state(1, false);
    let frame = state.compose(120, 42).expect("frame");
    let rows = frame_rows(&frame);
    let row = rows
        .iter()
        .find(|row| row.contains("›_"))
        .expect("pane row");
    assert!(!row.contains("1  ›_"), "one tab needs no number: {row}");
}

/// A busy real-world sidebar: many projects, some open, icons, idle and
/// working agents, panes spread over several tabs.
fn busy_sidebar_state() -> ClientShellState {
    // (label, icon, open, panes as (tab, agent kind or "" for a shell, status, task or shell name))
    type Pane = (usize, &'static str, AgentStatus, &'static str);
    let projects: [(&str, &str, bool, &[Pane]); 16] = [
        ("~Hibari", "🏠", false, &[(1, "", AgentStatus::Idle, "")]),
        (
            "taggle-superapp-v2",
            "📱",
            true,
            &[
                (
                    1,
                    "Claude",
                    AgentStatus::Idle,
                    "Nhánh set-b-account đã xong",
                ),
                (3, "", AgentStatus::Idle, ""),
                (4, "", AgentStatus::Idle, ""),
            ],
        ),
        ("THKCM RPM", "🐋", true, &[(1, "", AgentStatus::Idle, "")]),
        (
            "hientrangvisa-marketing",
            "🌐",
            false,
            &[(1, "", AgentStatus::Idle, "")],
        ),
        ("Canva", "🎨", false, &[(1, "", AgentStatus::Idle, "")]),
        (
            "Search Jobs",
            "🧭",
            false,
            &[(1, "Claude", AgentStatus::Idle, "")],
        ),
        ("COGINGON", "⭐", false, &[(1, "", AgentStatus::Idle, "")]),
        ("stroke360", "🏥", false, &[(1, "", AgentStatus::Idle, "")]),
        ("game-ios", "🎮", false, &[(1, "", AgentStatus::Idle, "")]),
        (
            "orcha",
            "🔥",
            false,
            &[(1, "Claude", AgentStatus::Idle, "Herdr, Orca và Ghostty")],
        ),
        (
            "LARK_AUTOMATION",
            "",
            true,
            &[(1, "Claude", AgentStatus::Working, "Tôi cần sửa luồng duyệt")],
        ),
        (
            "nca-pssb",
            "🧾",
            true,
            &[
                (
                    1,
                    "Claude",
                    AgentStatus::Blocked,
                    "Popup inactivity timeout",
                ),
                (
                    5,
                    "Codex",
                    AgentStatus::Working,
                    "Feedback màn hình thanh toán",
                ),
            ],
        ),
        ("WABI", "🌱", false, &[(1, "", AgentStatus::Idle, "")]),
        (
            "Party Agent",
            "🚀",
            false,
            &[(1, "Claude", AgentStatus::Done, "")],
        ),
        (
            "Easy Job",
            "⚡",
            false,
            &[(1, "Claude", AgentStatus::Idle, "")],
        ),
        (
            "Easy Job bridge",
            "",
            true,
            &[
                (1, "", AgentStatus::Idle, ""),
                (2, "", AgentStatus::Idle, "Easy Job"),
            ],
        ),
    ];
    let mut state = sidebar_state(1, false);
    let data = state.snapshot.as_mut().expect("snapshot");
    let workspace = data.workspaces[0].clone();
    let tab = data.tabs[0].clone();
    let pane = data.panes[0].clone();
    data.workspaces.clear();
    data.tabs.clear();
    data.panes.clear();
    data.agents.clear();
    let mut icons = Vec::new();
    let mut open = Vec::new();
    for (index, (label, icon, is_open, panes)) in projects.iter().enumerate() {
        let workspace_id = format!("ws_{index}");
        let focused = *label == "orcha";
        let mut next = workspace.clone();
        next.workspace_id = workspace_id.clone();
        next.new_workspace_cwd = format!("/projects/{label}");
        next.number = index + 1;
        next.label = (*label).into();
        next.branch = Some("orcha".into());
        next.focused = focused;
        next.active_tab_id = format!("{workspace_id}_tab_1");
        if !icon.is_empty() {
            icons.push((next.new_workspace_cwd.clone(), (*icon).to_owned()));
        }
        if *is_open {
            open.push(workspace_id.clone());
        }
        data.workspaces.push(next);
        let mut seen_tabs = Vec::new();
        for (pane_index, (tab_number, kind, status, text)) in panes.iter().enumerate() {
            let tab_id = format!("{workspace_id}_tab_{tab_number}");
            if !seen_tabs.contains(tab_number) {
                seen_tabs.push(*tab_number);
                let mut next_tab = tab.clone();
                next_tab.workspace_id = workspace_id.clone();
                next_tab.tab_id = tab_id.clone();
                next_tab.number = *tab_number;
                next_tab.focused = focused && *tab_number == 1;
                data.tabs.push(next_tab);
            }
            let pane_id = format!("{workspace_id}_pane_{pane_index}");
            let mut next_pane = pane.clone();
            next_pane.workspace_id = workspace_id.clone();
            next_pane.tab_id = tab_id.clone();
            next_pane.pane_id = pane_id.clone();
            next_pane.focused = focused && pane_index == 0;
            next_pane.label = (kind.is_empty() && !text.is_empty()).then(|| (*text).to_owned());
            data.panes.push(next_pane);
            if !kind.is_empty() {
                data.agents.push(ClientShellAgent {
                    pane_id,
                    workspace_id: workspace_id.clone(),
                    tab_id,
                    name: None,
                    display_agent: Some((*kind).into()),
                    agent: Some(kind.to_lowercase()),
                    title: None,
                    terminal_title: None,
                    terminal_title_stripped: None,
                    agent_status: *status,
                    state_change_seq: 1,
                    state_labels: Vec::new(),
                    tokens: [
                        (
                            "model",
                            if *kind == "Claude" {
                                "claude-opus-5-5"
                            } else {
                                ""
                            },
                        ),
                        ("task", *text),
                    ]
                    .into_iter()
                    .filter(|(_, value)| !value.is_empty())
                    .map(|(key, value)| (key.to_owned(), value.to_owned()))
                    .collect(),
                    focused: focused && pane_index == 0,
                });
            }
        }
    }
    state.orcha_ui.project_icons.extend(icons);
    state.orcha_ui.open_projects.extend(open);
    state
}

#[test]
#[ignore = "exports a busy sidebar for visual review"]
fn orcha_sidebar_busy_preview() {
    let Ok(path) = std::env::var("ORCHA_SIDEBAR_PREVIEW_OUT") else {
        return;
    };
    let mut state = busy_sidebar_state();
    let theme =
        std::env::var("ORCHA_SIDEBAR_PREVIEW_THEME").unwrap_or_else(|_| "orcha-dark".into());
    let config: Config = toml::from_str(&format!("[theme]\nname = '{theme}'\n")).expect("theme");
    state.config.palette = ClientShellConfig::from_config(&config).palette;
    state.config.theme_name = theme;
    let frame = state.compose(120, 46).expect("frame");
    let cells: Vec<_> = frame
        .cells
        .iter()
        .map(|cell| serde_json::json!([cell.symbol, cell.fg, cell.bg, cell.modifier]))
        .collect();
    std::fs::write(
        path,
        serde_json::json!({"width":frame.width,"height":frame.height,"cells":cells}).to_string(),
    )
    .expect("preview");
}

#[test]
fn projects_reorder_by_dragging_even_with_new_at_the_top() {
    let mut state = sidebar_state(4, false);
    state.compose(120, 42).expect("frame");
    let first = state.hits.workspaces[0].rect;
    let last = state.hits.workspaces[3].rect;
    assert!(
        state.hits.new_workspace.y < first.y,
        "orcha draws NEW above the list"
    );
    let mouse = |kind, column, row| {
        RawInputEvent::Mouse(crossterm::event::MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::empty(),
        })
    };
    state.handle_raw_events(vec![mouse(
        MouseEventKind::Down(MouseButton::Left),
        first.x + 2,
        first.y,
    )]);
    let drag = state.handle_raw_events(vec![mouse(
        MouseEventKind::Drag(MouseButton::Left),
        last.x + 2,
        last.y,
    )]);
    assert!(drag.repaint);
    assert!(
        matches!(
            state.chrome_drag,
            Some(ClientChromeDrag::Workspace {
                target: Some(_),
                ..
            })
        ),
        "a drag over the list has a drop target"
    );
    let release = state.handle_raw_events(vec![mouse(
        MouseEventKind::Up(MouseButton::Left),
        last.x + 2,
        last.y,
    )]);
    assert!(
        release.actions.iter().any(|action| matches!(
            action,
            ClientShellAction::Endpoint { request, .. }
                if matches!(
                    &request.method,
                    crate::api::schema::Method::WorkspaceMove(params) if params.workspace_id == "ws_1"
                )
        )),
        "dropping moves the project: {:?}",
        release.actions
    );
}

#[test]
fn every_agent_row_names_its_state_and_the_open_pane_is_marked() {
    let mut state = sidebar_state(4, true);
    add_agent(&mut state, 0, "Claude", AgentStatus::Working);
    add_agent(&mut state, 1, "Codex", AgentStatus::Done);
    add_agent(&mut state, 2, "Cursor", AgentStatus::Idle);
    add_agent(&mut state, 3, "Claude", AgentStatus::Blocked);
    let frame = state.compose(120, 42).expect("frame");
    let rows = frame_rows(&frame);
    let row_of = |pane: &str| {
        let (rect, _) = state
            .hits
            .agents
            .iter()
            .find(|(rect, id)| id == pane && rect.y < state.hits.workspace_body.bottom())
            .expect("agent row");
        rows[rect.y as usize].clone()
    };
    assert!(
        row_of("pane_1").trim_end().ends_with("running"),
        "{}",
        row_of("pane_1")
    );
    assert!(
        row_of("pane_2").trim_end().ends_with("done"),
        "{}",
        row_of("pane_2")
    );
    assert!(
        row_of("pane_3").trim_end().ends_with("idle"),
        "{}",
        row_of("pane_3")
    );
    assert!(
        row_of("pane_4").trim_end().ends_with("needs you"),
        "{}",
        row_of("pane_4")
    );
    // pane_1 is the open pane: only its row carries the bar
    assert!(row_of("pane_1").contains("▌✳"), "{}", row_of("pane_1"));
    assert!(!row_of("pane_2").contains("▌◎"), "{}", row_of("pane_2"));
}
