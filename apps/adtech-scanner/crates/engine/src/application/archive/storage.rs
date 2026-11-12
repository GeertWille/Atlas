//! Keeping the archives within their limit, and removing them.

use std::collections::HashSet;

use adscan_model::ArchiveStorage;

use super::Archive;
use crate::application::AppError;
use crate::application::ports::{ArchiveState, PackError};

impl Archive {
    pub async fn storage(&self, limit_bytes: f64) -> Result<ArchiveStorage, AppError> {
        Ok(self.index.storage(limit_bytes).await?)
    }

    /// Removes a scan's archive altogether: rows first, so that a crash in
    /// between leaves a file without rows, which the next start sweeps up,
    /// and never rows without their bytes.
    pub async fn remove(&self, scan_id: &str) -> Result<(), AppError> {
        let record = self.index.record(scan_id).await?;
        self.index.remove(scan_id).await?;
        if let Some(file) = record.and_then(|record| record.pack_file) {
            self.packs.delete(&file).await.map_err(pack_error)?;
        }
        Ok(())
    }

    /// Deletes packs nothing refers to any more: those of deleted scans.
    pub async fn remove_unreferenced_packs(&self) -> Result<u32, AppError> {
        let referenced: HashSet<String> = self
            .index
            .all()
            .await?
            .into_iter()
            .filter_map(|record| record.pack_file)
            .collect();
        let mut removed = 0;
        for pack in self.packs.list().await.map_err(pack_error)? {
            if !referenced.contains(&pack.file) {
                self.packs.delete(&pack.file).await.map_err(pack_error)?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// While all packs together exceed `limit_bytes`, the oldest sealed
    /// archive loses its pack; its request rows stay. The archive of
    /// `keep_scan`, the scan just written, is never taken. Returns how many
    /// were evicted.
    pub async fn enforce_limit(&self, limit_bytes: f64, keep_scan: Option<&str>) -> Result<u32, AppError> {
        let archives = self.index.all().await?;
        let mut total: f64 = archives.iter().map(|record| record.pack_bytes as f64).sum();
        let mut evicted = 0;
        for record in archives {
            if total <= limit_bytes {
                break;
            }
            if record.state != ArchiveState::Sealed || Some(record.scan_id.as_str()) == keep_scan {
                continue;
            }
            self.evict(&record.scan_id, record.pack_file.as_deref(), true).await?;
            total -= record.pack_bytes as f64;
            evicted += 1;
        }
        Ok(evicted)
    }

    /// "Clear stored scan data": every finished scan loses its network data,
    /// rows included. A scan that is still capturing keeps writing.
    pub async fn clear(&self) -> Result<(), AppError> {
        for record in self.index.all().await? {
            if matches!(record.state, ArchiveState::Sealed | ArchiveState::Evicted) {
                self.evict(&record.scan_id, record.pack_file.as_deref(), false).await?;
            }
        }
        self.index.compact().await?;
        Ok(())
    }

    async fn evict(&self, scan_id: &str, pack_file: Option<&str>, keep_rows: bool) -> Result<(), AppError> {
        self.index.mark_evicted(scan_id, keep_rows).await?;
        if let Some(file) = pack_file {
            self.packs.delete(file).await.map_err(pack_error)?;
        }
        Ok(())
    }

    /// Run once when the app starts, after interrupted scans were marked
    /// failed. An archive still being written belongs to a scan that was cut
    /// off: it is removed, since rows may be missing for bytes on disk. A
    /// pack without an archive is what a crash between deleting rows and
    /// deleting the file leaves. A sealed archive whose pack is gone (the
    /// user removed it) is evicted.
    pub async fn sweep(&self) -> Result<SweepReport, AppError> {
        let mut report = SweepReport::default();
        let on_disk: HashSet<String> = self
            .packs
            .list()
            .await
            .map_err(pack_error)?
            .into_iter()
            .map(|pack| pack.file)
            .collect();
        for record in self.index.all().await? {
            match (record.state, &record.pack_file) {
                (ArchiveState::Writing, _) => {
                    self.remove(&record.scan_id).await?;
                    report.unfinished += 1;
                }
                (ArchiveState::Sealed, Some(file)) if !on_disk.contains(file) => {
                    self.index.mark_evicted(&record.scan_id, true).await?;
                    report.missing += 1;
                }
                _ => {}
            }
        }
        report.orphans = self.remove_unreferenced_packs().await?;
        Ok(report)
    }
}

/// What the start-up sweep cleaned up.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SweepReport {
    /// Archives of scans that were cut off while capturing.
    pub unfinished: u32,
    /// Pack files no archive refers to.
    pub orphans: u32,
    /// Sealed archives whose pack file was gone.
    pub missing: u32,
}

pub(super) fn pack_error(error: PackError) -> AppError {
    AppError::Storage(error.to_string())
}
