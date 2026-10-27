//! orcha: a file shown read-only in a popup (FILES section clicks). The
//! file is read on a background thread; `bat` colors it when installed,
//! else it shows as plain text. Nothing here writes the file.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use ratatui::style::Color;

use super::brutal::Brutal;
use super::orcha_popup::{popup_frame, PopupHits};
use super::render::{display_width, put_text};
use super::*;

/// Bytes read from a file; a longer file shows its start.
const MAX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_LINES: usize = 20_000;
const TAB: &str = "    ";

/// A colored piece of a line; `None` takes the popup's ink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Span {
    pub text: String,
    pub fg: Option<Color>,
    pub modifier: Modifier,
}

pub(super) type Line = Vec<Span>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Content {
    Lines {
        lines: Vec<Line>,
        cut: bool,
    },
    /// Why there is nothing to show (binary, unreadable).
    Note(String),
}

pub(super) struct FileView {
    pub path: PathBuf,
    /// The tree's folder, for the path shown in the header.
    pub root: Option<PathBuf>,
    pub content: Option<Content>,
    pending: Option<Receiver<Content>>,
    pub scroll: usize,
    /// Rows of text the last frame had room for, for paging.
    pub body_rows: Cell<usize>,
}

impl FileView {
    pub(super) fn open(path: PathBuf, root: Option<PathBuf>) -> Self {
        let (tx, rx) = mpsc::channel();
        let read_path = path.clone();
        let spawned = crate::thread_spawn::spawn_named("orcha-file-view", move || {
            let _ = tx.send(read_content(&read_path));
        });
        let (content, pending) = match spawned {
            Ok(_) => (None, Some(rx)),
            Err(_) => (Some(Content::Note("could not read the file".into())), None),
        };
        Self {
            path,
            root,
            content,
            pending,
            scroll: 0,
            body_rows: Cell::new(20),
        }
    }

    /// Collects the finished read. Returns true when the content arrived.
    pub(super) fn poll(&mut self) -> bool {
        let Some(rx) = &self.pending else {
            return false;
        };
        match rx.try_recv() {
            Ok(content) => {
                self.content = Some(content);
                self.pending = None;
                true
            }
            Err(TryRecvError::Disconnected) => {
                self.content = Some(Content::Note("could not read the file".into()));
                self.pending = None;
                true
            }
            Err(TryRecvError::Empty) => false,
        }
    }

    fn line_count(&self) -> usize {
        match &self.content {
            Some(Content::Lines { lines, .. }) => lines.len(),
            _ => 0,
        }
    }

    fn max_scroll(&self) -> usize {
        self.line_count()
            .saturating_sub(self.body_rows.get().max(1))
    }

    /// Moves by `delta` lines, kept inside the file.
    pub(super) fn scroll_by(&mut self, delta: isize) {
        let next = self.scroll.saturating_add_signed(delta);
        self.scroll = next.min(self.max_scroll());
    }

    pub(super) fn page(&self) -> isize {
        isize::try_from(self.body_rows.get().saturating_sub(1).max(1)).unwrap_or(1)
    }

    pub(super) fn scroll_to_end(&mut self) {
        self.scroll = self.max_scroll();
    }
}

fn read_content(path: &Path) -> Content {
    let Ok(meta) = std::fs::metadata(path) else {
        return Content::Note("the file is gone".into());
    };
    let mut bytes = Vec::new();
    {
        use std::io::Read;
        let Ok(file) = std::fs::File::open(path) else {
            return Content::Note("the file can't be read".into());
        };
        if file.take(MAX_BYTES).read_to_end(&mut bytes).is_err() {
            return Content::Note("the file can't be read".into());
        }
    }
    if bytes.iter().take(8192).any(|byte| *byte == 0) {
        return Content::Note(format!(
            "binary file ({}) · o opens it in another app",
            human_size(meta.len())
        ));
    }
    let cut_bytes = meta.len() > MAX_BYTES;
    // bat only colors what was read whole; a cut file shows plain
    let colored = (!cut_bytes).then(|| bat_lines(path)).flatten();
    let mut lines = colored.unwrap_or_else(|| {
        String::from_utf8_lossy(&bytes)
            .lines()
            .map(parse_ansi)
            .collect()
    });
    let cut = cut_bytes || lines.len() > MAX_LINES;
    lines.truncate(MAX_LINES);
    Content::Lines { lines, cut }
}

/// The file colored by `bat`, when it is installed.
fn bat_lines(path: &Path) -> Option<Vec<Line>> {
    for program in [
        "bat",
        "batcat",
        "/opt/homebrew/bin/bat",
        "/usr/local/bin/bat",
    ] {
        let mut command = Command::new(program);
        command
            .args([
                "--color=always",
                "--style=plain",
                "--paging=never",
                "--wrap=never",
                "--",
            ])
            .arg(path)
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        crate::platform::configure_background_command(&mut command);
        let Ok(output) = command.output() else {
            continue;
        };
        if !output.status.success() {
            return None;
        }
        return Some(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(parse_ansi)
                .collect(),
        );
    }
    None
}

fn human_size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{} KB", bytes / 1024),
        _ => format!("{} MB", bytes / 1_048_576),
    }
}

/// Splits a line with SGR color codes into spans. Background colors and
/// every other escape are dropped; tabs become spaces.
pub(super) fn parse_ansi(line: &str) -> Line {
    let mut spans: Line = Vec::new();
    let mut fg: Option<Color> = None;
    let mut modifier = Modifier::empty();
    let mut text = String::new();
    let mut chars = line.chars().peekable();
    let flush = |spans: &mut Line, text: &mut String, fg: Option<Color>, modifier: Modifier| {
        if !text.is_empty() {
            spans.push(Span {
                text: std::mem::take(text),
                fg,
                modifier,
            });
        }
    };
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => {
                if chars.peek() != Some(&'[') {
                    chars.next();
                    continue;
                }
                chars.next();
                let mut params = String::new();
                let mut end = None;
                for next in chars.by_ref() {
                    if next.is_ascii_digit() || next == ';' || next == ':' {
                        params.push(next);
                    } else {
                        end = Some(next);
                        break;
                    }
                }
                if end != Some('m') {
                    continue;
                }
                flush(&mut spans, &mut text, fg, modifier);
                apply_sgr(&params, &mut fg, &mut modifier);
            }
            '\t' => text.push_str(TAB),
            ch if ch.is_control() => {}
            ch => text.push(ch),
        }
    }
    flush(&mut spans, &mut text, fg, modifier);
    spans
}

fn apply_sgr(params: &str, fg: &mut Option<Color>, modifier: &mut Modifier) {
    let codes: Vec<u16> = if params.is_empty() {
        vec![0]
    } else {
        params
            .split([';', ':'])
            .map(|code| code.parse().unwrap_or(0))
            .collect()
    };
    let mut index = 0;
    while index < codes.len() {
        match codes[index] {
            0 => {
                *fg = None;
                *modifier = Modifier::empty();
            }
            1 => *modifier |= Modifier::BOLD,
            2 => *modifier |= Modifier::DIM,
            3 => *modifier |= Modifier::ITALIC,
            4 => *modifier |= Modifier::UNDERLINED,
            22 => *modifier -= Modifier::BOLD | Modifier::DIM,
            23 => *modifier -= Modifier::ITALIC,
            24 => *modifier -= Modifier::UNDERLINED,
            code @ 30..=37 => *fg = Some(Color::Indexed((code - 30) as u8)),
            code @ 90..=97 => *fg = Some(Color::Indexed((code - 90 + 8) as u8)),
            39 => *fg = None,
            // extended colors: 38;5;n or 38;2;r;g;b (48 is the background, skipped)
            kind @ (38 | 48) => {
                let color = match codes.get(index + 1) {
                    Some(5) => {
                        let color = codes.get(index + 2).map(|n| Color::Indexed(*n as u8));
                        index += 2;
                        color
                    }
                    Some(2) => {
                        let rgb = (
                            codes.get(index + 2),
                            codes.get(index + 3),
                            codes.get(index + 4),
                        );
                        index += 4;
                        match rgb {
                            (Some(r), Some(g), Some(b)) => {
                                Some(Color::Rgb(*r as u8, *g as u8, *b as u8))
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if kind == 38 {
                    if let Some(color) = color {
                        *fg = Some(color);
                    }
                }
            }
            _ => {}
        }
        index += 1;
    }
}

/// FILE: the file's text under its name, line numbers on the left.
pub(super) fn render(
    buffer: &mut Buffer,
    view: &FileView,
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let screen = buffer.area;
    let name = view.path.file_name().map_or_else(
        || view.path.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let shown_path = view
        .root
        .as_deref()
        .and_then(|root| view.path.strip_prefix(root).ok())
        .unwrap_or(&view.path)
        .to_string_lossy()
        .into_owned();
    let total = view.line_count();
    let extra = match &view.content {
        Some(Content::Lines { cut, .. }) if total > 0 => {
            let first = view.scroll + 1;
            let last = (view.scroll + view.body_rows.get()).min(total);
            let more = if *cut { "+" } else { "" };
            format!("{first}–{last} of {total}{more}")
        }
        _ => String::new(),
    };
    let frame = popup_frame(
        buffer,
        screen.width.saturating_sub(8).max(30),
        screen.height.saturating_sub(10),
        &name,
        &shown_path,
        &[
            ("↑↓", "scroll"),
            ("PgUp PgDn", "page"),
            ("o", "open with…"),
            ("esc", "close"),
        ],
        &extra,
        t,
        hits,
    )?;
    let body = frame.body;
    view.body_rows.set(usize::from(body.height));
    let lines = match &view.content {
        None => {
            put_text(
                buffer,
                body.x + 2,
                body.y,
                body.width.saturating_sub(4),
                "reading…",
                Style::default().fg(t.muted).bg(t.bg),
            );
            return Some(());
        }
        Some(Content::Note(note)) => {
            put_text(
                buffer,
                body.x + 2,
                body.y,
                body.width.saturating_sub(4),
                note,
                Style::default().fg(t.muted).bg(t.bg),
            );
            return Some(());
        }
        Some(Content::Lines { lines, .. }) => lines,
    };
    if lines.is_empty() {
        put_text(
            buffer,
            body.x + 2,
            body.y,
            body.width.saturating_sub(4),
            "empty file",
            Style::default().fg(t.muted).bg(t.bg),
        );
        return Some(());
    }
    let gutter = u16::try_from(total.to_string().len()).unwrap_or(5) + 2;
    let text_x = body.x + 1 + gutter;
    let right = body.right().saturating_sub(1);
    let scroll = view.scroll.min(view.max_scroll());
    for (row, line) in lines
        .iter()
        .skip(scroll)
        .take(usize::from(body.height))
        .enumerate()
    {
        let y = body.y + row as u16;
        let number = (scroll + row + 1).to_string();
        put_text(
            buffer,
            body.x + 1,
            y,
            gutter,
            &format!("{number:>width$}", width = usize::from(gutter - 2)),
            Style::default().fg(t.muted).bg(t.bg),
        );
        let mut x = text_x;
        for span in line {
            if x >= right {
                break;
            }
            let room = right - x;
            let style = Style::default()
                .fg(span.fg.unwrap_or(t.ink))
                .bg(t.bg)
                .add_modifier(span.modifier);
            put_text(buffer, x, y, room, &span.text, style);
            x = x.saturating_add(display_width(&span.text));
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sgr_colors_become_spans_and_backgrounds_are_dropped() {
        let line = parse_ansi("\u{1b}[38;2;249;38;114mfn\u{1b}[0m main\u{1b}[48;5;1m()\tx");
        assert_eq!(
            line,
            vec![
                Span {
                    text: "fn".into(),
                    fg: Some(Color::Rgb(249, 38, 114)),
                    modifier: Modifier::empty(),
                },
                Span {
                    text: " main".into(),
                    fg: None,
                    modifier: Modifier::empty(),
                },
                Span {
                    text: "()    x".into(),
                    fg: None,
                    modifier: Modifier::empty(),
                },
            ]
        );
    }

    #[test]
    fn basic_and_bold_codes() {
        let line = parse_ansi("\u{1b}[1;32mok\u{1b}[22;39m!");
        assert_eq!(line[0].fg, Some(Color::Indexed(2)));
        assert_eq!(line[0].modifier, Modifier::BOLD);
        assert_eq!(line[1].fg, None);
        assert_eq!(line[1].modifier, Modifier::empty());
    }

    #[test]
    fn scrolling_stays_inside_the_file() {
        let mut view = FileView {
            path: PathBuf::from("/x"),
            root: None,
            content: Some(Content::Lines {
                lines: vec![Vec::new(); 50],
                cut: false,
            }),
            pending: None,
            scroll: 0,
            body_rows: Cell::new(20),
        };
        view.scroll_by(-5);
        assert_eq!(view.scroll, 0);
        view.scroll_by(100);
        assert_eq!(view.scroll, 30);
        view.scroll = 0;
        view.scroll_to_end();
        assert_eq!(view.scroll, 30);
    }

    #[test]
    fn binary_files_are_not_shown() {
        let path = std::env::temp_dir().join(format!("orcha-view-{}.bin", std::process::id()));
        std::fs::write(&path, [0u8, 1, 2, 3]).expect("write file");
        assert!(matches!(read_content(&path), Content::Note(note) if note.starts_with("binary")));
        let _ = std::fs::remove_file(&path);
    }
}
