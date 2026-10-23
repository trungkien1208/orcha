use super::*;

fn choice_style(selected: bool, palette: &Palette) -> Style {
    if selected {
        Style::default()
            .fg(contrast(palette))
            .bg(palette.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(palette.text).bg(palette.panel_bg)
    }
}

fn draw_choice(
    buffer: &mut Buffer,
    rect: Rect,
    label: &str,
    selected: bool,
    current: bool,
    palette: &Palette,
) {
    if super::super::brutal::orcha_look() {
        draw_orcha_choice(buffer, rect, label, selected, current, palette);
        return;
    }
    let style = choice_style(selected, palette);
    buffer.set_style(rect, style);
    let marker = if selected { "▸" } else { " " };
    let current = if current { " ✓" } else { "" };
    put_text(
        buffer,
        rect.x,
        rect.y,
        rect.width,
        &format!(" {marker} {label}{current}"),
        style,
    );
}

/// A choice in the orcha look: the highlighted row on frame fill with the
/// yellow edge, the one in use marked with a green check.
fn draw_orcha_choice(
    buffer: &mut Buffer,
    rect: Rect,
    label: &str,
    selected: bool,
    current: bool,
    palette: &Palette,
) {
    let t = super::super::brutal::Brutal::new(palette);
    let fill = if selected { t.frame } else { palette.panel_bg };
    buffer.set_style(rect, Style::default().bg(fill));
    if selected {
        put_text(
            buffer,
            rect.x,
            rect.y,
            1,
            "▌",
            Style::default().fg(t.mark(t.yellow, fill)).bg(fill),
        );
    }
    let mut style = Style::default().fg(t.ink).bg(fill);
    if selected {
        style = style.add_modifier(Modifier::BOLD);
    }
    let width = display_width(label).min(rect.width.saturating_sub(3));
    put_text(buffer, rect.x + 2, rect.y, width, label, style);
    if current {
        put_text(
            buffer,
            rect.x + 3 + width,
            rect.y,
            8,
            "✓ in use",
            Style::default().fg(t.text(t.green, fill)).bg(fill),
        );
    }
}

/// The orcha look's frame: a header with the title and ✕, the sections as
/// a column on the left, a footer with the keys and the primary button.
/// Returns (content, section hits, primary, close).
fn render_orcha_frame(
    buffer: &mut Buffer,
    inner: Rect,
    settings: &ClientSettingsOverlay,
    integration_badge: bool,
    show_primary: bool,
    palette: &Palette,
) -> (Rect, Vec<(Rect, ClientSettingsSection)>, Rect, Rect) {
    use super::super::brutal;
    let t = brutal::Brutal::new(palette);
    let card = palette.panel_bg;
    let rule = |buffer: &mut Buffer, y: u16| {
        put_text(
            buffer,
            inner.x,
            y,
            inner.width,
            &"━".repeat(usize::from(inner.width)),
            Style::default().fg(t.line).bg(card),
        );
    };
    // header
    let header_y = inner.y;
    put_text(
        buffer,
        inner.x + 1,
        header_y,
        9,
        "SETTINGS",
        Style::default()
            .fg(t.ink)
            .bg(card)
            .add_modifier(Modifier::BOLD),
    );
    put_text(
        buffer,
        inner.x + 11,
        header_y,
        inner.width.saturating_sub(16),
        "orcha's look, apps and alerts",
        Style::default().fg(t.muted).bg(card),
    );
    let close = Rect::new(inner.right().saturating_sub(4), header_y, 3, 1);
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
    rule(buffer, header_y + 1);
    // footer
    let footer_y = inner.bottom().saturating_sub(1);
    rule(buffer, footer_y - 1);
    let mut x = inner.x + 1;
    for (key, what) in [("↑↓", "choose"), ("tab", "section"), ("esc", "close")] {
        let key_width = display_width(key) + 2;
        brutal::chip(buffer, x, footer_y, key_width, key, t.ink, t.frame);
        put_text(
            buffer,
            x + key_width + 1,
            footer_y,
            display_width(what),
            what,
            Style::default().fg(t.muted).bg(card),
        );
        x += key_width + display_width(what) + 3;
    }
    let primary = if show_primary {
        let label = if settings.section == ClientSettingsSection::Integrations {
            " INSTALL ↵ "
        } else {
            " APPLY ↵ "
        };
        let width = display_width(label);
        let rect = Rect::new(inner.right().saturating_sub(width + 1), footer_y, width, 1);
        put_text(
            buffer,
            rect.x,
            rect.y,
            width,
            label,
            Style::default()
                .fg(t.chip_text)
                .bg(t.green)
                .add_modifier(Modifier::BOLD),
        );
        rect
    } else {
        Rect::default()
    };
    // sections, a column on the left
    let nav_width = 16u16;
    let top = header_y + 3;
    let bottom = footer_y.saturating_sub(2);
    let count = ClientSettingsSection::ALL.len() as u16;
    let step = if count * 2 <= bottom.saturating_sub(top) {
        2
    } else {
        1
    };
    let mut hits = Vec::new();
    for (index, section) in ClientSettingsSection::ALL.iter().enumerate() {
        let y = top + index as u16 * step;
        if y >= bottom {
            break;
        }
        let rect = Rect::new(inner.x + 1, y, nav_width, 1);
        let active = *section == settings.section;
        let fill = if active { t.frame } else { card };
        buffer.set_style(rect, Style::default().bg(fill));
        if active {
            put_text(
                buffer,
                rect.x,
                y,
                1,
                "▌",
                Style::default().fg(t.mark(t.yellow, fill)).bg(fill),
            );
        }
        let mut style = Style::default()
            .fg(if active { t.ink } else { t.muted })
            .bg(fill);
        if active {
            style = style.add_modifier(Modifier::BOLD);
        }
        put_text(buffer, rect.x + 2, y, nav_width - 3, section.label(), style);
        if *section == ClientSettingsSection::Integrations && integration_badge {
            put_text(
                buffer,
                rect.right().saturating_sub(2),
                y,
                1,
                "●",
                Style::default().fg(t.text(t.red, fill)).bg(fill),
            );
        }
        hits.push((rect, *section));
    }
    let divider_x = inner.x + nav_width + 2;
    for y in header_y + 2..footer_y - 1 {
        put_text(
            buffer,
            divider_x,
            y,
            1,
            "│",
            Style::default().fg(t.rail_ahead).bg(card),
        );
    }
    let content = Rect::new(
        divider_x + 2,
        top,
        inner.right().saturating_sub(divider_x + 3),
        bottom.saturating_sub(top) + 1,
    );
    (content, hits, primary, close)
}

pub(super) fn render_settings_overlay(
    buffer: &mut Buffer,
    settings: &ClientSettingsOverlay,
    integration_updates_available: bool,
    palette: &Palette,
) -> Option<OverlayRender> {
    let integration_height = 14u16
        .saturating_add(settings.integrations.len().max(1) as u16)
        .saturating_add(settings.integration_messages.len().min(6) as u16);
    let height = match settings.section {
        ClientSettingsSection::Integrations => integration_height.max(22),
        // every LOOK row with its gap, and the sketch beside them
        ClientSettingsSection::Look => 12 + 2 * super::settings::LookRow::ALL.len() as u16,
        _ => 22,
    };
    let orcha = super::super::brutal::orcha_look();
    // one size for every section, so switching sections does not move the frame
    let orcha_height =
        (12 + 2 * super::settings::LookRow::ALL.len() as u16).max(integration_height) + 2;
    let popup = if orcha {
        popup(buffer.area, 112, orcha_height)?
    } else {
        popup(buffer.area, 98, height)?
    };
    let inner = panel(buffer, popup, palette.accent, palette.panel_bg)?;
    if inner.width < 20 || inner.height < 8 {
        return None;
    }
    if orcha {
        let badge = integration_updates_available
            || settings.integrations.iter().any(|integration| {
                integration.state == crate::api::schema::IntegrationState::Outdated
            });
        let installable = settings
            .integrations
            .iter()
            .any(super::super::settings::integration_needs_install);
        let show_primary = settings.section != ClientSettingsSection::Integrations || installable;
        let (content, tab_hits, primary, close) =
            render_orcha_frame(buffer, inner, settings, badge, show_primary, palette);
        let choice_hits = render_section_content(buffer, content, settings, palette);
        return Some(OverlayRender {
            area: popup,
            primary,
            cancel: close,
            settings_popup: popup,
            settings_tabs: tab_hits,
            settings_choices: choice_hits,
            ..OverlayRender::default()
        });
    }

    put_text(
        buffer,
        inner.x,
        inner.y,
        inner.width,
        " settings",
        Style::default()
            .fg(palette.text)
            .bg(palette.panel_bg)
            .add_modifier(Modifier::BOLD),
    );

    let integration_badge = integration_updates_available
        || settings
            .integrations
            .iter()
            .any(|integration| integration.state == crate::api::schema::IntegrationState::Outdated);
    let mut tab_x = inner.x;
    let mut tab_hits = Vec::new();
    for section in ClientSettingsSection::ALL {
        let badge = *section == ClientSettingsSection::Integrations && integration_badge;
        let label = if badge {
            format!(" ● {} ", section.label())
        } else {
            format!(" {} ", section.label())
        };
        let width = display_width(&label).min(inner.right().saturating_sub(tab_x));
        let rect = Rect::new(tab_x, inner.y + 1, width, 1);
        let active = *section == settings.section;
        let style = if active {
            Style::default()
                .fg(contrast(palette))
                .bg(palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.overlay1).bg(palette.panel_bg)
        };
        buffer.set_style(rect, style);
        put_text(buffer, rect.x, rect.y, rect.width, &label, style);
        if badge && !active {
            put_text(
                buffer,
                rect.x.saturating_add(1),
                rect.y,
                rect.width.saturating_sub(1).min(2),
                "● ",
                Style::default()
                    .fg(palette.accent)
                    .bg(palette.panel_bg)
                    .add_modifier(Modifier::BOLD),
            );
        }
        tab_hits.push((rect, *section));
        tab_x = tab_x.saturating_add(width.saturating_add(1));
        if tab_x >= inner.right() {
            break;
        }
    }
    put_text(
        buffer,
        inner.x,
        inner.y + 2,
        inner.width,
        &"─".repeat(inner.width as usize),
        Style::default().fg(palette.surface0).bg(palette.panel_bg),
    );

    let content = Rect::new(
        inner.x,
        inner.y + 4,
        inner.width,
        inner.height.saturating_sub(7),
    );
    let choice_hits = render_section_content(buffer, content, settings, palette);

    let installable = settings
        .integrations
        .iter()
        .any(super::super::settings::integration_needs_install);
    let show_primary = settings.section != ClientSettingsSection::Integrations || installable;
    let labels = if show_primary { vec![10, 12] } else { vec![12] };
    let buttons = row(inner, &labels, 2, inner.height.saturating_sub(1));
    let (primary, close) = if show_primary {
        let primary = buttons[0];
        button(
            buffer,
            primary,
            if settings.section == ClientSettingsSection::Integrations {
                " ↵ install "
            } else {
                " ↵ apply "
            },
            Style::default()
                .fg(contrast(palette))
                .bg(palette.accent)
                .add_modifier(Modifier::BOLD),
        );
        (primary, buttons[1])
    } else {
        (Rect::default(), buttons[0])
    };
    button(
        buffer,
        close,
        " esc close ",
        Style::default()
            .fg(palette.text)
            .bg(palette.surface0)
            .add_modifier(Modifier::BOLD),
    );
    put_text(
        buffer,
        inner.x,
        inner.bottom().saturating_sub(2),
        inner.width,
        " ↑↓ select  tab section",
        Style::default().fg(palette.overlay1).bg(palette.panel_bg),
    );

    Some(OverlayRender {
        area: popup,
        primary,
        cancel: close,
        settings_popup: popup,
        settings_tabs: tab_hits,
        settings_choices: choice_hits,
        ..OverlayRender::default()
    })
}

/// The section's own rows in `content`; returns the clickable choices.
fn render_section_content(
    buffer: &mut Buffer,
    content: Rect,
    settings: &ClientSettingsOverlay,
    palette: &Palette,
) -> Vec<(Rect, usize)> {
    let mut choice_hits = Vec::new();
    match settings.section {
        ClientSettingsSection::Theme => {
            let visible = usize::from(content.height);
            let scroll = settings.selected.saturating_sub(visible.saturating_sub(1));
            for (visible_index, (index, name)) in crate::config::THEME_NAMES
                .iter()
                .enumerate()
                .skip(scroll)
                .take(visible)
                .enumerate()
            {
                let rect = Rect::new(
                    content.x,
                    content.y + visible_index as u16,
                    content.width,
                    1,
                );
                draw_choice(
                    buffer,
                    rect,
                    name,
                    index == settings.selected,
                    super::super::settings::normalized_theme_name(name)
                        == super::super::settings::normalized_theme_name(
                            &settings.original_theme_name,
                        ),
                    palette,
                );
                // the theme's own colors: paper, card, ink, then its accents
                if let Some(theme) = Palette::from_name(name) {
                    let paper = match theme.sidebar_bg {
                        ratatui::style::Color::Reset => theme.panel_bg,
                        color => color,
                    };
                    let swatches = [
                        paper,
                        theme.panel_bg,
                        theme.text,
                        theme.mauve,
                        theme.yellow,
                        theme.green,
                        theme.blue,
                        theme.red,
                    ];
                    let width = swatches.len() as u16 * 2;
                    let mut sx = rect.right().saturating_sub(width + 1);
                    for color in swatches {
                        put_text(buffer, sx, rect.y, 2, "██", Style::default().fg(color));
                        sx += 2;
                    }
                }
                choice_hits.push((rect, index));
            }
        }
        ClientSettingsSection::Font
        | ClientSettingsSection::FontSize
        | ClientSettingsSection::Ligatures
            if !crate::client::shell::brutal::host_is_ghostty() =>
        {
            render_choice_section(
                buffer,
                content,
                "font, size and ligatures",
                "orcha sets these in Ghostty only; pick them in this terminal's own settings",
                &[],
                0,
                palette,
                &mut choice_hits,
            );
        }
        ClientSettingsSection::Font => {
            let fonts = &settings.appearance.fonts;
            if fonts.is_empty() {
                put_text(
                    buffer,
                    content.x + 1,
                    content.y,
                    content.width,
                    "no font list (needs Ghostty: `ghostty +list-fonts`)",
                    Style::default().fg(palette.overlay0).bg(palette.panel_bg),
                );
            }
            let visible = usize::from(content.height.saturating_sub(2));
            let scroll = settings.selected.saturating_sub(visible.saturating_sub(1));
            for (visible_index, (index, name)) in fonts
                .iter()
                .enumerate()
                .skip(scroll)
                .take(visible)
                .enumerate()
            {
                let rect = Rect::new(
                    content.x,
                    content.y + visible_index as u16,
                    content.width,
                    1,
                );
                draw_choice(
                    buffer,
                    rect,
                    name,
                    index == settings.selected,
                    settings.appearance.font.starts_with(name.as_str()),
                    palette,
                );
                choice_hits.push((rect, index));
            }
            if let Some(note) = &settings.appearance.note {
                put_text(
                    buffer,
                    content.x + 1,
                    content.bottom().saturating_sub(1),
                    content.width,
                    note,
                    Style::default().fg(palette.green).bg(palette.panel_bg),
                );
            }
        }
        ClientSettingsSection::FontSize => {
            let now = settings.appearance.font_size;
            let labels: Vec<String> = crate::client::shell::settings::FONT_SIZES
                .map(|size| {
                    let mark = if size == now { "  (now)" } else { "" };
                    format!("{size} pt{mark}")
                })
                .collect();
            let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
            render_choice_section(
                buffer,
                content,
                "font size",
                "how big the terminal's text is; orcha's default is 15",
                &labels,
                settings.selected,
                palette,
                &mut choice_hits,
            );
            if let Some(note) = &settings.appearance.note {
                put_text(
                    buffer,
                    content.x + 1,
                    content.bottom().saturating_sub(1),
                    content.width,
                    note,
                    Style::default().fg(palette.green).bg(palette.panel_bg),
                );
            }
        }
        ClientSettingsSection::Ligatures => {
            render_choice_section(
                buffer,
                content,
                "font ligatures",
                "join -> != === >= into one glyph, or keep every character apart",
                &[
                    "on    ->  !=  ===  >=  (joined)",
                    "off   -> != === >= (as typed)",
                ],
                settings.selected,
                palette,
                &mut choice_hits,
            );
            if let Some(note) = &settings.appearance.note {
                put_text(
                    buffer,
                    content.x + 1,
                    content.bottom().saturating_sub(1),
                    content.width,
                    note,
                    Style::default().fg(palette.green).bg(palette.panel_bg),
                );
            }
        }
        ClientSettingsSection::Ide | ClientSettingsSection::GitUi => {
            let ide = settings.section == ClientSettingsSection::Ide;
            let (apps, title, description) = if ide {
                (
                    &settings.appearance.ides,
                    "IDE",
                    "the IDE button (right panel) opens the project here",
                )
            } else {
                (
                    &settings.appearance.git_guis,
                    "git ui",
                    "the Git button (right panel) opens the repository here",
                )
            };
            let names: Vec<&str> = apps.iter().map(|(name, _)| name.as_str()).collect();
            render_choice_section(
                buffer,
                content,
                title,
                description,
                &names,
                settings.selected,
                palette,
                &mut choice_hits,
            );
            if let Some(note) = &settings.appearance.note {
                put_text(
                    buffer,
                    content.x + 1,
                    content.bottom().saturating_sub(1),
                    content.width,
                    note,
                    Style::default().fg(palette.green).bg(palette.panel_bg),
                );
            }
        }
        ClientSettingsSection::Look => {
            let rows: Vec<String> = super::settings::LookRow::ALL
                .iter()
                .zip(&settings.look)
                .map(|(row, value)| format!("{:<13}{value}", row.label()))
                .collect();
            let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
            // the sketch takes the right part when there is room
            let sketch_width = 30u16;
            let (list, sketch) = if content.width >= 78 {
                (
                    Rect::new(
                        content.x,
                        content.y,
                        content.width - sketch_width - 2,
                        content.height,
                    ),
                    Some(Rect::new(
                        content.right() - sketch_width,
                        content.y + 3,
                        sketch_width,
                        content.height.saturating_sub(3).min(11),
                    )),
                )
            } else {
                (content, None)
            };
            if let Some(sketch) = sketch {
                render_look_map(buffer, sketch, &settings.look_map, palette);
            }
            let content = list;
            render_choice_section(
                buffer,
                content,
                "look",
                "enter or click: next value · the sketch shows the result",
                &rows,
                settings.selected,
                palette,
                &mut choice_hits,
            );
        }
        ClientSettingsSection::Indicators => {
            render_choice_section(
                buffer,
                content,
                "agent status indicators",
                "choose color dots or distinct symbols for each state",
                &["color dots  ● ● ● ○ ·", "distinct symbols  × ◐ ✓ ○ ·"],
                settings.selected,
                palette,
                &mut choice_hits,
            );
        }
        ClientSettingsSection::Sound => {
            render_choice_section(
                buffer,
                content,
                "sound alerts",
                "play sounds when agents change state in background",
                &["on", "off"],
                settings.selected,
                palette,
                &mut choice_hits,
            );
        }
        ClientSettingsSection::Toast => {
            render_choice_section(
                buffer,
                content,
                "notification popups",
                "choose where background popup notifications should appear",
                &["off", "inside herdr", "via terminal", "via system"],
                settings.selected,
                palette,
                &mut choice_hits,
            );
        }
        ClientSettingsSection::Integrations => {
            render_integrations(buffer, content, settings, palette);
        }
    }
    choice_hits
}

/// A small sketch of the window the LOOK values make: header, sidebar,
/// panes, right panel, PROCESS and the status bar, each where it will be.
fn render_look_map(
    buffer: &mut Buffer,
    area: Rect,
    map: &super::settings::LookMap,
    palette: &Palette,
) {
    use super::settings::MapProcess;
    if area.width < 20 || area.height < 8 {
        return;
    }
    let line = Style::default().fg(palette.overlay0).bg(palette.panel_bg);
    let label = Style::default().fg(palette.overlay1).bg(palette.panel_bg);
    let strong = Style::default()
        .fg(palette.text)
        .bg(palette.panel_bg)
        .add_modifier(Modifier::BOLD);
    let accent = Style::default()
        .fg(contrast(palette))
        .bg(palette.mauve)
        .add_modifier(Modifier::BOLD);
    put_text(buffer, area.x, area.y, area.width, "preview", label);
    let frame = Rect::new(area.x, area.y + 1, area.width, area.height - 1);
    let inner_w = frame.width - 2;
    put_text(
        buffer,
        frame.x,
        frame.y,
        frame.width,
        &format!("┌{}┐", "─".repeat(usize::from(inner_w))),
        line,
    );
    for y in frame.y + 1..frame.bottom() - 1 {
        put_text(buffer, frame.x, y, 1, "│", line);
        put_text(buffer, frame.right() - 1, y, 1, "│", line);
    }
    put_text(
        buffer,
        frame.x,
        frame.bottom() - 1,
        frame.width,
        &format!("└{}┘", "─".repeat(usize::from(inner_w))),
        line,
    );
    // header and status bar
    put_text(buffer, frame.x + 1, frame.y + 1, 7, " ORCHA ", accent);
    let status_y = frame.bottom() - 2;
    let status = if map.fuel_in_status {
        " status · FUEL"
    } else {
        " status"
    };
    put_text(buffer, frame.x + 1, status_y, inner_w, status, label);
    // the body between them: sidebar, panes, panel
    let top = frame.y + 2;
    let bottom = status_y;
    let sidebar_w = [6u16, 7, 9][map.sidebar.min(2)];
    let panel_w = if map.panel { 8u16 } else { 0 };
    let x0 = frame.x + 1;
    let x1 = frame.right() - 1;
    let (sidebar_x, panel_x) = if map.sidebar_right {
        (x1 - sidebar_w, x0)
    } else {
        (x0, x1 - panel_w)
    };
    // the panes fill whatever the side columns leave
    let (panes_x, panes_right) = match (map.sidebar_right, map.panel) {
        (false, true) => (sidebar_x + sidebar_w + 1, panel_x - 1),
        (false, false) => (sidebar_x + sidebar_w + 1, x1),
        (true, true) => (panel_x + panel_w + 1, sidebar_x - 1),
        (true, false) => (x0, sidebar_x - 1),
    };
    let column = |buffer: &mut Buffer, x: u16| {
        for y in top..bottom {
            put_text(buffer, x, y, 1, "┆", line);
        }
    };
    if map.sidebar_right {
        column(buffer, sidebar_x - 1);
    } else {
        column(buffer, sidebar_x + sidebar_w);
    }
    if map.panel {
        if map.sidebar_right {
            column(buffer, panel_x + panel_w);
        } else {
            column(buffer, panel_x - 1);
        }
    }
    put_text(buffer, sidebar_x, top, sidebar_w, "PROJ", strong);
    let panes_w = panes_right.saturating_sub(panes_x);
    put_text(
        buffer,
        panes_x + panes_w.saturating_sub(5) / 2,
        top + (bottom - top) / 2,
        5,
        "panes",
        strong,
    );
    if map.panel {
        if map.fuel_in_status {
            put_text(buffer, panel_x, top, panel_w, "GIT", strong);
        } else {
            put_text(buffer, panel_x, top, panel_w, "FUEL", strong);
            put_text(buffer, panel_x, top + 1, panel_w, "GIT", label);
        }
    }
    let process = match map.process {
        MapProcess::Sidebar => Some((sidebar_x, sidebar_w)),
        MapProcess::Panel => Some((panel_x, panel_w)),
        MapProcess::Off => None,
    };
    if let Some((x, width)) = process {
        put_text(buffer, x, bottom - 1, width, "PROC", accent);
    }
}

fn render_choice_section(
    buffer: &mut Buffer,
    area: Rect,
    title: &str,
    description: &str,
    choices: &[&str],
    selected: usize,
    palette: &Palette,
    hits: &mut Vec<(Rect, usize)>,
) {
    let title = if super::super::brutal::orcha_look() {
        title.to_uppercase()
    } else {
        title.to_owned()
    };
    put_text(
        buffer,
        area.x,
        area.y,
        area.width,
        &title,
        Style::default()
            .fg(palette.text)
            .bg(palette.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    put_text(
        buffer,
        area.x,
        area.y + 1,
        area.width,
        description,
        Style::default().fg(palette.overlay1).bg(palette.panel_bg),
    );
    let row_gap = u16::from(choices.len() > 2);
    for (index, choice) in choices.iter().enumerate() {
        let y = area.y + 3 + index as u16 * (1 + row_gap);
        if y >= area.bottom() {
            break;
        }
        let rect = Rect::new(area.x, y, area.width, 1);
        draw_choice(buffer, rect, choice, index == selected, false, palette);
        hits.push((rect, index));
    }
}

fn render_integrations(
    buffer: &mut Buffer,
    area: Rect,
    settings: &ClientSettingsOverlay,
    palette: &Palette,
) {
    let orcha = super::super::brutal::orcha_look();
    put_text(
        buffer,
        area.x,
        area.y,
        area.width,
        if orcha {
            "AGENT INTEGRATIONS"
        } else {
            "agent integrations"
        },
        Style::default()
            .fg(palette.text)
            .bg(palette.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    put_text(
        buffer,
        area.x,
        area.y + 1,
        area.width,
        "enable session restore and, where supported, direct status updates",
        Style::default().fg(palette.overlay1).bg(palette.panel_bg),
    );
    if settings.loading_integrations {
        put_text(
            buffer,
            area.x,
            area.y + 3,
            area.width,
            " loading integrations…",
            Style::default().fg(palette.overlay1).bg(palette.panel_bg),
        );
        return;
    }
    if settings.integrations.is_empty() {
        put_text(
            buffer,
            area.x,
            area.y + 3,
            area.width,
            " no integration targets available",
            Style::default().fg(palette.overlay1).bg(palette.panel_bg),
        );
        return;
    }
    for (index, integration) in settings.integrations.iter().enumerate() {
        let y = area.y + 3 + index as u16;
        if y >= area.bottom() {
            break;
        }
        let readable = |color| {
            if orcha {
                super::super::brutal::Brutal::new(palette).text(color, palette.panel_bg)
            } else {
                color
            }
        };
        let (marker, color, status) = match integration.state {
            crate::api::schema::IntegrationState::Current => ("✓", palette.green, "installed"),
            crate::api::schema::IntegrationState::Outdated => {
                ("↻", palette.yellow, "update available")
            }
            crate::api::schema::IntegrationState::NotInstalled if integration.available => {
                ("+", palette.accent, "available")
            }
            crate::api::schema::IntegrationState::NotInstalled => {
                ("–", palette.overlay0, "not found")
            }
        };
        put_text(
            buffer,
            area.x,
            y,
            3,
            &format!(" {marker}"),
            Style::default().fg(readable(color)).bg(palette.panel_bg),
        );
        put_text(
            buffer,
            area.x + 3,
            y,
            11.min(area.width.saturating_sub(3)),
            &format!("{:<9}", integration.label),
            Style::default().fg(palette.subtext0).bg(palette.panel_bg),
        );
        put_text(
            buffer,
            area.x + 14,
            y,
            area.width.saturating_sub(14),
            status,
            Style::default().fg(palette.overlay1).bg(palette.panel_bg),
        );
    }
    let message_y = area
        .y
        .saturating_add(4)
        .saturating_add(settings.integrations.len() as u16);
    for (offset, message) in settings.integration_messages.iter().take(6).enumerate() {
        let y = message_y.saturating_add(offset as u16);
        if y >= area.bottom() {
            break;
        }
        put_text(
            buffer,
            area.x,
            y,
            area.width,
            &format!(" {message}"),
            Style::default().fg(palette.overlay1).bg(palette.panel_bg),
        );
    }
    if settings.installing_integrations && message_y < area.bottom() {
        put_text(
            buffer,
            area.x,
            message_y,
            area.width,
            " installing…",
            Style::default().fg(palette.overlay1).bg(palette.panel_bg),
        );
    }
}
