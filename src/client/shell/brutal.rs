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
        .cell((rect.right(), rect.bottom()))
        .and_then(|cell| cell.style().bg)
        .unwrap_or(Color::Reset);
    draw_box(buffer, rect, line, fill, page);
    draw_box_shadow(buffer, rect, line, page);
}

/// A bright hue of the look, for things that keep their color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hue {
    Yellow,
    Green,
    Blue,
    Pink,
}

/// What the shell knows about an agent CLI: its color, its logo, its name,
/// the titles that only name the program, and whether a chat can be handed
/// to it. One row per agent in `AGENT_KINDS`; every lookup goes through
/// `AgentKind::of`, which ignores case.
#[derive(Debug)]
pub(super) struct AgentKind {
    /// herdr's agent id ("claude").
    pub id: &'static str,
    /// The name people know it by.
    pub name: &'static str,
    pub hue: Hue,
    /// Its glyph in the Agent Icons font, when the font has one.
    pub font_glyph: Option<&'static str>,
    /// The symbol drawn without the font.
    pub symbol: &'static str,
    /// Terminal titles that only name the program (lowercase): they say
    /// nothing about the task.
    pub programs: &'static [&'static str],
    /// A chat can be handed off to it.
    pub handoff: bool,
}

pub(super) const AGENT_KINDS: [AgentKind; 4] = [
    AgentKind {
        id: "claude",
        name: "Claude",
        hue: Hue::Yellow,
        font_glyph: Some("\u{100000}"),
        symbol: "✳",
        programs: &["claude", "claude code"],
        handoff: true,
    },
    AgentKind {
        id: "codex",
        name: "Codex",
        hue: Hue::Green,
        font_glyph: Some("\u{100001}"),
        symbol: "◎",
        programs: &["codex"],
        handoff: true,
    },
    AgentKind {
        id: "cursor",
        name: "Cursor",
        hue: Hue::Blue,
        font_glyph: Some("\u{100002}"),
        symbol: "▲",
        programs: &["cursor", "cursor agent"],
        handoff: true,
    },
    AgentKind {
        id: "gemini",
        name: "Gemini",
        hue: Hue::Pink,
        font_glyph: None,
        symbol: "✦",
        programs: &["gemini"],
        handoff: false,
    },
];

/// The logo of an agent orcha does not know.
pub(super) const UNKNOWN_AGENT_LOGO: &str = "▶";

/// The orcha icon in the Agent Icons font, and its symbol without it.
const ORCHA_GLYPH: &str = "\u{100003}";
const ORCHA_SYMBOL: &str = "◉";

/// Marks a tab name may start with besides the agents' logos: symbols
/// earlier builds stored, the orcha icon and the unknown agent's logo.
const OTHER_MARKS: [char; 9] = ['◇', '⚡', '◆', '◈', '◉', '◐', '✧', '▶', '●'];

impl AgentKind {
    /// The row for herdr's agent id or a display name, any case.
    pub(super) fn of(agent: Option<&str>) -> Option<&'static AgentKind> {
        let agent = agent?.trim();
        AGENT_KINDS.iter().find(|kind| {
            kind.id.eq_ignore_ascii_case(agent) || kind.name.eq_ignore_ascii_case(agent)
        })
    }

    /// The logo: the font glyph when the font is installed, else the symbol.
    pub(super) fn logo(&self, icon_font: bool) -> &'static str {
        match self.font_glyph {
            Some(glyph) if icon_font => glyph,
            _ => self.symbol,
        }
    }
}

/// `title` only names an agent's program ("Claude Code", "codex").
pub(super) fn names_only_a_program(title: &str) -> bool {
    let title = title.trim();
    AGENT_KINDS.iter().any(|kind| {
        kind.programs
            .iter()
            .any(|program| title.eq_ignore_ascii_case(program))
    })
}

/// Two agent ids name the same agent, whatever their case.
pub(super) fn same_agent(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
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

/// WCAG relative luminance of an sRGB color.
fn luminance((r, g, b): (u8, u8, u8)) -> f64 {
    let channel = |value: u8| {
        let c = f64::from(value) / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

fn contrast(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// `fg` darkened (on a light `bg`) until it reaches `ratio`; unchanged on
/// a dark page or when it reads already.
fn readable(fg: Color, bg: Color, ratio: f64) -> Color {
    let (Some(fg_rgb), Some(bg_rgb)) = (rgb(fg), rgb(bg)) else {
        return fg;
    };
    if !is_light(bg) || contrast(fg_rgb, bg_rgb) >= ratio {
        return fg;
    }
    let (r, g, b) = fg_rgb;
    // keep the hue: scale toward black in small steps
    for step in 1..=20u16 {
        let keep = 100 - step * 5;
        let scale = |value: u8| (u16::from(value) * keep / 100) as u8;
        let darker = (scale(r), scale(g), scale(b));
        if contrast(darker, bg_rgb) >= ratio {
            return Color::Rgb(darker.0, darker.1, darker.2);
        }
    }
    Color::Rgb(17, 17, 17)
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
            rail: if light {
                palette.text
            } else {
                palette.overlay1
            },
            rail_ahead: palette.surface1,
            empty: palette.surface_dim,
            chip_text: Color::Rgb(17, 17, 17),
        }
    }

    /// A hue of the look.
    pub(super) fn hue(&self, hue: Hue) -> Color {
        match hue {
            Hue::Yellow => self.yellow,
            Hue::Green => self.green,
            Hue::Blue => self.blue,
            Hue::Pink => self.pink,
        }
    }

    /// An agent's own color (`AgentKind::hue`); None for an agent orcha
    /// does not know.
    pub(super) fn brand(&self, agent: Option<&str>) -> Option<Color> {
        AgentKind::of(agent).map(|kind| self.hue(kind.hue))
    }

    /// An agent's color: Claude yellow, Codex green, Cursor blue, Gemini
    /// pink; an unknown agent sits on card.
    pub(super) fn agent(&self, agent: Option<&str>) -> Color {
        self.brand(agent).unwrap_or(self.card)
    }

    /// `color` as text on `on`: on a light fill the bright state and
    /// agent colors darken until they read (WCAG 4.5:1). Chips keep the
    /// bright fill; only letters on paper change.
    pub(super) fn text(&self, color: Color, on: Color) -> Color {
        readable(color, on, 4.5)
    }

    /// `color` for thin marks on `on` (gauge segments, rails, bars): 3:1.
    pub(super) fn mark(&self, color: Color, on: Color) -> Color {
        readable(color, on, 3.0)
    }

    /// A pane's memory: nothing while it is unremarkable, yellow from
    /// `HEAVY_MEMORY`, red from twice that.
    pub(super) fn memory(&self, bytes: u64) -> Option<Color> {
        let heavy = crate::orcha_usage::HEAVY_MEMORY;
        if bytes >= heavy.saturating_mul(2) {
            Some(self.red)
        } else if bytes >= heavy {
            Some(self.yellow)
        } else {
            None
        }
    }

    /// How full a context is: green, yellow from 60%, red from 85%.
    pub(super) fn context(&self, percent: u64) -> Color {
        if percent >= crate::orcha_usage::FULL_CONTEXT_PERCENT {
            self.red
        } else if percent >= crate::orcha_usage::HEAVY_CONTEXT_PERCENT {
            self.yellow
        } else {
            self.green
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

pub(super) fn set(buffer: &mut Buffer, x: u16, y: u16, symbol: &str, style: Style) {
    if let Some(cell) = buffer.cell_mut((x, y)) {
        cell.set_symbol(symbol);
        cell.set_style(style);
    }
}

/// A box. Its first and last rows are paper, carrying the top and bottom
/// lines right against the fill (so they read as the gap around the box,
/// not as padding inside it); the side lines run on the fill's own end
/// cells. Corners are square.
pub(super) fn draw_box(buffer: &mut Buffer, rect: Rect, line: Color, fill: Color, page: Color) {
    if rect.width < 2 || rect.height < 3 {
        return;
    }
    let clip = rect.intersection(buffer.area);
    let (top, bottom, right) = (rect.y, rect.bottom() - 1, rect.right() - 1);
    let edge = Style::reset().fg(line).bg(page);
    let side = Style::reset().fg(line).bg(fill);
    for y in clip.y..clip.bottom() {
        for x in clip.x..clip.right() {
            let symbol = if y == top {
                "▁"
            } else if y == bottom {
                "▔"
            } else if x == rect.x {
                "▏"
            } else if x == right {
                "▕"
            } else {
                " "
            };
            let style = if y == top || y == bottom {
                edge
            } else if x == rect.x || x == right {
                side
            } else {
                Style::reset().bg(fill)
            };
            set(buffer, x, y, symbol, style);
        }
    }
}

/// The hard shadow of a box: its bottom line thickens to half a row, and
/// half a column runs down its right side.
pub(super) fn draw_box_shadow(buffer: &mut Buffer, rect: Rect, shadow: Color, page: Color) {
    if rect.width < 2 || rect.height < 3 {
        return;
    }
    let style = Style::reset().fg(shadow).bg(page);
    let (bottom, right) = (rect.bottom() - 1, rect.right());
    for x in rect.x..right {
        set(buffer, x, bottom, "▀", style);
    }
    for y in rect.y + 2..bottom {
        set(buffer, right, y, "▌", style);
    }
    set(buffer, right, bottom, "▘", style);
}

/// The hard shadow of a block: the column right of it and the row under
/// it, half a cell in.
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
        // on the block's own end cells, so the lines hug its fill
        set(
            buffer,
            rect.x,
            rect.y,
            "▏",
            Style::reset().fg(t.line).bg(fill),
        );
        set(
            buffer,
            rect.right() - 1,
            rect.y,
            "▕",
            Style::reset().fg(t.line).bg(fill),
        );
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
        Style::default()
            .fg(fg)
            .bg(fill)
            .add_modifier(Modifier::BOLD),
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
/// agent's name. A logo the tab namer stored at the front of the name is
/// dropped: the chrome draws that logo itself.
pub(super) fn agent_label(agent: &ClientShellAgent) -> String {
    let raw = agent
        .tokens
        .iter()
        .find(|(key, value)| key == "model" && !value.trim().is_empty())
        .map(|(_, value)| model_label(value))
        .or_else(|| agent.display_agent.clone())
        .or_else(|| agent.agent.clone())
        .or_else(|| agent.name.clone())
        .unwrap_or_else(|| "agent".to_owned());
    let name = strip_leading_agent_mark(&raw);
    if name.is_empty() {
        agent
            .agent
            .clone()
            .or_else(|| agent.name.clone())
            .unwrap_or_else(|| "agent".to_owned())
    } else {
        name
    }
}

/// "󰚩 Cursor" → "Cursor". The mark is an Agent Icons glyph or a fallback symbol.
fn strip_leading_agent_mark(label: &str) -> String {
    let trimmed = label.trim_start();
    let Some(mark) = trimmed.chars().next() else {
        return String::new();
    };
    let is_mark = |text: &str| text.chars().eq(std::iter::once(mark));
    let owned = is_mark(ORCHA_GLYPH)
        || AGENT_KINDS
            .iter()
            .any(|kind| is_mark(kind.symbol) || kind.font_glyph.is_some_and(is_mark))
        || OTHER_MARKS.contains(&mark);
    if owned {
        trimmed[mark.len_utf8()..].trim_start().to_owned()
    } else {
        trimmed.to_owned()
    }
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
/// The terminal orcha runs in is Ghostty. Only Ghostty maps the icon
/// font's codepoints and draws the legacy-computing glyphs itself; other
/// terminals get plain symbols. Over SSH only `TERM` says so.
pub(crate) fn host_is_ghostty() -> bool {
    static GHOSTTY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *GHOSTTY.get_or_init(|| {
        cfg!(test)
            || std::env::var("TERM_PROGRAM")
                .is_ok_and(|value| value.eq_ignore_ascii_case("ghostty"))
            || std::env::var("TERM").is_ok_and(|value| value == "xterm-ghostty")
            || std::env::var_os("GHOSTTY_RESOURCES_DIR").is_some()
    })
}

/// Terminals that find a glyph in any installed font: Ghostty maps the
/// codepoints itself, WezTerm and iTerm2 fall back to system fonts.
fn host_draws_icon_font() -> bool {
    host_is_ghostty()
        || std::env::var("TERM_PROGRAM").is_ok_and(|value| {
            value.eq_ignore_ascii_case("wezterm") || value.eq_ignore_ascii_case("iTerm.app")
        })
}

/// The agent logos come from the Agent Icons font (`[orcha] icons`).
pub(super) fn icon_font_installed(mode: crate::config::IconMode) -> bool {
    use crate::config::IconMode;
    if cfg!(test) {
        return false;
    }
    match mode {
        IconMode::Symbols => false,
        IconMode::Font => true,
        IconMode::Auto => host_draws_icon_font() && icon_font_file(),
    }
}

/// The font is in a per-user fonts folder (macOS, Linux, Windows).
fn icon_font_file() -> bool {
    let Some(home) = crate::platform::user_home_dir() else {
        return false;
    };
    [
        "Library/Fonts/AgentIcons.ttf",
        ".local/share/fonts/AgentIcons.ttf",
        "AppData/Local/Microsoft/Windows/Fonts/AgentIcons.ttf",
    ]
    .iter()
    .any(|path| home.join(path).exists())
}

/// The orcha icon (a whale in a matcha bowl) from the icon font, else a symbol.
pub(super) fn orcha_icon(icon_font: bool) -> &'static str {
    if icon_font {
        ORCHA_GLYPH
    } else {
        ORCHA_SYMBOL
    }
}

/// The agent's logo: the Agent Icons font glyph when installed, else a symbol.
pub(super) fn agent_logo(agent: Option<&str>, icon_font: bool) -> &'static str {
    AgentKind::of(agent).map_or(UNKNOWN_AGENT_LOGO, |kind| kind.logo(icon_font))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_table_answers_for_every_agent_in_any_case() {
        let t = Brutal::new(&Palette::brutal());
        assert_eq!(t.agent(Some("CLAUDE")), t.yellow);
        assert_eq!(t.agent(Some("Codex")), t.green);
        assert_eq!(t.agent(Some("cursor")), t.blue);
        assert_eq!(t.agent(Some("gemini")), t.pink);
        assert_eq!(t.agent(Some("aider")), t.card);
        assert_eq!(t.brand(Some("aider")), None);
        assert_eq!(agent_logo(Some("Claude"), true), "\u{100000}");
        assert_eq!(agent_logo(Some("claude"), false), "✳");
        assert_eq!(agent_logo(Some("gemini"), true), "✦");
        assert_eq!(agent_logo(None, false), UNKNOWN_AGENT_LOGO);
        assert!(names_only_a_program("Claude Code"));
        assert!(names_only_a_program(" cursor agent "));
        assert!(!names_only_a_program("fix the login page"));
        assert!(same_agent("Claude", "claude"));
        let handoff: Vec<&str> = AGENT_KINDS
            .iter()
            .filter(|kind| kind.handoff)
            .map(|kind| kind.id)
            .collect();
        assert_eq!(handoff, ["claude", "codex", "cursor"]);
    }

    #[test]
    fn a_stored_logo_is_not_part_of_the_name() {
        assert_eq!(strip_leading_agent_mark("\u{100002} Cursor"), "Cursor");
        assert_eq!(strip_leading_agent_mark("\u{100001} Codex"), "Codex");
        assert_eq!(strip_leading_agent_mark("Cursor"), "Cursor");
    }

    #[test]
    fn model_names_read_like_the_mockup() {
        assert_eq!(model_label("claude-opus-5-5"), "OPUS 5.5");
        assert_eq!(model_label("claude-sonnet-5-5[1m]"), "SONNET 5.5");
        assert_eq!(model_label("claude-haiku-4-5-20251001"), "HAIKU 4.5");
        assert_eq!(model_label("gpt-6-luna"), "GPT-6 LUNA");
        assert_eq!(model_label("gpt-5"), "GPT-5");
    }

    #[test]
    fn state_colors_read_on_light_paper_and_stay_bright_on_dark() {
        let light = Brutal::new(&Palette::brutal());
        for color in [light.yellow, light.green, light.blue, light.red, light.pink] {
            let text = light.text(color, light.bg);
            let (Some(fg), Some(bg)) = (rgb(text), rgb(light.bg)) else {
                panic!("rgb colors expected");
            };
            assert!(contrast(fg, bg) >= 4.5, "{color:?} became {text:?}");
        }
        let dark = Brutal::new(&Palette::brutal_dark());
        assert_eq!(dark.text(dark.yellow, dark.bg), dark.yellow);
        // the status bar is dark in both looks: its colors stay as they are
        assert_eq!(light.text(light.green, light.status_bg), light.green);
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
