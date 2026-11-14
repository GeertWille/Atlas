//! Bidder checks: reads the scanned site's ads.txt and, unless the user
//! switched that off, the sellers.json of the exchanges that file names, then
//! verifies each bidder's setup. Of ads.txt only the derived results are
//! kept; a sellers.json is kept between scans (`application/sellers_json`).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use adscan_model::{CaptureProxy, DiagnosticStatus, LogKind, LogLevel};
use serde_json::json;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::application::AppError;
use crate::application::pipeline::audit::StepRun;
use crate::application::pipeline::state::{NodeError, NodeResult};
use crate::application::ports::{FetchError, FetchProxy, PublicFileRequest};
use crate::application::sellers_json::{Cancelled, Origin, SellersJsonSource};
use crate::domain::diagnostics::ads_txt::{self, AdsTxt, AdsTxtOutcome};
use crate::domain::diagnostics::identity::{ADS_TXT_ACCEPT, BrowserIdentity};
use crate::domain::diagnostics::sellers_cache::utc_day;
use crate::domain::diagnostics::sellers_json::{self, SellersOutcome};
use crate::domain::diagnostics::site::{AdsTxtLocation, ads_txt_location, subdomain_ads_txt_url};
use crate::domain::diagnostics::{self, CopyStoodIn, KeptCopy, SellersLookup, SiteFiles};
use crate::domain::scan::ResolvedBrowser;

/// Far above any real ads.txt; the largest in the wild are a few hundred KB.
const ADS_TXT_MAX_BYTES: usize = 1024 * 1024;
const ADS_TXT_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn diagnose_bidders(run: &mut StepRun<'_>) -> NodeResult {
    let page_url = run.state.capture()?.final_url.clone();
    let location = ads_txt_location(&page_url);
    let (ads_txt_url, outcome) = match &location {
        Some(location) => read_ads_txt(run, &page_url, location).await?,
        None => (
            String::new(),
            AdsTxtOutcome::Unreadable("the page address has no host".into()),
        ),
    };
    // No exchange knows a page that is served from this machine, such as the
    // demo site, so none is asked about it.
    let page_is_local = location.is_some_and(|location| location.loopback_origin.is_some());
    let sellers = if !run.job.settings.check_sellers_json {
        None
    } else if page_is_local {
        run.info("sellers.json is not looked up for a page served from this machine")
            .await?;
        None
    } else {
        Some(match &outcome {
            AdsTxtOutcome::Found(file) => read_sellers_json(run, file).await?,
            AdsTxtOutcome::Missing | AdsTxtOutcome::Unreadable(_) => SellersRead::default(),
        })
    };
    let (sellers, sellers_copies) = match sellers {
        Some(read) => (Some(read.outcomes), read.copies),
        None => (None, BTreeMap::new()),
    };
    let files = SiteFiles {
        ads_txt_url,
        ads_txt: outcome,
        sellers,
        sellers_copies,
    };

    let mut diagnosed = diagnostics::diagnose(run.state.capture()?, run.state.telemetry()?, &files);
    if !run.state.archived {
        // Without an archive there are no request rows to point at.
        for check in diagnosed
            .bidders
            .iter_mut()
            .flat_map(|bidder| &mut bidder.checks)
        {
            check.request_ids.clear();
        }
    }
    let scan_id = run.state.summary.id.clone();
    run.deps
        .scans
        .save_diagnostics(&scan_id, &diagnosed)
        .await?;

    let count = |status: DiagnosticStatus| {
        diagnosed
            .bidders
            .iter()
            .flat_map(|bidder| &bidder.checks)
            .filter(|check| check.status == status)
            .count()
    };
    let summary = format!(
        "{} bidders checked: {} problems, {} notices; ads.txt {}; sellers.json {}",
        diagnosed.bidders.len(),
        count(DiagnosticStatus::Problem),
        count(DiagnosticStatus::Notice),
        match &files.ads_txt {
            AdsTxtOutcome::Found(file) => format!("has {} records", file.records.len()),
            AdsTxtOutcome::Missing => "is missing".to_string(),
            AdsTxtOutcome::Unreadable(_) => "could not be read".to_string(),
        },
        match &files.sellers {
            Some(read) => format!("read at {} exchanges", read.len()),
            None => "not checked".to_string(),
        },
    );
    run.log(
        LogLevel::Info,
        LogKind::Message,
        format!("Bidder checks: {summary}"),
        Some(json!({ "diagnostics": diagnosed })),
    )
    .await?;
    run.state.diagnostics = Some(diagnosed);
    Ok(summary)
}

/// The site's ads.txt is read the way its pages were: through the scan's
/// proxy, so a site that answers differently per country is judged by the
/// same answer. A Browserbase browser's own network cannot be used for it:
/// then the file is read through the user's proxy if the scan has one, and
/// from this machine otherwise.
fn scan_proxy(run: &StepRun<'_>) -> FetchProxy {
    FetchProxy {
        route: run.job.scan.proxy.fetch.clone(),
        password: run.job.proxy_password.clone(),
    }
}

/// Returns the address that was read and what came of it.
async fn read_ads_txt(
    run: &mut StepRun<'_>,
    page_url: &str,
    location: &AdsTxtLocation,
) -> Result<(String, AdsTxtOutcome), NodeError> {
    let root = fetch_ads_txt(run, location, location.url.clone()).await?;
    let subdomain_url = match &root {
        AdsTxtOutcome::Found(file) => subdomain_ads_txt_url(page_url, &file.subdomains),
        _ => None,
    };
    let Some(subdomain_url) = subdomain_url else {
        return Ok((location.url.clone(), root));
    };
    run.info(format!(
        "{} hands this subdomain over to its own ads.txt",
        location.url
    ))
    .await?;
    let own = fetch_ads_txt(run, location, subdomain_url.clone()).await?;
    Ok((subdomain_url, own))
}

async fn fetch_ads_txt(
    run: &mut StepRun<'_>,
    location: &AdsTxtLocation,
    url: String,
) -> Result<AdsTxtOutcome, NodeError> {
    let from = match (run.job.scan.browser, &run.job.scan.proxy.fetch) {
        (ResolvedBrowser::Local, _) => "",
        (ResolvedBrowser::Browserbase { .. }, CaptureProxy::Custom { .. }) => {
            " through the scan's proxy, from this Mac (not from the Browserbase browser)"
        }
        (ResolvedBrowser::Browserbase { .. }, CaptureProxy::None | CaptureProxy::System) => {
            " from this Mac, not from the Browserbase browser: a site that answers differently per country may have shown the browser another file"
        }
    };
    run.info(format!("Requesting {url}{from}")).await?;
    let request = PublicFileRequest {
        url: url.clone(),
        max_bytes: ADS_TXT_MAX_BYTES,
        timeout: ADS_TXT_TIMEOUT,
        loopback_origin: location.loopback_origin.clone(),
        proxy: scan_proxy(run),
        known_version: None,
        identity: scan_identity(run)?,
        accept: ADS_TXT_ACCEPT,
    };
    let outcome = match run.deps.files.fetch(&request, &run.job.cancel).await {
        Ok(file) => ads_txt::read_response(file.status, file.content_type.as_deref(), &file.body),
        Err(FetchError::Cancelled) => return Err(NodeError::Cancelled),
        Err(error) => AdsTxtOutcome::Unreadable(error.to_string()),
    };
    let message = match &outcome {
        AdsTxtOutcome::Found(file) => format!(
            "{url}: {} seller records, {} invalid lines",
            file.records.len(),
            file.invalid.len()
        ),
        AdsTxtOutcome::Missing => format!("{url}: the site has no ads.txt"),
        AdsTxtOutcome::Unreadable(reason) => format!("{url} could not be read: {reason}"),
    };
    run.info(message).await?;
    Ok(outcome)
}

/// What the exchanges' sellers.json say, by ad system domain.
#[derive(Default)]
struct SellersRead {
    outcomes: BTreeMap<String, SellersOutcome>,
    /// Where a kept copy stood in for the exchange's answer.
    copies: BTreeMap<String, KeptCopy>,
}

/// Reads the sellers.json of every ad system whose seller ids need a lookup,
/// all at once: each is a different host.
async fn read_sellers_json(run: &mut StepRun<'_>, file: &AdsTxt) -> Result<SellersRead, NodeError> {
    let lookups = diagnostics::sellers_lookups(run.state.capture()?, file);
    if lookups.is_empty() {
        return Ok(SellersRead::default());
    }
    run.info(format!(
        "Reading sellers.json of {}",
        lookups
            .iter()
            .map(|lookup| lookup.ad_system_domain.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ))
    .await?;
    let identity = scan_identity(run)?;
    let mut pending = JoinSet::new();
    for lookup in lookups {
        pending.spawn(look_up_sellers(
            Arc::clone(&run.deps.sellers_json),
            lookup,
            identity.clone(),
            run.job.cancel.clone(),
        ));
    }
    let now = run.deps.clock.now_ms();
    let mut read = SellersRead::default();
    while let Some(joined) = pending.join_next().await {
        let (domain, looked_up) = joined.map_err(|error| {
            NodeError::Failed(AppError::Internal(format!(
                "a sellers.json lookup failed: {error}"
            )))
        })?;
        let (outcome, origin) = looked_up.map_err(|Cancelled| NodeError::Cancelled)?;
        let source = match origin {
            Origin::Downloaded => "downloaded".to_string(),
            Origin::Confirmed => "kept copy, confirmed unchanged by the exchange".to_string(),
            Origin::Kept { fetched_at } => {
                format!("kept copy, fetched {}", age(now - fetched_at))
            }
            Origin::KeptInstead {
                fetched_at,
                because,
            } => format!(
                "{}: kept copy, fetched {}",
                match because {
                    CopyStoodIn::ExchangeUnreachable => "the exchange could not be reached",
                    CopyStoodIn::AnswerNotSellersJson =>
                        "the exchange's answer was not a valid sellers.json",
                },
                utc_day(fetched_at)
            ),
        };
        let message = match (&outcome, origin) {
            (SellersOutcome::Read(sellers), _) => format!(
                "sellers.json of {domain}: {} of the seller ids found ({source})",
                sellers.len()
            ),
            // A failed lookup is not asked again within the hour.
            (SellersOutcome::Unavailable(reason), Origin::Kept { fetched_at }) => format!(
                "sellers.json of {domain} could not be read {}: {reason}",
                age(now - fetched_at)
            ),
            (SellersOutcome::Unavailable(reason), _) => {
                format!("sellers.json of {domain} could not be read: {reason}")
            }
        };
        run.info(message).await?;
        if let Origin::KeptInstead {
            fetched_at,
            because,
        } = origin
        {
            read.copies.insert(
                domain.clone(),
                KeptCopy {
                    fetched_on: utc_day(fetched_at),
                    because,
                },
            );
        }
        read.outcomes.insert(domain, outcome);
    }
    Ok(read)
}

/// How long ago, in the unit a person would say it in.
fn age(ms: f64) -> String {
    let minutes = (ms / 60_000.0).max(0.0) as u64;
    match minutes {
        0 => "less than a minute ago".to_string(),
        1..60 => format!("{minutes} min ago"),
        _ => format!("{} h ago", minutes / 60),
    }
}

/// The scan's browser as the app's own requests to sites present it.
fn scan_identity(run: &StepRun<'_>) -> Result<BrowserIdentity, NodeError> {
    let capture = run.state.capture()?;
    let language = capture
        .environment
        .browserbase
        .as_ref()
        .and_then(|facts| facts.language.as_deref());
    Ok(BrowserIdentity::of_scan(&capture.user_agent, language))
}

async fn look_up_sellers(
    source: Arc<SellersJsonSource>,
    lookup: SellersLookup,
    identity: BrowserIdentity,
    cancel: CancellationToken,
) -> (String, Result<(SellersOutcome, Origin), Cancelled>) {
    let file = match source
        .read(&lookup.ad_system_domain, &identity, &cancel)
        .await
    {
        Ok(file) => file,
        Err(Cancelled) => return (lookup.ad_system_domain, Err(Cancelled)),
    };
    let outcome = match file.content {
        Ok(json) => {
            let ids = lookup.seller_ids;
            // Megabytes of JSON, and a kept copy is as little trusted as a
            // download: parsed off the async workers, by the same parser.
            tokio::task::spawn_blocking(move || sellers_json::find_sellers(&json, &ids))
                .await
                .unwrap_or_else(|error| Err(format!("the file could not be parsed ({error})")))
                .map_or_else(SellersOutcome::Unavailable, SellersOutcome::Read)
        }
        Err(reason) => SellersOutcome::Unavailable(reason),
    };
    (lookup.ad_system_domain, Ok((outcome, file.origin)))
}
