//! The built-in demo publisher site: a fictional news site with a real-looking
//! ad stack (and planted bugs, see `demo-site/README.md`), embedded in the
//! binary and served on loopback ports: the site itself, a second origin for
//! its ad-tech partners, and a partner that hangs up on every caller.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::DemoSite;

struct Asset {
    path: &'static str,
    content_type: &'static str,
    body: &'static str,
}

const HTML: &str = "text/html; charset=utf-8";
const JAVASCRIPT: &str = "text/javascript; charset=utf-8";
const JSON: &str = "application/json";

macro_rules! asset {
    ($path:literal, $content_type:expr, $file:literal) => {
        Asset {
            path: $path,
            content_type: $content_type,
            body: include_str!(concat!("../demo-site/", $file)),
        }
    };
}

const ASSETS: &[Asset] = &[
    asset!(
        "/static/css/site.css",
        "text/css; charset=utf-8",
        "static/css/site.css"
    ),
    asset!("/static/favicon.svg", "image/svg+xml", "static/favicon.svg"),
    asset!(
        "/static/js/app.9b3e7f.js",
        JAVASCRIPT,
        "static/js/app.9b3e7f.js"
    ),
    asset!(
        "/static/js/vendors-ui.4c1d2a.chunk.js",
        JAVASCRIPT,
        "static/js/vendors-ui.4c1d2a.chunk.js"
    ),
    asset!(
        "/static/js/analytics.js",
        JAVASCRIPT,
        "static/js/analytics.js"
    ),
    asset!(
        "/static/js/analytics-worker.js",
        JAVASCRIPT,
        "static/js/analytics-worker.js"
    ),
    asset!("/static/ads/prebid.js", JAVASCRIPT, "static/ads/prebid.js"),
    asset!("/static/ads/apstag.js", JAVASCRIPT, "static/ads/apstag.js"),
    asset!(
        "/static/ads/hb-wrapper.min.js",
        JAVASCRIPT,
        "static/ads/hb-wrapper.min.js"
    ),
    asset!("/tag/js/gpt.js", JAVASCRIPT, "tag/js/gpt.js"),
    asset!("/ads.txt", "text/plain; charset=utf-8", "ads.txt"),
];

/// What is served as it is, byte for byte: an image and a font.
struct BinaryAsset {
    path: &'static str,
    content_type: &'static str,
    body: &'static [u8],
}

const BINARY_ASSETS: &[BinaryAsset] = &[
    BinaryAsset {
        path: "/static/img/harbour-dusk.png",
        content_type: "image/png",
        body: include_bytes!("../demo-site/static/img/harbour-dusk.png"),
    },
    BinaryAsset {
        path: "/static/fonts/meridian-initial.ttf",
        content_type: "font/ttf",
        body: include_bytes!("../demo-site/static/fonts/meridian-initial.ttf"),
    },
];

/// Everything of a page but its article: head, masthead, rail, footer.
const SHELL: &str = include_str!("../demo-site/index.html");

/// An article of the site: served as a page of its own at `path`, and as the
/// bare article at `/fragments/<slug>.html` for the page's in-place routing.
struct Article {
    slug: &'static str,
    path: &'static str,
    fragment: &'static str,
}

macro_rules! article {
    ($slug:literal, $path:expr) => {
        Article {
            slug: $slug,
            path: $path,
            fragment: include_str!(concat!("../demo-site/fragments/", $slug, ".html")),
        }
    };
}

const ARTICLES: &[Article] = &[
    article!("tide-powered-town", "/"),
    article!("village-substation", "/climate/village-substation"),
    article!("kelp-carbon", "/climate/kelp-carbon"),
    article!("white-roofs", "/climate/white-roofs"),
    article!("permit-queue", "/climate/permit-queue"),
];

impl Article {
    fn page(&self) -> String {
        SHELL
            .replace("%TITLE%", self.attribute("data-title"))
            .replace("%DESCRIPTION%", self.standfirst())
            .replace("%ARTICLE_ID%", self.slug)
            .replace("%ARTICLE%", self.fragment)
    }

    fn attribute(&self, name: &str) -> &'static str {
        let marker = format!("{name}=\"");
        self.fragment
            .split_once(marker.as_str())
            .and_then(|(_, rest)| rest.split_once('"'))
            .map_or("", |(value, _)| value)
    }

    fn standfirst(&self) -> &'static str {
        self.fragment
            .split_once("<p class=\"standfirst\">")
            .and_then(|(_, rest)| rest.split_once("</p>"))
            .map_or("", |(text, _)| text)
    }
}

/// Where the page's analytics script posts its beacons.
const BEACON_PATH: &str = "/collect";
/// The sampling rules the analytics worker fetches.
const SAMPLING_PATH: &str = "/collect/sampling.json";
const SAMPLING_RULES: &str = r#"{"pageView":1,"scrollDepth":0.5,"engagement":1}"#;
/// How often each path was requested, as JSON. Tests read it to prove that a
/// scripted visitor never requested a link it must not follow.
const HITS_PATH: &str = "/__demo/hits";
/// The index the page's search box loads: one large JSON document.
const ARCHIVE_INDEX_PATH: &str = "/api/archive-index.json";
const ARCHIVE_INDEX_ENTRIES: usize = 9_000;

/// The demo exchanges, one per bidder of the wrapper, at the path of that
/// bidder's real endpoint. Each is a mirror: the page's seeded simulation
/// decides what the bidder answers and when, sends that along in
/// `ext.standin`, and the exchange waits and answers with it.
const EXCHANGES: &[(&str, &str)] = &[
    ("appnexus", "/ut/v3/prebid"),
    ("rubicon", "/a/api/fastlane.json"),
    ("ix", "/openrtb/pbjs"),
    ("pubmatic", "/translator"),
    ("criteo", "/openrtb_2_5/pbjs/auction/request"),
    ("triplelift", "/header/auction"),
    ("openx", OPENX_PATH),
];
/// The exchange behind the `openx` bidder has blocked the publisher and says
/// so with an OpenRTB no-bid reason, whatever it is asked.
const OPENX_PATH: &str = "/openrtbb/prebidjs";
const OPENX_ANSWER: &str = r#"{"id":"meridian-post","nbr":7}"#;
/// No stand-in bidder is slower than this.
const MAX_EXCHANGE_LATENCY: Duration = Duration::from_secs(5);

/// A cookie sync in three hops: the site, the partner (which sets its id),
/// and back to the site for the pixel.
const SYNC_START_PATH: &str = "/usersync/start";
const SYNC_MATCH_PATH: &str = "/usersync/match";
const SYNC_PIXEL_PATH: &str = "/usersync/done.gif";
/// The partner's sync frame, and what the script in it asks for.
const SYNC_FRAME_PATH: &str = "/usersync/frame.html";
const SYNC_IDS_PATH: &str = "/usersync/ids.json";
const SYNC_FRAME: &str = r#"<!doctype html><title>Partner sync</title><script>
fetch('/usersync/ids.json').then(function (answer) { return answer.json(); }).then(function (ids) {
  new Image().src = '/usersync/done.gif?from=frame&uid=' + encodeURIComponent(ids.uid);
});
</script>"#;
const PARTNER_ID: &str = "mp-7f3a9c1e5b";
/// A transparent 1x1 GIF.
const PIXEL: &[u8] = &[
    0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xff, 0xff, 0xff, 0x21, 0xf9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x44, 0x01, 0x00, 0x3b,
];

type Hits = Arc<Mutex<BTreeMap<String, u32>>>;

/// What the pages are told about where the site's partners are.
struct Site {
    hits: Hits,
    /// The site's own origin, e.g. `http://127.0.0.1:49152`.
    origin: String,
    /// Another origin, of another site: the same server under the name
    /// `localhost`, on a port of its own.
    partner_origin: String,
    /// An origin that accepts connections and closes them unanswered.
    identity_origin: String,
    archive_index: String,
}

pub(crate) async fn start() -> std::io::Result<DemoSite> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let partner = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let hangs_up = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let site = Arc::new(Site {
        hits: Hits::default(),
        partner_origin: format!("http://localhost:{}", partner.local_addr()?.port()),
        identity_origin: format!("http://localhost:{}", hangs_up.local_addr()?.port()),
        archive_index: archive_index(),
        origin: origin.clone(),
    });
    let shutdown = CancellationToken::new();
    for listener in [listener, partner] {
        let routes = Router::new().fallback(respond).with_state(site.clone());
        let stopped = shutdown.clone().cancelled_owned();
        tokio::spawn(async move {
            let _ = axum::serve(listener, routes)
                .with_graceful_shutdown(stopped)
                .await;
        });
    }
    let stopped = shutdown.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = stopped.cancelled() => break,
                // Accepted and dropped: the caller gets an empty reply.
                accepted = hangs_up.accept() => drop(accepted),
            }
        }
    });
    Ok(DemoSite {
        url: format!("{origin}/"),
        shutdown,
    })
}

async fn respond(
    State(site): State<Arc<Site>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = uri.path();
    if path == HITS_PATH {
        let hits = site
            .hits
            .lock()
            .map(|hits| hits.clone())
            .unwrap_or_default();
        return typed(JSON, serde_json::to_string(&hits).unwrap_or_default());
    }
    if let Ok(mut hits) = site.hits.lock() {
        *hits.entry(path.to_string()).or_default() += 1;
    }
    if let Some((bidder, _)) = EXCHANGES.iter().find(|(_, at)| *at == path) {
        return exchange(bidder, &method, &headers, &body).await;
    }
    match path {
        BEACON_PATH => StatusCode::NO_CONTENT.into_response(),
        SAMPLING_PATH => typed(JSON, SAMPLING_RULES),
        ARCHIVE_INDEX_PATH => typed(JSON, site.archive_index.clone()),
        SYNC_START_PATH => redirect(
            format!("{}{SYNC_MATCH_PATH}?partner=meridian", site.partner_origin),
            None,
        ),
        SYNC_MATCH_PATH => redirect(
            format!("{}{SYNC_PIXEL_PATH}?uid={PARTNER_ID}", site.origin),
            Some(format!("uid={PARTNER_ID}; Path=/; Max-Age=7776000")),
        ),
        SYNC_PIXEL_PATH => typed("image/gif", PIXEL),
        SYNC_FRAME_PATH => typed(HTML, SYNC_FRAME),
        SYNC_IDS_PATH => typed(
            JSON,
            format!(r#"{{"uid":"{PARTNER_ID}","partners":["meridian"]}}"#),
        ),
        _ => file(&site, path, &method),
    }
}

/// One of the site's files or pages.
fn file(site: &Site, path: &str, method: &Method) -> Response {
    let found = BINARY_ASSETS
        .iter()
        .find(|asset| asset.path == path)
        .map(|asset| (asset.content_type, Bytes::from_static(asset.body)))
        .or_else(|| {
            let (content_type, text) = resource(path)?;
            let text = text
                .replace("%PARTNER_ORIGIN%", &site.partner_origin)
                .replace("%IDENTITY_ORIGIN%", &site.identity_origin);
            Some((content_type, Bytes::from(text)))
        });
    let Some((content_type, body)) = found else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        body,
    )
        .into_response()
}

fn typed(content_type: &'static str, body: impl Into<Bytes>) -> Response {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        body.into(),
    )
        .into_response()
}

fn redirect(to: String, cookie: Option<String>) -> Response {
    let mut response = StatusCode::FOUND.into_response();
    let headers = response.headers_mut();
    if let Ok(to) = HeaderValue::from_str(&to) {
        headers.insert(header::LOCATION, to);
    }
    if let Some(cookie) = cookie.and_then(|cookie| HeaderValue::from_str(&cookie).ok()) {
        headers.insert(header::SET_COOKIE, cookie);
    }
    response
}

/// A stand-in exchange answering a bid request. A page of another origin
/// may ask, which the browser first clears with a preflight.
async fn exchange(bidder: &str, method: &Method, headers: &HeaderMap, body: &[u8]) -> Response {
    let mut response = if method == Method::OPTIONS {
        StatusCode::NO_CONTENT.into_response()
    } else if method != Method::POST {
        StatusCode::METHOD_NOT_ALLOWED.into_response()
    } else if bidder == "openx" {
        typed(JSON, OPENX_ANSWER)
    } else {
        let request: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let decided = &request["ext"]["standin"];
        let latency = Duration::from_millis(decided["latencyMs"].as_u64().unwrap_or(0));
        tokio::time::sleep(latency.min(MAX_EXCHANGE_LATENCY)).await;
        typed(JSON, bid_response(bidder, &request).to_string())
    };
    if let Some(origin) = headers.get(header::ORIGIN) {
        let allowed = response.headers_mut();
        allowed.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
        allowed.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("content-type"),
        );
        allowed.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("POST"),
        );
    }
    response
}

/// The OpenRTB answer to `request`: the bids its `ext.standin` names, in one
/// seat, or no seat at all.
fn bid_response(bidder: &str, request: &Value) -> Value {
    let decided = request["ext"]["standin"]["bids"].as_array();
    let bids: Vec<Value> = decided
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(index, bid)| {
            json!({
                "id": format!("{bidder}-{}", index + 1),
                "impid": bid["impid"],
                "price": bid["price"],
                "w": bid["w"],
                "h": bid["h"],
                "crid": format!("{bidder}-cr-{}x{}", bid["w"], bid["h"]),
                "adomain": ["example-advertiser.test"],
                "adm": format!("<!-- stand-in creative of {bidder} -->"),
            })
        })
        .collect();
    let seats = if bids.is_empty() {
        json!([])
    } else {
        json!([{ "seat": bidder, "bid": bids }])
    };
    json!({ "id": request["id"], "cur": "USD", "seatbid": seats })
}

/// A search index of the paper's archive: the same few headlines over many
/// years, more than a megabyte of JSON.
fn archive_index() -> String {
    let mut index = String::from(r#"{"version":3,"articles":["#);
    for entry in 0..ARCHIVE_INDEX_ENTRIES {
        let article = &ARTICLES[entry % ARTICLES.len()];
        let separator = if entry == 0 { "" } else { "," };
        let _ = write!(
            index,
            r#"{separator}{{"id":"{slug}-{entry}","path":"{path}","title":{title},"section":"climate","year":{year},"words":{words}}}"#,
            slug = article.slug,
            path = article.path,
            title = Value::from(article.attribute("data-title")),
            year = 1987 + entry % 40,
            words = 600 + (entry * 37) % 2_400,
        );
    }
    index.push_str("]}");
    index
}

/// Content type and body of the text file or page served at `path`, with
/// the partners' origins still to be filled in.
fn resource(path: &str) -> Option<(&'static str, String)> {
    if let Some(asset) = ASSETS.iter().find(|asset| asset.path == path) {
        return Some((asset.content_type, asset.body.to_string()));
    }
    if let Some(article) = ARTICLES.iter().find(|article| article.path == path) {
        return Some((HTML, article.page()));
    }
    let slug = path.strip_prefix("/fragments/")?.strip_suffix(".html")?;
    let article = ARTICLES.iter().find(|article| article.slug == slug)?;
    Some((HTML, article.fragment.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paths the page links to that a scripted visitor must never request.
    /// None of them is served.
    const TRAPS: [&str; 5] = [
        "/account/login",
        "/logout",
        "/files/media-kit.pdf",
        "/sponsored/partner-story",
        "/sponsored/offer",
    ];

    fn entry_page() -> String {
        resource("/").unwrap().1
    }

    fn site() -> Arc<Site> {
        Arc::new(Site {
            hits: Hits::default(),
            origin: "http://127.0.0.1:4455".into(),
            partner_origin: "http://localhost:4456".into(),
            identity_origin: "http://localhost:4457".into(),
            archive_index: archive_index(),
        })
    }

    async fn ask(site: &Arc<Site>, method: Method, address: &'static str, body: &str) -> Response {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:4455"),
        );
        let body = Bytes::from(body.to_string());
        respond(
            State(site.clone()),
            method,
            Uri::from_static(address),
            headers,
            body,
        )
        .await
    }

    async fn text_of(response: Response) -> String {
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(body.to_vec()).unwrap()
    }

    fn header_of<'a>(response: &'a Response, name: header::HeaderName) -> &'a str {
        response
            .headers()
            .get(name)
            .map_or("", |value| value.to_str().unwrap())
    }

    #[test]
    fn every_local_reference_of_the_page_is_served() {
        let page = entry_page();
        let references = page
            .split(['"', '\''])
            .filter(|part| part.starts_with("/static/") || part.starts_with("/tag/"));
        let mut checked = 0;
        for reference in references {
            let is_binary = BINARY_ASSETS.iter().any(|asset| asset.path == reference);
            assert!(
                is_binary || resource(reference).is_some(),
                "{reference} is not served"
            );
            checked += 1;
        }
        assert!(
            checked >= 8,
            "expected the page to reference its scripts and styles, found {checked}"
        );
    }

    #[test]
    fn every_article_is_a_page_of_its_own_and_a_fragment_for_in_page_routing() {
        for article in ARTICLES {
            let (content_type, page) = resource(article.path).unwrap();
            assert_eq!(content_type, HTML);
            for placeholder in ["%TITLE%", "%DESCRIPTION%", "%ARTICLE_ID%", "%ARTICLE%"] {
                assert!(
                    !page.contains(placeholder),
                    "{}: {placeholder}",
                    article.slug
                );
            }
            let title = article.attribute("data-title");
            assert!(title.len() > 10, "{}", article.slug);
            assert!(page.contains(&format!("<title>{title} | The Meridian Post</title>")));
            assert!(page.contains(&format!("articleId: '{}'", article.slug)));
            assert!(
                page.contains("<header class=\"masthead\">") && page.contains(article.fragment)
            );

            let (_, fragment) = resource(&format!("/fragments/{}.html", article.slug)).unwrap();
            assert!(fragment.trim_start().starts_with("<article"));
            for slot in ["ad-inarticle-1", "ad-inarticle-2"] {
                assert_eq!(
                    fragment.matches(slot).count(),
                    1,
                    "{}: {slot}",
                    article.slug
                );
            }
        }
        assert!(resource("/fragments/unknown.html").is_none());
        assert!(resource("/climate/unknown").is_none());
    }

    #[test]
    fn every_route_the_page_links_to_exists_and_no_trap_does() {
        let page = entry_page();
        let routes: Vec<&str> = page
            .split("href=\"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .filter(|href| href.starts_with("/climate/"))
            .collect();
        assert!(routes.len() >= 6, "{routes:?}");
        for route in routes {
            assert!(resource(route).is_some(), "{route} is not served");
        }
        for trap in TRAPS {
            assert!(
                page.contains(&format!("href=\"{trap}\"")),
                "{trap} is not linked"
            );
            assert!(resource(trap).is_none(), "{trap} is served");
        }
        assert!(page.contains("href=\"mailto:"));
        assert!(page.contains("rel=\"sponsored\""));
        assert!(page.contains("class=\"ad-slot ad-slot--native\""));
    }

    #[tokio::test]
    async fn requests_are_counted_by_path_and_the_count_itself_is_not() {
        let site = site();
        let get = |path: &'static str| ask(&site, Method::GET, path, "");
        assert_eq!(get("/").await.status(), StatusCode::OK);
        assert_eq!(
            get("/climate/kelp-carbon?ref=home").await.status(),
            StatusCode::OK
        );
        assert_eq!(get("/logout").await.status(), StatusCode::NOT_FOUND);
        assert_eq!(get("/logout").await.status(), StatusCode::NOT_FOUND);
        assert_eq!(get(HITS_PATH).await.status(), StatusCode::OK);

        let counted = site.hits.lock().unwrap().clone();
        assert_eq!(
            counted,
            BTreeMap::from([
                ("/".to_string(), 1),
                ("/climate/kelp-carbon".to_string(), 1),
                ("/logout".to_string(), 2),
            ])
        );
    }

    #[tokio::test]
    async fn what_is_served_knows_where_the_partners_are() {
        let site = site();
        let library = text_of(ask(&site, Method::GET, "/static/ads/prebid.js", "").await).await;
        assert!(library.contains("var PARTNER_ORIGIN = 'http://localhost:4456';"));
        assert!(library.contains("var IDENTITY_ORIGIN = 'http://localhost:4457';"));
        assert!(!library.contains("%PARTNER_ORIGIN%") && !library.contains("%IDENTITY_ORIGIN%"));
        // The stand-in library asks every exchange the server has.
        for (bidder, path) in EXCHANGES {
            assert!(library.contains(&format!("{path}'")), "{bidder}: {path}");
        }

        let font = ask(&site, Method::GET, "/static/fonts/meridian-initial.ttf", "").await;
        assert_eq!(header_of(&font, header::CONTENT_TYPE), "font/ttf");
        let image = ask(&site, Method::GET, "/static/img/harbour-dusk.png", "").await;
        assert_eq!(header_of(&image, header::CONTENT_TYPE), "image/png");
        let styles = resource("/static/css/site.css").unwrap().1;
        assert!(styles.contains("url(\"/static/fonts/meridian-initial.ttf\")"));
    }

    #[tokio::test]
    async fn an_exchange_answers_what_the_request_tells_it_after_the_latency_it_names() {
        let site = site();
        let request = r#"{"id":"req-1","imp":[{"id":"bid-a"},{"id":"bid-b"}],"tmax":250,
            "ext":{"standin":{"latencyMs":120,"bids":[{"impid":"bid-b","price":1.75,"w":300,"h":250}]}}}"#;
        let asked = tokio::time::Instant::now();
        let answer = ask(&site, Method::POST, "/translator", request).await;
        assert!(asked.elapsed() >= Duration::from_millis(120));
        assert_eq!(header_of(&answer, header::CONTENT_TYPE), JSON);
        let answer: Value = serde_json::from_str(&text_of(answer).await).unwrap();
        assert_eq!(
            answer,
            json!({
                "id": "req-1",
                "cur": "USD",
                "seatbid": [{ "seat": "pubmatic", "bid": [{
                    "id": "pubmatic-1",
                    "impid": "bid-b",
                    "price": 1.75,
                    "w": 300,
                    "h": 250,
                    "crid": "pubmatic-cr-300x250",
                    "adomain": ["example-advertiser.test"],
                    "adm": "<!-- stand-in creative of pubmatic -->",
                }] }],
            })
        );

        // A bidder that passes answers without a seat.
        let passed =
            r#"{"id":"req-2","imp":[{"id":"bid-c"}],"ext":{"standin":{"latencyMs":0,"bids":[]}}}"#;
        let answer = text_of(ask(&site, Method::POST, "/openrtb/pbjs", passed).await).await;
        assert_eq!(
            serde_json::from_str::<Value>(&answer).unwrap(),
            json!({ "id": "req-2", "cur": "USD", "seatbid": [] })
        );
        // The exchange that blocked the publisher says so, whatever it is told.
        let answer = text_of(ask(&site, Method::POST, OPENX_PATH, request).await).await;
        assert_eq!(answer, OPENX_ANSWER);
        assert_eq!(
            ask(&site, Method::GET, "/translator", "").await.status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }

    #[tokio::test]
    async fn an_exchange_lets_a_page_of_another_origin_ask() {
        let site = site();
        let preflight = ask(&site, Method::OPTIONS, "/ut/v3/prebid", "").await;
        assert_eq!(preflight.status(), StatusCode::NO_CONTENT);
        let answer = ask(&site, Method::POST, "/ut/v3/prebid", "{}").await;
        for response in [&preflight, &answer] {
            assert_eq!(
                header_of(response, header::ACCESS_CONTROL_ALLOW_ORIGIN),
                "http://127.0.0.1:4455"
            );
            assert_eq!(
                header_of(response, header::ACCESS_CONTROL_ALLOW_HEADERS),
                "content-type"
            );
        }
    }

    #[tokio::test]
    async fn the_cookie_sync_goes_to_the_partner_and_back_in_three_hops() {
        let site = site();
        let first = ask(&site, Method::GET, "/usersync/start?bidder=rubicon", "").await;
        assert_eq!(first.status(), StatusCode::FOUND);
        assert_eq!(
            header_of(&first, header::LOCATION),
            "http://localhost:4456/usersync/match?partner=meridian"
        );
        let second = ask(&site, Method::GET, "/usersync/match?partner=meridian", "").await;
        assert_eq!(second.status(), StatusCode::FOUND);
        assert_eq!(
            header_of(&second, header::LOCATION),
            "http://127.0.0.1:4455/usersync/done.gif?uid=mp-7f3a9c1e5b"
        );
        assert!(header_of(&second, header::SET_COOKIE).starts_with("uid=mp-7f3a9c1e5b;"));
        let third = ask(
            &site,
            Method::GET,
            "/usersync/done.gif?uid=mp-7f3a9c1e5b",
            "",
        )
        .await;
        assert_eq!(third.status(), StatusCode::OK);
        assert_eq!(header_of(&third, header::CONTENT_TYPE), "image/gif");

        let frame = text_of(ask(&site, Method::GET, SYNC_FRAME_PATH, "").await).await;
        assert!(frame.contains(SYNC_IDS_PATH) && frame.contains(SYNC_PIXEL_PATH));
        let ids: Value =
            serde_json::from_str(&text_of(ask(&site, Method::GET, SYNC_IDS_PATH, "").await).await)
                .unwrap();
        assert_eq!(ids["uid"], PARTNER_ID);
    }

    #[test]
    fn the_archive_index_is_one_large_json_document() {
        let index = archive_index();
        assert!(index.len() > 1024 * 1024, "{} bytes", index.len());
        assert!(index.len() < 3 * 1024 * 1024, "{} bytes", index.len());
        let parsed: Value = serde_json::from_str(&index).unwrap();
        assert_eq!(
            parsed["articles"].as_array().unwrap().len(),
            ARCHIVE_INDEX_ENTRIES
        );
        assert_eq!(parsed["articles"][1]["path"], "/climate/village-substation");
    }

    #[test]
    fn ads_txt_authorises_every_bidder_of_the_wrapper_but_index_exchange() {
        let (content_type, ads_txt) = resource("/ads.txt").unwrap();
        assert!(content_type.starts_with("text/plain"));
        let mut ad_systems: Vec<&str> = ads_txt
            .lines()
            .filter(|line| line.matches(',').count() >= 2)
            .filter_map(|line| line.split(',').next())
            .collect();
        ad_systems.sort_unstable();
        assert_eq!(
            ad_systems,
            [
                "appnexus.com",
                "criteo.com",
                "openx.com",
                "pubmatic.com",
                "rubiconproject.com",
                "triplelift.com",
            ]
        );
    }

    #[test]
    fn nothing_is_loaded_from_the_internet() {
        // The one address outside the site is a link nobody may follow.
        const OFF_SITE_TRAP: &str = "href=\"https://partner.example/\"";
        let pages = ARTICLES
            .iter()
            .map(|article| (article.path, article.page()));
        let assets = ASSETS
            .iter()
            .map(|asset| (asset.path, asset.body.to_string()));
        for (path, body) in pages.chain(assets) {
            let body = body.replace(OFF_SITE_TRAP, "");
            for scheme in ["http://", "https://", "src=\"//", "href=\"//"] {
                let external = body
                    .match_indices(scheme)
                    .map(|(at, _)| &body[at..(at + 40).min(body.len())])
                    .find(|reference| !reference.contains("www.w3.org"));
                assert_eq!(external, None, "{path} references an external address");
            }
        }
        assert!(entry_page().contains(OFF_SITE_TRAP));
    }

    #[test]
    fn the_served_wrapper_is_minified() {
        let (_, wrapper) = resource("/static/ads/hb-wrapper.min.js").unwrap();
        let longest_line = wrapper.lines().map(str::len).max().unwrap();
        assert!(
            longest_line > 2_000,
            "the wrapper should be one long minified line"
        );
        for identifier in ["isMobileViewport", "isDoubleFire", "watchEngagementRefresh"] {
            assert!(
                !wrapper.contains(identifier),
                "identifiers should be mangled: {identifier}"
            );
        }
    }

    /// The served wrapper is built from the readable one; a change to either
    /// must reach the other.
    #[test]
    fn the_served_wrapper_has_the_planted_logic_of_its_source() {
        let source = include_str!("../demo-site/static/ads/hb-wrapper.src.js");
        let (_, served) = resource("/static/ads/hb-wrapper.min.js").unwrap();
        for literal in [
            "adstack.lastAuction",
            "adstack.pageViews",
            "meridian:route",
            "ad-recirc",
            "engagement_refresh",
            "impressionViewable",
        ] {
            assert!(source.contains(literal), "source lacks {literal}");
            assert!(served.contains(literal), "served wrapper lacks {literal}");
        }
        assert!(
            served.contains("doubleFireGuardMs:12e4"),
            "the guard is 120 s"
        );
    }
}
