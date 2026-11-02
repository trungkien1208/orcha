//! Shared popup geometry, choice rows, buttons and scrolling.

use super::*;

pub(in crate::client::shell) struct Frame {
    pub(in crate::client::shell) body: Rect,
}

/// The smallest popup box; on a screen with no room for it nothing is drawn.
pub(super) const MIN_POPUP_WIDTH: u16 = 30;
pub(super) const MIN_POPUP_HEIGHT: u16 = 8;
/// Columns and rows of page kept around the box.
pub(super) const POPUP_MARGIN_X: u16 = 4;
pub(super) const POPUP_MARGIN_Y: u16 = 2;

/// The box a popup gets on `screen`, or None when it does not fit.
pub(super) fn popup_box(screen: Rect, width: u16, body_height: u16) -> Option<(u16, u16)> {
    let width = width.min(screen.width.saturating_sub(POPUP_MARGIN_X));
    let height = body_height
        .saturating_add(6)
        .min(screen.height.saturating_sub(POPUP_MARGIN_Y));
    (width >= MIN_POPUP_WIDTH && height >= MIN_POPUP_HEIGHT).then_some((width, height))
}

/// True when a popup can be drawn on a screen of `size` (cols, rows), the
/// last one composed. Keys that act are ignored when it can't, since
/// nothing shows; None (not composed yet) counts as room. Holds for every
/// popup at least `MIN_POPUP_WIDTH` wide with two body rows.
pub(in crate::client::shell) fn popup_fits_screen(size: Option<(u16, u16)>) -> bool {
    size.is_none_or(|(cols, rows)| {
        popup_box(Rect::new(0, 0, cols, rows), MIN_POPUP_WIDTH, 2).is_some()
    })
}

/// The template's shell: dims the page, draws the box, header and footer.
pub(in crate::client::shell) fn popup_frame(
    buffer: &mut Buffer,
    width: u16,
    body_height: u16,
    title: &str,
    context: &str,
    keys: &[(&str, &str)],
    extra: &str,
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<Frame> {
    let screen = buffer.area;
    for y in screen.y..screen.bottom() {
        for x in screen.x..screen.right() {
            let cell = &mut buffer[(x, y)];
            cell.set_style(cell.style().add_modifier(Modifier::DIM));
        }
    }
    let (width, height) = popup_box(screen, width, body_height)?;
    let area = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height - height) / 2,
        width,
        height,
    );
    brutal::draw_box(buffer, area, t.line, t.bg, t.bg);
    brutal::draw_box_shadow(buffer, area, t.shadow, t.bg);
    hits.area = area;
    let inner_x = area.x + 1;
    let inner_w = area.width.saturating_sub(2);
    let line = Style::default().fg(t.line).bg(t.bg);

    // header on card, a heavy rule under it
    let header_y = area.y + 1;
    buffer.set_style(
        Rect::new(inner_x, header_y, inner_w, 1),
        Style::default().bg(t.card),
    );
    let title_width = display_width(title);
    put_text(
        buffer,
        inner_x + 1,
        header_y,
        title_width,
        title,
        Style::default()
            .fg(t.ink)
            .bg(t.card)
            .add_modifier(Modifier::BOLD),
    );
    let context_x = inner_x + 1 + title_width + 2;
    let context_width = (inner_x + inner_w).saturating_sub(context_x + 5);
    put_text(
        buffer,
        context_x,
        header_y,
        context_width,
        &brutal::clip(context, usize::from(context_width)),
        Style::default().fg(t.muted).bg(t.card),
    );
    let close = Rect::new((inner_x + inner_w).saturating_sub(4), header_y, 3, 1);
    put_text(
        buffer,
        close.x,
        header_y,
        3,
        " ✕ ",
        Style::default()
            .fg(t.ink)
            .bg(t.frame)
            .add_modifier(Modifier::BOLD),
    );
    hits.close = close;
    let rule = |buffer: &mut Buffer, y: u16| {
        put_text(
            buffer,
            inner_x,
            y,
            inner_w,
            &"━".repeat(usize::from(inner_w)),
            line,
        );
    };
    rule(buffer, header_y + 1);

    // footer: the keys as chips on card, a heavy rule above
    let footer_y = area.bottom() - 2;
    rule(buffer, footer_y - 1);
    buffer.set_style(
        Rect::new(inner_x, footer_y, inner_w, 1),
        Style::default().bg(t.card),
    );
    let mut x = inner_x + 1;
    // the chips stop short of the extra on the right
    let keys_right = if extra.is_empty() {
        inner_x + inner_w
    } else {
        (inner_x + inner_w).saturating_sub(display_width(extra) + 2)
    };
    for (key, what) in keys {
        let key_width = display_width(key) + 2;
        let what_width = display_width(what);
        if x + key_width + what_width + 3 > keys_right {
            break;
        }
        brutal::chip(buffer, x, footer_y, key_width, key, t.ink, t.frame);
        put_text(
            buffer,
            x + key_width + 1,
            footer_y,
            what_width,
            what,
            Style::default().fg(t.muted).bg(t.card),
        );
        x += key_width + what_width + 3;
    }
    if !extra.is_empty() {
        put_right_text(
            buffer,
            Rect::new(inner_x, footer_y, inner_w.saturating_sub(1), 1),
            footer_y,
            extra,
            Style::default().fg(t.muted).bg(t.card),
        );
        let width = display_width(extra);
        hits.extra = Rect::new(
            (inner_x + inner_w.saturating_sub(1)).saturating_sub(width),
            footer_y,
            width,
            1,
        );
    }
    Some(Frame {
        body: Rect::new(
            inner_x,
            header_y + 2,
            inner_w,
            footer_y.saturating_sub(header_y + 3),
        ),
    })
}

/// A body row: a flat band across the popup. The keyboard cursor's band
/// is on card with a yellow bar on the left (like the selected project);
/// rows after the first get a thin rule above them.
pub(in crate::client::shell) fn row_box(
    buffer: &mut Buffer,
    rect: Rect,
    first: bool,
    cursor: bool,
    t: &Brutal,
) -> ratatui::style::Color {
    let fill = if cursor { t.card } else { t.bg };
    buffer.set_style(rect, Style::default().bg(fill));
    if !first && rect.y > 0 {
        let y = rect.y - 1;
        put_text(
            buffer,
            rect.x + 1,
            y,
            rect.width.saturating_sub(2),
            &"─".repeat(usize::from(rect.width.saturating_sub(2))),
            Style::default().fg(t.empty).bg(t.bg),
        );
    }
    if cursor {
        for y in rect.y..rect.bottom() {
            put_text(
                buffer,
                rect.x,
                y,
                1,
                "▌",
                Style::default().fg(t.mark(t.yellow, fill)).bg(fill),
            );
        }
    }
    fill
}

/// A button: the primary one raised green, the others outlined, a
/// disabled one muted.
pub(in crate::client::shell) fn button(
    buffer: &mut Buffer,
    right: u16,
    y: u16,
    label: &str,
    kind: ButtonKind,
    fill: ratatui::style::Color,
    t: &Brutal,
) -> Rect {
    let text = format!(" {label} ");
    let width = display_width(&text);
    let x = right.saturating_sub(width + 1);
    let style = match kind {
        ButtonKind::Primary => Style::default()
            .fg(t.chip_text)
            .bg(t.green)
            .add_modifier(Modifier::BOLD),
        ButtonKind::Plain => Style::default()
            .fg(t.ink)
            .bg(t.frame)
            .add_modifier(Modifier::BOLD),
    };
    put_text(buffer, x, y, width, &text, style);
    if kind == ButtonKind::Primary {
        put_text(
            buffer,
            x + width,
            y,
            1,
            "▌",
            Style::default().fg(t.shadow).bg(fill),
        );
    }
    Rect::new(x, y, width, 1)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::client::shell) enum ButtonKind {
    Primary,
    Plain,
}

/// Name, gauges, then when each window resets. A thin rule sits between rows.
pub(super) const ROW: u16 = 3;

pub(super) fn rows_height(rows: usize) -> u16 {
    (rows.max(1) as u16) * (ROW + 1) - 1
}

/// How many rows `height` lines tall, a gap between them, fit in `lines`.
pub(super) fn rows_fitting(lines: u16, height: u16) -> usize {
    usize::from(lines.saturating_add(1) / height.saturating_add(1))
}

/// Which rows of a list show in `area`: the rect they go in, the first
/// index and one past the last. All of them when they fit. Else a line is
/// kept at the bottom for "↓ more", and at the top for "↑ more" once the
/// cursor is past the first screenful; the window then ends at the cursor.
pub(in crate::client::shell) fn list_window(
    area: Rect,
    height: u16,
    len: usize,
    cursor: usize,
) -> (Rect, usize, usize) {
    if rows_fitting(area.height, height) >= len {
        return (area, 0, len);
    }
    let cursor = cursor.min(len.saturating_sub(1));
    let below = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1));
    let fit = rows_fitting(below.height, height);
    if cursor < fit {
        return (below, 0, fit);
    }
    let both = Rect::new(
        area.x,
        area.y.saturating_add(1),
        area.width,
        area.height.saturating_sub(2),
    );
    let fit = rows_fitting(both.height, height);
    let start = (cursor + 1).saturating_sub(fit).min(len - fit.min(len));
    (both, start, (start + fit).min(len))
}

/// Draws a list of `len` rows, `height` lines each with a gap, in `area`,
/// scrolled so the cursor's row shows (see `list_window`). `draw` gets each
/// shown row's rect, its index and whether it is the first one shown; only
/// the shown rows draw, so their click targets match the screen.
pub(super) fn draw_rows(
    buffer: &mut Buffer,
    area: Rect,
    height: u16,
    len: usize,
    cursor: usize,
    t: &Brutal,
    mut draw: impl FnMut(&mut Buffer, Rect, usize, bool),
) {
    let (list, start, end) = list_window(area, height, len, cursor);
    let hint = |buffer: &mut Buffer, y: u16, text: &str| {
        put_right_text(
            buffer,
            Rect::new(area.x, y, area.width.saturating_sub(2), 1),
            y,
            text,
            Style::default().fg(t.muted).bg(t.bg),
        );
    };
    if area.height > 0 && start > 0 {
        hint(buffer, area.y, " ↑ more ");
    }
    if area.height > 0 && end < len {
        hint(buffer, area.bottom() - 1, " ↓ more ");
    }
    let mut y = list.y;
    for (offset, index) in (start..end).enumerate() {
        draw(
            buffer,
            Rect::new(list.x, y, list.width, height),
            index,
            offset == 0,
        );
        y = y.saturating_add(height + 1);
    }
}

/// What sits before a choice's title.
#[derive(Clone, Copy)]
pub(super) enum Lead<'a> {
    None,
    /// A check box; clicking it is the row's action.
    Check(bool),
    /// A logo chip on its color (an agent, a shell).
    Logo(&'a str, ratatui::style::Color),
}

/// A choice's button on the right, and when it shows.
#[derive(Clone, Copy)]
pub(super) enum RowButton<'a> {
    None,
    /// On every row; raised green on the cursor's.
    Always(&'a str),
    /// Only on the cursor's row.
    OnCursor(&'a str),
}

/// The title's color.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tone {
    Ink,
    /// Red: the choice removes something.
    Danger,
    /// Muted: the choice can't be picked, or goes nowhere new.
    Muted,
}

/// One choice in a popup list: a bold title, a muted detail line under it
/// (rows taller than one line), a button on the right.
pub(super) struct ChoiceRow<'a> {
    pub(super) title: &'a str,
    pub(super) detail: &'a str,
    pub(super) button: RowButton<'a>,
    pub(super) lead: Lead<'a>,
    pub(super) tone: Tone,
    pub(super) first: bool,
    pub(super) cursor: bool,
}

impl<'a> ChoiceRow<'a> {
    pub(super) fn new(title: &'a str, detail: &'a str, button: RowButton<'a>) -> Self {
        Self {
            title,
            detail,
            button,
            lead: Lead::None,
            tone: Tone::Ink,
            first: false,
            cursor: false,
        }
    }

    /// Its place in the list.
    pub(super) fn at(self, first: bool, cursor: bool) -> Self {
        Self {
            first,
            cursor,
            ..self
        }
    }
}

/// Draws one choice row in `rect` and records its click targets.
pub(super) fn choice_row(
    buffer: &mut Buffer,
    rect: Rect,
    index: usize,
    row: ChoiceRow<'_>,
    t: &Brutal,
    hits: &mut PopupHits,
) {
    let fill = row_box(buffer, rect, row.first, row.cursor, t);
    let y = rect.y;
    let mut x = rect.x + 2;
    match row.lead {
        Lead::None => {}
        Lead::Check(on) => {
            let width = brutal::chip(
                buffer,
                x,
                y,
                5,
                if on { "✓" } else { " " },
                t.chip_text,
                if on { t.green } else { t.frame },
            );
            hits.actions.push((Rect::new(x, y, width, 1), index));
            x += width + 1;
        }
        Lead::Logo(logo, color) => {
            x += brutal::chip(buffer, x, y, 4, logo, t.chip_text, color) + 1;
        }
    }
    // the text stops a cell short of the button, shown or not, so a quiet
    // row does not reflow when the cursor reaches it
    let label = match row.button {
        RowButton::None => None,
        RowButton::Always(label) | RowButton::OnCursor(label) => Some(label),
    };
    let text_right = match label {
        Some(label) => rect.right().saturating_sub(display_width(label) + 5),
        None => rect.right().saturating_sub(2),
    };
    let width = text_right.saturating_sub(x);
    let title_fg = match row.tone {
        Tone::Ink => t.ink,
        Tone::Danger => t.text(t.red, fill),
        Tone::Muted => t.muted,
    };
    put_text(
        buffer,
        x,
        y,
        width,
        &brutal::clip(row.title, usize::from(width)),
        Style::default()
            .fg(title_fg)
            .bg(fill)
            .add_modifier(Modifier::BOLD),
    );
    if rect.height > 1 && !row.detail.is_empty() {
        put_text(
            buffer,
            x,
            y + 1,
            width,
            &brutal::clip(row.detail, usize::from(width)),
            Style::default().fg(t.muted).bg(fill),
        );
    }
    let shown = match row.button {
        RowButton::Always(label) => Some(label),
        RowButton::OnCursor(label) if row.cursor => Some(label),
        _ => None,
    };
    if let Some(label) = shown {
        let kind = if row.cursor {
            ButtonKind::Primary
        } else {
            ButtonKind::Plain
        };
        let action = button(buffer, rect.right() - 1, y, label, kind, fill, t);
        hits.actions.push((action, index));
    }
    hits.rows.push((rect, index));
}
