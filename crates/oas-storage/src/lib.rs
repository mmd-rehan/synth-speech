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
