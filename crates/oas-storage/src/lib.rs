//! SQLite-backed persistence for Open Audio Studio.
//!
//! Uses a bundled SQLite (no system dependency) and a tiny forward-only
//! migration runner keyed on `PRAGMA user_version`. A heavier framework can be
//! adopted later if migrations become complex; for now this keeps the
//! dependency surface minimal and fully testable in-memory.

#![forbid(unsafe_code)]

mod migrations;
mod settings;

pub use migrations::LATEST_VERSION as SCHEMA_VERSION;
pub use settings::Settings;

use std::path::Path;
use std::sync::Mutex;

use oas_core::{OasError, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;

/// A row in `installed_models`, also the IPC shape for the installed list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct InstalledModel {
    pub id: String,
    pub file_name: String,
    #[specta(type = f64)]
    pub size_bytes: u64,
    pub sha256: String,
    #[specta(type = f64)]
    pub installed_at_ms: u64,
}

/// A row in `jobs`. Fields with rules (state, priority) are plain strings here:
/// `oas-jobs` owns what they mean.
#[derive(Debug, Clone, PartialEq)]
pub struct JobRow {
    pub id: String,
    pub input_path: String,
    pub input_duration_ms: Option<u64>,
    pub model_id: String,
    pub options_json: String,
    pub priority: String,
    pub rank: i64,
    pub state: String,
    pub pause_origin: Option<String>,
    pub progress: f32,
    pub retries: u32,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub metrics_json: Option<String>,
    pub created_at_ms: u64,
    pub started_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
}

/// A row in `segments`: one timed line of a transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentRow {
    pub idx: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub words_json: Option<String>,
}

/// A thread-safe handle to the application database.
pub struct Database {
    conn: Mutex<Connection>,
}

fn map_err(e: rusqlite::Error) -> OasError {
    OasError::Io {
        reason: format!("database error: {e}"),
    }
}

impl Database {
    /// Open (creating if needed) the database at `path` and run migrations.
    ///
    /// When existing data is about to be migrated, a copy is first saved next
    /// to the database as `<file>.v<old version>.bak`.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).map_err(map_err)?;
        let version = migrations::current_version(&conn)?;
        if version > 0 && version < migrations::LATEST_VERSION {
            let mut name = path.as_os_str().to_owned();
            name.push(format!(".v{version}.bak"));
            let backup = std::path::PathBuf::from(name);
            // An old backup would make VACUUM INTO fail.
            let _ = std::fs::remove_file(&backup);
            let target = backup.to_string_lossy().into_owned();
            conn.execute("VACUUM INTO ?1", [target]).map_err(map_err)?;
        }
        Self::from_connection(conn)
    }

    /// Open an in-memory database (used by tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(map_err)?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(map_err)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(map_err)?;
        // Safe with WAL (a power cut can lose the last commits, never corrupt),
        // and it keeps a write per transcript segment cheap.
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(map_err)?;
        migrations::run(&conn)?;
        Ok(Database {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A poisoned lock means a previous DB operation panicked; recover the
        // guard rather than propagate the panic.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Store the latest machine profile (as JSON) with its capture timestamp.
    /// There is only ever one row (the newest profile).
    pub fn save_machine_profile(&self, profile_json: &str, captured_at_ms: u64) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO machine_profile (id, profile_json, captured_at_ms)
             VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET
               profile_json = excluded.profile_json,
               captured_at_ms = excluded.captured_at_ms",
            rusqlite::params![profile_json, captured_at_ms as i64],
        )
        .map_err(map_err)?;
        Ok(())
    }

    /// Load the stored machine profile JSON and its timestamp, if present.
    pub fn load_machine_profile(&self) -> Result<Option<(String, u64)>> {
        let conn = self.lock();
        let row = conn
            .query_row(
                "SELECT profile_json, captured_at_ms FROM machine_profile WHERE id = 1",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(map_err(other)),
            })?;
        Ok(row.map(|(json, ts)| (json, ts as u64)))
    }

    /// The current schema version (for diagnostics/tests).
    pub fn schema_version(&self) -> Result<u32> {
        let conn = self.lock();
        migrations::current_version(&conn)
    }

    /// Insert or update an installed model record.
    pub fn save_installed_model(&self, m: &InstalledModel) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO installed_models (id, file_name, size_bytes, sha256, installed_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
               file_name = excluded.file_name,
               size_bytes = excluded.size_bytes,
               sha256 = excluded.sha256,
               installed_at_ms = excluded.installed_at_ms",
            rusqlite::params![
                m.id,
                m.file_name,
                m.size_bytes as i64,
                m.sha256,
                m.installed_at_ms as i64
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    /// List installed models, newest first.
    pub fn list_installed_models(&self) -> Result<Vec<InstalledModel>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, file_name, size_bytes, sha256, installed_at_ms
                 FROM installed_models ORDER BY installed_at_ms DESC",
            )
            .map_err(map_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(InstalledModel {
                    id: r.get(0)?,
                    file_name: r.get(1)?,
                    size_bytes: r.get::<_, i64>(2)? as u64,
                    sha256: r.get(3)?,
                    installed_at_ms: r.get::<_, i64>(4)? as u64,
                })
            })
            .map_err(map_err)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(map_err)?);
        }
        Ok(out)
    }

    /// Whether a model id is recorded as installed.
    pub fn is_installed(&self, id: &str) -> Result<bool> {
        let conn = self.lock();
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM installed_models WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .map_err(map_err)?;
        Ok(n > 0)
    }

    /// Remove an installed model record.
    pub fn remove_installed_model(&self, id: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute("DELETE FROM installed_models WHERE id = ?1", [id])
            .map_err(map_err)?;
        Ok(())
    }

    /// Insert or fully replace a job.
    pub fn save_job(&self, j: &JobRow) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO jobs (id, input_path, input_duration_ms, model_id,
               options_json, priority, rank, state, pause_origin, progress, retries,
               error_code, error_message, metrics_json, created_at_ms, started_at_ms,
               finished_at_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)
             ON CONFLICT(id) DO UPDATE SET
               input_duration_ms = excluded.input_duration_ms,
               options_json = excluded.options_json,
               priority = excluded.priority, rank = excluded.rank,
               state = excluded.state, pause_origin = excluded.pause_origin,
               progress = excluded.progress, retries = excluded.retries,
               error_code = excluded.error_code, error_message = excluded.error_message,
               metrics_json = excluded.metrics_json,
               started_at_ms = excluded.started_at_ms,
               finished_at_ms = excluded.finished_at_ms",
            rusqlite::params![
                j.id,
                j.input_path,
                j.input_duration_ms.map(|v| v as i64),
                j.model_id,
                j.options_json,
                j.priority,
                j.rank,
                j.state,
                j.pause_origin,
                j.progress as f64,
                j.retries as i64,
                j.error_code,
                j.error_message,
                j.metrics_json,
                j.created_at_ms as i64,
                j.started_at_ms.map(|v| v as i64),
                j.finished_at_ms.map(|v| v as i64),
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    /// All jobs, in queue order.
    pub fn list_jobs(&self) -> Result<Vec<JobRow>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, input_path, input_duration_ms, model_id, options_json, priority,
                        rank, state, pause_origin, progress, retries, error_code,
                        error_message, metrics_json, created_at_ms, started_at_ms,
                        finished_at_ms
                 FROM jobs ORDER BY rank, created_at_ms",
            )
            .map_err(map_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(JobRow {
                    id: r.get(0)?,
                    input_path: r.get(1)?,
                    input_duration_ms: r.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                    model_id: r.get(3)?,
                    options_json: r.get(4)?,
                    priority: r.get(5)?,
                    rank: r.get(6)?,
                    state: r.get(7)?,
                    pause_origin: r.get(8)?,
                    progress: r.get::<_, f64>(9)? as f32,
                    retries: r.get::<_, i64>(10)? as u32,
                    error_code: r.get(11)?,
                    error_message: r.get(12)?,
                    metrics_json: r.get(13)?,
                    created_at_ms: r.get::<_, i64>(14)? as u64,
                    started_at_ms: r.get::<_, Option<i64>>(15)?.map(|v| v as u64),
                    finished_at_ms: r.get::<_, Option<i64>>(16)?.map(|v| v as u64),
                })
            })
            .map_err(map_err)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(map_err)
    }

    /// Delete a job and (by cascade) its transcript.
    pub fn delete_job(&self, id: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute("DELETE FROM jobs WHERE id = ?1", [id])
            .map_err(map_err)?;
        Ok(())
    }

    /// Store one transcript segment. Replacing an index is harmless, so a
    /// redelivered segment cannot duplicate a line.
    pub fn save_segment(&self, job_id: &str, s: &SegmentRow) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT OR REPLACE INTO segments (job_id, idx, start_ms, end_ms, text, words_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                job_id,
                s.idx,
                s.start_ms as i64,
                s.end_ms as i64,
                s.text,
                s.words_json
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    /// A job's segments in transcript order.
    pub fn list_segments(&self, job_id: &str) -> Result<Vec<SegmentRow>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT idx, start_ms, end_ms, text, words_json
                 FROM segments WHERE job_id = ?1 ORDER BY idx",
            )
            .map_err(map_err)?;
        let rows = stmt
            .query_map([job_id], |r| {
                Ok(SegmentRow {
                    idx: r.get(0)?,
                    start_ms: r.get::<_, i64>(1)? as u64,
                    end_ms: r.get::<_, i64>(2)? as u64,
                    text: r.get(3)?,
                    words_json: r.get(4)?,
                })
            })
            .map_err(map_err)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(map_err)
    }

    /// Replace one segment's text (an edit in the transcript viewer).
    pub fn update_segment_text(&self, job_id: &str, idx: u32, text: &str) -> Result<()> {
        let conn = self.lock();
        let changed = conn
            .execute(
                "UPDATE segments SET text = ?3 WHERE job_id = ?1 AND idx = ?2",
                rusqlite::params![job_id, idx, text],
            )
            .map_err(map_err)?;
        if changed == 0 {
            return Err(OasError::InvalidInput {
                reason: "that transcript line no longer exists".into(),
            });
        }
        Ok(())
    }

    /// A stored speed correction and its sample count.
    pub fn get_calibration(&self, accelerator: &str, family: &str) -> Result<Option<(f32, u32)>> {
        let conn = self.lock();
        conn.query_row(
            "SELECT correction, samples FROM calibration
             WHERE accelerator = ?1 AND model_family = ?2",
            [accelerator, family],
            |r| Ok((r.get::<_, f64>(0)? as f32, r.get::<_, i64>(1)? as u32)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(map_err(other)),
        })
    }

    /// Store a speed correction.
    pub fn save_calibration(
        &self,
        accelerator: &str,
        family: &str,
        correction: f32,
        samples: u32,
        updated_at_ms: u64,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO calibration (accelerator, model_family, correction, samples, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(accelerator, model_family) DO UPDATE SET
               correction = excluded.correction,
               samples = excluded.samples,
               updated_at_ms = excluded.updated_at_ms",
            rusqlite::params![
                accelerator,
                family,
                correction as f64,
                samples,
                updated_at_ms as i64
            ],
        )
        .map_err(map_err)?;
        Ok(())
    }

    /// Load settings, with defaults for anything unset or unreadable.
    pub fn load_settings(&self) -> Result<Settings> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare("SELECT key, value_json FROM settings")
            .map_err(map_err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(map_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(map_err)?;
        Ok(Settings::from_stored(rows))
    }

    /// Save every setting, atomically.
    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(map_err)?;
        for (key, value) in settings.to_stored()? {
            tx.execute(
                "INSERT INTO settings (key, value_json) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
                rusqlite::params![key, value],
            )
            .map_err(map_err)?;
        }
        tx.commit().map_err(map_err)
    }

    /// Remove a job's transcript (a retry starts clean).
    pub fn clear_segments(&self, job_id: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute("DELETE FROM segments WHERE job_id = ?1", [job_id])
            .map_err(map_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_bring_schema_to_latest() {
        let db = Database::open_in_memory().expect("open");
        assert_eq!(db.schema_version().unwrap(), migrations::LATEST_VERSION);
    }

    #[test]
    fn installed_models_crud() {
        let db = Database::open_in_memory().expect("open");
        assert!(db.list_installed_models().unwrap().is_empty());
        assert!(!db.is_installed("whisper-tiny").unwrap());

        let m = InstalledModel {
            id: "whisper-tiny".into(),
            file_name: "ggml-tiny.bin".into(),
            size_bytes: 77_691_713,
            sha256: "be07".into(),
            installed_at_ms: 123,
        };
        db.save_installed_model(&m).unwrap();
        assert!(db.is_installed("whisper-tiny").unwrap());
        assert_eq!(db.list_installed_models().unwrap(), vec![m]);

        db.remove_installed_model("whisper-tiny").unwrap();
        assert!(!db.is_installed("whisper-tiny").unwrap());
    }

    fn job(id: &str, rank: i64) -> JobRow {
        JobRow {
            id: id.into(),
            input_path: "/Users/a b/Café – 録音.wav".into(),
            input_duration_ms: Some(1500),
            model_id: "whisper-tiny".into(),
            options_json: "{}".into(),
            priority: "normal".into(),
            rank,
            state: "queued".into(),
            pause_origin: None,
            progress: 0.25,
            retries: 1,
            error_code: None,
            error_message: None,
            metrics_json: None,
            created_at_ms: 10,
            started_at_ms: None,
            finished_at_ms: None,
        }
    }

    fn seg(idx: u32, text: &str) -> SegmentRow {
        SegmentRow {
            idx,
            start_ms: idx as u64 * 1000,
            end_ms: idx as u64 * 1000 + 900,
            text: text.into(),
            words_json: None,
        }
    }

    #[test]
    fn jobs_roundtrip_in_rank_order_and_update_in_place() {
        let db = Database::open_in_memory().unwrap();
        db.save_job(&job("b", 2)).unwrap();
        let mut a = job("a", 1);
        db.save_job(&a).unwrap();
        let ids: Vec<_> = db.list_jobs().unwrap().into_iter().map(|j| j.id).collect();
        assert_eq!(ids, ["a", "b"]);

        a.state = "failed".into();
        a.error_code = Some("X".into());
        a.finished_at_ms = Some(99);
        db.save_job(&a).unwrap();
        assert_eq!(db.list_jobs().unwrap()[0], a);
    }

    #[test]
    fn updating_a_job_keeps_its_segments() {
        let db = Database::open_in_memory().unwrap();
        let j = job("a", 1);
        db.save_job(&j).unwrap();
        db.save_segment("a", &seg(0, "hello")).unwrap();
        db.save_job(&j).unwrap();
        assert_eq!(db.list_segments("a").unwrap().len(), 1);
    }

    #[test]
    fn segments_keep_order_and_never_duplicate_an_index() {
        let db = Database::open_in_memory().unwrap();
        db.save_job(&job("a", 1)).unwrap();
        db.save_segment("a", &seg(1, "two")).unwrap();
        db.save_segment("a", &seg(0, "one")).unwrap();
        db.save_segment("a", &seg(1, "two again")).unwrap();
        let texts: Vec<_> = db
            .list_segments("a")
            .unwrap()
            .into_iter()
            .map(|s| s.text)
            .collect();
        assert_eq!(texts, ["one", "two again"]);
        db.clear_segments("a").unwrap();
        assert!(db.list_segments("a").unwrap().is_empty());
    }

    #[test]
    fn editing_a_segment_changes_only_its_text() {
        let db = Database::open_in_memory().unwrap();
        db.save_job(&job("a", 1)).unwrap();
        db.save_segment("a", &seg(0, "helo")).unwrap();
        db.update_segment_text("a", 0, "hello").unwrap();
        let rows = db.list_segments("a").unwrap();
        assert_eq!(rows[0].text, "hello");
        assert_eq!(rows[0].end_ms, 900);
        let missing = db.update_segment_text("a", 9, "x").unwrap_err();
        assert_eq!(missing.code(), "INVALID_INPUT");
    }

    #[test]
    fn settings_default_then_roundtrip_under_the_documented_keys() {
        let db = Database::open_in_memory().unwrap();
        assert_eq!(db.load_settings().unwrap(), Settings::default());
        let saved = Settings {
            theme: "dark".into(),
            offline_mode: true,
            thread_cap_per_job: 4,
            models_dir: Some("/Volumes/My Disk/modèles".into()),
            ..Settings::default()
        };
        db.save_settings(&saved).unwrap();
        db.save_settings(&saved).unwrap(); // saving twice is harmless
        assert_eq!(db.load_settings().unwrap(), saved);

        let conn = db.lock();
        let offline: String = conn
            .query_row(
                "SELECT value_json FROM settings WHERE key = 'net.offline_mode'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(offline, "true");
    }

    #[test]
    fn calibration_roundtrips_and_replaces() {
        let db = Database::open_in_memory().unwrap();
        assert_eq!(db.get_calibration("metal", "whisper").unwrap(), None);
        db.save_calibration("metal", "whisper", 0.5, 1, 10).unwrap();
        db.save_calibration("metal", "whisper", 0.75, 2, 20)
            .unwrap();
        assert_eq!(
            db.get_calibration("metal", "whisper").unwrap(),
            Some((0.75, 2))
        );
        assert_eq!(db.get_calibration("cpu_neon", "whisper").unwrap(), None);
    }

    #[test]
    fn a_bad_stored_setting_falls_back_alone() {
        let db = Database::open_in_memory().unwrap();
        db.save_settings(&Settings {
            theme: "dark".into(),
            ..Settings::default()
        })
        .unwrap();
        db.lock()
            .execute(
                "UPDATE settings SET value_json = '\"lots\"'
                 WHERE key = 'perf.thread_cap_per_job'",
                [],
            )
            .unwrap();
        let loaded = db.load_settings().unwrap();
        assert_eq!(loaded.thread_cap_per_job, 8);
        assert_eq!(loaded.theme, "dark");
    }

    #[test]
    fn deleting_a_job_removes_its_transcript() {
        let db = Database::open_in_memory().unwrap();
        db.save_job(&job("a", 1)).unwrap();
        db.save_segment("a", &seg(0, "x")).unwrap();
        db.delete_job("a").unwrap();
        assert!(db.list_jobs().unwrap().is_empty());
        assert!(db.list_segments("a").unwrap().is_empty());
    }

    #[test]
    fn upgrading_a_v2_database_keeps_data_and_writes_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oäs db.sqlite");
        {
            // The schema exactly as V2 shipped it.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE machine_profile (
                    id INTEGER PRIMARY KEY CHECK (id = 1),
                    profile_json TEXT NOT NULL, captured_at_ms INTEGER NOT NULL);
                 CREATE TABLE installed_models (
                    id TEXT PRIMARY KEY, file_name TEXT NOT NULL, size_bytes INTEGER NOT NULL,
                    sha256 TEXT NOT NULL, installed_at_ms INTEGER NOT NULL);
                 INSERT INTO installed_models VALUES ('whisper-tiny','m.bin',1,'ab',2);
                 PRAGMA user_version = 2;",
            )
            .unwrap();
        }
        let db = Database::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
        assert!(db.is_installed("whisper-tiny").unwrap());
        db.save_job(&job("a", 1)).unwrap();
        assert!(dir.path().join("oäs db.sqlite.v2.bak").exists());
    }

    #[test]
    fn profile_roundtrips_and_upserts() {
        let db = Database::open_in_memory().expect("open");
        assert!(db.load_machine_profile().unwrap().is_none());

        db.save_machine_profile(r#"{"a":1}"#, 1000).unwrap();
        let (json, ts) = db.load_machine_profile().unwrap().unwrap();
        assert_eq!(json, r#"{"a":1}"#);
        assert_eq!(ts, 1000);

        // Saving again replaces the single row.
        db.save_machine_profile(r#"{"a":2}"#, 2000).unwrap();
        let (json, ts) = db.load_machine_profile().unwrap().unwrap();
        assert_eq!(json, r#"{"a":2}"#);
        assert_eq!(ts, 2000);
    }
}
