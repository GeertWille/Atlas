//! The built-in demo publisher site: a fictional news site with a real-looking
//! ad stack (and planted bugs, see `demo-site/README.md`), embedded in the
//! binary and served on a loopback port.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
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
/// The demo exchange the stand-in OpenX adapter asks for bids. It has
/// blocked the publisher and says so with an OpenRTB no-bid reason.
const EXCHANGE_PATH: &str = "/openrtbb/prebidjs";
const EXCHANGE_ANSWER: &str = r#"{"id":"meridian-post","nbr":7}"#;
/// How often each path was requested, as JSON. Tests read it to prove that a
/// scripted visitor never requested a link it must not follow.
const HITS_PATH: &str = "/__demo/hits";

type Hits = Arc<Mutex<BTreeMap<String, u32>>>;

pub(crate) async fn start() -> std::io::Result<DemoSite> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let url = format!("http://{}/", listener.local_addr()?);
    let shutdown = CancellationToken::new();
    let stopped = shutdown.clone().cancelled_owned();
    let site = Router::new().fallback(respond).with_state(Hits::default());
    tokio::spawn(async move {
        let _ = axum::serve(listener, site)
            .with_graceful_shutdown(stopped)
            .await;
    });
    Ok(DemoSite { url, shutdown })
}

async fn respond(State(hits): State<Hits>, method: Method, uri: Uri) -> Response {
    let path = uri.path();
    if path == HITS_PATH {
        let hits = hits.lock().map(|hits| hits.clone()).unwrap_or_default();
        return (
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )],
            serde_json::to_string(&hits).unwrap_or_default(),
        )
            .into_response();
    }
    if let Ok(mut hits) = hits.lock() {
        *hits.entry(path.to_string()).or_default() += 1;
    }
    if path == BEACON_PATH {
        return StatusCode::NO_CONTENT.into_response();
    }
    if path == EXCHANGE_PATH && method == Method::POST {
        return (
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )],
            EXCHANGE_ANSWER,
        )
            .into_response();
    }
    let Some((content_type, body)) = resource(path) else {
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

/// Content type and body of what is served at `path`.
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

    #[test]
    fn every_local_reference_of_the_page_is_served() {
        let page = entry_page();
        let references = page
            .split(['"', '\''])
            .filter(|part| part.starts_with("/static/") || part.starts_with("/tag/"));
        let mut checked = 0;
        for reference in references {
            assert!(resource(reference).is_some(), "{reference} is not served");
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
        let hits = Hits::default();
        let get =
            |path: &'static str| respond(State(hits.clone()), Method::GET, Uri::from_static(path));
        assert_eq!(get("/").await.status(), StatusCode::OK);
        assert_eq!(
            get("/climate/kelp-carbon?ref=home").await.status(),
            StatusCode::OK
        );
        assert_eq!(get("/logout").await.status(), StatusCode::NOT_FOUND);
        assert_eq!(get("/logout").await.status(), StatusCode::NOT_FOUND);
        assert_eq!(get(HITS_PATH).await.status(), StatusCode::OK);

        let counted = hits.lock().unwrap().clone();
        assert_eq!(
            counted,
            BTreeMap::from([
                ("/".to_string(), 1),
                ("/climate/kelp-carbon".to_string(), 1),
                ("/logout".to_string(), 2),
            ])
        );
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
