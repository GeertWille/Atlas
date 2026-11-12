//! The index of the scan archives: bookkeeping per scan, body ranges and
//! request rows. Enums are stored as small integers in a fixed order; the
//! order below is part of the schema.

use adscan_model::{
    AdLink, ArchiveStorage, BodyState, InitiatorKind, NetworkCategory, RequestContext,
    ResourceKind, ServedFrom,
};
use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{SqliteStore, from_json, to_json};
use crate::application::ports::{
    ArchiveRecord, ArchiveRepository, ArchiveState, ArchiveTotals, BodyIndexRow, RepoResult,
    RequestAdLink, RequestEndKind, RequestIndexRow,
};
use crate::domain::archive::BodyKind;
use crate::domain::archive::pack::FORMAT_VERSION;

const CONTEXTS: [RequestContext; 4] = [
    RequestContext::Page,
    RequestContext::Iframe,
    RequestContext::Worker,
    RequestContext::ServiceWorker,
];
const METHODS: [&str; 7] = ["GET", "POST", "OPTIONS", "HEAD", "PUT", "DELETE", "PATCH"];
const OTHER_METHOD: i64 = 7;
const SCHEMES: [&str; 6] = ["https", "http", "wss", "ws", "data", "blob"];
const OTHER_SCHEME: i64 = 6;
const KINDS: [ResourceKind; 13] = [
    ResourceKind::Document,
    ResourceKind::Script,
    ResourceKind::Stylesheet,
    ResourceKind::Image,
    ResourceKind::Media,
    ResourceKind::Font,
    ResourceKind::Xhr,
    ResourceKind::Fetch,
    ResourceKind::Ping,
    ResourceKind::WebSocket,
    ResourceKind::EventSource,
    ResourceKind::Preflight,
    ResourceKind::Other,
];
const ENDS: [RequestEndKind; 6] = [
    RequestEndKind::Finished,
    RequestEndKind::Redirected,
    RequestEndKind::Failed,
    RequestEndKind::Blocked,
    RequestEndKind::Cancelled,
    RequestEndKind::Pending,
];
const SERVED_FROM: [ServedFrom; 5] = [
    ServedFrom::Network,
    ServedFrom::DiskCache,
    ServedFrom::MemoryCache,
    ServedFrom::ServiceWorker,
    ServedFrom::Prefetch,
];
const INITIATORS: [InitiatorKind; 5] = [
    InitiatorKind::Parser,
    InitiatorKind::Script,
    InitiatorKind::Preload,
    InitiatorKind::Preflight,
    InitiatorKind::Other,
];
const CATEGORIES: [NetworkCategory; 9] = [
    NetworkCategory::Bid,
    NetworkCategory::AdServer,
    NetworkCategory::Wrapper,
    NetworkCategory::Consent,
    NetworkCategory::Identity,
    NetworkCategory::Measurement,
    NetworkCategory::Creative,
    NetworkCategory::Site,
    NetworkCategory::OtherThirdParty,
];
const BODY_STATES: [BodyState; 9] = [
    BodyState::Kept,
    BodyState::Truncated,
    BodyState::Empty,
    BodyState::NotKeptType,
    BodyState::ScanLimit,
    BodyState::Dropped,
    BodyState::Unavailable,
    BodyState::Pending,
    BodyState::Evicted,
];

const FLAG_THIRD_PARTY: i64 = 1;
const FLAG_NO_HB_TARGETING: i64 = 2;

const ARCHIVE_COLUMNS: &str = "scan_id, state, pack_file, pack_bytes, request_count, body_count, \
     headers_mode, truncated, created_at";

const REQUEST_COLUMNS: &str = "r.seq, r.page, r.context, r.started_ms, r.ttfb_ms, r.duration_ms, r.method, \
     r.scheme, h.name, h.site, r.path, r.query_len, r.resource_type, r.mime, r.status, r.outcome, r.error, r.flags, \
     r.served_from, r.encoded_bytes, r.body_bytes, r.post_bytes, r.initiator_type, r.redirect_from, \
     r.preflight_of, r.bidder, r.category, r.body_state, r.body_no, r.record_off, r.record_len, r.record_idx, r.ad";

#[async_trait]
impl ArchiveRepository for SqliteStore {
    async fn begin(
        &self,
        scan_id: &str,
        pack_file: Option<&str>,
        headers_redacted: bool,
        created_at: f64,
    ) -> RepoResult<()> {
        let scan_id = scan_id.to_string();
        let pack_file = pack_file.map(str::to_string);
        self.call(move |connection| {
            let state = if pack_file.is_some() {
                ArchiveState::Writing
            } else {
                ArchiveState::Disabled
            };
            connection.execute(
                "INSERT INTO scan_archives (scan_id, state, format, pack_file, headers_mode, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    scan_id,
                    state_name(state),
                    FORMAT_VERSION,
                    pack_file,
                    if headers_redacted { "redacted" } else { "full" },
                    created_at,
                ],
            )?;
            Ok(())
        })
        .await
    }

    async fn append(
        &self,
        scan_id: &str,
        bodies: Vec<BodyIndexRow>,
        requests: Vec<RequestIndexRow>,
    ) -> RepoResult<()> {
        let scan_id = scan_id.to_string();
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let scan_no = scan_no(&transaction, &scan_id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
            {
                let mut insert_body = transaction.prepare_cached(
                    "INSERT INTO scan_bodies (scan_no, body_no, sha256, kind, raw_len, pack_off, pack_len, truncated)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                )?;
                for body in &bodies {
                    insert_body.execute(params![
                        scan_no,
                        body.body_no,
                        body.sha256.as_slice(),
                        body.kind.code(),
                        body.raw_len as i64,
                        body.pack_offset as i64,
                        body.pack_length,
                        body.truncated,
                    ])?;
                }
                let mut host_id = transaction.prepare_cached(
                    "INSERT INTO hosts (name, site) VALUES (?1, ?2)
                     ON CONFLICT (name) DO UPDATE SET site = excluded.site RETURNING id",
                )?;
                let mut insert_request = transaction.prepare_cached(
                    "INSERT INTO requests (scan_no, seq, page, context, started_ms, ttfb_ms, duration_ms, method,
                         scheme, host_id, path, query_len, resource_type, mime, status, outcome, error, flags,
                         served_from, encoded_bytes, body_bytes, post_bytes, initiator_type, redirect_from,
                         preflight_of, bidder, category, body_state, body_no, record_off, record_len, record_idx, ad)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19,
                         ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33)",
                )?;
                for request in &requests {
                    let host: i64 =
                        host_id.query_row(params![request.host, request.site], |row| row.get(0))?;
                    let flags = if request.third_party { FLAG_THIRD_PARTY } else { 0 }
                        | if request.no_hb_targeting { FLAG_NO_HB_TARGETING } else { 0 };
                    insert_request.execute(params![
                        scan_no,
                        request.seq,
                        request.page,
                        code(&CONTEXTS, request.context),
                        request.started_ms,
                        request.ttfb_ms,
                        request.duration_ms,
                        name_code(&METHODS, &request.method, OTHER_METHOD),
                        name_code(&SCHEMES, &request.scheme, OTHER_SCHEME),
                        host,
                        request.path,
                        request.query_len,
                        code(&KINDS, request.kind),
                        request.mime,
                        request.status,
                        code(&ENDS, request.end),
                        request.error,
                        flags,
                        code(&SERVED_FROM, request.served_from),
                        request.encoded_bytes as i64,
                        request.decoded_bytes as i64,
                        request.post_bytes as i64,
                        code(&INITIATORS, request.initiator_kind),
                        request.redirect_from,
                        request.preflight_of,
                        request.bidder,
                        code(&CATEGORIES, request.category),
                        code(&BODY_STATES, request.body_state),
                        request.body_no,
                        request.record_offset as i64,
                        request.record_length,
                        request.record_index,
                        stored_ad(request.ad.as_ref(), &request.bid_units)?,
                    ])?;
                }
            }
            transaction.execute(
                "UPDATE scan_archives SET request_count = request_count + ?2, body_count = body_count + ?3
                 WHERE scan_no = ?1",
                params![scan_no, requests.len() as i64, bodies.len() as i64],
            )?;
            transaction.commit()
        })
        .await
    }

    async fn seal(&self, scan_id: &str, totals: ArchiveTotals) -> RepoResult<()> {
        let scan_id = scan_id.to_string();
        self.call(move |connection| {
            connection.execute(
                "UPDATE scan_archives SET state = ?2, pack_bytes = ?3, truncated = ?4 WHERE scan_id = ?1",
                params![
                    scan_id,
                    state_name(ArchiveState::Sealed),
                    totals.pack_bytes as i64,
                    totals.truncated
                ],
            )?;
            Ok(())
        })
        .await
    }

    async fn record(&self, scan_id: &str) -> RepoResult<Option<ArchiveRecord>> {
        let scan_id = scan_id.to_string();
        self.call(move |connection| read_archive(connection, &scan_id))
            .await
    }

    async fn rows(
        &self,
        scan_id: &str,
        pages: &[u32],
        offset: u32,
        limit: u32,
    ) -> RepoResult<(Vec<RequestIndexRow>, u32)> {
        let scan_id = scan_id.to_string();
        // Page numbers are integers of ours, so they can be written into the
        // statement itself.
        let page_filter = if pages.is_empty() {
            String::new()
        } else {
            let list: Vec<String> = pages.iter().map(u32::to_string).collect();
            format!("AND r.page IN ({})", list.join(", "))
        };
        self.call(move |connection| {
            let Some(scan_no) = scan_no(connection, &scan_id)? else {
                return Ok((Vec::new(), 0));
            };
            let total: u32 = connection.query_row(
                &format!("SELECT count(*) FROM requests r WHERE r.scan_no = ?1 {page_filter}"),
                [scan_no],
                |row| row.get(0),
            )?;
            let mut statement = connection.prepare(&format!(
                "SELECT {REQUEST_COLUMNS} FROM requests r JOIN hosts h ON h.id = r.host_id
                 WHERE r.scan_no = ?1 {page_filter}
                 ORDER BY r.page, r.started_ms, r.seq LIMIT ?2 OFFSET ?3"
            ))?;
            let rows = statement
                .query_map(params![scan_no, limit, offset], read_request)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok((rows, total))
        })
        .await
    }

    async fn rows_by_seq(&self, scan_id: &str, seqs: &[u32]) -> RepoResult<Vec<RequestIndexRow>> {
        let scan_id = scan_id.to_string();
        let seqs = seqs.to_vec();
        self.call(move |connection| {
            let Some(scan_no) = scan_no(connection, &scan_id)? else {
                return Ok(Vec::new());
            };
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {REQUEST_COLUMNS} FROM requests r JOIN hosts h ON h.id = r.host_id
                 WHERE r.scan_no = ?1 AND r.seq = ?2"
            ))?;
            let mut rows = Vec::with_capacity(seqs.len());
            for seq in seqs {
                if let Some(row) = statement
                    .query_row(params![scan_no, seq], read_request)
                    .optional()?
                {
                    rows.push(row);
                }
            }
            Ok(rows)
        })
        .await
    }

    async fn body(&self, scan_id: &str, body_no: u32) -> RepoResult<Option<BodyIndexRow>> {
        let scan_id = scan_id.to_string();
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT b.body_no, b.sha256, b.kind, b.raw_len, b.pack_off, b.pack_len, b.truncated
                     FROM scan_bodies b JOIN scan_archives a ON a.scan_no = b.scan_no
                     WHERE a.scan_id = ?1 AND b.body_no = ?2",
                    params![scan_id, body_no],
                    read_body,
                )
                .optional()
        })
        .await
    }

    async fn body_by_sha(
        &self,
        scan_id: &str,
        sha256: [u8; 32],
    ) -> RepoResult<Option<BodyIndexRow>> {
        let scan_id = scan_id.to_string();
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT b.body_no, b.sha256, b.kind, b.raw_len, b.pack_off, b.pack_len, b.truncated
                     FROM scan_bodies b JOIN scan_archives a ON a.scan_no = b.scan_no
                     WHERE a.scan_id = ?1 AND b.sha256 = ?2",
                    params![scan_id, sha256.as_slice()],
                    read_body,
                )
                .optional()
        })
        .await
    }

    async fn save_ad_links(&self, scan_id: &str, links: Vec<RequestAdLink>) -> RepoResult<()> {
        let scan_id = scan_id.to_string();
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            if let Some(scan_no) = scan_no(&transaction, &scan_id)? {
                let mut update = transaction.prepare_cached(
                    "UPDATE requests SET ad = ?3 WHERE scan_no = ?1 AND seq = ?2",
                )?;
                for link in &links {
                    update.execute(params![
                        scan_no,
                        link.seq,
                        stored_ad(Some(&link.link), &link.bid_units)?
                    ])?;
                }
            }
            transaction.commit()
        })
        .await
    }

    async fn remove(&self, scan_id: &str) -> RepoResult<bool> {
        let scan_id = scan_id.to_string();
        self.call(move |connection| {
            let removed =
                connection.execute("DELETE FROM scan_archives WHERE scan_id = ?1", [scan_id])? > 0;
            forget_unused_hosts(connection)?;
            drop_deleted_pages(connection);
            Ok(removed)
        })
        .await
    }

    async fn mark_evicted(&self, scan_id: &str, keep_rows: bool) -> RepoResult<()> {
        let scan_id = scan_id.to_string();
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            if let Some(scan_no) = scan_no(&transaction, &scan_id)? {
                transaction.execute("DELETE FROM scan_bodies WHERE scan_no = ?1", [scan_no])?;
                if keep_rows {
                    transaction.execute(
                        "UPDATE requests SET body_no = NULL,
                             body_state = CASE WHEN body_state IN (?2, ?3) THEN ?4 ELSE body_state END
                         WHERE scan_no = ?1",
                        params![
                            scan_no,
                            code(&BODY_STATES, BodyState::Kept),
                            code(&BODY_STATES, BodyState::Truncated),
                            code(&BODY_STATES, BodyState::Evicted),
                        ],
                    )?;
                } else {
                    transaction.execute("DELETE FROM requests WHERE scan_no = ?1", [scan_no])?;
                    forget_unused_hosts(&transaction)?;
                }
                transaction.execute(
                    "UPDATE scan_archives SET state = ?2, pack_file = NULL, pack_bytes = 0, body_count = 0,
                         request_count = (SELECT count(*) FROM requests WHERE scan_no = ?1)
                     WHERE scan_no = ?1",
                    params![scan_no, state_name(ArchiveState::Evicted)],
                )?;
            }
            transaction.commit()?;
            drop_deleted_pages(connection);
            Ok(())
        })
        .await
    }

    async fn all(&self) -> RepoResult<Vec<ArchiveRecord>> {
        self.call(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {ARCHIVE_COLUMNS} FROM scan_archives ORDER BY created_at, scan_no"
            ))?;
            statement
                .query_map([], read_archive_row)?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .await
    }

    async fn storage(&self, limit_bytes: f64) -> RepoResult<ArchiveStorage> {
        self.call(move |connection| {
            connection.query_row(
                "SELECT count(*) FILTER (WHERE state IN ('sealed', 'writing')),
                        count(*) FILTER (WHERE state = 'evicted'),
                        coalesce(sum(pack_bytes), 0),
                        min(created_at) FILTER (WHERE state = 'sealed')
                 FROM scan_archives",
                [],
                |row| {
                    Ok(ArchiveStorage {
                        scans: row.get(0)?,
                        evicted_scans: row.get(1)?,
                        stored_bytes: row.get::<_, i64>(2)? as f64,
                        limit_bytes,
                        oldest_at: row.get(3)?,
                    })
                },
            )
        })
        .await
    }

    async fn compact(&self) -> RepoResult<()> {
        self.call(|connection| connection.execute_batch("VACUUM"))
            .await
    }
}

/// The scan's network record as the report shows it; `None` when the scan
/// has no archive row at all.
pub(super) fn network_info(
    connection: &Connection,
    scan_id: &str,
) -> rusqlite::Result<Option<adscan_model::NetworkArchiveInfo>> {
    Ok(read_archive(connection, scan_id)?.map(|archive| archive.info()))
}

/// Host names are shared by the requests of all scans. One that no request
/// names any more would still say where a deleted scan went.
pub(super) fn forget_unused_hosts(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute(
        "DELETE FROM hosts WHERE NOT EXISTS (SELECT 1 FROM requests WHERE requests.host_id = hosts.id)",
        [],
    )?;
    Ok(())
}

/// After scan data was deleted: the write-ahead log still holds the pages
/// as they were before. Folding it into the database and emptying it is
/// what makes the deletion true of the files. Best effort: a reader in
/// another process can hold it back, and the next deletion catches up.
pub(super) fn drop_deleted_pages(connection: &Connection) {
    let _ = connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
}

fn scan_no(connection: &Connection, scan_id: &str) -> rusqlite::Result<Option<i64>> {
    connection
        .query_row(
            "SELECT scan_no FROM scan_archives WHERE scan_id = ?1",
            [scan_id],
            |row| row.get(0),
        )
        .optional()
}

fn read_archive(connection: &Connection, scan_id: &str) -> rusqlite::Result<Option<ArchiveRecord>> {
    connection
        .query_row(
            &format!("SELECT {ARCHIVE_COLUMNS} FROM scan_archives WHERE scan_id = ?1"),
            [scan_id],
            read_archive_row,
        )
        .optional()
}

fn read_archive_row(row: &Row) -> rusqlite::Result<ArchiveRecord> {
    let state: String = row.get(1)?;
    let headers_mode: String = row.get(6)?;
    Ok(ArchiveRecord {
        scan_id: row.get(0)?,
        state: [
            ArchiveState::Writing,
            ArchiveState::Sealed,
            ArchiveState::Evicted,
            ArchiveState::Disabled,
        ]
        .into_iter()
        .find(|candidate| state_name(*candidate) == state)
        .ok_or_else(|| unreadable(1, format!("unknown archive state {state}")))?,
        pack_file: row.get(2)?,
        pack_bytes: row.get::<_, i64>(3)? as u64,
        request_count: row.get(4)?,
        body_count: row.get(5)?,
        headers_redacted: headers_mode == "redacted",
        truncated: row.get(7)?,
        created_at: row.get(8)?,
    })
}

fn read_body(row: &Row) -> rusqlite::Result<BodyIndexRow> {
    let sha256: Vec<u8> = row.get(1)?;
    let kind: u8 = row.get(2)?;
    Ok(BodyIndexRow {
        body_no: row.get(0)?,
        sha256: sha256
            .try_into()
            .map_err(|_| unreadable(1, "a body hash is not 32 bytes".into()))?,
        kind: BodyKind::from_code(kind)
            .ok_or_else(|| unreadable(2, format!("unknown body kind {kind}")))?,
        raw_len: row.get::<_, i64>(3)? as u64,
        pack_offset: row.get::<_, i64>(4)? as u64,
        pack_length: row.get(5)?,
        truncated: row.get(6)?,
    })
}

fn read_request(row: &Row) -> rusqlite::Result<RequestIndexRow> {
    let flags: i64 = row.get(17)?;
    let ad: Option<StoredAd> = row
        .get::<_, Option<String>>(32)?
        .as_deref()
        .map(from_json)
        .transpose()?;
    let (ad, bid_units) = ad.map_or((None, Vec::new()), |ad| (Some(ad.link), ad.bid_units));
    Ok(RequestIndexRow {
        seq: row.get(0)?,
        page: row.get(1)?,
        context: decode(&CONTEXTS, row, 2)?,
        started_ms: row.get(3)?,
        ttfb_ms: row.get(4)?,
        duration_ms: row.get(5)?,
        method: name_of(&METHODS, row.get(6)?, "OTHER"),
        scheme: name_of(&SCHEMES, row.get(7)?, "other"),
        host: row.get(8)?,
        site: row.get(9)?,
        path: row.get(10)?,
        query_len: row.get(11)?,
        kind: decode(&KINDS, row, 12)?,
        mime: row.get(13)?,
        status: row.get(14)?,
        end: decode(&ENDS, row, 15)?,
        error: row.get(16)?,
        third_party: flags & FLAG_THIRD_PARTY != 0,
        no_hb_targeting: flags & FLAG_NO_HB_TARGETING != 0,
        served_from: decode(&SERVED_FROM, row, 18)?,
        encoded_bytes: row.get::<_, i64>(19)? as u64,
        decoded_bytes: row.get::<_, i64>(20)? as u64,
        post_bytes: row.get::<_, i64>(21)? as u64,
        initiator_kind: decode(&INITIATORS, row, 22)?,
        redirect_from: row.get(23)?,
        preflight_of: row.get(24)?,
        bidder: row.get(25)?,
        category: decode(&CATEGORIES, row, 26)?,
        body_state: decode(&BODY_STATES, row, 27)?,
        body_no: row.get(28)?,
        record_offset: row.get::<_, i64>(29)? as u64,
        record_length: row.get(30)?,
        record_index: row.get(31)?,
        ad,
        bid_units,
    })
}

/// The `ad` column: the link and the bid ids it was made with.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredAd {
    link: AdLink,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    bid_units: Vec<(String, String)>,
}

fn stored_ad(
    link: Option<&AdLink>,
    bid_units: &[(String, String)],
) -> rusqlite::Result<Option<String>> {
    link.map(|link| {
        to_json(&StoredAd {
            link: link.clone(),
            bid_units: bid_units.to_vec(),
        })
    })
    .transpose()
}

fn state_name(state: ArchiveState) -> &'static str {
    match state {
        ArchiveState::Writing => "writing",
        ArchiveState::Sealed => "sealed",
        ArchiveState::Evicted => "evicted",
        ArchiveState::Disabled => "disabled",
    }
}

fn code<T: PartialEq + Copy>(order: &[T], value: T) -> i64 {
    order
        .iter()
        .position(|candidate| *candidate == value)
        .map_or(-1, |position| position as i64)
}

fn decode<T: Copy>(order: &[T], row: &Row, column: usize) -> rusqlite::Result<T> {
    let code: i64 = row.get(column)?;
    usize::try_from(code)
        .ok()
        .and_then(|position| order.get(position).copied())
        .ok_or_else(|| unreadable(column, format!("unknown code {code}")))
}

fn name_code(names: &[&str], name: &str, other: i64) -> i64 {
    names
        .iter()
        .position(|candidate| candidate.eq_ignore_ascii_case(name))
        .map_or(other, |position| position as i64)
}

fn name_of(names: &[&str], code: i64, other: &str) -> String {
    usize::try_from(code)
        .ok()
        .and_then(|position| names.get(position).copied())
        .unwrap_or(other)
        .to_string()
}

fn unreadable(column: usize, message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Integer,
        message.into(),
    )
}
