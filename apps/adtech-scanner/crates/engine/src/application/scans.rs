//! Scan use cases: start, cancel, browse, delete, export.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adscan_model::{
    BrowserInfo, DashboardStats, ExportFormat, LogEntry, LogSearchHit, ScanDetail, ScanEvent,
    ScanList, ScanListQuery, ScanOptions, ScanStatus, ScanSummary, SeverityCounts,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use super::AppError;
use super::export;
use super::pipeline::state::ScanJob;
use super::pipeline::{Pipeline, PipelineDeps};
use super::settings::SettingsService;
use crate::domain::scan::ValidatedScan;

/// Each scan drives its own browser; more than two at once starve each other.
pub const MAX_CONCURRENT_SCANS: usize = 2;
const DEFAULT_LOG_SEARCH_LIMIT: u32 = 50;
const MAX_LOG_SEARCH_LIMIT: u32 = 200;
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
pub const INTERRUPTED_MESSAGE: &str = "The app was closed while this scan was running";

struct ActiveScan {
    cancel: CancellationToken,
    /// Cancelled once the scan's final status is written.
    finished: CancellationToken,
}

type ActiveScans = Arc<Mutex<HashMap<String, ActiveScan>>>;

pub struct ScanService {
    deps: PipelineDeps,
    settings: Arc<SettingsService>,
    active: ActiveScans,
    capacity: Arc<Semaphore>,
}

impl ScanService {
    pub fn new(deps: PipelineDeps, settings: Arc<SettingsService>) -> Self {
        Self {
            deps,
            settings,
            active: ActiveScans::default(),
            capacity: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
        }
    }

    /// Scans still marked queued or running when the app starts were cut off
    /// by a crash or a forced quit.
    pub async fn recover_interrupted(&self) -> Result<u32, AppError> {
        Ok(self
            .deps
            .scans
            .fail_interrupted(INTERRUPTED_MESSAGE, self.deps.clock.now_ms())
            .await?)
    }

    pub async fn detect_browser(&self) -> Result<Option<BrowserInfo>, AppError> {
        let settings = self.settings.get().await?;
        Ok(self.deps.capture.locate_browser(settings.chrome_path).await)
    }

    pub async fn start(&self, options: ScanOptions) -> Result<ScanSummary, AppError> {
        let scan = ValidatedScan::new(options)?;
        let settings = self.settings.get().await?;
        let api_key = if scan.options.ai_enabled {
            self.settings.api_key().await
        } else {
            None
        };
        let permit = Arc::clone(&self.capacity).try_acquire_owned().ok();
        let summary = ScanSummary {
            id: self.deps.ids.new_id(),
            url: scan.url.as_str().to_string(),
            hostname: scan.url.hostname().to_string(),
            device: scan.options.device,
            status: if permit.is_some() {
                ScanStatus::Running
            } else {
                ScanStatus::Queued
            },
            started_at: self.deps.clock.now_ms(),
            finished_at: None,
            duration_ms: None,
            cost_usd: 0.0,
            finding_counts: SeverityCounts::default(),
            health_score: None,
            prebid_version: None,
            ai_used: false,
            error: None,
        };
        self.deps.scans.create(&summary, &scan.options).await?;
        self.deps.events.emit(ScanEvent::ScanUpdated {
            summary: summary.clone(),
        });

        let cancel = CancellationToken::new();
        let finished = CancellationToken::new();
        self.lock_active().insert(
            summary.id.clone(),
            ActiveScan {
                cancel: cancel.clone(),
                finished: finished.clone(),
            },
        );
        let job = ScanJob {
            summary: summary.clone(),
            scan,
            settings,
            api_key,
            cancel,
        };
        tokio::spawn(supervise(
            self.deps.clone(),
            Arc::clone(&self.capacity),
            Arc::clone(&self.active),
            permit,
            job,
            finished,
        ));
        Ok(summary)
    }

    pub async fn cancel(&self, scan_id: &str) -> Result<(), AppError> {
        if let Some(active) = self.lock_active().get(scan_id) {
            active.cancel.cancel();
            return Ok(());
        }
        // Cancelling a scan that already ended is a no-op, not an error.
        self.summary(scan_id).await.map(drop)
    }

    pub async fn list(&self, query: ScanListQuery) -> Result<ScanList, AppError> {
        Ok(self.deps.scans.list(&query).await?)
    }

    pub async fn get(&self, scan_id: &str) -> Result<ScanDetail, AppError> {
        self.deps.scans.detail(scan_id).await?.ok_or_else(|| not_found(scan_id))
    }

    pub async fn delete(&self, scan_id: &str) -> Result<(), AppError> {
        self.stop_and_wait(Some(scan_id)).await;
        if self.deps.scans.delete(scan_id).await? {
            Ok(())
        } else {
            Err(not_found(scan_id))
        }
    }

    pub async fn delete_all(&self) -> Result<(), AppError> {
        self.stop_and_wait(None).await;
        Ok(self.deps.scans.delete_all().await?)
    }

    pub async fn logs(&self, scan_id: &str, step_id: Option<&str>) -> Result<Vec<LogEntry>, AppError> {
        self.summary(scan_id).await?;
        Ok(self.deps.logs.list(scan_id, step_id).await?)
    }

    pub async fn search_logs(&self, query: &str, limit: Option<u32>) -> Result<Vec<LogSearchHit>, AppError> {
        let limit = limit.unwrap_or(DEFAULT_LOG_SEARCH_LIMIT).clamp(1, MAX_LOG_SEARCH_LIMIT);
        Ok(self.deps.logs.search(query, limit).await?)
    }

    pub async fn dashboard_stats(&self) -> Result<DashboardStats, AppError> {
        Ok(self.deps.scans.dashboard_stats().await?)
    }

    pub async fn export(&self, scan_id: &str, format: ExportFormat) -> Result<String, AppError> {
        let detail = self.get(scan_id).await?;
        match format {
            ExportFormat::Json => export::json(&detail),
            ExportFormat::Markdown => {
                let logs = self.deps.logs.list(scan_id, None).await?;
                Ok(export::markdown(&detail, &logs))
            }
        }
    }

    /// Cancels every running scan and gives them a moment to write their
    /// final status and close their browsers.
    pub async fn shutdown(&self) {
        let _ = tokio::time::timeout(SHUTDOWN_GRACE, self.stop_and_wait(None)).await;
    }

    async fn summary(&self, scan_id: &str) -> Result<ScanSummary, AppError> {
        self.deps.scans.summary(scan_id).await?.ok_or_else(|| not_found(scan_id))
    }

    async fn stop_and_wait(&self, scan_id: Option<&str>) {
        let finished: Vec<CancellationToken> = self
            .lock_active()
            .iter()
            .filter(|(id, _)| scan_id.is_none_or(|wanted| wanted == id.as_str()))
            .map(|(_, active)| {
                active.cancel.cancel();
                active.finished.clone()
            })
            .collect();
        for token in finished {
            token.cancelled().await;
        }
    }

    fn lock_active(&self) -> std::sync::MutexGuard<'_, HashMap<String, ActiveScan>> {
        self.active.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn not_found(scan_id: &str) -> AppError {
    AppError::NotFound(format!("Scan {scan_id}"))
}

/// Owns one scan from queue to final status. Whatever happens to the pipeline
/// task, the scan row ends in a terminal status.
async fn supervise(
    deps: PipelineDeps,
    capacity: Arc<Semaphore>,
    active: ActiveScans,
    permit: Option<OwnedSemaphorePermit>,
    job: ScanJob,
    finished: CancellationToken,
) {
    let scan_id = job.summary.id.clone();
    let fallback = job.summary.clone();
    let pipeline_deps = deps.clone();
    let run = tokio::spawn(async move {
        let _permit = match permit {
            Some(permit) => permit,
            None => tokio::select! {
                permit = capacity.acquire_owned() => match permit {
                    Ok(permit) => permit,
                    Err(_) => return None,
                },
                () = job.cancel.cancelled() => return None,
            },
        };
        Some(Pipeline::new(pipeline_deps).run(job).await)
    });
    match run.await {
        // The pipeline already published its final summary; writing it again
        // only matters if that write failed.
        Ok(Some(summary)) => {
            let _ = deps.scans.save_summary(&summary).await;
        }
        Ok(None) => close_without_pipeline(&deps, fallback, ScanStatus::Cancelled, None).await,
        Err(_) => {
            let stored = deps.scans.summary(&scan_id).await.ok().flatten();
            let error = "The scan stopped because of an internal error".to_string();
            close_without_pipeline(&deps, stored.unwrap_or(fallback), ScanStatus::Failed, Some(error)).await;
        }
    }
    active
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&scan_id);
    finished.cancel();
}

async fn close_without_pipeline(
    deps: &PipelineDeps,
    mut summary: ScanSummary,
    status: ScanStatus,
    error: Option<String>,
) {
    let now = deps.clock.now_ms();
    summary.status = status;
    summary.error = error;
    summary.finished_at = Some(now);
    summary.duration_ms = Some(now - summary.started_at);
    let _ = deps.scans.save_summary(&summary).await;
    deps.events.emit(ScanEvent::ScanUpdated { summary });
}
