//! SQLite persistence. One connection in WAL mode, used only from blocking
//! threads so the async runtime's workers never wait on disk.

mod logs;
mod scans;
mod settings;

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::application::ports::{RepoResult, RepositoryError};

const DATABASE_FILE: &str = "adtech-scanner.sqlite3";

/// Each entry migrates the schema from `user_version = index` to `index + 1`.
const MIGRATIONS: &[&str] = &[
    "
    CREATE TABLE scans (
        id TEXT PRIMARY KEY,
        url TEXT NOT NULL,
        hostname TEXT NOT NULL,
        device TEXT NOT NULL,
        status TEXT NOT NULL,
        started_at REAL NOT NULL,
        finished_at REAL,
        duration_ms REAL,
        cost_usd REAL NOT NULL DEFAULT 0,
        critical_count INTEGER NOT NULL DEFAULT 0,
        high_count INTEGER NOT NULL DEFAULT 0,
        medium_count INTEGER NOT NULL DEFAULT 0,
        low_count INTEGER NOT NULL DEFAULT 0,
        info_count INTEGER NOT NULL DEFAULT 0,
        health_score INTEGER,
        prebid_version TEXT,
        ai_used INTEGER NOT NULL DEFAULT 0,
        error TEXT,
        options TEXT NOT NULL,
        metrics TEXT,
        overview TEXT
    );
    CREATE INDEX scans_by_start ON scans (started_at DESC);

    CREATE TABLE findings (
        id TEXT PRIMARY KEY,
        scan_id TEXT NOT NULL REFERENCES scans (id) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        data TEXT NOT NULL
    );
    CREATE INDEX findings_by_scan ON findings (scan_id, position);

    CREATE TABLE auctions (
        scan_id TEXT NOT NULL REFERENCES scans (id) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        data TEXT NOT NULL,
        PRIMARY KEY (scan_id, position)
    );
    CREATE TABLE slots (
        scan_id TEXT NOT NULL REFERENCES scans (id) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        data TEXT NOT NULL,
        PRIMARY KEY (scan_id, position)
    );
    CREATE TABLE scripts (
        scan_id TEXT NOT NULL REFERENCES scans (id) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        data TEXT NOT NULL,
        PRIMARY KEY (scan_id, position)
    );

    CREATE TABLE steps (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        id TEXT NOT NULL UNIQUE,
        scan_id TEXT NOT NULL REFERENCES scans (id) ON DELETE CASCADE,
        data TEXT NOT NULL
    );
    CREATE INDEX steps_by_scan ON steps (scan_id, seq);

    CREATE TABLE logs (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        id TEXT NOT NULL UNIQUE,
        scan_id TEXT NOT NULL REFERENCES scans (id) ON DELETE CASCADE,
        step_id TEXT NOT NULL,
        node TEXT NOT NULL,
        ts REAL NOT NULL,
        level TEXT NOT NULL,
        kind TEXT NOT NULL,
        message TEXT NOT NULL,
        data TEXT
    );
    CREATE INDEX logs_by_scan ON logs (scan_id, seq);

    CREATE VIRTUAL TABLE logs_fts USING fts5 (message, data_text, tokenize = 'unicode61');
    CREATE TRIGGER logs_fts_delete AFTER DELETE ON logs BEGIN
        DELETE FROM logs_fts WHERE rowid = old.seq;
    END;

    CREATE TABLE settings (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        data TEXT NOT NULL
    );
    ",
    // Settings are one JSON document and every field is required on load, so
    // fields added to the contract are written into the stored document here.
    "
    UPDATE settings
    SET data = json_set(data, '$.jevEnabled', json('false'), '$.hasTypesafeKey', json('false'));
    ",
    "
    ALTER TABLE scans ADD COLUMN diagnostics TEXT;
    UPDATE settings SET data = json_set(data, '$.checkSellersJson', json('false'));
    ",
];

/// Implements the scan, log and settings repositories on one database.
#[derive(Clone)]
pub struct SqliteStore {
    connection: Arc<Mutex<Connection>>,
}

impl SqliteStore {
    /// Opens (creating if needed) the database in `data_dir`.
    pub fn open(data_dir: &Path) -> RepoResult<Self> {
        std::fs::create_dir_all(data_dir).map_err(|error| {
            RepositoryError(format!("cannot create {}: {error}", data_dir.display()))
        })?;
        Self::prepare(Connection::open(data_dir.join(DATABASE_FILE)).map_err(storage_error)?)
    }

    pub fn open_in_memory() -> RepoResult<Self> {
        Self::prepare(Connection::open_in_memory().map_err(storage_error)?)
    }

    fn prepare(mut connection: Connection) -> RepoResult<Self> {
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .and_then(|()| connection.pragma_update(None, "synchronous", "NORMAL"))
            .and_then(|()| connection.pragma_update(None, "foreign_keys", "ON"))
            .and_then(|()| connection.busy_timeout(std::time::Duration::from_secs(5)))
            .and_then(|()| migrate(&mut connection))
            .map_err(storage_error)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    async fn call<T, F>(&self, work: F) -> RepoResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let mut guard = connection
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            work(&mut guard)
        })
        .await
        .map_err(|error| RepositoryError(format!("storage task failed: {error}")))?
        .map_err(storage_error)
    }
}

fn migrate(connection: &mut Connection) -> rusqlite::Result<()> {
    let current: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for (version, migration) in (1u32..).zip(MIGRATIONS).skip(current as usize) {
        let transaction = connection.transaction()?;
        transaction.execute_batch(migration)?;
        transaction.pragma_update(None, "user_version", version)?;
        transaction.commit()?;
    }
    Ok(())
}

fn storage_error(error: rusqlite::Error) -> RepositoryError {
    RepositoryError(error.to_string())
}

fn to_json<T: Serialize>(value: &T) -> rusqlite::Result<String> {
    serde_json::to_string(value)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

fn from_json<T: DeserializeOwned>(text: &str) -> rusqlite::Result<T> {
    serde_json::from_str(text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

/// Contract enums are stored by their JSON name (`"completed"`, `"mobile"`).
fn enum_text<T: Serialize>(value: &T) -> rusqlite::Result<String> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => Ok(name),
        Ok(other) => Err(rusqlite::Error::ToSqlConversionFailure(
            format!("expected a unit enum, got {other}").into(),
        )),
        Err(error) => Err(rusqlite::Error::ToSqlConversionFailure(Box::new(error))),
    }
}

fn enum_from_text<T: DeserializeOwned>(text: String) -> rusqlite::Result<T> {
    serde_json::from_value(serde_json::Value::String(text)).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_schema_version_two_database_gains_the_diagnostics_column_and_setting() {
        const V2_SETTINGS: &str = r#"{"filterModel":"claude-haiku-5-5","analysisModel":"claude-sonnet-5-5","analysisEffort":"medium","budgetUsd":0.5,"chromePath":null,"headless":true,"defaultDevice":"desktop","defaultObserveSeconds":20,"theme":"system","hasApiKey":false,"jevEnabled":true,"hasTypesafeKey":false}"#;
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(MIGRATIONS[0]).unwrap();
        connection.execute_batch(MIGRATIONS[1]).unwrap();
        connection.pragma_update(None, "user_version", 2).unwrap();
        connection
            .execute(
                "INSERT INTO settings (id, data) VALUES (1, ?1)",
                [V2_SETTINGS],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO scans (id, url, hostname, device, status, started_at, options)
                 VALUES ('old', 'https://a.test/', 'a.test', 'desktop', 'completed', 1, '{}')",
                [],
            )
            .unwrap();

        migrate(&mut connection).unwrap();

        let stored: String = connection
            .query_row("SELECT data FROM settings WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        let settings: adscan_model::Settings = from_json(&stored).unwrap();
        assert!(settings.jev_enabled, "existing choices are kept");
        assert!(!settings.check_sellers_json, "the new lookup starts off");
        let diagnostics: Option<String> = connection
            .query_row(
                "SELECT diagnostics FROM scans WHERE id = 'old'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(diagnostics, None);
    }

    #[test]
    fn migrations_set_user_version_and_are_idempotent_across_reopens() {
        let dir = tempfile::tempdir().unwrap();
        for _ in 0..2 {
            let store = SqliteStore::open(dir.path()).unwrap();
            let connection = store.connection.lock().unwrap();
            let version: u32 = connection
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap();
            assert_eq!(version as usize, MIGRATIONS.len());
            let journal: String = connection
                .pragma_query_value(None, "journal_mode", |row| row.get(0))
                .unwrap();
            assert_eq!(journal, "wal");
            let foreign_keys: i64 = connection
                .pragma_query_value(None, "foreign_keys", |row| row.get(0))
                .unwrap();
            assert_eq!(foreign_keys, 1);
        }
    }

    #[test]
    fn a_database_from_an_older_schema_version_is_upgraded() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 0).unwrap();
        migrate(&mut connection).unwrap();
        let tables: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name IN ('scans', 'findings', 'logs', 'logs_fts', 'settings')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 5);
    }

    #[test]
    fn settings_saved_by_schema_version_one_load_after_the_upgrade() {
        const V1_SETTINGS: &str = r#"{"filterModel":"claude-haiku-5-5","analysisModel":"claude-sonnet-5-5","analysisEffort":"high","budgetUsd":1.25,"chromePath":null,"headless":false,"defaultDevice":"mobile","defaultObserveSeconds":30,"theme":"dark","hasApiKey":false}"#;
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(MIGRATIONS[0]).unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        connection
            .execute(
                "INSERT INTO settings (id, data) VALUES (1, ?1)",
                [V1_SETTINGS],
            )
            .unwrap();

        migrate(&mut connection).unwrap();

        let version: u32 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
        let stored: String = connection
            .query_row("SELECT data FROM settings WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        let settings: adscan_model::Settings = from_json(&stored).unwrap();
        assert_eq!(
            settings,
            adscan_model::Settings {
                analysis_effort: adscan_model::Effort::High,
                budget_usd: 1.25,
                headless: false,
                default_device: adscan_model::DeviceProfile::Mobile,
                default_observe_seconds: 30,
                theme: adscan_model::ThemePreference::Dark,
                ..adscan_model::Settings::default()
            }
        );
    }
}
