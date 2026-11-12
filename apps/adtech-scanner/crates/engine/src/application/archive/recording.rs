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
use crate::application::ports::{
    ArchiveRepository, ArchiveTotals, BodyIndexRow, PackWriter, RequestIndexRow,
};
use crate::domain::archive::codec;
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
use crate::domain::archive::redact::{stored_json, stored_url};

/// Rows reach the index at the latest this long after their request ended.
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// What a recording is started with.
#[derive(Clone)]
pub struct RecordingStart {
    pub scan_id: String,
    pub url: String,
    /// Host of the address the scan was started with: what "first party" is
    /// measured against for a request the capture put on no page view.
    pub scan_host: String,
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

/// What an admission set aside until the body's record arrives.
#[derive(Debug, Clone, Copy)]
struct Reservation {
    in_flight: u64,
    kept: u64,
    keep: Keep,
}

#[derive(Default)]
struct Budget {
    /// Body bytes admitted or queued and not yet dealt with.
    in_flight: u64,
    /// Body bytes this scan stores, before compression.
    kept: u64,
    reserved: HashMap<u32, Reservation>,
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
        // The browser hands a body over whole and it is cut to the cap
        // here, so all of it is in memory on the way: that is what the
        // in-flight budget counts. The scan's cap counts what is kept.
        let arriving = match offer.expected_bytes {
            0 => UNKNOWN_BODY_RESERVATION,
            expected => expected,
        };
        let kept = arriving.min(MAX_BODY_BYTES as u64);
        let mut budget = self.budget();
        let mut keep = if storable {
            Keep::Store
        } else {
            Keep::AnalyseOnly(BodyState::NotKeptType)
        };
        if storable && budget.kept + kept > MAX_SCAN_BODY_BYTES {
            budget.truncated = true;
            if !analysed {
                return BodyAdmission::Skip(BodySkip::ScanLimit);
            }
            keep = Keep::AnalyseOnly(BodyState::ScanLimit);
        }
        if budget.in_flight.saturating_add(arriving) > MAX_IN_FLIGHT_BODY_BYTES {
            budget.truncated |= storable;
            return BodyAdmission::Skip(BodySkip::Backlog);
        }
        budget.in_flight += arriving;
        if keep == Keep::Store {
            budget.kept += kept;
        }
        budget.reserved.insert(
            offer.seq,
            Reservation {
                in_flight: arriving,
                kept,
                keep,
            },
        );
        BodyAdmission::Read {
            max_bytes: MAX_BODY_BYTES,
        }
    }

    fn record(&self, mut record: NetworkRecord) {
        let body = match &record.body {
            CapturedBody::Read { bytes, .. } => bytes.len() as u64,
            _ => 0,
        };
        let keep = {
            let mut budget = self.budget();
            let reserved = budget.reserved.remove(&record.seq).unwrap_or(Reservation {
                in_flight: 0,
                kept: 0,
                keep: Keep::Store,
            });
            // From here on the body counts with its real size: in the queue
            // until the recorder is done with it, and against the scan's cap.
            budget.in_flight = budget.in_flight.saturating_sub(reserved.in_flight) + body;
            if reserved.keep == Keep::Store {
                budget.kept = budget.kept.saturating_sub(reserved.kept) + body;
            }
            // A request body waits in the same queue. Nobody admitted it,
            // so when the queue is full it is the part that is let go.
            let post = queued_post_bytes(&record);
            if budget.in_flight + post > MAX_IN_FLIGHT_BODY_BYTES {
                record.post_data = None;
                record.post_data_missing = true;
            } else {
                budget.in_flight += post;
            }
            reserved.keep
        };
        // A closed queue means the recorder is gone; the scan goes on.
        let _ = self.queue.send(Item::Record(Box::new(record), keep));
    }
}

fn queued_post_bytes(record: &NetworkRecord) -> u64 {
    record
        .post_data
        .as_ref()
        .map_or(0, |post| post.bytes.len() as u64)
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
                // The row first: a file without one is only found by the
                // start-up sweep, a row without a file by whoever removes it.
                archive
                    .index
                    .begin(
                        &start.scan_id,
                        Some(&file),
                        !start.full_headers,
                        start.started_at,
                    )
                    .await
                    .map_err(|error| error.0)?;
                let pack = archive
                    .packs
                    .create(&file)
                    .await
                    .map_err(|error| error.to_string())?;
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
            scan_site: site_of(&start.scan_host),
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
    scan_site: String,
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
        let mut row = RequestIndexRow::of(&record, &self.scan_site, state, None);
        self.analysis.observe(&record, &row);

        let captured = std::mem::replace(&mut record.body, CapturedBody::Absent);
        let queued = queued_post_bytes(&record)
            + match &captured {
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
        let document = RequestDocument::of(
            &start.scan_id,
            record,
            start.full_headers,
            row.body_state,
            stored,
        );
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
        // Searching, hashing and compressing megabytes is not for an async
        // worker.
        let (sha256, bytes) = tokio::task::spawn_blocking(move || {
            let bytes = if kind == BodyKind::Json {
                without_credentials(bytes)
            } else {
                bytes
            };
            let sha256: [u8; 32] = Sha256::digest(&bytes).into();
            (sha256, bytes)
        })
        .await
        .map_err(|error| error.to_string())?;
        let raw_length = bytes.len() as u64;
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

    async fn seal(
        mut self,
        start: &RecordingStart,
        truncated: bool,
    ) -> Result<ArchiveTotals, String> {
        self.flush(&start.scan_id).await?;
        let manifest = Manifest {
            format: FORMAT_VERSION,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            scan_id: start.scan_id.clone(),
            url: stored_url(&start.url).into_owned(),
            started_at: start.started_at,
            headers_redacted: !start.full_headers,
            truncated,
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

/// A JSON answer as it is stored: without the tokens a field of it carries.
fn without_credentials(bytes: Vec<u8>) -> Vec<u8> {
    let stored = match stored_json(&String::from_utf8_lossy(&bytes)) {
        std::borrow::Cow::Owned(stored) => Some(stored.into_bytes()),
        std::borrow::Cow::Borrowed(_) => None,
    };
    stored.unwrap_or(bytes)
}

#[cfg(test)]
mod tests {
    use adscan_model::{
        InitiatorKind, NetworkInitiator, RequestContext, RequestEnd, ResourceKind, ServedFrom,
    };

    use super::*;
    use crate::application::ports::{ArchiveState, ScanRepository};
    use crate::infrastructure::pack_store::LocalPackStore;
    use crate::infrastructure::sqlite::SqliteStore;

    const MIB: u64 = 1024 * 1024;

    struct Fixture {
        archive: Archive,
        _directory: tempfile::TempDir,
    }

    async fn fixture() -> Fixture {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::open_in_memory().unwrap();
        let summary = adscan_model::ScanSummary {
            id: "scan".into(),
            url: "https://pub.example/".into(),
            hostname: "pub.example".into(),
            device: adscan_model::DeviceProfile::Desktop,
            status: adscan_model::ScanStatus::Running,
            started_at: 1.0,
            finished_at: None,
            duration_ms: None,
            cost_usd: 0.0,
            finding_counts: adscan_model::SeverityCounts::default(),
            health_score: None,
            prebid_version: None,
            ai_used: false,
            error: None,
            configuration: None,
            proxy: None,
        };
        store
            .create(
                &summary,
                &adscan_model::ScanOptions::for_url("https://pub.example/"),
            )
            .await
            .unwrap();
        Fixture {
            archive: Archive {
                packs: Arc::new(LocalPackStore::new(directory.path())),
                index: Arc::new(store),
            },
            _directory: directory,
        }
    }

    fn start() -> RecordingStart {
        RecordingStart {
            scan_id: "scan".into(),
            url: "https://pub.example/".into(),
            scan_host: "pub.example".into(),
            started_at: 1.0,
            full_headers: false,
        }
    }

    fn offer(
        seq: u32,
        url: &'static str,
        kind: ResourceKind,
        mime: &'static str,
        bytes: u64,
    ) -> BodyOffer<'static> {
        BodyOffer {
            seq,
            url,
            bidder: None,
            kind,
            mime_type: Some(mime),
            status: Some(200),
            expected_bytes: bytes,
        }
    }

    fn script_offer(seq: u32, bytes: u64) -> BodyOffer<'static> {
        offer(
            seq,
            "https://cdn.example/a.js",
            ResourceKind::Script,
            "text/javascript",
            bytes,
        )
    }

    fn record(seq: u32, url: &str, body: CapturedBody) -> NetworkRecord {
        NetworkRecord {
            seq,
            page: 0,
            page_host: String::new(),
            context: RequestContext::Page,
            frame_id: None,
            target_id: None,
            method: "GET".into(),
            url: url.into(),
            kind: ResourceKind::Script,
            priority: None,
            request_headers: Vec::new(),
            post_data: None,
            post_data_missing: false,
            initiator: NetworkInitiator {
                kind: InitiatorKind::Other,
                url: None,
                line: None,
                column: None,
                stack: Vec::new(),
            },
            redirect_from: None,
            preflight_of: None,
            status: Some(200),
            mime_type: Some("text/javascript".into()),
            protocol: None,
            remote_address: None,
            response_headers: Vec::new(),
            served_from: ServedFrom::Network,
            started_at_ms: f64::from(seq),
            response_at_ms: None,
            ended_at_ms: Some(f64::from(seq) + 1.0),
            queued_ms: None,
            timing: None,
            encoded_bytes: 0,
            decoded_bytes: 0,
            end: RequestEnd::Finished,
            bidder: None,
            body,
        }
    }

    fn read(text: &str) -> CapturedBody {
        CapturedBody::Read {
            bytes: text.as_bytes().to_vec(),
            total_bytes: text.len() as u64,
        }
    }

    #[tokio::test]
    async fn admission_follows_the_kind_of_body() {
        let fixture = fixture().await;
        let recording = Recording::start(Some(&fixture.archive), start())
            .await
            .unwrap();
        let sink = recording.sink();
        let max_bytes = MAX_BODY_BYTES;
        assert_eq!(
            sink.admit_body(&script_offer(0, 1_000)),
            BodyAdmission::Read { max_bytes }
        );
        assert_eq!(
            sink.admit_body(&offer(
                1,
                "https://pub.example/a.png",
                ResourceKind::Image,
                "image/png",
                1_000
            )),
            BodyAdmission::Skip(BodySkip::Type)
        );
        assert_eq!(
            sink.admit_body(&offer(
                2,
                "https://pub.example/v.mp4",
                ResourceKind::Media,
                "video/mp4",
                0
            )),
            BodyAdmission::Skip(BodySkip::Type)
        );
        recording.abort();
    }

    #[tokio::test]
    async fn a_body_far_larger_than_what_is_kept_counts_in_full_while_it_is_on_its_way() {
        // The browser hands a body over whole before it can be cut to the
        // first 5 MiB: a page that streams huge answers must not be able to
        // have them all in memory at once.
        let fixture = fixture().await;
        let recording = Recording::start(Some(&fixture.archive), start())
            .await
            .unwrap();
        let sink = recording.sink();
        assert!(matches!(
            sink.admit_body(&script_offer(0, 30 * MIB)),
            BodyAdmission::Read { .. }
        ));
        assert!(matches!(
            sink.admit_body(&script_offer(1, 30 * MIB)),
            BodyAdmission::Read { .. }
        ));
        // 60 of the 64 MiB are spoken for.
        for (seq, bytes) in [(2, 30 * MIB), (3, 5 * MIB), (4, 400 * MIB), (5, u64::MAX)] {
            assert_eq!(
                sink.admit_body(&script_offer(seq, bytes)),
                BodyAdmission::Skip(BodySkip::Backlog),
                "{bytes}"
            );
        }
        // Against the scan's cap each counts with what is kept of it.
        let budget = Arc::clone(&recording.sink);
        assert_eq!(budget.budget().kept, 10 * MIB);
        for seq in 0..2 {
            sink.record(record(
                seq,
                "https://cdn.example/a.js",
                CapturedBody::Read {
                    bytes: vec![b'x'; 16],
                    total_bytes: 30 * MIB,
                },
            ));
        }
        recording.finish().await.unwrap();
        assert_eq!(budget.budget().in_flight, 0);
    }

    #[tokio::test]
    async fn request_bodies_that_wait_for_the_writer_are_bounded_too() {
        // A page that posts megabytes faster than the disk takes them: the
        // recorder task does not run while this test holds the thread, so
        // everything handed over waits in the queue.
        let fixture = fixture().await;
        let recording = Recording::start(Some(&fixture.archive), start())
            .await
            .unwrap();
        let sink = recording.sink();
        for seq in 0..80 {
            let mut posted = record(seq, "https://collect.example/batch", CapturedBody::Empty);
            posted.method = "POST".into();
            posted.post_data = Some(adscan_model::CapturedPostData {
                bytes: vec![b'x'; MIB as usize],
                total_bytes: MIB,
            });
            sink.record(posted);
        }
        let waiting = Arc::clone(&recording.sink);
        assert_eq!(waiting.budget().in_flight, 64 * MIB);
        recording.finish().await.unwrap();
        assert_eq!(waiting.budget().in_flight, 0);

        // The requests are all recorded; the ones over the budget say that
        // their body could not be kept.
        let (rows, total) = fixture
            .archive
            .index
            .rows("scan", &[], 0, 100)
            .await
            .unwrap();
        assert_eq!(total, 80);
        assert!(
            rows.iter()
                .all(|row| row.post_bytes == 0 || row.post_bytes == MIB)
        );
        assert_eq!(rows.iter().filter(|row| row.post_bytes == MIB).count(), 64);
    }

    #[tokio::test]
    async fn the_sink_stops_admitting_bodies_while_too_many_bytes_wait_for_the_writer() {
        let fixture = fixture().await;
        let recording = Recording::start(Some(&fixture.archive), start())
            .await
            .unwrap();
        let sink = recording.sink();
        // Twelve bodies of 5 MiB are announced: 60 of the 64 MiB that may
        // be in flight. Their records have not arrived yet.
        for seq in 0..12 {
            assert!(matches!(
                sink.admit_body(&script_offer(seq, 5 * MIB)),
                BodyAdmission::Read { .. }
            ));
        }
        assert_eq!(
            sink.admit_body(&script_offer(12, 5 * MIB)),
            BodyAdmission::Skip(BodySkip::Backlog)
        );
        // A small one still fits.
        assert!(matches!(
            sink.admit_body(&script_offer(13, MIB)),
            BodyAdmission::Read { .. }
        ));

        // The announced bodies arrive (much smaller than announced) and are
        // written: the room is free again.
        for seq in (0..12).chain([13]) {
            sink.record(record(
                seq,
                "https://cdn.example/a.js",
                read(&format!("var a{seq}=1;")),
            ));
        }
        sink.record(record(
            12,
            "https://cdn.example/a.js",
            CapturedBody::Skipped(BodySkip::Backlog),
        ));
        let recorded = recording.finish().await.unwrap();
        let totals = recorded.stored.unwrap().unwrap();
        assert!(totals.truncated, "a body was left out");

        let (rows, _) = fixture
            .archive
            .index
            .rows("scan", &[], 0, 100)
            .await
            .unwrap();
        let dropped = rows.iter().find(|row| row.seq == 12).unwrap();
        assert_eq!(
            (dropped.body_state, dropped.body_no),
            (BodyState::Dropped, None)
        );
        assert_eq!(
            rows.iter()
                .filter(|row| row.body_state == BodyState::Kept)
                .count(),
            13
        );
    }

    #[tokio::test]
    async fn past_the_scan_cap_requests_keep_their_metadata_and_say_why_the_body_is_missing() {
        let fixture = fixture().await;
        let recording = Recording::start(Some(&fixture.archive), start())
            .await
            .unwrap();
        let sink = recording.sink();
        // Fill the scan's 256 MiB with bodies of 5 MiB, in the batches the
        // in-flight limit allows.
        let mut seq = 0;
        let mut admitted = Vec::new();
        loop {
            match sink.admit_body(&script_offer(seq, 5 * MIB)) {
                BodyAdmission::Read { .. } => admitted.push(seq),
                BodyAdmission::Skip(BodySkip::Backlog) => break,
                other => panic!("{other:?}"),
            }
            seq += 1;
        }
        assert_eq!(admitted.len(), 12);
        // Each arrives at full size: 5 MiB really kept per body.
        let five_mib = "x".repeat(5 * MIB as usize);
        let mut kept = 0u64;
        let mut next = 0;
        let limited = loop {
            for seq in admitted.drain(..) {
                sink.record(record(
                    seq,
                    "https://cdn.example/a.js",
                    read(&format!("{five_mib}{seq}")),
                ));
                kept += 5 * MIB;
            }
            // Wait for the writer to work the queue off.
            tokio::time::sleep(Duration::from_millis(50)).await;
            next += 1;
            let seq = 1_000 + next;
            match sink.admit_body(&script_offer(seq, 5 * MIB)) {
                BodyAdmission::Read { .. } => admitted.push(seq),
                BodyAdmission::Skip(BodySkip::Backlog) => {}
                BodyAdmission::Skip(BodySkip::ScanLimit) => break seq,
                other => panic!("{other:?}"),
            }
            assert!(kept < MAX_SCAN_BODY_BYTES + 64 * MIB, "the cap never came");
        };
        assert!(
            kept + 5 * MIB > MAX_SCAN_BODY_BYTES - 5 * MIB,
            "kept {kept}"
        );

        // A bid call that no longer fits is still read for the analysis,
        // and not stored.
        let bid_offer = BodyOffer {
            bidder: Some("openx"),
            ..offer(
                5_000,
                "https://rtb.openx.net/openrtbb/prebidjs",
                ResourceKind::Fetch,
                "application/json",
                5 * MIB,
            )
        };
        assert!(matches!(
            sink.admit_body(&bid_offer),
            BodyAdmission::Read { .. }
        ));
        let mut bid = record(
            5_000,
            "https://rtb.openx.net/openrtbb/prebidjs",
            read(r#"{"id":"1","nbr":2}"#),
        );
        bid.kind = ResourceKind::Fetch;
        bid.method = "POST".into();
        bid.bidder = Some("openx".into());
        bid.mime_type = Some("application/json".into());
        sink.record(bid);
        sink.record(record(
            limited,
            "https://cdn.example/late.js",
            CapturedBody::Skipped(BodySkip::ScanLimit),
        ));

        let recorded = recording.finish().await.unwrap();
        assert!(recorded.stored.unwrap().unwrap().truncated);
        assert_eq!(recorded.analysis.exchanges.len(), 1);
        assert_eq!(
            recorded.analysis.exchanges[0].nbr,
            Some(2),
            "the answer was read"
        );
        let (rows, _) = fixture
            .archive
            .index
            .rows("scan", &[], 0, 1_000)
            .await
            .unwrap();
        let state = |seq: u32| rows.iter().find(|row| row.seq == seq).unwrap().body_state;
        assert_eq!(state(limited), BodyState::ScanLimit);
        assert_eq!(state(5_000), BodyState::ScanLimit);
        assert_eq!(
            state(0),
            BodyState::Truncated,
            "one byte more than the per-body cap"
        );
        let archive = fixture.archive.index.record("scan").await.unwrap().unwrap();
        assert_eq!(archive.state, ArchiveState::Sealed);
        assert!(archive.truncated);
    }

    #[tokio::test]
    async fn without_storage_only_bid_and_ad_server_calls_are_read_and_nothing_is_written() {
        let fixture = fixture().await;
        let recording = Recording::start(None, start()).await.unwrap();
        let sink = recording.sink();
        assert_eq!(
            sink.admit_body(&script_offer(0, 1_000)),
            BodyAdmission::Skip(BodySkip::NotStored)
        );
        assert_eq!(
            sink.admit_body(&offer(
                1,
                "https://pub.example/a.png",
                ResourceKind::Image,
                "image/png",
                10
            )),
            BodyAdmission::Skip(BodySkip::NotStored)
        );
        for (seq, url) in [
            (
                2,
                "https://securepubads.g.doubleclick.net/gampad/ads?iu=/1/top",
            ),
            (3, "https://ib.adnxs.com/ut/v3"),
            (4, "https://www8.smartadserver.com/ac?siteid=1"),
            (5, "https://pbs.example.net/openrtb2/auction"),
        ] {
            assert!(
                matches!(
                    sink.admit_body(&offer(seq, url, ResourceKind::Xhr, "application/json", 500)),
                    BodyAdmission::Read { .. }
                ),
                "{url}"
            );
        }
        let mut auction = record(
            5,
            "https://pbs.example.net/openrtb2/auction",
            read(r#"{"seatbid":[{"bid":[{"impid":"a","price":2.5}]}]}"#),
        );
        auction.method = "POST".into();
        auction.kind = ResourceKind::Fetch;
        sink.record(auction);
        sink.record(record(
            2,
            "https://securepubads.g.doubleclick.net/gampad/ads?iu=/1/top",
            read("{}"),
        ));
        sink.record(record(
            0,
            "https://cdn.example/a.js",
            CapturedBody::Skipped(BodySkip::NotStored),
        ));
        recording.store_script("var inline = 1;");

        let recorded = recording.finish().await.unwrap();
        assert!(recorded.stored.is_none());
        assert_eq!(recorded.analysis.exchanges.len(), 1);
        assert_eq!(recorded.analysis.exchanges[0].best_cpm, Some(2.5));
        assert!(recorded.analysis.exchanges[0].via_server);
        assert_eq!(recorded.analysis.ad_server_calls.len(), 1);
        assert_eq!(fixture.archive.index.record("scan").await.unwrap(), None);
        assert!(fixture.archive.packs.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn rows_never_point_past_what_is_durable_in_the_pack() {
        let fixture = fixture().await;
        let recording = Recording::start(Some(&fixture.archive), start())
            .await
            .unwrap();
        let sink = recording.sink();
        for seq in 0..300 {
            assert!(matches!(
                sink.admit_body(&script_offer(seq, 100)),
                BodyAdmission::Read { .. }
            ));
            sink.record(record(
                seq,
                "https://cdn.example/a.js",
                read(&format!("var unique{seq}={seq};")),
            ));
        }
        // Wait until some rows are in the index, then stop the writer as a
        // crash would, mid-flight.
        let mut indexed = 0;
        for _ in 0..200 {
            indexed = fixture
                .archive
                .index
                .rows("scan", &[], 0, 1)
                .await
                .unwrap()
                .1;
            if indexed >= 128 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(indexed >= 128, "a full record frame was flushed");
        recording.abort();

        let pack_bytes = fixture.archive.packs.list().await.unwrap()[0].bytes;
        let (rows, _) = fixture
            .archive
            .index
            .rows("scan", &[], 0, 1_000)
            .await
            .unwrap();
        for row in &rows {
            assert!(row.record_offset + u64::from(row.record_length) <= pack_bytes);
            let body = fixture
                .archive
                .index
                .body("scan", row.body_no.unwrap())
                .await
                .unwrap()
                .expect("a row's body is indexed with it");
            assert!(body.pack_offset + u64::from(body.pack_length) <= pack_bytes);
            // And the bytes are really there.
            let frame = fixture
                .archive
                .packs
                .read(&crate::domain::archive::Location {
                    file: pack_file("scan"),
                    offset: body.pack_offset,
                    length: body.pack_length,
                })
                .await
                .unwrap();
            assert!(codec::decompress(&frame, body.raw_len as usize).is_ok());
        }
        // Never sealed: the next start removes it.
        let archive = fixture.archive.index.record("scan").await.unwrap().unwrap();
        assert_eq!(archive.state, ArchiveState::Writing);
        let report = fixture.archive.sweep().await.unwrap();
        assert_eq!(report.unfinished, 1);
        assert!(fixture.archive.packs.list().await.unwrap().is_empty());
    }
}
