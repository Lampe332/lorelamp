//! Engines the app installed itself (official llama.cpp releases), their self-test results and
//! the last PC scan. The self-test and the scan are JSON values in `settings` (no migration).

use rusqlite::params;

use super::{now, Res, Store};

/// Key of the last PC scan in `settings`.
const SCAN_KEY: &str = "system_scan";

fn selftest_key(engine_id: i64) -> String {
    format!("engine_selftest_{engine_id}")
}

/// A new official engine (a release the app downloaded and tested).
pub struct NewEngine<'a> {
    pub name: &'a str,
    pub exe: &'a str,
    pub extra_path: &'a [String],
    pub backend: &'a str,
    pub version: Option<&'a str>,
}

#[derive(Debug, PartialEq)]
pub enum EngineDelete {
    /// Removed; the program file it pointed at (the caller removes its folder).
    Done(String),
    Missing,
    /// Only engines the app installed can go; own builds stay.
    Own,
    /// A model profile uses it.
    InUse,
}

impl Store {
    pub fn add_official_engine(&mut self, e: &NewEngine, selftest: &serde_json::Value) -> Res<i64> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO engines (name, exe, extra_path, backend, version, origin, created_at) VALUES (?1, ?2, ?3, ?4, ?5, 'official', ?6)",
            params![e.name, e.exe, serde_json::to_string(e.extra_path)?, e.backend, e.version, now()],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![selftest_key(id), selftest.to_string()],
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// The stored self-test of an engine (only engines the app installed have one).
    pub fn engine_selftest(&self, engine_id: i64) -> Res<Option<serde_json::Value>> {
        Ok(self.setting(&selftest_key(engine_id))?.and_then(|v| serde_json::from_str(&v).ok()))
    }

    pub fn delete_engine(&mut self, id: i64) -> Res<EngineDelete> {
        let Some(e) = self.engine(id)? else { return Ok(EngineDelete::Missing) };
        if e.origin != "official" {
            return Ok(EngineDelete::Own);
        }
        let used: i64 = self.conn.query_row("SELECT COUNT(*) FROM model_profiles WHERE engine_id = ?1", params![id], |r| r.get(0))?;
        if used > 0 {
            return Ok(EngineDelete::InUse);
        }
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM engines WHERE id = ?1", params![id])?;
        tx.execute("DELETE FROM settings WHERE key = ?1", params![selftest_key(id)])?;
        tx.commit()?;
        Ok(EngineDelete::Done(e.exe))
    }

    pub fn system_scan(&self) -> Res<Option<serde_json::Value>> {
        Ok(self.setting(SCAN_KEY)?.and_then(|v| serde_json::from_str(&v).ok()))
    }

    pub fn set_system_scan(&mut self, scan: &serde_json::Value) -> Res<()> {
        self.set_setting(SCAN_KEY, &scan.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::temp_store;
    use crate::store::ProfileEdit;
    use serde_json::json;

    #[test]
    fn official_engines_come_and_go_own_ones_and_used_ones_stay() {
        let (mut s, dir) = temp_store();
        let test = json!({ "ok": true, "devices": [] });
        let id = s
            .add_official_engine(&NewEngine { name: "llama.cpp b1", exe: "C:\\e\\llama-server.exe", extra_path: &[], backend: "cpu", version: Some("b1") }, &test)
            .unwrap();
        let e = s.engine(id).unwrap().unwrap();
        assert_eq!((e.origin.as_str(), e.backend.as_str(), e.version.as_deref()), ("official", "cpu", Some("b1")));
        assert_eq!(s.engine_selftest(id).unwrap(), Some(test));

        // In use by a profile: refused.
        let file = std::env::temp_dir().join("x.gguf").display().to_string();
        let p = s
            .add_profile(&ProfileEdit { name: Some("p".into()), engine_id: Some(id), model_file: Some(file), ..ProfileEdit::default() })
            .unwrap();
        assert_eq!(s.delete_engine(id).unwrap(), EngineDelete::InUse);
        s.delete_profile(p.id).unwrap();
        assert_eq!(s.delete_engine(id).unwrap(), EngineDelete::Done("C:\\e\\llama-server.exe".into()));
        assert_eq!(s.engine_selftest(id).unwrap(), None);
        assert_eq!(s.delete_engine(id).unwrap(), EngineDelete::Missing);

        // An own build is never removed.
        s.conn
            .execute("INSERT INTO engines (name, exe, extra_path, backend, origin, created_at) VALUES ('own', 'x', '[]', 'rocm', 'own', 0)", [])
            .unwrap();
        let own = s.conn.last_insert_rowid();
        assert_eq!(s.delete_engine(own).unwrap(), EngineDelete::Own);
        drop(s);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn scan_is_kept() {
        let (mut s, dir) = temp_store();
        assert_eq!(s.system_scan().unwrap(), None);
        s.set_system_scan(&json!({ "scanned_at": 5 })).unwrap();
        assert_eq!(s.system_scan().unwrap(), Some(json!({ "scanned_at": 5 })));
        drop(s);
        let _ = std::fs::remove_dir_all(dir);
    }
}
