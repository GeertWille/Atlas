//! Scan use cases: start, cancel, browse, delete, export.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adscan_model::{
    ApiProvider, BrowserInfo, DashboardStats, ExportFormat, LogEntry, LogSearchHit,
    ScanConfiguration, ScanConfigurationRef, ScanDetail, ScanEvent, ScanList, ScanListQuery,
    ScanOptions, ScanProfileInfo, ScanStatus, ScanSummary, SeverityCounts, StepStatus,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use super::AppError;
use super::export;
use super::pipeline::state::{FinishSignal, ScanJob};
use super::pipeline::{Pipeline, PipelineDeps};
use super::ports::ScanConfigurationRepository;
use super::settings::SettingsService;
use crate::domain::DomainError;
use crate::domain::scan::{ValidatedScan, seed_from_id};

/// Each scan drives its own browser; more than two at once starve each other.
pub const MAX_CONCURRENT_SCANS: usize = 2;
const DEFAULT_LOG_SEARCH_LIMIT: u32 = 50;
const MAX_LOG_SEARCH_LIMIT: u32 = 200;
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
const INTERRUPTED_MESSAGE: &str = "The app was closed while this scan was running";

struct ActiveScan {
    cancel: CancellationToken,
    finish: FinishSignal,
    /// Cancelled once the scan's final status is written.
    finished: CancellationToken,
}

type ActiveScans = Arc<Mutex<HashMap<String, ActiveScan>>>;

pub struct ScanService {
    deps: PipelineDeps,
    settings: Arc<SettingsService>,
    configurations: Arc<dyn ScanConfigurationRepository>,
    active: ActiveScans,
    capacity: Arc<Semaphore>,
    /// One browser at a time in the persistent scan profile: Chrome refuses
    /// to share a profile directory, so a second scan waits its turn.
    scan_profile: Arc<Semaphore>,
}

impl ScanService {
    pub fn new(
        deps: PipelineDeps,
        settings: Arc<SettingsService>,
        configurations: Arc<dyn ScanConfigurationRepository>,
    ) -> Self {
        Self {
            deps,
            settings,
            configurations,
            active: ActiveScans::default(),
            capacity: Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS)),
            scan_profile: Arc::new(Semaphore::new(1)),
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
        self.launch(options, None).await
    }

    /// Runs a saved configuration: its stored options, settled against the
    /// settings as they are now.
    pub async fn start_configuration(
        &self,
        configuration_id: &str,
    ) -> Result<ScanSummary, AppError> {
        let configuration = self
            .configurations
            .get(configuration_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Scan configuration {configuration_id}")))?;
        let options = configuration.options.clone();
        self.launch(options, Some(configuration)).await
    }

    async fn launch(
        &self,
        options: ScanOptions,
        configuration: Option<ScanConfiguration>,
    ) -> Result<ScanSummary, AppError> {
        let settings = self.settings.get().await?;
        let id = self.deps.ids.new_id();
        let scan = ValidatedScan::new(options, &settings, seed_from_id(&id)).map_err(|error| {
            match (&error, &configuration) {
                (DomainError::UnknownProxy, Some(configuration)) => AppError::InvalidInput(format!(
                    "The proxy of the scan configuration \"{}\" no longer exists. Edit the configuration and pick another proxy.",
                    configuration.name
                )),
                _ => error.into(),
            }
        })?;
        let proxy_password = match &scan.proxy.profile_id {
            Some(profile_id) => self.settings.proxy_password(profile_id).await?,
            None => None,
        };
        let ai_enabled = scan.options.ai_enabled;
        let api_key = if ai_enabled {
            self.settings.api_key(ApiProvider::Anthropic).await?
        } else {
            None
        };
        // Which classifier a scan uses is fixed here, when it starts.
        let jev_key = if ai_enabled && settings.jev_enabled {
            self.settings.api_key(ApiProvider::Typesafe).await?
        } else {
            None
        };
        let permit = Arc::clone(&self.capacity).try_acquire_owned().ok();
        let summary = ScanSummary {
            id,
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
            configuration: configuration.map(|configuration| ScanConfigurationRef {
                id: configuration.id,
                name: configuration.name,
            }),
            proxy: Some(scan.proxy.usage()),
        };
        self.deps.scans.create(&summary, &scan.options).await?;
        if let Some(configuration) = &summary.configuration {
            self.configurations
                .record_run(&configuration.id, &summary.id, summary.started_at)
                .await?;
        }

        // Registered before the first event so a cancel sent in response to
        // that event already finds the scan.
        let cancel = CancellationToken::new();
        let finish = FinishSignal::default();
        let finished = CancellationToken::new();
        self.lock_active().insert(
            summary.id.clone(),
            ActiveScan {
                cancel: cancel.clone(),
                finish: finish.clone(),
                finished: finished.clone(),
            },
        );
        self.deps.events.emit(ScanEvent::ScanUpdated {
            summary: summary.clone(),
        });
        let job = ScanJob {
            summary: summary.clone(),
            scan,
            settings,
            api_key,
            jev_key,
            proxy_password,
            cancel,
            finish,
            scan_profile: Arc::clone(&self.scan_profile),
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

    /// Ends a manual visit: the capture stops recording and the scan goes on
    /// with what it has. Does nothing while the scan is in any other step.
    pub async fn finish_capture(&self, scan_id: &str) -> Result<(), AppError> {
        if let Some(active) = self.lock_active().get(scan_id) {
            active.finish.request();
            return Ok(());
        }
        self.summary(scan_id).await.map(drop)
    }

    pub async fn scan_profile(&self) -> ScanProfileInfo {
        self.deps.capture.scan_profile_info().await
    }

    pub async fn clear_scan_profile(&self) -> Result<ScanProfileInfo, AppError> {
        // Held while clearing, so no scan can start a browser in the profile
        // halfway through.
        let _turn = self
            .scan_profile
            .try_acquire()
            .map_err(|_| AppError::ProfileBusy)?;
        self.deps.capture.clear_scan_profile().await?;
        Ok(self.deps.capture.scan_profile_info().await)
    }

    pub async fn list(&self, query: ScanListQuery) -> Result<ScanList, AppError> {
        Ok(self.deps.scans.list(&query).await?)
    }

    pub async fn get(&self, scan_id: &str) -> Result<ScanDetail, AppError> {
        self.deps
            .scans
            .detail(scan_id)
            .await?
            .ok_or_else(|| not_found(scan_id))
    }

    pub async fn delete(&self, scan_id: &str) -> Result<(), AppError> {
        self.stop_and_wait(Some(scan_id)).await;
        let archive = self.deps.archive.index.record(scan_id).await?;
        if !self.deps.scans.delete(scan_id).await? {
            return Err(not_found(scan_id));
        }
        // The archive's rows went with the scan. The pack goes last: a crash
        // in between leaves a file without rows, which the next start sweeps.
        if let Some(file) = archive.and_then(|archive| archive.pack_file) {
            self.deps
                .archive
                .packs
                .delete(&file)
                .await
                .map_err(|error| AppError::Storage(error.to_string()))?;
        }
        Ok(())
    }

    pub async fn delete_all(&self) -> Result<(), AppError> {
        self.stop_and_wait(None).await;
        self.deps.scans.delete_all().await?;
        self.deps.archive.remove_unreferenced_packs().await?;
        Ok(())
    }

    pub async fn logs(
        &self,
        scan_id: &str,
        step_id: Option<&str>,
    ) -> Result<Vec<LogEntry>, AppError> {
        self.summary(scan_id).await?;
        Ok(self.deps.logs.list(scan_id, step_id).await?)
    }

    pub async fn search_logs(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> Result<Vec<LogSearchHit>, AppError> {
        let limit = limit
            .unwrap_or(DEFAULT_LOG_SEARCH_LIMIT)
            .clamp(1, MAX_LOG_SEARCH_LIMIT);
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
        self.deps
            .scans
            .summary(scan_id)
            .await?
            .ok_or_else(|| not_found(scan_id))
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
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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
        Ok(None) => {
            close_without_pipeline(&deps, fallback.clone(), ScanStatus::Cancelled, None).await;
        }
        Err(_) => close_after_crash(&deps, fallback.clone()).await,
    }
    active
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&fallback.id);
    finished.cancel();
}

/// The pipeline task panicked. What it had persisted stays; the steps it left
/// open and the scan itself are closed as failed.
async fn close_after_crash(deps: &PipelineDeps, fallback: ScanSummary) {
    const CRASH_MESSAGE: &str = "The scan stopped because of an internal error";
    let stored = deps.scans.detail(&fallback.id).await.ok().flatten();
    let (summary, steps) = match stored {
        Some(detail) => (detail.summary, detail.steps),
        None => (fallback, Vec::new()),
    };
    for mut step in steps {
        match step.status {
            StepStatus::Running => {
                step.status = StepStatus::Failed;
                step.summary = Some(CRASH_MESSAGE.to_string());
            }
            StepStatus::Pending => {
                step.status = StepStatus::Skipped;
                step.summary = Some("Not run".to_string());
            }
            StepStatus::Completed | StepStatus::Failed | StepStatus::Skipped => continue,
        }
        let _ = deps.scans.save_step(&step).await;
        deps.events.emit(ScanEvent::StepUpdated { step });
    }
    close_without_pipeline(
        deps,
        summary,
        ScanStatus::Failed,
        Some(CRASH_MESSAGE.to_string()),
    )
    .await;
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
