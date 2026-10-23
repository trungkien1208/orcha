//! orcha: popups. Every popup follows one template (docs/orcha/SPEC.md §1
//! "Popup template"): dimmed page, heavy box with a hard shadow, a header
//! row (title, context, ✕), body rows in thin boxes (the current one heavy
//! with a yellow edge), one raised green primary action, and a footer with
//! the keys.
//!
//! Popups here: ACCOUNTS (switch which account the chat runs on), HAND
//! OFF (continue the chat with another agent), IMPORT, NEW WORKSPACE and NEW
//! AGENT. The first-run WELCOME lives in `orcha_welcome`.

use crate::api::schema::AgentStatus;

use super::accounts::Account;
use super::brutal::{self, Brutal};
use super::render::{display_width, put_right_text, put_text};
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OrchaPopup {
    /// Accounts and their fuel; acts on the agent in `pane`.
    Accounts {
        selected: usize,
        pane: Option<String>,
    },
    /// Pick a name for a new Claude account, then sign it in.
    AddAccount {
        selected: usize,
        names: Vec<String>,
        typed: String,
        pane: Option<String>,
    },
    /// Continue `pane`'s conversation with another agent.
    Handoff { selected: usize, pane: String },
    /// Bring in projects from other tools; `chosen` per source.
    Import { selected: usize, chosen: Vec<bool> },
    /// Start an agent on an account in a new tab of `workspace`.
    NewAgent {
        selected: usize,
        workspace: String,
        project: String,
        cwd: String,
    },
    /// Choose the current project or another project folder for a workspace.
    NewWorkspace {
        selected: usize,
        project: Option<String>,
    },
    /// The first-run setup (see `orcha_welcome`).
    Welcome(super::orcha_welcome::Welcome),
    /// A newer orcha is out: update now, later, or skip that version.
    Update {
        selected: usize,
        release: crate::orcha_app::Release,
    },
    /// What changed in this build, shown once after an update.
    WhatsNew { release: crate::orcha_app::Release },
    /// Ctrl+click on a file whose type has no saved app: choose one, and
    /// keep it for the extension when `remember` is on.
    OpenWith {
        selected: usize,
        path: String,
        line: u32,
        /// Lowercase extension; empty when the choice can't be kept.
        extension: String,
        choices: Vec<OpenChoice>,
        remember: bool,
    },
}

/// Where a clicked file opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OpenChoice {
    /// The app the system registered for the type (viewer, browser, ...).
    System,
    /// The folder holding the file, in the file manager.
    Reveal,
    /// An editor: `command` opens it (`{path}`), empty for the first found.
    Editor { name: String, command: String },
}

impl OpenChoice {
    /// The value kept in `[orcha.open_with]`.
    pub(crate) fn config_value(&self) -> String {
        match self {
            Self::System => "system".to_owned(),
            Self::Reveal => "folder".to_owned(),
            Self::Editor { command, .. } if command.is_empty() => "editor".to_owned(),
            Self::Editor { command, .. } => format!("editor:{command}"),
        }
    }

    pub(crate) fn from_config_value(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "folder" => Some(Self::Reveal),
            "editor" => Some(Self::Editor {
                name: String::new(),
                command: String::new(),
            }),
            other => other.strip_prefix("editor:").map(|command| Self::Editor {
                name: String::new(),
                command: command.to_owned(),
            }),
        }
    }

    fn title(&self) -> String {
        match self {
            Self::System => "Default app".to_owned(),
            Self::Reveal => "Show in folder".to_owned(),
            Self::Editor { name, command } if command.is_empty() || name.is_empty() => {
                "Editor".to_owned()
            }
            Self::Editor { name, .. } => name.clone(),
        }
    }

    fn detail(&self, extension: &str) -> String {
        match self {
            Self::System if extension.is_empty() => {
                "the app this computer uses for the file".to_owned()
            }
            Self::System => format!("the app this computer uses for .{extension} files"),
            Self::Reveal => "open the folder and pick the file yourself".to_owned(),
            Self::Editor { command, .. } if command.is_empty() => {
                "your code editor, at the line the agent printed".to_owned()
            }
            Self::Editor { .. } => "open the file in this editor".to_owned(),
        }
    }
}

/// Click targets of the open popup.
#[derive(Debug, Default, Clone)]
pub(super) struct PopupHits {
    pub area: Rect,
    pub close: Rect,
    /// (row, index)
    pub rows: Vec<(Rect, usize)>,
    /// (button, index)
    pub actions: Vec<(Rect, usize)>,
}

/// (agent kind, mode, label)
pub(super) const HANDOFF_TARGETS: [(&str, &str, &str); 6] = [
    ("codex", "full", "Codex · whole conversation"),
    ("codex", "summary", "Codex · summarize first, then continue"),
    ("claude", "full", "Claude · whole conversation"),
    (
        "claude",
        "summary",
        "Claude · summarize first, then continue",
    ),
    ("cursor", "full", "Cursor · whole conversation"),
    (
        "cursor",
        "summary",
        "Cursor · summarize first, then continue",
    ),
];

pub(super) struct Frame {
    pub(super) body: Rect,
}

/// The template's shell: dims the page, draws the box, header and footer.
pub(super) fn popup_frame(
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
    let width = width.min(screen.width.saturating_sub(4));
    let height = (body_height + 6).min(screen.height.saturating_sub(2));
    if width < 30 || height < 8 {
        return None;
    }
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
    for (key, what) in keys {
        let key_width = display_width(key) + 2;
        let what_width = display_width(what);
        if x + key_width + what_width + 3 > inner_x + inner_w {
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
pub(super) fn row_box(
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
                Style::default().fg(t.yellow).bg(fill),
            );
        }
    }
    fill
}

/// A button: the primary one raised green, the others outlined, a
/// disabled one muted.
pub(super) fn button(
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
pub(super) enum ButtonKind {
    Primary,
    Plain,
}

/// Names offered for a new Claude account (`oc personal`, …).
pub(super) const ADD_ACCOUNT_NAMES: [&str; 5] = ["personal", "work", "side", "team", "client"];

/// Suggestions that are not already an account id.
pub(super) fn account_name_choices(accounts: &[Account]) -> Vec<String> {
    let taken: std::collections::HashSet<&str> = accounts
        .iter()
        .map(|account| {
            account
                .id
                .strip_prefix("claude-")
                .unwrap_or(account.id.as_str())
        })
        .collect();
    ADD_ACCOUNT_NAMES
        .iter()
        .filter(|name| !taken.contains(*name))
        .map(|name| (*name).to_owned())
        .collect()
}

/// The account a switch should go to: most fuel among the others of the
/// current one's kind.
/// Where to continue: another account of the same agent while it has fuel
/// (the chat goes on as is), else whichever agent has the most.
pub(super) fn best_account(accounts: &[Account]) -> Option<usize> {
    let current = accounts.iter().find(|account| account.current)?;
    let usable = |account: &&Account| !account.current && !account.windows.is_empty();
    let most = |same_kind: bool| {
        accounts
            .iter()
            .enumerate()
            .filter(|(_, account)| usable(account) && (!same_kind || account.kind == current.kind))
            .max_by_key(|(_, account)| account.lowest().unwrap_or(0))
    };
    most(true)
        .filter(|(_, account)| account.lowest().unwrap_or(0) >= 20)
        .or_else(|| most(false))
        .map(|(index, _)| index)
}

/// Name, gauges, then when each window resets. A thin rule sits between rows.
const ROW: u16 = 3;

fn rows_height(rows: usize) -> u16 {
    (rows.max(1) as u16) * (ROW + 1) - 1
}

fn render_accounts(
    buffer: &mut Buffer,
    selected: usize,
    pane: Option<&str>,
    accounts: Option<&[Account]>,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let accounts = accounts.unwrap_or(&[]);
    let context = pane
        .and_then(|pane| snapshot.agents.iter().find(|agent| agent.pane_id == pane))
        .map(|agent| {
            let project = snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.workspace_id == agent.workspace_id)
                .map(|workspace| workspace.label.clone())
                .unwrap_or_default();
            format!(
                "{} {} · {project}",
                brutal::agent_logo(agent.agent.as_deref(), config.icon_font),
                brutal::agent_label(agent)
            )
        })
        .unwrap_or_else(|| "no agent in the focused pane".to_owned());
    // running low: say how low
    let low = accounts
        .iter()
        .find(|account| account.current)
        .and_then(|account| account.lowest().map(|left| (account, left)))
        .filter(|(_, left)| *left < config.orcha.low_fuel_percent.max(20));
    let context = match low {
        Some((account, left)) => format!("{context} · ⚠ {} {left}% left", account.label),
        None => context,
    };
    let frame = popup_frame(
        buffer,
        88,
        rows_height(accounts.len().max(1)).saturating_add(2),
        "CONTINUE ON",
        &context,
        &[
            ("↵", "continue"),
            ("alt ↵", "summary first"),
            ("esc", "not now"),
        ],
        "",
        t,
        hits,
    )?;
    if accounts.is_empty() {
        put_text(
            buffer,
            frame.body.x + 2,
            frame.body.y,
            frame.body.width.saturating_sub(4),
            "loading accounts…  (needs [orcha] accounts_command)",
            Style::default().fg(t.muted).bg(t.bg),
        );
    }
    let best = best_account(accounts);
    let current_kind = accounts
        .iter()
        .find(|account| account.current)
        .map(|account| account.kind.clone());
    let list_bottom = frame.body.bottom().saturating_sub(2);
    let mut y = frame.body.y;
    for (index, account) in accounts.iter().enumerate() {
        if y + ROW > list_bottom {
            break;
        }
        let rect = Rect::new(frame.body.x, y, frame.body.width, ROW);
        // another agent: the chat is handed off; the same agent: it goes on as is
        let hand_off = current_kind
            .as_deref()
            .is_some_and(|kind| kind != account.kind);
        let badge = if account.current {
            Some(("IN USE", t.ink, t.frame))
        } else if Some(index) == best {
            Some(("BEST", t.chip_text, t.green))
        } else if hand_off {
            Some(("HAND-OFF", t.muted, t.bg))
        } else {
            None
        };
        let action = if account.signed_out() {
            Some(("SIGN IN", ButtonKind::Plain))
        } else if account.current {
            None
        } else if index == selected {
            Some(("CONTINUE ↵", ButtonKind::Primary))
        } else {
            Some(("CONTINUE", ButtonKind::Plain))
        };
        account_row(
            buffer,
            rect,
            account,
            index,
            AccountRow {
                first: index == 0,
                cursor: index == selected,
                badge,
                action,
            },
            config,
            t,
            hits,
        );
        y += ROW + 1;
    }
    let _ = y;
    let index = accounts.len();
    let rect = Rect::new(
        frame.body.x,
        frame.body.bottom().saturating_sub(1),
        frame.body.width,
        1,
    );
    let cursor = index == selected;
    let fill = row_box(buffer, rect, accounts.is_empty(), cursor, t);
    put_text(
        buffer,
        rect.x + 2,
        rect.y,
        20,
        "+  ADD ACCOUNT",
        Style::default()
            .fg(t.ink)
            .bg(fill)
            .add_modifier(Modifier::BOLD),
    );
    let action = button(
        buffer,
        rect.right() - 1,
        rect.y,
        "ADD",
        if cursor {
            ButtonKind::Primary
        } else {
            ButtonKind::Plain
        },
        fill,
        t,
    );
    hits.actions.push((action, index));
    hits.rows.push((rect, index));
    Some(())
}

/// ADD ACCOUNT: a suggested name, or whatever was typed.
fn render_add_account(
    buffer: &mut Buffer,
    selected: usize,
    names: &[String],
    typed: &str,
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let rows = names.len() + 1;
    let frame = popup_frame(
        buffer,
        64,
        rows.max(1) as u16 * 2,
        "ADD ACCOUNT",
        "a Claude account, then sign in",
        &[("↵", "add"), ("type", "a name"), ("esc", "back")],
        "",
        t,
        hits,
    )?;
    let mut y = frame.body.y;
    for (index, name) in names.iter().enumerate() {
        if y >= frame.body.bottom() {
            break;
        }
        let rect = Rect::new(frame.body.x, y, frame.body.width, 1);
        let cursor = index == selected;
        let fill = row_box(buffer, rect, index == 0, cursor, t);
        put_text(
            buffer,
            rect.x + 2,
            rect.y,
            rect.width.saturating_sub(16),
            &format!("oc {name}"),
            Style::default()
                .fg(t.ink)
                .bg(fill)
                .add_modifier(Modifier::BOLD),
        );
        let action = button(
            buffer,
            rect.right() - 1,
            rect.y,
            "ADD",
            if cursor {
                ButtonKind::Primary
            } else {
                ButtonKind::Plain
            },
            fill,
            t,
        );
        hits.actions.push((action, index));
        hits.rows.push((rect, index));
        y += 2;
    }
    if y < frame.body.bottom() {
        let index = names.len();
        let rect = Rect::new(frame.body.x, y, frame.body.width, 1);
        let cursor = index == selected;
        let fill = row_box(buffer, rect, names.is_empty(), cursor, t);
        let shown = if typed.is_empty() {
            "type a name".to_owned()
        } else {
            format!("oc {typed}")
        };
        put_text(
            buffer,
            rect.x + 2,
            rect.y,
            rect.width.saturating_sub(16),
            &shown,
            Style::default()
                .fg(if typed.is_empty() { t.muted } else { t.ink })
                .bg(fill),
        );
        if !typed.is_empty() {
            let action = button(
                buffer,
                rect.right() - 1,
                rect.y,
                "ADD",
                if cursor {
                    ButtonKind::Primary
                } else {
                    ButtonKind::Plain
                },
                fill,
                t,
            );
            hits.actions.push((action, index));
        }
        hits.rows.push((rect, index));
    }
    Some(())
}

struct AccountRow<'a> {
    first: bool,
    cursor: bool,
    badge: Option<(&'a str, ratatui::style::Color, ratatui::style::Color)>,
    action: Option<(&'a str, ButtonKind)>,
}

/// Fuel columns: "WEEK " + 10 marks a cell apart + " 100%".
const GAUGE_COLS: u16 = 5 + 19 + 5;

/// One account: logo chip, name and badge, action button; email and the
/// fuel gauges under them.
#[allow(clippy::too_many_arguments)] // one row, all inputs are distinct facts
fn account_row(
    buffer: &mut Buffer,
    rect: Rect,
    account: &Account,
    index: usize,
    row: AccountRow<'_>,
    config: &ClientShellConfig,
    t: &Brutal,
    hits: &mut PopupHits,
) {
    let y = rect.y;
    let fill = row_box(buffer, rect, row.first, row.cursor, t);
    let x = rect.x + 2;
    let right = rect.right().saturating_sub(2);
    let logo = brutal::agent_logo(Some(&account.kind), config.icon_font);
    let chip_width = brutal::chip(
        buffer,
        x,
        y,
        4,
        logo,
        t.chip_text,
        t.agent(Some(&account.kind)),
    );
    let name_x = x + chip_width + 1;
    let name_width = display_width(&account.label).min(30);
    put_text(
        buffer,
        name_x,
        y,
        name_width,
        &account.label,
        Style::default()
            .fg(t.ink)
            .bg(fill)
            .add_modifier(Modifier::BOLD),
    );
    if let Some((text, fg, bg)) = row.badge {
        brutal::chip(buffer, name_x + name_width + 1, y, 12, text, fg, bg);
    }
    if let Some((label, kind)) = row.action {
        let action = button(buffer, right + 1, y, label, kind, fill, t);
        hits.actions.push((action, index));
    }
    // second line: email (or why there is no fuel), then the gauges
    let shown = account.windows.iter().take(2).count() as u16;
    let gauges_width = if shown == 0 {
        4
    } else {
        shown * GAUGE_COLS + (shown - 1) * 2
    };
    let note = if account.signed_out() {
        account
            .error
            .clone()
            .or_else(|| account.email.clone())
            .unwrap_or_default()
    } else {
        match (&account.email, &account.error) {
            (Some(email), _) => email.clone(),
            (None, Some(error)) => error.clone(),
            _ => String::new(),
        }
    };
    let note_x = name_x;
    let note_width = right.saturating_sub(note_x + gauges_width + 2);
    put_text(
        buffer,
        note_x,
        y + 1,
        note_width,
        &brutal::clip(&note, usize::from(note_width)),
        Style::default().fg(t.muted).bg(fill),
    );
    if shown == 0 {
        // signed in but no fuel numbers right now
        put_right_text(
            buffer,
            Rect::new(rect.x, y + 1, right - rect.x, 1),
            y + 1,
            "—",
            Style::default().fg(t.muted).bg(fill),
        );
    }
    let mut gx = right.saturating_sub(gauges_width);
    for window in account.windows.iter().take(2) {
        put_text(
            buffer,
            gx,
            y + 1,
            5,
            &window.label,
            Style::default().fg(t.muted).bg(fill),
        );
        super::git_panel::draw_gauge(buffer, gx + 5, y + 1, window.left, fill, t);
        put_text(
            buffer,
            gx + 24,
            y + 1,
            5,
            &format!("{:>4}%", window.left),
            Style::default()
                .fg(if window.left < 20 { t.red } else { t.ink })
                .bg(fill)
                .add_modifier(Modifier::BOLD),
        );
        gx += GAUGE_COLS + 2;
    }
    let resets: Vec<String> = account
        .windows
        .iter()
        .take(2)
        .filter_map(|window| {
            window
                .reset
                .as_deref()
                .filter(|reset| !reset.is_empty())
                .map(|reset| {
                    format!(
                        "{} {}",
                        window.label.trim(),
                        super::accounts::resets_in(reset)
                    )
                })
        })
        .collect();
    if !resets.is_empty() {
        put_right_text(
            buffer,
            Rect::new(rect.x, y + 2, right.saturating_sub(rect.x), 1),
            y + 2,
            &resets.join("   "),
            Style::default().fg(t.muted).bg(fill),
        );
    }
    hits.rows.push((rect, index));
}

/// NEW AGENT: which agent and account starts in a new tab of the project.
/// Rows are the accounts (with their fuel), then a plain shell.
fn render_new_agent(
    buffer: &mut Buffer,
    selected: usize,
    project: &str,
    accounts: Option<&[Account]>,
    config: &ClientShellConfig,
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let accounts = accounts.unwrap_or(&[]);
    let frame = popup_frame(
        buffer,
        88,
        rows_height(accounts.len() + 1),
        "NEW AGENT",
        &format!("new tab in {project}"),
        &[("↵", "start"), ("↑↓", "choose"), ("P", "new project")],
        "esc close",
        t,
        hits,
    )?;
    let most = accounts
        .iter()
        .enumerate()
        .filter(|(_, account)| !account.signed_out())
        .max_by_key(|(_, account)| account.lowest().unwrap_or(0))
        .map(|(index, _)| index);
    let mut y = frame.body.y;
    for (index, account) in accounts.iter().enumerate() {
        if y + ROW > frame.body.bottom() {
            break;
        }
        let rect = Rect::new(frame.body.x, y, frame.body.width, ROW);
        let action = if account.signed_out() {
            ("SIGN IN", ButtonKind::Plain)
        } else if index == selected {
            ("START ↵", ButtonKind::Primary)
        } else {
            ("START", ButtonKind::Plain)
        };
        let badge = (Some(index) == most).then_some(("MOST FUEL", t.chip_text, t.green));
        account_row(
            buffer,
            rect,
            account,
            index,
            AccountRow {
                first: index == 0,
                cursor: index == selected,
                badge,
                action: Some(action),
            },
            config,
            t,
            hits,
        );
        y += ROW + 1;
    }
    // a plain shell, last
    let index = accounts.len();
    if y + ROW <= frame.body.bottom() {
        let rect = Rect::new(frame.body.x, y, frame.body.width, ROW);
        let fill = row_box(buffer, rect, index == 0, index == selected, t);
        let chip_width = brutal::chip(buffer, rect.x + 2, y, 4, "▶", t.chip_text, t.rail_ahead);
        put_text(
            buffer,
            rect.x + 3 + chip_width,
            y,
            20,
            "Shell",
            Style::default()
                .fg(t.ink)
                .bg(fill)
                .add_modifier(Modifier::BOLD),
        );
        put_text(
            buffer,
            rect.x + 3 + chip_width,
            y + 1,
            30,
            "no agent, just a terminal",
            Style::default().fg(t.muted).bg(fill),
        );
        let kind = if index == selected {
            ButtonKind::Primary
        } else {
            ButtonKind::Plain
        };
        let action = button(buffer, rect.right() - 1, y, "OPEN", kind, fill, t);
        hits.actions.push((action, index));
        hits.rows.push((rect, index));
    }
    Some(())
}

fn render_new_workspace(
    buffer: &mut Buffer,
    selected: usize,
    project: Option<&str>,
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let frame = popup_frame(
        buffer,
        72,
        rows_height(3),
        "NEW WORKSPACE",
        "choose where the project starts",
        &[("↵", "choose"), ("↑↓", "move")],
        "esc close",
        t,
        hits,
    )?;
    let options = [
        (
            "Current project",
            project
                .map(|project| format!("create a workspace in {project}"))
                .unwrap_or_else(|| "no project is selected".to_owned()),
            if project.is_some() {
                "CREATE"
            } else {
                "CHOOSE"
            },
        ),
        (
            "Another folder",
            "browse for a project folder on this computer".to_owned(),
            "BROWSE",
        ),
        (
            "Enter folder path",
            "paste an absolute project folder path".to_owned(),
            "CHOOSE",
        ),
    ];
    let mut y = frame.body.y;
    for (index, (title, detail, action_label)) in options.iter().enumerate() {
        if y + ROW > frame.body.bottom() {
            break;
        }
        let rect = Rect::new(frame.body.x, y, frame.body.width, ROW);
        let fill = row_box(buffer, rect, index == 0, index == selected, t);
        put_text(
            buffer,
            rect.x + 2,
            y,
            rect.width.saturating_sub(18),
            title,
            Style::default()
                .fg(t.ink)
                .bg(fill)
                .add_modifier(Modifier::BOLD),
        );
        put_text(
            buffer,
            rect.x + 2,
            y + 1,
            rect.width.saturating_sub(18),
            detail,
            Style::default().fg(t.muted).bg(fill),
        );
        let action = button(
            buffer,
            rect.right() - 1,
            y,
            action_label,
            if index == selected {
                ButtonKind::Primary
            } else {
                ButtonKind::Plain
            },
            fill,
            t,
        );
        hits.actions.push((action, index));
        hits.rows.push((rect, index));
        y += ROW + 1;
    }
    Some(())
}

fn render_handoff(
    buffer: &mut Buffer,
    selected: usize,
    pane: &str,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let agent = snapshot.agents.iter().find(|agent| agent.pane_id == pane);
    let from = agent
        .and_then(|agent| agent.agent.clone())
        .unwrap_or_default();
    let context = agent
        .map(|agent| {
            format!(
                "from {} {} · {}",
                brutal::agent_logo(agent.agent.as_deref(), config.icon_font),
                brutal::agent_label(agent),
                brutal::agent_task(agent)
            )
        })
        .unwrap_or_default();
    let frame = popup_frame(
        buffer,
        72,
        rows_height(HANDOFF_TARGETS.len()),
        "HAND OFF",
        &context,
        &[("↵", "continue there"), ("↑↓", "choose")],
        "esc close",
        t,
        hits,
    )?;
    let mut y = frame.body.y;
    for (index, (kind, mode, label)) in HANDOFF_TARGETS.iter().enumerate() {
        if y + ROW > frame.body.bottom() {
            break;
        }
        let rect = Rect::new(frame.body.x, y, frame.body.width, ROW);
        let same = *kind == from;
        let fill = row_box(buffer, rect, index == 0, index == selected, t);
        let x = rect.x + 2;
        let chip_width = brutal::chip(
            buffer,
            x,
            y,
            4,
            brutal::agent_logo(Some(kind), config.icon_font),
            t.chip_text,
            t.agent(Some(kind)),
        );
        put_text(
            buffer,
            x + chip_width + 1,
            y,
            rect.width.saturating_sub(chip_width + 18),
            label,
            Style::default()
                .fg(if same { t.muted } else { t.ink })
                .bg(fill)
                .add_modifier(Modifier::BOLD),
        );
        put_text(
            buffer,
            x + chip_width + 1,
            y + 1,
            rect.width.saturating_sub(chip_width + 18),
            if *mode == "summary" {
                "a short summary opens the new chat"
            } else {
                "the whole conversation is replayed"
            },
            Style::default().fg(t.muted).bg(fill),
        );
        let kind_button = if index == selected {
            ButtonKind::Primary
        } else {
            ButtonKind::Plain
        };
        let action = button(
            buffer,
            rect.right() - 1,
            y,
            if index == selected {
                "CONTINUE ↵"
            } else {
                "CONTINUE"
            },
            kind_button,
            fill,
            t,
        );
        hits.actions.push((action, index));
        hits.rows.push((rect, index));
        y += ROW + 1;
    }
    Some(())
}

/// IMPORT: projects from the tools used before, one row per tool, each
/// with a check; the primary button brings the checked ones in.
fn render_import(
    buffer: &mut Buffer,
    selected: usize,
    chosen: &[bool],
    sources: &[crate::orcha_import::ImportSource],
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let count: usize = sources
        .iter()
        .zip(chosen)
        .filter(|(_, on)| **on)
        .map(|(source, _)| source.projects.len())
        .sum();
    let frame = popup_frame(
        buffer,
        76,
        rows_height(sources.len()) + 2,
        "IMPORT PROJECTS",
        "found on this Mac · each becomes a project in the sidebar",
        &[("↵", "import"), ("space", "check"), ("esc", "not now")],
        "",
        t,
        hits,
    )?;
    if sources.is_empty() {
        put_text(
            buffer,
            frame.body.x + 2,
            frame.body.y,
            frame.body.width.saturating_sub(4),
            "nothing to import: no Orca, VS Code, Cursor or JetBrains projects found",
            Style::default().fg(t.muted).bg(t.bg),
        );
        return Some(());
    }
    let mut y = frame.body.y;
    for (index, source) in sources.iter().enumerate() {
        if y + ROW > frame.body.bottom() {
            break;
        }
        let rect = Rect::new(frame.body.x, y, frame.body.width, ROW);
        let fill = row_box(buffer, rect, index == 0, index == selected, t);
        let on = chosen.get(index).copied().unwrap_or(false);
        let check = brutal::chip(
            buffer,
            rect.x + 2,
            y,
            5,
            if on { "✓" } else { " " },
            t.chip_text,
            if on { t.green } else { t.frame },
        );
        let x = rect.x + 3 + check;
        let title = format!("{} · {} projects", source.name, source.projects.len());
        put_text(
            buffer,
            x,
            y,
            display_width(&title),
            &title,
            Style::default()
                .fg(t.ink)
                .bg(fill)
                .add_modifier(Modifier::BOLD),
        );
        let names = source
            .projects
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let width = rect.right().saturating_sub(x + 2);
        put_text(
            buffer,
            x,
            y + 1,
            width,
            &brutal::clip(&names, usize::from(width)),
            Style::default().fg(t.muted).bg(fill),
        );
        hits.actions
            .push((Rect::new(rect.x + 2, y, check, 1), index));
        hits.rows.push((rect, index));
        y += ROW + 1;
    }
    // the primary action, under the list
    let label = if count == 0 {
        "IMPORT".to_owned()
    } else {
        format!("IMPORT {count} ↵")
    };
    let action = button(
        buffer,
        frame.body.right() - 1,
        frame.body.bottom().saturating_sub(1),
        &label,
        if count == 0 {
            ButtonKind::Plain
        } else {
            ButtonKind::Primary
        },
        t.bg,
        t,
    );
    hits.actions.push((action, sources.len()));
    Some(())
}

/// The highlights of a release, one bullet per line, from `y`.
fn render_notes(buffer: &mut Buffer, body: Rect, y: u16, notes: &[String], t: &Brutal) -> u16 {
    let mut y = y;
    for note in notes {
        if y >= body.bottom() {
            break;
        }
        let width = body.width.saturating_sub(6);
        put_text(
            buffer,
            body.x + 2,
            y,
            2,
            "•",
            Style::default().fg(t.green).bg(t.bg),
        );
        put_text(
            buffer,
            body.x + 4,
            y,
            width,
            &brutal::clip(note, usize::from(width)),
            Style::default().fg(t.ink).bg(t.bg),
        );
        y += 1;
    }
    y
}

/// UPDATE: what the new version brings, then update now, later or skip it.
fn render_update(
    buffer: &mut Buffer,
    selected: usize,
    release: &crate::orcha_app::Release,
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let notes = u16::try_from(release.notes.len()).unwrap_or(0);
    let gap = u16::from(notes > 0);
    let frame = popup_frame(
        buffer,
        72,
        notes + gap + rows_height(3),
        "UPDATE",
        &format!("orcha {} is out", release.version),
        &[("↵", "choose"), ("↑↓", "move")],
        "esc later",
        t,
        hits,
    )?;
    let mut y = render_notes(buffer, frame.body, frame.body.y, &release.notes, t) + gap;
    let options = [
        (
            "Update now".to_owned(),
            "installs in the background; your agents keep running".to_owned(),
            "UPDATE",
        ),
        (
            "Later".to_owned(),
            "the green button in the header stays until you update".to_owned(),
            "LATER",
        ),
        (
            format!("Skip {}", release.version),
            "no reminder until a newer version".to_owned(),
            "SKIP",
        ),
    ];
    for (index, (title, detail, label)) in options.iter().enumerate() {
        if y + ROW > frame.body.bottom() {
            break;
        }
        let rect = Rect::new(frame.body.x, y, frame.body.width, ROW);
        let fill = row_box(buffer, rect, index == 0, index == selected, t);
        put_text(
            buffer,
            rect.x + 2,
            y,
            rect.width.saturating_sub(16),
            title,
            Style::default()
                .fg(t.ink)
                .bg(fill)
                .add_modifier(Modifier::BOLD),
        );
        put_text(
            buffer,
            rect.x + 2,
            y + 1,
            rect.width.saturating_sub(16),
            detail,
            Style::default().fg(t.muted).bg(fill),
        );
        let action = button(
            buffer,
            rect.right() - 1,
            y,
            label,
            if index == selected {
                ButtonKind::Primary
            } else {
                ButtonKind::Plain
            },
            fill,
            t,
        );
        hits.actions.push((action, index));
        hits.rows.push((rect, index));
        y += ROW + 1;
    }
    Some(())
}

/// WHAT'S NEW: the highlights of the version just installed, once.
fn render_whats_new(
    buffer: &mut Buffer,
    release: &crate::orcha_app::Release,
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let notes = u16::try_from(release.notes.len()).unwrap_or(0);
    let frame = popup_frame(
        buffer,
        64,
        notes + 2,
        "WHAT'S NEW",
        &format!("orcha {}", release.version),
        &[("↵", "got it")],
        "",
        t,
        hits,
    )?;
    render_notes(buffer, frame.body, frame.body.y, &release.notes, t);
    let action = button(
        buffer,
        frame.body.right() - 1,
        frame.body.bottom().saturating_sub(1),
        "GOT IT ↵",
        ButtonKind::Primary,
        t.bg,
        t,
    );
    hits.actions.push((action, 0));
    Some(())
}

/// OPEN WITH: the suggested app first, then the other choices; a check
/// under the list keeps the choice for the extension.
#[allow(clippy::too_many_arguments)] // Render inputs are independent view facts.
fn render_open_with(
    buffer: &mut Buffer,
    selected: usize,
    path: &str,
    extension: &str,
    choices: &[OpenChoice],
    remember: bool,
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let name = std::path::Path::new(path).file_name().map_or_else(
        || path.to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let frame = popup_frame(
        buffer,
        72,
        rows_height(choices.len()) + 2,
        "OPEN WITH",
        &name,
        &[("↵", "open"), ("↑↓", "move"), ("space", "remember")],
        "esc close",
        t,
        hits,
    )?;
    let mut y = frame.body.y;
    for (index, choice) in choices.iter().enumerate() {
        if y + ROW > frame.body.bottom().saturating_sub(1) {
            break;
        }
        let rect = Rect::new(frame.body.x, y, frame.body.width, ROW);
        let fill = row_box(buffer, rect, index == 0, index == selected, t);
        let mut title = choice.title();
        if index == 0 {
            title.push_str("  · suggested");
        }
        put_text(
            buffer,
            rect.x + 2,
            y,
            rect.width.saturating_sub(14),
            &title,
            Style::default()
                .fg(t.ink)
                .bg(fill)
                .add_modifier(Modifier::BOLD),
        );
        put_text(
            buffer,
            rect.x + 2,
            y + 1,
            rect.width.saturating_sub(14),
            &choice.detail(extension),
            Style::default().fg(t.muted).bg(fill),
        );
        let action = button(
            buffer,
            rect.right() - 1,
            y,
            "OPEN",
            if index == selected {
                ButtonKind::Primary
            } else {
                ButtonKind::Plain
            },
            fill,
            t,
        );
        hits.actions.push((action, index));
        hits.rows.push((rect, index));
        y += ROW + 1;
    }
    if !extension.is_empty() {
        let y = frame.body.bottom().saturating_sub(1);
        let check = brutal::chip(
            buffer,
            frame.body.x + 2,
            y,
            5,
            if remember { "✓" } else { " " },
            t.chip_text,
            if remember { t.green } else { t.frame },
        );
        let label = format!("always open .{extension} files this way");
        put_text(
            buffer,
            frame.body.x + 3 + check,
            y,
            display_width(&label),
            &label,
            Style::default().fg(t.ink).bg(t.bg),
        );
        hits.actions.push((
            Rect::new(frame.body.x + 2, y, check + 1 + display_width(&label), 1),
            choices.len(),
        ));
    }
    Some(())
}

/// Draws the open popup over the composed frame.
pub(super) fn render_popup(
    buffer: &mut Buffer,
    popup: &OrchaPopup,
    accounts: Option<&[Account]>,
    ui: &OrchaUiState,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
) -> PopupHits {
    let t = Brutal::new(&config.palette);
    let mut hits = PopupHits::default();
    match popup {
        OrchaPopup::Accounts { selected, pane } => {
            let _ = render_accounts(
                buffer,
                *selected,
                pane.as_deref(),
                accounts,
                snapshot,
                config,
                &t,
                &mut hits,
            );
        }
        OrchaPopup::AddAccount {
            selected,
            names,
            typed,
            ..
        } => {
            let _ = render_add_account(buffer, *selected, names, typed, &t, &mut hits);
        }
        OrchaPopup::Handoff { selected, pane } => {
            let _ = render_handoff(buffer, *selected, pane, snapshot, config, &t, &mut hits);
        }
        OrchaPopup::Import { selected, chosen } => {
            let _ = render_import(buffer, *selected, chosen, &ui.import_sources, &t, &mut hits);
        }
        OrchaPopup::Welcome(welcome) => {
            let _ = super::orcha_welcome::render(buffer, welcome, ui, config, &t, &mut hits);
        }
        OrchaPopup::NewAgent {
            selected, project, ..
        } => {
            let _ = render_new_agent(buffer, *selected, project, accounts, config, &t, &mut hits);
        }
        OrchaPopup::NewWorkspace { selected, project } => {
            let _ = render_new_workspace(buffer, *selected, project.as_deref(), &t, &mut hits);
        }
        OrchaPopup::Update { selected, release } => {
            let _ = render_update(buffer, *selected, release, &t, &mut hits);
        }
        OrchaPopup::WhatsNew { release } => {
            let _ = render_whats_new(buffer, release, &t, &mut hits);
        }
        OrchaPopup::OpenWith {
            selected,
            path,
            extension,
            choices,
            remember,
            ..
        } => {
            let _ = render_open_with(
                buffer, *selected, path, extension, choices, *remember, &t, &mut hits,
            );
        }
    }
    hits
}

/// The agent a popup acts on: the focused pane's.
pub(super) fn focused_agent_pane(snapshot: &ClientShellSnapshot) -> Option<String> {
    let focused = snapshot.focused_pane_id.as_deref()?;
    snapshot
        .agents
        .iter()
        .find(|agent| agent.pane_id == focused && agent.agent_status != AgentStatus::Unknown)
        .map(|agent| agent.pane_id.clone())
        .or_else(|| Some(focused.to_owned()))
}
