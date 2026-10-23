//! orcha: the header row, the main frame with the project's tabs in its top
//! edge, and the dressing of the panes inside it (paper colors and a title
//! bar in the agent's color). See docs/orcha/SPEC.md §2.

use crate::protocol::{color_to_u32, modifier_to_u16, FrameData};

use super::brutal::{self, Brutal};
use super::render::{display_width, put_text};
use super::*;

/// Header, one row: the pink ORCHA chip with the icon, the counts, the
/// low-fuel offer, and the outlined settings button.
pub(super) fn render_header(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    ui: &super::OrchaUiState,
    hits: &mut ShellHitMap,
) {
    let t = Brutal::new(&config.palette);
    let icon = brutal::orcha_icon(config.icon_font);
    let label = format!("{icon} ORCHA");
    // a colored cell on the window's edge would stretch into its padding
    let chip = Rect::new(area.x + 1, area.y, display_width(&label) + 4, 1);
    brutal::pill(buffer, chip, &label, t.chip_text, t.pink, false, &t);
    let projects = snapshot.workspaces.len();
    let agents = snapshot.agents.len();
    let counts = format!(
        "{projects} project{} · {agents} agent{}",
        if projects == 1 { "" } else { "s" },
        if agents == 1 { "" } else { "s" },
    );
    let x = chip.right() + 2;
    put_text(
        buffer,
        x,
        area.y,
        area.right().saturating_sub(x),
        &counts,
        Style::default().fg(t.muted).bg(t.bg),
    );
    let text = "⌘, settings";
    let button = Rect::new(
        area.right().saturating_sub(display_width(text) + 5),
        area.y,
        display_width(text) + 4,
        1,
    );
    if button.x <= x + display_width(&counts) + 4 {
        return;
    }
    brutal::pill(buffer, button, text, t.ink, t.card, true, &t);
    hits.orcha.settings = button;
    // low fuel: a red offer left of settings, a click opens the accounts
    if let Some(hint) = &ui.low_fuel_hint {
        let hint = format!(" ⚠ {hint} ");
        let room = button.x.saturating_sub(x + display_width(&counts) + 4);
        let width = display_width(&hint).min(room);
        let hx = button.x.saturating_sub(width + 2);
        put_text(
            buffer,
            hx,
            area.y,
            width,
            &hint,
            Style::default()
                .fg(t.card)
                .bg(t.red)
                .add_modifier(Modifier::BOLD),
        );
        hits.orcha.fuel_hint = Rect::new(hx, area.y, width, 1);
    }
}

/// The main frame (heavy, frame fill, hard shadow) and the focused
/// project's tabs over it: each tab a box sitting on the frame's top edge,
/// the open one heavy and joined to the frame.
#[allow(clippy::too_many_arguments)] // one render call, all inputs are distinct facts
pub(super) fn render_frame_and_tabs(
    buffer: &mut Buffer,
    frame: Rect,
    tab_area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    tab_scroll: &mut usize,
    reveal_focused_tab: &mut bool,
    tab_drag_insert_index: Option<usize>,
    hits: &mut ShellHitMap,
) {
    let t = Brutal::new(&config.palette);
    let top = tab_area.y;
    let paper = Rect::new(
        frame.x.saturating_sub(2),
        top,
        frame.width.saturating_add(4),
        frame.bottom().saturating_sub(top) + 1,
    )
    .intersection(buffer.area);
    buffer.set_style(paper, Style::default().bg(t.bg));
    // the frame's shadow, outside the panes
    brutal::draw_shadow(
        buffer,
        Rect::new(frame.x, frame.y, frame.width.saturating_sub(1), frame.height.saturating_sub(1)),
        t.shadow,
        t.bg,
    );

    let tabs: Vec<&ClientShellTab> = snapshot
        .tabs
        .iter()
        .filter(|tab| Some(tab.workspace_id.as_str()) == snapshot.focused_workspace_id.as_deref())
        .collect();
    // a tab nobody named is called after what its agent is doing
    let label = |tab: &ClientShellTab| {
        // herdr names an unnamed tab after its folder or program; the chat
        // title the agent set says more
        let auto = !tab.custom_label;
        let task = auto
            .then(|| {
                snapshot
                    .agents
                    .iter()
                    .filter(|agent| agent.tab_id == tab.tab_id)
                    .map(brutal::agent_task)
                    // a bare program name is not a chat title yet
                    .find(|task| {
                        let task = task.trim().to_lowercase();
                        !task.is_empty()
                            && !matches!(
                                task.as_str(),
                                "claude" | "claude code" | "codex" | "cursor" | "cursor agent"
                            )
                    })
            })
            .flatten();
        let name = task.unwrap_or_else(|| {
            if tab.label.trim().is_empty() {
                tab.number.to_string()
            } else {
                tab.label.clone()
            }
        });
        brutal::clip(&name, 22)
    };
    // box width: an edge, " name ", the state mark and a space, an edge
    // a state square only when an agent runs in the tab
    let width_of = |tab: &ClientShellTab| {
        display_width(&label(tab)) + 4 + if t.state(tab.agent_status).is_some() { 2 } else { 0 }
    };
    if std::mem::take(reveal_focused_tab) {
        if let Some(focused) = tabs.iter().position(|tab| tab.focused) {
            if focused < *tab_scroll {
                *tab_scroll = focused;
            } else {
                let budget = tab_area.width.saturating_sub(14);
                while *tab_scroll < focused
                    && tabs[*tab_scroll..=focused]
                        .iter()
                        .map(|tab| width_of(tab) + 1)
                        .sum::<u16>()
                        > budget
                {
                    *tab_scroll += 1;
                }
            }
        }
    }
    *tab_scroll = (*tab_scroll).min(tabs.len().saturating_sub(1));

    let y = tab_area.y;
    let right = tab_area.right();
    let reserve = 12;
    // the first tab starts on the frame's edge; a column between tabs
    let mut x = frame.x;
    if *tab_scroll > 0 {
        let more = format!("‹{}", *tab_scroll);
        let w = display_width(&more);
        put_text(buffer, x, y, w, &more, Style::default().fg(t.muted).bg(t.bg));
        hits.tab_scroll_left = Rect::new(x, y, w, 1);
        x += w + 1;
    }
    let mut shown_until = *tab_scroll;
    for (index, tab) in tabs.iter().enumerate().skip(*tab_scroll) {
        let w = width_of(tab);
        let last = index + 1 == tabs.len();
        let limit = if last { right.saturating_sub(4) } else { right.saturating_sub(reserve) };
        if x + w > limit {
            break;
        }
        // a box over the frame: a top edge in the row above, a line down
        // each side; the open one has a yellow top and opens into the frame
        let open = tab.focused;
        let fill = if open { t.frame } else { t.card };
        let rect = Rect::new(x, y, w, 1);
        brutal::pill(buffer, rect, "", t.ink, fill, true, &t);
        let edge = Style::default().fg(t.line).bg(t.bg);
        let above = y.saturating_sub(1);
        if open {
            put_text(buffer, x, above, w, &"▂".repeat(usize::from(w)), Style::default().fg(t.yellow).bg(t.bg));

        } else {
            put_text(buffer, x, above, w, &"▁".repeat(usize::from(w)), edge);
        }
        let name = label(tab);
        let mut style = Style::default()
            .fg(if open { t.ink } else { t.muted })
            .bg(fill);
        if open {
            style = style.add_modifier(Modifier::BOLD);
        }
        put_text(buffer, x + 2, y, display_width(&name), &name, style);
        if let Some(color) = t.state(tab.agent_status) {
            put_text(
                buffer,
                x + 2 + display_width(&name) + 1,
                y,
                1,
                "■",
                Style::default().fg(color).bg(fill),
            );
        }
        hits.tabs.push((rect, tab.tab_id.clone()));
        x += w + 1;
        shown_until = index + 1;
    }
    let hidden = tabs.len().saturating_sub(shown_until);
    if hidden > 0 {
        let more = format!(" +{hidden} ▾ ");
        let w = display_width(&more);
        put_text(buffer, x, y, w, &more, Style::default().fg(t.muted).bg(t.bg));
        hits.tab_scroll_right = Rect::new(x, y, w, 1);
        x += w + 1;
    }
    if config.mouse_capture && x + 3 <= right {
        put_text(
            buffer,
            x,
            y,
            3,
            " + ",
            Style::default()
                .fg(t.ink)
                .bg(t.bg)
                .add_modifier(Modifier::BOLD),
        );
        hits.new_tab = Rect::new(x, y, 3, 1);
    }
    if let Some(insert) = tab_drag_insert_index {
        let at = tabs
            .get(insert)
            .and_then(|tab| hits.tabs.iter().find(|(_, id)| id == &tab.tab_id))
            .map(|(rect, _)| rect.x.saturating_sub(1))
            .or_else(|| hits.tabs.last().map(|(rect, _)| rect.right()));
        if let Some(ix) = at {
            put_text(buffer, ix, y, 1, "┃", Style::default().fg(t.pink).bg(t.bg));
        }
    }
}

/// Paints the panes inside the frame after the server's surface is blitted:
/// default-colored cells get the style's paper colors, borders the line
/// color, and a bordered pane's top row becomes a title bar in its agent's
/// color ("✳ OPUS 5.5 · dark mode").
pub(super) fn dress_panes(
    frame: &mut FrameData,
    surface: Rect,
    panes: &[PaneHit],
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
) {
    let t = Brutal::new(&config.palette);
    let reset = color_to_u32(ratatui::style::Color::Reset);
    let width = usize::from(frame.width);
    let height = usize::from(frame.height);
    let paper = config.orcha.pane_paper;
    let (pane_bg, pane_fg, line, frame_bg) = (
        color_to_u32(t.pane),
        color_to_u32(t.pane_text),
        color_to_u32(t.line),
        color_to_u32(t.frame),
    );
    let inside = |rect: Rect, x: u16, y: u16| {
        x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom()
    };
    for y in surface.y..surface.bottom() {
        for x in surface.x..surface.right() {
            let (ux, uy) = (usize::from(x), usize::from(y));
            if ux >= width || uy >= height {
                continue;
            }
            let Some(cell) = frame.cells.get_mut(uy * width + ux) else {
                continue;
            };
            match panes.iter().find(|pane| inside(pane.rect, x, y)) {
                Some(pane) if inside(pane.inner_rect, x, y) => {
                    if paper {
                        if cell.bg == reset {
                            cell.bg = pane_bg;
                        }
                        if cell.fg == reset {
                            cell.fg = pane_fg;
                        }
                    }
                }
                Some(pane) => {
                    // the pane's lines run along the very edge of its own
                    // cells, on its paper: square corners, no gap
                    let r = pane.rect;
                    let (left, right, bottom) =
                        (x == r.x, x + 1 == r.right(), y + 1 == r.bottom());
                    let glyph = match (left, right, bottom) {
                        (true, _, true) => Some("\u{1FB7C}"),
                        (_, true, true) => Some("\u{1FB7F}"),
                        (_, _, true) => Some("▁"),
                        (true, _, _) => Some("▏"),
                        (_, true, _) => Some("▕"),
                        _ if y == r.y => Some("▔"),
                        _ => None,
                    };
                    match glyph {
                        Some(glyph) => {
                            cell.fg = line;
                            cell.bg = pane_bg;
                            cell.symbol = glyph.to_owned();
                        }
                        None => {
                            if cell.bg == reset {
                                cell.bg = pane_bg;
                            }
                        }
                    }
                }
                None => {
                    if cell.bg == reset {
                        cell.bg = frame_bg;
                    }
                }
            }
        }
    }

    for pane in panes {
        if pane.inner_rect.y <= pane.rect.y || pane.rect.width < 6 {
            continue;
        }
        let agent = snapshot.agents.iter().find(|agent| agent.pane_id == pane.pane_id);
        let focused = snapshot.focused_pane_id.as_deref() == Some(pane.pane_id.as_str());
        let (title, bg) = match agent {
            Some(agent) => {
                let logo = brutal::agent_logo(agent.agent.as_deref(), config.icon_font);
                let task = brutal::agent_task(agent);
                let name = brutal::agent_label(agent).to_uppercase();
                let title = if task.is_empty() {
                    format!(" {logo} {name}")
                } else {
                    format!(" {logo} {name} · {task}")
                };
                (title, t.agent(agent.agent.as_deref()))
            }
            None => {
                let pane_info = snapshot.panes.iter().find(|p| p.pane_id == pane.pane_id);
                let label = pane_info
                    .and_then(|p| p.label.clone())
                    .or_else(|| {
                        pane_info
                            .and_then(|p| p.foreground_cwd.clone().or_else(|| p.cwd.clone()))
                            .map(|cwd| {
                                std::path::Path::new(&cwd)
                                    .file_name()
                                    .map(|name| name.to_string_lossy().into_owned())
                                    .unwrap_or(cwd)
                            })
                    })
                    .unwrap_or_else(|| "shell".to_owned());
                (format!(" ▶ {label}"), t.rail_ahead)
            }
        };
        let mark = if focused { " ●" } else { "" };
        let text = brutal::clip(
            &format!("{title}{mark}"),
            usize::from(pane.rect.width.saturating_sub(1)),
        );
        let mut modifier = ratatui::style::Modifier::empty();
        if focused {
            modifier |= ratatui::style::Modifier::BOLD;
        }
        let y = usize::from(pane.rect.y);
        if y >= height {
            continue;
        }
        let (first_x, last_x) = (pane.rect.x, pane.rect.right().saturating_sub(1));
        let mut chars = text.chars().peekable();
        let mut x = pane.rect.x;
        while x < pane.rect.right() {
            let ux = usize::from(x);
            if ux >= width {
                break;
            }
            // the pane's side lines run up its title bar
            let symbol = if x == first_x {
                "▏".to_owned()
            } else if x == last_x {
                "▕".to_owned()
            } else {
                chars.next().map(|c| c.to_string()).unwrap_or_else(|| " ".into())
            };
            let wide = UnicodeWidthStr::width(symbol.as_str()) > 1;
            let edge = x == first_x || x == last_x;
            if let Some(cell) = frame.cells.get_mut(y * width + ux) {
                cell.symbol = symbol;
                cell.fg = color_to_u32(if edge { t.line } else { t.chip_text });
                cell.bg = color_to_u32(bg);
                cell.modifier = modifier_to_u16(modifier);
                cell.skip = false;
                cell.hyperlink = None;
            }
            x += 1;
            if wide && x < pane.rect.right() {
                if let Some(cell) = frame.cells.get_mut(y * width + usize::from(x)) {
                    cell.symbol = String::new();
                    cell.bg = color_to_u32(bg);
                    cell.skip = true;
                }
                x += 1;
            }
        }
    }
}

/// The pane's paper colors on cells that keep the terminal's defaults: the
/// same as the full frame gets, for rows a surface patch sends straight out
/// (scrolling), which would otherwise show the terminal's own background
/// (see-through with background blur).
pub(super) fn paper_cells(cells: &mut [crate::protocol::CellData], config: &ClientShellConfig) {
    if !config.orcha.pane_paper {
        return;
    }
    let t = Brutal::new(&config.palette);
    let reset = color_to_u32(ratatui::style::Color::Reset);
    let (pane_bg, pane_fg) = (color_to_u32(t.pane), color_to_u32(t.pane_text));
    for cell in cells {
        if cell.bg == reset {
            cell.bg = pane_bg;
        }
        if cell.fg == reset {
            cell.fg = pane_fg;
        }
    }
}
