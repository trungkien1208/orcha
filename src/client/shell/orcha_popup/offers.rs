//! Consent, account-switch and release-news choices.

use super::*;

/// The highlights of a release, one bullet per line, from `y`.
pub(super) fn render_notes(
    buffer: &mut Buffer,
    body: Rect,
    y: u16,
    notes: &[String],
    t: &Brutal,
) -> u16 {
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
            Style::default().fg(t.text(t.green, t.bg)).bg(t.bg),
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

/// The rows of the account switch choice: (title, detail, button, usable).
pub(in crate::client::shell) fn switch_account_rows(
    label: &str,
    restart_blocked: Option<&str>,
) -> [(String, String, &'static str, bool); 3] {
    [
        (
            "Keep Claude running, swap the sign-in".to_owned(),
            "no restart; other Claude chats on this sign-in move too".to_owned(),
            "SWAP",
            true,
        ),
        (
            format!("Restart Claude on {label}"),
            restart_blocked.map_or_else(
                || "only this chat moves; it picks up where it was".to_owned(),
                str::to_owned,
            ),
            "RESTART",
            restart_blocked.is_none(),
        ),
        (
            "Keep it".to_owned(),
            "the chat stays on its current account".to_owned(),
            "KEEP",
            true,
        ),
    ]
}

/// SWITCH ACCOUNT: how a Claude chat moves to another account.
pub(super) fn render_switch_account(
    buffer: &mut Buffer,
    selected: usize,
    label: &str,
    restart_blocked: Option<&str>,
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let rows = switch_account_rows(label, restart_blocked);
    let frame = popup_frame(
        buffer,
        72,
        rows_height(rows.len()),
        "SWITCH ACCOUNT",
        &format!("move this chat to {label}"),
        &[("↵", "choose"), ("↑↓", "move")],
        "esc close",
        t,
        hits,
    )?;
    draw_rows(
        buffer,
        frame.body,
        ROW,
        rows.len(),
        selected,
        t,
        |buffer, rect, index, first| {
            let Some((title, detail, action, usable)) = rows.get(index) else {
                return;
            };
            let row = ChoiceRow {
                tone: if *usable { Tone::Ink } else { Tone::Muted },
                ..ChoiceRow::new(
                    title,
                    detail,
                    if *usable {
                        RowButton::OnCursor(action)
                    } else {
                        RowButton::None
                    },
                )
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
    Some(())
}

/// What a row of the plugin offer does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::client::shell) enum PluginOfferChoice {
    Add { autostart: bool },
    NotNow,
    Never,
}

pub(in crate::client::shell) fn plugin_offer_rows(
    plugin: &crate::orcha_app::OfferedPlugin,
) -> Vec<(PluginOfferChoice, String, String, &'static str)> {
    let mut rows = vec![(
        PluginOfferChoice::Add { autostart: false },
        format!("Add {}", plugin.name),
        "an action in COMMANDS; nothing runs until you use it".to_owned(),
        "ADD",
    )];
    if plugin.autostart.is_some() {
        rows.push((
            PluginOfferChoice::Add { autostart: true },
            format!("Add and start {} with Orcha", plugin.name),
            "runs in a background project that never takes focus".to_owned(),
            "ADD",
        ));
    }
    rows.push((
        PluginOfferChoice::NotNow,
        "Not now".to_owned(),
        "asks again after the next Orcha update".to_owned(),
        "NOT NOW",
    ));
    rows.push((
        PluginOfferChoice::Never,
        "Never".to_owned(),
        "add it later with orcha office install".to_owned(),
        "NEVER",
    ));
    rows
}

/// Write the working rules into a new project, choose them in the
/// browser, not now, or never for it.
pub(in crate::client::shell) fn rules_offer_rows(
    file: &str,
    rules: &[String],
) -> [(String, String, &'static str); 4] {
    let titles: Vec<&str> = crate::orcha_rules::RULES
        .iter()
        .filter(|rule| rules.iter().any(|id| id == rule.id))
        .map(|rule| rule.title)
        .collect();
    [
        (format!("Add them to {file}"), titles.join(" · "), "ADD"),
        (
            "Choose in the browser".to_owned(),
            "pick rules and skills for this project, see the change first".to_owned(),
            "CHOOSE",
        ),
        (
            "Not now".to_owned(),
            "Settings › Projects has them any time".to_owned(),
            "NOT NOW",
        ),
        (
            "Never for this project".to_owned(),
            "orcha will not ask about it again".to_owned(),
            "NEVER",
        ),
    ]
}

/// Ask the agent for its hand-off checklist now, or not now.
pub(in crate::client::shell) fn checklist_rows(
    kind: &str,
    file: &str,
) -> [(String, String, &'static str); 2] {
    [
        (
            format!("Ask {kind} now"),
            format!("it writes what is done and what is next to {file}"),
            "ASK",
        ),
        (
            "Not now".to_owned(),
            "orcha will not ask again for this account".to_owned(),
            "NOT NOW",
        ),
    ]
}

/// `path` with the home folder written as `~`.
pub(in crate::client::shell) fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_owned(),
    }
}

/// Give orcha back its folder access, or not now.
pub(in crate::client::shell) fn folder_access_rows() -> [(String, String, &'static str); 2] {
    let orcha = std::env::current_exe()
        .map(|path| tilde(&path.to_string_lossy()))
        .unwrap_or_else(|_| "orcha".to_owned());
    [
        (
            "Open Full Disk Access".to_owned(),
            format!("add {orcha}, then restart orcha's server"),
            "OPEN",
        ),
        (
            "Not now".to_owned(),
            "shells in these folders fail until orcha can read them".to_owned(),
            "NOT NOW",
        ),
    ]
}

/// Remove, or keep it (the safe default).
pub(in crate::client::shell) fn plugin_remove_rows(
    name: &str,
) -> [(String, String, &'static str); 2] {
    [
        (
            format!("Remove {name}"),
            "stops it and removes it from Orcha; its own data stays".to_owned(),
            "REMOVE",
        ),
        ("Keep it".to_owned(), "nothing changes".to_owned(), "KEEP"),
    ]
}

/// A small popup of choices: `header` and its `context` line, then one
/// row per choice (title, detail, button); the button shows on the
/// cursor's row.
pub(super) fn render_choice_rows(
    buffer: &mut Buffer,
    selected: usize,
    header: &str,
    context: &str,
    rows: &[(String, String, &str)],
    t: &Brutal,
    hits: &mut PopupHits,
) -> Option<()> {
    let frame = popup_frame(
        buffer,
        72,
        rows_height(rows.len()),
        header,
        context,
        &[("↵", "choose"), ("↑↓", "move")],
        "esc close",
        t,
        hits,
    )?;
    draw_rows(
        buffer,
        frame.body,
        ROW,
        rows.len(),
        selected,
        t,
        |buffer, rect, index, first| {
            let Some((title, detail, action)) = rows.get(index) else {
                return;
            };
            let row = ChoiceRow::new(title, detail, RowButton::OnCursor(action))
                .at(first, index == selected);
            choice_row(buffer, rect, index, row, t, hits);
        },
    );
    Some(())
}

/// Update now, or not now (COMMANDS keeps an update row).
pub(in crate::client::shell) fn plugin_update_rows(
    update: &crate::orcha_plugin_updates::PluginUpdate,
    name: &str,
) -> [(String, String, &'static str); 2] {
    [
        (
            format!("Update {name} to {}", update.latest_version),
            "downloads it, then restarts its running panes".to_owned(),
            "UPDATE",
        ),
        (
            "Not now".to_owned(),
            format!("COMMANDS keeps an Update {name} row"),
            "NOT NOW",
        ),
    ]
}

/// UPDATE: what the new version brings, then update now, not now or skip it.
pub(super) fn render_update(
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
        "esc close",
        t,
        hits,
    )?;
    let y = render_notes(buffer, frame.body, frame.body.y, &release.notes, t) + gap;
    let options = [
        (
            "Update now".to_owned(),
            "installs in the background; your agents keep running".to_owned(),
            "UPDATE",
        ),
        (
            "Not now".to_owned(),
            "the green button in the header stays until you update".to_owned(),
            "NOT NOW",
        ),
        (
            format!("Skip {}", release.version),
            "no reminder until a newer version".to_owned(),
            "SKIP",
        ),
    ];
    let list = Rect::new(
        frame.body.x,
        y,
        frame.body.width,
        frame.body.bottom().saturating_sub(y),
    );
    draw_rows(
        buffer,
        list,
        ROW,
        options.len(),
        selected,
        t,
        |buffer, rect, index, first| {
            let Some((title, detail, action)) = options.get(index) else {
                return;
            };
            let row = ChoiceRow::new(title, detail, RowButton::Always(action))
                .at(first, index == selected);
            choice_row(buffer, rect, index, row, t, hits);
        },
    );
    Some(())
}

/// WHAT'S NEW: the highlights of the version just installed, once.
pub(super) fn render_whats_new(
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
    hits.actions
        .push((action, PopupTarget::Button(PopupButton::Primary)));
    Some(())
}
