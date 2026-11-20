//! The general app settings (tab "Allgemein"): plain values in the table `settings`, one per key.
//! What the keys are and which values they take is decided in `routes/general.rs`.

use super::{Res, Store};

impl Store {
    /// The stored value of a general setting; `None` = never chosen.
    pub fn general(&self, key: &str) -> Res<Option<String>> {
        self.setting(key)
    }

    pub fn set_general(&mut self, key: &str, value: &str) -> Res<()> {
        self.set_setting(key, value)
    }
}
