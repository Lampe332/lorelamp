//! Settings the model downloads read: the app's model folder (where downloads go, setting
//! `models_app_folder`, chosen in the tab "Modell") and the model files the profiles use (so the
//! catalog can tell which models are already there), and the last PC scan for the fit pill.

use super::{Res, Store};

/// Setting key of the app's model folder; empty or missing = `<data>/models`.
pub const MODELS_APP_FOLDER: &str = "models_app_folder";

impl Store {
    /// The model folder the user chose, if any.
    pub fn download_folder(&self) -> Res<Option<String>> {
        Ok(self.setting(MODELS_APP_FOLDER)?.filter(|v| !v.trim().is_empty()))
    }

    /// The last PC scan (`POST /system/scan`, JSON), if one ran.
    pub fn last_pc_scan_json(&self) -> Res<Option<String>> {
        self.setting("system_scan")
    }

    /// Model files of all profiles (paths as stored).
    pub fn profile_model_files(&self) -> Res<Vec<String>> {
        Ok(self.profiles()?.into_iter().map(|p| p.model_file).collect())
    }
}
