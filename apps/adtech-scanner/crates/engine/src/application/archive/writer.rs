//! Writes one scan's archive while its capture runs.
//!
//! The capture hands every request to [`ArchiveSink`], which never blocks:
//! it only accounts for bytes and queues the record. One task takes the
//! queue, compresses bodies into frames of the scan's pack, batches the
//! request documents into record frames, and adds the index rows once the
//! bytes they point at are durable.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use adscan_model::{
    BodyAdmission, BodyOffer, BodySkip, BodyState, CapturedBody, NetworkRecord, NetworkSink,
};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::codec;
use crate::application::ports::{
    ArchiveRepository, ArchiveTotals, BodyIndexRow, PackStore, PackWriter, RequestIndexRow,
};
use crate::domain::archive::document::{RequestDocument, StoredBody, state_without_bytes};
use crate::domain::archive::index::{sha256_hex, site_of};
use crate::domain::archive::kinds::{
    BodyKind, MAX_BODY_BYTES, MAX_IN_FLIGHT_BODY_BYTES, MAX_SCAN_BODY_BYTES,
    UNKNOWN_BODY_RESERVATION, kept_kind,
};
use crate::domain::archive::location::pack_file;
use crate::domain::archive::pack::{
    FORMAT_VERSION, MAX_RECORD_FRAME_BYTES, Manifest, ManifestBody, ManifestRecords,
    RECORDS_PER_FRAME, trailer,
};

/// Rows reach the index at the latest this long after their request ended.
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// What a scan's archive is started with.
pub struct ArchiveStart {
    pub scan_id: String,
    pub url: String,
    /// Host of the scanned page: what "first party" is measured against.
    pub page_host: String,
    /// Unix epoch ms.
    pub started_at: f64,
    pub full_headers: bool,
}

enum Item {
    Record(Box<NetworkRecord>),
    /// A script source of the capture; inline and runtime code has no
    /// response to come with.
    Script(String),
    Finish,
}

#[derive(Default)]
struct Budget {
    /// Body bytes admitted or queued and not yet written.
    in_flight: u64,
    /// Body bytes this scan keeps, before compression.
    kept: u64,
    /// What an admission set aside until its record arrives, by `seq`.
    reserved: HashMap<u32, u64>,
    /// A body was left out for lack of room.
    truncated: bool,
}

/// The capture's side of the archive.
pub struct ArchiveSink {
    budget: Mutex<Budget>,
    queue: mpsc::UnboundedSender<Item>,
}

impl ArchiveSink {
    fn budget(&self) -> std::sync::MutexGuard<'_, Budget> {
        self.budget.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl NetworkSink for ArchiveSink {
    fn admit_body(&self, offer: &BodyOffer<'_>) -> BodyAdmission {
        if kept_kind(offer.kind, offer.mime_type).is_none() {
            return BodyAdmission::Skip(BodySkip::Type);
        }
        let reservation = match offer.expected_bytes {
            0 => UNKNOWN_BODY_RESERVATION,
            expected => expected.min(MAX_BODY_BYTES as u64),
        };
        let mut budget = self.budget();
        if budget.kept + reservation > MAX_SCAN_BODY_BYTES {
            budget.truncated = true;
            return BodyAdmission::Skip(BodySkip::ScanLimit);
        }
        if budget.in_flight + reservation > MAX_IN_FLIGHT_BODY_BYTES {
            budget.truncated = true;
            return BodyAdmission::Skip(BodySkip::Backlog);
        }
        budget.in_flight += reservation;
        budget.kept += reservation;
        budget.reserved.insert(offer.seq, reservation);
        BodyAdmission::Read {
            max_bytes: MAX_BODY_BYTES,
        }
    }

    fn record(&self, record: NetworkRecord) {
        let actual = match &record.body {
            CapturedBody::Read { bytes, .. } => bytes.len() as u64,
            _ => 0,
        };
        {
            let mut budget = self.budget();
            let reserved = budget.reserved.remove(&record.seq).unwrap_or(0);
            // From here on the body counts with its real size: in the queue
            // until the writer has it on disk, and against the scan's cap.
            budget.in_flight = budget.in_flight.saturating_sub(reserved) + actual;
            budget.kept = budget.kept.saturating_sub(reserved) + actual;
        }
        // A closed queue means the writer failed; the scan goes on without.
        let _ = self.queue.send(Item::Record(Box::new(record)));
    }
}

/// A scan's archive while it is being written.
pub struct ArchiveSession {
    sink: Arc<ArchiveSink>,
    writer: JoinHandle<Result<ArchiveTotals, String>>,
}

impl ArchiveSession {
    /// Creates the pack and registers the archive; from here on the sink
    /// accepts records.
    pub async fn start(
        packs: &Arc<dyn PackStore>,
        index: &Arc<dyn ArchiveRepository>,
        start: ArchiveStart,
    ) -> Result<Self, String> {
        let file = pack_file(&start.scan_id);
        let pack = packs.create(&file).await.map_err(|error| error.to_string())?;
        index
            .begin(&start.scan_id, Some(&file), !start.full_headers, start.started_at)
            .await
            .map_err(|error| error.0)?;
        let (queue, items) = mpsc::unbounded_channel();
        let sink = Arc::new(ArchiveSink {
            budget: Mutex::new(Budget::default()),
            queue,
        });
        let writer = Writer {
            page_site: site_of(&start.page_host),
            start,
            pack,
            index: Arc::clone(index),
            sink: Arc::clone(&sink),
            bodies: HashMap::new(),
            manifest_bodies: Vec::new(),
            manifest_records: Vec::new(),
            pending_bodies: Vec::new(),
            pending_rows: Vec::new(),
            lines: Vec::new(),
            lines_bytes: 0,
            request_count: 0,
            pack_bytes: 0,
        };
        Ok(Self {
            sink,
            writer: tokio::spawn(writer.run(items)),
        })
    }

    pub fn sink(&self) -> Arc<dyn NetworkSink> {
        Arc::clone(&self.sink) as Arc<dyn NetworkSink>
    }

    /// Adds a script source of the capture. A body the archive already has
    /// (the script's own response) is not stored twice.
    pub fn store_script(&self, body: &str) {
        let _ = self.sink.queue.send(Item::Script(body.to_string()));
    }

    /// Writes what is queued, the manifest and the trailer, and seals the
    /// pack.
    pub async fn finish(self) -> Result<ArchiveTotals, String> {
        // Not sent when the writer has already failed; its error is below.
        let _ = self.sink.queue.send(Item::Finish);
        self.writer
            .await
            .map_err(|error| format!("the archive writer stopped: {error}"))?
    }

    /// Stops writing. What is on disk is removed by the caller.
    pub fn abort(self) {
        self.writer.abort();
    }
}

struct StoredFrame {
    body_no: u32,
    stored: StoredBody,
}

struct Writer {
    start: ArchiveStart,
    page_site: String,
    pack: Box<dyn PackWriter>,
    index: Arc<dyn ArchiveRepository>,
    sink: Arc<ArchiveSink>,
    /// Bodies this scan already has, by sha-256.
    bodies: HashMap<[u8; 32], StoredFrame>,
    manifest_bodies: Vec<ManifestBody>,
    manifest_records: Vec<ManifestRecords>,
    /// Rows whose frames are appended but not yet synced.
    pending_bodies: Vec<BodyIndexRow>,
    pending_rows: Vec<RequestIndexRow>,
    /// The record frame being filled: one JSON document per line.
    lines: Vec<String>,
    lines_bytes: usize,
    request_count: u32,
    pack_bytes: u64,
}

impl Writer {
    async fn run(mut self, mut items: mpsc::UnboundedReceiver<Item>) -> Result<ArchiveTotals, String> {
        let mut flush = tokio::time::interval(FLUSH_INTERVAL);
        flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                item = items.recv() => match item {
                    Some(Item::Record(record)) => self.write_record(*record).await?,
                    Some(Item::Script(body)) => self.write_script(body).await?,
                    Some(Item::Finish) | None => break,
                },
                _ = flush.tick() => self.flush().await?,
            }
        }
        self.seal().await
    }

    async fn write_record(&mut self, mut record: NetworkRecord) -> Result<(), String> {
        let captured = std::mem::replace(&mut record.body, CapturedBody::Absent);
        let mut state = state_without_bytes(&captured, &record.end);
        let mut stored = None;
        let mut body_no = None;
        if let CapturedBody::Read { bytes, total_bytes } = captured
            && !bytes.is_empty()
        {
            let queued = bytes.len() as u64;
            // The sink admitted this kind; a body that arrives unasked is
            // kept as text rather than lost.
            let kind = kept_kind(record.kind, record.mime_type.as_deref()).unwrap_or(BodyKind::Text);
            let frame = self.store_body(bytes, total_bytes, kind).await;
            self.release(queued);
            let frame = frame?;
            state = if frame.stored.truncated {
                BodyState::Truncated
            } else {
                BodyState::Kept
            };
            body_no = Some(frame.body_no);
            stored = Some(StoredBody {
                // The same content can arrive cut for one request and whole
                // for another; each says what it received.
                bytes: total_bytes,
                ..frame.stored.clone()
            });
        }
        let document = RequestDocument::of(&self.start.scan_id, &record, self.start.full_headers, state, stored);
        let line = serde_json::to_string(&document).map_err(|error| error.to_string())?;
        let mut row = RequestIndexRow::of(&record, &self.page_site, state, body_no);
        row.record_index = self.lines.len() as u32;
        self.lines_bytes += line.len() + 1;
        self.lines.push(line);
        self.pending_rows.push(row);
        self.request_count += 1;
        if self.lines.len() >= RECORDS_PER_FRAME || self.lines_bytes >= MAX_RECORD_FRAME_BYTES {
            self.flush().await?;
        }
        Ok(())
    }

    async fn write_script(&mut self, body: String) -> Result<(), String> {
        let bytes = body.into_bytes();
        let length = bytes.len() as u64;
        {
            let mut budget = self.sink.budget();
            if budget.kept + length > MAX_SCAN_BODY_BYTES {
                budget.truncated = true;
                return Ok(());
            }
        }
        let known_before = self.bodies.len();
        let stored = self.store_body(bytes, length, BodyKind::Js).await?.stored.stored_bytes;
        if self.bodies.len() > known_before {
            self.sink.budget().kept += stored;
        }
        Ok(())
    }

    /// The frame of this content, written now unless the scan has it already.
    async fn store_body(
        &mut self,
        mut bytes: Vec<u8>,
        total_bytes: u64,
        kind: BodyKind,
    ) -> Result<&StoredFrame, String> {
        bytes.truncate(MAX_BODY_BYTES);
        let truncated = (bytes.len() as u64) < total_bytes;
        let raw_length = bytes.len() as u64;
        // Hashing and compressing megabytes is not for an async worker.
        let (sha256, bytes) = tokio::task::spawn_blocking(move || (sha256_of(&bytes), bytes))
            .await
            .map_err(|error| error.to_string())?;
        if !self.bodies.contains_key(&sha256) {
            let compressed = tokio::task::spawn_blocking(move || codec::compress(&bytes))
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())?;
            let location = self
                .pack
                .append(compressed)
                .await
                .map_err(|error| error.to_string())?;
            self.pack_bytes = location.offset + u64::from(location.length);
            let body_no = self.bodies.len() as u32;
            let hex = sha256_hex(&sha256);
            self.pending_bodies.push(BodyIndexRow {
                body_no,
                sha256,
                kind,
                raw_len: raw_length,
                pack_offset: location.offset,
                pack_length: location.length,
                truncated,
            });
            self.manifest_bodies.push(ManifestBody {
                sha256: hex.clone(),
                offset: location.offset,
                length: location.length,
                raw_length,
                kind: kind.name().to_string(),
                truncated,
            });
            self.bodies.insert(
                sha256,
                StoredFrame {
                    body_no,
                    stored: StoredBody {
                        sha256: hex,
                        location: location.compose(),
                        bytes: total_bytes,
                        stored_bytes: raw_length,
                        kind: kind.name().to_string(),
                        truncated,
                    },
                },
            );
        }
        Ok(&self.bodies[&sha256])
    }

    fn release(&self, queued: u64) {
        let mut budget = self.sink.budget();
        budget.in_flight = budget.in_flight.saturating_sub(queued);
    }

    /// Closes the record frame, makes the pack durable up to here, and only
    /// then adds the rows that point into it.
    async fn flush(&mut self) -> Result<(), String> {
        if self.lines.is_empty() && self.pending_bodies.is_empty() {
            return Ok(());
        }
        if !self.lines.is_empty() {
            let content = std::mem::take(&mut self.lines).join("\n");
            self.lines_bytes = 0;
            let compressed = tokio::task::spawn_blocking(move || codec::compress(content.as_bytes()))
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())?;
            let location = self
                .pack
                .append(compressed)
                .await
                .map_err(|error| error.to_string())?;
            self.pack_bytes = location.offset + u64::from(location.length);
            for row in &mut self.pending_rows {
                row.record_offset = location.offset;
                row.record_length = location.length;
            }
            self.manifest_records.push(ManifestRecords {
                offset: location.offset,
                length: location.length,
                count: self.pending_rows.len() as u32,
            });
        }
        self.pack.sync().await.map_err(|error| error.to_string())?;
        self.index
            .append(
                &self.start.scan_id,
                std::mem::take(&mut self.pending_bodies),
                std::mem::take(&mut self.pending_rows),
            )
            .await
            .map_err(|error| error.0)
    }

    async fn seal(mut self) -> Result<ArchiveTotals, String> {
        self.flush().await?;
        let truncated = self.sink.budget().truncated;
        let manifest = Manifest {
            format: FORMAT_VERSION,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            scan_id: self.start.scan_id.clone(),
            url: self.start.url.clone(),
            started_at: self.start.started_at,
            headers_redacted: !self.start.full_headers,
            request_count: self.request_count,
            bodies: std::mem::take(&mut self.manifest_bodies),
            records: std::mem::take(&mut self.manifest_records),
        };
        let manifest = serde_json::to_vec(&manifest).map_err(|error| error.to_string())?;
        let frame = codec::compress(&manifest).map_err(|error| error.to_string())?;
        let location = self.pack.append(frame).await.map_err(|error| error.to_string())?;
        self.pack
            .append(trailer(location.offset, location.length).to_vec())
            .await
            .map_err(|error| error.to_string())?;
        let sealed = self.pack.seal().await.map_err(|error| error.to_string())?;
        let totals = ArchiveTotals {
            pack_bytes: sealed.bytes,
            truncated,
        };
        self.index
            .seal(&self.start.scan_id, totals)
            .await
            .map_err(|error| error.0)?;
        Ok(totals)
    }
}

fn sha256_of(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
