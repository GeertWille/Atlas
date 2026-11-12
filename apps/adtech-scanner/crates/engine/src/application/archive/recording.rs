//! Takes the visit's requests from the capture while it runs.
//!
//! The capture hands every request to [`RecordingSink`], which never blocks:
//! it decides which bodies are read, accounts for their bytes and queues the
//! record. One task takes the queue. Each record first passes the analysis
//! ([`RecordAnalysis`]), with its bodies still in memory; then, when the scan
//! stores network data, it is written: bodies into frames of the scan's
//! pack, request documents batched into record frames, and the index rows
//! once the bytes they point at are durable.
//!
//! Reading a body and storing it are separate decisions. A bid or ad-server
//! call is always read, so that the analysis works with storage off and past
//! the scan's storage cap; whether it is also stored follows the same rules
//! as every other body.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use adscan_model::{
    BodyAdmission, BodyOffer, BodySkip, BodyState, CapturedBody, NetworkRecord, NetworkSink,
};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::Archive;
use super::analysis::RecordAnalysis;
use super::codec;
use crate::application::ports::{
    ArchiveRepository, ArchiveTotals, BodyIndexRow, PackWriter, RequestIndexRow,
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

/// What a recording is started with.
pub struct RecordingStart {
    pub scan_id: String,
    pub url: String,
    /// Host of the scanned page: what "first party" is measured against.
    pub page_host: String,
    /// Unix epoch ms.
    pub started_at: f64,
    pub full_headers: bool,
}

/// What becomes of a body that was admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Keep {
    Store,
    /// Read for the analysis only; the record says why it was not stored.
    AnalyseOnly(BodyState),
}

enum Item {
    Record(Box<NetworkRecord>, Keep),
    /// A script source of the capture; inline and runtime code has no
    /// response to come with.
    Script(String),
    Finish,
}

#[derive(Default)]
struct Budget {
    /// Body bytes admitted or queued and not yet dealt with.
    in_flight: u64,
    /// Body bytes this scan stores, before compression.
    kept: u64,
    /// What an admission set aside until its record arrives, by `seq`.
    reserved: HashMap<u32, (u64, Keep)>,
    /// A body was not stored for lack of room.
    truncated: bool,
}

/// The capture's side of a recording.
pub struct RecordingSink {
    /// Whether this scan stores network data at all.
    stores: bool,
    budget: Mutex<Budget>,
    queue: mpsc::UnboundedSender<Item>,
}

impl RecordingSink {
    fn budget(&self) -> std::sync::MutexGuard<'_, Budget> {
        self.budget.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl NetworkSink for RecordingSink {
    fn admit_body(&self, offer: &BodyOffer<'_>) -> BodyAdmission {
        let analysed = RecordAnalysis::wants_body(offer.url, offer.bidder);
        let storable = self.stores && kept_kind(offer.kind, offer.mime_type).is_some();
        if !analysed && !storable {
            return BodyAdmission::Skip(if self.stores {
                BodySkip::Type
            } else {
                BodySkip::NotStored
            });
        }
        let reservation = match offer.expected_bytes {
            0 => UNKNOWN_BODY_RESERVATION,
            expected => expected.min(MAX_BODY_BYTES as u64),
        };
        let mut budget = self.budget();
        let mut keep = if storable {
            Keep::Store
        } else {
            Keep::AnalyseOnly(BodyState::NotKeptType)
        };
        if storable && budget.kept + reservation > MAX_SCAN_BODY_BYTES {
            budget.truncated = true;
            if !analysed {
                return BodyAdmission::Skip(BodySkip::ScanLimit);
            }
            keep = Keep::AnalyseOnly(BodyState::ScanLimit);
        }
        if budget.in_flight + reservation > MAX_IN_FLIGHT_BODY_BYTES {
            budget.truncated |= storable;
            return BodyAdmission::Skip(BodySkip::Backlog);
        }
        budget.in_flight += reservation;
        if keep == Keep::Store {
            budget.kept += reservation;
        }
        budget.reserved.insert(offer.seq, (reservation, keep));
        BodyAdmission::Read {
            max_bytes: MAX_BODY_BYTES,
        }
    }

    fn record(&self, record: NetworkRecord) {
        let actual = match &record.body {
            CapturedBody::Read { bytes, .. } => bytes.len() as u64,
            _ => 0,
        };
        let keep = {
            let mut budget = self.budget();
            let (reserved, keep) = budget
                .reserved
                .remove(&record.seq)
                .unwrap_or((0, Keep::Store));
            // From here on the body counts with its real size: in the queue
            // until the recorder is done with it, and against the scan's cap.
            budget.in_flight = budget.in_flight.saturating_sub(reserved) + actual;
            if keep == Keep::Store {
                budget.kept = budget.kept.saturating_sub(reserved) + actual;
            }
            keep
        };
        // A closed queue means the recorder is gone; the scan goes on.
        let _ = self.queue.send(Item::Record(Box::new(record), keep));
    }
}

/// What a recording leaves behind.
pub struct Recorded {
    pub analysis: RecordAnalysis,
    /// `None` when the scan stores no network data; an error when storing
    /// failed (the analysis is complete all the same).
    pub stored: Option<Result<ArchiveTotals, String>>,
}

/// A capture's requests on their way to the analysis and the archive.
pub struct Recording {
    sink: Arc<RecordingSink>,
    recorder: JoinHandle<Recorded>,
}

impl Recording {
    /// Starts taking records. With `archive`, creates the scan's pack and
    /// registers it; without, nothing is written anywhere.
    pub async fn start(archive: Option<&Archive>, start: RecordingStart) -> Result<Self, String> {
        let store = match archive {
            Some(archive) => {
                let file = pack_file(&start.scan_id);
                let pack = archive
                    .packs
                    .create(&file)
                    .await
                    .map_err(|error| error.to_string())?;
                archive
                    .index
                    .begin(&start.scan_id, Some(&file), !start.full_headers, start.started_at)
                    .await
                    .map_err(|error| error.0)?;
                Some(Store {
                    pack,
                    index: Arc::clone(&archive.index),
                    bodies: HashMap::new(),
                    manifest_bodies: Vec::new(),
                    manifest_records: Vec::new(),
                    pending_bodies: Vec::new(),
                    pending_rows: Vec::new(),
                    lines: Vec::new(),
                    lines_bytes: 0,
                    request_count: 0,
                })
            }
            None => None,
        };
        let (queue, items) = mpsc::unbounded_channel();
        let sink = Arc::new(RecordingSink {
            stores: store.is_some(),
            budget: Mutex::new(Budget::default()),
            queue,
        });
        let recorder = Recorder {
            page_site: site_of(&start.page_host),
            start,
            sink: Arc::clone(&sink),
            analysis: RecordAnalysis::default(),
            store,
            store_error: None,
        };
        Ok(Self {
            sink,
            recorder: tokio::spawn(recorder.run(items)),
        })
    }

    pub fn sink(&self) -> Arc<dyn NetworkSink> {
        Arc::clone(&self.sink) as Arc<dyn NetworkSink>
    }

    /// Adds a script source of the capture to the archive. Content the
    /// archive already has (the script's own response) is not stored twice.
    pub fn store_script(&self, body: &str) {
        if self.sink.stores {
            let _ = self.sink.queue.send(Item::Script(body.to_string()));
        }
    }

    /// Works off what is queued and, when storing, writes the manifest and
    /// the trailer and seals the pack.
    pub async fn finish(self) -> Result<Recorded, String> {
        let _ = self.sink.queue.send(Item::Finish);
        self.recorder
            .await
            .map_err(|error| format!("the recorder stopped: {error}"))
    }

    /// Stops at once. What is on disk is removed by the caller.
    pub fn abort(self) {
        self.recorder.abort();
    }
}

struct StoredFrame {
    body_no: u32,
    stored: StoredBody,
}

/// The archive being written.
struct Store {
    pack: Box<dyn PackWriter>,
    index: Arc<dyn ArchiveRepository>,
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
}

struct Recorder {
    start: RecordingStart,
    page_site: String,
    sink: Arc<RecordingSink>,
    analysis: RecordAnalysis,
    /// `None` when the scan stores nothing, and after storing failed.
    store: Option<Store>,
    store_error: Option<String>,
}

impl Recorder {
    async fn run(mut self, mut items: mpsc::UnboundedReceiver<Item>) -> Recorded {
        let mut flush = tokio::time::interval(FLUSH_INTERVAL);
        flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let written = tokio::select! {
                item = items.recv() => match item {
                    Some(Item::Record(record, keep)) => self.take(*record, keep).await,
                    Some(Item::Script(body)) => self.take_script(body).await,
                    Some(Item::Finish) | None => break,
                },
                _ = flush.tick() => match &mut self.store {
                    Some(store) => store.flush(&self.start.scan_id).await,
                    None => Ok(()),
                },
            };
            if let Err(error) = written {
                // Storing is over for this scan; the analysis goes on.
                self.store = None;
                self.store_error = Some(error);
            }
        }
        let stored = match (self.store, self.store_error) {
            (_, Some(error)) => Some(Err(error)),
            (Some(store), None) => {
                let truncated = self.sink.budget().truncated;
                Some(store.seal(&self.start, truncated).await)
            }
            (None, None) => None,
        };
        Recorded {
            analysis: self.analysis,
            stored,
        }
    }

    async fn take(&mut self, mut record: NetworkRecord, keep: Keep) -> Result<(), String> {
        let state = state_without_bytes(&record.body, &record.end);
        let mut row = RequestIndexRow::of(&record, &self.page_site, state, None);
        self.analysis.observe(&record, &row);

        let captured = std::mem::replace(&mut record.body, CapturedBody::Absent);
        let queued = match &captured {
            CapturedBody::Read { bytes, .. } => bytes.len() as u64,
            _ => 0,
        };
        let written = match &mut self.store {
            Some(store) => {
                store
                    .write(&self.start, &record, captured, keep, &mut row)
                    .await
            }
            None => Ok(()),
        };
        let mut budget = self.sink.budget();
        budget.in_flight = budget.in_flight.saturating_sub(queued);
        written
    }

    async fn take_script(&mut self, body: String) -> Result<(), String> {
        let Some(store) = &mut self.store else {
            return Ok(());
        };
        let bytes = body.into_bytes();
        let length = bytes.len() as u64;
        {
            let mut budget = self.sink.budget();
            if budget.kept + length > MAX_SCAN_BODY_BYTES {
                budget.truncated = true;
                return Ok(());
            }
        }
        let known_before = store.bodies.len();
        let stored = store
            .body(bytes, length, BodyKind::Js)
            .await?
            .stored
            .stored_bytes;
        if store.bodies.len() > known_before {
            self.sink.budget().kept += stored;
        }
        Ok(())
    }
}

impl Store {
    async fn write(
        &mut self,
        start: &RecordingStart,
        record: &NetworkRecord,
        captured: CapturedBody,
        keep: Keep,
        row: &mut RequestIndexRow,
    ) -> Result<(), String> {
        let mut stored = None;
        match (captured, keep) {
            (CapturedBody::Read { bytes, total_bytes }, Keep::Store) if !bytes.is_empty() => {
                // The sink admitted this kind; a body that arrives unasked
                // is kept as text rather than lost.
                let kind =
                    kept_kind(record.kind, record.mime_type.as_deref()).unwrap_or(BodyKind::Text);
                let frame = self.body(bytes, total_bytes, kind).await?;
                row.body_state = if frame.stored.truncated {
                    BodyState::Truncated
                } else {
                    BodyState::Kept
                };
                row.body_no = Some(frame.body_no);
                stored = Some(StoredBody {
                    // The same content can arrive cut for one request and
                    // whole for another; each says what it received.
                    bytes: total_bytes,
                    ..frame.stored.clone()
                });
            }
            (CapturedBody::Read { bytes, .. }, Keep::AnalyseOnly(why)) if !bytes.is_empty() => {
                row.body_state = why;
            }
            _ => {}
        }
        let document =
            RequestDocument::of(&start.scan_id, record, start.full_headers, row.body_state, stored);
        let line = serde_json::to_string(&document).map_err(|error| error.to_string())?;
        row.record_index = self.lines.len() as u32;
        self.lines_bytes += line.len() + 1;
        self.lines.push(line);
        self.pending_rows.push(row.clone());
        self.request_count += 1;
        if self.lines.len() >= RECORDS_PER_FRAME || self.lines_bytes >= MAX_RECORD_FRAME_BYTES {
            self.flush(&start.scan_id).await?;
        }
        Ok(())
    }

    /// The frame of this content, written now unless the scan has it already.
    async fn body(
        &mut self,
        mut bytes: Vec<u8>,
        total_bytes: u64,
        kind: BodyKind,
    ) -> Result<&StoredFrame, String> {
        bytes.truncate(MAX_BODY_BYTES);
        let truncated = (bytes.len() as u64) < total_bytes;
        let raw_length = bytes.len() as u64;
        // Hashing and compressing megabytes is not for an async worker.
        let (sha256, bytes) = tokio::task::spawn_blocking(move || {
            let sha256: [u8; 32] = Sha256::digest(&bytes).into();
            (sha256, bytes)
        })
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

    /// Closes the record frame, makes the pack durable up to here, and only
    /// then adds the rows that point into it.
    async fn flush(&mut self, scan_id: &str) -> Result<(), String> {
        if self.lines.is_empty() && self.pending_bodies.is_empty() {
            return Ok(());
        }
        if !self.lines.is_empty() {
            let content = std::mem::take(&mut self.lines).join("\n");
            self.lines_bytes = 0;
            let compressed =
                tokio::task::spawn_blocking(move || codec::compress(content.as_bytes()))
                    .await
                    .map_err(|error| error.to_string())?
                    .map_err(|error| error.to_string())?;
            let location = self
                .pack
                .append(compressed)
                .await
                .map_err(|error| error.to_string())?;
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
                scan_id,
                std::mem::take(&mut self.pending_bodies),
                std::mem::take(&mut self.pending_rows),
            )
            .await
            .map_err(|error| error.0)
    }

    async fn seal(mut self, start: &RecordingStart, truncated: bool) -> Result<ArchiveTotals, String> {
        self.flush(&start.scan_id).await?;
        let manifest = Manifest {
            format: FORMAT_VERSION,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            scan_id: start.scan_id.clone(),
            url: start.url.clone(),
            started_at: start.started_at,
            headers_redacted: !start.full_headers,
            request_count: self.request_count,
            bodies: std::mem::take(&mut self.manifest_bodies),
            records: std::mem::take(&mut self.manifest_records),
        };
        let manifest = serde_json::to_vec(&manifest).map_err(|error| error.to_string())?;
        let frame = codec::compress(&manifest).map_err(|error| error.to_string())?;
        let location = self
            .pack
            .append(frame)
            .await
            .map_err(|error| error.to_string())?;
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
            .seal(&start.scan_id, totals)
            .await
            .map_err(|error| error.0)?;
        Ok(totals)
    }
}
