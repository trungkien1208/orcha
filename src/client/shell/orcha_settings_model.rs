//! The settings page's data: every setting as the page shows it (grouped
//! and worded for people), and the edits behind each control, the projects'
//! rules and skills, the accounts and the background list. The web server
//! in `orcha_settings_web` only routes requests here.

use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::settings::{
    installed_apps, ACCENTS, FONT_SIZES, GIT_GUI_APPS, IDE_APPS, SIDEBAR_WIDTHS,
};
use crate::config::{
    Config, ConfigEdit, FuelPlacement, IconMode, ProcessPlacement, SidebarSide,
    StatusIndicatorStyle, SwitchMode, ToastDelivery,
};

/// How low an account runs, in percent, before each hand-off step: the
/// page's sliders and what saving accepts. 0 turns a step off.
const CHECKLIST_PERCENT: RangeInclusive<u8> = 0..=60;
const LOW_FUEL_PERCENT: RangeInclusive<u8> = 1..=50;
const AUTO_HANDOFF_PERCENT: RangeInclusive<u8> = 0..=50;

/// The lines of text the page edits in config.json: "table.field", or a
/// key at the top.
const JSON_TEXTS: &[&str] = &[
    "weather_city",
    "tracker.task_pattern",
    "tracker.azure_pat_file",
    "repo.branch_prefix",
    "repo.base",
    "repo.protected",
    "repo.check",
];

/// Each variant, the value config.toml and the page call it, and the
/// page's words for it, in the order the page lists them.
type Choices<T> = &'static [(T, &'static str, &'static str)];

/// A config.toml choice the page offers. Showing it and saving it both
/// read its one table, so the two cannot drift apart.
trait Choice: Copy + PartialEq + 'static {
    const CHOICES: Choices<Self>;

    fn as_str(self) -> &'static str {
        Self::CHOICES
            .iter()
            .find(|(each, _, _)| *each == self)
            .map_or("", |(_, value, _)| value)
    }

    /// The variant the page sent; any other value is refused, never guessed.
    fn parse(value: &Value) -> Result<Self, String> {
        let text = value.as_str().ok_or("expected text")?;
        Self::CHOICES
            .iter()
            .find(|(_, each, _)| *each == text)
            .map(|(variant, _, _)| *variant)
            .ok_or_else(|| not_one_of(text, Self::CHOICES.iter().map(|(_, value, _)| *value)))
    }
}

impl Choice for SidebarSide {
    const CHOICES: Choices<Self> = &[
        (Self::Left, "left", "left"),
        (Self::Right, "right", "right"),
    ];
}

impl Choice for FuelPlacement {
    const CHOICES: Choices<Self> = &[
        (Self::Panel, "panel", "right panel"),
        (Self::StatusBar, "status_bar", "status bar"),
    ];
}

impl Choice for ProcessPlacement {
    const CHOICES: Choices<Self> = &[
        (Self::Sidebar, "sidebar", "sidebar"),
        (Self::Panel, "panel", "right panel"),
        (Self::Off, "off", "hidden"),
    ];
}

impl Choice for IconMode {
    const CHOICES: Choices<Self> = &[
        (Self::Auto, "auto", "auto"),
        (Self::Font, "font", "icon font"),
        (Self::Symbols, "symbols", "plain symbols"),
    ];
}

impl Choice for StatusIndicatorStyle {
    const CHOICES: Choices<Self> = &[
        (Self::Dots, "dots", "dots"),
        (Self::Symbols, "symbols", "symbols"),
    ];
}

impl Choice for ToastDelivery {
    const CHOICES: Choices<Self> = &[
        (Self::Off, "off", "off"),
        (Self::Herdr, "herdr", "in orcha"),
        (Self::Terminal, "terminal", "terminal"),
        (Self::System, "system", "system"),
    ];
}

impl Choice for SwitchMode {
    const CHOICES: Choices<Self> = &[
        (Self::Ask, "ask", "ask me"),
        (Self::Keep, "keep", "keep it running"),
        (Self::Restart, "restart", "restart the agent"),
        (Self::NewPane, "new_pane", "new pane"),
    ];
}

fn option(value: &str, label: &str) -> Value {
    json!({"value": value, "label": label})
}

/// The file types orcha remembers how to open (Ctrl+click), A to Z: each one
/// can switch app or be forgotten, so orcha asks again next time.
fn open_with_fields(open_with: &std::collections::HashMap<String, String>) -> Vec<Value> {
    let mut extensions: Vec<_> = open_with.iter().collect();
    extensions.sort();
    extensions
        .into_iter()
        .map(|(extension, choice)| {
            let mut options = vec![
                option("system", "default app"),
                option("folder", "show in folder"),
                option("editor", "editor"),
            ];
            if choice.starts_with("editor:") {
                options.push(option(choice, &format!("editor: {}", &choice[7..])));
            }
            options.push(option("forget", "ask me next time"));
            let mut out = select(&format!("open_with.{extension}"), choice, options);
            out["label"] = json!(format!(".{extension} files"));
            out["help"] = json!("what a Ctrl+click on one opens");
            out
        })
        .collect()
}

/// One setting as the page draws it, in the words WORDS has for `key`;
/// `extra` (an object) adds the options or a slider's bounds.
fn field(key: &str, kind: &str, value: impl Into<Value>, extra: Value) -> Value {
    let (label, help) = words(key);
    let mut out =
        json!({"key": key, "label": label, "help": help, "kind": kind, "value": value.into()});
    if let (Some(out), Value::Object(extra)) = (out.as_object_mut(), extra) {
        out.extend(extra);
    }
    out
}

fn select(key: &str, value: &str, options: Vec<Value>) -> Value {
    field(key, "select", value, json!({"options": options}))
}

fn toggle(key: &str, value: bool) -> Value {
    field(key, "toggle", value, Value::Null)
}

fn text(key: &str, value: &str) -> Value {
    field(key, "text", value, Value::Null)
}

fn percent_slider(key: &str, range: &RangeInclusive<u8>, value: u8) -> Value {
    field(
        key,
        "range",
        value,
        json!({"min": range.start(), "max": range.end(), "unit": "%"}),
    )
}

fn choice<T: Choice>(key: &str, current: T) -> Value {
    let options = T::CHOICES
        .iter()
        .map(|(_, value, label)| option(value, label))
        .collect();
    select(key, current.as_str(), options)
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

fn palettes(root: &Path) -> Vec<Value> {
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

/// A line of text the page edits in config.json (JSON_TEXTS).
fn json_text<'a>(json: &'a Value, key: &str) -> &'a str {
    match key.split_once('.') {
        Some((table, field)) => &json[table][field],
        None => &json[key],
    }
    .as_str()
    .unwrap_or_default()
}

fn language_name(code: &str) -> &str {
    match code {
        "en" => "English",
        "vi" => "Tiếng Việt",
        other => other,
    }
}

/// What orcha-config keeps: its config.json and its checkout (palettes,
/// languages, the widgets' words). Read once per request.
struct ToolFiles {
    /// config.json; `{}` when it is missing or broken.
    json: Value,
    /// orcha-config's checkout, when it is installed.
    root: Option<PathBuf>,
}

impl ToolFiles {
    fn read() -> Self {
        Self {
            json: orcha_json(),
            root: config_tool_root(),
        }
    }

    fn lang(&self) -> &str {
        self.json["lang"].as_str().unwrap_or("en")
    }

    fn font_family(&self) -> &str {
        self.json["font"]["family"].as_str().unwrap_or_default()
    }

    /// The settings orcha-config keeps, as the page shows them.
    fn fields(&self) -> Vec<Value> {
        let mut fields = self.terminal_fields();
        fields.extend(self.widget_fields());
        fields.extend(self.project_fields());
        fields
    }

    /// Ghostty's colors and font.
    fn terminal_fields(&self) -> Vec<Value> {
        let font = &self.json["font"];
        let mut fields = Vec::new();
        if let Some(root) = &self.root {
            fields.push(field(
                "palette",
                "palettes",
                self.json["palette"].clone(),
                json!({"options": palettes(root)}),
            ));
        }
        let font_names = fonts();
        let family = self.font_family();
        if !font_names.is_empty() {
            let mut options: Vec<Value> =
                font_names.iter().map(|name| option(name, name)).collect();
            if !family.is_empty() && !font_names.iter().any(|name| name == family) {
                options.insert(0, option(family, family));
            }
            fields.push(select("font", family, options));
        }
        fields.push(field(
            "font_size",
            "range",
            font["size"].as_u64().unwrap_or(15),
            json!({"min": FONT_SIZES.start(), "max": FONT_SIZES.end()}),
        ));
        fields.push(toggle(
            "ligatures",
            font["ligatures"].as_bool().unwrap_or(true),
        ));
        fields
    }

    /// Each widget orcha-config has, in its own words, and the weather
    /// city; nothing when there are no widgets.
    fn widget_fields(&self) -> Vec<Value> {
        let Some(widgets) = self.json["widgets"].as_object() else {
            return Vec::new();
        };
        let words: Value = self
            .root
            .as_ref()
            .and_then(|root| {
                std::fs::read(root.join("i18n").join(format!("{}.json", self.lang()))).ok()
            })
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let mut fields: Vec<Value> = widgets
            .iter()
            .map(|(name, on)| {
                let label = words[format!("w_{name}")].as_str().unwrap_or(name);
                widget_field(name, label, on.as_bool().unwrap_or(false))
            })
            .collect();
        fields.push(text("weather_city", json_text(&self.json, "weather_city")));
        fields
    }

    /// Branch rules, the task tracker and the language.
    fn project_fields(&self) -> Vec<Value> {
        let mut fields: Vec<Value> = JSON_TEXTS
            .iter()
            .filter(|key| key.contains('.'))
            .map(|key| text(key, json_text(&self.json, key)))
            .collect();
        if let Some(root) = &self.root {
            let options = names_in(root.join("i18n"))
                .iter()
                .map(|name| option(name, language_name(name)))
                .collect();
            fields.push(select("lang", self.lang(), options));
        }
        fields
    }
}

/// A widget's switch. orcha-config words it "Place: what" ("Header",
/// "Claude"); the page says the place in plain words under it.
fn widget_field(name: &str, words: &str, on: bool) -> Value {
    let (place, label) = words.split_once(": ").unwrap_or(("", words));
    let place = match place.to_lowercase().as_str() {
        "" => String::new(),
        "claude" => "in Claude Code".to_owned(),
        other => format!("in the {other}"),
    };
    let mut letters = label.chars();
    let label = letters
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + letters.as_str())
        .unwrap_or_default();
    json!({"key": format!("widgets.{name}"), "label": label, "help": place, "kind": "toggle", "value": on})
}

/// The settings kept in config.toml, as the page shows them. No files are
/// read here, so the defaults come from this too.
fn config_fields(config: &Config) -> Vec<Value> {
    let orcha = &config.orcha;
    let ui = &config.ui;
    let accent_color = config
        .theme
        .custom
        .as_ref()
        .and_then(|custom| custom.mauve.clone());
    let accent = ACCENTS
        .iter()
        .find(|(_, color)| color.map(str::to_owned) == accent_color)
        .map_or("theme", |(name, _)| name);
    let accents: Vec<Value> = ACCENTS
        .iter()
        .map(|(name, color)| json!({"value": name, "label": name, "color": color}))
        .collect();
    let width = SIDEBAR_WIDTHS
        .iter()
        .min_by_key(|(_, columns)| columns.abs_diff(ui.sidebar_width))
        .map_or("normal", |(name, _)| name);
    let widths = SIDEBAR_WIDTHS
        .iter()
        .map(|(name, _)| option(name, name))
        .collect();
    let mut fields = vec![
        field(
            "theme",
            "themes",
            config.theme.name.as_deref().unwrap_or("orcha"),
            json!({"options": theme_cards()}),
        ),
        toggle("theme_auto", config.theme.auto_switch),
        field("accent", "swatches", accent, json!({"options": accents})),
        choice("sidebar_side", orcha.sidebar_side),
        select("sidebar_width", width, widths),
        toggle("right_panel", ui.right_panel),
        choice("fuel", orcha.fuel),
        choice("process", orcha.process),
        choice("icons", orcha.icons),
        toggle("pane_paper", orcha.pane_paper),
        toggle("confirm_close", ui.confirm_close),
        choice("status_indicators", ui.status_indicators),
        toggle("sound", ui.sound.enabled),
        choice("toast", ui.toast.delivery),
        percent_slider(
            "checklist_percent",
            &CHECKLIST_PERCENT,
            orcha.checklist_percent,
        ),
        percent_slider(
            "low_fuel_percent",
            &LOW_FUEL_PERCENT,
            orcha.low_fuel_percent,
        ),
        percent_slider(
            "auto_handoff_percent",
            &AUTO_HANDOFF_PERCENT,
            orcha.auto_handoff_percent,
        ),
        choice("switch_mode", orcha.switch_mode),
        text("checklist_file", &orcha.checklist_file),
        toggle("update_plugins", orcha.update_plugins),
    ];
    fields.extend(open_with_fields(&orcha.open_with));
    fields
}

/// The apps that open a project's folder and its repository: the ones
/// installed, and the one set now even when it is not found.
fn app_fields(config: &Config) -> Vec<Value> {
    let apps = |key: &str, names: &[&str], current: &str| {
        let mut options: Vec<Value> = installed_apps(names)
            .into_iter()
            .map(|(label, command)| option(&command, &label))
            .collect();
        if !current.is_empty() && !options.iter().any(|option| option["value"] == current) {
            options.push(option(current, current));
        }
        select(key, current, options)
    };
    vec![
        apps("ide", IDE_APPS, &config.orcha.ide),
        apps("git_gui", GIT_GUI_APPS, &config.orcha.git_gui),
    ]
}

/// Every setting, ungrouped.
fn fields(config: &Config, files: &ToolFiles) -> Vec<Value> {
    let mut fields = config_fields(config);
    fields.extend(app_fields(config));
    fields.extend(files.fields());
    fields
}

/// What each DEFAULT_KEYS setting is out of the box: a dot marks a change,
/// ↺ goes back.
fn default_values() -> HashMap<String, Value> {
    config_fields(&Config::default())
        .into_iter()
        .filter_map(|field| {
            let key = field["key"].as_str()?;
            DEFAULT_KEYS
                .contains(&key)
                .then(|| (key.to_owned(), field["value"].clone()))
        })
        .collect()
}

/// Everything the page shows. config.toml, config.json and orcha-config's
/// files are each read once.
pub(super) fn state() -> Value {
    let config = Config::load().config;
    let files = ToolFiles::read();
    let defaults = default_values();
    let mut fields = fields(&config, &files);
    for field in &mut fields {
        let key = field["key"].as_str().unwrap_or_default().to_owned();
        if let Some(default) = defaults.get(&key) {
            field["default"] = default.clone();
        }
        if let Some(warning) = warning_for(&key, &config, &files) {
            field["warn"] = json!(warning);
        }
    }
    json!({
        "version": crate::orcha_app::ORCHA_VERSION,
        "sections": regroup(&fields),
        "has_config_tool": files.root.is_some(),
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
            ("orcha", &["theme", "theme_auto", "accent"]),
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
            ("Closing", &["confirm_close"]),
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
            ("Opening files", &["open_with.*"]),
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

/// Plain words for each setting: (key, label, help). The only place a
/// setting's label and help are written; a choice's option words live in
/// its `Choice` table, widgets bring theirs from orcha-config.
const WORDS: &[(&str, &str, &str)] = &[
    (
        "theme",
        "App colors",
        "orcha's panels, its sidebar and this page",
    ),
    (
        "theme_auto",
        "Follow light and dark",
        "switch to the light or dark version of these colors with your terminal",
    ),
    (
        "confirm_close",
        "Ask before closing a project",
        "one last check before a project and its agents close",
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

/// A setting's label and help, from WORDS.
fn words(key: &str) -> (&str, &str) {
    WORDS
        .iter()
        .find(|(each, _, _)| *each == key)
        .map_or((key, ""), |(_, label, help)| (*label, *help))
}

/// The settings, grouped by task; keys stay as they are.
fn regroup(fields: &[Value]) -> Vec<Value> {
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
    "theme_auto",
    "confirm_close",
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
fn warning_for(key: &str, config: &Config, files: &ToolFiles) -> Option<String> {
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
        "process" if orcha.process == ProcessPlacement::Panel && !config.ui.right_panel => {
            Some("the right panel is off, so PROCESS shows in the sidebar for now".to_owned())
        }
        "fuel" if orcha.fuel == FuelPlacement::Panel && !config.ui.right_panel => {
            Some("the right panel is off: turn it on, or move the gauges to the status bar".to_owned())
        }
        "font" => {
            let family = files.font_family();
            let found = fonts();
            (!family.is_empty() && !found.is_empty() && !found.iter().any(|name| name == family))
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

fn not_one_of<'a>(text: &str, allowed: impl Iterator<Item = &'a str>) -> String {
    format!(
        "{text} is not one of {}",
        allowed.collect::<Vec<_>>().join(", ")
    )
}

fn one_of<'a>(value: &'a Value, allowed: &[&str]) -> Result<&'a str, String> {
    let text = value.as_str().ok_or("expected text")?;
    if allowed.contains(&text) {
        Ok(text)
    } else {
        Err(not_one_of(text, allowed.iter().copied()))
    }
}

/// The item a (name, item) table holds under the name the page sent.
fn named<T: Copy>(value: &Value, table: &[(&str, T)]) -> Result<T, String> {
    let text = value.as_str().ok_or("expected text")?;
    table
        .iter()
        .find(|(name, _)| *name == text)
        .map(|(_, item)| *item)
        .ok_or_else(|| not_one_of(text, table.iter().map(|(name, _)| *name)))
}

fn flag(value: &Value) -> Result<bool, String> {
    value
        .as_bool()
        .ok_or_else(|| "expected on or off".to_owned())
}

fn on_off(value: &Value) -> Result<&'static str, String> {
    Ok(if flag(value)? { "on" } else { "off" })
}

fn percent(value: &Value, range: &RangeInclusive<u8>) -> Result<u8, String> {
    value
        .as_u64()
        .and_then(|percent| u8::try_from(percent).ok())
        .filter(|percent| range.contains(percent))
        .ok_or_else(|| format!("expected {} to {}", range.start(), range.end()))
}

fn one_line(value: &Value) -> Result<&str, String> {
    let text = value.as_str().ok_or("expected text")?;
    if text.contains('\n') || text.len() > 400 {
        return Err("expected one short line".to_owned());
    }
    Ok(text)
}

/// Text as a TOML string (a JSON string is one).
fn toml_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

fn checklist_file(value: &Value) -> Result<&str, String> {
    let file = value.as_str().ok_or("expected text")?.trim();
    let path = Path::new(file);
    if file.is_empty()
        || file.contains('\n')
        || path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err("expected a path inside the project, like .orcha/handoff.md".to_owned());
    }
    Ok(file)
}

/// An app orcha found, or the one set now.
fn app_command<'a>(key: &str, value: &'a Value) -> Result<&'a str, String> {
    let command = value.as_str().ok_or("expected text")?;
    let names = if key == "ide" { IDE_APPS } else { GIT_GUI_APPS };
    let known = installed_apps(names)
        .into_iter()
        .any(|(_, each)| each == command);
    let current = Config::load().config.orcha;
    let unchanged = command
        == if key == "ide" {
            current.ide
        } else {
            current.git_gui
        };
    if !known && !unchanged {
        return Err(format!("{command} is not an app orcha found"));
    }
    Ok(command)
}

/// A palette or a language orcha-config has a file for.
fn tool_file<'a>(key: &str, value: &'a Value) -> Result<&'a str, String> {
    let text = value.as_str().ok_or("expected text")?;
    let root = config_tool_root().ok_or("orcha-config is not installed")?;
    let dir = if key == "palette" { "palettes" } else { "i18n" };
    if !names_in(root.join(dir)).iter().any(|name| name == text) {
        return Err(format!("unknown {key}: {text}"));
    }
    Ok(text)
}

fn font_family(value: &Value) -> Result<&str, String> {
    let family = value.as_str().ok_or("expected text")?;
    if family.is_empty() || family.contains('\n') {
        return Err("expected a font name".to_owned());
    }
    Ok(family)
}

/// orcha-config sets the size together with the font in use.
fn font_size_args(value: &Value) -> Result<Vec<String>, String> {
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
    Ok(vec!["font".to_owned(), family, size.to_string()])
}

fn widget_args(key: &str, value: &Value) -> Result<Vec<String>, String> {
    let name = key.strip_prefix("widgets.").unwrap_or(key);
    if !orcha_json()["widgets"]
        .get(name)
        .is_some_and(Value::is_boolean)
    {
        return Err(format!("unknown widget: {name}"));
    }
    Ok(args(&["widget", name, on_off(value)?]))
}

fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

/// What saving one setting changes: checked, not written yet.
enum Change<'a> {
    /// A typed edit to config.toml.
    Edit(ConfigEdit<'a>),
    /// A value under `[orcha]` in config.toml, already written as TOML.
    Orcha(String),
    /// An orcha-config command; `ghostty` when Ghostty shows it only after
    /// it reloads its config.
    Tool { args: Vec<String>, ghostty: bool },
    /// A line of text in config.json (JSON_TEXTS), which `orcha-config
    /// apply` then hands on.
    Json(&'a str),
}

/// A remembered file type's new app, or forgetting it.
fn open_with_change<'a>(extension: &'a str, value: &'a Value) -> Result<Change<'a>, String> {
    if extension.is_empty()
        || !extension
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    {
        return Err(format!("unknown file type: {extension}"));
    }
    let choice = value.as_str().ok_or("expected a choice")?;
    let choice = match choice {
        "forget" => None,
        "system" | "folder" | "editor" => Some(choice),
        _ if choice.starts_with("editor:") && choice.len() > 7 => Some(choice),
        _ => return Err(format!("unknown choice: {choice}")),
    };
    Ok(Change::Edit(ConfigEdit::OpenWith(extension, choice)))
}

/// Checks the value the page sent for `key` and says what saving it
/// changes. Nothing is written here.
fn change_for<'a>(key: &'a str, value: &'a Value) -> Result<Change<'a>, String> {
    use ConfigEdit as Edit;
    if let Some(extension) = key.strip_prefix("open_with.") {
        return open_with_change(extension, value);
    }
    let change = match key {
        "theme" => Change::Edit(Edit::Theme(one_of(value, crate::config::THEME_NAMES)?)),
        "accent" => Change::Edit(Edit::Accent(named(value, &ACCENTS)?)),
        "sidebar_width" => Change::Edit(Edit::SidebarWidth(named(value, &SIDEBAR_WIDTHS)?)),
        "sidebar_side" => Change::Edit(Edit::SidebarSide(Choice::parse(value)?)),
        "fuel" => Change::Edit(Edit::Fuel(Choice::parse(value)?)),
        "process" => Change::Edit(Edit::Process(Choice::parse(value)?)),
        "icons" => Change::Edit(Edit::Icons(Choice::parse(value)?)),
        "status_indicators" => Change::Edit(Edit::StatusIndicators(Choice::parse(value)?)),
        "toast" => Change::Edit(Edit::ToastDelivery(Choice::parse(value)?)),
        "right_panel" => Change::Edit(Edit::RightPanel(flag(value)?)),
        "pane_paper" => Change::Edit(Edit::PanePaper(flag(value)?)),
        "theme_auto" => Change::Edit(Edit::ThemeAutoSwitch(flag(value)?)),
        "confirm_close" => Change::Edit(Edit::ConfirmClose(flag(value)?)),
        "update_plugins" => Change::Edit(Edit::UpdatePlugins(flag(value)?)),
        "sound" => Change::Edit(Edit::Sound(flag(value)?)),
        "checklist_percent" => Change::Orcha(percent(value, &CHECKLIST_PERCENT)?.to_string()),
        "low_fuel_percent" => Change::Orcha(percent(value, &LOW_FUEL_PERCENT)?.to_string()),
        "auto_handoff_percent" => Change::Orcha(percent(value, &AUTO_HANDOFF_PERCENT)?.to_string()),
        "switch_mode" => {
            let mode: SwitchMode = Choice::parse(value)?;
            Change::Orcha(toml_string(Choice::as_str(mode)))
        }
        "checklist_file" => Change::Orcha(toml_string(checklist_file(value)?)),
        "ide" | "git_gui" => Change::Orcha(toml_string(app_command(key, value)?)),
        "palette" => Change::Tool {
            args: args(&["theme", tool_file(key, value)?]),
            ghostty: true,
        },
        "lang" => Change::Tool {
            args: args(&["lang", tool_file(key, value)?]),
            ghostty: true,
        },
        "font" => Change::Tool {
            args: args(&["font", font_family(value)?]),
            ghostty: true,
        },
        "font_size" => Change::Tool {
            args: font_size_args(value)?,
            ghostty: true,
        },
        "ligatures" => Change::Tool {
            args: args(&["ligatures", on_off(value)?]),
            ghostty: true,
        },
        _ if key.starts_with("widgets.") => Change::Tool {
            args: widget_args(key, value)?,
            ghostty: false,
        },
        _ if JSON_TEXTS.contains(&key) => Change::Json(one_line(value)?),
        _ => return Err(format!("unknown setting: {key}")),
    };
    Ok(change)
}

/// What saving one setting did, once it was written.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Saved {
    /// A hint for the page, if any.
    pub(super) note: Option<String>,
    /// The value is saved, but a step after it failed: the page shows it
    /// as a problem, not as a failed save.
    pub(super) warning: Option<String>,
}

/// Saves one setting. Err means nothing was saved.
pub(super) fn apply(key: &str, value: &Value) -> Result<Saved, String> {
    match change_for(key, value)? {
        Change::Edit(edit) => {
            crate::config::write_edit(edit)?;
            reload_server();
            Ok(Saved::default())
        }
        Change::Orcha(toml) => {
            write_orcha_value(key, &toml)?;
            reload_server();
            Ok(Saved::default())
        }
        Change::Tool { args, ghostty } => {
            run_config_tool(&args)?;
            Ok(Saved {
                note: ghostty.then(ghostty_note),
                warning: None,
            })
        }
        Change::Json(text) => {
            save_json_text(&orcha_json_path(), key, text)?;
            Ok(applied_after_saving(run_config_tool(&["apply"])))
        }
    }
}

/// config.json is written before `orcha-config apply` hands it on: when
/// that fails the value stays saved, so it is a warning, not an error.
fn applied_after_saving(applied: Result<(), String>) -> Saved {
    Saved {
        note: None,
        warning: applied
            .err()
            .map(|why| format!("Saved, but orcha-config could not apply it yet: {why}")),
    }
}

/// Puts one line of text at `key` ("table.field" or a top-level key).
fn set_json_text(config: &mut Value, key: &str, text: &str) -> Result<(), String> {
    if !config.is_object() {
        return Err("config.json is not a JSON object, so it was left alone".to_owned());
    }
    let slot = match key.split_once('.') {
        Some((table, field)) => {
            if config.get(table).is_some_and(|value| !value.is_object()) {
                return Err(format!(
                    "{table} in config.json is not a JSON object, so it was left alone"
                ));
            }
            if config.get(table).is_none() {
                config[table] = json!({});
            }
            &mut config[table][field]
        }
        None => &mut config[key],
    };
    *slot = Value::String(text.to_owned());
    Ok(())
}

/// Puts one line of text into config.json through the helper that saves
/// config.toml (`write_orcha_value`): the file is read right before it is
/// written, and one that is not valid JSON is left alone.
fn save_json_text(path: &Path, key: &str, text: &str) -> Result<(), String> {
    crate::config::update_file_at(path, "config.json", |content| {
        let mut config: Value = if content.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(content).map_err(|err| {
                format!("config.json is not valid JSON, so it was left alone: {err}")
            })?
        };
        set_json_text(&mut config, key, text)?;
        let pretty = serde_json::to_string_pretty(&config).map_err(|err| err.to_string())?;
        Ok(pretty + "\n")
    })
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
            crate::orcha_office::stop_quietly()?;
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

fn run_config_tool<S: AsRef<str>>(args: &[S]) -> Result<(), String> {
    let tool =
        crate::orcha_app::config_tool().ok_or("orcha-config is not installed: nothing changed")?;
    let args: Vec<String> = args.iter().map(|arg| arg.as_ref().to_owned()).collect();
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

    /// orcha-config's files as a test sees them: widgets, no checkout.
    fn tool_files() -> ToolFiles {
        ToolFiles {
            json: json!({"widgets": {"clock": true, "weather": false}, "repo": {"base": "main"}}),
            root: None,
        }
    }

    fn keys(fields: &[Value]) -> std::collections::BTreeSet<String> {
        fields
            .iter()
            .filter_map(|field| field["key"].as_str().map(str::to_owned))
            .collect()
    }

    #[test]
    fn grouping_keeps_every_setting() {
        let all = fields(&Config::default(), &tool_files());
        let grouped: Vec<Value> = regroup(&all)
            .iter()
            .flat_map(|section| section["fields"].as_array().cloned().unwrap_or_default())
            .collect();
        assert_eq!(
            keys(&grouped),
            keys(&all),
            "a setting is missing from GROUPS"
        );
    }

    #[test]
    fn every_setting_has_its_words() {
        for field in fields(&Config::default(), &tool_files()) {
            let key = field["key"].as_str().unwrap_or_default();
            if key.starts_with("widgets.") {
                continue;
            }
            assert!(
                WORDS.iter().any(|(each, _, _)| *each == key),
                "{key} has no entry in WORDS"
            );
        }
        let clock = widget_field("clock", "header: clock", true);
        assert_eq!(clock["label"], "Clock");
        assert_eq!(clock["help"], "in the header");
    }

    #[test]
    fn sliders_and_saving_share_their_bounds() {
        let fields = config_fields(&Config::default());
        for (key, range) in [
            ("checklist_percent", &CHECKLIST_PERCENT),
            ("low_fuel_percent", &LOW_FUEL_PERCENT),
            ("auto_handoff_percent", &AUTO_HANDOFF_PERCENT),
        ] {
            let field = fields.iter().find(|field| field["key"] == key).unwrap();
            assert_eq!(field["min"], json!(range.start()), "{key}");
            assert_eq!(field["max"], json!(range.end()), "{key}");
            for ok in [*range.start(), *range.end()] {
                assert!(
                    matches!(change_for(key, &json!(ok)), Ok(Change::Orcha(text)) if text == ok.to_string()),
                    "{key} refused {ok}"
                );
            }
            assert!(change_for(key, &json!(range.end() + 1)).is_err(), "{key}");
            if let Some(below) = range.start().checked_sub(1) {
                assert!(change_for(key, &json!(below)).is_err(), "{key}");
            }
            assert!(change_for(key, &json!(-1)).is_err(), "{key}");
            assert!(change_for(key, &json!("10")).is_err(), "{key}");
        }
    }

    /// Every value a choice shows comes back as its own variant, and is the
    /// name config.toml itself reads.
    fn round_trip<T>()
    where
        T: Choice + serde::de::DeserializeOwned + std::fmt::Debug,
    {
        let mut seen = std::collections::BTreeSet::new();
        for (variant, value, label) in T::CHOICES {
            assert!(seen.insert(*value), "{value} is listed twice");
            assert!(!label.is_empty(), "{value} has no words");
            assert_eq!(variant.as_str(), *value);
            assert_eq!(T::parse(&json!(value)), Ok(*variant));
            let from_toml: T = serde_json::from_value(json!(value)).unwrap();
            assert_eq!(from_toml, *variant, "config.toml reads {value} differently");
        }
        assert!(T::parse(&json!("nowhere")).is_err());
        assert!(T::parse(&json!(1)).is_err());
    }

    #[test]
    fn choices_round_trip_and_refuse_unknown_values() {
        round_trip::<SidebarSide>();
        round_trip::<FuelPlacement>();
        round_trip::<ProcessPlacement>();
        round_trip::<IconMode>();
        round_trip::<StatusIndicatorStyle>();
        round_trip::<ToastDelivery>();
        round_trip::<SwitchMode>();
        let fuel = choice("fuel", FuelPlacement::StatusBar);
        assert_eq!(fuel["value"], "status_bar");
        assert_eq!(
            fuel["options"][1],
            json!({"value": "status_bar", "label": "status bar"})
        );
        assert_eq!(
            change_for("fuel", &json!("roof")).err().as_deref(),
            Some("roof is not one of panel, status_bar")
        );
        assert!(matches!(
            change_for("switch_mode", &json!("new_pane")),
            Ok(Change::Orcha(text)) if text == "\"new_pane\""
        ));
        assert!(change_for("toast", &json!("pigeon")).is_err());
        assert!(change_for("sidebar_width", &json!("huge")).is_err());
        assert!(change_for("accent", &json!("plaid")).is_err());
        assert!(matches!(
            change_for("theme_auto", &json!(true)),
            Ok(Change::Edit(ConfigEdit::ThemeAutoSwitch(true)))
        ));
        assert!(matches!(
            change_for("confirm_close", &json!(false)),
            Ok(Change::Edit(ConfigEdit::ConfirmClose(false)))
        ));
        assert!(matches!(
            change_for("open_with.png", &json!("folder")),
            Ok(Change::Edit(ConfigEdit::OpenWith("png", Some("folder"))))
        ));
        assert!(matches!(
            change_for("open_with.md", &json!("forget")),
            Ok(Change::Edit(ConfigEdit::OpenWith("md", None)))
        ));
        assert!(matches!(
            change_for("open_with.md", &json!("editor:zed {path}")),
            Ok(Change::Edit(ConfigEdit::OpenWith(
                "md",
                Some("editor:zed {path}")
            )))
        ));
        assert!(change_for("open_with.md", &json!("browser")).is_err());
        assert!(change_for("open_with.../x", &json!("system")).is_err());
        assert!(change_for("open_with.", &json!("system")).is_err());
    }

    /// Every variant of an enum, through a match with no wildcard: a variant
    /// added to the enum stops this compiling until it is listed here too.
    macro_rules! every_variant {
        ($ty:ident: $($variant:ident),+ $(,)?) => {{
            let _exhaustive = |value: $ty| match value {
                $($ty::$variant)|+ => (),
            };
            [$($ty::$variant),+]
        }};
    }

    /// Each variant has a row in its `Choice` table, so `as_str` never
    /// falls back to "" and the page never shows a blank value.
    fn listed<T: Choice + std::fmt::Debug>(variants: &[T]) {
        assert_eq!(variants.len(), T::CHOICES.len(), "{variants:?}");
        for variant in variants {
            assert!(!variant.as_str().is_empty(), "{variant:?} has no value");
        }
    }

    #[test]
    fn every_choice_variant_has_a_value() {
        listed(&every_variant!(SidebarSide: Left, Right));
        listed(&every_variant!(FuelPlacement: Panel, StatusBar));
        listed(&every_variant!(ProcessPlacement: Sidebar, Panel, Off));
        listed(&every_variant!(IconMode: Auto, Font, Symbols));
        listed(&every_variant!(StatusIndicatorStyle: Dots, Symbols));
        listed(&every_variant!(ToastDelivery: Off, Herdr, Terminal, System));
        listed(&every_variant!(SwitchMode: Ask, Keep, Restart, NewPane));
    }

    #[test]
    fn a_failed_apply_after_saving_is_a_warning_not_an_error() {
        assert_eq!(applied_after_saving(Ok(())), Saved::default());
        let saved = applied_after_saving(Err("no palette".to_owned()));
        assert_eq!(saved.note, None);
        let warning = saved.warning.unwrap_or_default();
        assert!(warning.starts_with("Saved, but"), "{warning}");
        assert!(warning.ends_with("no palette"), "{warning}");
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

    #[test]
    fn config_json_texts_keep_the_rest_of_the_file() {
        let dir = std::env::temp_dir().join(format!("orcha-settings-json-{}", std::process::id()));
        let path = dir.join("config.json");
        let _ = std::fs::remove_dir_all(&dir);
        // a missing file starts empty
        save_json_text(&path, "weather_city", "Hanoi").unwrap();
        let fresh = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, r#"{"lang": "en", "repo": {"base": "main"}}"#).unwrap();
        save_json_text(&path, "repo.check", "npm test").unwrap();
        save_json_text(&path, "tracker.task_pattern", "#(\\d+)").unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        let broken = save_json_text(&path, "repo.base", "dev");
        let left = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(
            serde_json::from_str::<Value>(&fresh).unwrap(),
            json!({"weather_city": "Hanoi"})
        );
        assert!(written.ends_with("}\n"));
        let config: Value = serde_json::from_str(&written).unwrap();
        assert_eq!(config["lang"], "en");
        assert_eq!(json_text(&config, "repo.base"), "main");
        assert_eq!(json_text(&config, "repo.check"), "npm test");
        assert_eq!(json_text(&config, "tracker.task_pattern"), "#(\\d+)");
        assert!(broken.is_err());
        assert_eq!(left, "{ not json", "a broken config.json was overwritten");
        assert!(set_json_text(&mut json!([]), "weather_city", "Hanoi").is_err());
    }

    #[test]
    fn a_json_table_with_an_unexpected_type_is_left_untouched() {
        let dir = std::env::temp_dir().join(format!("orcha-settings-shape-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let old = r#"{"repo":"managed elsewhere","weather_city":"Hanoi"}"#;
        std::fs::write(&path, old).unwrap();
        assert!(save_json_text(&path, "repo.base", "main").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), old);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
