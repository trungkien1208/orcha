//! Plugin, settings, office and update background jobs.

use super::*;

impl ClientShellState {
    /// Updates a GitHub plugin off the input path, then restarts its running
    /// panes so they run the new files. The click was the consent.
    pub(in crate::client::shell) fn start_plugin_update(&mut self, id: String, name: String) {
        if self.orcha_ui.plugin_update_run.is_some() {
            self.show_notice("a plugin update is already running".to_owned());
            return;
        }
        let working = format!("updating {name}…");
        let update_id = id.clone();
        self.orcha_ui.plugin_update_run = BackgroundJob::spawn("orcha-plugin-update", move || {
            let result = crate::orcha_plugin_updates::update_plugin(&update_id);
            if result.is_ok() {
                crate::orcha_plugin_updates::restart_running_panes(&update_id);
            }
            (name, result)
        });
        if self.orcha_ui.plugin_update_run.is_some() {
            self.orcha_ui
                .plugin_updates
                .retain(|update| update.id != id);
            self.start_working_notice("orcha-plugin-update", working);
        }
    }

    /// Removes a plugin off the input path; the popup's Remove was the consent.
    pub(super) fn start_plugin_remove(&mut self, id: String, name: String) {
        let working = format!("removing {name}…");
        self.orcha_ui.plugin_remove_run = BackgroundJob::spawn("orcha-plugin-remove", move || {
            let result = if id == crate::orcha_office::PLUGIN_ID {
                crate::orcha_office::remove_quietly()
            } else {
                crate::orcha_app::run_self_quietly(&["plugin", "uninstall", &id])
            };
            result
                .map(|()| format!("{name} removed"))
                .map_err(|message| format!("{name} was not removed: {message}"))
        });
        if self.orcha_ui.plugin_remove_run.is_some() {
            self.start_working_notice("orcha-plugin-remove", working);
            self.orcha_ui.office_checked_at = None;
        }
    }

    /// Opens every setting in the browser (starting its page when needed),
    /// off the input path.
    pub(in crate::client::shell) fn open_settings_web(&mut self) {
        self.open_settings_web_at(String::new());
    }

    /// The same page, scrolled to `section`.
    pub(in crate::client::shell) fn open_settings_web_at(&mut self, section: String) {
        if super::super::orcha_settings_web::over_ssh() {
            // the browser is on the other machine; the address holds a token, so not in a notice
            self.show_notice(
                "over SSH: run `orcha settings` in a pane for the address to forward".to_owned(),
            );
            return;
        }
        self.start_notice_job(
            "orcha-settings-web",
            Some("opening settings in the browser…".to_owned()),
            move || {
                super::super::orcha_settings_web::open_quietly(&section)
                    .map(|_| String::new())
                    .map_err(|message| {
                        tracing::warn!(%message, "the settings page did not open");
                        format!("Settings did not open: {message}")
                    })
            },
        );
    }

    /// Opens Easy Job in the browser (starting it when needed), off the input path.
    pub(in crate::client::shell) fn open_office(&mut self) {
        self.start_notice_job(
            "orcha-office-open",
            Some("opening Easy Job…".to_owned()),
            || {
                crate::orcha_office::open_quietly()
                    .map(|()| String::new())
                    .map_err(|message| {
                        tracing::warn!(%message, "Easy Job did not open");
                        format!("Easy Job did not open: {message}")
                    })
            },
        );
        // look again soon: it is starting
        self.orcha_ui.office_checked_at = None;
    }

    /// Looks, off the input path, whether Easy Job is added and running, for
    /// the header chip: every few seconds once it is added, else now and
    /// then (adding it from elsewhere shows within half a minute).
    pub(super) fn poll_office(&mut self) -> bool {
        const ADDED: std::time::Duration = std::time::Duration::from_secs(5);
        const NOT_ADDED: std::time::Duration = std::time::Duration::from_secs(30);
        let ui = &mut self.orcha_ui;
        let mut changed = false;
        if let Some(Ok(state)) = BackgroundJob::take_finished(&mut ui.office_check) {
            changed = ui.office != state;
            ui.office = state;
        }
        let every = if ui.office.is_some() {
            ADDED
        } else {
            NOT_ADDED
        };
        let due = ui.office_checked_at.is_none_or(|at| at.elapsed() >= every);
        if due && ui.office_check.is_none() {
            ui.office_checked_at = Some(std::time::Instant::now());
            ui.office_check =
                BackgroundJob::spawn("orcha-office-check", crate::orcha_office::office_state);
        }
        changed
    }

    /// Reads what each pane costs (memory, context) every few seconds, off
    /// the input path. Only for orcha's own server: another machine's
    /// process ids mean nothing here. True when the answer changed.
    pub(in crate::client::shell) fn poll_usage(&mut self) -> bool {
        const EVERY: std::time::Duration = std::time::Duration::from_secs(4);
        // tests never look at this machine's processes
        if cfg!(test) || self.active_endpoint_id != ClientEndpointId::Local {
            return false;
        }
        let ui = &mut self.orcha_ui;
        let mut changed = false;
        if let Some(Ok(usage)) = BackgroundJob::take_finished(&mut ui.usage_check) {
            changed = ui.usage != usage;
            ui.usage = usage;
        }
        let due = ui.usage_checked_at.is_none_or(|at| at.elapsed() >= EVERY);
        if due && ui.usage_check.is_none() {
            ui.usage_checked_at = Some(std::time::Instant::now());
            ui.usage_check = BackgroundJob::spawn("orcha-usage", crate::orcha_usage::read);
        }
        changed
    }

    /// Asks (at most hourly, off the input path) whether a newer orcha is out.
    pub(super) fn start_update_check(&mut self) {
        self.orcha_ui.update_checked_at = Some(std::time::Instant::now());
        self.orcha_ui.update_check =
            BackgroundJob::spawn("orcha-update-check", crate::orcha_app::newer_release);
        let automatic = self.config.orcha.update_plugins;
        self.orcha_ui.plugin_update_check =
            BackgroundJob::spawn("orcha-plugin-update-check", move || {
                crate::orcha_plugin_updates::news(automatic)
            });
    }

    /// The header's update button: shows a newer release, then the update
    /// running, then what to do next. Also says how the other background
    /// jobs went. Returns true when it changed.
    pub(in crate::client::shell) fn poll_update(&mut self) -> bool {
        let changed = self.poll_notice_jobs() | self.poll_default_accounts();
        if self.poll_office() {
            return true;
        }
        if let Some(result) = BackgroundJob::take_finished(&mut self.orcha_ui.plugin_remove_run) {
            self.end_working_notice("orcha-plugin-remove");
            self.orcha_ui.office_checked_at = None;
            match result.unwrap_or_else(|lost| Err(lost.to_string())) {
                Ok(message) => self.show_notice(message),
                Err(message) => self.show_failure(message),
            }
            return true;
        }
        if let Some(finished) = BackgroundJob::take_finished(&mut self.orcha_ui.plugin_add_run) {
            self.end_working_notice("orcha-plugin-add");
            match finished {
                Ok((_, name, Ok(()))) => self.show_notice(format!("{name} added")),
                Ok((id, _, Err(message))) => {
                    tracing::warn!(%message, "offered plugin was not added");
                    // ask again next time
                    crate::orcha_app::forget_plugin_offer(id);
                    self.show_failure(message);
                }
                Err(lost) => self.show_failure(format!("The plugin was not added: {lost}")),
            }
            return true;
        }
        if let Some(finished) = BackgroundJob::take_finished(&mut self.orcha_ui.plugin_update_run) {
            self.end_working_notice("orcha-plugin-update");
            match finished {
                Ok((name, Ok(version))) => {
                    self.show_notice(format!("{name} updated to {version}"));
                }
                Ok((name, Err(log))) => {
                    tracing::warn!(%log, "plugin update failed");
                    let last = log.lines().last().unwrap_or_default().to_owned();
                    self.show_failure(format!("{name} was not updated: {last}"));
                }
                Err(lost) => self.show_failure(format!("The plugin was not updated: {lost}")),
            }
            return true;
        }
        let ui = &mut self.orcha_ui;
        if let Some(Ok(news)) = BackgroundJob::take_finished(&mut ui.plugin_update_check) {
            ui.plugin_updates = news.updates;
            ui.plugin_update_notice = news.notice;
            if let Some(updated) = news.updated {
                self.show_notice(updated);
                return true;
            }
        }
        if let Some(Ok(news)) = BackgroundJob::take_finished(&mut ui.whats_new_check) {
            ui.whats_new = news;
        }
        if let Some(Ok(offer)) = BackgroundJob::take_finished(&mut ui.plugin_offer_check) {
            ui.plugin_offer = offer;
        }
        // after "what's new", once nothing else is on screen
        if ui.popup.is_none() && ui.whats_new.is_none() && ui.whats_new_check.is_none() {
            if let Some(plugin) = ui.plugin_offer.take() {
                ui.popup = Some(super::super::orcha_popup::OrchaPopup::PluginOffer {
                    selected: 0,
                    plugin,
                });
                return true;
            }
        }
        if ui.popup.is_none() && ui.whats_new.is_none() && ui.whats_new_check.is_none() {
            if let Some((update, name)) = ui.plugin_update_notice.take() {
                if update.installable() {
                    ui.popup = Some(super::super::orcha_popup::OrchaPopup::PluginUpdate {
                        selected: 0,
                        update,
                        name,
                    });
                } else {
                    let notice = update.notice(&name);
                    self.show_notice(notice);
                }
                return true;
            }
        }
        // shown once nothing else is on screen
        if ui.popup.is_none() {
            if let Some(release) = ui.whats_new.take() {
                ui.popup = Some(super::super::orcha_popup::OrchaPopup::WhatsNew { release });
                return true;
            }
        }
        if let Some(Ok(Some(release))) = BackgroundJob::take_finished(&mut ui.update_check) {
            ui.update_note = Some(format!("⬆ update to {}", release.version));
            ui.update_available = Some(release.version.clone());
            // ask once per version, when nothing else is on screen
            if ui.popup.is_none() && crate::orcha_app::first_offer(&release.version) {
                ui.popup = Some(super::super::orcha_popup::OrchaPopup::Update {
                    selected: 0,
                    release,
                });
            }
            return true;
        }
        if let Some(finished) = BackgroundJob::take_finished(&mut ui.update_run) {
            ui.update_note = Some(if finished == Ok(true) {
                "✓ updated: quit and reopen".to_owned()
            } else {
                "update failed: run orcha update".to_owned()
            });
            return true;
        }
        // an app left open asks again: a release can come out after it started
        let due = ui
            .update_checked_at
            .is_some_and(|at| at.elapsed() >= crate::orcha_app::RELEASE_CHECK_EVERY);
        if due
            && ui.update_check.is_none()
            && ui.update_run.is_none()
            && ui.update_available.is_none()
        {
            ui.update_checked_at = Some(std::time::Instant::now());
            ui.update_check =
                BackgroundJob::spawn("orcha-update-check", crate::orcha_app::newer_release);
        }
        changed
    }

    /// Click on the update button (or the menu's "update orcha"): installs
    /// the newer release behind the UI.
    pub(in crate::client::shell) fn start_update(&mut self) {
        if self.orcha_ui.update_run.is_some() || self.orcha_ui.update_available.is_none() {
            return;
        }
        self.orcha_ui.update_run =
            BackgroundJob::spawn("orcha-update", || crate::orcha_app::run_installer(true));
        if self.orcha_ui.update_run.is_some() {
            self.orcha_ui.update_available = None;
            // installed (or failed) once: no more reminders until reopened
            self.orcha_ui.update_checked_at = None;
            self.orcha_ui.update_note = Some("⬆ updating…".to_owned());
        }
    }
}
