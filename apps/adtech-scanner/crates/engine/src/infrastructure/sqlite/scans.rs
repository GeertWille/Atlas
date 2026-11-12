use adscan_model::{
    AdSlot, Auction, BidderDiagnostics, DashboardStats, Finding, PageView, PipelineStep,
    ScanConfigurationRef, ScanDetail, ScanEnvironment, ScanList, ScanListQuery, ScanMetrics,
    ScanOptions, ScanStatus, ScanSummary, ScriptRecord, SeverityCounts, StepStatus,
};
use async_trait::async_trait;
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params, params_from_iter};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{SqliteStore, enum_from_text, enum_text, from_json, to_json};
use crate::application::ports::{RepoResult, ScanRepository};

const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 200;

const SUMMARY_COLUMNS: &str = "id, url, hostname, device, status, started_at, finished_at, duration_ms, cost_usd, \
     critical_count, high_count, medium_count, low_count, info_count, health_score, prebid_version, ai_used, error, \
     configuration_id, configuration_name, proxy";

#[async_trait]
impl ScanRepository for SqliteStore {
    async fn create(&self, summary: &ScanSummary, options: &ScanOptions) -> RepoResult<()> {
        let summary = summary.clone();
        let options = options.clone();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO scans (id, url, hostname, device, status, started_at, options,
                     configuration_id, configuration_name, proxy)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    summary.id,
                    summary.url,
                    summary.hostname,
                    enum_text(&summary.device)?,
                    enum_text(&summary.status)?,
                    summary.started_at,
                    to_json(&options)?,
                    summary.configuration.as_ref().map(|from| &from.id),
                    summary.configuration.as_ref().map(|from| &from.name),
                    summary.proxy.as_ref().map(to_json).transpose()?,
                ],
            )?;
            write_summary(connection, &summary)
        })
        .await
    }

    async fn save_summary(&self, summary: &ScanSummary) -> RepoResult<()> {
        let summary = summary.clone();
        self.call(move |connection| write_summary(connection, &summary))
            .await
    }

    async fn save_telemetry(
        &self,
        scan_id: &str,
        metrics: &ScanMetrics,
        pages: &[PageView],
        auctions: &[Auction],
        slots: &[AdSlot],
    ) -> RepoResult<()> {
        let scan_id = scan_id.to_string();
        let metrics = metrics.clone();
        let pages = to_json(&pages);
        let auctions = auctions.to_vec();
        let slots = slots.to_vec();
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "UPDATE scans SET metrics = ?2, pages = ?3 WHERE id = ?1",
                params![scan_id, to_json(&metrics)?, pages?],
            )?;
            replace_rows(&transaction, "auctions", &scan_id, &auctions)?;
            replace_rows(&transaction, "slots", &scan_id, &slots)?;
            transaction.commit()
        })
        .await
    }

    async fn save_environment(
        &self,
        scan_id: &str,
        environment: &ScanEnvironment,
    ) -> RepoResult<()> {
        let scan_id = scan_id.to_string();
        let environment = to_json(environment);
        self.call(move |connection| {
            connection.execute(
                "UPDATE scans SET environment = ?2 WHERE id = ?1",
                params![scan_id, environment?],
            )?;
            Ok(())
        })
        .await
    }

    async fn save_scripts(&self, scan_id: &str, scripts: &[ScriptRecord]) -> RepoResult<()> {
        let scan_id = scan_id.to_string();
        let scripts = scripts.to_vec();
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            replace_rows(&transaction, "scripts", &scan_id, &scripts)?;
            transaction.commit()
        })
        .await
    }

    async fn save_diagnostics(
        &self,
        scan_id: &str,
        diagnostics: &BidderDiagnostics,
    ) -> RepoResult<()> {
        let scan_id = scan_id.to_string();
        let diagnostics = to_json(diagnostics);
        self.call(move |connection| {
            connection.execute(
                "UPDATE scans SET diagnostics = ?2 WHERE id = ?1",
                params![scan_id, diagnostics?],
            )?;
            Ok(())
        })
        .await
    }

    async fn save_findings(&self, scan_id: &str, findings: &[Finding]) -> RepoResult<()> {
        let scan_id = scan_id.to_string();
        let findings = findings.to_vec();
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            transaction.execute("DELETE FROM findings WHERE scan_id = ?1", [&scan_id])?;
            for (position, finding) in findings.iter().enumerate() {
                transaction.execute(
                    "INSERT INTO findings (id, scan_id, position, data) VALUES (?1, ?2, ?3, ?4)",
                    params![finding.id, scan_id, position as i64, to_json(finding)?],
                )?;
            }
            transaction.commit()
        })
        .await
    }

    async fn save_overview(&self, scan_id: &str, overview: &str) -> RepoResult<()> {
        let scan_id = scan_id.to_string();
        let overview = overview.to_string();
        self.call(move |connection| {
            connection.execute(
                "UPDATE scans SET overview = ?2 WHERE id = ?1",
                params![scan_id, overview],
            )?;
            Ok(())
        })
        .await
    }

    async fn save_step(&self, step: &PipelineStep) -> RepoResult<()> {
        let step = step.clone();
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO steps (id, scan_id, data) VALUES (?1, ?2, ?3)
                 ON CONFLICT (id) DO UPDATE SET data = excluded.data",
                params![step.id, step.scan_id, to_json(&step)?],
            )?;
            Ok(())
        })
        .await
    }

    async fn summary(&self, scan_id: &str) -> RepoResult<Option<ScanSummary>> {
        let scan_id = scan_id.to_string();
        self.call(move |connection| read_summary_by_id(connection, &scan_id))
            .await
    }

    async fn detail(&self, scan_id: &str) -> RepoResult<Option<ScanDetail>> {
        let scan_id = scan_id.to_string();
        self.call(move |connection| {
            let Some(summary) = read_summary_by_id(connection, &scan_id)? else {
                return Ok(None);
            };
            type Json = Option<String>;
            type Columns = (String, Json, Option<String>, Json, Json, Json);
            let (options, metrics, overview, diagnostics, pages, environment): Columns = connection
                .query_row(
                    "SELECT options, metrics, overview, diagnostics, pages, environment
                     FROM scans WHERE id = ?1",
                    [&scan_id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                        ))
                    },
                )?;
            Ok(Some(ScanDetail {
                summary,
                options: from_json(&options)?,
                metrics: metrics
                    .as_deref()
                    .map(from_json)
                    .transpose()?
                    .unwrap_or_default(),
                findings: read_rows(connection, "findings", "position", &scan_id)?,
                auctions: read_rows(connection, "auctions", "position", &scan_id)?,
                slots: read_rows(connection, "slots", "position", &scan_id)?,
                scripts: read_rows(connection, "scripts", "position", &scan_id)?,
                steps: read_rows(connection, "steps", "seq", &scan_id)?,
                overview,
                bidder_diagnostics: diagnostics.as_deref().map(from_json).transpose()?,
                // NULL in scans stored before page views were recorded.
                pages: pages
                    .as_deref()
                    .map(from_json)
                    .transpose()?
                    .unwrap_or_default(),
                environment: environment.as_deref().map(from_json).transpose()?,
            }))
        })
        .await
    }

    async fn list(&self, query: &ScanListQuery) -> RepoResult<ScanList> {
        let query = query.clone();
        self.call(move |connection| {
            let mut conditions = Vec::new();
            let mut values: Vec<SqlValue> = Vec::new();
            if let Some(search) = query
                .search
                .as_deref()
                .map(str::trim)
                .filter(|search| !search.is_empty())
            {
                values.push(SqlValue::Text(format!("%{}%", escape_like(search))));
                conditions.push(format!("url LIKE ?{} ESCAPE '\\'", values.len()));
            }
            if let Some(status) = query.status {
                values.push(SqlValue::Text(enum_text(&status)?));
                conditions.push(format!("status = ?{}", values.len()));
            }
            if let Some(device) = query.device {
                values.push(SqlValue::Text(enum_text(&device)?));
                conditions.push(format!("device = ?{}", values.len()));
            }
            if let Some(configuration_id) = query.configuration_id {
                values.push(SqlValue::Text(configuration_id));
                conditions.push(format!("configuration_id = ?{}", values.len()));
            }
            let filter = if conditions.is_empty() {
                String::new()
            } else {
                format!("WHERE {}", conditions.join(" AND "))
            };
            let total: u32 = connection.query_row(
                &format!("SELECT count(*) FROM scans {filter}"),
                params_from_iter(values.iter()),
                |row| row.get(0),
            )?;
            let limit = query
                .limit
                .unwrap_or(DEFAULT_PAGE_SIZE)
                .clamp(1, MAX_PAGE_SIZE);
            let offset = query.offset.unwrap_or(0);
            let mut statement = connection.prepare(&format!(
                "SELECT {SUMMARY_COLUMNS} FROM scans {filter}
                 ORDER BY started_at DESC, rowid DESC LIMIT {limit} OFFSET {offset}"
            ))?;
            let items = statement
                .query_map(params_from_iter(values.iter()), read_summary)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(ScanList { items, total })
        })
        .await
    }

    async fn delete(&self, scan_id: &str) -> RepoResult<bool> {
        let scan_id = scan_id.to_string();
        self.call(move |connection| {
            Ok(connection.execute("DELETE FROM scans WHERE id = ?1", [scan_id])? > 0)
        })
        .await
    }

    async fn delete_all(&self) -> RepoResult<()> {
        self.call(|connection| {
            connection.execute("DELETE FROM scans", [])?;
            Ok(())
        })
        .await
    }

    async fn dashboard_stats(&self) -> RepoResult<DashboardStats> {
        self.call(|connection| {
            let (total_scans, sites_scanned, total_cost_usd, avg_health_score): (u32, u32, f64, Option<f64>) =
                connection.query_row(
                    "SELECT count(*), count(DISTINCT hostname), coalesce(sum(cost_usd), 0), avg(health_score) FROM scans",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )?;
            let total_findings: u32 = connection.query_row("SELECT count(*) FROM findings", [], |row| row.get(0))?;
            // "Open" means: still present in the most recent completed scan of that page and device.
            let open_critical_findings: u32 = connection.query_row(
                "SELECT coalesce(sum(critical_count), 0) FROM scans latest
                 WHERE status = 'completed' AND started_at = (
                     SELECT max(started_at) FROM scans
                     WHERE url = latest.url AND device = latest.device AND status = 'completed'
                 )",
                [],
                |row| row.get(0),
            )?;
            Ok(DashboardStats {
                total_scans,
                sites_scanned,
                open_critical_findings,
                total_findings,
                total_cost_usd,
                avg_cost_usd: if total_scans == 0 {
                    0.0
                } else {
                    total_cost_usd / f64::from(total_scans)
                },
                avg_health_score,
            })
        })
        .await
    }

    async fn fail_interrupted(&self, message: &str, now_ms: f64) -> RepoResult<u32> {
        let message = message.to_string();
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let interrupted: Vec<String> = {
                let mut statement =
                    transaction.prepare("SELECT id FROM scans WHERE status IN ('queued', 'running')")?;
                statement.query_map([], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?
            };
            for scan_id in &interrupted {
                transaction.execute(
                    "UPDATE scans SET status = ?2, error = ?3, finished_at = ?4, duration_ms = ?4 - started_at
                     WHERE id = ?1",
                    params![scan_id, enum_text(&ScanStatus::Failed)?, message, now_ms],
                )?;
                let steps: Vec<PipelineStep> = read_rows(&transaction, "steps", "seq", scan_id)?;
                for mut step in steps {
                    match step.status {
                        StepStatus::Running => step.status = StepStatus::Failed,
                        StepStatus::Pending => step.status = StepStatus::Skipped,
                        _ => continue,
                    }
                    step.summary = Some("Interrupted".into());
                    transaction.execute(
                        "UPDATE steps SET data = ?2 WHERE id = ?1",
                        params![step.id, to_json(&step)?],
                    )?;
                }
            }
            transaction.commit()?;
            Ok(interrupted.len() as u32)
        })
        .await
    }
}

fn write_summary(connection: &Connection, summary: &ScanSummary) -> rusqlite::Result<()> {
    let counts = summary.finding_counts;
    connection.execute(
        "UPDATE scans SET status = ?2, finished_at = ?3, duration_ms = ?4, cost_usd = ?5,
             critical_count = ?6, high_count = ?7, medium_count = ?8, low_count = ?9, info_count = ?10,
             health_score = ?11, prebid_version = ?12, ai_used = ?13, error = ?14
         WHERE id = ?1",
        params![
            summary.id,
            enum_text(&summary.status)?,
            summary.finished_at,
            summary.duration_ms,
            summary.cost_usd,
            counts.critical,
            counts.high,
            counts.medium,
            counts.low,
            counts.info,
            summary.health_score,
            summary.prebid_version,
            summary.ai_used,
            summary.error,
        ],
    )?;
    Ok(())
}

fn read_summary_by_id(
    connection: &Connection,
    scan_id: &str,
) -> rusqlite::Result<Option<ScanSummary>> {
    connection
        .query_row(
            &format!("SELECT {SUMMARY_COLUMNS} FROM scans WHERE id = ?1"),
            [scan_id],
            read_summary,
        )
        .optional()
}

fn read_summary(row: &Row<'_>) -> rusqlite::Result<ScanSummary> {
    Ok(ScanSummary {
        id: row.get(0)?,
        url: row.get(1)?,
        hostname: row.get(2)?,
        device: enum_from_text(row.get(3)?)?,
        status: enum_from_text(row.get(4)?)?,
        started_at: row.get(5)?,
        finished_at: row.get(6)?,
        duration_ms: row.get(7)?,
        cost_usd: row.get(8)?,
        finding_counts: SeverityCounts {
            critical: row.get(9)?,
            high: row.get(10)?,
            medium: row.get(11)?,
            low: row.get(12)?,
            info: row.get(13)?,
        },
        health_score: row.get(14)?,
        prebid_version: row.get(15)?,
        ai_used: row.get(16)?,
        error: row.get(17)?,
        configuration: match (row.get(18)?, row.get(19)?) {
            (Some(id), Some(name)) => Some(ScanConfigurationRef { id, name }),
            _ => None,
        },
        proxy: row
            .get::<_, Option<String>>(20)?
            .as_deref()
            .map(from_json)
            .transpose()?,
    })
}

/// `table` and `order_by` are compile-time names from this module, never input.
fn read_rows<T: DeserializeOwned>(
    connection: &Connection,
    table: &str,
    order_by: &str,
    scan_id: &str,
) -> rusqlite::Result<Vec<T>> {
    let mut statement = connection.prepare(&format!(
        "SELECT data FROM {table} WHERE scan_id = ?1 ORDER BY {order_by}"
    ))?;
    statement
        .query_map([scan_id], |row| from_json(&row.get::<_, String>(0)?))?
        .collect()
}

fn replace_rows<T: Serialize>(
    transaction: &Transaction<'_>,
    table: &str,
    scan_id: &str,
    rows: &[T],
) -> rusqlite::Result<()> {
    transaction.execute(
        &format!("DELETE FROM {table} WHERE scan_id = ?1"),
        [scan_id],
    )?;
    let mut insert = transaction.prepare(&format!(
        "INSERT INTO {table} (scan_id, position, data) VALUES (?1, ?2, ?3)"
    ))?;
    for (position, row) in rows.iter().enumerate() {
        insert.execute(params![scan_id, position as i64, to_json(row)?])?;
    }
    Ok(())
}

fn escape_like(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
