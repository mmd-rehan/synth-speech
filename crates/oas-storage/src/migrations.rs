//! Forward-only schema migrations, applied by index and tracked in
//! `PRAGMA user_version`. To add a migration, append its SQL to [`MIGRATIONS`];
//! never edit or reorder an existing entry.

use oas_core::{OasError, Result};
use rusqlite::Connection;

/// Each element is one migration's SQL, applied in order. The 1-based count of
/// applied migrations is stored in `user_version`.
const MIGRATIONS: &[&str] = &[
    // V1: the machine profile cache (a single newest-profile row).
    "CREATE TABLE machine_profile (
        id             INTEGER PRIMARY KEY CHECK (id = 1),
        profile_json   TEXT    NOT NULL,
        captured_at_ms INTEGER NOT NULL
    );",
    // V2: installed models.
    "CREATE TABLE installed_models (
        id              TEXT    PRIMARY KEY,
        file_name       TEXT    NOT NULL,
        size_bytes      INTEGER NOT NULL,
        sha256          TEXT    NOT NULL,
        installed_at_ms INTEGER NOT NULL
    );",
    // V3: the job queue and incrementally stored transcripts.
    "CREATE TABLE jobs (
        id                TEXT    PRIMARY KEY,
        input_path        TEXT    NOT NULL,
        input_duration_ms INTEGER,
        model_id          TEXT    NOT NULL,
        options_json      TEXT    NOT NULL,
        priority          TEXT    NOT NULL,
        rank              INTEGER NOT NULL,
        state             TEXT    NOT NULL,
        pause_origin      TEXT,
        progress          REAL    NOT NULL DEFAULT 0,
        retries           INTEGER NOT NULL DEFAULT 0,
        error_code        TEXT,
        error_message     TEXT,
        metrics_json      TEXT,
        created_at_ms     INTEGER NOT NULL,
        started_at_ms     INTEGER,
        finished_at_ms    INTEGER
    );
    CREATE INDEX idx_jobs_state ON jobs(state);
    CREATE TABLE segments (
        job_id     TEXT    NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
        idx        INTEGER NOT NULL,
        start_ms   INTEGER NOT NULL,
        end_ms     INTEGER NOT NULL,
        text       TEXT    NOT NULL,
        words_json TEXT,
        PRIMARY KEY (job_id, idx)
    );",
];

/// The schema version after all migrations are applied.
pub const LATEST_VERSION: u32 = MIGRATIONS.len() as u32;

fn map_err(e: rusqlite::Error) -> OasError {
    OasError::Io {
        reason: format!("migration error: {e}"),
    }
}

/// Read the current applied-migration count.
pub fn current_version(conn: &Connection) -> Result<u32> {
    let v: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(map_err)?;
    Ok(v as u32)
}

/// Apply any migrations the database has not yet run.
pub fn run(conn: &Connection) -> Result<()> {
    let applied = current_version(conn)?;
    for (idx, sql) in MIGRATIONS.iter().enumerate() {
        let version = idx as u32 + 1;
        if version <= applied {
            continue;
        }
        conn.execute_batch(sql).map_err(map_err)?;
        // `user_version` only accepts a literal, so format it in (it is derived
        // from our own code, never user input).
        conn.pragma_update(None, "user_version", version as i64)
            .map_err(map_err)?;
        tracing::info!(version, "applied database migration");
    }
    Ok(())
}
