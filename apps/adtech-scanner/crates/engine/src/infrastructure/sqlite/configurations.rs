use adscan_model::ScanConfiguration;
use async_trait::async_trait;
use rusqlite::{OptionalExtension, Row, params};

use super::{SqliteStore, from_json, to_json};
use crate::application::ports::{RepoResult, ScanConfigurationRepository};

/// A configuration only points at its last scan while that scan exists:
/// deleting scans must not leave links to nothing.
const SELECT: &str = "SELECT id, name, options, created_at, updated_at,
        (SELECT id FROM scans WHERE scans.id = scan_configurations.last_scan_id),
        last_run_at
     FROM scan_configurations";

#[async_trait]
impl ScanConfigurationRepository for SqliteStore {
    async fn list(&self) -> RepoResult<Vec<ScanConfiguration>> {
        self.call(|connection| {
            let mut statement =
                connection.prepare(&format!("{SELECT} ORDER BY name COLLATE NOCASE, id"))?;
            statement.query_map([], read)?.collect()
        })
        .await
    }

    async fn get(&self, id: &str) -> RepoResult<Option<ScanConfiguration>> {
        let id = id.to_string();
        self.call(move |connection| {
            connection
                .query_row(&format!("{SELECT} WHERE id = ?1"), [id], read)
                .optional()
        })
        .await
    }

    async fn save(&self, configuration: &ScanConfiguration) -> RepoResult<()> {
        let configuration = configuration.clone();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO scan_configurations (id, name, options, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (id) DO UPDATE SET
                     name = excluded.name, options = excluded.options, updated_at = excluded.updated_at",
                params![
                    configuration.id,
                    configuration.name,
                    to_json(&configuration.options)?,
                    configuration.created_at,
                    configuration.updated_at,
                ],
            )?;
            Ok(())
        })
        .await
    }

    async fn delete(&self, id: &str) -> RepoResult<bool> {
        let id = id.to_string();
        self.call(move |connection| {
            Ok(connection.execute("DELETE FROM scan_configurations WHERE id = ?1", [id])? > 0)
        })
        .await
    }

    async fn record_run(&self, id: &str, scan_id: &str, at_ms: f64) -> RepoResult<()> {
        let id = id.to_string();
        let scan_id = scan_id.to_string();
        self.call(move |connection| {
            connection.execute(
                "UPDATE scan_configurations SET last_scan_id = ?2, last_run_at = ?3 WHERE id = ?1",
                params![id, scan_id, at_ms],
            )?;
            Ok(())
        })
        .await
    }
}

fn read(row: &Row<'_>) -> rusqlite::Result<ScanConfiguration> {
    Ok(ScanConfiguration {
        id: row.get(0)?,
        name: row.get(1)?,
        options: from_json(&row.get::<_, String>(2)?)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
        last_scan_id: row.get(5)?,
        last_run_at: row.get(6)?,
    })
}
