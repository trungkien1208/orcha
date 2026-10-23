//! orcha: the neo-brutal look ("Calm"). Tokens come from the palette so every
//! theme gets the same shapes; the brutal themes set the palette so the
//! tokens match the mockup exactly (docs/orcha/SPEC.md §1).
//!
//! Shapes in cells: thick border = heavy box drawing, thin = light, dashed =
//! light dashes; a hard shadow is a half-block strip one cell right and down.

use crate::protocol::ClientShellAgent;
use ratatui::style::Color;

use super::render::{display_width, put_text};
use super::*;

/// The orcha look is on screen (set per frame by the shell render), so
/// herdr's shared popup frame draws like orcha's popups.
static ORCHA_LOOK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(super) fn set_orcha_look(on: bool) {
    ORCHA_LOOK.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub(super) fn orcha_look() -> bool {
    ORCHA_LOOK.load(std::sync::atomic::Ordering::Relaxed)
}

/// herdr's popup frame in the orcha template: the fill, black eighth-block
/// edges, a hard shadow merged into the right and bottom edges.
pub(super) fn popup_panel(buffer: &mut Buffer, rect: Rect, fill: Color) {
    let line = if is_light(fill) {
        Color::Rgb(17, 17, 17)
    } else {
        Color::Rgb(0, 0, 0)
    };
    // the shadow sits on whatever (dimmed) page is around the popup
    let page = buffer
        .cell((rect.right().saturating_sub(1), rect.bottom().saturating_sub(1)))
        .and_then(|cell| cell.style().bg)
        .unwrap_or(Color::Reset);
    draw_box(buffer, rect, Edge::Heavy, line, fill, page);
    draw_box_shadow(buffer, rect, line, page);
}

/// The look's colors, resolved from a palette.
#[derive(Debug, Clone, Copy)]
pub(super) struct Brutal {
    pub bg: Color,
    pub card: Color,
    pub frame: Color,
    pub ink: Color,
    pub line: Color,
    pub shadow: Color,
    pub muted: Color,
    pub pane: Color,
    pub pane_text: Color,
    pub status_bg: Color,
    pub status_text: Color,
    pub yellow: Color,
    pub pink: Color,
    pub green: Color,
    pub blue: Color,
    pub red: Color,
    pub rail: Color,
    pub rail_ahead: Color,
    pub empty: Color,
    /// Text on a bright chip.
    pub chip_text: Color,
}

fn rgb(color: Color) -> Option<(u8, u8, u8)> {
    match color {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        _ => None,
    }
}

pub(super) fn is_light(color: Color) -> bool {
    rgb(color).is_some_and(|(r, g, b)| {
        u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114 > 128_000
    })
}

impl Brutal {
    pub(super) fn new(palette: &Palette) -> Self {
        let bg = match palette.sidebar_bg {
            Color::Reset => palette.panel_bg,
            color => color,
        };
        let light = is_light(bg);
        // dark mode keeps its lines dark: black on charcoal, never bright
        let (line, shadow) = if light {
            (palette.text, palette.text)
        } else {
            (Color::Rgb(11, 10, 9), Color::Rgb(0, 0, 0))
        };
        Self {
            bg,
            card: palette.panel_bg,
            frame: palette.surface0,
            ink: palette.text,
            line,
            shadow,
            muted: palette.overlay0,
            pane: if light {
                palette.panel_bg
            } else {
                palette.active_row_bg
            },
            pane_text: palette.text,
            status_bg: line,
            status_text: if light { bg } else { palette.text },
            yellow: palette.yellow,
            pink: palette.mauve,
            green: palette.green,
            blue: palette.blue,
            red: palette.red,
            rail: if light { palette.text } else { palette.overlay1 },
            rail_ahead: palette.surface1,
            empty: palette.surface_dim,
            chip_text: Color::Rgb(17, 17, 17),
        }
    }

    /// An agent's color: Claude yellow, Codex green, Cursor blue.
    pub(super) fn agent(&self, agent: Option<&str>) -> Color {
        match agent.unwrap_or("").to_ascii_lowercase().as_str() {
            "claude" => self.yellow,
            "codex" => self.green,
            "cursor" => self.blue,
            "gemini" => self.pink,
            _ => self.card,
        }
    }

    /// A state's color: yellow working, green done, red needs you.
    pub(super) fn state(&self, status: crate::api::schema::AgentStatus) -> Option<Color> {
        use crate::api::schema::AgentStatus;
        match status {
            AgentStatus::Working => Some(self.yellow),
            AgentStatus::Blocked => Some(self.red),
            AgentStatus::Done => Some(self.green),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Edge {
    Heavy,
    Light,
    Dashed,
}

impl Edge {
    /// (top-left, top, top-right, left, right, bottom-left, bottom, bottom-right)
    ///
    /// Heavy and light edges are block elements that hug the fill: a line
    /// drawn in the middle of a cell would leave half a cell of fill
    /// outside it.
    fn glyphs(self) -> [&'static str; 8] {
        match self {
            // ~3 px like the mockup's thick lines: half a column, a quarter row
            // eighth-block lines hug the fill; their corners would be an
            // eighth of a cell, so they stay blank rather than stick out
            // (heavy boxes get their weight from the shadow)
            Edge::Heavy | Edge::Light => [" ", "▁", " ", "▕", "▏", " ", "▔", " "],
            Edge::Dashed => ["┌", "┄", "┐", "┆", "┆", "└", "┄", "┘"],
        }
    }
}

pub(super) fn set(buffer: &mut Buffer, x: u16, y: u16, symbol: &str, style: Style) {
    if let Some(cell) = buffer.cell_mut((x, y)) {
        cell.set_symbol(symbol);
        cell.set_style(style);
    }
}

/// A box: the edge in `line` on the `page` around it, the inside filled.
pub(super) fn draw_box(
    buffer: &mut Buffer,
    rect: Rect,
    edge: Edge,
    line: Color,
    fill: Color,
    page: Color,
) {
    if rect.width < 2 || rect.height < 2 {
        return;
    }
    // a box hides whatever was under it
    let clip = rect.intersection(buffer.area);
    for y in clip.y..clip.bottom() {
        for x in clip.x..clip.right() {
            set(buffer, x, y, " ", Style::reset().bg(fill));
        }
    }
    let [tl, top, tr, left, right_edge, bl, bottom_edge, br] = edge.glyphs();
    // dashed lines sit on the fill (it is the page there)
    let style = Style::reset().fg(line).bg(if edge == Edge::Dashed { fill } else { page });
    let (right, bottom) = (rect.right() - 1, rect.bottom() - 1);
    for x in rect.x + 1..right {
        set(buffer, x, rect.y, top, style);
        set(buffer, x, bottom, bottom_edge, style);
    }
    for y in rect.y + 1..bottom {
        set(buffer, rect.x, y, left, style);
        set(buffer, right, y, right_edge, style);
    }
    set(buffer, rect.x, rect.y, tl, style);
    set(buffer, right, rect.y, tr, style);
    set(buffer, rect.x, bottom, bl, style);
    set(buffer, right, bottom, br, style);
}

/// The hard shadow of a box, merged into its right and bottom edges: half
/// a cell of line and shadow down the right, half a row along the bottom.
pub(super) fn draw_box_shadow(buffer: &mut Buffer, rect: Rect, shadow: Color, page: Color) {
    if rect.width < 2 || rect.height < 2 {
        return;
    }
    let style = Style::reset().fg(shadow).bg(page);
    let (right, bottom) = (rect.right() - 1, rect.bottom() - 1);
    for y in rect.y + 1..bottom {
        set(buffer, right, y, "▌", style);
    }
    for x in rect.x + 1..right {
        set(buffer, x, bottom, "▀", style);
    }
    set(buffer, right, bottom, "▀", style);
}

/// The hard shadow of a one-row block (a button): half a cell right and
/// down, on the row under it.
pub(super) fn draw_shadow(buffer: &mut Buffer, rect: Rect, shadow: Color, page: Color) {
    let style = Style::reset().fg(shadow).bg(page);
    let (right, below) = (rect.right(), rect.bottom());
    for y in rect.y..below {
        set(buffer, right, y, if y == rect.y { "▖" } else { "▌" }, style);
    }
    set(buffer, rect.x, below, "▝", style);
    for x in rect.x + 1..right {
        set(buffer, x, below, "▀", style);
    }
    set(buffer, right, below, "▘", style);
}

/// A one-row block: filled, the text bold and centered; with `edges`, a
/// thick line down each side (an outlined button or a folded card).
#[allow(clippy::too_many_arguments)] // one block, all inputs are distinct facts
pub(super) fn pill(
    buffer: &mut Buffer,
    rect: Rect,
    text: &str,
    fg: Color,
    fill: Color,
    edges: bool,
    t: &Brutal,
) {
    if rect.width < 3 {
        return;
    }
    for x in rect.x..rect.right() {
        set(buffer, x, rect.y, " ", Style::reset().bg(fill));
    }
    if edges {
        set(buffer, rect.x, rect.y, "▕", Style::reset().fg(t.line).bg(t.bg));
        set(buffer, rect.right() - 1, rect.y, "▏", Style::reset().fg(t.line).bg(t.bg));
    }
    if text.is_empty() {
        return;
    }
    let inner = rect.width.saturating_sub(2);
    let width = display_width(text).min(inner);
    put_text(
        buffer,
        rect.x + 1 + inner.saturating_sub(width) / 2,
        rect.y,
        width,
        text,
        Style::default().fg(fg).bg(fill).add_modifier(Modifier::BOLD),
    );
}

/// A run of colored cells with padded bold text; returns its width.
pub(super) fn chip(
    buffer: &mut Buffer,
    x: u16,
    y: u16,
    max: u16,
    text: &str,
    fg: Color,
    bg: Color,
) -> u16 {
    let label = format!(" {text} ");
    let width = display_width(&label).min(max);
    put_text(
        buffer,
        x,
        y,
        width,
        &label,
        Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
    );
    width
}

/// Clips to `width` cells with an ellipsis.
pub(super) fn clip(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        used += w;
        out.push(ch);
    }
    out.push('…');
    out
}

/// "claude-opus-5-5" → "OPUS 5.5", "gpt-6-luna" → "GPT-6 LUNA". The train
/// and pane title show the model, never the agent name.
pub(super) fn model_label(model: &str) -> String {
    let model = model.trim();
    let lower = model.to_ascii_lowercase();
    let rest = lower.strip_prefix("claude-").unwrap_or(&lower);
    let rest = rest.split('[').next().unwrap_or(rest);
    // drop a trailing date stamp like -20260101
    let parts: Vec<&str> = rest
        .split('-')
        .filter(|part| !(part.len() == 8 && part.chars().all(|c| c.is_ascii_digit())))
        .collect();
    if lower.starts_with("claude-") && parts.len() >= 2 {
        let family = parts[0].to_ascii_uppercase();
        let version = parts[1..].join(".");
        return format!("{family} {version}");
    }
    if let Some(first) = parts.first().filter(|_| lower.starts_with("gpt-")) {
        let head = format!("{}-{}", first, parts.get(1).copied().unwrap_or(""));
        let tail = parts.get(2..).map(|p| p.join(" ")).unwrap_or_default();
        return format!("{head} {tail}").trim().to_ascii_uppercase();
    }
    model.to_ascii_uppercase()
}

/// A pane's model label: the `model` token an integration reports, else the
/// agent's name.
pub(super) fn agent_label(agent: &ClientShellAgent) -> String {
    agent
        .tokens
        .iter()
        .find(|(key, value)| key == "model" && !value.trim().is_empty())
        .map(|(_, value)| model_label(value))
        .or_else(|| agent.display_agent.clone())
        .or_else(|| agent.agent.clone())
        .or_else(|| agent.name.clone())
        .unwrap_or_else(|| "agent".to_owned())
}

/// What the agent is working on: the `task` token, else its terminal title.
pub(super) fn agent_task(agent: &ClientShellAgent) -> String {
    agent
        .tokens
        .iter()
        .find(|(key, value)| key == "task" && !value.trim().is_empty())
        .map(|(_, value)| value.clone())
        .or_else(|| {
            agent
                .terminal_title_stripped
                .clone()
                .filter(|title| !title.trim().is_empty())
        })
        .or_else(|| agent.title.clone())
        .unwrap_or_default()
}

/// The "Agent Icons" font (U+100000.., built by orcha's setup) is installed.
pub(super) fn icon_font_installed() -> bool {
    if cfg!(test) {
        return false;
    }
    let Some(home) = std::env::var_os("HOME") else {
        return false;
    };
    let home = std::path::Path::new(&home);
    ["Library/Fonts/AgentIcons.ttf", ".local/share/fonts/AgentIcons.ttf"]
        .iter()
        .any(|path| home.join(path).exists())
}

/// The orcha icon (a whale in a matcha bowl) from the icon font, else a symbol.
pub(super) fn orcha_icon(icon_font: bool) -> &'static str {
    if icon_font {
        "\u{100003}"
    } else {
        "◉"
    }
}

/// The agent's logo: the Agent Icons font glyph when installed, else a symbol.
pub(super) fn agent_logo(agent: Option<&str>, icon_font: bool) -> &'static str {
    match (agent.unwrap_or("").to_ascii_lowercase().as_str(), icon_font) {
        ("claude", true) => "\u{100000}",
        ("codex", true) => "\u{100001}",
        ("cursor", true) => "\u{100002}",
        ("claude", false) => "✳",
        ("codex", false) => "◎",
        ("cursor", false) => "▲",
        _ => "▶",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_names_read_like_the_mockup() {
        assert_eq!(model_label("claude-opus-5-5"), "OPUS 5.5");
        assert_eq!(model_label("claude-sonnet-5-5[1m]"), "SONNET 5.5");
        assert_eq!(model_label("claude-haiku-4-5-20251001"), "HAIKU 4.5");
        assert_eq!(model_label("gpt-6-luna"), "GPT-6 LUNA");
        assert_eq!(model_label("gpt-5"), "GPT-5");
    }

    #[test]
    fn dark_lines_stay_dark() {
        let dark = Brutal::new(&Palette::brutal_dark());
        assert_eq!(dark.line, Color::Rgb(11, 10, 9));
        let light = Brutal::new(&Palette::brutal());
        assert_eq!(light.line, Color::Rgb(17, 17, 17));
        assert_eq!(light.bg, Color::Rgb(255, 244, 214));
    }
}
