//! The built-in demo publisher site: a fictional news page with a real-looking
//! ad stack (and planted bugs, see `demo-site/README.md`), embedded in the
//! binary and served on a loopback port.

use std::net::Ipv4Addr;

use axum::Router;
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
    asset!("/", HTML, "index.html"),
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

/// Where the page's analytics script posts its beacons.
const BEACON_PATH: &str = "/collect";
/// The demo exchange the stand-in OpenX adapter asks for bids. It has
/// blocked the publisher and says so with an OpenRTB no-bid reason.
const EXCHANGE_PATH: &str = "/openrtbb/prebidjs";
const EXCHANGE_ANSWER: &str = r#"{"id":"meridian-post","nbr":7}"#;

pub(crate) async fn start() -> std::io::Result<DemoSite> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let url = format!("http://{}/", listener.local_addr()?);
    let shutdown = CancellationToken::new();
    let stopped = shutdown.clone().cancelled_owned();
    tokio::spawn(async move {
        let _ = axum::serve(listener, Router::new().fallback(respond))
            .with_graceful_shutdown(stopped)
            .await;
    });
    Ok(DemoSite { url, shutdown })
}

async fn respond(method: Method, uri: Uri) -> Response {
    if uri.path() == BEACON_PATH {
        return StatusCode::NO_CONTENT.into_response();
    }
    if uri.path() == EXCHANGE_PATH && method == Method::POST {
        return (
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )],
            EXCHANGE_ANSWER,
        )
            .into_response();
    }
    let Some(asset) = ASSETS.iter().find(|asset| asset.path == uri.path()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static(asset.content_type),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        asset.body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_local_reference_of_the_page_is_served() {
        let page = ASSETS[0].body;
        let references = page
            .split(['"', '\''])
            .filter(|part| part.starts_with("/static/") || part.starts_with("/tag/"));
        let mut checked = 0;
        for reference in references {
            assert!(
                ASSETS.iter().any(|asset| asset.path == reference),
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
    fn ads_txt_authorises_every_bidder_of_the_wrapper_but_index_exchange() {
        let ads_txt = ASSETS
            .iter()
            .find(|asset| asset.path == "/ads.txt")
            .unwrap();
        assert!(ads_txt.content_type.starts_with("text/plain"));
        let mut ad_systems: Vec<&str> = ads_txt
            .body
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
        for asset in ASSETS {
            for scheme in ["http://", "https://", "src=\"//", "href=\"//"] {
                let external = asset
                    .body
                    .match_indices(scheme)
                    .map(|(at, _)| &asset.body[at..(at + 40).min(asset.body.len())])
                    .find(|reference| !reference.contains("www.w3.org"));
                assert_eq!(
                    external, None,
                    "{} references an external address",
                    asset.path
                );
            }
        }
    }

    #[test]
    fn the_served_wrapper_is_minified() {
        let wrapper = ASSETS
            .iter()
            .find(|asset| asset.path.ends_with("hb-wrapper.min.js"))
            .unwrap();
        let longest_line = wrapper.body.lines().map(str::len).max().unwrap();
        assert!(
            longest_line > 2_000,
            "the wrapper should be one long minified line"
        );
        assert!(
            !wrapper.body.contains("isMobileViewport"),
            "identifiers should be mangled"
        );
    }
}
