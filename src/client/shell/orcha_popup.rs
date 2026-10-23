//! orcha: popups. Every popup follows one template (docs/orcha/SPEC.md §1
//! "Popup template"): dimmed page, heavy box with a hard shadow, a header
//! row (title, context, ✕), body rows in thin boxes (the current one heavy
//! with a yellow edge), one raised green primary action, and a footer with
//! the keys.
//!
//! Popups here: ACCOUNTS (switch which account the chat runs on) and HAND
//! OFF (continue the chat with another agent).

use crate::api::schema::AgentStatus;

use super::accounts::Account;
use super::brutal::{self, Brutal, Edge};
use super::render::{display_width, put_right_text, put_text};
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OrchaPopup {
    /// Accounts and their fuel; acts on the agent in `pane`.
    Accounts { selected: usize, pane: Option<String> },
    /// Continue `pane`'s conversation with another agent.
    Handoff { selected: usize, pane: String },
    /// Start an agent on an account in a new tab of `workspace`.
    NewAgent {
        selected: usize,
        workspace: String,
        project: String,
        cwd: String,
    },
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
pub(super) const HANDOFF_TARGETS: [(&str, &str, &str); 4] = [
    ("codex", "full", "Codex · whole conversation"),
    ("codex", "summary", "Codex · summarize first, then continue"),
    ("claude", "full", "Claude · whole conversation"),
    ("claude", "summary", "Claude · summarize first, then continue"),
];

struct Frame {
    body: Rect,
}

/// The template's shell: dims the page, draws the box, header and footer.
fn popup_frame(
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
    brutal::draw_box(buffer, area, Edge::Heavy, t.line, t.bg, t.bg);
    brutal::draw_box_shadow(buffer, area, t.shadow, t.bg);
    hits.area = area;
    let inner_x = area.x + 1;
    let inner_w = area.width.saturating_sub(2);
    let line = Style::default().fg(t.line).bg(t.bg);

    // header on card, a heavy rule under it
    let header_y = area.y + 1;
    buffer.set_style(Rect::new(inner_x, header_y, inner_w, 1), Style::default().bg(t.card));
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
        put_text(buffer, inner_x, y, inner_w, &"━".repeat(usize::from(inner_w)), line);
    };
    rule(buffer, header_y + 1);

    // footer: the keys on card, a heavy rule above
    let footer_y = area.bottom() - 2;
    rule(buffer, footer_y - 1);
    buffer.set_style(Rect::new(inner_x, footer_y, inner_w, 1), Style::default().bg(t.card));
    let mut x = inner_x + 1;
    for (key, what) in keys {
        let key_width = display_width(key);
        let what_width = display_width(what);
        if x + key_width + what_width + 3 > inner_x + inner_w {
            break;
        }
        put_text(
            buffer,
            x,
            footer_y,
            key_width,
            key,
            Style::default()
                .fg(t.ink)
                .bg(t.card)
                .add_modifier(Modifier::BOLD),
        );
        put_text(
            buffer,
            x + key_width + 1,
            footer_y,
            what_width,
            what,
            Style::default().fg(t.ink).bg(t.card),
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
            inner_x + 1,
            header_y + 3,
            inner_w.saturating_sub(2),
            footer_y.saturating_sub(header_y + 5),
        ),
    })
}

/// A body row: thin box; the current one heavy on card with a yellow edge;
/// the keyboard cursor's border in pink.
fn row_box(buffer: &mut Buffer, rect: Rect, current: bool, cursor: bool, t: &Brutal) -> ratatui::style::Color {
    let line = if cursor { t.pink } else { t.line };
    if current {
        brutal::draw_box(buffer, rect, Edge::Heavy, line, t.card, t.bg);
        for y in rect.y + 1..rect.bottom().saturating_sub(1) {
            put_text(buffer, rect.x + 1, y, 1, "▌", Style::default().fg(t.yellow).bg(t.card));
        }
        t.card
    } else {
        brutal::draw_box(
            buffer,
            rect,
            if cursor { Edge::Heavy } else { Edge::Light },
            line,
            t.bg,
            t.bg,
        );
        t.bg
    }
}

/// A button: the primary one raised green, the others outlined, a
/// disabled one muted.
fn button(
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
        ButtonKind::Muted => Style::default().fg(t.muted).bg(fill),
    };
    put_text(buffer, x, y, width, &text, style);
    if kind == ButtonKind::Primary {
        put_text(buffer, x + width, y, 1, "▌", Style::default().fg(t.shadow).bg(fill));
    }
    Rect::new(x, y, width, 1)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ButtonKind {
    Primary,
    Plain,
    Muted,
}

/// The account a switch should go to: most fuel among the others of the
/// current one's kind.
pub(super) fn best_account(accounts: &[Account]) -> Option<usize> {
    let current = accounts.iter().find(|account| account.current)?;
    accounts
        .iter()
        .enumerate()
        .filter(|(_, account)| {
            !account.current && account.kind == current.kind && account.error.is_none()
        })
        .max_by_key(|(_, account)| account.lowest().unwrap_or(0))
        .map(|(index, _)| index)
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
    let row_height = 4;
    let body_height = (accounts.len().max(1) as u16) * (row_height + 1);
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
                "this chat: {} {} · {project}",
                brutal::agent_logo(agent.agent.as_deref(), config.icon_font),
                brutal::agent_label(agent)
            )
        })
        .unwrap_or_else(|| "no agent in the focused pane".to_owned());
    let frame = popup_frame(
        buffer,
        84,
        body_height,
        "ACCOUNTS",
        &context,
        &[("↵", "switch this chat"), ("⌥↵", "open in new tab"), ("H", "hand off")],
        "esc close",
        t,
        hits,
    )?;
    if accounts.is_empty() {
        put_text(
            buffer,
            frame.body.x,
            frame.body.y,
            frame.body.width,
            "loading accounts…  (needs [orcha] accounts_command)",
            Style::default().fg(t.muted).bg(t.bg),
        );
        return Some(());
    }
    let best = best_account(accounts);
    let mut y = frame.body.y;
    for (index, account) in accounts.iter().enumerate() {
        if y + row_height > frame.body.bottom() + 1 {
            break;
        }
        let rect = Rect::new(frame.body.x, y, frame.body.width, row_height);
        let badge = if account.current {
            Some(("IN USE", t.card, t.ink))
        } else if Some(index) == best {
            Some(("MOST FUEL", t.chip_text, t.green))
        } else {
            None
        };
        let signed_out = account.error.is_some() && account.windows.is_empty();
        let (label, kind) = if account.current {
            ("IN USE", ButtonKind::Muted)
        } else if signed_out {
            ("SIGN IN", ButtonKind::Plain)
        } else if Some(index) == best {
            ("SWITCH ↵", ButtonKind::Primary)
        } else {
            ("SWITCH", ButtonKind::Plain)
        };
        account_row(
            buffer,
            rect,
            account,
            index,
            AccountRow {
                current: account.current,
                cursor: index == selected,
                badge,
                action: (label, kind),
                clickable: !account.current,
            },
            config,
            t,
            hits,
        );
        y += row_height + 1;
    }
    Some(())
}

struct AccountRow<'a> {
    current: bool,
    cursor: bool,
    badge: Option<(&'a str, ratatui::style::Color, ratatui::style::Color)>,
    action: (&'a str, ButtonKind),
    clickable: bool,
}

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
    let fill = row_box(buffer, rect, row.current, row.cursor, t);
    let x = rect.x + 3;
    let logo = brutal::agent_logo(Some(&account.kind), config.icon_font);
    let chip_width = brutal::chip(buffer, x, y + 1, 4, logo, t.chip_text, t.agent(Some(&account.kind)));
    let name_x = x + chip_width + 1;
    let name_width = display_width(&account.label).min(30);
    put_text(
        buffer,
        name_x,
        y + 1,
        name_width,
        &account.label,
        Style::default()
            .fg(t.ink)
            .bg(fill)
            .add_modifier(Modifier::BOLD),
    );
    if let Some((text, fg, bg)) = row.badge {
        brutal::chip(buffer, name_x + name_width + 1, y + 1, 12, text, fg, bg);
    }
    let (label, kind) = row.action;
    let action = button(buffer, rect.right() - 1, y + 1, label, kind, fill, t);
    if row.clickable {
        hits.actions.push((action, index));
    }
    let email = account
        .email
        .clone()
        .or_else(|| account.error.clone())
        .unwrap_or_default();
    let gauges_width = account.windows.iter().take(2).count() as u16 * 22;
    let email_width = rect.width.saturating_sub(gauges_width + 8);
    put_text(
        buffer,
        x,
        y + 2,
        email_width,
        &brutal::clip(&email, usize::from(email_width)),
        Style::default().fg(t.muted).bg(fill),
    );
    let mut gx = rect.right().saturating_sub(gauges_width + 2);
    for window in account.windows.iter().take(2) {
        put_text(buffer, gx, y + 2, 5, &window.label, Style::default().fg(t.muted).bg(fill));
        super::git_panel::draw_gauge(buffer, gx + 5, y + 2, window.left, fill, t);
        put_text(
            buffer,
            gx + 16,
            y + 2,
            5,
            &format!("{:>3}%", window.left),
            Style::default()
                .fg(if window.left < 20 { t.red } else { t.ink })
                .bg(fill)
                .add_modifier(Modifier::BOLD),
        );
        gx += 22;
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
    let row_height = 4;
    let rows = accounts.len() as u16 + 1;
    let frame = popup_frame(
        buffer,
        84,
        rows * (row_height + 1),
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
        .filter(|(_, account)| account.error.is_none() || !account.windows.is_empty())
        .max_by_key(|(_, account)| account.lowest().unwrap_or(0))
        .map(|(index, _)| index);
    let mut y = frame.body.y;
    for (index, account) in accounts.iter().enumerate() {
        if y + row_height > frame.body.bottom() + 1 {
            break;
        }
        let rect = Rect::new(frame.body.x, y, frame.body.width, row_height);
        let signed_out = account.error.is_some() && account.windows.is_empty();
        let action = if signed_out {
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
                current: false,
                cursor: index == selected,
                badge,
                action,
                clickable: true,
            },
            config,
            t,
            hits,
        );
        y += row_height + 1;
    }
    // a plain shell, last
    let index = accounts.len();
    if y + 3 <= frame.body.bottom() + 1 {
        let rect = Rect::new(frame.body.x, y, frame.body.width, 3);
        let fill = row_box(buffer, rect, false, index == selected, t);
        let chip_width = brutal::chip(buffer, rect.x + 3, y + 1, 4, "▶", t.chip_text, t.rail_ahead);
        put_text(
            buffer,
            rect.x + 4 + chip_width,
            y + 1,
            20,
            "Shell",
            Style::default().fg(t.ink).bg(fill).add_modifier(Modifier::BOLD),
        );
        let kind = if index == selected {
            ButtonKind::Primary
        } else {
            ButtonKind::Plain
        };
        let action = button(buffer, rect.right() - 1, y + 1, "OPEN", kind, fill, t);
        hits.actions.push((action, index));
        hits.rows.push((rect, index));
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
    let from = agent.and_then(|agent| agent.agent.clone()).unwrap_or_default();
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
        HANDOFF_TARGETS.len() as u16 * 4,
        "HAND OFF",
        &context,
        &[("↵", "continue there"), ("↑↓", "choose")],
        "esc close",
        t,
        hits,
    )?;
    let mut y = frame.body.y;
    for (index, (kind, mode, label)) in HANDOFF_TARGETS.iter().enumerate() {
        if y + 3 > frame.body.bottom() + 1 {
            break;
        }
        let rect = Rect::new(frame.body.x, y, frame.body.width, 3);
        let same = *kind == from;
        let fill = row_box(buffer, rect, false, index == selected, t);
        let x = rect.x + 2;
        let chip_width = brutal::chip(
            buffer,
            x,
            y + 1,
            4,
            brutal::agent_logo(Some(kind), config.icon_font),
            t.chip_text,
            t.agent(Some(kind)),
        );
        put_text(
            buffer,
            x + chip_width + 1,
            y + 1,
            rect.width.saturating_sub(chip_width + 16),
            label,
            Style::default()
                .fg(if same { t.muted } else { t.ink })
                .bg(fill)
                .add_modifier(Modifier::BOLD),
        );
        let kind_button = if index == selected {
            ButtonKind::Primary
        } else {
            ButtonKind::Plain
        };
        let action = button(
            buffer,
            rect.right() - 1,
            y + 1,
            if *mode == "summary" { "SUMMARY ↵" } else { "FULL ↵" },
            kind_button,
            fill,
            t,
        );
        hits.actions.push((action, index));
        hits.rows.push((rect, index));
        y += 4;
    }
    Some(())
}

/// Draws the open popup over the composed frame.
pub(super) fn render_popup(
    buffer: &mut Buffer,
    popup: &OrchaPopup,
    accounts: Option<&[Account]>,
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
        OrchaPopup::Handoff { selected, pane } => {
            let _ = render_handoff(buffer, *selected, pane, snapshot, config, &t, &mut hits);
        }
        OrchaPopup::NewAgent {
            selected, project, ..
        } => {
            let _ = render_new_agent(buffer, *selected, project, accounts, config, &t, &mut hits);
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
