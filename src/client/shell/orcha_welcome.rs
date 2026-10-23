//! orcha: WELCOME, the first-run setup. One popup in the popup template,
//! five steps: what orcha is, a theme (the page previews it as the cursor
//! moves), where projects open (IDE, Git GUI), agent hooks, and projects to
//! bring in from other tools. Each step keeps its choice when it is left;
//! esc skips the rest. Shown once; the menu's "setup" opens it again.

use crossterm::event::{KeyCode, MouseButton, MouseEventKind};

use super::brutal::{self, Brutal};
use super::orcha_popup::{button, popup_frame, row_box, ButtonKind, OrchaPopup, PopupHits};
use super::render::{display_width, put_right_text, put_text};
use super::*;

/// Written once the setup was shown (finished or skipped).
const MARKER: &str = ".welcome-done";

/// Click targets past the rows.
const NEXT: usize = usize::MAX;
const BACK: usize = usize::MAX - 1;
const SKIP: usize = usize::MAX - 2;

const WIDTH: u16 = 80;
const BODY_HEIGHT: u16 = 18;

/// The setup was shown already: don't open it on its own again.
pub(crate) fn done() -> bool {
    crate::config::config_dir().join(MARKER).exists()
}

fn mark_done() {
    if cfg!(test) {
        return;
    }
    let dir = crate::config::config_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(MARKER), "the welcome setup was shown\n");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    Intro,
    Theme,
    Apps,
    Agents,
    Projects,
}

impl Step {
    const ALL: [Self; 5] = [
        Self::Intro,
        Self::Theme,
        Self::Apps,
        Self::Agents,
        Self::Projects,
    ];

    /// 1-based.
    fn number(self) -> usize {
        Self::ALL.iter().position(|step| *step == self).unwrap_or(0) + 1
    }

    fn name(self) -> &'static str {
        match self {
            Self::Intro => "welcome",
            Self::Theme => "theme",
            Self::Apps => "where things open",
            Self::Agents => "agent hooks",
            Self::Projects => "projects",
        }
    }

    fn next(self) -> Option<Self> {
        Self::ALL.get(self.number()).copied()
    }

    fn prev(self) -> Option<Self> {
        self.number()
            .checked_sub(2)
            .and_then(|index| Self::ALL.get(index).copied())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Welcome {
    pub(super) step: Step,
    /// The cursor in the step's list.
    pub(super) selected: usize,
    /// Index into `THEME_NAMES`.
    pub(super) theme: usize,
    /// Indexes into `welcome_apps`.
    pub(super) ide: usize,
    pub(super) git_gui: usize,
    /// Per agent hook: install it (filled when the list comes in).
    pub(super) agents: Vec<bool>,
    /// Per import source: bring its projects in (filled when the scan is in).
    pub(super) projects: Vec<bool>,
}

/// An agent hook can be checked: it is missing or old, and the agent is here.
fn installable(info: &crate::api::schema::IntegrationInfo) -> bool {
    super::settings::integration_needs_install(info)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Radio(bool),
    Check(bool),
    /// Nothing to do: installed already.
    Done,
    /// Can't be picked.
    Off,
}

struct Item {
    index: usize,
    mark: Mark,
    title: String,
    detail: String,
    /// A theme row shows that theme's colors.
    theme: Option<&'static str>,
}

enum Line {
    Header(&'static str),
    Item(Item),
}

/// The rows of the current step, with group headers.
fn lines(welcome: &Welcome, ui: &OrchaUiState) -> Vec<Line> {
    match welcome.step {
        Step::Intro => Vec::new(),
        Step::Theme => crate::config::THEME_NAMES
            .iter()
            .enumerate()
            .map(|(index, name)| {
                Line::Item(Item {
                    index,
                    mark: Mark::Radio(index == welcome.theme),
                    title: (*name).to_owned(),
                    detail: String::new(),
                    theme: Some(name),
                })
            })
            .collect(),
        Step::Apps => {
            let (ides, gits) = &ui.welcome_apps;
            let mut out = vec![Line::Header("IDE · the IDE button opens the project in")];
            out.extend(ides.iter().enumerate().map(|(index, (name, _))| {
                Line::Item(Item {
                    index,
                    mark: Mark::Radio(index == welcome.ide),
                    title: name.clone(),
                    detail: String::new(),
                    theme: None,
                })
            }));
            out.push(Line::Header("GIT · the GIT button opens the repository in"));
            out.extend(gits.iter().enumerate().map(|(index, (name, _))| {
                Line::Item(Item {
                    index: ides.len() + index,
                    mark: Mark::Radio(index == welcome.git_gui),
                    title: name.clone(),
                    detail: String::new(),
                    theme: None,
                })
            }));
            out
        }
        Step::Agents => ui
            .welcome_integrations
            .iter()
            .flatten()
            .enumerate()
            .map(|(index, info)| {
                use crate::api::schema::IntegrationState;
                let (mark, detail) = match info.state {
                    IntegrationState::Current => (Mark::Done, "installed"),
                    _ if !info.available => (Mark::Off, "not available on this device"),
                    _ if installable(info) => (
                        Mark::Check(welcome.agents.get(index).copied().unwrap_or(false)),
                        if info.state == IntegrationState::Outdated {
                            "installed · an update is ready"
                        } else {
                            "found on this computer"
                        },
                    ),
                    _ => (Mark::Off, "not installed"),
                };
                Line::Item(Item {
                    index,
                    mark,
                    title: info.label.clone(),
                    detail: detail.to_owned(),
                    theme: None,
                })
            })
            .collect(),
        Step::Projects => ui
            .import_sources
            .iter()
            .enumerate()
            .map(|(index, source)| {
                Line::Item(Item {
                    index,
                    mark: Mark::Check(welcome.projects.get(index).copied().unwrap_or(false)),
                    title: format!("{} · {}", source.name, source.projects.len()),
                    detail: source
                        .projects
                        .iter()
                        .map(|(name, _)| name.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    theme: None,
                })
            })
            .collect(),
    }
}

/// How many rows the cursor moves over in `step`.
fn item_count(step: Step, ui: &OrchaUiState) -> usize {
    match step {
        Step::Intro => 0,
        Step::Theme => crate::config::THEME_NAMES.len(),
        Step::Apps => ui.welcome_apps.0.len() + ui.welcome_apps.1.len(),
        Step::Agents => ui.welcome_integrations.as_ref().map_or(0, Vec::len),
        Step::Projects => ui.import_sources.len(),
    }
}

fn agents_to_install(welcome: &Welcome, ui: &OrchaUiState) -> usize {
    ui.welcome_integrations
        .iter()
        .flatten()
        .zip(&welcome.agents)
        .filter(|(info, on)| **on && installable(info))
        .count()
}

fn projects_to_import(welcome: &Welcome, ui: &OrchaUiState) -> usize {
    ui.import_sources
        .iter()
        .zip(&welcome.projects)
        .filter(|(_, on)| **on)
        .map(|(source, _)| source.projects.len())
        .sum()
}

/// Draws the setup over the page.
pub(super) fn render(
    buffer: &mut Buffer,
    welcome: &Welcome,
    ui: &OrchaUiState,
    config: &ClientShellConfig,
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let step = welcome.step;
    let keys: &[(&str, &str)] = match step {
        Step::Intro => &[("↵", "start"), ("esc", "skip setup")],
        Step::Theme => &[("↑↓", "try a theme"), ("↵", "keep it"), ("←", "back")],
        Step::Apps => &[
            ("↑↓", "move"),
            ("space", "pick"),
            ("↵", "next"),
            ("←", "back"),
        ],
        Step::Agents => &[("space", "check"), ("↵", "next"), ("←", "back")],
        Step::Projects => &[("space", "check"), ("↵", "finish"), ("←", "back")],
    };
    let dots: String = Step::ALL
        .iter()
        .map(|other| {
            if other.number() <= step.number() {
                '●'
            } else {
                '○'
            }
        })
        .collect();
    let context = format!(
        "{dots}  step {} of {} · {}",
        step.number(),
        Step::ALL.len(),
        step.name()
    );
    let frame = popup_frame(
        buffer,
        WIDTH,
        BODY_HEIGHT,
        if step == Step::Intro {
            "WELCOME TO ORCHA"
        } else {
            "SETUP"
        },
        &context,
        keys,
        if step == Step::Intro {
            ""
        } else {
            "esc skip setup"
        },
        t,
        hits,
    )?;
    let body = frame.body;
    if body.height < 6 {
        return Some(());
    }
    let x = body.x + 2;
    let width = body.width.saturating_sub(4);
    let (lead, sub) = match step {
        Step::Intro => (
            "orcha runs your coding agents side by side.",
            "each project is a row on the left; its agents run in tabs next to it.",
        ),
        Step::Theme => (
            "pick a look",
            "the page changes as you move; ↵ keeps it (settings can change it later)",
        ),
        Step::Apps => (
            "where things open",
            "pick with space or a click; ctrl-click on a file path in a pane opens it in the IDE",
        ),
        Step::Agents => (
            "agent hooks",
            "a small hook tells orcha when an agent works, waits for you or is done",
        ),
        Step::Projects => (
            "bring your projects",
            "found on this computer · each becomes a project in the sidebar",
        ),
    };
    put_text(
        buffer,
        x,
        body.y + 1,
        width,
        lead,
        Style::default()
            .fg(t.ink)
            .bg(t.bg)
            .add_modifier(Modifier::BOLD),
    );
    put_text(
        buffer,
        x,
        body.y + 2,
        width,
        &brutal::clip(sub, usize::from(width)),
        Style::default().fg(t.muted).bg(t.bg),
    );
    let list = Rect::new(
        body.x,
        body.y + 4,
        body.width,
        body.height.saturating_sub(6),
    );
    match step {
        Step::Intro => draw_intro(buffer, list, config, t),
        Step::Agents if ui.welcome_integrations.is_none() => {
            note(buffer, list, "looking for agents…", t);
        }
        Step::Agents if item_count(step, ui) == 0 => {
            note(
                buffer,
                list,
                "no agents to hook into: nothing to do here",
                t,
            );
        }
        Step::Projects if ui.import_scan.is_some() => {
            note(buffer, list, "looking for projects…", t);
        }
        Step::Projects if ui.import_sources.is_empty() => note(
            buffer,
            list,
            "nothing to bring in: no Orca, VS Code, Cursor or JetBrains projects found",
            t,
        ),
        _ => draw_list(
            buffer,
            list,
            &lines(welcome, ui),
            welcome.selected,
            config,
            t,
            hits,
        ),
    }

    // buttons: skip on the left, back and the primary action on the right
    let y = body.bottom().saturating_sub(1);
    let next_label = match step {
        Step::Intro => "START SETUP ↵".to_owned(),
        Step::Agents => match agents_to_install(welcome, ui) {
            0 => "NEXT ↵".to_owned(),
            count => format!("INSTALL {count} · NEXT ↵"),
        },
        Step::Projects => match projects_to_import(welcome, ui) {
            0 => "FINISH ↵".to_owned(),
            count => format!("IMPORT {count} · FINISH ↵"),
        },
        _ => "NEXT ↵".to_owned(),
    };
    let next = button(
        buffer,
        body.right().saturating_sub(1),
        y,
        &next_label,
        ButtonKind::Primary,
        t.bg,
        t,
    );
    hits.actions.push((next, NEXT));
    if step != Step::Intro {
        let back = button(buffer, next.x, y, "BACK", ButtonKind::Plain, t.bg, t);
        hits.actions.push((back, BACK));
    }
    let skip = "skip setup";
    put_text(
        buffer,
        x,
        y,
        display_width(skip),
        skip,
        Style::default()
            .fg(t.muted)
            .bg(t.bg)
            .add_modifier(Modifier::UNDERLINED),
    );
    hits.actions
        .push((Rect::new(x, y, display_width(skip), 1), SKIP));
    Some(())
}

fn note(buffer: &mut Buffer, area: Rect, text: &str, t: &Brutal) {
    let width = area.width.saturating_sub(4);
    put_text(
        buffer,
        area.x + 2,
        area.y,
        width,
        &brutal::clip(text, usize::from(width)),
        Style::default().fg(t.muted).bg(t.bg),
    );
}

/// The intro: how orcha is used, the prefix key as a chip.
fn draw_intro(buffer: &mut Buffer, area: Rect, config: &ClientShellConfig, t: &Brutal) {
    let prefix = config.keybinds.primary_prefix_label();
    let label = Style::default()
        .fg(t.ink)
        .bg(t.bg)
        .add_modifier(Modifier::BOLD);
    let text = Style::default().fg(t.muted).bg(t.bg);
    let x = area.x + 2;
    let right = area.right().saturating_sub(2);
    let rows: [(&str, Vec<(String, bool)>); 4] = [
        (
            "mouse",
            vec![(
                "click to switch · drag a border to resize · right-click for more".into(),
                false,
            )],
        ),
        (
            "keys",
            vec![
                (prefix.clone(), true),
                (" then a key · ".into(), false),
                (prefix.clone(), true),
                (" then ".into(), false),
                ("?".into(), true),
                (" lists every key".into(), false),
            ],
        ),
        (
            "agents",
            vec![(
                "the NEW button starts one; the right panel shows its changes".into(),
                false,
            )],
        ),
        (
            "next",
            vec![(
                "a theme, your IDE, agent hooks and projects · about a minute".into(),
                false,
            )],
        ),
    ];
    for (row, (name, parts)) in rows.iter().enumerate() {
        let y = area.y + (row as u16) * 2;
        if y >= area.bottom() {
            break;
        }
        put_text(buffer, x, y, 8, name, label);
        let mut cx = x + 9;
        for (part, key) in parts {
            if cx >= right {
                break;
            }
            if *key {
                cx += brutal::chip(buffer, cx, y, right - cx, part, t.ink, t.frame);
            } else {
                let width = display_width(part).min(right - cx);
                put_text(buffer, cx, y, width, part, text);
                cx += width;
            }
        }
    }
}

/// One-line rows, scrolled to keep the cursor in view.
fn draw_list(
    buffer: &mut Buffer,
    area: Rect,
    lines: &[Line],
    cursor: usize,
    config: &ClientShellConfig,
    t: &Brutal,
    hits: &mut PopupHits,
) {
    let height = usize::from(area.height);
    if height == 0 {
        return;
    }
    let cursor_line = lines
        .iter()
        .position(|line| matches!(line, Line::Item(item) if item.index == cursor))
        .unwrap_or(0);
    let start = (cursor_line + 1).saturating_sub(height);
    for (offset, line) in lines.iter().skip(start).take(height).enumerate() {
        let y = area.y + offset as u16;
        match line {
            Line::Header(text) => {
                // a gap above every group but the first
                put_text(
                    buffer,
                    area.x + 2,
                    y,
                    area.width.saturating_sub(4),
                    text,
                    Style::default()
                        .fg(t.muted)
                        .bg(t.bg)
                        .add_modifier(Modifier::BOLD),
                );
            }
            Line::Item(item) => {
                let rect = Rect::new(area.x, y, area.width, 1);
                draw_item(buffer, rect, item, item.index == cursor, config, t);
                hits.rows.push((rect, item.index));
            }
        }
    }
    // more below: say so on the last row's right
    if start + height < lines.len() {
        put_right_text(
            buffer,
            Rect::new(area.x, area.bottom() - 1, area.width.saturating_sub(2), 1),
            area.bottom() - 1,
            " ↓ more ",
            Style::default().fg(t.muted).bg(t.card),
        );
    }
}

fn draw_item(
    buffer: &mut Buffer,
    rect: Rect,
    item: &Item,
    cursor: bool,
    config: &ClientShellConfig,
    t: &Brutal,
) {
    let fill = row_box(buffer, rect, true, cursor, t);
    let y = rect.y;
    let x = rect.x + 2;
    match item.mark {
        Mark::Radio(on) => put_text(
            buffer,
            x + 1,
            y,
            1,
            if on { "●" } else { "○" },
            Style::default()
                .fg(if on { t.green } else { t.muted })
                .bg(fill),
        ),
        Mark::Check(on) => {
            brutal::chip(
                buffer,
                x,
                y,
                3,
                if on { "✓" } else { " " },
                t.chip_text,
                if on { t.green } else { t.frame },
            );
        }
        Mark::Done => put_text(
            buffer,
            x + 1,
            y,
            1,
            "✓",
            Style::default().fg(t.green).bg(fill),
        ),
        Mark::Off => put_text(
            buffer,
            x + 1,
            y,
            1,
            "–",
            Style::default().fg(t.muted).bg(fill),
        ),
    }
    let title_x = x + 5;
    let right = rect.right().saturating_sub(2);
    // a theme row: that theme's page, ink and accents, and light or dark
    let mut detail = item.detail.clone();
    let mut detail_right = right;
    if let Some(name) = item.theme {
        let palette = crate::app::client_palette_for_theme(&config.theme_runtime, name);
        let theme = Brutal::new(&palette);
        let swatches = [theme.bg, theme.ink, theme.green, theme.yellow, theme.pink];
        let width = swatches.len() as u16 * 2;
        let mut sx = right.saturating_sub(width);
        detail_right = sx.saturating_sub(1);
        for color in swatches {
            put_text(buffer, sx, y, 2, "██", Style::default().fg(color).bg(fill));
            sx += 2;
        }
        detail = if brutal::is_light(theme.bg) {
            "light".into()
        } else {
            "dark".into()
        };
    }
    let off = item.mark == Mark::Off;
    let title_width = display_width(&item.title).min(detail_right.saturating_sub(title_x));
    put_text(
        buffer,
        title_x,
        y,
        title_width,
        &item.title,
        Style::default()
            .fg(if off { t.muted } else { t.ink })
            .bg(fill)
            .add_modifier(Modifier::BOLD),
    );
    let detail_x = title_x + title_width + 2;
    if item.theme.is_some() {
        put_right_text(
            buffer,
            Rect::new(detail_x, y, detail_right.saturating_sub(detail_x), 1),
            y,
            &detail,
            Style::default().fg(t.muted).bg(fill),
        );
    } else {
        let width = detail_right.saturating_sub(detail_x);
        put_text(
            buffer,
            detail_x,
            y,
            width,
            &brutal::clip(&detail, usize::from(width)),
            Style::default().fg(t.muted).bg(fill),
        );
    }
}

impl ClientShellState {
    fn welcome(&self) -> Option<&Welcome> {
        match self.orcha_ui.popup.as_ref() {
            Some(OrchaPopup::Welcome(welcome)) => Some(welcome),
            _ => None,
        }
    }

    fn welcome_mut(&mut self) -> Option<&mut Welcome> {
        match self.orcha_ui.popup.as_mut() {
            Some(OrchaPopup::Welcome(welcome)) => Some(welcome),
            _ => None,
        }
    }

    /// Opens the setup: finds the apps, asks the server for the agent hooks
    /// and looks for projects to import, all while the intro is read.
    pub(super) fn open_welcome(&mut self, outcome: &mut ClientShellInput) {
        let ides = super::settings::installed_apps(super::settings::IDE_APPS);
        let gits = super::settings::installed_apps(super::settings::GIT_GUI_APPS);
        let position = |apps: &[(String, String)], now: &str| {
            apps.iter()
                .position(|(_, command)| command == now)
                .unwrap_or(0)
        };
        let ide = position(&ides, &self.config.orcha.ide);
        let git_gui = position(&gits, &self.config.orcha.git_gui);
        let theme_name = super::settings::normalized_theme_name(&self.config.theme_name);
        let theme = crate::config::THEME_NAMES
            .iter()
            .position(|name| *name == theme_name)
            .unwrap_or(0);
        self.orcha_ui.welcome_apps = (ides, gits);
        self.orcha_ui.welcome_theme =
            Some((self.config.theme_name.clone(), self.config.palette.clone()));
        self.orcha_ui.popup = Some(OrchaPopup::Welcome(Welcome {
            step: Step::Intro,
            selected: 0,
            theme,
            ide,
            git_gui,
            agents: Vec::new(),
            projects: Vec::new(),
        }));
        self.orcha_ui.welcome_integrations = None;
        if !self.push_endpoint_method_with_kind(
            crate::api::schema::Method::IntegrationList(crate::api::schema::EmptyParams::default()),
            PendingEndpointKind::IntegrationList,
            outcome,
        ) {
            self.orcha_ui.welcome_integrations = Some(Vec::new());
        }
        self.orcha_ui.import_sources.clear();
        if !cfg!(test) {
            self.start_import_scan();
        }
        outcome.repaint = true;
    }

    /// The server's agent hooks came in while the setup is open; the
    /// missing and old ones start checked. Returns false when it is closed.
    pub(super) fn welcome_integrations_listed(
        &mut self,
        integrations: Vec<crate::api::schema::IntegrationInfo>,
    ) -> bool {
        let Some(welcome) = self.welcome_mut() else {
            return false;
        };
        welcome.agents = integrations.iter().map(installable).collect();
        self.orcha_ui.welcome_integrations = Some(integrations);
        true
    }

    /// The import scan came in while the setup is open: every source
    /// starts checked. Returns false when it is closed.
    pub(super) fn welcome_sources_found(
        &mut self,
        sources: Vec<crate::orcha_import::ImportSource>,
    ) -> bool {
        let Some(welcome) = self.welcome_mut() else {
            return false;
        };
        welcome.projects = vec![true; sources.len()];
        self.orcha_ui.import_sources = sources;
        self.orcha_ui.import_scan = None;
        true
    }

    pub(super) fn route_welcome_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        let Some(welcome) = self.welcome() else {
            return;
        };
        let (step, selected) = (welcome.step, welcome.selected);
        let count = item_count(step, &self.orcha_ui);
        outcome.repaint = true;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.skip_welcome(),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => self.welcome_next(outcome),
            KeyCode::Left | KeyCode::Backspace | KeyCode::Char('h') => self.welcome_back(),
            KeyCode::Char(' ') if count > 0 => self.welcome_pick(selected),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab if count > 0 => {
                self.welcome_move((selected + 1) % count);
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab if count > 0 => {
                self.welcome_move((selected + count - 1) % count);
            }
            _ => {}
        }
    }

    /// Clicks: a row picks (or checks) it, the buttons act. A click
    /// outside does nothing: the setup is left with esc or "skip setup".
    pub(super) fn handle_welcome_mouse(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        outcome: &mut ClientShellInput,
    ) {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let point = (mouse.column, mouse.row);
        let hits = self.hits.orcha.popup.clone();
        outcome.repaint = true;
        if super::contains(hits.close, point) {
            self.skip_welcome();
            return;
        }
        if let Some((_, index)) = hits
            .actions
            .iter()
            .find(|(rect, _)| super::contains(*rect, point))
        {
            match *index {
                NEXT => self.welcome_next(outcome),
                BACK => self.welcome_back(),
                SKIP => self.skip_welcome(),
                _ => {}
            }
            return;
        }
        if let Some((_, index)) = hits
            .rows
            .iter()
            .find(|(rect, _)| super::contains(*rect, point))
        {
            if let Some(welcome) = self.welcome_mut() {
                welcome.selected = *index;
            }
            self.welcome_pick(*index);
        }
    }

    /// The cursor moves; on the theme step the page follows it.
    fn welcome_move(&mut self, index: usize) {
        let Some(welcome) = self.welcome_mut() else {
            return;
        };
        welcome.selected = index;
        if welcome.step == Step::Theme {
            self.welcome_pick(index);
        }
    }

    /// Space or a click on row `index`.
    fn welcome_pick(&mut self, index: usize) {
        let ide_count = self.orcha_ui.welcome_apps.0.len();
        let installable_row = self
            .orcha_ui
            .welcome_integrations
            .as_ref()
            .and_then(|list| list.get(index))
            .is_some_and(installable);
        let Some(welcome) = self.welcome_mut() else {
            return;
        };
        match welcome.step {
            Step::Intro => {}
            Step::Theme => {
                welcome.theme = index;
                if let Some(name) = crate::config::THEME_NAMES.get(index) {
                    self.config.theme_name = (*name).to_owned();
                    self.config.palette =
                        crate::app::client_palette_for_theme(&self.config.theme_runtime, name);
                }
            }
            Step::Apps if index < ide_count => welcome.ide = index,
            Step::Apps => welcome.git_gui = index - ide_count,
            Step::Agents if installable_row => {
                if let Some(on) = welcome.agents.get_mut(index) {
                    *on = !*on;
                }
            }
            Step::Agents => {}
            Step::Projects => {
                if let Some(on) = welcome.projects.get_mut(index) {
                    *on = !*on;
                }
            }
        }
    }

    /// Where the cursor starts on `step`: on the current choice.
    fn welcome_cursor(welcome: &Welcome, step: Step) -> usize {
        match step {
            Step::Theme => welcome.theme,
            Step::Apps => welcome.ide,
            _ => 0,
        }
    }

    /// ↵: keep this step's choice and go on (the last step finishes).
    fn welcome_next(&mut self, outcome: &mut ClientShellInput) {
        let Some(welcome) = self.welcome().cloned() else {
            return;
        };
        match welcome.step {
            Step::Intro => {}
            Step::Theme => self.keep_welcome_theme(welcome.theme, outcome),
            Step::Apps => {
                let (ides, gits) = &self.orcha_ui.welcome_apps;
                let ide = ides.get(welcome.ide).map(|(_, command)| command.clone());
                let git = gits
                    .get(welcome.git_gui)
                    .map(|(_, command)| command.clone());
                if let Some(command) = ide.filter(|command| *command != self.config.orcha.ide) {
                    self.save_open_app(true, command);
                }
                if let Some(command) = git.filter(|command| *command != self.config.orcha.git_gui) {
                    self.save_open_app(false, command);
                }
            }
            Step::Agents => self.install_welcome_agents(&welcome, outcome),
            Step::Projects => {
                self.import_projects(&welcome.projects, outcome);
                self.finish_welcome();
                return;
            }
        }
        let Some(step) = welcome.step.next() else {
            return;
        };
        if let Some(open) = self.welcome_mut() {
            open.selected = Self::welcome_cursor(open, step);
            open.step = step;
        }
    }

    fn welcome_back(&mut self) {
        let Some(welcome) = self.welcome_mut() else {
            return;
        };
        let Some(step) = welcome.step.prev() else {
            return;
        };
        welcome.selected = Self::welcome_cursor(welcome, step);
        welcome.step = step;
    }

    /// Saves the theme the page shows (when it changed).
    fn keep_welcome_theme(&mut self, theme: usize, outcome: &mut ClientShellInput) {
        let Some(name) = crate::config::THEME_NAMES.get(theme).copied() else {
            return;
        };
        let kept = self
            .orcha_ui
            .welcome_theme
            .as_ref()
            .map(|(kept, _)| super::settings::normalized_theme_name(kept) == name);
        if kept != Some(true)
            && !self.save_settings_edit(crate::config::ConfigEdit::Theme(name), outcome)
        {
            return;
        }
        self.orcha_ui.welcome_theme = Some((name.to_owned(), self.config.palette.clone()));
    }

    fn install_welcome_agents(&mut self, welcome: &Welcome, outcome: &mut ClientShellInput) {
        let targets: Vec<_> = self
            .orcha_ui
            .welcome_integrations
            .iter()
            .flatten()
            .zip(&welcome.agents)
            .filter(|(info, on)| **on && installable(info))
            .map(|(info, _)| info.target)
            .collect();
        for target in targets {
            if self.push_endpoint_method_with_kind(
                crate::api::schema::Method::IntegrationInstall(
                    crate::api::schema::IntegrationInstallParams { target },
                ),
                PendingEndpointKind::IntegrationInstall,
                outcome,
            ) {
                self.pending_integration_installs += 1;
            }
        }
    }

    /// esc: what was kept stays; a theme only tried goes back.
    fn skip_welcome(&mut self) {
        if let Some((name, palette)) = self.orcha_ui.welcome_theme.take() {
            self.config.theme_name = name;
            self.config.palette = palette;
        }
        self.finish_welcome();
    }

    fn finish_welcome(&mut self) {
        mark_done();
        if !cfg!(test) {
            crate::orcha_import::mark_asked();
        }
        // a scan still running must not open the import popup afterwards
        self.orcha_ui.import_scan = None;
        self.orcha_ui.welcome_theme = None;
        self.orcha_ui.popup = None;
    }
}
