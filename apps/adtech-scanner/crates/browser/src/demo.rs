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
];

/// Where the page's analytics script posts its beacons.
const BEACON_PATH: &str = "/collect";

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
