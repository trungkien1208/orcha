//! orcha: the main frame's tabs and the panes dressed inside it, at fixed
//! geometry with one and many panes and tabs.
use super::*;

const COLS: u16 = 200;
const ROWS: u16 = 50;

#[test]
fn idle_clients_sleep_longer_without_delaying_feedback_or_selection() {
    let mut state = chrome_state(1, 1);
    let now = std::time::Instant::now();
    let ms = std::time::Duration::from_millis;
    assert_eq!(state.timer_delay(now), ms(500));
    state.copy_feedback_deadline = Some(now + ms(40));
    assert_eq!(state.timer_delay(now), ms(40));
    state.selection_repaint_deadline = Some(now + ms(16));
    assert_eq!(state.timer_delay(now), ms(16));
    state.selection_repaint_deadline = None;
    state.copy_feedback_deadline = None;
    state.orcha_ui.next_process_tick = Some(now + ms(220));
    assert_eq!(state.timer_delay(now), ms(220));
}

#[test]
fn hiding_orcha_does_not_leave_an_expired_animation_waking_the_loop() {
    let mut state = chrome_state(1, 1);
    state.config.sidebar_tree = false;
    let now = std::time::Instant::now();
    state.orcha_ui.next_process_tick = Some(now);
    state.tick_orcha(now);
    assert!(state.orcha_ui.next_process_tick.is_none());
    assert_eq!(
        state.timer_delay(now),
        std::time::Duration::from_millis(500)
    );
}

#[test]
fn pane_titles_keep_combining_letters_and_emoji_as_whole_cells() {
    let mut state = chrome_state(1, 1);
    let data = state.snapshot.as_deref_mut().expect("snapshot");
    data.agents.retain(|agent| agent.pane_id != "pane_1");
    data.agents.push(agent(
        "pane_1",
        "tab_1",
        "Claude",
        "Fix e\u{301} and 👩‍💻",
        AgentStatus::Working,
    ));
    let frame = state.compose(COLS, ROWS).expect("frame").frame;
    assert!(frame.cells.iter().any(|cell| cell.symbol == "e\u{301}"));
    assert!(frame.cells.iter().any(|cell| cell.symbol == "👩‍💻"));
}

fn agent(pane: &str, tab: &str, kind: &str, task: &str, status: AgentStatus) -> ClientShellAgent {
    ClientShellAgent {
        pane_id: pane.into(),
        workspace_id: "ws_1".into(),
        tab_id: tab.into(),
        name: None,
        display_agent: Some(kind.into()),
        agent: Some(kind.to_lowercase()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: status,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: if task.is_empty() {
            Vec::new()
        } else {
            vec![("task".into(), task.into())]
        },
        focused: false,
    }
}

/// `panes` panes tiled in the focused tab (an agent in each but the last),
/// `tabs` tabs in the focused project (an agent in each other tab).
fn chrome_state(panes: usize, tabs: usize) -> ClientShellState {
    let mut data = snapshot();
    let tab = data.tabs[0].clone();
    let pane = data.panes[0].clone();
    data.tabs.clear();
    data.panes.clear();
    let kinds = ["Claude", "Codex", "Cursor"];
    let statuses = [
        AgentStatus::Working,
        AgentStatus::Blocked,
        AgentStatus::Done,
    ];
    for index in 0..tabs {
        let mut next = tab.clone();
        next.tab_id = format!("tab_{}", index + 1);
        next.number = index + 1;
        next.label = if index % 2 == 0 {
            String::new()
        } else {
            format!("work {index}")
        };
        next.custom_label = index % 4 == 3;
        next.focused = index == 0;
        next.agent_status = statuses[index % 3];
        data.tabs.push(next);
        if index > 0 {
            let id = format!("pane_t{index}");
            let mut other = pane.clone();
            other.pane_id = id.clone();
            other.tab_id = format!("tab_{}", index + 1);
            other.focused = false;
            data.panes.push(other);
            let task = if index % 3 == 0 {
                "claude".to_owned()
            } else {
                format!("Fix the flaky login test number {index}")
            };
            data.agents.push(agent(
                &id,
                &format!("tab_{}", index + 1),
                kinds[index % 3],
                &task,
                statuses[index % 3],
            ));
        }
    }
    for index in 0..panes {
        let id = format!("pane_{}", index + 1);
        let mut next = pane.clone();
        next.pane_id = id.clone();
        next.focused = index == 0;
        next.label = (index % 5 == 4).then(|| format!("logs {index}"));
        next.cwd = Some(format!("/projects/app-{index}"));
        next.foreground_cwd = None;
        data.panes.push(next);
        if index + 1 < panes || panes == 1 {
            data.agents.push(agent(
                &id,
                "tab_1",
                kinds[index % 3],
                if index % 2 == 0 {
                    "Dark mode for the settings menu"
                } else {
                    ""
                },
                statuses[index % 3],
            ));
        }
    }
    data.focused_pane_id = Some("pane_1".into());
    let config: Config = toml::from_str("[theme]\nname = 'orcha'\n").expect("theme");
    let mut config = ClientShellConfig::from_config(&config);
    config.sidebar_tree = true;
    config.right_panel = false;
    config.status_bar = true;
    let mut state = ClientShellState::new(config);
    state.sidebar_width = 34;
    state.set_snapshot(Box::new(data));
    let area = state.layout(COLS, ROWS).pane_surface;
    state.set_pane_surface(tiled_surface(area.width, area.height, panes));
    state
}

/// A surface `width`×`height` with `count` bordered panes in a grid, some
/// cells colored by the program and the rest on the terminal's defaults.
fn tiled_surface(width: u16, height: u16, count: usize) -> PaneSurfaceFrame {
    let mut surface = surface();
    let columns = (count as f64).sqrt().ceil() as u16;
    let rows = (count as u16).div_ceil(columns);
    let mut buffer = Buffer::empty(Rect::new(0, 0, width, height));
    for y in 0..height {
        for x in 0..width {
            let cell = &mut buffer[(x, y)];
            cell.set_symbol(if (x + y) % 7 == 0 { "#" } else { "x" });
            if (x * 3 + y) % 11 == 0 {
                cell.set_bg(ratatui::style::Color::Rgb(10, 20, 30));
            }
            if (x + y * 5) % 13 == 0 {
                cell.set_fg(ratatui::style::Color::Rgb(200, 100, 50));
            }
        }
    }
    surface.frame = FrameData::from_ratatui_buffer_with_hyperlinks(&buffer, None, &[]);
    let pane_width = width / columns;
    let pane_height = height / rows;
    surface.panes = (0..count)
        .map(|index| {
            let (column, row) = (index as u16 % columns, index as u16 / columns);
            let x = column * pane_width;
            let y = row * pane_height;
            // the last column and row take what is left, but leave a gap
            // on the right so some cells sit outside every pane
            let w = if column + 1 == columns {
                width.saturating_sub(x + 2)
            } else {
                pane_width
            };
            let h = if row + 1 == rows {
                height - y
            } else {
                pane_height
            };
            PaneSurfacePane {
                pane_id: format!("pane_{}", index + 1),
                content_revision: 0,
                rect: SurfaceRect {
                    x,
                    y,
                    width: w,
                    height: h,
                },
                inner_rect: SurfaceRect {
                    x: x + 1,
                    y: y + 1,
                    width: w.saturating_sub(2),
                    height: h.saturating_sub(2),
                },
                scrollbar_rect: None,
                scroll: None,
                focused: index == 0,
                mouse_reporting: false,
                sgr_pixel_mouse: false,
                alternate_screen_active: false,
                pixel_width: 0,
                pixel_height: 0,
            }
        })
        .collect();
    surface
}

#[test]
#[ignore = "manual: writes composed frames for a before/after comparison"]
fn orcha_chrome_dump() {
    let Ok(path) = std::env::var("ORCHA_CHROME_DUMP_OUT") else {
        return;
    };
    let mut out = String::new();
    for (panes, tabs) in [(1, 1), (4, 3), (15, 15), (7, 30)] {
        for paper in [true, false] {
            let mut state = chrome_state(panes, tabs);
            state.config.orcha.pane_paper = paper;
            let frame = state.compose(COLS, ROWS).expect("frame").frame;
            out.push_str(&format!("== panes={panes} tabs={tabs} paper={paper}\n"));
            for cell in &frame.cells {
                out.push_str(&format!(
                    "{:?} {} {} {} {} {:?}\n",
                    cell.symbol, cell.fg, cell.bg, cell.modifier, cell.skip, cell.hyperlink
                ));
            }
            out.push_str(&format!("{:?}\n", state.hits.tabs));
        }
    }
    std::fs::write(path, out).expect("write dump");
}

#[test]
#[ignore = "manual fixed-geometry chrome profile"]
fn orcha_chrome_render_scale_profile() {
    for count in [1, 15] {
        let mut state = chrome_state(count, count);
        let mut samples = Vec::new();
        for index in 0..400 {
            let start = std::time::Instant::now();
            std::hint::black_box(state.compose(COLS, ROWS).expect("frame"));
            if index >= 50 {
                samples.push(start.elapsed().as_nanos());
            }
        }
        samples.sort_unstable();
        eprintln!(
            "orcha_chrome compose panes={count} tabs={count} geometry={COLS}x{ROWS} median_ns={} p95_ns={}",
            samples[samples.len() / 2],
            samples[samples.len() * 95 / 100]
        );
        // the pane dressing alone, on a frame already composed
        let mut frame = state.compose(COLS, ROWS).expect("frame").frame;
        let surface = state.layout(COLS, ROWS).pane_surface;
        let snapshot = state.snapshot.as_deref().expect("snapshot");
        let mut samples = Vec::new();
        for index in 0..400 {
            let start = std::time::Instant::now();
            super::super::orcha_chrome::dress_panes(
                &mut frame,
                surface,
                &state.hits.panes,
                snapshot,
                &state.config,
            );
            if index >= 50 {
                samples.push(start.elapsed().as_nanos());
            }
        }
        std::hint::black_box(&frame);
        samples.sort_unstable();
        eprintln!(
            "orcha_chrome dress_panes panes={count} geometry={COLS}x{ROWS} median_ns={} p95_ns={}",
            samples[samples.len() / 2],
            samples[samples.len() * 95 / 100]
        );
    }
}

/// WCAG contrast of two RGB colors; None for palette indexes.
fn contrast(a: ratatui::style::Color, b: ratatui::style::Color) -> Option<f64> {
    let luminance = |color| match color {
        ratatui::style::Color::Rgb(r, g, b) => {
            let channel = |value: u8| {
                let c = f64::from(value) / 255.0;
                if c <= 0.039_28 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            Some(0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b))
        }
        _ => None,
    };
    let (a, b) = (luminance(a)?, luminance(b)?);
    Some((a.max(b) + 0.05) / (a.min(b) + 0.05))
}

#[test]
fn the_focused_pane_has_the_bright_bar_and_every_bar_names_its_state() {
    for theme in ["orcha", "orcha-dark"] {
        let mut state = chrome_state(2, 1);
        let config: Config =
            toml::from_str(&format!("[theme]\nname = '{theme}'\n")).expect("theme");
        state.config.palette = ClientShellConfig::from_config(&config).palette;
        // two Claude panes side by side: the focused one runs, the other is done
        {
            let data = state.snapshot.as_mut().expect("snapshot");
            data.agents
                .retain(|agent| agent.pane_id != "pane_1" && agent.pane_id != "pane_2");
            data.agents.push(agent(
                "pane_1",
                "tab_1",
                "Claude",
                "Dark mode for the settings menu",
                AgentStatus::Working,
            ));
            data.agents
                .push(agent("pane_2", "tab_1", "Claude", "", AgentStatus::Done));
        }
        let frame = state.compose(COLS, ROWS).expect("frame").frame;
        let t = super::super::brutal::Brutal::new(&state.config.palette);
        let bar = |pane_id: &str| {
            let hit = state
                .hits
                .panes
                .iter()
                .find(|hit| hit.pane_id == pane_id)
                .expect("pane hit");
            let row = usize::from(hit.rect.y) * usize::from(frame.width);
            let cells = &frame.cells
                [row + usize::from(hit.rect.x) + 1..row + usize::from(hit.rect.right()) - 1];
            let text: String = cells.iter().map(|cell| cell.symbol.as_str()).collect();
            (text, cells.to_vec())
        };
        let (focused_text, focused) = bar("pane_1");
        let (quiet_text, quiet) = bar("pane_2");
        assert_eq!(
            focused[1].bg,
            crate::protocol::color_to_u32(t.yellow),
            "{theme}: Claude's color"
        );
        assert_eq!(
            quiet[1].bg,
            crate::protocol::color_to_u32(t.rail_ahead),
            "{theme}: a quiet bar"
        );
        assert_ne!(
            focused[1].modifier & crate::protocol::modifier_to_u16(Modifier::BOLD),
            0,
            "{theme}: the focused bar is bold"
        );
        assert_eq!(
            quiet[1].modifier,
            crate::protocol::modifier_to_u16(Modifier::empty())
        );
        assert!(
            focused_text.trim_end().ends_with("running"),
            "{focused_text}"
        );
        assert!(focused_text.contains("· Dark mode"), "{focused_text}");
        assert!(quiet_text.trim_end().ends_with("done"), "{quiet_text}");
        assert!(!focused_text.contains('●') && !quiet_text.contains('●'));
        // the quiet bar's name and state word read on it
        if let Some(ratio) = contrast(t.ink, t.rail_ahead) {
            assert!(ratio >= 4.5, "{theme}: ink on the quiet bar {ratio:.2}");
        }
        let word = quiet
            .iter()
            .rev()
            .find(|cell| cell.symbol == "e")
            .expect("the word's last letter");
        assert_eq!(
            word.fg,
            crate::protocol::color_to_u32(t.text(t.green, t.rail_ahead))
        );
        // the logo in the state color: done is green
        assert_eq!(
            quiet[1].fg,
            crate::protocol::color_to_u32(t.mark(t.green, t.rail_ahead))
        );
    }
}

#[test]
fn a_narrow_bar_clips_the_task_before_the_state_word() {
    let mut state = chrome_state(4, 1);
    {
        let data = state.snapshot.as_mut().expect("snapshot");
        data.agents.retain(|agent| agent.pane_id != "pane_2");
        data.agents.push(agent(
            "pane_2",
            "tab_1",
            "Codex",
            "A very long task title that cannot fit in a quarter of the frame at all",
            AgentStatus::Blocked,
        ));
    }
    let frame = state.compose(COLS, ROWS).expect("frame").frame;
    let hit = state
        .hits
        .panes
        .iter()
        .find(|hit| hit.pane_id == "pane_2")
        .expect("pane hit");
    let row = usize::from(hit.rect.y) * usize::from(frame.width);
    let text: String = frame.cells
        [row + usize::from(hit.rect.x)..row + usize::from(hit.rect.right())]
        .iter()
        .map(|cell| cell.symbol.as_str())
        .collect();
    assert!(text.contains('…'), "{text}");
    assert!(text.ends_with("needs you ▕"), "{text}");
}
