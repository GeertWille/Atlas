//! An exchange that answers 200 with something that is not a sellers.json (a
//! web page, a soft 404, half a file): what is kept of it, and what becomes
//! of a copy that was kept before. The exchange is a mock web server on
//! loopback, read by the real fetcher into the real cache on a temporary
//! directory; no test here contacts a real host.

use std::sync::{Arc, Mutex};

use adscan_engine::application::ports::{
    CachedSellersJson, Clock, FetchError, FileVersion, PublicFile, PublicFileFetcher,
    PublicFileRequest, SellersJsonCache,
};
use adscan_engine::application::sellers_json::{Origin, SellersJsonFile, SellersJsonSource};
use adscan_engine::infrastructure::public_files::HttpPublicFileFetcher;
use adscan_engine::infrastructure::sellers_cache::LocalSellersJsonCache;
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const EXCHANGE: &str = "appnexus.com";
const SELLERS: &str = r#"{"sellers":[{"seller_id":"958","seller_type":"PUBLISHER"}]}"#;
const HOUR: f64 = 3_600_000.0;
const START: f64 = 1_794_500_000_000.0;

/// What exchanges were seen to answer with status 200 instead of the file.
const UNREADABLE: [(&str, &str, &str); 3] = [
    (
        "a web page",
        "text/html",
        "<!DOCTYPE html><html><head><title>Just a moment...</title></head><body></body></html>",
    ),
    (
        "JSON without a sellers array",
        "application/json",
        r#"{"status":404,"error":"Not Found"}"#,
    ),
    (
        "a file that was cut short",
        "application/json",
        r#"{"sellers":[{"seller_id":"958","seller_type":"PUBLISHER"},{"seller_id":"9"#,
    ),
];

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

/// The exchange's address answered by the mock server. The real fetcher only
/// contacts loopback as the origin it was told to, so it is told to.
struct MockExchange {
    origin: String,
}

#[async_trait]
impl PublicFileFetcher for MockExchange {
    async fn fetch(
        &self,
        request: &PublicFileRequest,
        cancel: &CancellationToken,
    ) -> Result<PublicFile, FetchError> {
        let file = request
            .url
            .strip_prefix("https://appnexus.com")
            .expect("only the exchange is asked");
        let local = PublicFileRequest {
            url: format!("{}{file}", self.origin),
            loopback_origin: Some(self.origin.clone()),
            ..request.clone()
        };
        HttpPublicFileFetcher.fetch(&local, cancel).await
    }
}

struct Fixture {
    source: SellersJsonSource,
    cache: Arc<LocalSellersJsonCache>,
    clock: Arc<ManualClock>,
    server: MockServer,
    _directory: tempfile::TempDir,
}

async fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    let cache = Arc::new(LocalSellersJsonCache::new(directory.path()));
    let clock = Arc::new(ManualClock(Mutex::new(START)));
    Fixture {
        source: SellersJsonSource::new(
            Arc::new(MockExchange {
                origin: server.uri(),
            }),
            cache.clone(),
            clock.clone(),
        ),
        cache,
        clock,
        server,
        _directory: directory,
    }
}

fn answer(status: u16, content_type: &str, body: &str, etag: &str) -> ResponseTemplate {
    ResponseTemplate::new(status)
        .set_body_raw(body.as_bytes().to_vec(), content_type)
        .insert_header("etag", etag)
}

fn the_file() -> ResponseTemplate {
    answer(200, "application/json", SELLERS, "\"v1\"")
}

impl Fixture {
    /// From now on the exchange answers every request with `response`.
    async fn exchange_answers(&self, response: ResponseTemplate) {
        self.server.reset().await;
        Mock::given(method("GET"))
            .and(path("/sellers.json"))
            .respond_with(response)
            .mount(&self.server)
            .await;
    }

    async fn read(&self) -> SellersJsonFile {
        self.source
            .read(EXCHANGE, &CancellationToken::new())
            .await
            .unwrap()
    }

    /// The `If-None-Match` of every request since the exchange's answer was
    /// last set.
    async fn asked_about(&self) -> Vec<Option<String>> {
        self.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|request| {
                request
                    .headers
                    .get("if-none-match")
                    .map(|value| value.to_str().unwrap().to_string())
            })
            .collect()
    }

    async fn kept(&self) -> CachedSellersJson {
        self.cache.load(EXCHANGE).await.expect("an entry is kept")
    }

    /// A file as a version of the app that did not check it kept it.
    async fn keep_unchecked(&self, body: &str) {
        let entry = CachedSellersJson {
            fetched_at: self.clock.now_ms(),
            final_url: "https://appnexus.com/sellers.json".into(),
            version: FileVersion {
                etag: Some("\"old\"".into()),
                last_modified: None,
            },
            content: Ok(body.as_bytes().to_vec()),
        };
        self.cache.store(EXCHANGE, entry).await.unwrap();
    }
}

fn file(origin: Origin) -> SellersJsonFile {
    SellersJsonFile {
        content: Ok(SELLERS.as_bytes().to_vec()),
        origin,
    }
}

fn not_sellers_json(read: &SellersJsonFile) -> &str {
    let reason = read.content.as_ref().unwrap_err();
    assert!(
        reason.starts_with("the file is not valid sellers.json ("),
        "{reason}"
    );
    reason
}

#[tokio::test]
async fn an_unreadable_answer_is_a_failed_lookup_that_is_asked_again_after_an_hour() {
    for (what, content_type, body) in UNREADABLE {
        let fixture = fixture().await;
        fixture
            .exchange_answers(answer(200, content_type, body, "\"page\""))
            .await;

        let read = fixture.read().await;
        assert_eq!(read.origin, Origin::Downloaded, "{what}");
        let reason = not_sellers_json(&read).to_string();
        // Kept as the note of a failed lookup, never as a file.
        let kept = fixture.kept().await;
        assert_eq!(kept.content, Err(reason.clone()), "{what}");
        assert_eq!(kept.version, FileVersion::default(), "{what}");

        fixture.clock.advance(0.9 * HOUR);
        assert_eq!(
            fixture.read().await,
            SellersJsonFile {
                content: Err(reason),
                origin: Origin::Kept { fetched_at: START },
            },
            "{what}"
        );
        assert_eq!(fixture.asked_about().await, [None], "{what}");

        // After the hour the exchange is asked afresh: the answer it gave
        // has no version worth asking about.
        fixture.clock.advance(0.2 * HOUR);
        fixture.exchange_answers(the_file()).await;
        assert_eq!(fixture.read().await, file(Origin::Downloaded), "{what}");
        assert_eq!(fixture.asked_about().await, [None], "{what}");
        assert_eq!(
            fixture.kept().await.content,
            Ok(SELLERS.as_bytes().to_vec()),
            "{what}"
        );
    }
}

#[tokio::test]
async fn an_unreadable_answer_never_replaces_an_older_copy_that_still_serves() {
    for (what, content_type, body) in UNREADABLE {
        let fixture = fixture().await;
        fixture.exchange_answers(the_file()).await;
        assert_eq!(fixture.read().await, file(Origin::Downloaded), "{what}");
        let copy = fixture.kept().await;

        // A week later the copy is stale, and the exchange serves something else.
        fixture.clock.advance(7.0 * 24.0 * HOUR);
        fixture
            .exchange_answers(answer(200, content_type, body, "\"page\""))
            .await;
        assert_eq!(
            fixture.read().await,
            file(Origin::KeptWhileUnreachable { fetched_at: START }),
            "{what}"
        );
        assert_eq!(fixture.kept().await, copy, "{what}: the copy is untouched");
        // Used, not renewed: the next scan asks again, about the copy's own version.
        fixture.read().await;
        assert_eq!(
            fixture.asked_about().await,
            [Some("\"v1\"".to_string()), Some("\"v1\"".to_string())],
            "{what}"
        );
        assert_eq!(fixture.kept().await, copy, "{what}");

        // Past a week the copy is not used any more, and the answer is
        // reported for what it is.
        fixture.clock.advance(HOUR);
        let late = fixture.read().await;
        assert_eq!(late.origin, Origin::Downloaded, "{what}");
        not_sellers_json(&late);
        assert_eq!(fixture.kept().await.content, late.content, "{what}");
    }
}

#[tokio::test]
async fn a_copy_outlives_an_unreadable_answer_and_is_confirmed_when_the_exchange_recovers() {
    let (_, content_type, body) = UNREADABLE[0];
    let fixture = fixture().await;
    fixture.exchange_answers(the_file()).await;
    fixture.read().await;

    fixture.clock.advance(25.0 * HOUR);
    fixture
        .exchange_answers(answer(200, content_type, body, "\"page\""))
        .await;
    assert_eq!(
        fixture.read().await,
        file(Origin::KeptWhileUnreachable { fetched_at: START })
    );

    fixture.server.reset().await;
    Mock::given(method("GET"))
        .and(path("/sellers.json"))
        .and(header("if-none-match", "\"v1\""))
        .respond_with(ResponseTemplate::new(304).insert_header("etag", "\"v1\""))
        .mount(&fixture.server)
        .await;
    assert_eq!(fixture.read().await, file(Origin::Confirmed));
    assert_eq!(fixture.kept().await.fetched_at, START + 25.0 * HOUR);
}

#[tokio::test]
async fn not_modified_does_not_renew_the_note_of_a_failed_lookup() {
    let fixture = fixture().await;
    fixture.exchange_answers(ResponseTemplate::new(404)).await;
    assert_eq!(
        fixture.read().await.content,
        Err("the exchange answered HTTP 404".to_string())
    );

    // An exchange that says "not modified" to a request that named no version.
    fixture.clock.advance(2.0 * HOUR);
    fixture
        .exchange_answers(ResponseTemplate::new(304).insert_header("etag", "\"v1\""))
        .await;
    assert_eq!(
        fixture.read().await,
        SellersJsonFile {
            content: Err("the exchange answered HTTP 304".into()),
            origin: Origin::Downloaded,
        }
    );
    assert_eq!(fixture.asked_about().await, [None]);
    let kept = fixture.kept().await;
    assert_eq!(
        (kept.fetched_at, kept.content),
        (
            START + 2.0 * HOUR,
            Err("the exchange answered HTTP 304".to_string())
        )
    );
    // The cache itself refuses to move the date of an entry without a file.
    assert!(fixture.cache.confirm(EXCHANGE, START).await.is_err());
}

#[tokio::test]
async fn an_unreadable_file_kept_before_files_were_checked_is_discarded_and_downloaded_again() {
    for (what, _, body) in UNREADABLE {
        // Kept a moment ago: by its date alone it would be used without asking.
        let fixture = fixture().await;
        fixture.keep_unchecked(body).await;
        fixture.exchange_answers(the_file()).await;
        assert_eq!(fixture.read().await, file(Origin::Downloaded), "{what}");
        assert_eq!(
            fixture.asked_about().await,
            [None],
            "{what}: its version is not asked about"
        );
        let kept = fixture.kept().await;
        assert_eq!(kept.content, Ok(SELLERS.as_bytes().to_vec()), "{what}");
        assert_eq!(kept.version.etag.as_deref(), Some("\"v1\""), "{what}");
        assert_eq!(
            fixture.read().await,
            file(Origin::Kept { fetched_at: START }),
            "{what}"
        );
    }
}

#[tokio::test]
async fn an_unreadable_kept_file_is_never_confirmed_or_used_in_place_of_the_exchange() {
    let (_, _, body) = UNREADABLE[0];
    for (status, reason) in [
        (304, "the exchange answered HTTP 304"),
        (503, "the exchange answered HTTP 503"),
    ] {
        let fixture = fixture().await;
        fixture.keep_unchecked(body).await;
        // Stale but within the week in which a readable copy would still serve.
        fixture.clock.advance(25.0 * HOUR);
        fixture
            .exchange_answers(ResponseTemplate::new(status))
            .await;

        assert_eq!(
            fixture.read().await,
            SellersJsonFile {
                content: Err(reason.into()),
                origin: Origin::Downloaded,
            },
            "{status}"
        );
        assert_eq!(fixture.asked_about().await, [None], "{status}");
        assert_eq!(
            fixture.kept().await.content,
            Err(reason.to_string()),
            "{status}: the unreadable file is gone"
        );
    }
}
