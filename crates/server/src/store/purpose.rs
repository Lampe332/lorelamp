//! What the user wants to do with a model (`rp_core::purpose::Purpose`): one sentence and the
//! needs read from it or corrected by hand, kept in the settings table (`use_purpose`). The tab
//! "Vorschläge" and "Beste Einstellung finden" share it.

use rp_core::purpose::Purpose;

use super::{Res, Store};

const KEY: &str = "use_purpose";

impl Store {
    /// The stored purpose; the empty one when none was given (or the stored text does not parse).
    pub fn use_purpose(&self) -> Res<Purpose> {
        Ok(self.setting(KEY)?.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default())
    }

    pub fn set_use_purpose(&mut self, purpose: &Purpose) -> Res<()> {
        self.set_setting(KEY, &serde_json::to_string(purpose).unwrap_or_else(|_| "{}".into()))
    }
}
