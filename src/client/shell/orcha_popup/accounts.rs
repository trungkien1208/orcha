//! Account, agent and file-opening choices.

use super::*;

/// Names offered for a new Claude account (`oc personal`, …).
pub(in crate::client::shell) const ADD_ACCOUNT_NAMES: [&str; 5] =
    ["personal", "work", "side", "team", "client"];

/// Suggestions that are not already an account id.
pub(in crate::client::shell) fn account_name_choices(accounts: &[Account]) -> Vec<String> {
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

/// Fuel under this percent always reads as low (red gauges, a warning),
/// even when `low_fuel_percent` is set lower; a higher setting wins.
pub(in crate::client::shell) const LOW_FUEL_FLOOR: u8 = 20;

/// The one test for "running low", shared by the popups and the actions.
pub(in crate::client::shell) fn fuel_is_low(left: u8, orcha: &crate::config::OrchaConfig) -> bool {
    left < orcha.low_fuel_percent.max(LOW_FUEL_FLOOR)
}

/// Where to continue: another account of the same agent while it is not
/// low (the chat goes on as is), else whichever agent has the most fuel.
/// Without the config here, "not low" means at `LOW_FUEL_FLOOR` or above.
pub(in crate::client::shell) fn best_account(accounts: &[Account]) -> Option<usize> {
    let current = accounts.iter().find(|account| account.current)?;
    let usable = |account: &&Account| !account.current && !account.windows.is_empty();
    let most = |same_kind: bool| {
        accounts
            .iter()
            .enumerate()
            .filter(|(_, account)| {
                usable(account) && (!same_kind || brutal::same_agent(&account.kind, &current.kind))
            })
            .max_by_key(|(_, account)| account.lowest().unwrap_or(0))
    };
    most(true)
        .filter(|(_, account)| account.lowest().unwrap_or(0) >= LOW_FUEL_FLOOR)
        .or_else(|| most(false))
        .map(|(index, _)| index)
}

pub(super) fn render_accounts(
    buffer: &mut Buffer,
    selected: usize,
    pane: Option<&str>,
    accounts: Option<&[Account]>,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let loading = accounts.is_none();
    let accounts = accounts.unwrap_or(&[]);
    let current_kind = pane
        .and_then(|pane| snapshot.agents.iter().find(|agent| agent.pane_id == pane))
        .and_then(|agent| agent.agent.as_deref());
    let is_current = |account: &Account| {
        account.current && current_kind.is_none_or(|kind| brutal::same_agent(kind, &account.kind))
    };
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
        .find(|account| is_current(account))
        .and_then(|account| account.lowest().map(|left| (account, left)))
        .filter(|(_, left)| fuel_is_low(*left, &config.orcha));
    let context = match low {
        Some((account, left)) => format!("{context} · ⚠ {} {left}% left", account.label),
        None => context,
    };
    // alt ↵ hands a chat to another agent with only its latest part, and
    // opens another account of the same agent in a new pane
    let alternate = match (accounts.get(selected), current_kind) {
        (Some(account), Some(kind)) if !brutal::same_agent(&account.kind, kind) => "latest part",
        _ => "new pane",
    };
    let frame = popup_frame(
        buffer,
        88,
        rows_height(accounts.len().max(1)).saturating_add(2),
        "ACCOUNTS",
        &context,
        &[
            ("↵", "continue"),
            ("alt ↵", alternate),
            ("h", "hand off"),
            ("d", "remove"),
            ("i", "find"),
        ],
        "↻ refresh (r)",
        t,
        hits,
    )?;
    if accounts.is_empty() {
        put_text(
            buffer,
            frame.body.x + 2,
            frame.body.y,
            frame.body.width.saturating_sub(4),
            if loading {
                "looking up accounts…"
            } else {
                "no accounts yet: add one below, or press i to find the ones on this computer"
            },
            Style::default().fg(t.muted).bg(t.bg),
        );
    }
    let best = best_account(accounts);
    // the accounts, then ADD ACCOUNT pinned to the last line
    let list = Rect::new(
        frame.body.x,
        frame.body.y,
        frame.body.width,
        frame.body.height.saturating_sub(2),
    );
    draw_rows(
        buffer,
        list,
        ROW,
        accounts.len(),
        selected,
        t,
        |buffer, rect, index, first| {
            let Some(account) = accounts.get(index) else {
                return;
            };
            // another agent: the chat is handed off; the same agent: it goes on as is
            let hand_off =
                current_kind.is_some_and(|kind| !brutal::same_agent(kind, &account.kind));
            let badge = if is_current(account) {
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
            } else if is_current(account) {
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
                    first,
                    cursor: index == selected,
                    badge,
                    action,
                },
                config,
                t,
                hits,
            );
        },
    );
    let add = Rect::new(
        frame.body.x,
        frame.body.bottom().saturating_sub(1),
        frame.body.width,
        1,
    );
    choice_row(
        buffer,
        add,
        accounts.len(),
        ChoiceRow::new("+  ADD ACCOUNT", "", RowButton::Always("ADD"))
            .at(accounts.is_empty(), selected == accounts.len()),
        t,
        hits,
    );
    Some(())
}

/// REMOVE ACCOUNT: take it off orcha's list, or keep it.
pub(super) fn render_remove_account(
    buffer: &mut Buffer,
    selected: usize,
    label: &str,
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let frame = popup_frame(
        buffer,
        64,
        rows_height(2),
        "REMOVE ACCOUNT",
        label,
        &[("↵", "choose"), ("esc", "close")],
        "",
        t,
        hits,
    )?;
    let options = [
        (
            "Remove from orcha",
            "its sign-in and chats stay; add it again any time",
            "REMOVE",
        ),
        ("Keep it", "back to the accounts", "KEEP"),
    ];
    draw_rows(
        buffer,
        frame.body,
        ROW,
        options.len(),
        selected,
        t,
        |buffer, rect, index, first| {
            let Some((title, detail, action)) = options.get(index) else {
                return;
            };
            let mut row = ChoiceRow::new(title, detail, RowButton::Always(action))
                .at(first, index == selected);
            if index == 0 {
                row.tone = Tone::Danger;
            }
            choice_row(buffer, rect, index, row, t, hits);
        },
    );
    Some(())
}

/// ADD ACCOUNT: a suggested name, or whatever was typed.
pub(super) fn render_add_account(
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
        hits.actions.push((action, PopupTarget::Row(index)));
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
            hits.actions.push((action, PopupTarget::Row(index)));
        }
        hits.rows.push((rect, index));
    }
    Some(())
}

pub(super) struct AccountRow<'a> {
    first: bool,
    cursor: bool,
    badge: Option<(&'a str, ratatui::style::Color, ratatui::style::Color)>,
    action: Option<(&'a str, ButtonKind)>,
}

/// Fuel columns: "WEEK " + 10 marks a cell apart + " 100%".
pub(super) const GAUGE_COLS: u16 = 5 + 19 + 5;

/// One account: logo chip, name and badge, action button; email and the
/// fuel gauges under them.
pub(super) fn account_row(
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
        hits.actions.push((action, PopupTarget::Row(index)));
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
        super::super::git_panel::draw_gauge(buffer, gx + 5, y + 1, window.left, fill, t);
        put_text(
            buffer,
            gx + 24,
            y + 1,
            5,
            &format!("{:>4}%", window.left),
            Style::default()
                .fg(if fuel_is_low(window.left, &config.orcha) {
                    t.red
                } else {
                    t.ink
                })
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
                        super::super::accounts::resets_in(reset)
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
/// Rows are the accounts (with their fuel), then a plain shell, then a new
/// project.
pub(super) fn render_new_agent(
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
        rows_height(accounts.len() + NEW_AGENT_EXTRA_ROWS),
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
    // the accounts, then a plain shell, then a new project
    draw_rows(
        buffer,
        frame.body,
        ROW,
        accounts.len() + NEW_AGENT_EXTRA_ROWS,
        selected,
        t,
        |buffer, rect, index, first| {
            let cursor = index == selected;
            let Some(account) = accounts.get(index) else {
                let row = if index == accounts.len() {
                    ChoiceRow {
                        lead: Lead::Logo("▶", t.rail_ahead),
                        ..ChoiceRow::new(
                            "Shell",
                            "no agent, just a terminal",
                            RowButton::Always("OPEN"),
                        )
                    }
                } else {
                    ChoiceRow {
                        lead: Lead::Logo("+", t.green),
                        ..ChoiceRow::new(
                            "New project…",
                            "a folder on this computer, or a new worktree",
                            RowButton::Always("CHOOSE"),
                        )
                    }
                };
                choice_row(buffer, rect, index, row.at(first, cursor), t, hits);
                return;
            };
            let action = if account.signed_out() {
                ("SIGN IN", ButtonKind::Plain)
            } else if cursor {
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
                    first,
                    cursor,
                    badge,
                    action: Some(action),
                },
                config,
                t,
                hits,
            );
        },
    );
    Some(())
}

/// FOUND ACCOUNTS: Claude sign-ins already on this machine, one row each
/// with a check; IMPORT adds the checked ones as they are (no new sign-in).
pub(super) fn render_found_accounts(
    buffer: &mut Buffer,
    selected: usize,
    chosen: &[bool],
    found: &[crate::orcha_accounts::FoundAccount],
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let count = chosen.iter().filter(|on| **on).count();
    let frame = popup_frame(
        buffer,
        76,
        rows_height(found.len()) + 2,
        "FOUND ACCOUNTS",
        "agents on this machine · import adds them to Accounts",
        &[("↵", "import"), ("space", "check"), ("esc", "close")],
        "",
        t,
        hits,
    )?;
    draw_rows(
        buffer,
        list_above_button(frame.body),
        ROW,
        found.len(),
        selected,
        t,
        |buffer, rect, index, first| {
            let Some(account) = found.get(index) else {
                return;
            };
            let claude = brutal::same_agent(&account.kind, "claude");
            let title = if claude {
                format!("Claude · {}  {}", account.name, account.email)
            } else {
                format!("{}  {}", account.name, account.email)
            };
            let command = crate::orcha_accounts::agent_kind(&account.kind)
                .map_or(account.kind.as_str(), |agent| agent.command);
            let detail = match (claude, account.signed_in) {
                (true, _) => account
                    .dir
                    .clone()
                    .unwrap_or_else(|| "~/.claude (default)".to_owned()),
                (_, true) => format!("signed in · runs {command}"),
                (_, false) => {
                    format!("installed · sign in from Accounts after importing ({command})")
                }
            };
            let row = ChoiceRow {
                lead: Lead::Check(chosen.get(index).copied().unwrap_or(false)),
                ..ChoiceRow::new(&title, &detail, RowButton::None)
            };
            choice_row(
                buffer,
                rect,
                index,
                row.at(first, index == selected),
                t,
                hits,
            );
        },
    );
    primary_under_list(buffer, frame.body, count, t, hits);
    Some(())
}

/// OPEN WITH: the suggested app first, then the other choices; a check
/// under the list keeps the choice for the extension.
pub(super) fn render_open_with(
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
    draw_rows(
        buffer,
        list_above_button(frame.body),
        ROW,
        choices.len(),
        selected,
        t,
        |buffer, rect, index, first| {
            let Some(choice) = choices.get(index) else {
                return;
            };
            let mut title = choice.title();
            if index == 0 {
                title.push_str("  · suggested");
            }
            let detail = choice.detail(extension);
            let row = ChoiceRow::new(&title, &detail, RowButton::Always("OPEN"))
                .at(first, index == selected);
            choice_row(buffer, rect, index, row, t, hits);
        },
    );
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
            PopupTarget::Check,
        ));
    }
    Some(())
}
