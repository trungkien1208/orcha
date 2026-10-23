use super::*;
use crossterm::event::{KeyCode, KeyModifiers};

/// The built-in name a theme goes by (old names like "brutal" included).
pub(super) fn normalized_theme_name(name: &str) -> String {
    let normalized = name.to_lowercase().replace([' ', '_'], "-");
    crate::config::canonical_theme_name(&normalized)
        .map(str::to_owned)
        .unwrap_or(normalized)
}

/// LOOK: accent colors offered for the brand (`[theme.custom] mauve`).
/// The first keeps the theme's own.
pub(super) const ACCENTS: [(&str, Option<&str>); 8] = [
    ("theme", None),
    ("pink", Some("#ff7ab6")),
    ("coral", Some("#ff8a65")),
    ("orange", Some("#ffa94d")),
    ("yellow", Some("#ffd23f")),
    ("green", Some("#7be0ad")),
    ("blue", Some("#6c8cff")),
    ("purple", Some("#b18cff")),
];

/// LOOK: sidebar widths, in columns.
pub(super) const SIDEBAR_WIDTHS: [(&str, u16); 3] = [("narrow", 24), ("normal", 28), ("wide", 36)];

/// Where the LOOK sketch draws PROCESS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum MapProcess {
    #[default]
    Sidebar,
    Panel,
    Off,
}

/// What the LOOK sketch shows: the layout the current values make.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct LookMap {
    pub(super) sidebar_right: bool,
    /// Index into `SIDEBAR_WIDTHS`.
    pub(super) sidebar: usize,
    pub(super) panel: bool,
    pub(super) process: MapProcess,
    pub(super) fuel_in_status: bool,
}

/// The LOOK rows, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LookRow {
    Accent,
    Sidebar,
    RightPanel,
    Fuel,
    Panes,
    Logos,
    Process,
    Side,
}

impl LookRow {
    pub(super) const ALL: [Self; 8] = [
        Self::Accent,
        Self::Side,
        Self::Sidebar,
        Self::RightPanel,
        Self::Fuel,
        Self::Process,
        Self::Panes,
        Self::Logos,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Accent => "accent",
            Self::Sidebar => "sidebar",
            Self::RightPanel => "right panel",
            Self::Fuel => "fuel gauges",
            Self::Panes => "panes",
            Self::Logos => "agent logos",
            Self::Process => "process",
            Self::Side => "sidebar side",
        }
    }
}

/// The accent in use: the offered color the palette's brand matches, else
/// the theme's own.
pub(super) fn accent_index(palette: &Palette) -> usize {
    ACCENTS
        .iter()
        .position(|(_, hex)| {
            hex.is_some_and(|hex| crate::config::parse_color(hex) == palette.mauve)
        })
        .unwrap_or(0)
}

/// The sidebar width offered closest to `width`.
pub(super) fn sidebar_width_index(width: u16) -> usize {
    SIDEBAR_WIDTHS
        .iter()
        .enumerate()
        .min_by_key(|(_, (_, columns))| columns.abs_diff(width))
        .map_or(1, |(index, _)| index)
}

fn theme_index(name: &str) -> usize {
    let normalized = normalized_theme_name(name);
    crate::config::THEME_NAMES
        .iter()
        .position(|candidate| normalized_theme_name(candidate) == normalized)
        .unwrap_or(0)
}

/// Font sizes offered for the terminal, in points.
pub(super) const FONT_SIZES: std::ops::RangeInclusive<u16> = 11..=20;
/// orcha-config's default font size.
const DEFAULT_FONT_SIZE: u16 = 15;

fn font_size_index(size: u16) -> usize {
    usize::from(size.clamp(*FONT_SIZES.start(), *FONT_SIZES.end()) - FONT_SIZES.start())
}

fn indicator_index(style: crate::config::StatusIndicatorStyle) -> usize {
    usize::from(style == crate::config::StatusIndicatorStyle::Symbols)
}

fn toast_index(delivery: crate::config::ToastDelivery) -> usize {
    match delivery {
        crate::config::ToastDelivery::Off => 0,
        crate::config::ToastDelivery::Herdr => 1,
        crate::config::ToastDelivery::Terminal => 2,
        crate::config::ToastDelivery::System => 3,
    }
}

pub(super) fn integration_needs_install(info: &crate::api::schema::IntegrationInfo) -> bool {
    info.state == crate::api::schema::IntegrationState::Outdated
        || info.available && info.state == crate::api::schema::IntegrationState::NotInstalled
}

/// The terminal's fonts (Ghostty lists them) and the font and ligatures
/// orcha-config set.
fn load_appearance() -> OrchaAppearance {
    if cfg!(test) {
        return OrchaAppearance::default();
    }
    let fonts = std::process::Command::new("ghostty")
        .arg("+list-fonts")
        .output()
        .ok()
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter(|line| !line.is_empty() && !line.starts_with(char::is_whitespace))
                .filter(|line| *line != "Agent Icons")
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let config: serde_json::Value =
        std::fs::read_to_string(crate::config::config_dir().join("config.json"))
            .ok()
            .or_else(|| {
                crate::platform::orcha_config_base_dir()
                    .and_then(|base| std::fs::read_to_string(base.join("orcha/config.json")).ok())
            })
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
    OrchaAppearance {
        ides: installed_apps(IDE_APPS),
        git_guis: installed_apps(GIT_GUI_APPS),
        fonts,
        font: config["font"]["family"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        font_size: config["font"]["size"]
            .as_u64()
            .and_then(|size| u16::try_from(size).ok())
            .unwrap_or(DEFAULT_FONT_SIZE),
        ligatures: config["font"]["ligatures"].as_bool().unwrap_or(true),
        note: None,
    }
}

/// IDEs orcha knows how to open a folder in, with platform-specific commands.
pub(super) const IDE_APPS: &[&str] = &[
    "Cursor",
    "Visual Studio Code",
    "Zed",
    "Windsurf",
    "WebStorm",
    "IntelliJ IDEA",
    "IntelliJ IDEA CE",
    "PyCharm",
    "GoLand",
    "RustRover",
    "Sublime Text",
    "Nova",
    "Xcode",
];
/// Git GUIs orcha knows how to open a repository in.
pub(super) const GIT_GUI_APPS: &[&str] = &[
    "Fork",
    "GitHub Desktop",
    "Tower",
    "Sourcetree",
    "GitKraken",
    "Sublime Merge",
    "SmartGit",
];

/// "auto" (the first one found), then each app that is installed, with
/// the command opening a folder in it.
pub(super) fn installed_apps(names: &[&str]) -> Vec<(String, String)> {
    let mut out = vec![("auto: the first one found".to_owned(), String::new())];
    if cfg!(test) {
        return out;
    }
    if let Some(apps) = crate::platform::orcha_installed_apps(names) {
        out.extend(apps);
        return out;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    for name in names {
        let found = [
            format!("/Applications/{name}.app"),
            format!("{home}/Applications/{name}.app"),
        ]
        .iter()
        .any(|path| std::path::Path::new(path).is_dir());
        if found {
            out.push(((*name).to_owned(), format!("open -a '{name}' {{path}}")));
        }
    }
    out
}

/// Runs orcha-config with `args` in the background (it rewrites the
/// terminal's config).
fn run_config_tool(args: Vec<String>) -> Option<String> {
    let Some(tool) = crate::orcha_app::config_tool() else {
        return Some("orcha-config is not installed: nothing changed".to_owned());
    };
    let _ = crate::thread_spawn::spawn_named("orcha-config", move || {
        let _ = crate::platform::orcha_tool_command(&tool, &args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
    Some("saved · reload the terminal's config (Ghostty: ⌘⇧,) to see it".to_owned())
}

impl ClientShellState {
    pub(super) fn open_settings_overlay(&mut self) {
        self.overlay = Some(ClientShellOverlay::Settings(ClientSettingsOverlay {
            section: ClientSettingsSection::Theme,
            selected: theme_index(&self.config.theme_name),
            original_theme_name: self.config.theme_name.clone(),
            original_palette: self.config.palette.clone(),
            integrations: Vec::new(),
            integration_messages: Vec::new(),
            loading_integrations: false,
            installing_integrations: false,
            appearance: load_appearance(),
            look: Vec::new(),
            look_map: LookMap::default(),
        }));
        self.refresh_look();
    }

    /// Recomputes the LOOK rows shown in the open settings.
    fn refresh_look(&mut self) {
        let look: Vec<String> = LookRow::ALL
            .iter()
            .map(|row| self.look_value(*row))
            .collect();
        let map = self.look_map();
        if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
            settings.look = look;
            settings.look_map = map;
        }
    }

    /// The layout the LOOK values make, for the sketch beside them.
    fn look_map(&self) -> LookMap {
        use crate::config::{FuelPlacement, ProcessPlacement, SidebarSide};
        let panel = self.config.right_panel;
        LookMap {
            sidebar_right: self.config.orcha.sidebar_side == SidebarSide::Right,
            sidebar: sidebar_width_index(self.config.sidebar_width),
            panel,
            process: match self.config.orcha.process {
                ProcessPlacement::Off => MapProcess::Off,
                ProcessPlacement::Panel if panel => MapProcess::Panel,
                _ => MapProcess::Sidebar,
            },
            fuel_in_status: self.config.orcha.fuel == FuelPlacement::StatusBar || !panel,
        }
    }

    fn selected_index_for_settings_section(&self, section: ClientSettingsSection) -> usize {
        match section {
            ClientSettingsSection::Theme => theme_index(&self.config.theme_name),
            ClientSettingsSection::Look => 0,
            ClientSettingsSection::Font => match self.overlay.as_ref() {
                Some(ClientShellOverlay::Settings(settings)) => settings
                    .appearance
                    .fonts
                    .iter()
                    .position(|font| settings.appearance.font.starts_with(font.as_str()))
                    .unwrap_or(0),
                _ => 0,
            },
            ClientSettingsSection::FontSize => match self.overlay.as_ref() {
                Some(ClientShellOverlay::Settings(settings)) => {
                    font_size_index(settings.appearance.font_size)
                }
                _ => 0,
            },
            ClientSettingsSection::Ligatures => match self.overlay.as_ref() {
                Some(ClientShellOverlay::Settings(settings)) => {
                    usize::from(!settings.appearance.ligatures)
                }
                _ => 0,
            },
            ClientSettingsSection::Ide | ClientSettingsSection::GitUi => {
                match self.overlay.as_ref() {
                    Some(ClientShellOverlay::Settings(settings)) => {
                        let (apps, now) = if section == ClientSettingsSection::Ide {
                            (&settings.appearance.ides, &self.config.orcha.ide)
                        } else {
                            (&settings.appearance.git_guis, &self.config.orcha.git_gui)
                        };
                        apps.iter()
                            .position(|(_, command)| command == now)
                            .unwrap_or(0)
                    }
                    _ => 0,
                }
            }
            ClientSettingsSection::Indicators => indicator_index(self.config.status_indicators),
            ClientSettingsSection::Sound => usize::from(!self.config.sound_enabled),
            ClientSettingsSection::Toast => toast_index(self.config.toast_delivery),
            ClientSettingsSection::Integrations => 0,
        }
    }

    pub(super) fn select_settings_section(
        &mut self,
        section: ClientSettingsSection,
        outcome: &mut ClientShellInput,
    ) {
        let selected = self.selected_index_for_settings_section(section);
        let request_integrations = matches!(section, ClientSettingsSection::Integrations)
            && matches!(
                self.overlay,
                Some(ClientShellOverlay::Settings(ClientSettingsOverlay {
                    loading_integrations: false,
                    installing_integrations: false,
                    ..
                }))
            );
        if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
            settings.section = section;
            settings.selected = selected;
        }
        if request_integrations {
            self.queue_integration_list(outcome, true);
        }
        outcome.repaint = true;
    }

    fn move_settings_section(&mut self, delta: isize, outcome: &mut ClientShellInput) {
        let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_ref() else {
            return;
        };
        let current = ClientSettingsSection::ALL
            .iter()
            .position(|section| *section == settings.section)
            .unwrap_or(0);
        let next = (current as isize + delta).rem_euclid(ClientSettingsSection::ALL.len() as isize)
            as usize;
        self.select_settings_section(ClientSettingsSection::ALL[next], outcome);
    }

    fn settings_choice_count(&self) -> usize {
        match self.overlay.as_ref() {
            Some(ClientShellOverlay::Settings(settings)) => match settings.section {
                ClientSettingsSection::Theme => crate::config::THEME_NAMES.len(),
                ClientSettingsSection::Look => LookRow::ALL.len(),
                // orcha sets the font in Ghostty only
                ClientSettingsSection::Font
                | ClientSettingsSection::FontSize
                | ClientSettingsSection::Ligatures
                    if !super::brutal::host_is_ghostty() =>
                {
                    0
                }
                ClientSettingsSection::Font => settings.appearance.fonts.len(),
                ClientSettingsSection::FontSize => FONT_SIZES.len(),
                ClientSettingsSection::Ligatures => 2,
                ClientSettingsSection::Ide => settings.appearance.ides.len(),
                ClientSettingsSection::GitUi => settings.appearance.git_guis.len(),
                ClientSettingsSection::Indicators | ClientSettingsSection::Sound => 2,
                ClientSettingsSection::Toast => 4,
                ClientSettingsSection::Integrations => settings.integrations.len(),
            },
            _ => 0,
        }
    }

    pub(super) fn move_settings_selection(&mut self, delta: isize) {
        let count = self.settings_choice_count();
        let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() else {
            return;
        };
        if count == 0 {
            settings.selected = 0;
            return;
        }
        settings.selected = (settings.selected as isize + delta)
            .clamp(0, count.saturating_sub(1) as isize) as usize;
        if settings.section == ClientSettingsSection::Theme {
            self.preview_selected_theme();
        }
    }

    pub(super) fn select_settings_choice(&mut self, index: usize) {
        let count = self.settings_choice_count();
        if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
            if count > 0 {
                settings.selected = index.min(count - 1);
            }
        }
        if matches!(
            self.overlay,
            Some(ClientShellOverlay::Settings(ClientSettingsOverlay {
                section: ClientSettingsSection::Theme,
                ..
            }))
        ) {
            self.preview_selected_theme();
        }
    }

    fn preview_selected_theme(&mut self) {
        let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_ref() else {
            return;
        };
        let Some(name) = crate::config::THEME_NAMES.get(settings.selected) else {
            return;
        };
        self.config.theme_name = (*name).to_owned();
        self.config.palette =
            crate::app::client_palette_for_theme(&self.config.theme_runtime, name);
    }

    pub(super) fn cancel_settings_overlay(&mut self) {
        let Some(ClientShellOverlay::Settings(settings)) = self.overlay.take() else {
            return;
        };
        self.config.theme_name = settings.original_theme_name;
        self.config.palette = settings.original_palette;
    }

    pub(super) fn save_settings_edit(
        &mut self,
        edit: crate::config::ConfigEdit<'_>,
        outcome: &mut ClientShellInput,
    ) -> bool {
        if let Err(error) = crate::config::write_edit(edit) {
            self.set_endpoint_error(error);
            outcome.repaint = true;
            return false;
        }
        self.reload_client_config();
        self.push_endpoint_method_with_kind(
            crate::api::schema::Method::ServerReloadConfig(
                crate::api::schema::EmptyParams::default(),
            ),
            PendingEndpointKind::ReloadConfig,
            outcome,
        );
        outcome.repaint = true;
        true
    }

    pub(super) fn apply_settings_choice(&mut self, outcome: &mut ClientShellInput) {
        let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_ref() else {
            return;
        };
        let section = settings.section;
        let selected = settings.selected;
        match section {
            ClientSettingsSection::Theme => {
                let Some(name) = crate::config::THEME_NAMES.get(selected).copied() else {
                    return;
                };
                if self.save_settings_edit(crate::config::ConfigEdit::Theme(name), outcome) {
                    self.overlay = None;
                }
            }
            ClientSettingsSection::Font => {
                let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() else {
                    return;
                };
                let Some(font) = settings.appearance.fonts.get(selected).cloned() else {
                    return;
                };
                settings.appearance.note = run_config_tool(vec!["font".into(), font.clone()]);
                settings.appearance.font = font;
                outcome.repaint = true;
            }
            ClientSettingsSection::FontSize => {
                let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() else {
                    return;
                };
                let Some(size) = FONT_SIZES.clone().nth(selected) else {
                    return;
                };
                // orcha-config sets the size along with the family
                if settings.appearance.font.is_empty() {
                    settings.appearance.note = Some("pick a font first".to_owned());
                } else {
                    settings.appearance.note = run_config_tool(vec![
                        "font".into(),
                        settings.appearance.font.clone(),
                        size.to_string(),
                    ]);
                    settings.appearance.font_size = size;
                }
                outcome.repaint = true;
            }
            ClientSettingsSection::Ligatures => {
                let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() else {
                    return;
                };
                let on = selected == 0;
                settings.appearance.note = run_config_tool(vec![
                    "ligatures".into(),
                    if on { "on" } else { "off" }.into(),
                ]);
                settings.appearance.ligatures = on;
                outcome.repaint = true;
            }
            ClientSettingsSection::Ide | ClientSettingsSection::GitUi => {
                let ide = section == ClientSettingsSection::Ide;
                let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() else {
                    return;
                };
                let apps = if ide {
                    &settings.appearance.ides
                } else {
                    &settings.appearance.git_guis
                };
                let Some((name, command)) = apps.get(selected).cloned() else {
                    return;
                };
                settings.appearance.note = Some(format!("opens with {name}"));
                self.save_open_app(ide, command);
                outcome.repaint = true;
            }
            ClientSettingsSection::Indicators => {
                let style = if selected == 0 {
                    crate::config::StatusIndicatorStyle::Dots
                } else {
                    crate::config::StatusIndicatorStyle::Symbols
                };
                self.save_settings_edit(
                    crate::config::ConfigEdit::StatusIndicators(style),
                    outcome,
                );
            }
            ClientSettingsSection::Sound => {
                self.save_settings_edit(crate::config::ConfigEdit::Sound(selected == 0), outcome);
            }
            ClientSettingsSection::Toast => {
                let delivery = match selected {
                    0 => crate::config::ToastDelivery::Off,
                    1 => crate::config::ToastDelivery::Herdr,
                    2 => crate::config::ToastDelivery::Terminal,
                    _ => crate::config::ToastDelivery::System,
                };
                self.save_settings_edit(
                    crate::config::ConfigEdit::ToastDelivery(delivery),
                    outcome,
                );
            }
            ClientSettingsSection::Integrations => self.install_recommended_integrations(outcome),
            ClientSettingsSection::Look => {
                if let Some(row) = LookRow::ALL.get(selected) {
                    self.cycle_look(*row, outcome);
                    self.refresh_look();
                }
            }
        }
    }

    /// The value a LOOK row shows.
    pub(super) fn look_value(&self, row: LookRow) -> String {
        match row {
            LookRow::Accent => ACCENTS[accent_index(&self.config.palette)].0.to_owned(),
            LookRow::Sidebar => {
                let (name, columns) =
                    SIDEBAR_WIDTHS[sidebar_width_index(self.config.sidebar_width)];
                format!("{name} · {columns} columns")
            }
            LookRow::RightPanel => if self.config.right_panel {
                "shown"
            } else {
                "hidden · the panes take its room"
            }
            .to_owned(),
            LookRow::Fuel => match self.config.orcha.fuel {
                crate::config::FuelPlacement::Panel => "right panel",
                crate::config::FuelPlacement::StatusBar => "status bar",
            }
            .to_owned(),
            LookRow::Panes => if self.config.orcha.pane_paper {
                "paper (the style's colors)"
            } else {
                "the terminal's colors"
            }
            .to_owned(),
            LookRow::Side => match self.config.orcha.sidebar_side {
                crate::config::SidebarSide::Left => "left · panel on the right",
                crate::config::SidebarSide::Right => "right · panel on the left",
            }
            .to_owned(),
            LookRow::Process => {
                let (sidebar, panel) = match self.config.orcha.sidebar_side {
                    crate::config::SidebarSide::Left => ("left", "right"),
                    crate::config::SidebarSide::Right => ("right", "left"),
                };
                match self.config.orcha.process {
                    crate::config::ProcessPlacement::Sidebar => {
                        format!("{sidebar} · under the projects")
                    }
                    crate::config::ProcessPlacement::Panel if !self.config.right_panel => {
                        format!("{panel} panel (hidden, so under the projects)")
                    }
                    crate::config::ProcessPlacement::Panel => format!("{panel} · in the panel"),
                    crate::config::ProcessPlacement::Off => "hidden".to_owned(),
                }
            }
            LookRow::Logos => {
                let mode = match self.config.orcha.icons {
                    crate::config::IconMode::Auto => "auto",
                    crate::config::IconMode::Font => "icon font",
                    crate::config::IconMode::Symbols => "plain symbols",
                };
                let shown = if self.config.icon_font {
                    "logos shown"
                } else {
                    "symbols shown"
                };
                format!("{mode} · {shown}")
            }
        }
    }

    /// Moves a LOOK row to its next value and saves it.
    fn cycle_look(&mut self, row: LookRow, outcome: &mut ClientShellInput) {
        use crate::config::ConfigEdit;
        match row {
            LookRow::Accent => {
                let next = (accent_index(&self.config.palette) + 1) % ACCENTS.len();
                self.save_settings_edit(ConfigEdit::Accent(ACCENTS[next].1), outcome);
            }
            LookRow::Sidebar => {
                let next =
                    (sidebar_width_index(self.config.sidebar_width) + 1) % SIDEBAR_WIDTHS.len();
                self.save_settings_edit(ConfigEdit::SidebarWidth(SIDEBAR_WIDTHS[next].1), outcome);
            }
            LookRow::RightPanel => {
                self.save_settings_edit(ConfigEdit::RightPanel(!self.config.right_panel), outcome);
            }
            LookRow::Fuel => {
                let next = match self.config.orcha.fuel {
                    crate::config::FuelPlacement::Panel => crate::config::FuelPlacement::StatusBar,
                    crate::config::FuelPlacement::StatusBar => crate::config::FuelPlacement::Panel,
                };
                self.save_settings_edit(ConfigEdit::Fuel(next), outcome);
            }
            LookRow::Panes => {
                self.save_settings_edit(
                    ConfigEdit::PanePaper(!self.config.orcha.pane_paper),
                    outcome,
                );
            }
            LookRow::Side => {
                let next = match self.config.orcha.sidebar_side {
                    crate::config::SidebarSide::Left => crate::config::SidebarSide::Right,
                    crate::config::SidebarSide::Right => crate::config::SidebarSide::Left,
                };
                self.save_settings_edit(ConfigEdit::SidebarSide(next), outcome);
                self.invalidate_pane_surface();
                outcome.resize = true;
            }
            LookRow::Process => {
                let next = match self.config.orcha.process {
                    crate::config::ProcessPlacement::Sidebar => {
                        crate::config::ProcessPlacement::Panel
                    }
                    crate::config::ProcessPlacement::Panel => crate::config::ProcessPlacement::Off,
                    crate::config::ProcessPlacement::Off => {
                        crate::config::ProcessPlacement::Sidebar
                    }
                };
                self.save_settings_edit(ConfigEdit::Process(next), outcome);
            }
            LookRow::Logos => {
                let next = match self.config.orcha.icons {
                    crate::config::IconMode::Auto => crate::config::IconMode::Font,
                    crate::config::IconMode::Font => crate::config::IconMode::Symbols,
                    crate::config::IconMode::Symbols => crate::config::IconMode::Auto,
                };
                self.save_settings_edit(ConfigEdit::Icons(next), outcome);
            }
        }
    }

    /// Keeps the IDE (`ide`) or Git GUI command, in memory and in the
    /// config's `[orcha]` table (written off the input path).
    pub(super) fn save_open_app(&mut self, ide: bool, command: String) {
        let key = if ide { "ide" } else { "git_gui" };
        if ide {
            self.config.orcha.ide = command.clone();
        } else {
            self.config.orcha.git_gui = command.clone();
        }
        let path = crate::config::config_path();
        let value = format!("\"{command}\"");
        let _ = crate::thread_spawn::spawn_named("orcha-app-pref", move || {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            let updated = crate::config::upsert_section_value(&content, "orcha", key, &value);
            if let Err(err) = std::fs::write(&path, updated) {
                tracing::warn!(err = %err, "orcha: failed to save the {key} setting");
            }
        });
    }

    fn queue_integration_list(&mut self, outcome: &mut ClientShellInput, clear_messages: bool) {
        if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
            settings.loading_integrations = true;
            if clear_messages {
                settings.integration_messages.clear();
            }
        }
        if !self.push_endpoint_method_with_kind(
            crate::api::schema::Method::IntegrationList(crate::api::schema::EmptyParams::default()),
            PendingEndpointKind::IntegrationList,
            outcome,
        ) {
            if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
                settings.loading_integrations = false;
            }
        }
    }

    fn install_recommended_integrations(&mut self, outcome: &mut ClientShellInput) {
        if self.pending_integration_installs > 0 {
            return;
        }
        let targets = match self.overlay.as_ref() {
            Some(ClientShellOverlay::Settings(settings)) => settings
                .integrations
                .iter()
                .filter(|integration| integration_needs_install(integration))
                .map(|integration| integration.target)
                .collect::<Vec<_>>(),
            _ => return,
        };
        if targets.is_empty() {
            return;
        }
        if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
            settings.installing_integrations = true;
            settings.integration_messages.clear();
        }
        self.pending_integration_installs = 0;
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
        if self.pending_integration_installs == 0 {
            if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
                settings.installing_integrations = false;
            }
        }
        outcome.repaint = true;
    }

    pub(super) fn handle_settings_endpoint_result(
        &mut self,
        kind: PendingEndpointKind,
        result: Result<crate::api::schema::ResponseResult, ClientShellEndpointError>,
    ) -> (bool, Vec<ClientShellAction>) {
        match kind {
            PendingEndpointKind::IntegrationList => {
                if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
                    settings.loading_integrations = false;
                    match result {
                        Ok(crate::api::schema::ResponseResult::IntegrationList {
                            integrations,
                        }) => {
                            settings.integrations = integrations;
                            settings.selected = settings
                                .selected
                                .min(settings.integrations.len().saturating_sub(1));
                        }
                        Ok(_) => {
                            self.set_endpoint_error(
                                "endpoint returned an unexpected integration list result",
                            );
                        }
                        Err(_) => {}
                    }
                } else if let Ok(crate::api::schema::ResponseResult::IntegrationList {
                    integrations,
                }) = result
                {
                    // orcha: the first-run setup asked
                    self.welcome_integrations_listed(integrations);
                }
                (true, Vec::new())
            }
            PendingEndpointKind::IntegrationInstall => {
                let cancelled = result
                    .as_ref()
                    .is_err_and(|error| error.code.as_deref() == Some("endpoint_cancelled"));
                self.pending_integration_installs =
                    self.pending_integration_installs.saturating_sub(1);
                if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
                    match result {
                        Ok(crate::api::schema::ResponseResult::IntegrationInstall {
                            details,
                            ..
                        }) => settings.integration_messages.extend(details.messages),
                        Ok(_) => settings
                            .integration_messages
                            .push("endpoint returned an unexpected integration result".into()),
                        Err(error) => settings.integration_messages.push(error.message),
                    }
                    settings.installing_integrations = self.pending_integration_installs > 0;
                } else if let Err(error) = result {
                    // orcha: installed from the first-run setup; only a failure says something
                    if !cancelled {
                        self.set_endpoint_error(error.message);
                    }
                }
                let actions = if !cancelled
                    && self.pending_integration_installs == 0
                    && matches!(self.overlay, Some(ClientShellOverlay::Settings(_)))
                {
                    let mut deferred = ClientShellInput::default();
                    self.queue_integration_list(&mut deferred, false);
                    deferred.actions
                } else {
                    Vec::new()
                };
                (true, actions)
            }
            _ => (false, Vec::new()),
        }
    }

    pub(super) fn route_settings_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> bool {
        if !matches!(self.overlay, Some(ClientShellOverlay::Settings(_))) {
            return false;
        }
        let (code, modifiers) = crate::config::normalize_key_combo((key.code, key.modifiers));
        if code == KeyCode::Esc {
            if !matches!(
                self.overlay,
                Some(ClientShellOverlay::Settings(ClientSettingsOverlay {
                    installing_integrations: true,
                    ..
                }))
            ) {
                self.cancel_settings_overlay();
                outcome.repaint = true;
            }
            return true;
        }
        if matches!(code, KeyCode::Tab | KeyCode::Right | KeyCode::Char('l'))
            && modifiers.is_empty()
        {
            self.move_settings_section(1, outcome);
            return true;
        }
        if matches!(code, KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h'))
            && modifiers.difference(KeyModifiers::SHIFT).is_empty()
        {
            self.move_settings_section(-1, outcome);
            return true;
        }
        if matches!(code, KeyCode::Up | KeyCode::Char('k')) && modifiers.is_empty() {
            self.move_settings_selection(-1);
            outcome.repaint = true;
            return true;
        }
        if matches!(code, KeyCode::Down | KeyCode::Char('j')) && modifiers.is_empty() {
            self.move_settings_selection(1);
            outcome.repaint = true;
            return true;
        }
        if matches!(code, KeyCode::Enter | KeyCode::Char(' ')) && modifiers.is_empty() {
            self.apply_settings_choice(outcome);
            return true;
        }
        true
    }
}
