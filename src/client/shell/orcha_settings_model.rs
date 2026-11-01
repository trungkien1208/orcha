//! The settings page's data: every setting as the page shows it (grouped
//! and worded for people), and the edits behind each control, the projects'
//! rules and skills, the accounts and the background list. The web server
//! in `orcha_settings_web` only routes requests here.

use std::path::PathBuf;

use serde_json::{json, Value};

use super::settings::{
    installed_apps, ACCENTS, FONT_SIZES, GIT_GUI_APPS, IDE_APPS, SIDEBAR_WIDTHS,
};

fn option(value: &str, label: &str) -> Value {
    json!({"value": value, "label": label})
}

fn select(key: &str, label: &str, help: &str, value: &str, options: Vec<Value>) -> Value {
    json!({"key": key, "label": label, "help": help, "kind": "select", "value": value, "options": options})
}

fn toggle(key: &str, label: &str, help: &str, value: bool) -> Value {
    json!({"key": key, "label": label, "help": help, "kind": "toggle", "value": value})
}

fn text(key: &str, label: &str, help: &str, value: &str) -> Value {
    json!({"key": key, "label": label, "help": help, "kind": "text", "value": value})
}

/// orcha-config's checkout (palettes, languages), when it is installed.
fn config_tool_root() -> Option<PathBuf> {
    Some(
        crate::orcha_app::config_tool()?
            .parent()?
            .parent()?
            .to_path_buf(),
    )
}

fn orcha_json_path() -> PathBuf {
    crate::config::config_dir().join("config.json")
}

fn orcha_json() -> Value {
    std::fs::read(orcha_json_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| json!({}))
}

fn names_in(dir: PathBuf) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".json").map(str::to_owned)
        })
        .collect();
    names.sort();
    names
}

/// Ghostty's fonts, asked once: the page polls, and listing fonts is slow.
fn fonts() -> Vec<String> {
    static FONTS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    FONTS.get_or_init(super::settings::ghostty_fonts).clone()
}

fn palettes(root: &std::path::Path) -> Vec<Value> {
    names_in(root.join("palettes"))
        .into_iter()
        .map(|name| {
            let file: Value = std::fs::read(root.join("palettes").join(format!("{name}.json")))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
            json!({
                "value": name,
                "label": file["name"].as_str().unwrap_or(&name),
                "help": file["description"],
                "swatch": [file["bg"], file["fg"], file["ansi"][1], file["ansi"][2], file["ansi"][4], file["ansi"][5]],
            })
        })
        .collect()
}

pub(super) fn state() -> Value {
    let config = crate::config::Config::load().config;
    let mut sections = regroup(sections_for(&config));
    // what each setting is out of the box: a dot marks a change, ↺ goes back
    let defaults: std::collections::HashMap<String, Value> =
        sections_for(&crate::config::Config::default())
            .iter()
            .flat_map(|section| section["fields"].as_array().cloned().unwrap_or_default())
            .filter(|field| DEFAULT_KEYS.contains(&field["key"].as_str().unwrap_or_default()))
            .filter_map(|field| Some((field["key"].as_str()?.to_owned(), field["value"].clone())))
            .collect();
    for section in &mut sections {
        for field in section["fields"].as_array_mut().into_iter().flatten() {
            let key = field["key"].as_str().unwrap_or_default().to_owned();
            if let Some(default) = defaults.get(&key) {
                field["default"] = default.clone();
            }
            if let Some(warning) = warning_for(&key, &config) {
                field["warn"] = json!(warning);
            }
        }
    }
    json!({
        "version": crate::orcha_app::ORCHA_VERSION,
        "sections": sections,
        "has_config_tool": config_tool_root().is_some(),
        "page_theme": page_theme(&config),
    })
}

/// The page's groups, by what a person wants to do, each setting in plain
/// words: (id, title, intro, [(sub-heading, [keys])]). "widgets.*" takes
/// every widget.
/// A sub-heading and the keys under it.
type Group = (&'static str, &'static [&'static str]);

const GROUPS: &[(&str, &str, &str, &[Group])] = &[
    (
        "look",
        "Look & colors",
        "Colors and font for orcha and the terminal inside it.",
        &[
            ("orcha", &["theme", "accent"]),
            ("Terminal", &["palette", "font", "font_size", "ligatures"]),
        ],
    ),
    (
        "layout",
        "Layout",
        "Where things sit on screen, and what orcha shows.",
        &[
            (
                "Panels",
                &[
                    "sidebar_side",
                    "sidebar_width",
                    "right_panel",
                    "fuel",
                    "process",
                    "icons",
                    "pane_paper",
                ],
            ),
            ("Extras", &["widgets.*", "weather_city"]),
        ],
    ),
    (
        "agents",
        "Agents & accounts",
        "How orcha gets your attention, and what it does when an account runs low.",
        &[
            (
                "Getting your attention",
                &["status_indicators", "sound", "toast"],
            ),
            (
                "When an account runs low",
                &[
                    "checklist_percent",
                    "low_fuel_percent",
                    "auto_handoff_percent",
                    "switch_mode",
                ],
            ),
        ],
    ),
    (
        "advanced",
        "Advanced",
        "Apps, branch rules and language. You set these once.",
        &[
            ("Apps", &["ide", "git_gui"]),
            (
                "Branches & tasks",
                &[
                    "repo.branch_prefix",
                    "repo.base",
                    "repo.protected",
                    "repo.check",
                    "tracker.task_pattern",
                    "tracker.azure_pat_file",
                ],
            ),
            ("Other", &["checklist_file", "lang", "update_plugins"]),
        ],
    ),
];

/// Plain words for each setting: (key, label, help).
const WORDS: &[(&str, &str, &str)] = &[
    (
        "theme",
        "App colors",
        "orcha's panels, its sidebar and this page",
    ),
    (
        "accent",
        "Highlight color",
        "buttons, the selected tab, the active row",
    ),
    (
        "palette",
        "Terminal colors",
        "the colors inside your panes (Ghostty)",
    ),
    (
        "font",
        "Terminal font",
        "Ghostty shows it after ⌘⇧, (reload config)",
    ),
    ("font_size", "Font size", ""),
    ("ligatures", "Join symbols", "draw -> and != as one symbol"),
    (
        "sidebar_side",
        "Project list on the",
        "the right panel takes the other side",
    ),
    ("sidebar_width", "Project list width", ""),
    (
        "right_panel",
        "Right panel",
        "accounts, changed files and running agents",
    ),
    (
        "fuel",
        "Plan left shows in",
        "how much each account still has",
    ),
    (
        "process",
        "Running agents",
        "little trains for agents at work",
    ),
    ("icons", "Agent logos", "Claude, Codex and Cursor icons"),
    (
        "pane_paper",
        "Fill empty panes",
        "paint blank pane space in the theme color",
    ),
    ("status_indicators", "Agent state shown as", ""),
    ("sound", "Sound", "when an agent finishes or needs you"),
    ("toast", "Pop-up notes", "where short messages appear"),
    (
        "checklist_percent",
        "Offer a checklist",
        "the agent writes down what is done and what is next",
    ),
    (
        "low_fuel_percent",
        "Ask where to continue",
        "orcha offers another account or agent",
    ),
    (
        "auto_handoff_percent",
        "Move on by itself",
        "the chat goes to the account with the most left",
    ),
    (
        "switch_mode",
        "When a chat moves",
        "to another account of the same agent",
    ),
    (
        "weather_city",
        "Weather city",
        "empty: found from where you are",
    ),
    ("ide", "Open folders in", ""),
    ("git_gui", "Open repositories in", ""),
    (
        "checklist_file",
        "Checklist file",
        "inside the project; a hand-off points the next agent at it",
    ),
    (
        "repo.branch_prefix",
        "New branch prefix",
        "new branches start with it",
    ),
    ("repo.base", "Base branch", "empty: found from the repo"),
    (
        "repo.protected",
        "Protected branches",
        "a regex; agents are warned before changing these",
    ),
    (
        "repo.check",
        "Check command",
        "auto, or a command like npm run typecheck",
    ),
    (
        "tracker.task_pattern",
        "Task number in branch names",
        "a regex; its first group is the task number",
    ),
    (
        "tracker.azure_pat_file",
        "Azure DevOps token file",
        "where the personal access token is kept",
    ),
    ("lang", "Language", "orcha's own words"),
    (
        "update_plugins",
        "Update plugins by themselves",
        "off: orcha asks before installing a new version",
    ),
];

/// The settings, regrouped by task and reworded; keys stay as they are.
fn regroup(sections: Vec<Value>) -> Vec<Value> {
    let fields: Vec<Value> = sections
        .iter()
        .flat_map(|section| section["fields"].as_array().cloned().unwrap_or_default())
        .collect();
    let take = |key: &str| -> Vec<Value> {
        fields
            .iter()
            .filter(|field| {
                let k = field["key"].as_str().unwrap_or_default();
                match key.strip_suffix('*') {
                    Some(prefix) => k.starts_with(prefix),
                    None => k == key,
                }
            })
            .cloned()
            .collect()
    };
    GROUPS
        .iter()
        .map(|(id, title, intro, groups)| {
            let mut out = Vec::new();
            for (group, keys) in *groups {
                for key in *keys {
                    for mut field in take(key) {
                        let k = field["key"].as_str().unwrap_or_default().to_owned();
                        if let Some((_, label, help)) = WORDS.iter().find(|(each, _, _)| *each == k)
                        {
                            field["label"] = json!(label);
                            field["help"] = json!(help);
                        }
                        if let Some(place) = k.strip_prefix("widgets.").and(field["help"].as_str())
                        {
                            // orcha-config says where a widget lives ("Header", "Claude")
                            let place = match place.to_lowercase().as_str() {
                                "" => String::new(),
                                "claude" => "in Claude Code".to_owned(),
                                other => format!("in the {other}"),
                            };
                            field["help"] = json!(place);
                            let label = field["label"].as_str().unwrap_or_default().to_owned();
                            let mut letters = label.chars();
                            if let Some(first) = letters.next() {
                                field["label"] = json!(
                                    first.to_uppercase().collect::<String>() + letters.as_str()
                                );
                            }
                        }
                        if k == "switch_mode" {
                            field["options"] = json!([
                                option("ask", "ask me"),
                                option("keep", "keep it running"),
                                option("restart", "restart the agent"),
                                option("new_pane", "new pane"),
                            ]);
                        }
                        field["group"] = json!(group);
                        out.push(field);
                    }
                }
            }
            json!({"id": id, "title": title, "intro": intro, "fields": out})
        })
        .filter(|section| {
            section["fields"]
                .as_array()
                .is_some_and(|fields| !fields.is_empty())
        })
        .collect()
}

/// Settings whose out-of-the-box value is orcha's (config.toml); the
/// orcha-config ones have their own defaults.
const DEFAULT_KEYS: &[&str] = &[
    "accent",
    "sidebar_side",
    "sidebar_width",
    "right_panel",
    "fuel",
    "process",
    "icons",
    "pane_paper",
    "status_indicators",
    "sound",
    "toast",
    "checklist_percent",
    "low_fuel_percent",
    "auto_handoff_percent",
    "switch_mode",
    "checklist_file",
    "update_plugins",
];

/// A setting that clashes with another, or cannot work on this machine.
fn warning_for(key: &str, config: &crate::config::Config) -> Option<String> {
    let orcha = &config.orcha;
    match key {
        "checklist_percent"
            if orcha.checklist_percent > 0 && orcha.checklist_percent <= orcha.low_fuel_percent =>
        {
            Some(format!(
                "at or under the switch offer ({}%): the checklist would come too late to help; set it higher",
                orcha.low_fuel_percent
            ))
        }
        "auto_handoff_percent"
            if orcha.auto_handoff_percent > 0 && orcha.auto_handoff_percent >= orcha.low_fuel_percent =>
        {
            Some(format!(
                "at or above the switch offer ({}%): orcha would move the chat before asking; set it lower",
                orcha.low_fuel_percent
            ))
        }
        "process"
            if orcha.process == crate::config::ProcessPlacement::Panel && !config.ui.right_panel =>
        {
            Some("the right panel is off, so PROCESS shows in the sidebar for now".to_owned())
        }
        "fuel" if orcha.fuel == crate::config::FuelPlacement::Panel && !config.ui.right_panel => {
            Some("the right panel is off: turn it on, or move the gauges to the status bar".to_owned())
        }
        "font" => {
            let family = orcha_json()["font"]["family"].as_str().unwrap_or_default().to_owned();
            let found = fonts();
            (!family.is_empty() && !found.is_empty() && !found.contains(&family))
                .then(|| format!("{family} is not installed: Ghostty falls back to another font"))
        }
        _ => None,
    }
}

fn hex(color: ratatui::style::Color) -> Option<String> {
    match color {
        ratatui::style::Color::Rgb(r, g, b) => Some(format!("#{r:02x}{g:02x}{b:02x}")),
        _ => None,
    }
}

fn is_dark(palette: &crate::app::state::Palette) -> bool {
    match palette.sidebar_bg {
        ratatui::style::Color::Rgb(r, g, b) => {
            (u32::from(r) * 3 + u32::from(g) * 6 + u32::from(b)) / 10 < 128
        }
        _ => true,
    }
}

/// The light and dark sibling of an orcha theme.
fn theme_pair(name: &str) -> (&'static str, &'static str) {
    match name {
        "matcha" | "matcha-dark" => ("matcha", "matcha-dark"),
        "catppuccin" | "catppuccin-latte" => ("catppuccin-latte", "catppuccin"),
        "tokyo-night" | "tokyo-night-day" => ("tokyo-night-day", "tokyo-night"),
        "gruvbox" | "gruvbox-light" => ("gruvbox-light", "gruvbox"),
        "one-dark" | "one-light" => ("one-light", "one-dark"),
        "solarized" | "solarized-light" => ("solarized-light", "solarized"),
        "kanagawa" | "kanagawa-lotus" => ("kanagawa-lotus", "kanagawa"),
        "rose-pine" | "rose-pine-dawn" => ("rose-pine-dawn", "rose-pine"),
        _ => ("orcha", "orcha-dark"),
    }
}

/// The page's own colors, from the orcha theme in use (its dark and light
/// sibling), so the page looks like the app it sets up.
fn page_theme(config: &crate::config::Config) -> Value {
    use crate::app::state::Palette;
    let name = config.theme.name.as_deref().unwrap_or("orcha");
    let accent = config
        .theme
        .custom
        .as_ref()
        .and_then(|custom| custom.mauve.clone());
    let current = Palette::from_name(name);
    let (light, dark) = theme_pair(name);
    let pick = |want_dark: bool, fallback: &str| {
        current
            .clone()
            .filter(|palette| is_dark(palette) == want_dark && hex(palette.sidebar_bg).is_some())
            .or_else(|| Palette::from_name(fallback))
            .unwrap_or_else(Palette::brutal_dark)
    };
    let colors = |palette: Palette| {
        json!({
            "bg": hex(palette.sidebar_bg), "card": hex(palette.panel_bg), "raised": hex(palette.surface0),
            "border": hex(palette.surface1), "text": hex(palette.text), "muted": hex(palette.overlay0),
            "accent": accent.clone().or_else(|| hex(palette.accent)),
            "green": hex(palette.green), "yellow": hex(palette.yellow), "red": hex(palette.red),
        })
    };
    json!({"dark": colors(pick(true, dark)), "light": colors(pick(false, light))})
}

/// Each orcha theme's look, for the theme cards.
fn theme_cards() -> Vec<Value> {
    crate::config::THEME_NAMES
        .iter()
        .map(|name| {
            let palette = crate::app::state::Palette::from_name(name);
            let swatch: Vec<Option<String>> = palette
                .as_ref()
                .map(|p| {
                    [
                        p.sidebar_bg,
                        p.panel_bg,
                        p.text,
                        p.accent,
                        p.green,
                        p.yellow,
                    ]
                    .map(hex)
                    .to_vec()
                })
                .unwrap_or_default();
            json!({"value": name, "label": name, "swatch": swatch,
                "help": if palette.as_ref().is_some_and(is_dark) { "dark" } else { "light" }})
        })
        .collect()
}

/// Every account with its plan windows, sign-in and whether new agents
/// start on it. `fresh` asks the providers instead of the cache.
pub(super) fn accounts_state(fresh: bool) -> Value {
    let configs = crate::orcha_accounts::accounts();
    let accounts: Vec<Value> = crate::orcha_accounts::accounts_with_fuel(None, fresh)
        .into_iter()
        .map(|account| {
            let default = crate::orcha_accounts::default_account(&configs, &account.kind).as_deref() == Some(account.id.as_str());
            json!({
                "id": account.id, "kind": account.kind, "label": account.label, "email": account.email,
                "current": account.current, "error": account.error, "signed_in": account.signed_in, "default": default,
                "windows": account.windows.iter().map(|window| json!({"label": window.label, "left": window.left, "reset": window.reset})).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({"accounts": accounts})
}

/// Sign in, add, remove, make default or refresh, from the accounts list.
/// Sign-ins run in a new pane inside orcha, where the agent's own login
/// flow (a browser page) finishes them.
pub(super) fn account_action(action: &str, body: &Value) -> Result<Value, String> {
    let id = || {
        let id = body["id"].as_str().ok_or("expected an account")?;
        if crate::orcha_accounts::accounts()
            .iter()
            .any(|account| account.id == id)
        {
            Ok(id.to_owned())
        } else {
            Err(format!("unknown account: {id}"))
        }
    };
    let note = match action {
        "refresh" => {
            return Ok(
                json!({"ok": true, "note": "fuel is up to date", "state": accounts_state(true)}),
            )
        }
        "login" => {
            let id = id()?;
            crate::orcha_accounts::login(&id, None)?;
            "a pane opened in orcha: finish signing in there".to_owned()
        }
        "add" => {
            let name = body["name"].as_str().unwrap_or_default().trim();
            if name.is_empty() || name.len() > 40 {
                return Err("give it a short name, like work or personal".to_owned());
            }
            let id = crate::orcha_accounts::add(name, None)?;
            format!("{id} added: finish signing in in the new pane in orcha")
        }
        "remove" => {
            let id = id()?;
            crate::orcha_accounts::remove(&id)?;
            format!("{id} removed")
        }
        "default" => {
            let id = id()?;
            crate::orcha_accounts::set_default(&id)?;
            format!("new agents start on {id}")
        }
        other => return Err(format!("unknown action: {other}")),
    };
    Ok(json!({"ok": true, "note": note, "state": accounts_state(false)}))
}

fn sections_for(config: &crate::config::Config) -> Vec<Value> {
    let orcha = &config.orcha;
    let ui = &config.ui;
    let json_config = orcha_json();
    let tool = config_tool_root();
    let lang = json_config["lang"].as_str().unwrap_or("en").to_owned();
    let words: Value = tool
        .as_ref()
        .and_then(|root| std::fs::read(root.join("i18n").join(format!("{lang}.json"))).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();

    let accent_value = config
        .theme
        .custom
        .as_ref()
        .and_then(|custom| custom.mauve.clone());
    let accent = ACCENTS
        .iter()
        .find(|(_, color)| color.map(str::to_owned) == accent_value)
        .map_or("theme", |(name, _)| name);
    let width = SIDEBAR_WIDTHS
        .iter()
        .min_by_key(|(_, columns)| columns.abs_diff(ui.sidebar_width))
        .map_or("normal", |(name, _)| name);
    let apps = |names: &[&str], current: &str| {
        let mut options: Vec<Value> = installed_apps(names)
            .into_iter()
            .map(|(label, command)| option(&command, &label))
            .collect();
        if !current.is_empty() && !options.iter().any(|option| option["value"] == current) {
            options.push(option(current, current));
        }
        options
    };

    let mut sections = vec![json!({"id": "look", "title": "Look", "fields": [
            json!({"key": "theme", "label": "Theme", "help": "orcha's colors for panes, panels and this page", "kind": "themes",
                "value": config.theme.name.as_deref().unwrap_or("orcha"), "options": theme_cards()}),
        json!({"key": "accent", "label": "Accent", "help": "the brand color on buttons and highlights", "kind": "swatches", "value": accent,
            "options": ACCENTS.iter().map(|(name, color)| json!({"value": name, "label": name, "color": color})).collect::<Vec<_>>()}),
        select("sidebar_side", "Sidebar side", "the right panel takes the other side", match orcha.sidebar_side {
            crate::config::SidebarSide::Left => "left", crate::config::SidebarSide::Right => "right" },
            vec![option("left", "left"), option("right", "right")]),
        select("sidebar_width", "Sidebar width", "", width,
            SIDEBAR_WIDTHS.iter().map(|(name, _)| option(name, name)).collect()),
        toggle("right_panel", "Right panel", "accounts, changes and PROCESS beside your panes", ui.right_panel),
        select("fuel", "Fuel gauges", "how much each account has left", match orcha.fuel {
            crate::config::FuelPlacement::Panel => "panel", crate::config::FuelPlacement::StatusBar => "status_bar" },
            vec![option("panel", "right panel"), option("status_bar", "status bar")]),
        select("process", "Process", "the running agents as little trains", match orcha.process {
            crate::config::ProcessPlacement::Panel => "panel", crate::config::ProcessPlacement::Sidebar => "sidebar",
            crate::config::ProcessPlacement::Off => "off" },
            vec![option("sidebar", "sidebar"), option("panel", "right panel"), option("off", "hidden")]),
        select("icons", "Agent logos", "", match orcha.icons {
            crate::config::IconMode::Auto => "auto", crate::config::IconMode::Font => "font",
            crate::config::IconMode::Symbols => "symbols" },
            vec![option("auto", "auto"), option("font", "icon font"), option("symbols", "plain symbols")]),
        toggle("pane_paper", "Pane paper", "paint empty pane cells with the theme's paper", orcha.pane_paper),
    ]})];

    let mut font_fields = Vec::new();
    if let Some(root) = tool.as_ref() {
        font_fields.push(json!({"key": "palette", "label": "Terminal palette", "help": "Ghostty's colors, set by orcha-config", "kind": "palettes",
            "value": json_config["palette"], "options": palettes(root)}));
    }
    let font_names = fonts();
    let family = json_config["font"]["family"].as_str().unwrap_or_default();
    if !font_names.is_empty() {
        let mut options: Vec<Value> = font_names.iter().map(|name| option(name, name)).collect();
        if !family.is_empty() && !font_names.iter().any(|name| name == family) {
            options.insert(0, option(family, family));
        }
        font_fields.push(select(
            "font",
            "Font",
            "Ghostty reloads it with ⌘⇧,",
            family,
            options,
        ));
    }
    font_fields.push(
        json!({"key": "font_size", "label": "Font size", "help": "", "kind": "range",
        "min": FONT_SIZES.start(), "max": FONT_SIZES.end(),
        "value": json_config["font"]["size"].as_u64().unwrap_or(15)}),
    );
    font_fields.push(toggle(
        "ligatures",
        "Ligatures",
        "-> and != drawn as one symbol",
        json_config["font"]["ligatures"].as_bool().unwrap_or(true),
    ));
    sections.push(json!({"id": "font", "title": "Terminal & font", "fields": font_fields}));

    sections.push(json!({"id": "apps", "title": "Apps", "fields": [
        select("ide", "IDE", "opens a project's folder", &orcha.ide, apps(IDE_APPS, &orcha.ide)),
        select("git_gui", "Git app", "opens a project's repository", &orcha.git_gui, apps(GIT_GUI_APPS, &orcha.git_gui)),
    ]}));

    sections.push(json!({"id": "alerts", "title": "Notifications", "fields": [
        select("status_indicators", "Status indicators", "how an agent's state is drawn", ui.status_indicators.as_str(),
            vec![option("dots", "dots"), option("symbols", "symbols")]),
        toggle("sound", "Sound", "a sound when an agent finishes or needs you", ui.sound.enabled),
        select("toast", "Toasts", "where a pop-up note goes", match ui.toast.delivery {
            crate::config::ToastDelivery::Off => "off", crate::config::ToastDelivery::Herdr => "herdr",
            crate::config::ToastDelivery::Terminal => "terminal", crate::config::ToastDelivery::System => "system" },
            vec![option("off", "off"), option("herdr", "in orcha"), option("terminal", "terminal"), option("system", "system")]),
    ]}));

    sections.push(json!({"id": "handoff", "title": "Hand-off", "fields": [
        json!({"key": "checklist_percent", "label": "Ask for a checklist below", "help": "orcha asks you, then the agent writes what is done and what is next while it still can; 0: never",
            "kind": "range", "min": 0, "max": 60, "unit": "%", "value": orcha.checklist_percent}),
        json!({"key": "low_fuel_percent", "label": "Offer to switch below", "help": "orcha asks where to continue",
            "kind": "range", "min": 1, "max": 50, "unit": "%", "value": orcha.low_fuel_percent}),
        json!({"key": "auto_handoff_percent", "label": "Move on by itself below", "help": "the chat goes to the account with the most fuel, no question; 0: never",
            "kind": "range", "min": 0, "max": 50, "unit": "%", "value": orcha.auto_handoff_percent}),
        select("switch_mode", "Switching account", "how a chat moves to another account of the same agent", orcha.switch_mode.as_str(), vec![
            option("ask", "ask each time"), option("keep", "keep the chat running"),
            option("restart", "restart the agent"), option("new_pane", "new pane")]),
        text("checklist_file", "Checklist file", "in the project; a hand-off points the next agent at it", &orcha.checklist_file),
    ]}));

    if let Some(widgets) = json_config["widgets"].as_object() {
        let mut fields: Vec<Value> = widgets
            .iter()
            .map(|(name, on)| {
                let label = words[format!("w_{name}")].as_str().unwrap_or(name);
                let (place, text_label) = label.split_once(": ").unwrap_or(("", label));
                json!({"key": format!("widgets.{name}"), "label": text_label, "help": place, "kind": "toggle",
                    "value": on.as_bool().unwrap_or(false)})
            })
            .collect();
        fields.push(text(
            "weather_city",
            "Weather city",
            "empty: found from your location",
            json_config["weather_city"].as_str().unwrap_or_default(),
        ));
        sections.push(json!({"id": "widgets", "title": "Widgets", "fields": fields}));
    }

    let tracker = &json_config["tracker"];
    let repo = &json_config["repo"];
    let mut project = vec![
        text(
            "tracker.task_pattern",
            "Task number in branch names",
            "a regex; its first group is the task number",
            tracker["task_pattern"].as_str().unwrap_or_default(),
        ),
        text(
            "tracker.azure_pat_file",
            "Azure DevOps token file",
            "where the personal access token is kept",
            tracker["azure_pat_file"].as_str().unwrap_or_default(),
        ),
        text(
            "repo.branch_prefix",
            "Branch prefix",
            "new branches start with it",
            repo["branch_prefix"].as_str().unwrap_or_default(),
        ),
        text(
            "repo.base",
            "Base branch",
            "empty: found from the repo",
            repo["base"].as_str().unwrap_or_default(),
        ),
        text(
            "repo.protected",
            "Protected branches",
            "a regex; agents are warned before changing these",
            repo["protected"].as_str().unwrap_or_default(),
        ),
        text(
            "repo.check",
            "Check command",
            "auto: found from the repo, or a command like npm run typecheck",
            repo["check"].as_str().unwrap_or_default(),
        ),
    ];
    if let Some(root) = tool.as_ref() {
        project.push(select(
            "lang",
            "Language",
            "",
            &lang,
            names_in(root.join("i18n"))
                .iter()
                .map(|name| {
                    option(
                        name,
                        match name.as_str() {
                            "en" => "English",
                            "vi" => "Tiếng Việt",
                            other => other,
                        },
                    )
                })
                .collect(),
        ));
    }
    sections.push(json!({"id": "project", "title": "Project & language", "fields": project}));

    plugins_section(&mut sections, orcha.update_plugins);
    sections
}

fn plugins_section(sections: &mut Vec<Value>, update_plugins: bool) {
    sections.push(json!({"id": "plugins", "title": "Plugins", "fields": [
        toggle("update_plugins", "Update plugins by themselves", "off: orcha asks before installing a new version", update_plugins),
    ]}));
}

fn one_of<'a>(value: &'a Value, allowed: &[&str]) -> Result<&'a str, String> {
    let text = value.as_str().ok_or("expected text")?;
    if allowed.contains(&text) {
        Ok(text)
    } else {
        Err(format!("{text} is not one of {}", allowed.join(", ")))
    }
}

fn flag(value: &Value) -> Result<bool, String> {
    value
        .as_bool()
        .ok_or_else(|| "expected on or off".to_owned())
}

/// Saves one setting. Ok(note) carries a hint for the page, if any.
pub(super) fn apply(key: &str, value: &Value) -> Result<Option<String>, String> {
    use crate::config::ConfigEdit;
    let edit = match key {
        "theme" => {
            let name = one_of(value, crate::config::THEME_NAMES)?;
            Some(ConfigEdit::Theme(name))
        }
        "accent" => {
            let names: Vec<&str> = ACCENTS.iter().map(|(name, _)| *name).collect();
            let name = one_of(value, &names)?;
            let color = ACCENTS
                .iter()
                .find(|(each, _)| *each == name)
                .and_then(|(_, color)| *color);
            Some(ConfigEdit::Accent(color))
        }
        "sidebar_width" => {
            let names: Vec<&str> = SIDEBAR_WIDTHS.iter().map(|(name, _)| *name).collect();
            let name = one_of(value, &names)?;
            let columns = SIDEBAR_WIDTHS
                .iter()
                .find(|(each, _)| *each == name)
                .map_or(28, |(_, columns)| *columns);
            Some(ConfigEdit::SidebarWidth(columns))
        }
        "right_panel" => Some(ConfigEdit::RightPanel(flag(value)?)),
        "pane_paper" => Some(ConfigEdit::PanePaper(flag(value)?)),
        "update_plugins" => Some(ConfigEdit::UpdatePlugins(flag(value)?)),
        "sound" => Some(ConfigEdit::Sound(flag(value)?)),
        "sidebar_side" => Some(ConfigEdit::SidebarSide(
            match one_of(value, &["left", "right"])? {
                "left" => crate::config::SidebarSide::Left,
                _ => crate::config::SidebarSide::Right,
            },
        )),
        "fuel" => Some(ConfigEdit::Fuel(
            match one_of(value, &["panel", "status_bar"])? {
                "panel" => crate::config::FuelPlacement::Panel,
                _ => crate::config::FuelPlacement::StatusBar,
            },
        )),
        "process" => Some(ConfigEdit::Process(
            match one_of(value, &["panel", "sidebar", "off"])? {
                "panel" => crate::config::ProcessPlacement::Panel,
                "sidebar" => crate::config::ProcessPlacement::Sidebar,
                _ => crate::config::ProcessPlacement::Off,
            },
        )),
        "icons" => Some(ConfigEdit::Icons(
            match one_of(value, &["auto", "font", "symbols"])? {
                "auto" => crate::config::IconMode::Auto,
                "font" => crate::config::IconMode::Font,
                _ => crate::config::IconMode::Symbols,
            },
        )),
        "status_indicators" => Some(ConfigEdit::StatusIndicators(
            match one_of(value, &["dots", "symbols"])? {
                "dots" => crate::config::StatusIndicatorStyle::Dots,
                _ => crate::config::StatusIndicatorStyle::Symbols,
            },
        )),
        "toast" => Some(ConfigEdit::ToastDelivery(
            match one_of(value, &["off", "herdr", "terminal", "system"])? {
                "off" => crate::config::ToastDelivery::Off,
                "herdr" => crate::config::ToastDelivery::Herdr,
                "terminal" => crate::config::ToastDelivery::Terminal,
                _ => crate::config::ToastDelivery::System,
            },
        )),
        _ => None,
    };
    if let Some(edit) = edit {
        crate::config::write_edit(edit)?;
        reload_server();
        return Ok(None);
    }
    match key {
        "checklist_percent" | "auto_handoff_percent" => {
            let percent = value
                .as_u64()
                .filter(|percent| *percent <= if key == "checklist_percent" { 60 } else { 50 })
                .ok_or("expected a percentage")?;
            write_orcha_value(key, &percent.to_string())?;
            reload_server();
            Ok(None)
        }
        "switch_mode" => {
            let mode = one_of(value, &["ask", "keep", "restart", "new_pane"])?;
            write_orcha_value(key, &format!("\"{mode}\""))?;
            reload_server();
            Ok(None)
        }
        "checklist_file" => {
            let file = value.as_str().ok_or("expected text")?.trim();
            let path = std::path::Path::new(file);
            if file.is_empty()
                || file.contains('\n')
                || path.is_absolute()
                || path
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir))
            {
                return Err("expected a path inside the project, like .orcha/handoff.md".to_owned());
            }
            write_orcha_value(key, &serde_json::to_string(file).unwrap_or_default())?;
            reload_server();
            Ok(None)
        }
        "low_fuel_percent" => {
            let percent = value
                .as_u64()
                .filter(|percent| (1..=50).contains(percent))
                .ok_or("expected 1 to 50")?;
            write_orcha_value("low_fuel_percent", &percent.to_string())?;
            reload_server();
            Ok(None)
        }
        "ide" | "git_gui" => {
            let command = value.as_str().ok_or("expected text")?;
            let names = if key == "ide" { IDE_APPS } else { GIT_GUI_APPS };
            let known = installed_apps(names)
                .into_iter()
                .any(|(_, each)| each == command);
            let current = crate::config::Config::load().config.orcha;
            let unchanged = command
                == if key == "ide" {
                    current.ide
                } else {
                    current.git_gui
                };
            if !known && !unchanged {
                return Err(format!("{command} is not an app orcha found"));
            }
            write_orcha_value(key, &serde_json::to_string(command).unwrap_or_default())?;
            reload_server();
            Ok(None)
        }
        "palette" | "lang" => {
            let text = value.as_str().ok_or("expected text")?;
            let root = config_tool_root().ok_or("orcha-config is not installed")?;
            let dir = if key == "palette" { "palettes" } else { "i18n" };
            if !names_in(root.join(dir)).iter().any(|name| name == text) {
                return Err(format!("unknown {key}: {text}"));
            }
            let command = if key == "palette" { "theme" } else { "lang" };
            run_config_tool(&[command, text])?;
            Ok(Some(ghostty_note()))
        }
        "font" => {
            let family = value.as_str().ok_or("expected text")?;
            if family.is_empty() || family.contains('\n') {
                return Err("expected a font name".to_owned());
            }
            run_config_tool(&["font", family])?;
            Ok(Some(ghostty_note()))
        }
        "font_size" => {
            let size = value
                .as_u64()
                .and_then(|size| u16::try_from(size).ok())
                .filter(|size| FONT_SIZES.contains(size))
                .ok_or("expected a font size")?;
            let family = orcha_json()["font"]["family"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if family.is_empty() {
                return Err("pick a font first".to_owned());
            }
            run_config_tool(&["font", &family, &size.to_string()])?;
            Ok(Some(ghostty_note()))
        }
        "ligatures" => {
            run_config_tool(&["ligatures", if flag(value)? { "on" } else { "off" }])?;
            Ok(Some(ghostty_note()))
        }
        _ if key.starts_with("widgets.") => {
            let name = &key["widgets.".len()..];
            if !orcha_json()["widgets"]
                .get(name)
                .is_some_and(Value::is_boolean)
            {
                return Err(format!("unknown widget: {name}"));
            }
            run_config_tool(&["widget", name, if flag(value)? { "on" } else { "off" }])?;
            Ok(None)
        }
        "weather_city"
        | "tracker.task_pattern"
        | "tracker.azure_pat_file"
        | "repo.branch_prefix"
        | "repo.base"
        | "repo.protected"
        | "repo.check" => {
            let text = value.as_str().ok_or("expected text")?;
            if text.contains('\n') || text.len() > 400 {
                return Err("expected one short line".to_owned());
            }
            let mut config = orcha_json();
            let slot = match key.split_once('.') {
                Some((table, field)) => {
                    if !config[table].is_object() {
                        config[table] = json!({});
                    }
                    &mut config[table][field]
                }
                None => &mut config[key],
            };
            *slot = Value::String(text.to_owned());
            let pretty = serde_json::to_string_pretty(&config).map_err(|err| err.to_string())?;
            std::fs::write(orcha_json_path(), pretty + "\n")
                .map_err(|err| format!("could not save config.json: {err}"))?;
            run_config_tool(&["apply"])?;
            Ok(None)
        }
        _ => Err(format!("unknown setting: {key}")),
    }
}

/// The rules and skills catalogs, the defaults for new projects and
/// orcha's projects.
pub(super) fn projects_state() -> Value {
    use crate::orcha_rules::{RULES, SKILLS};
    json!({
        "defaults": crate::orcha_rules::defaults(),
        "rules": RULES.iter().map(|rule| json!({"id": rule.id, "title": rule.title, "summary": rule.summary, "body": rule.body})).collect::<Vec<_>>(),
        "skills": SKILLS.iter().map(|skill| json!({"name": skill.name, "title": skill.title, "summary": skill.summary,
            "group": skill.group, "source": skill.source, "skills": skill.skills})).collect::<Vec<_>>(),
        "projects": crate::orcha_rules::projects().into_iter()
            .map(|(name, path)| json!({"name": name, "path": path.display().to_string()})).collect::<Vec<_>>(),
    })
}

/// A project the page may touch: only one of orcha's own projects, never a
/// path the request made up.
fn known_project(body: &Value) -> Result<PathBuf, String> {
    let asked = body["path"].as_str().ok_or("expected a project")?;
    crate::orcha_rules::projects()
        .into_iter()
        .map(|(_, path)| path)
        .find(|path| path.to_string_lossy() == asked)
        .ok_or_else(|| format!("{asked} is not one of orcha's projects"))
}

fn rule_ids(body: &Value) -> Vec<String> {
    body["rules"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|id| id.as_str().map(str::to_owned))
        .collect()
}

pub(super) fn project_action(action: &str, body: &Value) -> Result<Value, String> {
    match action {
        "defaults" => {
            crate::orcha_rules::set_defaults(&rule_ids(body))?;
            Ok(json!({"ok": true, "note": "saved · new projects are offered these"}))
        }
        "show" => {
            let project = known_project(body)?;
            let file = crate::orcha_rules::instruction_file(&project);
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            let skills: serde_json::Map<String, Value> = crate::orcha_rules::SKILLS
                .iter()
                .map(|skill| {
                    let (here, global) =
                        crate::orcha_rules::skill_installed(&project, skill.skills[0]);
                    (
                        skill.name.to_owned(),
                        json!({"project": here, "global": global}),
                    )
                })
                .collect();
            Ok(json!({
                "file": file.display().to_string(),
                "file_exists": file.is_file(),
                "applied": crate::orcha_rules::applied_in(&text),
                "skills": skills,
            }))
        }
        "preview" => {
            let project = known_project(body)?;
            let (file, diff, changed) = crate::orcha_rules::preview(&project, &rule_ids(body))?;
            Ok(json!({"file": file.display().to_string(), "diff": diff, "changed": changed}))
        }
        "apply" => {
            let project = known_project(body)?;
            let file = crate::orcha_rules::apply(&project, &rule_ids(body))?;
            Ok(json!({"ok": true, "note": format!("written to {}", file.display())}))
        }
        "install" => {
            let project = known_project(body)?;
            let name = body["skill"].as_str().ok_or("expected a skill")?;
            let note = crate::orcha_rules::install_skill(&project, name, body["global"] == true)?;
            Ok(json!({"ok": true, "note": note}))
        }
        other => Err(format!("unknown action: {other}")),
    }
}

/// Stop or go to something from the background list.
pub(super) fn background_action(body: &Value) -> Result<String, String> {
    let pane = || {
        body["pane_id"]
            .as_str()
            .filter(|pane| !pane.is_empty() && pane.len() < 64)
            .ok_or_else(|| "expected a pane".to_owned())
    };
    let call = |method: &str, params: Value| {
        crate::orcha_handoff::herdr_json(&["api", "call", method, &params.to_string()])
            .map(|_| ())
            .ok_or_else(|| format!("{method} did not work"))
    };
    match body["what"].as_str().unwrap_or_default() {
        "focus" => {
            call("agent.focus", json!({"target": pane()?}))?;
            Ok("focused in orcha".to_owned())
        }
        "plugin_pane" => {
            call("plugin.pane.stop", json!({"pane_id": pane()?}))?;
            Ok("stopped".to_owned())
        }
        "office" => {
            crate::orcha_office::stop_quietly();
            Ok("Easy Job stopped".to_owned())
        }
        "server" => {
            call("server.stop", json!({}))?;
            Ok("the server and every agent stopped".to_owned())
        }
        "settings" => Ok("this page is closing".to_owned()),
        other => Err(format!("unknown action: {other}")),
    }
}

fn ghostty_note() -> String {
    "saved · Ghostty shows it after ⌘⇧, (reload config)".to_owned()
}

fn write_orcha_value(key: &str, value: &str) -> Result<(), String> {
    crate::config::update_file_at(&crate::config::config_path(), key, |content| {
        Ok(crate::config::upsert_section_value(
            content, "orcha", key, value,
        ))
    })
}

fn run_config_tool(args: &[&str]) -> Result<(), String> {
    let tool =
        crate::orcha_app::config_tool().ok_or("orcha-config is not installed: nothing changed")?;
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let output = crate::platform::orcha_tool_command(&tool, &args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|err| format!("could not run orcha-config: {err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let text = String::from_utf8_lossy(&output.stderr);
        Err(text
            .lines()
            .rfind(|line| !line.trim().is_empty())
            .unwrap_or("orcha-config failed")
            .trim()
            .to_owned())
    }
}

/// Tells the running server to read config.toml again; it passes the
/// reload on to every open orcha window.
fn reload_server() {
    let _ = crate::orcha_handoff::herdr(&["server", "reload-config"])
        .stdout(std::process::Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouping_keeps_every_setting() {
        let keys = |sections: &[Value]| -> std::collections::BTreeSet<String> {
            sections
                .iter()
                .flat_map(|section| section["fields"].as_array().cloned().unwrap_or_default())
                .filter_map(|field| field["key"].as_str().map(str::to_owned))
                .collect()
        };
        let all = sections_for(&crate::config::Config::default());
        let grouped = regroup(all.clone());
        assert_eq!(
            keys(&grouped),
            keys(&all),
            "a setting is missing from GROUPS"
        );
    }

    #[test]
    fn unknown_or_out_of_range_values_are_refused() {
        assert!(apply("nope", &json!(true)).is_err());
        assert!(apply("theme", &json!("not-a-theme")).is_err());
        assert!(apply("sidebar_side", &json!("middle")).is_err());
        assert!(apply("right_panel", &json!("yes")).is_err());
        assert!(apply("low_fuel_percent", &json!(0)).is_err());
        assert!(apply("low_fuel_percent", &json!(90)).is_err());
        assert!(apply("repo.base", &json!("a\nb")).is_err());
        assert!(apply("checklist_percent", &json!(61)).is_err());
        assert!(apply("switch_mode", &json!("teleport")).is_err());
        assert!(apply("checklist_file", &json!("/etc/passwd")).is_err());
        assert!(apply("checklist_file", &json!("../outside.md")).is_err());
        assert!(background_action(&json!({"what": "nope"})).is_err());
        assert!(project_action("nope", &json!({})).is_err());
        assert!(account_action("nope", &json!({})).is_err());
        assert!(account_action("remove", &json!({"id": "not-an-account"})).is_err());
        assert!(account_action("add", &json!({"name": ""})).is_err());
        assert!(project_action("apply", &json!({"path": "/etc", "rules": ["slices"]})).is_err());
        assert!(background_action(&json!({"what": "focus"})).is_err());
    }
}
