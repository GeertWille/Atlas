//! A script's source for the viewer: the text as captured from the scan's
//! archive, or formatted on demand by the same formatter the analysis uses,
//! with the places findings point at.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use adscan_model::{ScriptRecord, ScriptSource, SourceAvailability, SourceFormat, SourceView};

use super::Archive;
use super::network::read_body;
use crate::application::AppError;
use crate::application::ports::{
    ArchiveState, FormatMethod, FormattedSource, LogRepository, ScanRepository, SourceFormatter,
};
use crate::domain::archive::index::sha256_from_hex;
use crate::domain::source;

/// A reply carries at most this much text.
const MAX_SOURCE_BYTES: usize = 8 * 1024 * 1024;
/// Formatted sources kept in memory, by total size. Formatted text is never
/// written to disk: it can be made again, and a formatter upgrade would
/// leave stale copies behind.
const FORMAT_CACHE_BYTES: usize = 64 * 1024 * 1024;

pub struct ScriptSources {
    archive: Archive,
    scans: Arc<dyn ScanRepository>,
    logs: Arc<dyn LogRepository>,
    formatter: Arc<dyn SourceFormatter>,
    formatted: Mutex<FormatCache>,
}

/// Where in the original source the viewer wants to land; 1-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OriginalPosition {
    pub line: u32,
    pub column: u32,
}

impl ScriptSources {
    pub fn new(
        archive: Archive,
        scans: Arc<dyn ScanRepository>,
        logs: Arc<dyn LogRepository>,
        formatter: Arc<dyn SourceFormatter>,
    ) -> Self {
        Self {
            archive,
            scans,
            logs,
            formatter,
            formatted: Mutex::new(FormatCache::default()),
        }
    }

    pub async fn get(
        &self,
        scan_id: &str,
        script_id: &str,
        view: SourceView,
        position: Option<OriginalPosition>,
    ) -> Result<ScriptSource, AppError> {
        let scan = self
            .scans
            .detail(scan_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Scan {scan_id}")))?;
        let script = scan
            .scripts
            .iter()
            .find(|script| script.id == script_id)
            .ok_or_else(|| AppError::NotFound(format!("Script {script_id}")))?;
        let unavailable = |availability| ScriptSource {
            scan_id: scan_id.to_string(),
            script_id: script_id.to_string(),
            view,
            availability,
            text: None,
            line_count: 0,
            format: SourceFormat::Unformatted,
            anchors: Vec::new(),
            regions: Vec::new(),
            ad_calls: Vec::new(),
            focus_line: None,
        };
        let raw = match self.raw(scan_id, script).await? {
            Ok(raw) => raw,
            Err(availability) => return Ok(unavailable(availability)),
        };
        if view == SourceView::Original {
            if raw.len() > MAX_SOURCE_BYTES {
                return Ok(unavailable(SourceAvailability::TooLarge));
            }
            let lines: Vec<String> = raw.lines().map(str::to_string).collect();
            return Ok(ScriptSource {
                line_count: lines.len() as u32,
                ad_calls: source::ad_calls(&lines),
                focus_line: position.map(|position| position.line),
                text: Some(raw),
                availability: SourceAvailability::Available,
                ..unavailable(SourceAvailability::Available)
            });
        }

        let formatted = self.format(script, &raw).await;
        let text = formatted.lines.join("\n");
        if text.len() > MAX_SOURCE_BYTES {
            return Ok(unavailable(SourceAvailability::TooLarge));
        }
        let logs = self.logs.list(scan_id, None).await?;
        let line_count = formatted.lines.len() as u32;
        Ok(ScriptSource {
            line_count,
            format: match formatted.method {
                FormatMethod::Parser => SourceFormat::Parser,
                FormatMethod::Reflow => SourceFormat::Reflow,
                FormatMethod::Unformatted => SourceFormat::Unformatted,
            },
            anchors: source::anchors(script, &scan.findings, &formatted.lines),
            regions: source::regions(script_id, &logs, line_count),
            ad_calls: source::ad_calls(&formatted.lines),
            focus_line: position.and_then(|position| {
                source::focus_line(&raw, &formatted.lines, position.line, position.column)
            }),
            text: Some(text),
            ..unavailable(SourceAvailability::Available)
        })
    }

    /// The script's source as captured, or why there is none.
    async fn raw(
        &self,
        scan_id: &str,
        script: &ScriptRecord,
    ) -> Result<Result<String, SourceAvailability>, AppError> {
        let Some(record) = self.archive.index.record(scan_id).await? else {
            return Ok(Err(SourceAvailability::NotRecorded));
        };
        let file = match (record.state, record.pack_file) {
            (ArchiveState::Disabled, _) => return Ok(Err(SourceAvailability::NotRetained)),
            (ArchiveState::Evicted, _) | (_, None) => return Ok(Err(SourceAvailability::Evicted)),
            (ArchiveState::Writing | ArchiveState::Sealed, Some(file)) => file,
        };
        let Some(sha256) = script.sha256.as_deref().and_then(sha256_from_hex) else {
            return Ok(Err(SourceAvailability::NotRecorded));
        };
        // Not in the archive although the scan has one: the scan had
        // reached its size cap when the script came.
        let Some(body) = self.archive.index.body_by_sha(scan_id, sha256).await? else {
            return Ok(Err(SourceAvailability::NotRetained));
        };
        let bytes = read_body(
            &self.archive,
            file,
            body.pack_offset,
            body.pack_length,
            body.raw_len,
        )
        .await?;
        Ok(Ok(String::from_utf8_lossy(&bytes).into_owned()))
    }

    async fn format(&self, script: &ScriptRecord, raw: &str) -> Arc<FormattedSource> {
        let key = script.sha256.clone().unwrap_or_default();
        if let Some(cached) = self.cache().get(&key) {
            return cached;
        }
        let formatted = Arc::new(self.formatter.format(raw.to_string()).await);
        self.cache().put(key, Arc::clone(&formatted));
        formatted
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, FormatCache> {
        self.formatted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Least recently used first.
#[derive(Default)]
struct FormatCache {
    entries: VecDeque<(String, Arc<FormattedSource>, usize)>,
    bytes: usize,
}

impl FormatCache {
    fn get(&mut self, key: &str) -> Option<Arc<FormattedSource>> {
        let position = self
            .entries
            .iter()
            .position(|(cached, _, _)| cached == key)?;
        let entry = self.entries.remove(position)?;
        let formatted = Arc::clone(&entry.1);
        self.entries.push_back(entry);
        Some(formatted)
    }

    fn put(&mut self, key: String, formatted: Arc<FormattedSource>) {
        let size: usize = formatted.lines.iter().map(|line| line.len() + 1).sum();
        if size > FORMAT_CACHE_BYTES {
            return;
        }
        self.entries.push_back((key, formatted, size));
        self.bytes += size;
        while self.bytes > FORMAT_CACHE_BYTES {
            match self.entries.pop_front() {
                Some((_, _, evicted)) => self.bytes -= evicted,
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn formatted(bytes: usize) -> Arc<FormattedSource> {
        Arc::new(FormattedSource {
            lines: vec!["x".repeat(bytes - 1)],
            method: FormatMethod::Parser,
        })
    }

    #[test]
    fn the_cache_drops_the_least_recently_used_source_when_it_is_full() {
        let mut cache = FormatCache::default();
        let third = FORMAT_CACHE_BYTES / 3;
        cache.put("a".into(), formatted(third));
        cache.put("b".into(), formatted(third));
        cache.put("c".into(), formatted(third));
        assert!(cache.get("a").is_some(), "a is now the most recently used");
        cache.put("d".into(), formatted(third));
        assert!(cache.get("b").is_none(), "b was the oldest");
        assert!(cache.get("a").is_some() && cache.get("c").is_some() && cache.get("d").is_some());
        assert!(cache.bytes <= FORMAT_CACHE_BYTES);
    }

    #[test]
    fn a_source_larger_than_the_cache_is_not_cached() {
        let mut cache = FormatCache::default();
        cache.put("huge".into(), formatted(FORMAT_CACHE_BYTES + 2));
        assert!(cache.get("huge").is_none());
        assert_eq!(cache.bytes, 0);
    }
}
