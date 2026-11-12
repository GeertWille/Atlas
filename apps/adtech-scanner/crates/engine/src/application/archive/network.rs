//! Reading a scan's network record back: the request list in chunks, the
//! digest, one request with its headers and decoded exchange, and bodies by
//! byte range.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use adscan_model::{
    AdContext, AdContextUnit, BodyRef, BodySide, BodyState, DiagnosticStatus, ExchangeVerdict,
    NetworkArchiveInfo, NetworkAvailability, NetworkBodyChunk, NetworkCategory,
    NetworkCategoryCount, NetworkPageCount, NetworkRequestDetail, NetworkRow, NetworkRowChunk,
    NetworkSummary, NetworkVendorCount, RedirectHop, RequestOutcome, ScanDetail, ServedFrom,
    request_id, request_seq,
};

use super::storage::pack_error;
use super::{Archive, codec};
use crate::application::AppError;
use crate::application::ports::{ArchiveRecord, RequestIndexRow, ScanRepository};
use crate::domain::archive::document::RequestDocument;
use crate::domain::archive::redact::displayed_headers;
use crate::domain::archive::Location;
use crate::domain::network::rows::{self, ScriptIndex};
use crate::domain::network::{classify, openrtb};

const DEFAULT_CHUNK_ROWS: u32 = 2_000;
const MAX_CHUNK_ROWS: u32 = 5_000;
const DEFAULT_BODY_CHUNK_BYTES: usize = 2 * 1024 * 1024;
const MAX_BODY_CHUNK_BYTES: usize = 8 * 1024 * 1024;
/// The longest UTF-8 character: a smaller chunk could hold none.
const MIN_BODY_CHUNK_BYTES: f64 = 4.0;
/// A record frame is closed at 256 KiB of JSON, but its last document can
/// carry a megabyte of request body; this only stops a wrong index row.
const MAX_RECORD_FRAME_CONTENT: usize = 64 * 1024 * 1024;
const MAX_NOTABLE_ROWS: usize = 8;
const TOP_VENDORS: usize = 10;

pub struct NetworkView {
    archive: Archive,
    scans: Arc<dyn ScanRepository>,
}

impl NetworkView {
    pub fn new(archive: Archive, scans: Arc<dyn ScanRepository>) -> Self {
        Self { archive, scans }
    }

    pub async fn list(
        &self,
        scan_id: &str,
        pages: &[u32],
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<NetworkRowChunk, AppError> {
        let scan = self.scan(scan_id).await?;
        let offset = match cursor {
            Some(cursor) => cursor
                .parse::<u32>()
                .map_err(|_| AppError::InvalidInput(format!("`{cursor}` is not a cursor of this list")))?,
            None => 0,
        };
        let limit = limit.unwrap_or(DEFAULT_CHUNK_ROWS).clamp(1, MAX_CHUNK_ROWS);
        let Some(record) = self.archive.index.record(scan_id).await? else {
            return Ok(NetworkRowChunk {
                rows: Vec::new(),
                next_cursor: None,
                total: 0,
            });
        };
        let (index_rows, total) = self.archive.index.rows(scan_id, pages, offset, limit).await?;
        let documents = self.documents(&record, &index_rows).await;
        let scripts = ScriptIndex::new(&scan.scripts);
        let next = offset + index_rows.len() as u32;
        Ok(NetworkRowChunk {
            rows: index_rows
                .iter()
                .map(|row| rows::row(row, documents.get(&row.seq), &scripts))
                .collect(),
            next_cursor: (next < total).then(|| next.to_string()),
            total,
        })
    }

    pub async fn summary(&self, scan_id: &str) -> Result<NetworkSummary, AppError> {
        let scan = self.scan(scan_id).await?;
        let record = self.archive.index.record(scan_id).await?;
        let archive = record.as_ref().map_or(
            NetworkArchiveInfo {
                availability: NetworkAvailability::NotRecorded,
                request_count: 0,
                body_count: 0,
                stored_bytes: 0.0,
                headers_redacted: false,
                truncated: false,
            },
            ArchiveRecord::info,
        );
        let (index_rows, _) = self.archive.index.rows(scan_id, &[], 0, u32::MAX).await?;
        let transferred = |row: &RequestIndexRow| {
            if row.served_from == ServedFrom::Network {
                row.encoded_bytes as f64
            } else {
                0.0
            }
        };
        let mut categories: BTreeMap<u8, NetworkCategoryCount> = BTreeMap::new();
        let mut vendors: HashMap<&'static str, u32> = HashMap::new();
        let mut pages: BTreeMap<u32, NetworkPageCount> = BTreeMap::new();
        let mut summary = NetworkSummary {
            archive,
            total_requests: index_rows.len() as u32,
            transferred_bytes: 0.0,
            failed: 0,
            blocked_or_cancelled: 0,
            late_bids: 0,
            unanswered_bids: 0,
            from_cache: 0,
            categories: Vec::new(),
            vendors: Vec::new(),
            pages: Vec::new(),
            notable: Vec::new(),
        };
        for row in &index_rows {
            let bytes = transferred(row);
            summary.transferred_bytes += bytes;
            match rows::outcome(row) {
                RequestOutcome::HttpError | RequestOutcome::Failed => summary.failed += 1,
                RequestOutcome::Blocked | RequestOutcome::Cancelled => summary.blocked_or_cancelled += 1,
                RequestOutcome::Ok | RequestOutcome::Pending => {}
            }
            match row.ad.as_ref().map(|ad| ad.verdict) {
                Some(ExchangeVerdict::LateOnNetwork) => summary.late_bids += 1,
                Some(ExchangeVerdict::NeverAnswered) => summary.unanswered_bids += 1,
                _ => {}
            }
            if row.served_from != ServedFrom::Network {
                summary.from_cache += 1;
            }
            let category = categories
                .entry(category_order(row.category))
                .or_insert(NetworkCategoryCount {
                    category: row.category,
                    requests: 0,
                    transferred_bytes: 0.0,
                });
            category.requests += 1;
            category.transferred_bytes += bytes;
            if let Some(vendor) = classify::vendor(&row.host) {
                *vendors.entry(vendor).or_default() += 1;
            }
            let page = pages.entry(row.page).or_insert(NetworkPageCount {
                page: row.page,
                requests: 0,
                transferred_bytes: 0.0,
            });
            page.requests += 1;
            page.transferred_bytes += bytes;
        }
        summary.categories = categories.into_values().collect();
        summary.pages = pages.into_values().collect();
        let mut vendors: Vec<NetworkVendorCount> = vendors
            .into_iter()
            .map(|(vendor, requests)| NetworkVendorCount {
                vendor: vendor.to_string(),
                requests,
            })
            .collect();
        vendors.sort_by(|a, b| b.requests.cmp(&a.requests).then(a.vendor.cmp(&b.vendor)));
        vendors.truncate(TOP_VENDORS);
        summary.vendors = vendors;

        let notable: Vec<RequestIndexRow> = notable(&index_rows, &scan)
            .into_iter()
            .cloned()
            .collect();
        if let Some(record) = &record {
            let documents = self.documents(record, &notable).await;
            let scripts = ScriptIndex::new(&scan.scripts);
            summary.notable = notable
                .iter()
                .map(|row| rows::row(row, documents.get(&row.seq), &scripts))
                .collect();
        }
        Ok(summary)
    }

    pub async fn request(&self, scan_id: &str, id: &str) -> Result<NetworkRequestDetail, AppError> {
        let scan = self.scan(scan_id).await?;
        let (record, all, index_row) = self.locate(scan_id, id).await?;
        let scripts = ScriptIndex::new(&scan.scripts);

        let chain = redirect_chain(&all, &index_row);
        let mut wanted: Vec<RequestIndexRow> = chain.iter().map(|hop| (*hop).clone()).collect();
        if wanted.is_empty() {
            wanted.push(index_row.clone());
        }
        let documents = self.documents(&record, &wanted).await;
        let document = documents.get(&index_row.seq);
        let row = rows::row(&index_row, document, &scripts);

        let response_text = match (document, index_row.category) {
            (Some(_), NetworkCategory::Bid) => self.stored_body(scan_id, &record, &index_row).await.ok(),
            _ => None,
        };
        let decoded = document.filter(|_| index_row.category == NetworkCategory::Bid).and_then(|document| {
            let auction_units: Vec<&str> = scan
                .auctions
                .iter()
                .filter(|auction| row.ad.as_ref().is_none_or(|ad| ad.auction_id == auction.id))
                .flat_map(|auction| auction.ad_units.iter().map(|unit| unit.code.as_str()))
                .collect();
            let resolve = |found: &str| {
                index_row
                    .bid_units
                    .iter()
                    .find(|(bid_id, _)| bid_id == found)
                    .map(|(_, unit)| unit.clone())
                    .or_else(|| auction_units.contains(&found).then(|| found.to_string()))
            };
            openrtb::decode(
                &index_row.path,
                document.post_data.as_deref(),
                response_text.as_ref().map(|body| String::from_utf8_lossy(body)).as_deref(),
                &resolve,
            )
        });

        let request_body = match document {
            Some(document) => BodyRef {
                state: match &document.post_data {
                    Some(_) if document.post_truncated() => BodyState::Truncated,
                    Some(_) => BodyState::Kept,
                    None if document.post_missing => BodyState::Unavailable,
                    None => BodyState::Empty,
                },
                bytes: document.post_bytes as f64,
                stored_bytes: document.post_data.as_ref().map_or(0.0, |post| post.len() as f64),
                mime: header(&document.headers, "content-type"),
                script_id: None,
            },
            None => BodyRef {
                state: if index_row.post_bytes > 0 {
                    BodyState::Evicted
                } else {
                    BodyState::Empty
                },
                bytes: index_row.post_bytes as f64,
                stored_bytes: 0.0,
                mime: None,
                script_id: None,
            },
        };
        let stored = document.and_then(|document| document.body.as_ref());
        let response_body = BodyRef {
            state: index_row.body_state,
            bytes: stored.map_or(index_row.decoded_bytes as f64, |body| body.bytes as f64),
            stored_bytes: stored.map_or(0.0, |body| body.stored_bytes as f64),
            mime: index_row.mime.clone(),
            script_id: stored.and_then(|body| scripts.id_of_content(&body.sha256)),
        };
        Ok(NetworkRequestDetail {
            available: document.is_some(),
            request_headers: document.map_or_else(Vec::new, |document| {
                displayed_headers(&document.headers, document.headers_redacted)
            }),
            response_headers: document.map_or_else(Vec::new, |document| {
                displayed_headers(&document.response_headers, document.headers_redacted)
            }),
            protocol: document.and_then(|document| document.protocol.clone()),
            priority: document.and_then(|document| document.priority.clone()),
            remote_address: document.and_then(|document| document.remote_address.clone()),
            timing: document.and_then(|document| rows::timing_detail(document, index_row.duration_ms)),
            redirect_chain: chain
                .iter()
                .map(|hop| RedirectHop {
                    request_id: request_id(hop.seq),
                    url: rows::row(hop, documents.get(&hop.seq), &scripts).url,
                    status: hop.status,
                })
                .collect(),
            preflight_ms: all
                .iter()
                .find(|other| other.preflight_of == Some(index_row.seq))
                .and_then(|preflight| preflight.duration_ms),
            stack: document.map_or_else(Vec::new, |document| rows::stack(&document.initiator, &scripts)),
            request_body,
            response_body,
            ad_context: ad_context(&row, &scan),
            decoded,
            row,
        })
    }

    pub async fn body(
        &self,
        scan_id: &str,
        id: &str,
        side: BodySide,
        offset: Option<f64>,
        max_bytes: Option<f64>,
    ) -> Result<NetworkBodyChunk, AppError> {
        self.scan(scan_id).await?;
        let (record, _, row) = self.locate(scan_id, id).await?;
        let (state, content) = match side {
            BodySide::Response => match row.body_no {
                Some(_) => (row.body_state, Some(self.stored_body(scan_id, &record, &row).await?)),
                None => (row.body_state, None),
            },
            BodySide::Request => {
                let documents = self.documents(&record, std::slice::from_ref(&row)).await;
                match documents.get(&row.seq) {
                    Some(document) => match &document.post_data {
                        Some(post) if document.post_truncated() => {
                            (BodyState::Truncated, Some(post.clone().into_bytes()))
                        }
                        Some(post) => (BodyState::Kept, Some(post.clone().into_bytes())),
                        None if document.post_missing => (BodyState::Unavailable, None),
                        None => (BodyState::Empty, None),
                    },
                    None if row.post_bytes > 0 => (BodyState::Evicted, None),
                    None => (BodyState::Empty, None),
                }
            }
        };
        let Some(content) = content else {
            return Ok(NetworkBodyChunk {
                state,
                text: None,
                offset: 0.0,
                next_offset: None,
                total_bytes: 0.0,
            });
        };
        let limit = max_bytes
            .map_or(DEFAULT_BODY_CHUNK_BYTES, |max| max.max(MIN_BODY_CHUNK_BYTES) as usize)
            .min(MAX_BODY_CHUNK_BYTES);
        let (start, end) = text_range(&content, offset.unwrap_or(0.0).max(0.0) as usize, limit);
        Ok(NetworkBodyChunk {
            state,
            text: Some(String::from_utf8_lossy(&content[start..end]).into_owned()),
            offset: start as f64,
            next_offset: (end < content.len()).then_some(end as f64),
            total_bytes: content.len() as f64,
        })
    }

    async fn scan(&self, scan_id: &str) -> Result<ScanDetail, AppError> {
        self.scans
            .detail(scan_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Scan {scan_id}")))
    }

    /// The archive, all its rows and the row of request `id`.
    async fn locate(
        &self,
        scan_id: &str,
        id: &str,
    ) -> Result<(ArchiveRecord, Vec<RequestIndexRow>, RequestIndexRow), AppError> {
        let not_found = || AppError::NotFound(format!("Request {id}"));
        let record = self.archive.index.record(scan_id).await?.ok_or_else(not_found)?;
        let seq = request_seq(id).ok_or_else(not_found)?;
        let (all, _) = self.archive.index.rows(scan_id, &[], 0, u32::MAX).await?;
        let row = all.iter().find(|row| row.seq == seq).cloned().ok_or_else(not_found)?;
        Ok((record, all, row))
    }

    /// The documents of `rows`, by `seq`. A row whose frame cannot be read
    /// (the pack was evicted, or is damaged) has none and is shown from the
    /// index alone.
    async fn documents(&self, record: &ArchiveRecord, rows: &[RequestIndexRow]) -> HashMap<u32, RequestDocument> {
        let mut documents = HashMap::with_capacity(rows.len());
        let Some(file) = &record.pack_file else {
            return documents;
        };
        let mut by_frame: BTreeMap<(u64, u32), Vec<&RequestIndexRow>> = BTreeMap::new();
        for row in rows {
            by_frame
                .entry((row.record_offset, row.record_length))
                .or_default()
                .push(row);
        }
        for ((offset, length), rows) in by_frame {
            let location = Location {
                file: file.clone(),
                offset,
                length,
            };
            let Ok(frame) = self.archive.packs.read(&location).await else {
                continue;
            };
            let parsed = tokio::task::spawn_blocking({
                let wanted: Vec<(u32, u32)> = rows.iter().map(|row| (row.seq, row.record_index)).collect();
                move || {
                    let content = codec::decompress(&frame, MAX_RECORD_FRAME_CONTENT).ok()?;
                    let text = std::str::from_utf8(&content).ok()?;
                    let lines: Vec<&str> = text.split('\n').collect();
                    Some(
                        wanted
                            .into_iter()
                            .filter_map(|(seq, index)| {
                                let document: RequestDocument =
                                    serde_json::from_str(lines.get(index as usize)?).ok()?;
                                (document.seq == seq).then_some((seq, document))
                            })
                            .collect::<Vec<_>>(),
                    )
                }
            })
            .await;
            if let Ok(Some(found)) = parsed {
                documents.extend(found);
            }
        }
        documents
    }

    /// The stored bytes of the request's response body, checked against the
    /// index.
    async fn stored_body(
        &self,
        scan_id: &str,
        record: &ArchiveRecord,
        row: &RequestIndexRow,
    ) -> Result<Vec<u8>, AppError> {
        let missing = || AppError::Storage("the body is not in the archive any more".into());
        let body_no = row.body_no.ok_or_else(missing)?;
        let file = record.pack_file.clone().ok_or_else(missing)?;
        let body = self.archive.index.body(scan_id, body_no).await?.ok_or_else(missing)?;
        read_body(&self.archive, file, body.pack_offset, body.pack_length, body.raw_len).await
    }
}

/// Reads and unpacks one body frame.
pub(super) async fn read_body(
    archive: &Archive,
    file: String,
    offset: u64,
    length: u32,
    raw_len: u64,
) -> Result<Vec<u8>, AppError> {
    let frame = archive
        .packs
        .read(&Location { file, offset, length })
        .await
        .map_err(pack_error)?;
    tokio::task::spawn_blocking(move || codec::decompress(&frame, raw_len as usize))
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?
        .map_err(|error| AppError::Storage(format!("a stored body could not be read: {error}")))
}

/// Start and end of at most `limit` bytes from `offset` that do not cut a
/// UTF-8 character in two.
fn text_range(content: &[u8], offset: usize, limit: usize) -> (usize, usize) {
    let is_continuation = |at: usize| content.get(at).is_some_and(|byte| byte & 0xC0 == 0x80);
    let mut start = offset.min(content.len());
    while is_continuation(start) {
        start += 1;
    }
    let mut end = start.saturating_add(limit).min(content.len());
    // At most three continuation bytes belong to one character; more than
    // that is not text and is cut where the limit falls.
    let mut back = 0;
    while end > start && is_continuation(end) && back < 3 {
        end -= 1;
        back += 1;
    }
    (start, end.max(start))
}

fn header(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
}

fn category_order(category: NetworkCategory) -> u8 {
    use NetworkCategory::*;
    match category {
        Bid => 0,
        AdServer => 1,
        Wrapper => 2,
        Consent => 3,
        Identity => 4,
        Measurement => 5,
        Creative => 6,
        Site => 7,
        OtherThirdParty => 8,
    }
}

/// Every hop of the redirect chain `row` is part of, in order; empty when
/// it is not part of one.
fn redirect_chain<'a>(all: &'a [RequestIndexRow], row: &'a RequestIndexRow) -> Vec<&'a RequestIndexRow> {
    let by_seq = |seq: u32| all.iter().find(|other| other.seq == seq);
    let mut chain = vec![row];
    while let Some(previous) = chain[0].redirect_from.and_then(by_seq) {
        if chain.len() > all.len() {
            break;
        }
        chain.insert(0, previous);
    }
    while let Some(next) = all
        .iter()
        .find(|other| other.redirect_from == Some(chain[chain.len() - 1].seq))
    {
        if chain.len() > all.len() {
            break;
        }
        chain.push(next);
    }
    if chain.len() == 1 { Vec::new() } else { chain }
}

/// The rows worth opening first: bid calls that failed, answers that came
/// late or never, blocked requests, and the slowest request before the
/// entry page's first auction.
fn notable<'a>(rows: &'a [RequestIndexRow], scan: &ScanDetail) -> Vec<&'a RequestIndexRow> {
    let failed_bid = |row: &&RequestIndexRow| {
        row.category == NetworkCategory::Bid
            && matches!(
                rows::outcome(row),
                RequestOutcome::HttpError | RequestOutcome::Failed | RequestOutcome::Blocked
            )
    };
    let late = |row: &&RequestIndexRow| {
        row.ad.as_ref().is_some_and(|ad| {
            matches!(
                ad.verdict,
                ExchangeVerdict::LateOnNetwork | ExchangeVerdict::NeverAnswered
            )
        })
    };
    let blocked = |row: &&RequestIndexRow| rows::outcome(row) == RequestOutcome::Blocked;
    let first_auction = scan
        .auctions
        .iter()
        .filter(|auction| auction.page == 0)
        .map(|auction| auction.started_at_ms)
        .reduce(f64::min);
    let slowest_before_auction = first_auction.and_then(|started| {
        rows.iter()
            .filter(|row| row.page == 0 && row.started_ms < started)
            .filter(|row| row.duration_ms.is_some())
            .max_by(|a, b| a.duration_ms.unwrap_or(0.0).total_cmp(&b.duration_ms.unwrap_or(0.0)))
    });
    let mut picked: Vec<&RequestIndexRow> = Vec::new();
    let candidates = rows
        .iter()
        .filter(failed_bid)
        .chain(rows.iter().filter(late))
        .chain(rows.iter().filter(blocked))
        .chain(slowest_before_auction);
    for row in candidates {
        if picked.len() == MAX_NOTABLE_ROWS {
            break;
        }
        if !picked.iter().any(|seen| seen.seq == row.seq) {
            picked.push(row);
        }
    }
    picked
}

/// The auction side of a bid request: its ad units as Prebid saw them, and
/// what the report says about its bidders.
fn ad_context(row: &NetworkRow, scan: &ScanDetail) -> Option<AdContext> {
    let link = row.ad.clone()?;
    let auction = scan.auctions.iter().rev().find(|auction| auction.id == link.auction_id);
    let mut units = Vec::new();
    let mut bidders: Vec<&str> = Vec::new();
    for unit in auction.iter().flat_map(|auction| &auction.ad_units) {
        for bid in &unit.bids {
            let Some(exchange) = &bid.exchange else { continue };
            if exchange.request_id.as_deref() == Some(row.id.as_str()) {
                units.push(AdContextUnit {
                    ad_unit: unit.code.clone(),
                    status: bid.status,
                    cpm: bid.cpm,
                    verdict: exchange.verdict,
                });
                if !bidders.contains(&bid.bidder.as_str()) {
                    bidders.push(&bid.bidder);
                }
            }
        }
    }
    let checks = scan.bidder_diagnostics.iter().flat_map(|diagnostics| &diagnostics.bidders);
    Some(AdContext {
        bidder: row
            .bidder
            .clone()
            .or_else(|| (bidders.len() == 1).then(|| bidders[0].to_string())),
        units,
        auction_started_at_ms: auction.map(|auction| auction.started_at_ms),
        auction_ended_at_ms: auction.and_then(|auction| auction.ended_at_ms),
        timeout_ms: auction.and_then(|auction| auction.timeout_ms),
        slot_element_id: None,
        finding_ids: scan
            .findings
            .iter()
            .filter(|finding| {
                finding
                    .affected_bidders
                    .iter()
                    .any(|bidder| bidders.contains(&bidder.as_str()))
            })
            .map(|finding| finding.id.clone())
            .collect(),
        check_codes: checks
            .filter(|diagnosis| bidders.contains(&diagnosis.bidder.as_str()))
            .flat_map(|diagnosis| &diagnosis.checks)
            .filter(|check| matches!(check.status, DiagnosticStatus::Problem | DiagnosticStatus::Notice))
            .map(|check| check.code.clone())
            .collect(),
        link,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chunk_of_text_never_ends_inside_a_character() {
        let text = "aé€😀z".as_bytes(); // 1 + 2 + 3 + 4 + 1 bytes
        assert_eq!(text_range(text, 0, 100), (0, 11));
        assert_eq!(text_range(text, 0, 2), (0, 1), "é does not fit whole");
        assert_eq!(text_range(text, 1, 2), (1, 3));
        assert_eq!(text_range(text, 3, 4), (3, 6), "the emoji waits for the next chunk");
        assert_eq!(text_range(text, 6, 4), (6, 10));
        // An offset inside a character moves on to the next one.
        assert_eq!(text_range(text, 4, 100), (6, 11));
        assert_eq!(text_range(text, 99, 10), (11, 11));
        assert_eq!(text_range(b"", 0, 10), (0, 0));
    }

    #[test]
    fn chunks_read_in_sequence_give_back_the_whole_text() {
        let text = "naïve — “bid” 💶 ".repeat(50);
        let bytes = text.as_bytes();
        let mut offset = 0;
        let mut read = String::new();
        while offset < bytes.len() {
            let (start, end) = text_range(bytes, offset, 7);
            assert!(end > start);
            read.push_str(std::str::from_utf8(&bytes[start..end]).unwrap());
            offset = end;
        }
        assert_eq!(read, text);
    }
}
