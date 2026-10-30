//! SQLite-backed persistence for Open Audio Studio.
//!
//! Uses a bundled SQLite (no system dependency) and a tiny forward-only
//! migration runner keyed on `PRAGMA user_version`. A heavier framework can be
//! adopted later if migrations become complex; for now this keeps the
//! dependency surface minimal and fully testable in-memory.

#![forbid(unsafe_code)]

mod migrations;

pub use migrations::LATEST_VERSION as SCHEMA_VERSION;

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
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).map_err(map_err)?;
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
