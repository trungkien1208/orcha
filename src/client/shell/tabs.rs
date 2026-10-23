use super::*;

const TAB_SCROLL_BUTTON_WIDTH: u16 = 3;
const MIN_TAB_STRIP_WIDTH: u16 =
    MIN_TAB_WIDTH + NEW_TAB_WIDTH + TAB_SCROLL_BUTTON_WIDTH.saturating_mul(2);

pub(crate) fn render_tab_bar(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    tab_scroll: &mut usize,
    reveal_focused_tab: &mut bool,
    tab_drag_insert_index: Option<usize>,
    hits: &mut ShellHitMap,
) {
    let palette = &config.palette;
    // orcha: header, sidebars and status bar share one chrome color
    let chrome = if config.status_bar {
        palette.sidebar_bg
    } else {
        palette.panel_bg
    };
    buffer.set_style(area, Style::default().bg(chrome));
    let tabs = snapshot
        .tabs
        .iter()
        .filter(|tab| Some(tab.workspace_id.as_str()) == snapshot.focused_workspace_id.as_deref())
        .collect::<Vec<_>>();
    let desired_widths = tabs
        .iter()
        .map(|tab| {
            let label = tab_label(tab);
            display_width(&label).saturating_add(4).max(MIN_TAB_WIDTH)
        })
        .collect::<Vec<_>>();
    // orcha: with the status bar on, its entries live there and the tabs get the whole row
    let content = if config.status_bar {
        area
    } else {
        tab_bar_content_area(snapshot, area)
    };
    let mouse_chrome = config.mouse_capture;
    let new_tab_width = if mouse_chrome { NEW_TAB_WIDTH } else { 0 };
    let desired_total = desired_widths
        .iter()
        .copied()
        .fold(0_u16, u16::saturating_add)
        .saturating_add(tabs.len().saturating_sub(1).min(u16::MAX as usize) as u16)
        .saturating_add(new_tab_width);
    let overflow =
        desired_total > content.width && (!mouse_chrome || content.width >= MIN_TAB_STRIP_WIDTH);
    let available = if overflow && mouse_chrome {
        content
            .width
            .saturating_sub(NEW_TAB_WIDTH)
            .saturating_sub(TAB_SCROLL_BUTTON_WIDTH.saturating_mul(2))
    } else {
        content.width.saturating_sub(new_tab_width)
    };
    let max_scroll = max_tab_scroll(&desired_widths, available);
    if !overflow {
        *tab_scroll = 0;
    } else if *reveal_focused_tab {
        if let Some(focused) = tabs.iter().position(|tab| tab.focused) {
            *tab_scroll = centered_tab_scroll(focused, &desired_widths, available).min(max_scroll);
        }
    } else {
        *tab_scroll = (*tab_scroll).min(max_scroll);
    }
    *reveal_focused_tab = false;

    let mut x = content.x;
    let tab_right = if overflow && mouse_chrome {
        hits.tab_scroll_left = Rect::new(
            content.x,
            content.y,
            TAB_SCROLL_BUTTON_WIDTH.min(content.width),
            1,
        );
        put_text(
            buffer,
            hits.tab_scroll_left.x,
            content.y,
            hits.tab_scroll_left.width,
            " < ",
            Style::default()
                .fg(if *tab_scroll > 0 {
                    palette.overlay1
                } else {
                    palette.overlay0
                })
                .bg(palette.surface0),
        );
        x = hits.tab_scroll_left.right();
        content
            .right()
            .saturating_sub(NEW_TAB_WIDTH + TAB_SCROLL_BUTTON_WIDTH)
    } else {
        content.right().saturating_sub(new_tab_width)
    };

    let mut first_visible = None;
    let mut last_visible = None;
    for (index, tab) in tabs.iter().enumerate().skip(*tab_scroll) {
        let name = tab_label(tab);
        let desired = desired_widths[index];
        let remaining = tab_right.saturating_sub(x);
        let width = desired.min(remaining);
        if width == 0 {
            break;
        }
        let rect = Rect::new(x, area.y, width, 1);
        let style = if tab.focused {
            let base = Style::default()
                .fg(panel_contrast_fg(palette))
                .bg(palette.accent);
            if tab.custom_label {
                base.add_modifier(Modifier::BOLD)
            } else {
                base
            }
        } else if tab.custom_label {
            Style::default().fg(palette.overlay1).bg(palette.surface0)
        } else {
            Style::default().fg(palette.overlay0).bg(palette.surface0)
        };
        let padding = width.saturating_sub(display_width(&name));
        let left = padding / 2;
        let text = format!(
            "{empty:left$}{name}{empty:right_padding$}",
            empty = "",
            left = left as usize,
            right_padding = padding.saturating_sub(left) as usize,
        );
        put_text(buffer, rect.x, rect.y, rect.width, &text, style);
        hits.tabs.push((rect, tab.tab_id.clone()));
        first_visible.get_or_insert(index);
        last_visible = Some(index);
        x = x.saturating_add(width + 1);
        if width < desired {
            break;
        }
    }

    if overflow && mouse_chrome {
        hits.tab_scroll_right = Rect::new(tab_right, area.y, TAB_SCROLL_BUTTON_WIDTH, 1);
        let can_scroll_right = *tab_scroll < max_scroll;
        put_text(
            buffer,
            hits.tab_scroll_right.x,
            area.y,
            hits.tab_scroll_right.width,
            " > ",
            Style::default()
                .fg(if can_scroll_right {
                    palette.overlay1
                } else {
                    palette.overlay0
                })
                .bg(palette.surface0),
        );
        hits.new_tab = Rect::new(
            hits.tab_scroll_right.right(),
            area.y,
            content
                .right()
                .saturating_sub(hits.tab_scroll_right.right())
                .min(NEW_TAB_WIDTH),
            1,
        );
    } else if mouse_chrome {
        hits.new_tab = Rect::new(
            x.min(content.right()),
            area.y,
            content.right().saturating_sub(x).min(NEW_TAB_WIDTH),
            1,
        );
    }
    if mouse_chrome {
        put_text(
            buffer,
            hits.new_tab.x,
            area.y,
            hits.new_tab.width,
            " + ",
            Style::default().fg(palette.overlay1).bg(palette.panel_bg),
        );
    }

    if first_visible.is_some_and(|index| index > 0) {
        let ellipsis_x = if hits.tab_scroll_left.width > 0 {
            hits.tab_scroll_left.right()
        } else {
            content.x
        };
        put_text(
            buffer,
            ellipsis_x,
            area.y,
            u16::from(ellipsis_x < content.right()),
            "…",
            Style::default().fg(palette.overlay0),
        );
    }
    if last_visible.is_some_and(|index| index + 1 < tabs.len()) {
        let ellipsis_x = if hits.tab_scroll_right.width > 0 {
            hits.tab_scroll_right.x.saturating_sub(1)
        } else {
            content.right().saturating_sub(1)
        };
        put_text(
            buffer,
            ellipsis_x,
            area.y,
            u16::from(ellipsis_x >= content.x && ellipsis_x < content.right()),
            "…",
            Style::default().fg(palette.overlay0),
        );
    }

    if let Some(insert_index) = tab_drag_insert_index {
        if let Some(indicator_x) = tab_drop_indicator_x(hits, &tabs, insert_index) {
            put_text(
                buffer,
                indicator_x.min(content.right().saturating_sub(1)),
                area.y,
                1,
                "│",
                Style::default().fg(palette.accent),
            );
        }
    }
    if !config.status_bar {
        render_tab_bar_status(buffer, area, snapshot, palette);
    }
}

/// orcha: the `tab_bar_right` entries in a full-width black bar at the
/// bottom. What you are working on sits on the left (the task with its `#id`
/// as a pink chip, its state after `●` in green, changed files muted); the
/// ambient facts (timers, clock) sit on the right, the clock in bold. FUEL
/// joins them when it is placed in the status bar.
pub(crate) fn render_status_bar(
    buffer: &mut Buffer,
    outer: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    fuel: Option<&super::super::accounts::Account>,
    hits: &mut ShellHitMap,
) {
    let t = super::super::brutal::Brutal::new(&config.palette);
    let bg = t.status_bg;
    // the margin around it is page
    let page = Rect::new(
        0,
        outer.y,
        buffer.area.width,
        buffer.area.bottom() - outer.y,
    )
    .intersection(buffer.area);
    buffer.set_style(page, Style::default().bg(t.bg));
    let area = Rect::new(outer.x, outer.y + outer.height / 2, outer.width, 1);
    buffer.set_style(area, Style::default().bg(bg).fg(t.status_text));
    let mut parts: Vec<&str> = Vec::new();
    // plan usage printed by a status script (an account, "5h ███ 40%"): orcha
    // draws FUEL itself, in the panel or here
    let usage = |part: &str| {
        let lower = part.to_lowercase();
        part.contains('@')
            || part.contains('█')
            || part.contains('░')
            || lower.starts_with("5h")
            || lower.starts_with("week")
    };
    for segment in &snapshot.tab_bar_right {
        for part in segment.text.split(" · ") {
            let part = part.trim();
            if !part.is_empty() && !usage(part) {
                parts.push(part);
            }
        }
    }
    let is_left =
        |index: usize, part: &str| index == 0 && part.contains('#') || part.starts_with('✎');
    let left: Vec<&str> = parts
        .iter()
        .enumerate()
        .filter(|(i, p)| is_left(*i, p))
        .map(|(_, p)| *p)
        .collect();
    let right: Vec<&str> = parts
        .iter()
        .enumerate()
        .filter(|(i, p)| !is_left(*i, p))
        .map(|(_, p)| *p)
        .collect();
    let text = Style::default().fg(t.status_text).bg(bg);
    let muted = Style::default().fg(t.muted).bg(bg);

    let mut x = area.x.saturating_add(2);
    for (index, part) in left.iter().enumerate() {
        if index > 0 {
            x = x.saturating_add(2);
        }
        let mut rest = *part;
        if let Some(hash) = part.find('#') {
            let id_end = part[hash + 1..]
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '-')
                .map_or(part.len(), |end| hash + 1 + end);
            let badge = format!(" {} ", &part[hash..id_end]);
            let width = display_width(&badge).min(area.right().saturating_sub(x));
            put_text(
                buffer,
                x,
                area.y,
                width,
                &badge,
                Style::default()
                    .fg(t.chip_text)
                    .bg(t.pink)
                    .add_modifier(Modifier::BOLD),
            );
            x = x.saturating_add(width).saturating_add(1);
            rest = part[id_end..].trim_start();
        }
        // "Title ● State": the title bold, the state green
        let (title, state) = match rest.split_once('●') {
            Some((title, state)) => (title.trim_end(), Some(state.trim())),
            None => (rest, None),
        };
        let edit = title.starts_with('✎');
        let style = if edit {
            muted
        } else {
            text.add_modifier(Modifier::BOLD)
        };
        x = put_segment(buffer, x, area.y, area.right(), title, style);
        if let Some(state) = state {
            x = put_segment(
                buffer,
                x.saturating_add(2),
                area.y,
                area.right(),
                &format!("● {}", state.to_lowercase()),
                Style::default().fg(t.green).bg(bg),
            );
        }
    }

    // right group, laid out from the right edge
    let fuel_text = fuel
        .filter(|_| config.orcha.fuel == crate::config::FuelPlacement::StatusBar)
        .map(|account| {
            let who = account
                .email
                .clone()
                .unwrap_or_else(|| account.label.clone());
            (
                who,
                account.windows.iter().take(2).cloned().collect::<Vec<_>>(),
            )
        });
    // "▴ FUEL email  5H ▬ ▬ … 18%  WEEK …"
    let fuel_width = fuel_text.as_ref().map_or(0, |(who, windows)| {
        8 + display_width(who)
            + 2
            + windows
                .iter()
                .map(|w| {
                    display_width(&w.label)
                        + 27
                        + w.reset
                            .as_ref()
                            .filter(|reset| !reset.is_empty())
                            .map(|reset| 3 + display_width(reset))
                            .unwrap_or(0)
                })
                .sum::<u16>()
    });
    let joined = right.join("  ");
    let width = display_width(&joined) + fuel_width;
    let start = area
        .right()
        .saturating_sub(width.saturating_add(2))
        .max(x.saturating_add(2));
    let mut cx = start;
    if let Some((who, windows)) = fuel_text {
        // ▴ puts FUEL back in the panel
        let show = Rect::new(cx, area.y, 6, 1);
        cx = put_segment(
            buffer,
            cx,
            area.y,
            area.right(),
            "▴ FUEL",
            Style::default()
                .fg(t.status_text)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        );
        hits.orcha.fuel_show = show;
        cx += 2;
        cx = put_segment(buffer, cx, area.y, area.right(), &who, muted);
        cx += 2;
        for window in windows {
            cx = put_segment(buffer, cx, area.y, area.right(), &window.label, muted);
            cx += 1;
            super::super::git_panel::draw_gauge(buffer, cx, area.y, window.left, bg, &t);
            cx += 20;
            let pct = format!("{:>3}%", window.left);
            cx = put_segment(
                buffer,
                cx,
                area.y,
                area.right(),
                &pct,
                Style::default()
                    .fg(t.text(super::super::git_panel::fuel_color(window.left, &t), bg))
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            );
            if let Some(reset) = window.reset.as_deref().filter(|reset| !reset.is_empty()) {
                cx += 1;
                cx = put_segment(
                    buffer,
                    cx,
                    area.y,
                    area.right(),
                    &format!("· {reset}"),
                    muted,
                );
            }
            cx += 1;
        }
    }
    for (index, part) in right.iter().enumerate() {
        if index > 0 {
            cx = cx.saturating_add(2);
        }
        let style = if index + 1 == right.len() {
            text.add_modifier(Modifier::BOLD)
        } else {
            muted
        };
        cx = put_segment(buffer, cx, area.y, area.right(), part, style);
    }
}

pub(crate) fn tab_bar_status_width(snapshot: &ClientShellSnapshot) -> u16 {
    let content = snapshot.tab_bar_right.iter().fold(0u16, |width, segment| {
        width.saturating_add(display_width(&segment.text))
    });
    let separators = snapshot.tab_bar_right.len().saturating_sub(1);
    content.saturating_add(
        display_width(&snapshot.tab_bar_right_separator)
            .saturating_mul(separators.min(u16::MAX as usize) as u16),
    )
}

fn tab_bar_status_area(snapshot: &ClientShellSnapshot, area: Rect) -> Option<Rect> {
    let width = tab_bar_status_width(snapshot);
    if width == 0 {
        return None;
    }
    let reserved = width.saturating_add(1);
    (area.width.saturating_sub(reserved) >= MIN_TAB_STRIP_WIDTH)
        .then(|| Rect::new(area.right().saturating_sub(width), area.y, width, 1))
}

fn tab_bar_content_area(snapshot: &ClientShellSnapshot, area: Rect) -> Rect {
    let reserved = tab_bar_status_area(snapshot, area)
        .map(|status| status.width.saturating_add(1))
        .unwrap_or(0);
    Rect {
        width: area.width.saturating_sub(reserved),
        ..area
    }
}

fn render_tab_bar_status(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    palette: &Palette,
) {
    let Some(status) = tab_bar_status_area(snapshot, area) else {
        return;
    };
    let separator_width = display_width(&snapshot.tab_bar_right_separator);
    let mut x = status.x;
    for (index, segment) in snapshot.tab_bar_right.iter().enumerate() {
        if index > 0 && separator_width > 0 {
            put_text(
                buffer,
                x,
                area.y,
                separator_width,
                &snapshot.tab_bar_right_separator,
                Style::default().fg(palette.overlay0).bg(palette.panel_bg),
            );
            x = x.saturating_add(separator_width);
        }
        let width = display_width(&segment.text);
        let style = if segment.accent {
            Style::default()
                .fg(panel_contrast_fg(palette))
                .bg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.overlay1).bg(palette.panel_bg)
        };
        put_text(buffer, x, area.y, width, &segment.text, style);
        x = x.saturating_add(width);
    }
}

fn tab_drop_indicator_x(
    hits: &ShellHitMap,
    tabs: &[&ClientShellTab],
    insert_index: usize,
) -> Option<u16> {
    let visible = hits
        .tabs
        .iter()
        .filter_map(|(rect, tab_id)| {
            tabs.iter()
                .position(|tab| tab.tab_id == *tab_id)
                .map(|index| (index, *rect))
        })
        .collect::<Vec<_>>();
    let (first_index, first_rect) = *visible.first()?;
    let (last_index, last_rect) = *visible.last()?;
    if insert_index == 0 {
        return Some(if first_index == 0 {
            first_rect.x
        } else {
            hits.tab_scroll_left.right()
        });
    }
    if let Some((_, rect)) = visible.iter().find(|(index, _)| *index == insert_index) {
        return Some(rect.x.saturating_sub(1));
    }
    if insert_index >= tabs.len() {
        return Some(if last_index + 1 >= tabs.len() {
            last_rect.right()
        } else {
            hits.tab_scroll_right.x.saturating_sub(1)
        });
    }
    None
}

fn centered_tab_scroll(focused: usize, widths: &[u16], available: u16) -> usize {
    let mut best = focused;
    let mut best_distance = u16::MAX;
    for start in 0..=focused {
        let before = widths
            .iter()
            .copied()
            .enumerate()
            .skip(start)
            .take(focused.saturating_sub(start))
            .fold(0u16, |width, (_, tab)| width.saturating_add(tab + 1));
        if before >= available {
            continue;
        }
        let focused_width = widths[focused].min(available.saturating_sub(before));
        let center = before.saturating_mul(2).saturating_add(focused_width);
        let distance = center.abs_diff(available);
        if distance <= best_distance {
            best_distance = distance;
            best = start;
        }
    }
    best
}

fn max_tab_scroll(widths: &[u16], available: u16) -> usize {
    let Some((&last, preceding)) = widths.split_last() else {
        return 0;
    };
    let mut start = preceding.len();
    let mut used = u32::from(last);
    // Keep the longest fully visible suffix, not merely a sliver of the last tab.
    // An oversized last tab must still be reachable at the start of the strip.
    for width in preceding.iter().rev() {
        let required = used + 1 + u32::from(*width);
        if required > u32::from(available) {
            break;
        }
        used = required;
        start -= 1;
    }
    start
}

fn tab_label(tab: &ClientShellTab) -> String {
    if tab.zoomed {
        format!("{} Z", tab.label)
    } else {
        tab.label.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::max_tab_scroll;

    #[test]
    fn trailing_scroll_limit_accounts_for_full_widths_and_separators() {
        for (widths, available, expected) in [
            (&[][..], 0, 0),
            (&[8, 13][..], 0, 1),
            (&[8, 13][..], 1, 1),
            (&[8, 13][..], 12, 1),
            (&[8, 13][..], 21, 1),
            (&[8, 13][..], 22, 0),
            (&[8, 13][..], 30, 0),
            (&[8, u16::MAX][..], u16::MAX, 1),
        ] {
            assert_eq!(
                max_tab_scroll(widths, available),
                expected,
                "widths={widths:?}, available={available}"
            );
        }
    }
}
