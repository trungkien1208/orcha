#[derive(Clone, Copy)]
pub(crate) enum ConfigEdit<'a> {
    Theme(&'a str),
    StatusIndicators(super::StatusIndicatorStyle),
    Sound(bool),
    ToastDelivery(super::ToastDelivery),
    /// orcha: the brand color (`[theme.custom] mauve`); None is the theme's.
    Accent(Option<&'a str>),
    SidebarWidth(u16),
    RightPanel(bool),
    /// orcha: where the FUEL gauges live.
    Fuel(super::FuelPlacement),
    /// orcha: paint default-colored pane cells with the style's paper.
    PanePaper(bool),
    /// orcha: install GitHub plugin updates without asking.
    UpdatePlugins(bool),
    /// orcha: agent logos from the icon font or plain symbols.
    Icons(super::IconMode),
    /// orcha: where PROCESS sits.
    Process(super::ProcessPlacement),
    /// orcha: the project sidebar's side.
    SidebarSide(super::SidebarSide),
    /// Follow the terminal's light or dark appearance (`[theme] auto_switch`).
    ThemeAutoSwitch(bool),
    /// Ask before closing a workspace (`[ui] confirm_close`).
    ConfirmClose(bool),
    /// orcha: how Ctrl+clicked files of one extension open
    /// (`[orcha.open_with]`); None forgets the choice, so orcha asks again.
    OpenWith(&'a str, Option<&'a str>),
}

impl ConfigEdit<'_> {
    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::Theme(_) => "theme",
            Self::StatusIndicators(_) => "status indicators",
            Self::Sound(_) => "sound setting",
            Self::ToastDelivery(_) => "toast setting",
            Self::Accent(_) => "accent color",
            Self::SidebarWidth(_) => "sidebar width",
            Self::RightPanel(_) => "right panel setting",
            Self::Fuel(_) => "fuel placement",
            Self::PanePaper(_) => "pane colors",
            Self::UpdatePlugins(_) => "plugin update setting",
            Self::Icons(_) => "logo setting",
            Self::Process(_) => "process placement",
            Self::SidebarSide(_) => "sidebar side",
            Self::ThemeAutoSwitch(_) => "light and dark setting",
            Self::ConfirmClose(_) => "close confirmation",
            Self::OpenWith(..) => "open-with choice",
        }
    }

    pub(crate) fn apply(self, content: &str) -> String {
        match self {
            Self::Theme(name) => {
                let content =
                    super::upsert_section_value(content, "theme", "name", &format!("\"{name}\""));
                super::upsert_section_bool(&content, "theme", "auto_switch", false)
            }
            Self::StatusIndicators(style) => super::upsert_section_value(
                content,
                "ui",
                "status_indicators",
                &format!("\"{}\"", style.as_str()),
            ),
            Self::Sound(enabled) => {
                super::upsert_section_bool(content, "ui.sound", "enabled", enabled)
            }
            Self::ToastDelivery(delivery) => {
                let value = match delivery {
                    super::ToastDelivery::Off => "\"off\"",
                    super::ToastDelivery::Herdr => "\"herdr\"",
                    super::ToastDelivery::Terminal => "\"terminal\"",
                    super::ToastDelivery::System => "\"system\"",
                };
                let content = super::upsert_section_value(content, "ui.toast", "delivery", value);
                super::remove_section_key(&content, "ui.toast", "enabled")
            }
            Self::Accent(Some(color)) => super::upsert_section_value(
                content,
                "theme.custom",
                "mauve",
                &format!("\"{color}\""),
            ),
            Self::Accent(None) => super::remove_section_key(content, "theme.custom", "mauve"),
            Self::SidebarWidth(width) => {
                super::upsert_section_value(content, "ui", "sidebar_width", &width.to_string())
            }
            Self::RightPanel(on) => super::upsert_section_bool(content, "ui", "right_panel", on),
            Self::Fuel(placement) => {
                let value = match placement {
                    super::FuelPlacement::Panel => "\"panel\"",
                    super::FuelPlacement::StatusBar => "\"status_bar\"",
                };
                super::upsert_section_value(content, "orcha", "fuel", value)
            }
            Self::PanePaper(on) => super::upsert_section_bool(content, "orcha", "pane_paper", on),
            Self::UpdatePlugins(on) => {
                super::upsert_section_bool(content, "orcha", "update_plugins", on)
            }
            Self::Icons(mode) => {
                let value = match mode {
                    super::IconMode::Auto => "\"auto\"",
                    super::IconMode::Font => "\"font\"",
                    super::IconMode::Symbols => "\"symbols\"",
                };
                super::upsert_section_value(content, "orcha", "icons", value)
            }
            Self::Process(placement) => {
                let value = match placement {
                    super::ProcessPlacement::Panel => "\"panel\"",
                    super::ProcessPlacement::Sidebar => "\"sidebar\"",
                    super::ProcessPlacement::Off => "\"off\"",
                };
                super::upsert_section_value(content, "orcha", "process", value)
            }
            Self::SidebarSide(side) => {
                let value = match side {
                    super::SidebarSide::Left => "\"left\"",
                    super::SidebarSide::Right => "\"right\"",
                };
                super::upsert_section_value(content, "orcha", "sidebar_side", value)
            }
            Self::ThemeAutoSwitch(on) => {
                super::upsert_section_bool(content, "theme", "auto_switch", on)
            }
            Self::ConfirmClose(on) => {
                super::upsert_section_bool(content, "ui", "confirm_close", on)
            }
            Self::OpenWith(extension, Some(choice)) => super::upsert_section_value(
                content,
                "orcha.open_with",
                extension,
                &format!("\"{}\"", choice.replace('\\', "\\\\").replace('"', "\\\"")),
            ),
            Self::OpenWith(extension, None) => {
                super::remove_section_key(content, "orcha.open_with", extension)
            }
        }
    }
}

pub(crate) fn update_file_at(
    path: &std::path::Path,
    description: &str,
    update: impl FnOnce(&str) -> Result<String, String>,
) -> Result<(), String> {
    let _lock = crate::orcha_app::lock_for_update(path)
        .map_err(|error| format!("failed to lock config before saving {description}: {error}"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create config directory: {error}"))?;
    }
    let content = match super::io::read_optional_config(path) {
        Ok(Some(content)) => content,
        Ok(None) => String::new(),
        Err(error) => {
            return Err(format!(
                "failed to read config before saving {description}: {error}"
            ));
        }
    };
    let updated = update(&content)?;
    crate::orcha_app::write_atomic(path, &updated)
        .map_err(|error| format!("failed to save {description}: {error}"))
}

pub(crate) fn write_edit(edit: ConfigEdit<'_>) -> Result<(), String> {
    update_file_at(&super::config_path(), edit.description(), |content| {
        Ok(edit.apply(content))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_config_edits_keep_each_writers_change() {
        let dir =
            std::env::temp_dir().join(format!("orcha-config-concurrent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let _ = std::fs::remove_file(&path);
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for index in 0..8 {
                let path = &path;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    update_file_at(path, "test setting", |current| {
                        Ok(format!("{current}key{index} = true\n"))
                    })
                    .expect("concurrent edit");
                });
            }
        });
        let text = std::fs::read_to_string(&path).unwrap();
        let parsed: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(parsed.as_table().unwrap().len(), 8);
        for index in 0..8 {
            assert_eq!(parsed[format!("key{index}")].as_bool(), Some(true));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn look_edits_write_their_keys_and_the_theme_accent_removes_the_override() {
        let content = ConfigEdit::Accent(Some("#6c8cff")).apply("");
        let content = ConfigEdit::SidebarWidth(40).apply(&content);
        let content = ConfigEdit::RightPanel(false).apply(&content);
        let content = ConfigEdit::Fuel(crate::config::FuelPlacement::StatusBar).apply(&content);
        let content = ConfigEdit::PanePaper(false).apply(&content);
        let content = ConfigEdit::UpdatePlugins(true).apply(&content);
        let value: toml::Value = toml::from_str(&content).expect("valid TOML");
        assert_eq!(value["theme"]["custom"]["mauve"].as_str(), Some("#6c8cff"));
        assert_eq!(value["ui"]["sidebar_width"].as_integer(), Some(40));
        assert_eq!(value["ui"]["right_panel"].as_bool(), Some(false));
        assert_eq!(value["orcha"]["fuel"].as_str(), Some("status_bar"));
        assert_eq!(value["orcha"]["pane_paper"].as_bool(), Some(false));
        assert_eq!(value["orcha"]["update_plugins"].as_bool(), Some(true));
        let reset = ConfigEdit::Accent(None).apply(&content);
        assert!(!reset.contains("mauve"));
    }

    #[test]
    fn update_file_at_does_not_move_a_leading_bom_into_the_file() {
        let dir = std::env::temp_dir().join(format!("herdr-config-bom-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            b"\xEF\xBB\xBF[terminal]\ndefault_shell = \"pwsh.exe\"\n",
        )
        .unwrap();

        update_file_at(&path, "onboarding setting", |content| {
            Ok(crate::config::upsert_top_level_bool(
                content,
                "onboarding",
                false,
            ))
        })
        .unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);

        assert!(
            !written.contains('\u{feff}'),
            "unexpected BOM in {written:?}"
        );
        assert!(
            toml::from_str::<toml::Value>(&written).is_ok(),
            "written config is not valid TOML: {written:?}"
        );
    }

    #[test]
    fn open_with_choices_are_written_and_forgotten() {
        let content = ConfigEdit::OpenWith("png", Some("folder")).apply("");
        assert!(content.contains("[orcha.open_with]"), "{content}");
        assert!(content.contains("png = \"folder\""), "{content}");
        let content = ConfigEdit::OpenWith("md", Some("editor:zed \"{path}\"")).apply(&content);
        assert!(
            content.contains(r#"md = "editor:zed \"{path}\"""#),
            "{content}"
        );
        let content = ConfigEdit::OpenWith("png", None).apply(&content);
        assert!(!content.contains("png ="), "{content}");
        let content = ConfigEdit::ThemeAutoSwitch(true).apply(&content);
        assert!(content.contains("auto_switch = true"), "{content}");
        let content = ConfigEdit::ConfirmClose(false).apply(&content);
        assert!(content.contains("confirm_close = false"), "{content}");
    }
}
