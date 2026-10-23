use super::*;
use crossterm::event::{KeyCode, KeyModifiers};

/// The built-in name a theme goes by (old names like "brutal" included).
pub(super) fn normalized_theme_name(name: &str) -> String {
    let normalized = name.to_lowercase().replace([' ', '_'], "-");
    crate::config::canonical_theme_name(&normalized)
        .map(str::to_owned)
        .unwrap_or(normalized)
}

fn theme_index(name: &str) -> usize {
    let normalized = normalized_theme_name(name);
    crate::config::THEME_NAMES
        .iter()
        .position(|candidate| normalized_theme_name(candidate) == normalized)
        .unwrap_or(0)
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
    let config: serde_json::Value = std::fs::read_to_string(crate::config::config_dir().join("config.json"))
        .ok()
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .and_then(|home| std::fs::read_to_string(format!("{home}/.config/orcha/config.json")).ok())
        })
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    OrchaAppearance {
        ides: installed_apps(IDE_APPS),
        git_guis: installed_apps(GIT_GUI_APPS),
        fonts,
        font: config["font"]["family"].as_str().unwrap_or_default().to_owned(),
        ligatures: config["font"]["ligatures"].as_bool().unwrap_or(true),
        note: None,
    }
}

/// IDEs orcha knows how to open a folder in (macOS app names).
const IDE_APPS: &[&str] = &[
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
const GIT_GUI_APPS: &[&str] = &[
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
fn installed_apps(names: &[&str]) -> Vec<(String, String)> {
    let mut out = vec![("auto: the first one found".to_owned(), String::new())];
    if cfg!(test) {
        return out;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    for name in names {
        let found = [format!("/Applications/{name}.app"), format!("{home}/Applications/{name}.app")]
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
        let _ = std::process::Command::new(tool)
            .args(args)
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
        }));
    }

    fn selected_index_for_settings_section(&self, section: ClientSettingsSection) -> usize {
        match section {
            ClientSettingsSection::Theme => theme_index(&self.config.theme_name),
            ClientSettingsSection::Font => match self.overlay.as_ref() {
                Some(ClientShellOverlay::Settings(settings)) => settings
                    .appearance
                    .fonts
                    .iter()
                    .position(|font| settings.appearance.font.starts_with(font.as_str()))
                    .unwrap_or(0),
                _ => 0,
            },
            ClientSettingsSection::Ligatures => match self.overlay.as_ref() {
                Some(ClientShellOverlay::Settings(settings)) => usize::from(!settings.appearance.ligatures),
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
                        apps.iter().position(|(_, command)| command == now).unwrap_or(0)
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
                ClientSettingsSection::Font => settings.appearance.fonts.len(),
                // orcha sets the font in Ghostty only
                ClientSettingsSection::Font | ClientSettingsSection::Ligatures
                    if !super::brutal::host_is_ghostty() =>
                {
                    0
                }
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

    fn save_settings_edit(
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
        }
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
