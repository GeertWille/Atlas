//! The sellers.json of an exchange is kept between scans: when the exchange
//! is asked, what it is asked, and what is used when it does not answer.
//! The web is a test double and the cache is the real one on a temporary
//! directory; no test here contacts a real host.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adscan_engine::application::ports::{
    Clock, FetchError, FetchProxy, FileVersion, PublicFile, PublicFileFetcher, PublicFileRequest,
};
use adscan_engine::application::sellers_json::{
    Cancelled, Origin, SellersJsonFile, SellersJsonSource,
};
use adscan_engine::domain::diagnostics::CopyStoodIn;
use adscan_engine::domain::diagnostics::identity::BrowserIdentity;
use adscan_engine::infrastructure::sellers_cache::LocalSellersJsonCache;
use adscan_model::CaptureProxy;
use async_trait::async_trait;
use common::{FakePublicFiles, text_file};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// The browser of the scan that asks.
fn scan_identity() -> BrowserIdentity {
    BrowserIdentity::of_scan(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36",
        Some("nl-BE"),
    )
}

const EXCHANGE: &str = "appnexus.com";
const URL: &str = "https://appnexus.com/sellers.json";
const SELLERS: &str = r#"{"sellers":[{"seller_id":"958","seller_type":"PUBLISHER"}]}"#;
const HOUR: f64 = 3_600_000.0;
const START: f64 = 1_794_500_000_000.0;

struct ManualClock(Mutex<f64>);

impl ManualClock {
    fn advance(&self, ms: f64) {
        *self.0.lock().unwrap() += ms;
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> f64 {
        *self.0.lock().unwrap()
    }
}

struct Fixture {
    source: SellersJsonSource,
    files: Arc<FakePublicFiles>,
    clock: Arc<ManualClock>,
    directory: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let files = Arc::new(FakePublicFiles::default());
    let clock = Arc::new(ManualClock(Mutex::new(START)));
    Fixture {
        source: SellersJsonSource::new(
            files.clone(),
            Arc::new(LocalSellersJsonCache::new(directory.path())),
            clock.clone(),
        ),
        files,
        clock,
        directory,
    }
}

fn sellers_json(body: &str, etag: Option<&str>) -> Result<PublicFile, FetchError> {
    Ok(PublicFile {
        status: 200,
        content_type: Some("application/json".into()),
        body: body.as_bytes().to_vec(),
        final_url: URL.into(),
        version: FileVersion {
            etag: etag.map(str::to_string),
            last_modified: Some("Wed, 11 Nov 2026 09:00:00 GMT".into()),
        },
    })
}

impl Fixture {
    async fn read(&self) -> SellersJsonFile {
        self.source
            .read(EXCHANGE, &scan_identity(), &CancellationToken::new())
            .await
            .unwrap()
    }

    fn requests(&self) -> Vec<PublicFileRequest> {
        std::mem::take(&mut *self.files.requests.lock().unwrap())
    }
}

fn file(content: &str, origin: Origin) -> SellersJsonFile {
    SellersJsonFile {
        content: Ok(content.as_bytes().to_vec()),
        origin,
    }
}

fn unavailable(reason: &str) -> SellersJsonFile {
    SellersJsonFile {
        content: Err(reason.into()),
        origin: Origin::Downloaded,
    }
}

#[tokio::test]
async fn a_file_is_downloaded_once_and_then_read_from_the_kept_copy() {
    let fixture = fixture();
    fixture
        .files
        .answer(URL, sellers_json(SELLERS, Some("\"v1\"")));

    assert_eq!(fixture.read().await, file(SELLERS, Origin::Downloaded));
    // Asked directly (the system's proxy at most), never through a scan's
    // proxy, without a version, and with the limits of a sellers.json.
    assert_eq!(
        fixture.requests(),
        [PublicFileRequest {
            url: URL.into(),
            max_bytes: 24 * 1024 * 1024,
            timeout: Duration::from_secs(20),
            loopback_origin: None,
            proxy: FetchProxy {
                route: CaptureProxy::System,
                password: None
            },
            known_version: None,
            // As the scan's browser would ask, for a JSON file.
            identity: scan_identity(),
            accept: "application/json,*/*;q=0.8",
        }]
    );

    fixture.clock.advance(23.0 * HOUR);
    assert_eq!(
        fixture.read().await,
        file(SELLERS, Origin::Kept { fetched_at: START })
    );
    assert_eq!(fixture.requests(), []);
    // Another source on the same directory, as after a restart of the app.
    let restarted = SellersJsonSource::new(
        fixture.files.clone(),
        Arc::new(LocalSellersJsonCache::new(fixture.directory.path())),
        fixture.clock.clone(),
    );
    assert_eq!(
        restarted
            .read(EXCHANGE, &scan_identity(), &CancellationToken::new())
            .await
            .unwrap(),
        file(SELLERS, Origin::Kept { fetched_at: START })
    );
    assert_eq!(fixture.requests(), []);
    assert_eq!(fixture.source.cache_info().await.entries, 1);
}

#[tokio::test]
async fn after_a_day_the_exchange_is_asked_whether_the_file_changed() {
    let fixture = fixture();
    fixture
        .files
        .answer(URL, sellers_json(SELLERS, Some("\"v1\"")));
    fixture.read().await;
    fixture.requests();

    // Unchanged: the exchange says so without sending the file again.
    fixture.clock.advance(24.0 * HOUR);
    fixture.files.answer(URL, text_file(304, ""));
    assert_eq!(fixture.read().await, file(SELLERS, Origin::Confirmed));
    let asked = fixture.requests();
    assert_eq!(asked.len(), 1);
    assert_eq!(
        asked[0].known_version,
        Some(FileVersion {
            etag: Some("\"v1\"".into()),
            last_modified: Some("Wed, 11 Nov 2026 09:00:00 GMT".into()),
        })
    );
    // That counts as fetched now: nothing is asked for another day.
    fixture.clock.advance(23.0 * HOUR);
    assert_eq!(
        fixture.read().await,
        file(
            SELLERS,
            Origin::Kept {
                fetched_at: START + 24.0 * HOUR
            }
        )
    );
    assert_eq!(fixture.requests(), []);

    // Changed: the new file replaces the kept one, with its own version.
    const CHANGED: &str = r#"{"sellers":[]}"#;
    fixture.clock.advance(HOUR);
    fixture
        .files
        .answer(URL, sellers_json(CHANGED, Some("\"v2\"")));
    assert_eq!(fixture.read().await, file(CHANGED, Origin::Downloaded));
    fixture.requests();
    fixture.clock.advance(24.0 * HOUR);
    fixture.files.answer(URL, text_file(304, ""));
    assert_eq!(fixture.read().await, file(CHANGED, Origin::Confirmed));
    assert_eq!(
        fixture.requests()[0]
            .known_version
            .as_ref()
            .unwrap()
            .etag
            .as_deref(),
        Some("\"v2\"")
    );
}

#[tokio::test]
async fn an_exchange_that_cannot_be_reached_is_answered_from_a_copy_of_at_most_a_week() {
    for trouble in [
        Err(FetchError::Network("could not connect".into())),
        Err(FetchError::TimedOut),
        text_file(503, ""),
        text_file(429, ""),
    ] {
        let fixture = fixture();
        fixture
            .files
            .answer(URL, sellers_json(SELLERS, Some("\"v1\"")));
        fixture.read().await;
        fixture.files.answer(URL, trouble.clone());

        fixture.clock.advance(7.0 * 24.0 * HOUR);
        assert_eq!(
            fixture.read().await,
            file(
                SELLERS,
                Origin::KeptInstead {
                    fetched_at: START,
                    because: CopyStoodIn::ExchangeUnreachable,
                }
            ),
            "{trouble:?}"
        );
        // Used, not renewed: the next scan asks again.
        fixture.requests();
        fixture.read().await;
        assert_eq!(fixture.requests().len(), 1, "{trouble:?}");

        // Past a week the copy is not used any more, whatever happens.
        fixture.clock.advance(HOUR);
        let late = fixture.read().await;
        assert_eq!(late.origin, Origin::Downloaded);
        assert!(late.content.is_err(), "{trouble:?}: {late:?}");
    }
}

#[tokio::test]
async fn an_exchange_that_says_the_file_is_gone_is_believed() {
    let fixture = fixture();
    fixture
        .files
        .answer(URL, sellers_json(SELLERS, Some("\"v1\"")));
    fixture.read().await;
    fixture.clock.advance(25.0 * HOUR);
    fixture.files.answer(URL, text_file(404, ""));
    assert_eq!(
        fixture.read().await,
        unavailable("the exchange answered HTTP 404")
    );
}

#[tokio::test]
async fn a_failed_lookup_is_remembered_for_an_hour() {
    let fixture = fixture();
    fixture.files.answer(URL, text_file(404, ""));
    assert_eq!(
        fixture.read().await,
        unavailable("the exchange answered HTTP 404")
    );
    assert_eq!(fixture.requests().len(), 1);

    fixture.clock.advance(0.9 * HOUR);
    let remembered = fixture.read().await;
    assert_eq!(
        remembered,
        SellersJsonFile {
            content: Err("the exchange answered HTTP 404".into()),
            origin: Origin::Kept { fetched_at: START },
        }
    );
    assert_eq!(fixture.requests(), []);

    // Then the exchange is asked afresh, with no version to compare with.
    fixture.clock.advance(0.2 * HOUR);
    fixture.files.answer(URL, sellers_json(SELLERS, None));
    assert_eq!(fixture.read().await, file(SELLERS, Origin::Downloaded));
    let asked = fixture.requests();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].known_version, None);

    // An address that cannot be connected to is a failed lookup like any other.
    let unreachable = fixture_without_answer().await;
    assert_eq!(unreachable.0, unavailable("could not connect"));
    assert_eq!(unreachable.1, 1, "and it is remembered too");
}

/// A refusal can be of the asking scan's browser alone (a firewall that
/// turns one user agent away): it does not replace a copy that still
/// serves, and is never handed to another scan as that scan's answer.
#[tokio::test]
async fn a_refusal_leaves_the_kept_copy_in_place_and_is_never_another_scans_answer() {
    for status in [401, 403] {
        let fixture = fixture();
        fixture
            .files
            .answer(URL, sellers_json(SELLERS, Some("\"v1\"")));
        fixture.read().await;
        fixture.clock.advance(2.0 * 24.0 * HOUR);
        fixture.files.answer(URL, text_file(status, "Forbidden"));

        assert_eq!(
            fixture.read().await,
            file(
                SELLERS,
                Origin::KeptInstead {
                    fetched_at: START,
                    because: CopyStoodIn::AnswerNotSellersJson,
                }
            ),
            "{status}"
        );
        // Still the copy of two days ago, and still asked about.
        fixture.requests();
        fixture
            .files
            .answer(URL, sellers_json(SELLERS, Some("\"v1\"")));
        assert_eq!(fixture.read().await, file(SELLERS, Origin::Downloaded));
        assert_eq!(
            fixture.requests()[0].known_version.as_ref().unwrap().etag,
            Some("\"v1\"".to_string()),
            "{status}"
        );

        // Without a copy the scan that was refused is told so, and the next
        // scan asks for itself.
        let fixture = self::fixture();
        fixture.files.answer(URL, text_file(status, "Forbidden"));
        let refused = format!("the site refused the request (HTTP {status})");
        assert_eq!(fixture.read().await, unavailable(&refused));
        assert_eq!(fixture.read().await, unavailable(&refused));
        assert_eq!(fixture.requests().len(), 2, "{status}: not remembered");
        assert_eq!(fixture.source.cache_info().await.entries, 0, "{status}");
    }
}

/// The outcome of a lookup nothing answers, and how many requests two
/// lookups in a row made.
async fn fixture_without_answer() -> (SellersJsonFile, usize) {
    let fixture = fixture();
    let first = fixture.read().await;
    fixture.read().await;
    (first, fixture.requests().len())
}

#[tokio::test]
async fn a_name_that_is_no_public_domain_is_never_asked_and_never_kept() {
    let fixture = fixture();
    for name in ["localhost", "10.0.0.1", "../../etc", "exchange", ""] {
        let read = fixture
            .source
            .read(name, &scan_identity(), &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            read,
            unavailable("it is not a public domain name"),
            "{name:?}"
        );
    }
    assert_eq!(fixture.requests(), []);
    assert_eq!(fixture.source.cache_info().await.entries, 0);
}

/// An exchange that answers when the test lets it.
struct GatedFiles {
    started: AtomicUsize,
    gate: Notify,
    /// The language each request was asked in, by address.
    asked_in: Mutex<Vec<(String, String)>>,
}

#[async_trait]
impl PublicFileFetcher for GatedFiles {
    async fn fetch(
        &self,
        request: &PublicFileRequest,
        cancel: &CancellationToken,
    ) -> Result<PublicFile, FetchError> {
        self.asked_in.lock().unwrap().push((
            request.url.clone(),
            request.identity.accept_language.clone(),
        ));
        self.started.fetch_add(1, Ordering::SeqCst);
        tokio::select! {
            () = self.gate.notified() => sellers_json(SELLERS, None),
            () = cancel.cancelled() => Err(FetchError::Cancelled),
        }
    }
}

fn gated() -> (Arc<SellersJsonSource>, Arc<GatedFiles>, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let files = Arc::new(GatedFiles {
        started: AtomicUsize::new(0),
        gate: Notify::new(),
        asked_in: Mutex::default(),
    });
    let source = Arc::new(SellersJsonSource::new(
        files.clone(),
        Arc::new(LocalSellersJsonCache::new(directory.path())),
        Arc::new(ManualClock(Mutex::new(START))),
    ));
    (source, files, directory)
}

async fn until_started(files: &GatedFiles, count: usize) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while files.started.load(Ordering::SeqCst) < count {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("the download never started");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scans_that_need_the_same_exchange_at_once_share_one_download() {
    let (source, files, _directory) = gated();
    // Six scans, each with its own browser: the one download is asked for
    // as the scan whose turn it is would ask, whichever that is.
    let languages = ["nl-BE", "de-DE", "fr-FR", "da-DK", "it-IT", "es-ES"];
    let readers: Vec<_> = languages
        .into_iter()
        .map(|language| {
            let source = Arc::clone(&source);
            let identity = BrowserIdentity::of_scan(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36",
                Some(language),
            );
            tokio::spawn(async move {
                let read = source
                    .read(EXCHANGE, &identity, &CancellationToken::new())
                    .await;
                (identity, read)
            })
        })
        .collect();
    // A different exchange is not held up by them.
    let other = {
        let source = Arc::clone(&source);
        tokio::spawn(async move {
            source
                .read(
                    "rubiconproject.com",
                    &scan_identity(),
                    &CancellationToken::new(),
                )
                .await
        })
    };
    until_started(&files, 2).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        files.started.load(Ordering::SeqCst),
        2,
        "one download per exchange"
    );

    files.gate.notify_waiters();
    let mut origins = Vec::new();
    let mut downloaded_for = Vec::new();
    for reader in readers {
        let (identity, read) = reader.await.unwrap();
        let read = read.unwrap();
        assert_eq!(read.content.as_deref(), Ok(SELLERS.as_bytes()));
        if read.origin == Origin::Downloaded {
            downloaded_for.push(identity.accept_language);
        }
        origins.push(read.origin);
    }
    other.await.unwrap().unwrap();
    assert_eq!(files.started.load(Ordering::SeqCst), 2);
    assert_eq!(
        origins
            .iter()
            .filter(|origin| **origin == Origin::Downloaded)
            .count(),
        1
    );
    assert_eq!(
        origins
            .iter()
            .filter(|origin| **origin == Origin::Kept { fetched_at: START })
            .count(),
        5
    );
    let asked_in: Vec<String> = files
        .asked_in
        .lock()
        .unwrap()
        .iter()
        .filter(|(url, _)| url == URL)
        .map(|(_, language)| language.clone())
        .collect();
    assert_eq!(downloaded_for.len(), 1);
    assert_eq!(
        asked_in, downloaded_for,
        "the download carries its own scan's language"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cancelled_scan_keeps_nothing_and_holds_nobody_up() {
    let (source, files, directory) = gated();
    let cancelled = CancellationToken::new();
    let first = {
        let (source, cancel) = (Arc::clone(&source), cancelled.clone());
        tokio::spawn(async move { source.read(EXCHANGE, &scan_identity(), &cancel).await })
    };
    until_started(&files, 1).await;

    // A second scan waits its turn; cancelled while it waits, it leaves at once.
    let impatient = CancellationToken::new();
    let waiting = {
        let (source, cancel) = (Arc::clone(&source), impatient.clone());
        tokio::spawn(async move { source.read(EXCHANGE, &scan_identity(), &cancel).await })
    };
    let patient = {
        let source = Arc::clone(&source);
        tokio::spawn(async move {
            source
                .read(EXCHANGE, &scan_identity(), &CancellationToken::new())
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(30)).await;
    impatient.cancel();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), waiting)
            .await
            .unwrap()
            .unwrap(),
        Err(Cancelled)
    );
    assert_eq!(files.started.load(Ordering::SeqCst), 1);

    // The scan that was downloading is cancelled in the middle of it.
    cancelled.cancel();
    assert_eq!(first.await.unwrap(), Err(Cancelled));
    assert_eq!(source.cache_info().await.entries, 0, "nothing was kept");
    assert_eq!(
        std::fs::read_dir(directory.path().join("cache/sellers-json"))
            .map(Iterator::count)
            .unwrap_or(0),
        0
    );

    // The scan that kept waiting downloads the file itself.
    until_started(&files, 2).await;
    files.gate.notify_waiters();
    assert_eq!(
        patient.await.unwrap().unwrap(),
        file(SELLERS, Origin::Downloaded)
    );
    assert_eq!(source.cache_info().await.entries, 1);
}

#[tokio::test]
async fn a_damaged_copy_is_thrown_away_and_the_file_is_downloaded_again() {
    let fixture = fixture();
    fixture
        .files
        .answer(URL, sellers_json(SELLERS, Some("\"v1\"")));
    fixture.read().await;
    fixture.requests();
    let cache_directory = fixture.directory.path().join("cache/sellers-json");
    for entry in std::fs::read_dir(&cache_directory).unwrap() {
        let path = entry.unwrap().path();
        if path.to_str().unwrap().ends_with(".json.zst") {
            std::fs::write(path, b"not a zstd frame").unwrap();
        }
    }

    assert_eq!(fixture.read().await, file(SELLERS, Origin::Downloaded));
    let asked = fixture.requests();
    assert_eq!(asked.len(), 1);
    assert_eq!(
        asked[0].known_version, None,
        "nothing of the damaged copy is used"
    );
    assert_eq!(
        fixture.read().await,
        file(SELLERS, Origin::Kept { fetched_at: START })
    );
}

#[tokio::test]
async fn clearing_the_cache_makes_the_next_scan_download_again() {
    let fixture = fixture();
    fixture
        .files
        .answer(URL, sellers_json(SELLERS, Some("\"v1\"")));
    fixture.read().await;
    assert!(fixture.source.cache_info().await.stored_bytes > 0.0);

    let cleared = fixture.source.clear_cache().await.unwrap();
    assert_eq!((cleared.entries, cleared.stored_bytes), (0, 0.0));
    fixture.requests();
    assert_eq!(fixture.read().await, file(SELLERS, Origin::Downloaded));
    assert_eq!(fixture.requests().len(), 1);
}
