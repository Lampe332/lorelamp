//! App settings (table `settings`, one value per key): theme, start layout, the backups folder's
//! limit, and the user's name, which is the name of the active persona.

use rusqlite::{params, OptionalExtension};

use super::{PersonaEdit, Res, Store};

impl Store {
    pub(super) fn setting(&self, key: &str) -> Res<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get(0)).optional()?)
    }

    pub(super) fn set_setting(&mut self, key: &str, value: &str) -> Res<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![key, value],
        )?;
        Ok(())
    }

    /// `{{user}}`: the name of the active persona.
    pub fn user_name(&self) -> Res<String> {
        Ok(self.active_persona()?.name)
    }

    /// Renames the active persona (the single-name setting the UI had before personas).
    pub fn set_user_name(&mut self, name: &str) -> Res<i64> {
        let id = self.active_persona_id()?;
        self.update_persona(id, &PersonaEdit::name(name))?;
        Ok(id)
    }

    /// The colour theme of the UI (an id the web UI knows); `None` = never chosen.
    pub fn theme(&self) -> Res<Option<String>> {
        self.setting("theme")
    }

    pub fn set_theme(&mut self, theme: &str) -> Res<()> {
        self.set_setting("theme", theme)
    }

    /// The app's language (`de`, `en`, `es`, `ru`): the UI's texts and the language the figures
    /// answer in. German until one is chosen (also for a value this version does not know).
    pub fn language(&self) -> Res<rp_core::prompt::Language> {
        Ok(self.setting("language")?.and_then(|l| rp_core::prompt::Language::parse(&l)).unwrap_or_default())
    }

    pub fn set_language(&mut self, language: rp_core::prompt::Language) -> Res<()> {
        self.set_setting("language", language.as_str())
    }

    /// The layout of the start screen (an id the web UI knows); `None` = never chosen.
    pub fn home_layout(&self) -> Res<Option<String>> {
        self.setting("home_layout")
    }

    pub fn set_home_layout(&mut self, layout: &str) -> Res<()> {
        self.set_setting("home_layout", layout)
    }

    /// The layout of the characters tab (`poster`, `compact`, `list`); `None` = never chosen.
    pub fn library_layout(&self) -> Res<Option<String>> {
        self.setting("library_layout")
    }

    pub fn set_library_layout(&mut self, layout: &str) -> Res<()> {
        self.set_setting("library_layout", layout)
    }

    /// Unload the model after this many quiet minutes; 0 = never (the default).
    pub fn idle_unload_min(&self) -> Res<u32> {
        Ok(self.setting("idle_unload_min")?.and_then(|v| v.parse().ok()).unwrap_or(0))
    }

    pub fn set_idle_unload_min(&mut self, minutes: u32) -> Res<()> {
        self.set_setting("idle_unload_min", &minutes.to_string())
    }

    /// The limit of the backups folder in bytes (one of `backup::LIMIT_CHOICES`; default 1 GB).
    pub fn backup_limit(&self) -> u64 {
        crate::backup::limit_in(&self.conn)
    }

    /// Err `bad_limit` for a value that is not one of `backup::LIMIT_CHOICES` (nothing saved).
    pub fn set_backup_limit(&mut self, bytes: u64) -> Res<()> {
        if !crate::backup::LIMIT_CHOICES.contains(&bytes) {
            return Err("bad_limit".into());
        }
        self.set_setting(crate::backup::LIMIT_KEY, &bytes.to_string())
    }

    /// The port of the managed model server (read with `engine_port`; the supervisor moves the
    /// model server there at its next start, never while a reply is written).
    pub fn set_engine_port(&mut self, port: u16) -> Res<()> {
        self.set_setting("engine_port", &port.to_string())
    }

    /// The model folders the user chose: (the app's own, None = default; further ones).
    pub fn model_folders(&self) -> Res<(Option<String>, Vec<String>)> {
        let app = self.setting("models_app_folder")?.filter(|s| !s.is_empty());
        let extra = self.setting("models_extra_folders")?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Ok((app, extra))
    }

    /// The switch "Handy verbinden": phones in the home network may reach the app (default off).
    pub fn lan_enabled(&self) -> Res<bool> {
        Ok(self.setting("lan_enabled")?.as_deref() == Some("1"))
    }

    pub fn set_lan_enabled(&mut self, on: bool) -> Res<()> {
        self.set_setting("lan_enabled", if on { "1" } else { "0" })
    }

    /// The port phones use (kept, so a QR code stays valid after a restart); None before the first use.
    pub fn lan_port(&self) -> Res<Option<u16>> {
        Ok(self.setting("lan_port")?.and_then(|v| v.parse().ok()))
    }

    pub fn set_lan_port(&mut self, port: u16) -> Res<()> {
        self.set_setting("lan_port", &port.to_string())
    }

    /// The token phones use (not the desktop window's, which changes at every start).
    pub fn lan_token(&self) -> Res<Option<String>> {
        Ok(self.setting("lan_token")?.filter(|t| t.len() >= 32))
    }

    pub fn set_lan_token(&mut self, token: &str) -> Res<()> {
        self.set_setting("lan_token", token)
    }

    /// The address the user picked for the QR code when the PC has several; None = the default one.
    pub fn lan_ip(&self) -> Res<Option<String>> {
        Ok(self.setting("lan_ip")?.filter(|s| !s.is_empty()))
    }

    pub fn set_lan_ip(&mut self, ip: &str) -> Res<()> {
        self.set_setting("lan_ip", ip)
    }

    pub fn set_model_folders(&mut self, app: Option<&str>, extra: &[String]) -> Res<()> {
        self.set_setting("models_app_folder", app.unwrap_or_default())?;
        self.set_setting("models_extra_folders", &serde_json::to_string(extra)?)
    }
}
