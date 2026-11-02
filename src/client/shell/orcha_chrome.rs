//! orcha: the header row, the main frame with the project's tabs in its top
//! edge, and the dressing of the panes inside it (paper colors and a title
//! bar in the agent's color). See docs/orcha/SPEC.md §2.

use std::collections::HashMap;
use unicode_segmentation::UnicodeSegmentation;

use crate::protocol::{
    color_to_u32, modifier_to_u16, ClientShellAgent, ClientShellPane, FrameData,
};

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
    // the version first: which orcha this is, at a glance
    let counts = format!(
        "v{} · {projects} project{} · {agents} agent{}",
        crate::orcha_app::ORCHA_VERSION,
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
    // agents waiting on you: a red chip, a click goes to the first of them
    let waiting = snapshot
        .agents
        .iter()
        .filter(|agent| agent.agent_status == crate::api::schema::AgentStatus::Blocked)
        .count();
    let mut counts_end = x + display_width(&counts);
    if waiting > 0 {
        let text = format!(
            " ⚠ {waiting} need{} you ",
            if waiting == 1 { "s" } else { "" }
        );
        let width = display_width(&text);
        let nx = counts_end + 2;
        if nx + width < area.right() {
            put_text(
                buffer,
                nx,
                area.y,
                width,
                &text,
                Style::default()
                    .fg(t.card)
                    .bg(t.red)
                    .add_modifier(Modifier::BOLD),
            );
            hits.orcha.needs_you = Rect::new(nx, area.y, width, 1);
            counts_end = nx + width;
        }
    }
    // right: settings, then quit (back to the terminal; agents keep running)
    let quit_text = "⏻ quit";
    let quit = Rect::new(
        area.right().saturating_sub(display_width(quit_text) + 5),
        area.y,
        display_width(quit_text) + 4,
        1,
    );
    let text = "⚙ settings";
    let button = Rect::new(
        quit.x.saturating_sub(display_width(text) + 5),
        area.y,
        display_width(text) + 4,
        1,
    );
    if button.x <= counts_end + 4 {
        return;
    }
    brutal::pill(buffer, button, text, t.ink, t.card, true, &t);
    hits.orcha.settings = button;
    brutal::pill(buffer, quit, quit_text, t.ink, t.card, true, &t);
    hits.orcha.quit = quit;
    // COMMANDS: every action in one search, left of settings
    let mut left_of = button.x;
    let commands_text = "⌕ commands";
    let commands = Rect::new(
        button.x.saturating_sub(display_width(commands_text) + 5),
        area.y,
        display_width(commands_text) + 4,
        1,
    );
    if commands.x > counts_end + 4 {
        brutal::pill(buffer, commands, commands_text, t.ink, t.card, true, &t);
        hits.orcha.commands = commands;
        left_of = commands.x;
    }
    // Easy Job: shown once added, ● while its server runs; a click opens it
    if let Some(running) = ui.office {
        let text = if running { "● office" } else { "○ office" };
        let chip = Rect::new(
            left_of.saturating_sub(display_width(text) + 5),
            area.y,
            display_width(text) + 4,
            1,
        );
        if chip.x > counts_end + 4 {
            let fill = if running { t.green } else { t.card };
            let ink = if running { t.chip_text } else { t.muted };
            brutal::pill(buffer, chip, text, ink, fill, true, &t);
            hits.orcha.office = chip;
            left_of = chip.x;
        }
    }
    // a newer orcha: a green button left of the buttons, a click installs it
    if let Some(note) = &ui.update_note {
        let text = format!(" {note} ");
        let room = left_of.saturating_sub(counts_end + 4);
        let width = display_width(&text).min(room);
        let ux = left_of.saturating_sub(width + 2);
        put_text(
            buffer,
            ux,
            area.y,
            width,
            &text,
            Style::default()
                .fg(t.chip_text)
                .bg(t.green)
                .add_modifier(Modifier::BOLD),
        );
        hits.orcha.update = Rect::new(ux, area.y, width, 1);
        left_of = ux;
    }
    // low fuel: a red offer left of settings, a click opens the accounts
    if let Some(hint) = &ui.low_fuel_hint {
        let hint = format!(" ⚠ {hint} ");
        let room = left_of.saturating_sub(counts_end + 4);
        let width = display_width(&hint).min(room);
        let hx = left_of.saturating_sub(width + 2);
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

/// The focused project's tabs laid out before they are drawn: each tab's
/// name and box width, and the first tab shown (already moved to reveal the
/// open tab and clamped). Drawing only reads it.
pub(super) struct TabStrip<'a> {
    tabs: Vec<&'a ClientShellTab>,
    boxes: Vec<(String, u16)>,
    scroll: usize,
}

/// Lays the tab strip out for `tab_area`. The only state it changes is the
/// client's tab scroll: `tab_scroll` comes back clamped, and moved to show
/// the open tab when `reveal_focused_tab` asks for it (which it clears).
pub(super) fn layout_tabs<'a>(
    tab_area: Rect,
    snapshot: &'a ClientShellSnapshot,
    config: &ClientShellConfig,
    tab_scroll: &mut usize,
    reveal_focused_tab: &mut bool,
) -> TabStrip<'a> {
    let t = Brutal::new(&config.palette);
    let tabs: Vec<&ClientShellTab> = snapshot
        .tabs
        .iter()
        .filter(|tab| Some(tab.workspace_id.as_str()) == snapshot.focused_workspace_id.as_deref())
        .collect();
    // Find the first useful agent title for each tab in one snapshot pass.
    // Looking through every agent again per tab scales quadratically.
    let mut tasks = HashMap::new();
    for agent in &snapshot.agents {
        if tasks.contains_key(agent.tab_id.as_str()) {
            continue;
        }
        let task = brutal::agent_task(agent);
        let trimmed = task.trim();
        if !trimmed.is_empty() && !brutal::names_only_a_program(trimmed) {
            tasks.insert(agent.tab_id.as_str(), task);
        }
    }
    // each tab's name and box width, worked out once: the scroll and the
    // drawing both need them
    let boxes: Vec<(String, u16)> = tabs
        .iter()
        .map(|tab| {
            let name = tab_label(tab, tasks.get(tab.tab_id.as_str()).map(String::as_str));
            // box width: an edge, " name ", the state mark and a space, an
            // edge; a state square only when an agent runs in the tab
            let mark = if t.state(tab.agent_status).is_some() {
                2
            } else {
                0
            };
            let width = display_width(&name) + 4 + mark;
            (name, width)
        })
        .collect();
    if std::mem::take(reveal_focused_tab) {
        if let Some(focused) = tabs.iter().position(|tab| tab.focused) {
            if focused < *tab_scroll {
                *tab_scroll = focused;
            } else {
                let budget = tab_area.width.saturating_sub(14);
                while *tab_scroll < focused
                    && boxes[*tab_scroll..=focused]
                        .iter()
                        .map(|(_, width)| width + 1)
                        .sum::<u16>()
                        > budget
                {
                    *tab_scroll += 1;
                }
            }
        }
    }
    *tab_scroll = (*tab_scroll).min(tabs.len().saturating_sub(1));
    TabStrip {
        tabs,
        boxes,
        scroll: *tab_scroll,
    }
}

/// The main frame (heavy, frame fill, hard shadow) and the focused
/// project's tabs over it (laid out by `layout_tabs`): each tab a box
/// sitting on the frame's top edge, the open one heavy and joined to the
/// frame.
#[allow(clippy::too_many_arguments)] // one render call, all inputs are distinct facts
pub(super) fn render_frame_and_tabs(
    buffer: &mut Buffer,
    frame: Rect,
    tab_area: Rect,
    strip: &TabStrip<'_>,
    config: &ClientShellConfig,
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
        Rect::new(
            frame.x,
            frame.y,
            frame.width.saturating_sub(1),
            frame.height.saturating_sub(1),
        ),
        t.shadow,
        t.bg,
    );
    let (tabs, boxes, scroll) = (&strip.tabs, &strip.boxes, strip.scroll);

    let y = tab_area.y;
    let right = tab_area.right();
    let reserve = 12;
    // the first tab starts on the frame's edge; a column between tabs
    let mut x = frame.x;
    if scroll > 0 {
        let more = format!("‹{scroll}");
        let w = display_width(&more);
        put_text(
            buffer,
            x,
            y,
            w,
            &more,
            Style::default().fg(t.muted).bg(t.bg),
        );
        hits.tab_scroll_left = Rect::new(x, y, w, 1);
        x += w + 1;
    }
    let mut shown_until = scroll;
    for (index, (tab, (name, w))) in tabs.iter().zip(boxes).enumerate().skip(scroll) {
        let w = *w;
        let last = index + 1 == tabs.len();
        let limit = if last {
            right.saturating_sub(4)
        } else {
            right.saturating_sub(reserve)
        };
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
            put_text(
                buffer,
                x,
                above,
                w,
                &"▂".repeat(usize::from(w)),
                Style::default().fg(t.mark(t.yellow, t.bg)).bg(t.bg),
            );
        } else {
            put_text(buffer, x, above, w, &"▁".repeat(usize::from(w)), edge);
        }
        let mut style = Style::default()
            .fg(if open { t.ink } else { t.muted })
            .bg(fill);
        if open {
            style = style.add_modifier(Modifier::BOLD);
        }
        let name_width = display_width(name);
        put_text(buffer, x + 2, y, name_width, name, style);
        if let Some(color) = t.state(tab.agent_status) {
            put_text(
                buffer,
                x + 2 + name_width + 1,
                y,
                1,
                "■",
                Style::default().fg(t.mark(color, fill)).bg(fill),
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
        put_text(
            buffer,
            x,
            y,
            w,
            &more,
            Style::default().fg(t.muted).bg(t.bg),
        );
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

/// A tab nobody named is called after what its agent is doing.
fn tab_label(tab: &ClientShellTab, task: Option<&str>) -> String {
    // herdr names an unnamed tab after its folder or program; the chat
    // title the agent set says more
    let task = if tab.custom_label {
        None
    } else {
        task.map(str::to_owned)
    };
    let mut name = task.unwrap_or_else(|| {
        if tab.label.trim().is_empty() {
            tab.number.to_string()
        } else {
            tab.label.clone()
        }
    });
    clip_in_place(&mut name, 22);
    name
}

/// `brutal::clip` on a string the caller owns: cut to `width` columns with
/// an ellipsis, without a second string.
fn clip_in_place(text: &mut String, width: usize) {
    if UnicodeWidthStr::width(text.as_str()) <= width {
        return;
    }
    let mut used = 0;
    let mut end = 0;
    for (index, symbol) in text.grapheme_indices(true) {
        let w = UnicodeWidthStr::width(symbol);
        if used + w + 1 > width {
            break;
        }
        used += w;
        end = index + symbol.len();
    }
    text.truncate(end);
    text.push('…');
}

/// Writes `symbol` into a cell, reusing the cell's own string.
fn set_symbol(cell: &mut crate::protocol::CellData, symbol: &str) {
    if cell.symbol != symbol {
        cell.symbol.clear();
        cell.symbol.push_str(symbol);
    }
}

/// The paper colors on a cell that keeps the terminal's defaults.
fn paper_cell(cell: &mut crate::protocol::CellData, reset: u32, bg: u32, fg: u32) {
    if cell.bg == reset {
        cell.bg = bg;
    }
    if cell.fg == reset {
        cell.fg = fg;
    }
}

/// Paints the panes inside the frame after the server's surface is blitted:
/// default-colored cells get the style's paper colors, borders the line
/// color, and a bordered pane's top row becomes a title bar with its
/// agent's state at the end ("✳ OPUS 5.5 · dark mode   running"): bright in
/// the agent's color for the focused pane, quiet for the others.
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
    let (end_x, end_y) = (
        surface.right().min(frame.width),
        surface.bottom().min(frame.height),
    );
    for y in surface.y..end_y {
        let row = usize::from(y) * width;
        let mut x = surface.x;
        // a row goes in runs of cells with one owner: the first pane listed
        // that holds the cell (or none). A run ends at its pane's right edge
        // or where a pane listed before it starts on this row, so the owner
        // is looked up once per run, not once per cell.
        while x < end_x {
            let index = panes
                .iter()
                .position(|pane| inside(pane.rect, x, y))
                .unwrap_or(panes.len());
            let (earlier, rest) = panes.split_at(index);
            let owner = rest.first();
            let mut end = owner.map_or(end_x, |pane| pane.rect.right().min(end_x));
            for pane in earlier {
                if pane.rect.x > x && pane.rect.x < end && inside(pane.rect, pane.rect.x, y) {
                    end = pane.rect.x;
                }
            }
            for cx in x..end {
                let Some(cell) = frame.cells.get_mut(row + usize::from(cx)) else {
                    continue;
                };
                match owner {
                    Some(pane) if inside(pane.inner_rect, cx, y) => {
                        if paper {
                            paper_cell(cell, reset, pane_bg, pane_fg);
                        }
                    }
                    Some(pane) => {
                        // the pane's lines run along the very edge of its
                        // own cells, on its paper: square corners, no gap
                        let r = pane.rect;
                        let (left, right, bottom) =
                            (cx == r.x, cx + 1 == r.right(), y + 1 == r.bottom());
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
                                set_symbol(cell, glyph);
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
            x = end;
        }
    }

    // a title bar only on a bordered pane with room for one
    let titled = |pane: &&PaneHit| pane.inner_rect.y > pane.rect.y && pane.rect.width >= 6;
    // each titled pane's agent and pane facts (the first listed of each),
    // found in one pass over the snapshot
    let mut facts: HashMap<&str, (Option<&ClientShellAgent>, Option<&ClientShellPane>)> = panes
        .iter()
        .filter(titled)
        .map(|pane| (pane.pane_id.as_str(), (None, None)))
        .collect();
    if facts.is_empty() {
        return;
    }
    for agent in &snapshot.agents {
        if let Some((slot, _)) = facts.get_mut(agent.pane_id.as_str()) {
            slot.get_or_insert(agent);
        }
    }
    for info in &snapshot.panes {
        if let Some((_, slot)) = facts.get_mut(info.pane_id.as_str()) {
            slot.get_or_insert(info);
        }
    }
    let edge_fg = color_to_u32(t.line);
    let (plain, bold) = (
        modifier_to_u16(Modifier::empty()),
        modifier_to_u16(Modifier::BOLD),
    );
    // one string for every title's name and task, built in place
    let mut name = String::new();
    for pane in panes.iter().filter(titled) {
        let (agent, pane_info) = facts
            .get(pane.pane_id.as_str())
            .copied()
            .unwrap_or_default();
        let y = usize::from(pane.rect.y);
        let Some(cells) = frame.cells.get_mut(y * width..(y + 1) * width) else {
            continue;
        };
        let focused = snapshot.focused_pane_id.as_deref() == Some(pane.pane_id.as_str());
        let brand = agent.and_then(|agent| t.brand(agent.agent.as_deref()));
        // the focused pane's bar is bright (its agent's color; a shell's in
        // ink), the others are quiet
        let (bar, text) = match (focused, brand) {
            (true, Some(brand)) => (brand, t.chip_text),
            (true, None) => (t.ink, t.card),
            (false, _) => (t.rail_ahead, t.ink),
        };
        let modifier = if focused { bold } else { plain };
        // the logo in the agent's state color on a quiet bar; the state at
        // the bar's right end, in the sidebar's words and colors
        let (logo, logo_fg, state) = match agent {
            Some(agent) => {
                let color = t.state(agent.agent_status).unwrap_or(t.muted);
                let logo = brutal::agent_logo(agent.agent.as_deref(), config.icon_font);
                let logo_fg = if focused { text } else { t.mark(color, bar) };
                let word_fg = if !focused {
                    t.text(color, bar)
                } else if reads(&t, color, bar) {
                    color
                } else {
                    text
                };
                let blocked = agent.agent_status == crate::api::schema::AgentStatus::Blocked;
                let word_modifier = if focused || blocked { bold } else { plain };
                (
                    logo,
                    logo_fg,
                    Some((state_word(agent.agent_status), word_fg, word_modifier)),
                )
            }
            None => ("▶", text, None),
        };
        name.clear();
        name.push(' ');
        match agent {
            Some(agent) => {
                name.extend(
                    brutal::agent_label(agent)
                        .chars()
                        .flat_map(char::to_uppercase),
                );
                let task = brutal::agent_task(agent);
                if !task.is_empty() {
                    name.push_str(" · ");
                    name.push_str(&task);
                }
            }
            None => push_pane_name(&mut name, pane_info),
        }
        let (first_x, last_x) = (pane.rect.x, pane.rect.right().saturating_sub(1));
        // the cells between the side lines: " logo name · task  state "
        let inner = last_x.saturating_sub(first_x + 1);
        let lead = 1 + display_width(logo);
        // the state word keeps its room (a space either side); the task,
        // then the name, give way. Too narrow for it: no word at all.
        let state = state.filter(|(word, _, _)| lead + display_width(word) + 2 <= inner);
        let room =
            inner.saturating_sub(lead + state.map_or(0, |(word, _, _)| display_width(word) + 2));
        if room == 0 {
            name.clear();
        } else {
            clip_in_place(&mut name, usize::from(room));
        }
        let (bar, text) = (color_to_u32(bar), color_to_u32(text));
        let mut row = TitleRow {
            cells,
            end: last_x,
            bg: bar,
        };
        // the pane's side lines run up its title bar
        row.edge(first_x, "▏", edge_fg, modifier);
        row.edge(last_x, "▕", edge_fg, modifier);
        let mut x = row.put(first_x + 1, " ", text, modifier);
        x = row.put(x, logo, color_to_u32(logo_fg), modifier);
        x = row.put(x, &name, text, modifier);
        if let Some((word, word_fg, word_modifier)) = state {
            let word_x = last_x.saturating_sub(display_width(word) + 1);
            x = row.fill(x, word_x, text, modifier);
            x = row.put(x, word, color_to_u32(word_fg), word_modifier);
        }
        row.fill(x, last_x, text, modifier);
    }
}

/// The sidebar's word for an agent's state.
fn state_word(status: crate::api::schema::AgentStatus) -> &'static str {
    use crate::api::schema::AgentStatus;
    match status {
        AgentStatus::Blocked => "needs you",
        AgentStatus::Working => "running",
        AgentStatus::Done => "done",
        _ => "idle",
    }
}

/// `fg` reads on `bg` as it is (4.5:1). `Brutal::text` leaves a color that
/// reads on a light fill unchanged; contrast goes both ways, so a light
/// `fg` tests a dark fill. Two dark colors count as not reading.
fn reads(t: &Brutal, fg: ratatui::style::Color, bg: ratatui::style::Color) -> bool {
    if brutal::is_light(bg) {
        t.text(fg, bg) == fg
    } else if brutal::is_light(fg) {
        t.text(bg, fg) == bg
    } else {
        false
    }
}

/// A pane's title row in the frame: text goes in up to `end` (its right
/// side line) on the bar's color.
struct TitleRow<'a> {
    cells: &'a mut [crate::protocol::CellData],
    end: u16,
    bg: u32,
}

impl TitleRow<'_> {
    fn set(&mut self, x: u16, symbol: &str, fg: u32, modifier: u16, skip: bool) {
        if let Some(cell) = self.cells.get_mut(usize::from(x)) {
            set_symbol(cell, symbol);
            cell.fg = fg;
            cell.bg = self.bg;
            cell.modifier = modifier;
            cell.skip = skip;
            cell.hyperlink = None;
        }
    }

    fn edge(&mut self, x: u16, symbol: &str, fg: u32, modifier: u16) {
        self.set(x, symbol, fg, modifier, false);
    }

    /// Writes `text` from `x`; returns the column after it. A wide glyph
    /// without room for both its cells is left out.
    fn put(&mut self, mut x: u16, text: &str, fg: u32, modifier: u16) -> u16 {
        for symbol in text.graphemes(true) {
            let width = display_width(symbol);
            if width == 0 {
                continue;
            }
            if x.saturating_add(width) > self.end {
                break;
            }
            self.set(x, symbol, fg, modifier, false);
            x += 1;
            for _ in 1..width {
                self.set(x, "", fg, modifier, true);
                x += 1;
            }
        }
        x
    }

    /// Blank bar from `x` up to `to`.
    fn fill(&mut self, mut x: u16, to: u16, fg: u32, modifier: u16) -> u16 {
        while x < to.min(self.end) {
            self.set(x, " ", fg, modifier, false);
            x += 1;
        }
        x
    }
}

/// A shell pane's name: its label, else its folder's name, else "shell".
fn push_pane_name(title: &mut String, pane: Option<&ClientShellPane>) {
    if let Some(label) = pane.and_then(|pane| pane.label.as_deref()) {
        title.push_str(label);
        return;
    }
    match pane.and_then(|pane| pane.foreground_cwd.as_deref().or(pane.cwd.as_deref())) {
        Some(cwd) => match std::path::Path::new(cwd).file_name() {
            Some(name) => title.push_str(&name.to_string_lossy()),
            None => title.push_str(cwd),
        },
        None => title.push_str("shell"),
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
        paper_cell(cell, reset, pane_bg, pane_fg);
    }
}
