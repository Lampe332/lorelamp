//! Engines (llama-server builds) and model profiles (model file + load arguments + sampler).
//! Exactly one profile is active at a time (unique partial index); the supervisor runs it. The
//! port and "stop on exit" are app settings, the same for every profile.

use rp_core::sampler::Sampler;
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::{now, Res, Store};
use crate::engine::{detect_backend, EngineConfig, LaunchSpec, DEFAULT_ENGINE_PORT};

/// Name of the engine taken over from `engine.json` (UI text).
const OWN_BUILD: &str = "Eigener Build";
/// Longest profile name (characters).
pub const PROFILE_NAME_MAX: usize = 60;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EngineRow {
    pub id: i64,
    pub name: String,
    /// The llama-server program.
    pub exe: String,
    /// Folders put in front of PATH (GPU runtime).
    pub extra_path: Vec<String>,
    /// `rocm`, `vulkan`, `cuda`, `cpu`, … or `unknown`.
    pub backend: String,
    /// Build (`b12387`), once known.
    pub version: Option<String>,
    /// `own` (a build of the user) or `official` (a llama.cpp release the app installed).
    pub origin: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Profile {
    pub id: i64,
    pub name: String,
    pub engine_id: i64,
    pub model_file: String,
    /// llama-server arguments without `-m`, `--host` and `--port`.
    pub load_args: Vec<String>,
    pub sampler: Sampler,
    /// `off` or `on` (thinking of reasoning models).
    pub reasoning: String,
    /// The load arguments come from a measurement (benchmark), not from a guess.
    pub measured: bool,
    pub active: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A new profile or a change to one; fields left out stay as they are (for a new one: name,
/// engine and model file are required, the rest has defaults).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProfileEdit {
    pub name: Option<String>,
    pub engine_id: Option<i64>,
    pub model_file: Option<String>,
    pub load_args: Option<Vec<String>>,
    pub sampler: Option<Sampler>,
    pub reasoning: Option<String>,
    pub measured: Option<bool>,
}

impl Profile {
    /// What a benchmark result of this profile was measured with (`measured_with` in the result):
    /// the model file, the engine and the profile itself (its number can come back after a delete).
    pub fn bench_stamp(&self) -> serde_json::Value {
        serde_json::json!({ "model_file": self.model_file, "engine_id": self.engine_id, "profile_created_at": self.created_at })
    }
}

impl ProfileEdit {
    /// Changes what the server is started with (and so needs a restart when active).
    pub fn changes_launch(&self) -> bool {
        self.engine_id.is_some() || self.model_file.is_some() || self.load_args.is_some()
    }
}

#[derive(Debug, PartialEq)]
pub enum ProfileDelete {
    Done,
    Missing,
    /// The active profile cannot go (the model runs from it).
    Active,
}

const PROFILE_COLS: &str = "id, name, engine_id, model_file, load_args, sampler, reasoning, measured, active, created_at, updated_at";

fn profile_row(r: &Row) -> rusqlite::Result<Profile> {
    let load_args: String = r.get(4)?;
    let sampler: String = r.get(5)?;
    Ok(Profile {
        id: r.get(0)?,
        name: r.get(1)?,
        engine_id: r.get(2)?,
        model_file: r.get(3)?,
        load_args: serde_json::from_str(&load_args).unwrap_or_default(),
        sampler: serde_json::from_str(&sampler).unwrap_or_default(),
        reasoning: r.get(6)?,
        measured: r.get(7)?,
        active: r.get(8)?,
        created_at: r.get(9)?,
        updated_at: r.get(10)?,
    })
}

/// The result was measured with what the profile starts now (or does not say: stored before
/// results named it).
fn measured_with(result: &serde_json::Value, p: &Profile) -> bool {
    result.get("measured_with").is_none_or(|m| *m == p.bench_stamp())
}

fn engine_row(r: &Row) -> rusqlite::Result<EngineRow> {
    let extra: String = r.get(3)?;
    Ok(EngineRow {
        id: r.get(0)?,
        name: r.get(1)?,
        exe: r.get(2)?,
        extra_path: serde_json::from_str(&extra).unwrap_or_default(),
        backend: r.get(4)?,
        version: r.get(5)?,
        origin: r.get(6)?,
        created_at: r.get(7)?,
    })
}

impl Store {
    pub fn engines(&self) -> Res<Vec<EngineRow>> {
        let mut st = self.conn.prepare("SELECT id, name, exe, extra_path, backend, version, origin, created_at FROM engines ORDER BY id")?;
        let rows = st.query_map([], engine_row)?.collect::<Result<_, _>>()?;
        Ok(rows)
    }

    pub fn engine(&self, id: i64) -> Res<Option<EngineRow>> {
        Ok(self
            .conn
            .query_row("SELECT id, name, exe, extra_path, backend, version, origin, created_at FROM engines WHERE id = ?1", params![id], engine_row)
            .optional()?)
    }

    /// Remembers the build of an engine once `--version` told it.
    pub fn set_engine_version(&mut self, id: i64, version: &str) -> Res<()> {
        self.conn.execute("UPDATE engines SET version = ?2 WHERE id = ?1", params![id, version])?;
        Ok(())
    }

    pub fn profiles(&self) -> Res<Vec<Profile>> {
        let mut st = self.conn.prepare(&format!("SELECT {PROFILE_COLS} FROM model_profiles ORDER BY id"))?;
        let rows = st.query_map([], profile_row)?.collect::<Result<_, _>>()?;
        Ok(rows)
    }

    pub fn profile(&self, id: i64) -> Res<Option<Profile>> {
        Ok(self.conn.query_row(&format!("SELECT {PROFILE_COLS} FROM model_profiles WHERE id = ?1"), params![id], profile_row).optional()?)
    }

    pub fn active_profile(&self) -> Res<Option<Profile>> {
        Ok(self.conn.query_row(&format!("SELECT {PROFILE_COLS} FROM model_profiles WHERE active = 1"), [], profile_row).optional()?)
    }

    /// A new, inactive profile. The caller checked the fields (name, engine and file present).
    pub fn add_profile(&mut self, e: &ProfileEdit) -> Res<Profile> {
        let t = now();
        self.conn.execute(
            "INSERT INTO model_profiles (name, engine_id, model_file, load_args, sampler, reasoning, measured, active, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?8)",
            params![
                e.name.as_deref().unwrap_or_default(),
                e.engine_id.ok_or("engine missing")?,
                e.model_file.as_deref().unwrap_or_default(),
                serde_json::to_string(e.load_args.as_deref().unwrap_or_default())?,
                serde_json::to_string(&e.sampler.clone().unwrap_or_default())?,
                e.reasoning.as_deref().unwrap_or("off"),
                e.measured.unwrap_or(false),
                t
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.profile(id)?.ok_or_else(|| "profile vanished".into())
    }

    /// Applies the set fields. None: no such profile.
    pub fn update_profile(&mut self, id: i64, e: &ProfileEdit) -> Res<Option<Profile>> {
        let Some(mut p) = self.profile(id)? else { return Ok(None) };
        // Another model file or engine: what was measured holds no more.
        let other_model = e.model_file.as_ref().is_some_and(|f| *f != p.model_file) || e.engine_id.is_some_and(|v| v != p.engine_id);
        if other_model {
            if e.measured.is_none() {
                p.measured = false;
            }
            self.conn.execute("DELETE FROM settings WHERE key IN (?1, ?2)", params![format!("bench_result_{id}"), format!("bench_results_{id}")])?;
        }
        if let Some(v) = &e.name {
            p.name = v.clone();
        }
        if let Some(v) = e.engine_id {
            p.engine_id = v;
        }
        if let Some(v) = &e.model_file {
            p.model_file = v.clone();
        }
        if let Some(v) = &e.load_args {
            if *v != p.load_args && e.measured.is_none() {
                p.measured = false; // start options typed by hand are no longer the measured ones
            }
            p.load_args = v.clone();
        }
        if let Some(v) = &e.sampler {
            p.sampler = v.clone();
        }
        if let Some(v) = &e.reasoning {
            p.reasoning = v.clone();
        }
        if let Some(v) = e.measured {
            p.measured = v;
        }
        self.conn.execute(
            "UPDATE model_profiles SET name = ?2, engine_id = ?3, model_file = ?4, load_args = ?5, sampler = ?6, reasoning = ?7,
               measured = ?8, updated_at = ?9 WHERE id = ?1",
            params![
                id,
                p.name,
                p.engine_id,
                p.model_file,
                serde_json::to_string(&p.load_args)?,
                serde_json::to_string(&p.sampler)?,
                p.reasoning,
                p.measured,
                now()
            ],
        )?;
        self.profile(id)
    }

    pub fn delete_profile(&mut self, id: i64) -> Res<ProfileDelete> {
        match self.profile(id)? {
            None => Ok(ProfileDelete::Missing),
            Some(p) if p.active => Ok(ProfileDelete::Active),
            Some(_) => {
                self.conn.execute("DELETE FROM model_profiles WHERE id = ?1", params![id])?;
                self.conn.execute("DELETE FROM settings WHERE key IN (?1, ?2)", params![format!("bench_result_{id}"), format!("bench_results_{id}")])?;
                Ok(ProfileDelete::Done)
            }
        }
    }

    /// Makes this profile the active one. False: no such profile.
    pub fn activate_profile(&mut self, id: i64) -> Res<bool> {
        let tx = self.conn.transaction()?;
        if tx.query_row("SELECT 1 FROM model_profiles WHERE id = ?1", params![id], |_| Ok(())).optional()?.is_none() {
            return Ok(false);
        }
        tx.execute("UPDATE model_profiles SET active = 0 WHERE active = 1 AND id <> ?1", params![id])?;
        tx.execute("UPDATE model_profiles SET active = 1 WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(true)
    }

    /// Sampler and thinking of the active profile for the next request (no active profile: the
    /// defaults, which are what the app sent before there were profiles).
    pub fn sampling(&self) -> Res<(Sampler, bool)> {
        Ok(self.active_profile()?.map_or_else(|| (Sampler::default(), false), |p| (p.sampler, p.reasoning == "on")))
    }

    /// The port llama-server runs on, once set (taken over from `engine.json`).
    pub fn engine_port(&self) -> Res<Option<u16>> {
        Ok(self.setting("engine_port")?.and_then(|p| p.parse().ok()))
    }

    /// What the active profile starts. None: no active profile.
    pub fn launch_spec(&self) -> Res<Option<LaunchSpec>> {
        match self.active_profile()? {
            Some(p) => self.spec_of(p).map(Some),
            None => Ok(None),
        }
    }

    /// What this profile would start. None: no such profile.
    pub fn profile_spec(&self, id: i64) -> Res<Option<LaunchSpec>> {
        match self.profile(id)? {
            Some(p) => self.spec_of(p).map(Some),
            None => Ok(None),
        }
    }

    fn spec_of(&self, p: Profile) -> Res<LaunchSpec> {
        let engine = self.engine(p.engine_id)?.ok_or("the profile has no engine")?;
        Ok(LaunchSpec {
            exe: engine.exe.into(),
            extra_path: engine.extra_path.into_iter().map(Into::into).collect(),
            model: p.model_file.into(),
            port: self.engine_port()?.unwrap_or(DEFAULT_ENGINE_PORT),
            args: p.load_args,
            profile_id: Some(p.id),
        })
    }

    /// The last benchmark result of a profile (JSON, see `crate::bench`), if it was measured with
    /// the profile's model file and engine as they are now (`Profile::bench_stamp`; a result
    /// stored before results named them counts).
    pub fn bench_result(&self, profile_id: i64) -> Res<Option<serde_json::Value>> {
        let Some(p) = self.profile(profile_id)? else { return Ok(None) };
        let stored: Option<serde_json::Value> = self.setting(&format!("bench_result_{profile_id}"))?.and_then(|v| serde_json::from_str(&v).ok());
        Ok(stored.filter(|r| measured_with(r, &p)))
    }

    /// Every kept benchmark result of a profile, newest first, one per kind of memory conditions
    /// (`bench_results_<id>`; before there was a list, the one last result); only those valid for
    /// the profile's model file and engine, as [`Store::bench_result`].
    pub fn bench_results(&self, profile_id: i64) -> Res<Vec<serde_json::Value>> {
        let Some(p) = self.profile(profile_id)? else { return Ok(Vec::new()) };
        match self.setting(&format!("bench_results_{profile_id}"))?.and_then(|v| serde_json::from_str::<Vec<serde_json::Value>>(&v).ok()) {
            Some(list) => Ok(list.into_iter().filter(|r| measured_with(r, &p)).collect()),
            None => Ok(self.bench_result(profile_id)?.into_iter().collect()),
        }
    }

    /// Stores a new result as the last one and in the list, where it replaces the results measured
    /// under similar conditions and keeps the others (`rp_core::bench::keep_results`).
    pub fn set_bench_result(&mut self, profile_id: i64, result: &serde_json::Value) -> Res<()> {
        let kept = rp_core::bench::keep_results(self.bench_results(profile_id)?, result.clone());
        self.set_setting(&format!("bench_results_{profile_id}"), &serde_json::Value::from(kept).to_string())?;
        self.set_setting(&format!("bench_result_{profile_id}"), &result.to_string())
    }

    /// `engine.json` was taken over already (it never is a second time).
    pub fn engine_json_imported(&self) -> Res<bool> {
        Ok(self.setting("engine_json_imported")?.is_some())
    }

    /// Once: `engine.json` becomes the engine "Eigener Build" and a profile with the same model
    /// and arguments (active if none is), port and stop-on-exit become settings. The file itself
    /// stays as it is. Some((engine id, profile id)) when it was taken over now.
    pub fn import_engine_json(&mut self, cfg: &EngineConfig) -> Res<Option<(i64, i64)>> {
        if self.engine_json_imported()? {
            return Ok(None);
        }
        let t = now();
        let extra: Vec<String> = cfg.extra_path.iter().map(|p| p.display().to_string()).collect();
        let name: String = cfg
            .model
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Modell".into())
            .chars()
            .take(PROFILE_NAME_MAX)
            .collect();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO engines (name, exe, extra_path, backend, version, origin, created_at) VALUES (?1, ?2, ?3, ?4, NULL, 'own', ?5)",
            params![OWN_BUILD, cfg.llama_server.display().to_string(), serde_json::to_string(&extra)?, detect_backend(&cfg.llama_server), t],
        )?;
        let engine_id = tx.last_insert_rowid();
        let none_active = tx.query_row("SELECT COUNT(*) FROM model_profiles WHERE active = 1", [], |r| r.get::<_, i64>(0))? == 0;
        tx.execute(
            "INSERT INTO model_profiles (name, engine_id, model_file, load_args, sampler, reasoning, measured, active, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'off', 0, ?6, ?7, ?7)",
            params![name, engine_id, cfg.model.display().to_string(), serde_json::to_string(&cfg.args)?, serde_json::to_string(&Sampler::legacy())?, none_active, t],
        )?;
        let profile_id = tx.last_insert_rowid();
        for (key, value) in [
            ("engine_port", cfg.port.to_string()),
            ("engine_json_imported", t.to_string()),
        ] {
            tx.execute("INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2", params![key, value])?;
        }
        tx.commit()?;
        Ok(Some((engine_id, profile_id)))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::store::tests::temp_store;

    /// M4: a benchmark result holds for the model file and engine it was measured with. Changing
    /// either drops the results and "gemessen"; a result stored afterwards by a benchmark of the old
    /// combination is not delivered; a result from before results named their combination stays.
    #[test]
    fn a_result_holds_only_for_the_model_file_and_engine_it_was_measured_with() {
        let (mut s, dir) = temp_store();
        let cfg: EngineConfig = serde_json::from_str(r#"{ "llama_server": "a.exe", "model": "m.gguf" }"#).unwrap();
        let (engine, id) = s.import_engine_json(&cfg).unwrap().unwrap();
        s.conn.execute("INSERT INTO engines (name, exe, extra_path, backend, version, origin, created_at) VALUES ('CPU', 'b.exe', '[]', 'cpu', NULL, 'own', 0)", []).unwrap();
        let other_engine = s.conn.last_insert_rowid();
        let measured = |s: &mut Store| {
            let p = s.profile(id).unwrap().unwrap();
            s.set_bench_result(id, &json!({ "rows": [], "measured_with": p.bench_stamp() })).unwrap();
            let set = ProfileEdit { load_args: Some(vec!["-c".into(), "8192".into()]), measured: Some(true), ..ProfileEdit::default() };
            assert!(s.update_profile(id, &set).unwrap().unwrap().measured);
            assert!(s.bench_result(id).unwrap().is_some());
            p
        };
        let delivered = |s: &Store| (s.bench_result(id).unwrap().is_some(), s.bench_results(id).unwrap().len());

        // Name, sampler and the same file again keep it.
        let old = measured(&mut s);
        let same = ProfileEdit { name: Some("Neu".into()), model_file: Some("m.gguf".into()), engine_id: Some(engine), ..ProfileEdit::default() };
        assert!(s.update_profile(id, &same).unwrap().unwrap().measured);
        assert_eq!(delivered(&s), (true, 1));

        // Another model file: no result, not measured.
        let file = ProfileEdit { model_file: Some("tiny-4b.gguf".into()), ..ProfileEdit::default() };
        assert!(!s.update_profile(id, &file).unwrap().unwrap().measured);
        assert_eq!(delivered(&s), (false, 0));
        // A benchmark of the old file that ends now stores its result: not valid for the new file.
        s.set_bench_result(id, &json!({ "rows": [], "measured_with": old.bench_stamp() })).unwrap();
        assert_eq!(delivered(&s), (false, 0));

        // Another engine: the same.
        measured(&mut s);
        let eng = ProfileEdit { engine_id: Some(other_engine), ..ProfileEdit::default() };
        assert!(!s.update_profile(id, &eng).unwrap().unwrap().measured);
        assert_eq!(delivered(&s), (false, 0));

        // A result stored before results named their combination is still shown (nothing to compare).
        s.set_bench_result(id, &json!({ "rows": [] })).unwrap();
        assert_eq!(delivered(&s), (true, 1));
        drop(s);
        let _ = std::fs::remove_dir_all(dir);
    }
}
