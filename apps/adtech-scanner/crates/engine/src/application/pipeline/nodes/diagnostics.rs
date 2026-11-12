//! Bidder checks: reads the scanned site's ads.txt and, when the user allows
//! it, the sellers.json of the exchanges that file names, then verifies each
//! bidder's setup. The files stay in memory; only the derived results are kept.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use adscan_model::{DiagnosticStatus, LogKind, LogLevel};
use serde_json::json;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::application::AppError;
use crate::application::pipeline::audit::StepRun;
use crate::application::pipeline::state::{NodeError, NodeResult};
use crate::application::ports::{FetchError, PublicFileFetcher, PublicFileRequest};
use crate::domain::diagnostics::ads_txt::{self, AdsTxt, AdsTxtOutcome};
use crate::domain::diagnostics::sellers_json::{self, SellersOutcome};
use crate::domain::diagnostics::site::{
    AdsTxtLocation, ads_txt_location, sellers_json_url, subdomain_ads_txt_url,
};
use crate::domain::diagnostics::{self, SellersLookup, SiteFiles};

/// Far above any real ads.txt; the largest in the wild are a few hundred KB.
const ADS_TXT_MAX_BYTES: usize = 1024 * 1024;
const ADS_TXT_TIMEOUT: Duration = Duration::from_secs(10);
/// The sellers.json of a large exchange is a few megabytes.
const SELLERS_JSON_MAX_BYTES: usize = 24 * 1024 * 1024;
const SELLERS_JSON_TIMEOUT: Duration = Duration::from_secs(20);

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
            AdsTxtOutcome::Missing | AdsTxtOutcome::Unreadable(_) => BTreeMap::new(),
        })
    };
    let files = SiteFiles {
        ads_txt_url,
        ads_txt: outcome,
        sellers,
    };

    let diagnosed = diagnostics::diagnose(run.state.capture()?, run.state.telemetry()?, &files);
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
    run.info(format!("Requesting {url}")).await?;
    let request = PublicFileRequest {
        url: url.clone(),
        max_bytes: ADS_TXT_MAX_BYTES,
        timeout: ADS_TXT_TIMEOUT,
        loopback_origin: location.loopback_origin.clone(),
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

/// Reads the sellers.json of every ad system whose seller ids need a lookup,
/// all at once: each is a different host.
async fn read_sellers_json(
    run: &mut StepRun<'_>,
    file: &AdsTxt,
) -> Result<BTreeMap<String, SellersOutcome>, NodeError> {
    let lookups = diagnostics::sellers_lookups(run.state.capture()?, file);
    if lookups.is_empty() {
        return Ok(BTreeMap::new());
    }
    run.info(format!(
        "Requesting sellers.json from {}",
        lookups
            .iter()
            .map(|lookup| lookup.ad_system_domain.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ))
    .await?;
    let mut pending = JoinSet::new();
    for lookup in lookups {
        pending.spawn(look_up_sellers(
            Arc::clone(&run.deps.files),
            lookup,
            run.job.cancel.clone(),
        ));
    }
    let mut read = BTreeMap::new();
    while let Some(joined) = pending.join_next().await {
        let (domain, outcome) = joined.map_err(|error| {
            NodeError::Failed(AppError::Internal(format!(
                "a sellers.json lookup failed: {error}"
            )))
        })?;
        let outcome = outcome?;
        let message = match &outcome {
            SellersOutcome::Read(sellers) => {
                format!(
                    "sellers.json of {domain}: {} of the seller ids found",
                    sellers.len()
                )
            }
            SellersOutcome::Unavailable(reason) => {
                format!("sellers.json of {domain} could not be read: {reason}")
            }
        };
        run.info(message).await?;
        read.insert(domain, outcome);
    }
    Ok(read)
}

async fn look_up_sellers(
    files: Arc<dyn PublicFileFetcher>,
    lookup: SellersLookup,
    cancel: CancellationToken,
) -> (String, Result<SellersOutcome, NodeError>) {
    let request = PublicFileRequest {
        url: sellers_json_url(&lookup.ad_system_domain),
        max_bytes: SELLERS_JSON_MAX_BYTES,
        timeout: SELLERS_JSON_TIMEOUT,
        loopback_origin: None,
    };
    let outcome = match files.fetch(&request, &cancel).await {
        Ok(file) if (200..300).contains(&file.status) => {
            let ids = lookup.seller_ids;
            // Megabytes of JSON: parsed off the async workers.
            tokio::task::spawn_blocking(move || sellers_json::find_sellers(&file.body, &ids))
                .await
                .unwrap_or_else(|error| Err(format!("the file could not be parsed ({error})")))
                .map_or_else(SellersOutcome::Unavailable, SellersOutcome::Read)
        }
        Ok(file) => {
            SellersOutcome::Unavailable(format!("the exchange answered HTTP {}", file.status))
        }
        Err(FetchError::Cancelled) => return (lookup.ad_system_domain, Err(NodeError::Cancelled)),
        Err(error) => SellersOutcome::Unavailable(error.to_string()),
    };
    (lookup.ad_system_domain, Ok(outcome))
}
