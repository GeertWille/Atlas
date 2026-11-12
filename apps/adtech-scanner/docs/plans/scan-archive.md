# Scan archive: storage for scripts, requests, responses and bid payloads

Status: design, 2026-11-12. Nothing in the repo was changed. Scripts and raw outputs are under
`/tmp/claude-0/-home-user/ee93eb55-ef69-5991-bc2c-57c8a322dcdf/scratchpad/storage/` (`scripts/`, `work/*.txt`).

## Recommendation in short

1. **One pack file per scan**, in the owner's data-lake format: independently zstd-compressed frames
   appended to one file, addressed by a location `file#o=<offset>&l=<length>`. Local adapter is
   "open, seek, read"; an S3 adapter later is a Range GET. Deleting a scan is one unlink. The pack plus
   a few appended report frames is the export bundle.
2. **SQLite stays the only database.** It gains a `requests` table (one row per request, host
   dictionary, no query strings, no headers) and a `scan_bodies` table (sha-256 to byte range). Headers,
   full URLs and POST bodies live in the pack.
3. **No cross-scan dedupe in v1, no hybrid.** Measured cost: a repeat scan stores about 4 MB where a
   content-addressed store would add about 1 MB. A 5 GB default cap still holds roughly 1,000 scans.
   The schema keeps the sha-256 of every body, so dedupe can be added later without a format change.
4. **No S3 server, no `object_store` dependency, no DuckDB now.** A small own `PackStore` port shaped
   around locations; Parquet export later for analysts.
5. **Retention on by default** with a switch, a size limit (5 GB) and redaction of `Cookie`,
   `Set-Cookie`, `Authorization` and `Proxy-Authorization` values.

What is measured and what is assumed is marked throughout. Measurements ran in a Linux container
(4 cores, Python 3.13 harness, python-zstandard 0.25 / libzstd 1.5.7, SQLite 3.53.1, DuckDB 2.0.0). Absolute times
on a Mac will differ; ratios and sizes will not.

## The data behind the numbers

I did not use synthetic bodies. `scripts/capture.mjs` drives the local Chromium over raw CDP (Network
domain on the page and on every auto-attached iframe, worker and service worker session), scrolls for
30 s and saves every request with headers, POST data and response body. Eight captures of five ad-heavy
pages, three of them captured twice a few minutes apart:

| Capture (one page view, 30 s) | Requests | Hosts | In iframe/worker sessions | Kept-class bodies, unique | Raw MB | zstd -3 MB |
|---|---|---|---|---|---|---|
| bbc.com/news | 345 | 58 | 24% | 213 | 10.8 | 3.2 |
| theguardian.com/international | 979 | 183 | 42% | 388 | 9.6 | 2.6 |
| cnet.com (two runs) | 1,219 / 1,225 | 252 | 50% | 257 | 12.9 | 3.6 |
| independent.co.uk (two runs) | 1,402 / 1,674 | 300 | 75% | 200 / 219 | 14.5 / 14.8 | 4.0 / 4.2 |
| weather.com (two runs) | 1,209 / 1,231 | 290 | 54% | 378 | 18.4 | 5.3 |

"Kept class" means JS, HTML, JSON, text/XML, CSS and POST bodies. Other facts from the same captures
(`work/` outputs of `analyze.py`, `per_scan.py`):

- JavaScript is 85 to 94% of the compressed kept bytes. HTML, JSON, text, CSS and POST bodies together
  are 0.3 to 0.5 MB per page view.
- Images, video and fonts would add 1 to 11 MB per page view compressed (they do not compress) and a
  third of the CDP traffic. They carry nothing for ad debugging.
- Headers are 1.6 to 2.3 KB of JSON per request, 2.2 to 2.7 MB raw per page view.
- URLs average 170 to 270 bytes; 4 to 30 per page view exceed 2,000 characters.
- 20 to 33% of requests on the four ad-heavy pages are redirect hops (cookie-sync chains); 2% on bbc.com. 80 to 240 per page view finish with an
  empty body.
- No WebSocket was opened on any of the five pages.

Limits of this corpus: five English-language news and utility pages, no consent click, no login, one
page view each, a US exit. A downloaded corpus (prebid, gpt.js, pubads_impl, apstag, OneTrust, npm
libraries) is in `downloads/` and was used only for the release-to-release test in B.

## A. What to capture and keep

### Scope

| Data | Keep | Where |
|---|---|---|
| Every request: method, host, path, status, type, MIME, sizes, timing, flags, page view, context, bidder | Always | SQLite row |
| Full URL, request and response headers, initiator, redirect link, remote address | Always (headers redacted by default) | Pack, record frame |
| POST body | Text bodies up to 1 MiB, inline in the record | Pack, record frame |
| Response body: JS, HTML documents, JSON, text/XML, CSS | Yes, up to 5 MiB each | Pack, body frame |
| Inline and runtime scripts (`Debugger.getScriptSource`) | Yes, as today's capture already reads them | Pack, body frame |
| Images, video, audio, fonts, other binary | Metadata only: sizes and MIME type | SQLite row |
| WebSocket frames, EventSource streams | No. One row per socket | SQLite row |

Two corrections to the brief's wording:

- **No hash for bodies that are not kept.** A hash needs the body, and fetching image and media bodies
  over CDP is what costs a third of the traffic. Size and type are free.
- **CSS is kept.** It measured at most 0.07 MB per page view compressed, and a Network view with a
  missing class of text bodies reads as broken.

### Caps (defaults)

| Cap | Value | Basis |
|---|---|---|
| Per body | 5 MiB, truncated and flagged | Largest body seen: 1.7 MB (JS), 1.5 MB (HTML). Same as today's `MAX_SCRIPT_BYTES`. |
| Per POST body | 1 MiB, truncated and flagged | Largest seen: 140 KB. |
| Per scan | 256 MiB of raw kept bodies; after that metadata only, flagged | 10 to 18 MB raw per page view measured, six page views maximum, manual visits up to 15 minutes. |
| Total | 5 GB of pack files, oldest evicted first | See "Disk use" in E. |

### How to read it over CDP

Measured in the eight captures, bodies requested at `Network.loadingFinished`:

- **Response bodies of kept classes: no real failures.** Of 86 `getResponseBody` errors in 5,500
  finished requests, 76 were CORS preflights (which never have a body), 6 fonts and 4 images.
- **Asking late loses bodies.** Re-requesting every body at the end of the 30 s visit failed for 2.5 to
  16% of requests (cnet: 115 of 739), because an iframe that navigates or is removed takes its
  resources with it. The current recorder's "ask the moment loading finishes" rule is right and must
  be kept.
- **POST data: over 99%.** 55 to 100% of POST bodies arrive inline in `requestWillBeSent` (with
  `maxPostDataSize` set to 1 MiB in `Network.enable`); the rest came from
  `Network.getRequestPostData`, with 4 misses in about 800.
- **Memory-cached responses** returned bodies normally (185 of 191). Disk-cached ones too.
- **Cookies are only visible in the `ExtraInfo` events.** `requestWillBeSentExtraInfo` and
  `responseReceivedExtraInfo` fired for 64 to 92% of requests and must be merged into the record.

Rules that follow:

- **Network domain, not Fetch.** Fetch pauses every request and resumes it from the app, which adds
  latency to exactly the timings this product reports. Fetch stays limited to what it does today
  (proxy password, first document).
- **Enable Network on child sessions.** Today child targets are "set up when they attached, then left
  alone" (`capture/page.rs`), so requests made inside out-of-process iframes and workers are invisible.
  That was 24 to 74% of all requests in the captures, and it is where ad creatives and many bid and
  sync calls live. This is the largest change in the browser crate. Request ids are unique per
  session, so the key is (session, request id).
- **Redirects.** Each hop is its own row (same request id, `redirectResponse` on the next
  `requestWillBeSent`), linked by `redirect_from`. A 3xx has no body.
- **Service workers.** Their own fetches arrive on the service worker session and get
  `context = service_worker`; page requests answered by a worker get a flag.
- **`data:` and `blob:` URLs.** A row with the URL reduced as `without_embedded_payload` does today,
  no body.
- **WebSockets and EventSource.** Skipped beyond one row per connection. Revisit only if a real
  publisher stack needs it.
- **Bid endpoints.** `bid_request()` already recognises them; the row gets the bidder code, and
  `ObservedBidResponse` gains the request's `seq` so the bid inspector opens the right record.

What cannot be captured: bodies of redirects and preflights, bodies of loads that fail or are
cancelled, responses that never finish (long polls, media streams), anything evicted because one
response exceeds `maxResourceBufferSize` (32 MB today), requests sent while the page unloads whose
answer never reaches the renderer, and requests of other tabs or of the browser itself.

Cost, measured: fetching every body including images moved 31 to 87 MB over the CDP socket per page
view; kept classes are two thirds of that (about 18 MB per page view). Hashing and
compressing 18 MB takes under 0.2 s of one core (zstd -3 at 206 MB/s measured; sha-256 speed assumed).
`getResponseBody` answered in 6 ms to 1.3 s at the median and up to 9 s under bursts in my Node
client; that delayed nothing on the page and lost no kept body. Whether it slows page load was not
measured reliably (load times varied more between runs than between modes).

### Sensitive data

| Item | Default | Why |
|---|---|---|
| `Cookie`, `Set-Cookie`, `Authorization`, `Proxy-Authorization` values | Redacted. Header name, cookie names and value lengths are kept (`_ga=[27 bytes]`) | These are session credentials. Names and lengths are what ad debugging needs (was the ID cookie sent, how large). A `Cookie` header was on 5 to 36% of requests. |
| Other headers, including `x-api-key`, `Referer`, `User-Agent` | Kept | Needed to reproduce a bid request. |
| URLs, including tokens and ids in query strings | Kept as captured, in the pack only. SQLite holds host and path. | A sync or bid URL without its query string is useless. |
| POST bodies and response bodies | Kept as captured | The bid inspector is the point. |
| TC string | Kept where it appears in URLs and bodies | It is a consent record, not a credential, and the consent checks need it. The handover sentence "the string itself is never captured" must change. |
| Document HTML and inline scripts | Kept | A manual visit or a persistent profile can make this signed-in content. The wording must say so. |

One Settings switch, "Store full headers", off by default, removes the redaction for new scans. Each
archive records which mode it was written in. Redaction is a pure function in the engine's domain
layer, applied before anything reaches disk; the browser crate hands over full headers in memory.
Measured effect of redaction on size: 12 to 15% of raw record bytes.

**Retention on by default: yes.** The owner accepted local storage, and the viewer, Network view and
bid inspector are all empty without it. Controls: a switch "Keep network data with reports", the size
limit, an optional "delete network data older than N days" (off), and "Clear network data".

## B. Blob storage layout

### What the owner's platform does (read-only look at `clew-app`)

`packages/server/src/services/data-lake/DataLakeService.ts`:

- A batch is immutable: each input becomes one JSON line (`{_v:1, id, fullUrl, headers, postData,
  responseHeaders, responseContent}` for requests), compressed alone at zstd level 3. The frames are
  concatenated and stored as one object with content type `application/zstd`.
- The object key is `requests/<organizationId>/<auditId>/<scanId>/<batchId>.batch.zst`; the batch id
  is the first 32 hex characters of the sha-256 over the frames.
- A location is `<stored file location>#o=<offset>&l=<length>` (`composeDataLakeBatchLocation`,
  `parseDataLakeLocation`); length 0 is invalid; no fragment means the whole file.
- `readAsJson` does a Range GET `bytes=o-(o+l-1)`, fails when the byte count differs, decompresses and
  parses. Locations are stored per row (`dataLakeLocation`, plus the store name).
- Deleting a scan is deleting the key prefix. There is no dedupe. Response-content capture is still
  switched off there (`CAPTURE_RESPONSE_CONTENT = false`, JavaScript MIME types only).

### Measurements

Six real scans (three sites, each twice), unique kept bodies compressed once at zstd -3
(`bench_layout.py`). Target for the read test: a 380 KB script.

| Layout | Disk for the 6 scans | Write, all 6 | Read 380 KB script, warm / cold | Delete 3 scans | Notes |
|---|---|---|---|---|---|
| 1. Content-addressed BLOBs in a SQLite file | 16.2 MB | 0.26 s (transaction per body), 0.15 s (per scan) | 0.44 ms / 8.8 ms | 46 ms, then a vacuum step | Transactional. One large file that changes on every scan. |
| 2. Content-addressed loose files, `ab/<sha>` | 18.0 MB allocated (14.3 MB of data, 1,246 files) | 0.12 s, or 0.58 s with fsync per file | 0.32 ms / 1.4 ms | 48 ms for 432 unlinks, after a reference query | 26% lost to block rounding. Orphans need a sweep. |
| 3. Pack per scan, independent frames + index | 25.9 MB | 0.06 s, one fsync per scan | 0.33 ms / 1.8 ms | 8 ms for 3 unlinks | No cross-scan dedupe. Whole file is a valid multi-frame zstd file (checked by stream-decoding it). |
| 3x. Packs that may reference frames in older packs | 15.9 MB | as 3 | as 3 | Needs reference counts or compaction | Same bytes as 1. |
| 3h. Hybrid: scripts content-addressed, the rest in the scan's pack | 14.8 MB | between 1 and 3 | as 1 or 3 | Both mechanisms | |
| 4. One solid zstd stream per scan | 22.9 MB | 0.44 s | 17 ms and 18 MB of RAM for any single body | 3 unlinks | Not range-readable: any read downloads and decodes the whole scan. |

(The SQLite vacuum figure in my run is not valid: the harness did not step `incremental_vacuum` to
completion. I did not rerun it because the layout is not recommended.)

Reading: every layout reads a script in under half a millisecond warm. Writing: every layout stores a
scan in well under a second. Neither criterion separates them. What separates them is dedupe,
deletion, and how many moving parts there are.

**Why not one big stream (4).** It is 12% smaller than independent frames at level 3 (a shared window
helps), but a single read costs the whole scan, and on S3 a whole-object download. With `--long=27`
at level 19 a solid stream reaches 6.3x against 4.0x for frames, which is the most that this route
could ever give. Not worth losing range reads. The owner's layout is 3, not 4.

### Dedupe: what a per-scan pack gives up

Second scan of the same page, minutes later (the best case for dedupe):

| Pair | Compressed, second scan | Already stored by the first | JS shared | Everything else shared |
|---|---|---|---|---|
| independent.co.uk | 4.17 MB | 85% | 92% | 1 to 14% |
| weather.com | 5.27 MB | 72% | 76% | 0 to 34% |
| cnet.com | 3.52 MB | 75% | 78% | 50% |

So a content-addressed store adds about 0.6 to 1.5 MB per repeat scan where a pack adds 3.5 to 5.3 MB.
Over many repeat scans of one site that is a factor of about 4. Only scripts matter: everything else
is under 0.5 MB per scan and mostly unique.

The saving shrinks with time between scans. Two releases of Google's `pubads_impl.js` four days apart
share nothing as whole files. Content-defined chunking does not rescue that: 1% of bytes shared
between the two releases, because the minifier renames everything (54% between two Prebid cores built
from adjacent versions, 0% across a major version). **Chunk-level dedupe is rejected.** Delta
compression against the old version works (244 KB down to 60 KB) but makes a frame depend on another
frame, which breaks range reads and deletion. Rejected too.

Dedupe inside one scan is free in every layout and large: 9 to 43% of raw kept bytes are repeats
(the same script loaded by several iframes). The pack writer keeps a sha-256 set per scan.

### Compression settings

Per-body frames, 1,371 unique bodies, 58.3 MB raw (`bench_compress.py`), single thread:

| Codec | Ratio | Compress MB/s | Decompress MB/s |
|---|---|---|---|
| zstd -1 | 3.26 | 309 | 948 |
| **zstd -3** | **3.54** | **206** | **998** |
| zstd -6 | 3.78 | 82 | 963 |
| zstd -9 | 3.85 | 51 | 969 |
| zstd -12 | 3.88 | 20 | 1,079 |
| zstd -19 | 4.04 | 3.1 | 995 |
| gzip -6 | 3.49 | 34 | 251 |
| brotli -5 | 3.89 | 40 | 291 |
| brotli -11 (4 MB JS sample; zstd -19 on it: 3.63) | 3.90 | 0.5 | 235 |

By class at level 3: JS 3.4x, HTML 7.7x, CSS 6.4x, JSON 4.5x, text 4.2x, POST bodies 2.75x.

- **Level 3.** It is the platform's level, costs under 0.1 s per page view, and level 9 buys 8% for
  four times the CPU. Level 19 buys 12% at 3 MB/s. Brotli -5 matches zstd -9 on ratio but decodes 3.4
  times slower and is not the platform's codec. Keep it one constant.
- **Long-distance mode: no.** On single bodies of 1 MB and more it saved 1% (1.66 to 1.64 MB). It only
  pays on solid streams.
- **Dictionary for small bodies: no.** Trained on four sites and tested on the fifth, a 110 KB
  dictionary saved 28 KB on the 448 KB of bodies up to 16 KB (2.46x to 2.91x). Bodies up to 16 KB are
  75% of the count but 4% of the bytes. A dictionary would also have to be shipped, versioned and
  uploaded with every pack.
- **Request records are the exception, and batching solves it without a dictionary.** One frame per
  request compresses headers only 1.8x (1.2 MB per page view). 128 requests per frame gives 8.5x
  (0.26 MB), better than one frame per request with a dictionary (4.4 to 5.4x). Decoding a 128-request
  frame takes 0.06 ms.
- **Enable the zstd frame checksum.** It costs 4 bytes per frame and catches a damaged range.

### Verdict

**Per-scan pack, layout 3, without cross-pack references and without a hybrid.**

- It is the owner's format, so a sealed pack can be uploaded as one object and read by location.
- It has the fewest states: no reference counts, no orphan blobs, no garbage collector, no vacuum.
  Deleting a scan removes its bytes at once, which is also the only honest meaning of "delete" now
  that packs contain page HTML and bid payloads.
- Sealed packs never change, so Time Machine copies each once. A SQLite blob file is recopied whole
  after every scan.
- Its cost is disk: about 4x on long series of repeat scans. At 3 to 6 MB per page view a 5 GB limit
  holds roughly 600 to 1,500 scans, and the limit is a setting.

A hybrid is not worth it now. It would bring back reference counting and a second store to save disk
that a retention limit already bounds. If disk does become the complaint, the route is: script bodies
(external `GET` scripts and stylesheets only, never documents, inline scripts or payloads) go to
shared packs under `archive/shared/`, a `scan_bodies` row then names that pack instead of the scan's
own, a shared pack is deleted when no scan references it and rewritten when more than half of it is
dead. `scan_bodies` already carries what that needs except a pack column. Trigger: users at the limit
whose scans are mostly repeats of a few sites.

## C. The S3-compatible API idea

**(i) A local S3 server (MinIO or similar): no.** It would add a second process to start, supervise
and stop, a listening port, credentials, a large server binary with its own licence terms inside a
signed desktop bundle, and an HTTP round trip for what is a `pread`. The platform does have a MinIO adapter beside its S3
one, but that is a server deployment choosing between two object stores; its test double is an
in-memory map behind the same interface. The desktop app needs that interface, not the wire protocol.

**(ii) What keeps the door open: a port shaped like the platform's interface.**

```rust
/// `scans/<scan id>.pack.zst#o=<offset>&l=<length>`; same syntax as the platform's data lake location.
pub struct Location { pub file: String, pub offset: u64, pub length: u32 }

#[async_trait]
pub trait PackStore: Send + Sync {
    async fn create(&self, file: &str) -> Result<Box<dyn PackWriter>, ArchiveError>;
    /// Exactly `location.length` bytes, or an error. Never a short read.
    async fn read(&self, location: &Location) -> Result<Vec<u8>, ArchiveError>;
    async fn copy_to(&self, file: &str, target: &Path) -> Result<u64, ArchiveError>;
    async fn delete(&self, file: &str) -> Result<(), ArchiveError>;
    async fn list(&self) -> Result<Vec<PackInfo>, ArchiveError>;
}

#[async_trait]
pub trait PackWriter: Send {
    /// `frame` is one complete zstd frame.
    async fn append(&mut self, frame: Vec<u8>) -> Result<Location, ArchiveError>;
    async fn sync(&mut self) -> Result<(), ArchiveError>;
    async fn seal(self: Box<Self>) -> Result<PackInfo, ArchiveError>;
}
```

`LocalPackStore` is about 150 lines of `std::fs` on blocking threads. An `S3PackStore` later maps
`read` to a Range GET, `delete` and `list` to their S3 calls, and `create`/`append`/`seal` to a
multipart upload, or more simply uploads sealed local packs.

**`object_store` crate: not now, perhaps as the body of the S3 adapter later.** Facts from crates.io
today: version 0.14.2 (2026-09-15), 109 million downloads, features `fs`, `aws`, `gcp`, `azure`,
`http`; it depends on `reqwest ^0.13` (the engine's version), `chrono`, `itertools`, `parking_lot`,
and with `aws` on `quick-xml`, `hyper`, `ring` or `aws-lc-rs`. It is async on tokio and has
`get_range`. Reasons not to adopt it as the port: its local store has no append, so the capture
writer would bypass it anyway; its `Path`, `PutPayload` and error types would leak into the
application layer; and five methods do not need an abstraction library. Compile time was not
measured. `opendal` (0.59.5) is the same answer with more surface.

**(iii) Range reads.** The index is the seek table: SQLite holds offset and length of every frame,
and the pack ends with a manifest that repeats them. The zstd seekable format solves a different
problem (random access into one stream cut into fixed-size frames) and is not needed. Its Rust status,
for the record: `zstd-safe` and `zstd-sys` 2.1.2 have a `seekable` feature over the upstream contrib
code; `zeekstd` 0.6.3 (2026-10-19) is a maintained Rust implementation; `zstd-seekable` 0.1.23 has not
been updated since 2023.

**What "sync scans to S3" would need later, and what today's design already fixes:**

- Key layout: `requests/<organizationId>/<auditId>/<scanId>/<batchId>.batch.zst` on the platform. The
  local file name is `scans/<scanId>.pack.zst`; locations are stored as (offset, length) integers per
  row and composed with the file name at the port, so a new prefix changes no row.
- A manifest per scan: already the last frame of every pack.
- Idempotent upload: packs are immutable once sealed; upload with "only if absent".
- Encryption: bucket-side encryption first. If client-side is required, encrypt per frame so ranges
  stay readable; not designed here.
- Credentials: Keychain items like the proxy passwords (`s3-secret:<profile id>`), never in SQLite.
- Product rules: the "local only" hard rule and the About host list change on that day, not before.

**Differences from the platform's batches, stated plainly:**

| | Platform batch | Local pack | Why |
|---|---|---|---|
| Files per scan | Several immutable batches, named by content hash | One, append-only during capture, then sealed | A capture is a stream; the hash name can be given at upload. |
| Frame content | One JSON document per request, response body inline as a string | Record frames: up to 128 request documents as JSON lines. Body frames: raw bytes, one per distinct body. | Measured 1.8x against 8.5x on records. Raw bodies serve the viewer directly, are binary-safe and dedupe inside the scan. |
| Request document | `_v, id, fullUrl, headers, postData, responseHeaders, responseContent` | The same seven fields with the same meaning, plus the fields in E. `responseContent` is null and `body.location` points at the body frame. | Kept compatible on purpose. |

A platform reader therefore needs two small additions: split a record frame into lines, and a
`readBytes(location)` beside `readAsJson`. I could not test that against the platform.

## D. The database question

### Volume

Measured: 345 to 1,674 requests per 30 s page view with iframes and workers included. A scan has one
to six page views, so 1,000 to 10,000 rows per scan is right, on the high side for browse visits.
Ten thousand scans is nine years at three scans a day. A heavy user after two years is nearer 3 to 10
million rows; 100 million is the far tail.

### SQLite at 5.1 million rows

`bench_db.py`: 1,700 scans of 3,000 rows sampled from the real requests, 8,147 hosts, table clustered
by scan (`PRIMARY KEY (scan_no, seq) WITHOUT ROWID`), an index on `(host_id, scan_no)` and a partial
index on the body hash.

| | Host id + path (recommended) | With a full URL column |
|---|---|---|
| File size | 0.71 GB, 138 bytes per row | 2.58 GB, 506 bytes per row |
| One scan, first 200 rows | 0.4 ms | 0.5 ms |
| One scan, all 3,000 rows, list columns | 3.1 ms | 3.4 ms |
| One scan, sort by size, top 200 | 0.5 ms | 0.6 ms |
| One scan, `path LIKE '%prebid%'` | 0.3 ms | 0.4 ms |
| One scan, group by host | 1.0 ms | 1.1 ms |
| All scans that called `*.rubiconproject.com` (1,488 scans) | 14 ms | 13 ms |
| 500 newest requests to that host across scans | 4.5 ms | 12 ms |
| Every scan that loaded one exact body (by sha-256) | 0.07 ms | 0.07 ms |
| Whole table: top 20 hosts by count | 211 ms | 343 ms |
| Whole table: `path LIKE '%fastlane%'` | 725 ms | 1,185 ms |
| Whole table: sum of bytes by type | 2,633 ms | 3,790 ms |
| Delete one scan (3,000 rows) | 7 ms | 10 ms |
| Insert rate, Python generating rows | 71,000 rows/s | 47,000 rows/s |

Everything a Network view or a cross-scan host question does is indexed and takes milliseconds.
Only whole-table aggregates are slow, and they grow linearly: about 50 s at 100 million rows
(extrapolated). Size at 100 million rows: about 14 GB (extrapolated). Keeping query strings out of
SQLite is worth a factor of 3.7 in size.

**URL full-text index: no.** Inside one scan `LIKE` over 3,000 rows takes 0.3 ms. Across scans the
questions are by host (indexed) or by exact body (indexed). An FTS5 trigram index over paths would
cost more than the table to answer a question nobody has asked. Not measured.

### DuckDB

Same 5.1 million rows (`bench_duckdb.py`). The synthetic rows repeat more than real ones, which
flatters columnar compression.

| | SQLite | DuckDB table | Parquet (zstd) read by DuckDB |
|---|---|---|---|
| Size | 705 MB | 210 MB | 132 MB |
| One scan, all rows | 3.1 ms | 3.2 ms | 4.8 ms |
| Scans that called the host | 14 ms | 26 ms | 94 ms |
| Top 20 hosts, whole table | 211 ms | 21 ms | 40 ms |
| `LIKE` over all paths | 725 ms | 10 ms | not run |
| Sum by type, whole table | 2,633 ms | 18 ms | not run |
| 3,000 single-row inserts in one transaction | under 50 ms (from the insert rate) | 3,666 ms through the Python SQL API (the appender API is the fast path; not measured) | n/a |

Checked facts:

- **Attaching a SQLite file needs an extension that is not built in.** In DuckDB 2.0.0 (Python wheel)
  `sqlite_scanner` is `NOT_INSTALLED`; `ATTACH ... (TYPE sqlite)` fails with "Install it first using
  INSTALL sqlite_scanner", which is a download. The Rust crate's features on crates.io (`bundled`,
  `parquet`, `json`, `icu`, `httpfs`, `extensions-full`, ...) list no SQLite scanner, so I infer the
  same there; not built and verified. A local-only app must not download extensions at run time, so
  that path would mean building and shipping the extension ourselves.
- **Weight.** `libduckdb-sys` is an 8.1 MB crate of C++ sources compiled by the `bundled` feature. The
  Python extension module is 67.6 MB on x86-64. Expect tens of MB on the app binary and a build that
  is minutes longer per target (not measured; from the size of the amalgamation). A universal macOS
  build compiles it twice. Not verified on macOS.
- **Concurrency.** One process may open a database read-write; inside it, MVCC with optimistic
  conflicts. Fine for one app, but it is a second store with its own file, WAL and checkpoint rules
  beside SQLite.
- Parquet is built in. The crate was last published on 2026-10-28.

### Options and verdict

| Option | Verdict |
|---|---|
| 1. SQLite for everything, normalised `requests` plus a hosts table | **Recommended.** Measured above. One store, one transaction model, one delete path. |
| 2. SQLite for app state, DuckDB for request facts | No. Two engines to keep consistent on delete and crash, a much larger binary, and the queries the product needs are already milliseconds. |
| 3. SQLite plus per-scan Parquet read by DuckDB on demand | No. Same binary cost; cross-scan queries over thousands of small Parquet files are worse than one indexed table. |
| 4. SQLite now, "Export requests as Parquet" later | **This is the door to keep open**, and it is cheap: the rows are flat, and an analyst can open the export with DuckDB outside the app. Writing Parquet from Rust needs the `parquet` crate, not DuckDB. |

Revisit when the product gets a screen that aggregates over all scans interactively (trends by vendor,
bytes by type over months) and the table is past roughly 20 million rows. The answer then is likely a
small summary table maintained at scan end, before it is a second engine.

**JSON columns for auctions, slots, scripts, findings: keep.** They are read whole per scan, their
shapes change with the contract, and no query filters inside them. The rule stays "a column for what
is filtered or sorted across scans" (`prebid_version` is already one, which answers "which scans of
this site loaded Prebid 9.x" today). Requests are different because they are filtered, sorted and
paged, and counted across scans.

## E. Concrete design

### Files

```
<app data dir>/
  adtech-scanner.sqlite3            existing database, three new tables
  archive/
    scans/<scan id>.pack.zst        one per scan that has network data
  scan-profile/                     existing
```

### Pack format (version 1)

A pack is a sequence of zstd frames, each with the content checksum on, in capture order:

| Frame | Content | Written |
|---|---|---|
| Body | Raw bytes of one distinct body (response body or script source) | When the body arrives |
| Records | Up to 128 request documents as JSON lines, or fewer when 256 KiB of JSON is reached | Every 128 requests or every second |
| Manifest | One JSON document: format version, app version, scan id, URL, start time, header mode, counts, and the offset and length of every body frame (with sha-256, raw length, kind) and every record frame (with first `seq` and count) | When the capture ends |
| Trailer | A zstd skippable frame (magic `0x184D2A50`) holding `ADSCANPK`, the format version and the manifest's offset and length | Last |

The file is a valid `.zst` from the first byte to the last; any zstd tool skips the trailer. A reader
with only the file finds the trailer at a fixed distance from the end, reads the manifest, and can
rebuild every SQLite row by replaying the record frames. That is what makes the pack importable.

One request document (a JSON line; the first seven names are the platform's):

```json
{"_v":1,"id":"<scanId>:412","fullUrl":"https://fastlane.rubiconproject.com/a/api/fastlane.json?...",
 "headers":{"cookie":"khaos=[36 bytes]; audit=[212 bytes]","user-agent":"..."},
 "postData":null,"responseHeaders":{"set-cookie":"[redacted 2 cookies, 310 bytes]","content-type":"application/json"},
 "responseContent":null,
 "seq":412,"page":0,"context":"page","method":"GET","resourceType":"XHR","status":200,
 "mimeType":"application/json","protocol":"h2","remoteAddress":"203.0.113.9:443",
 "startedMs":1834.2,"ttfbMs":181.0,"durationMs":212.4,"timing":{"dns":0,"connect":0,"ssl":0,"send":0.3,"wait":180.2,"receive":31.1},
 "encodedBytes":1812,"bodyBytes":5120,"postBytes":0,
 "initiator":{"type":"script","url":"https://cdn.example/prebid.js","line":1,"column":23451,"seq":17},
 "redirectFrom":null,"flags":["thirdParty","bidEndpoint"],"error":null,"bidder":"rubicon",
 "postTruncated":false,
 "body":{"sha256":"9055f7...","location":"scans/<scanId>.pack.zst#o=1048576&l=1433","bytes":5120,"kind":"json","truncated":false}}
```

POST bodies are inline (`postData`, as on the platform) because they are small (2 to 6 KB on average,
largest seen 140 KB) and are read together with the headers.

### Schema (migration 5)

```sql
CREATE TABLE scan_archives (
    scan_no INTEGER PRIMARY KEY,           -- compact key for the request rows
    scan_id TEXT NOT NULL UNIQUE REFERENCES scans (id) ON DELETE CASCADE,
    state TEXT NOT NULL,                   -- writing | sealed | evicted
    format INTEGER NOT NULL,
    pack_file TEXT,                        -- 'scans/<scan id>.pack.zst'; NULL once evicted
    pack_bytes INTEGER NOT NULL DEFAULT 0,
    raw_bytes INTEGER NOT NULL DEFAULT 0,
    request_count INTEGER NOT NULL DEFAULT 0,
    body_count INTEGER NOT NULL DEFAULT 0,
    headers_mode TEXT NOT NULL,            -- redacted | full
    created_at REAL NOT NULL
);
CREATE INDEX scan_archives_by_age ON scan_archives (state, created_at);

CREATE TABLE hosts (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,             -- 'fastlane.rubiconproject.com'
    site TEXT NOT NULL                     -- registrable domain: 'rubiconproject.com'
);
CREATE INDEX hosts_by_site ON hosts (site);

-- One row per distinct body of a scan: script sources and response bodies.
CREATE TABLE scan_bodies (
    scan_no INTEGER NOT NULL REFERENCES scan_archives (scan_no) ON DELETE CASCADE,
    body_no INTEGER NOT NULL,
    sha256 BLOB NOT NULL,                  -- 32 bytes, of the stored (possibly truncated) raw bytes
    kind INTEGER NOT NULL,                 -- 0 js, 1 html, 2 json, 3 text, 4 css
    raw_len INTEGER NOT NULL,
    pack_off INTEGER NOT NULL,
    pack_len INTEGER NOT NULL,
    truncated INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (scan_no, body_no)
) WITHOUT ROWID;
CREATE UNIQUE INDEX scan_bodies_by_sha ON scan_bodies (sha256, scan_no);

CREATE TABLE requests (
    scan_no INTEGER NOT NULL REFERENCES scan_archives (scan_no) ON DELETE CASCADE,
    seq INTEGER NOT NULL,                  -- order of first sight within the scan
    page INTEGER NOT NULL,                 -- page view index
    context INTEGER NOT NULL,              -- 0 page, 1 iframe, 2 worker, 3 service worker
    started_ms REAL NOT NULL,              -- since the start of its page view, like every other time
    ttfb_ms REAL,
    duration_ms REAL,                      -- NULL when it never finished
    method INTEGER NOT NULL,               -- 0 GET, 1 POST, 2 OPTIONS, 3 HEAD, 4 PUT, 5 DELETE, 6 PATCH, 7 other
    scheme INTEGER NOT NULL,               -- 0 https, 1 http, 2 wss, 3 ws, 4 data, 5 blob, 6 other
    host_id INTEGER NOT NULL REFERENCES hosts (id),
    path TEXT NOT NULL,                    -- no query string, at most 512 characters
    query_len INTEGER NOT NULL,
    resource_type INTEGER NOT NULL,        -- CDP ResourceType in a fixed order
    mime TEXT,
    status INTEGER,                        -- NULL when there was no response
    error TEXT,                            -- net error or blocked reason, at most 80 characters
    flags INTEGER NOT NULL DEFAULT 0,      -- bits below
    encoded_bytes INTEGER,                 -- on the wire
    body_bytes INTEGER,                    -- decoded
    post_bytes INTEGER,
    initiator_type INTEGER NOT NULL,       -- 0 parser, 1 script, 2 preload, 3 redirect, 4 other
    initiator_seq INTEGER,                 -- request that delivered the initiating script, when known
    redirect_from INTEGER,                 -- seq of the previous hop
    bidder TEXT,                           -- Prebid bidder code of a recognised bid endpoint
    body_no INTEGER,                       -- scan_bodies row; NULL when no body was kept
    record_off INTEGER NOT NULL,           -- record frame in the pack
    record_len INTEGER NOT NULL,
    record_idx INTEGER NOT NULL,           -- line within that frame
    PRIMARY KEY (scan_no, seq)
) WITHOUT ROWID;
CREATE INDEX requests_by_host ON requests (host_id, scan_no);
```

Flag bits: third party, memory cache, disk cache, answered by a service worker, failed, blocked,
cancelled, is a redirect hop, has POST data, POST truncated, body truncated, body not kept (class),
body not kept (per-scan cap or backlog), headers redacted.

Notes:

- `scan_no` exists because a 36-character scan id in every row and index would double the table.
- Script records stay JSON in `scripts`. `ScriptRecord` gains `sha256` (full, additive; the 16-character
  `hash` stays). The viewer resolves script to body through `scan_bodies_by_sha`. Inline and runtime
  scripts have a `scan_bodies` row and no request row.
- "Which scans loaded this exact file" and "did this script change between two scans" are index
  lookups on `scan_bodies_by_sha` and never open a pack. The diff itself reads two bodies.
- A one-time `PRAGMA auto_vacuum = INCREMENTAL` followed by `VACUUM` runs before migration 5 while the
  database is still small, so later deletes can return pages with `PRAGMA incremental_vacuum(n)`.
- Settings gain `keepNetworkData` (true), `storeFullHeaders` (false), `archiveLimitGb` (5),
  `archiveKeepDays` (null), written into the stored document by the migration as earlier ones do.
- Scans stored before this have no `scan_archives` row. That is the whole migration story for them.

### Rust structure

| Layer | Item | Content |
|---|---|---|
| `adscan-model` (`capture.rs`) | `NetworkRecord`, `CapturedBody` | What the browser crate reports per finished or failed request: every field of the request document with full headers, plus the body bytes for kept classes. In memory only. |
| `adscan-model` (`lib.rs`) | `RequestRow`, `RequestDetail`, `RequestQuery`, `RequestPage`, `RequestFacets`, `BodyText`, `ArchiveInfo`, `ArchiveUsage`, `ScriptSource` (from the viewer plan) | The frontend contract below. |
| Engine `domain/archive/` | `Location`, `BodyKind`, `kept_kind(resource_type, mime)`, `redact_headers`, `RequestDocument` (serde, the JSON line), `Manifest`, caps as constants | Pure. Unit-tested without I/O. |
| Engine `application/ports.rs` | `PackStore`, `PackWriter` (section C); `ArchiveRepository` (rows: begin, insert batch, seal, list, detail lookup, body lookup, usage, eviction candidates, mark evicted, delete); `NetworkSink` | Ports only. |
| Engine `application/archive/` | `ArchiveWriter` (the write loop), use cases `list_requests`, `get_request`, `get_request_body`, `get_script_source`, `search_requests`, `archive_usage`, `clear_archive`, `enforce_limit`, `sweep_on_start`, `export_scan_archive`, `import_scan_archive` | zstd encode and decode live here as plain functions; the store only sees opaque frames. |
| Engine `infrastructure/` | `pack_store.rs` (`LocalPackStore`), `sqlite/archive.rs` (`ArchiveRepository` on `SqliteStore`) | New dependency: `zstd` 0.14.3 (libzstd 1.5.7, C, small). `sha2` is already there. |
| Browser crate | Network recorder per session, `NetworkSink` calls | `CaptureSession` gains `network: Option<Arc<dyn NetworkSink>>`; `None` when retention is off. |

`NetworkSink::record(&self, record: NetworkRecord)` must not block and must not fail the capture: it
is a `try_send` into a bounded channel.

### Write path

1. Scan starts with retention on: insert `scan_archives` (`writing`), `PackStore::create`.
2. The recorder builds a `NetworkRecord` per request across `requestWillBeSent`, the two `ExtraInfo`
   events, `responseReceived` and `loadingFinished`/`loadingFailed`, asks for the body of kept classes
   at `loadingFinished`, and hands the finished record to the sink. Scripts keep flowing into
   `Capture.scripts` as today, because the pipeline needs them in memory.
3. `ArchiveWriter` (one task, CPU work on a blocking thread): truncate to the cap, sha-256, skip if the
   scan already has that hash, compress, `append`; redact headers; push the request document into the
   current record buffer.
4. Every 128 documents or one second: append the record frame, `sync` the pack, then one SQLite
   transaction with the new `hosts`, `scan_bodies` and `requests` rows. **Bytes are durable before the
   rows that point at them.**
5. Capture ends: flush, append manifest and trailer, `sync`, `seal`, set `sealed` with the totals,
   then run `enforce_limit`.

Back-pressure: the channel is bounded by bytes (64 MiB). When it is full, or the per-scan cap is
reached, the record is stored without its body and flagged. CDP reading is never stalled by disk.

Crash consistency, checked at every start (`sweep_on_start`):

- An archive still `writing`: its scan was already marked failed by `fail_interrupted`. Delete rows
  and pack. A half-captured network log of a scan that has no report is not worth a recovery path.
- A pack file with no `scan_archives` row (crash between row delete and unlink): delete the file.
- A `sealed` row whose file is missing (the user removed it): mark `evicted`.

A cancelled scan deletes its archive. A scan that fails after capture keeps it.

### Read path

- **Script viewer.** `get_script_source`: script record → `sha256` → `scan_bodies` → `Location` →
  `PackStore::read` → check length → decompress → verify sha-256 → text. Measured 0.3 ms warm and
  under 2 ms cold for 380 KB before formatting. "Original" returns that text. "Formatted" runs the
  existing `OxcFormatter` on demand and caches the result in memory, keyed by sha-256, least recently
  used, about 64 MB. Formatted text is never written to disk: it is derivable, and a formatter
  upgrade would otherwise leave stale copies. Evidence anchors keep using `locate`, as the viewer
  plan says. This replaces that plan's `ScriptSourceStore` and its separate SQLite file.
- **Request list.** SQL only, never the pack.
- **Request detail.** One record frame (at most 128 documents, 0.06 ms to decode) gives URL, headers,
  timing, initiator and POST body. The response body is a second range read, only when its tab opens.
- **Search inside one scan's bodies.** Brute force over the pack: decompression runs at about 1 GB/s,
  so 18 MB of bodies is tens of milliseconds plus the regex. No index.

### Retention, eviction, deletion

- `enforce_limit` after every seal: while the sum of `pack_bytes` exceeds the limit, take the oldest
  `sealed` archive that is not the scan just written, delete its pack and its `scan_bodies` rows, set
  `evicted`. With `archiveKeepDays` set, the same for archives older than that.
- **Evicted keeps the request rows.** They are host, path, status, sizes and timing: about 0.4 MB per
  3,000-request scan, a tenth of the pack, and they keep cross-scan host questions working over the
  whole history. The Network view of an evicted scan lists requests and says that headers and bodies
  were removed to stay under the limit.
- `delete_scan`: delete the `scans` row (cascades to archive, bodies, requests) in one transaction,
  then unlink the pack. Database first, so a crash leaves an orphan file for the sweep and never a
  row without bytes. `delete_all_scans` does the same and removes `archive/scans/`.
- "Clear network data": delete all `scan_archives` rows (cascade), remove the directory, then
  `VACUUM`. Reports stay.
- Space: pack space returns at unlink. SQLite reuses freed pages for the next scan; after a large
  delete run `PRAGMA incremental_vacuum`.
- Turning retention off stops writing for new scans and asks "Also delete stored network data?".

### Export and import

**Recommended: the bundle is a pack.** `export_scan_archive` copies the scan's pack byte for byte
(no recompression), then appends frames for the report (`ScanDetail` as JSON) and the audit trail
(log entries as JSON lines), a new manifest that lists them, and a new trailer. Result: one file,
`<site>-<date>.adscan.zst`, the same reader, readable on the platform by location. Import reads the
trailer and manifest, inserts the scan under a new id when the id exists, replays the record frames
into rows and copies the file into `archive/scans/`.

Rejected: a SQLite file as the bundle (a second schema to version, and bodies would have to be copied
into BLOBs), and a tar of JSON plus pack (a second container format for no gain; the pack is already
one file). The existing Markdown and JSON exports stay as they are and never contain bodies. The
export dialog must say that the bundle contains raw page data.

### Integrity

- Range reads return exactly the requested length or fail, as on the platform.
- zstd frame checksums catch damaged frames; body reads also verify sha-256.
- The manifest lets `verify_archive` walk a pack; run it in export and import, not on a schedule.

### Encryption at rest

**Rely on FileVault. No app-level encryption.**

- A key in the Keychain protects against someone reading the disk offline, which is exactly what
  FileVault already covers. It does not protect against other processes of the same user, who can ask
  the Keychain as the app does.
- The findings database next to the packs is not encrypted either, and it holds excerpts of the same
  material.
- It would add a way to lose every archive (a missing Keychain item), and encrypted packs would no
  longer be plain zstd files or match the platform's format.
- Protection against the realistic risk, a pack copied off the machine, belongs in the product
  wording and the export dialog.

### Frontend contract

All arguments camelCase, all results `Result<T, CommandError>`, as today.

| Command | Arguments | Returns |
|---|---|---|
| `list_requests` | `{ scanId, query: RequestQuery }` | `RequestPage` |
| `get_request_facets` | `{ scanId }` | `RequestFacets` |
| `get_request` | `{ scanId, seq }` | `RequestDetail` |
| `get_request_body` | `{ scanId, seq, part, pretty? }` | `BodyText` |
| `get_script_source` | `{ scanId, scriptId, view }` | `ScriptSource` (viewer plan; `availability` gains `evicted`) |
| `search_requests` | `{ query: CrossScanQuery }` | `CrossScanHit[]` |
| `get_body_occurrences` | `{ sha256, limit? }` | `BodyOccurrence[]` |
| `search_scan_content` (v1.1) | `{ scanId, query, regex?, caseSensitive?, scope[], limit? }` | `ContentHit[]` |
| `get_archive_usage` | – | `ArchiveUsage` |
| `clear_archive` | `{ olderThanDays? }` | `ArchiveUsage` |
| `export_scan_archive` | `{ scanId, path }` | `string` (the path) |
| `import_scan_archive` | `{ path }` | `ScanSummary` |

```ts
type RequestQuery = {
  page?: number;                 // page view index; omit for the whole visit
  search?: string;               // case-insensitive substring of host + path
  types?: ResourceGroup[];       // "document" | "script" | "stylesheet" | "xhr" | "image" | "media" | "font" | "websocket" | "other"
  status?: StatusGroup[];        // "2xx" | "3xx" | "4xx" | "5xx" | "failed" | "pending"
  hosts?: string[];              // exact host names
  site?: string;                 // registrable domain, matches all its hosts
  party?: "first" | "third";
  context?: RequestContext[];    // "page" | "iframe" | "worker" | "serviceWorker"
  bidder?: string;
  method?: string[];
  hasBody?: boolean;
  sort?: { by: "seq" | "started" | "duration" | "ttfb" | "size" | "status" | "host" | "type"; dir: "asc" | "desc" };
  offset?: number;               // default 0
  limit?: number;                // default 200, at most 1000
};

type RequestRow = {
  seq: number; page: number; context: RequestContext;
  startedMs: number; ttfbMs: number | null; durationMs: number | null;
  method: string; scheme: string; host: string; path: string; queryLen: number;
  resourceType: string; mime: string | null; status: number | null; error: string | null;
  encodedBytes: number | null; bodyBytes: number | null; postBytes: number | null;
  thirdParty: boolean; cache: "memory" | "disk" | null; serviceWorker: boolean;
  redirectFrom: number | null; initiatorType: string; initiatorSeq: number | null;
  bidder: string | null; hasBody: boolean; hasPost: boolean;
};

type RequestPage = {
  rows: RequestRow[];
  total: number;                 // rows matching the query
  transferredBytes: number;      // over all matching rows, for the status bar
  bodyBytes: number;
  archive: ArchiveInfo;
};

type RequestFacets = {           // counts over the whole scan, for filter chips and the page-view switcher
  pages: { page: number; url: string; requests: number; transferredBytes: number }[];
  types: { type: ResourceGroup; requests: number }[];
  status: { group: StatusGroup; requests: number }[];
  sites: { site: string; hosts: number; requests: number; thirdParty: boolean }[];
  bidders: { bidder: string; requests: number }[];
};

type RequestDetail = {
  row: RequestRow;
  available: boolean;            // false when the archive was evicted: only `row` is filled
  url: string;
  requestHeaders: Header[];      // { name, value, redacted }
  responseHeaders: Header[];
  timing: { dns: number; connect: number; ssl: number; send: number; wait: number; receive: number } | null;
  protocol: string | null; remoteAddress: string | null;
  initiator: { type: string; url: string | null; line: number | null; column: number | null; seq: number | null; scriptId: string | null };
  redirectChain: number[];       // seq of every hop, in order, this one included
  post: { bytes: number; truncated: boolean; mime: string | null } | null;
  body: { sha256: string; bytes: number; kind: BodyKind; truncated: boolean; scriptId: string | null } | null;
  notKept: "type" | "scanLimit" | "empty" | "failed" | null;
};

type BodyText = { text: string; bytes: number; truncated: boolean; pretty: boolean; language: "javascript" | "json" | "html" | "css" | "text" };
type ArchiveInfo = { state: "none" | "writing" | "sealed" | "evicted"; requests: number; bodies: number; packBytes: number; headersMode: "redacted" | "full" };
type ArchiveUsage = { packBytes: number; indexBytes: number; scans: number; evictedScans: number; limitBytes: number; oldestAt: number | null };
type CrossScanQuery = { host?: string; site?: string; pathContains?: string; bidder?: string; scannedSite?: string; sinceMs?: number; offset?: number; limit?: number };
type CrossScanHit = { scanId: string; scanStartedAt: number; scannedHost: string; seq: number; method: string; host: string; path: string; status: number | null };
```

What the Network view design can rely on:

- **Thousands of rows.** Any filter and sort over one scan answers in under 5 ms at the SQLite level
  (measured: all 3,000 rows in 3 ms, sort and top 200 in 0.5 ms). Page with `offset`/`limit` of 200
  into the virtualised table; a filter keystroke can re-query without debounce tricks. The whole scan
  as one reply (10,000 rows, about 2 MB of JSON) is possible but not needed.
- **Totals** for the status bar come with every page; **facets** are one more call per scan.
- **Per-page-view grouping** is the `page` filter plus `facets.pages`; times are relative to the page
  view's start, as everywhere else in the contract.
- **Search in URLs** covers host and path in SQL. Query strings, headers, POST and response bodies are
  `search_scan_content`, a scan of the pack (v1.1).
- **Detail** is one call for headers, timing, initiator and POST; `get_request_body` is called when
  the Response or Payload tab opens. `pretty` formats JSON in Rust; JavaScript bodies carry a
  `scriptId` and open in the script viewer.
- **Redacted headers** arrive as `{ name, value: "_ga=[27 bytes]", redacted: true }`.
- **Bid inspector.** `ObservedBidResponse.requestSeq` (additive) links a bidder's answer to its
  request; `list_requests { bidder, page }` lists a bidder's calls next to the auction;
  `get_request` gives the POST payload and `get_request_body` the response.
- **No live view in v1.** Rows exist in SQLite within a second of each request, so a live Network
  view is a later `scan-event` addition, not a storage change.
- `ScanSummary` and `ScanDetail` gain `archive: ArchiveInfo | null`.

### Disk use and defaults

Per page view, measured (zstd -3): bodies 2.6 to 5.3 MB, request records 0.25 to 0.32 MB, SQLite rows
0.05 to 0.23 MB. Total **3 to 6 MB per page view**.

| Scan | Estimate | Basis |
|---|---|---|
| Typical: one page view | 3 to 6 MB | Measured |
| Browse visit, three page views | 6 to 10 MB | Assumed: scripts repeat within the scan, the rest does not. Not measured. |
| Heavy: six page views or a long manual visit | 15 to 30 MB | Assumed |

Defaults: 5 MiB per body, 1 MiB per POST body, 256 MiB raw per scan, 5 GB total (600 to 1,500 scans),
no age limit.

## F. Wording, plan, tests, open decisions

### Wording

**Settings, Data section**

- Switch "Keep network data with reports" (on): "Stores what each scan loaded on this Mac, next to
  the report: every request with its headers, the scripts, pages and JSON it received, and the bid
  requests and responses. It is what the script viewer, the Network view and the bid inspector show.
  Images, video and fonts are not stored. Deleted with the report."
- Switch "Store full headers" (off): "Cookie and authorization values are replaced by their length.
  Turn this on to keep them as sent."
- Below: "Network data: 1.2 GB in 310 scans. Limit [5 GB]. When the limit is reached the oldest scans
  lose their network data; their reports stay." and a "Clear network data" button.
- "What is stored": "Reports keep findings, auctions, slots, metrics, bidder check results, script
  metadata and the audit trail. With Keep network data on, each scan also keeps the requests it made
  and the text content it received, compressed, in a file of its own. If you scan a page while signed
  in, or browse it yourself, that content can include your account's details. ads.txt and
  sellers.json files are read in memory and not kept."

**About**: the outbound host list is unchanged (nothing new is contacted). Add: "Scan data never
leaves this Mac unless you export it."

**HANDOVER hard rules**: replace "No raw payloads stored" with: "Raw scan data is stored locally and
only locally: one compressed archive per scan with requests, headers (cookie and authorization values
redacted unless the user turns that off), scripts, documents, JSON and bid payloads; never images,
video or fonts. It is deleted with the scan, bounded by a size limit, and can be switched off." Also
change "Bodies are read in memory and dropped", "Kept in memory only" where it refers to scripts,
"The string itself is never captured", and remove "any switch that writes script bodies to disk" from
"Not built, on purpose". Update `SettingsScreen.test.tsx`.

### Implementation order

1. **Contract.** Model types above, settings fields, `ScriptRecord.sha256`,
   `ObservedBidResponse.request_seq`, `archive` on the scan types, `NetworkRecord`; command stubs in
   both API adapters; bindings; fake-backend data for one scan with about 1,500 requests.
2. **Browser crate.** Network on child sessions; the per-session request state machine with both
   `ExtraInfo` events, redirects, cache and service worker flags, failures; POST data;
   kept-class body reads at `loadingFinished`; `NetworkSink`. The script and bid-response paths keep
   working from the same events.
3. **Engine storage.** `domain/archive` (kinds, redaction, document, manifest); `zstd`;
   `LocalPackStore`; migration 5 and `sqlite/archive.rs`; `ArchiveWriter`; start-up sweep; limit
   enforcement; delete wiring.
4. **Engine reads and commands.** List, facets, detail, body, script source with the formatter cache,
   cross-scan search, usage, clear; then export and import.
5. **Frontend.** Script viewer on real sources, Network view, bid inspector links, Settings.
6. Later: `search_scan_content`, Parquet export, live view.

Steps 2 and 3 can run in parallel after step 1.

### Tests

- **Domain (unit).** Redaction (cookie names kept, lengths right, `Set-Cookie` with several cookies,
  case-insensitive header names, full mode untouched); kind classification from type and MIME; the
  request document round trip, including that the platform's seven fields are present; location
  compose and parse against the platform's cases (`#o=0&l=1`, zero length rejected, no fragment).
- **Pack store.** Append returns contiguous locations; read returns exact bytes and fails on a range
  past the end; a sealed pack stream-decodes with a stock zstd decoder; trailer and manifest are
  found from the file alone; a truncated file is reported, not misread.
- **Writer.** A body repeated in a scan is stored once; caps truncate and flag; a full channel drops
  bodies and keeps rows; rows never point past the synced length (kill the writer between append and
  commit, reopen, sweep).
- **SQLite.** Migration from schema 4 with existing scans; cascade on scan delete; each
  `RequestQuery` filter and sort; facets; cross-scan host and body-hash queries; eviction keeps rows
  and removes bodies; a query-plan test that the list uses the primary key and the host search uses
  `requests_by_host`.
- **Sweep.** Orphan pack removed; `writing` archive removed; missing file becomes `evicted`.
- **Export and import.** Round trip gives equal rows and bodies; importing twice creates a second
  scan; a bundle with a damaged frame is refused.
- **Browser crate (integration, demo site).** A request inside a cross-origin iframe and one from a
  worker are recorded; a redirect chain is linked; the demo exchange's bid request has its POST body
  and response body; an image has a row and no body; a body is still read when the iframe is removed
  right after loading.
- **Settings.** Retention off writes no pack and no rows; "Store full headers" changes the mode of
  new archives only.
- **e2e-tauri.** Demo scan, open the wrapper in the viewer after an app restart, open the `openx` bid
  request and see request and response, delete the scan and see the file gone and usage drop.
- **Performance budgets.** List query under 20 ms at 5 million rows; script open under 50 ms before
  formatting; capture of the demo site not measurably longer with retention on.

### Decisions for the product owner

| # | Decision | Recommendation |
|---|---|---|
| 1 | Per-scan packs without cross-scan dedupe (about 4 MB per repeat scan instead of about 1 MB)? | Yes. Simplest, matches the platform, delete means delete. Revisit if users hit the limit with repeat scans. |
| 2 | Retention on by default, including for people who installed under the old "nothing raw is stored" text? | Yes, with a one-time notice after the update that names the switch. |
| 3 | Default limit | 5 GB, no age limit. |
| 4 | Redact cookie and authorization values by default? | Yes; "Store full headers" off. |
| 5 | Keep query strings, POST bodies, the TC string and document HTML unredacted? | Yes. Say it in Settings. |
| 6 | When the limit evicts a scan's archive, keep its request list (host, path, status, timing)? | Yes. |
| 7 | CSS bodies | Keep (under 0.1 MB per scan). |
| 8 | WebSocket frames | Not captured in v1. |
| 9 | Record frames hold 128 requests each and bodies are separate raw frames, unlike the platform's one JSON document per frame. Acceptable, given the platform reader needs a line split and a `readBytes`? | Yes. One frame per request costs 4.7x more for headers. |
| 10 | Export bundle is a pack with report frames appended, containing raw page data | Yes, with a warning in the dialog. |
| 11 | No encryption beyond FileVault | Yes. |
| 12 | Drop the viewer plan's in-memory source store and its "Keep script sources" switch in favour of this archive? With retention off the viewer then says "not kept". | Yes. One path instead of two. |
| 13 | Manual visits and persistent-profile scans are the ones that can capture signed-in pages. Same default for them? | Same default, and name it in the manual-visit start dialog. |
| 14 | DuckDB | Not now. Add "Export requests as Parquet" when someone asks for analysis outside the app. |

### Not verified

- Anything on macOS (APFS timings, universal build with `zstd`).
- Capture cost inside the app's own recorder; my numbers come from a Node client on the same CDP calls.
- Whether enabling Network on child sessions changes page timing.
- Pages behind consent walls, signed-in pages, single-page-app route changes, pages with WebSockets.
- Multi-page scan sizes (estimated, not measured).
- The platform accepting local packs as they are (section C lists the two reader additions).
- DuckDB's Rust crate behaviour for the SQLite scanner (inferred from its feature list and from the
  Python build), its compile time and its binary size in this app.
- `object_store` compile time.
