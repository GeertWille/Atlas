//! Reads the sellers.json of an ad exchange: from the copy kept between
//! scans when that is recent enough, from the exchange otherwise.
//!
//! The file is the same whoever asks and wherever from, which is what lets
//! every scan of every site share one copy per exchange. It is therefore
//! requested the way the app's other traffic is (the system's proxy, if
//! any), never through the proxy of one scan: a copy fetched through one
//! scan's proxy would be handed to scans that chose another, and a metered
//! proxy would be billed megabytes that do not depend on where they are
//! fetched from.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use adscan_model::{CaptureProxy, SellersJsonCacheInfo};
use tokio_util::sync::CancellationToken;

use super::AppError;
use super::ports::{
    CachedSellersJson, Clock, FetchError, FetchProxy, PublicFile, PublicFileFetcher,
    PublicFileRequest, SellersJsonCache,
};
use crate::domain::diagnostics::identity::{BrowserIdentity, SELLERS_JSON_ACCEPT};
use crate::domain::diagnostics::sellers_cache::{
    MAX_FILE_BYTES, Standing, is_passing_trouble, standing,
};
use crate::domain::diagnostics::sellers_json::{check_readable, is_web_page};
use crate::domain::diagnostics::site::sellers_json_url;
use crate::domain::diagnostics::{CopyStoodIn, refusal};

const TIMEOUT: Duration = Duration::from_secs(20);

/// Where a file that was handed out came from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Origin {
    /// Downloaded for this request.
    Downloaded,
    /// The kept copy; the exchange confirmed just now that it is current.
    Confirmed,
    /// The kept copy of `fetched_at` (epoch ms), recent enough to be used
    /// without asking.
    Kept { fetched_at: f64 },
    /// The kept copy of `fetched_at`, used in place of what the exchange
    /// gave this request.
    KeptInstead {
        fetched_at: f64,
        because: CopyStoodIn,
    },
}

/// Why the lookup failed when the exchange answered 2xx with a web page:
/// a firewall's block page, a challenge, a redirect to the home page or a
/// "not found" page served as found. Which one it is cannot be told.
const WEB_PAGE: &str = "the site answered with a web page instead of the file";

#[derive(Debug, Clone, PartialEq)]
pub struct SellersJsonFile {
    /// The file, or why there is none to read.
    pub content: Result<Vec<u8>, String>,
    pub origin: Origin,
}

/// `file` if it is a sellers.json, or why it is not. Megabytes of JSON:
/// parsed off the async workers.
async fn readable(file: Vec<u8>) -> Result<Vec<u8>, String> {
    tokio::task::spawn_blocking(move || check_readable(&file).map(|()| file))
        .await
        .unwrap_or_else(|error| Err(format!("the file could not be parsed ({error})")))
}

/// The scan that asked was cancelled while it waited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

pub struct SellersJsonSource {
    files: Arc<dyn PublicFileFetcher>,
    cache: Arc<dyn SellersJsonCache>,
    clock: Arc<dyn Clock>,
    /// One lookup per exchange at a time: a second scan that needs the same
    /// file waits for the first and then finds it kept. Only exchanges of
    /// the adapter table are ever asked, so the map stays small.
    turns: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl SellersJsonSource {
    pub fn new(
        files: Arc<dyn PublicFileFetcher>,
        cache: Arc<dyn SellersJsonCache>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            files,
            cache,
            clock,
            turns: Mutex::default(),
        }
    }

    /// The sellers.json of the exchange with this ad system domain, asked
    /// for as `identity` would ask.
    pub async fn read(
        &self,
        domain: &str,
        identity: &BrowserIdentity,
        cancel: &CancellationToken,
    ) -> Result<SellersJsonFile, Cancelled> {
        let Some(url) = sellers_json_url(domain) else {
            return Ok(SellersJsonFile {
                content: Err("it is not a public domain name".into()),
                origin: Origin::Downloaded,
            });
        };
        let turn = Arc::clone(
            self.turns
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(domain.to_string())
                .or_default(),
        );
        let _turn = tokio::select! {
            turn = turn.lock() => turn,
            () = cancel.cancelled() => return Err(Cancelled),
        };

        let kept = match self.cache.load(domain).await {
            // A kept file is as little trusted as a download. One that is
            // not a sellers.json (kept before files were checked) counts as
            // no entry: the exchange is asked, and its answer replaces it.
            Some(kept) => match kept.content {
                Ok(file) => readable(file).await.ok().map(|file| CachedSellersJson {
                    content: Ok(file),
                    ..kept
                }),
                Err(_) => Some(kept),
            },
            None => None,
        };
        let kept_standing = kept
            .as_ref()
            .map(|kept| standing(kept.content.is_ok(), kept.fetched_at, self.clock.now_ms()));
        if let (Some(kept), Some(Standing::Fresh)) = (&kept, kept_standing) {
            return Ok(SellersJsonFile {
                content: kept.content.clone(),
                origin: Origin::Kept {
                    fetched_at: kept.fetched_at,
                },
            });
        }
        // Only a kept file has a version worth asking about.
        let kept_file = kept.and_then(|kept| match kept.content {
            Ok(file) => Some((file, kept.fetched_at, kept.version)),
            Err(_) => None,
        });
        let request = PublicFileRequest {
            url,
            max_bytes: MAX_FILE_BYTES,
            timeout: TIMEOUT,
            loopback_origin: None,
            proxy: FetchProxy {
                route: CaptureProxy::System,
                password: None,
            },
            known_version: kept_file
                .as_ref()
                .map(|(_, _, version)| version.clone())
                .filter(|version| version.is_stated()),
            identity: identity.clone(),
            accept: SELLERS_JSON_ACCEPT,
        };
        let answer = match self.files.fetch(&request, cancel).await {
            // Nothing is kept of a lookup that was called off.
            Err(FetchError::Cancelled) => return Err(Cancelled),
            answer => answer,
        };
        let now = self.clock.now_ms();
        // Why there is no file, what may stand in for it, and whether the
        // answer is the exchange's to every scan or only to this one's.
        let (reason, trouble, for_every_scan, kept_file) = match (answer, kept_file) {
            (Ok(answer), Some((file, _, _))) if answer.status == 304 => {
                // Should the date not be written, the copy is asked about
                // again next time; it is current all the same.
                let _ = self.cache.confirm(domain, now).await;
                return Ok(SellersJsonFile {
                    content: Ok(file),
                    origin: Origin::Confirmed,
                });
            }
            (Ok(answer), kept_file) if (200..300).contains(&answer.status) => {
                // A valid file is a file whatever type it was announced as;
                // that it looks like a web page only names a failure.
                let web_page = is_web_page(answer.content_type.as_deref(), &answer.body);
                match readable(answer.body).await {
                    Ok(body) => {
                        let file = PublicFile { body, ..answer };
                        return Ok(self.keep(domain, file, now).await);
                    }
                    // A web page, a soft 404 or half a file says nothing
                    // about the sellers. Like an exchange that cannot be
                    // reached, it does not replace a copy that still serves.
                    Err(reason) => (
                        if web_page {
                            WEB_PAGE.to_string()
                        } else {
                            reason
                        },
                        Some(CopyStoodIn::AnswerNotSellersJson),
                        true,
                        kept_file,
                    ),
                }
            }
            // A refusal may be of this scan's browser only (a firewall that
            // turns some user agents away): it neither replaces a copy that
            // still serves nor is it handed to other scans as their answer.
            (Ok(answer), kept_file) if matches!(answer.status, 401 | 403) => (
                refusal(answer.status),
                Some(CopyStoodIn::AnswerNotSellersJson),
                false,
                kept_file,
            ),
            (Ok(answer), kept_file) => (
                format!("the exchange answered HTTP {}", answer.status),
                is_passing_trouble(answer.status).then_some(CopyStoodIn::ExchangeUnreachable),
                true,
                kept_file,
            ),
            (Err(error), kept_file) => (
                error.to_string(),
                Some(CopyStoodIn::ExchangeUnreachable),
                true,
                kept_file,
            ),
        };
        if let (Some((file, fetched_at, _)), Some(Standing::AskAgainOrUse), Some(because)) =
            (kept_file, kept_standing, trouble)
        {
            return Ok(SellersJsonFile {
                content: Ok(file),
                origin: Origin::KeptInstead {
                    fetched_at,
                    because,
                },
            });
        }
        if for_every_scan {
            let unavailable = CachedSellersJson {
                fetched_at: now,
                final_url: request.url,
                version: Default::default(),
                content: Err(reason.clone()),
            };
            // A cache that cannot be written costs the next scan a request.
            let _ = self.cache.store(domain, unavailable).await;
        }
        Ok(SellersJsonFile {
            content: Err(reason),
            origin: Origin::Downloaded,
        })
    }

    async fn keep(&self, domain: &str, file: PublicFile, now: f64) -> SellersJsonFile {
        let entry = CachedSellersJson {
            fetched_at: now,
            final_url: file.final_url,
            version: file.version,
            content: Ok(file.body.clone()),
        };
        let _ = self.cache.store(domain, entry).await;
        SellersJsonFile {
            content: Ok(file.body),
            origin: Origin::Downloaded,
        }
    }

    pub async fn cache_info(&self) -> SellersJsonCacheInfo {
        self.cache.info().await
    }

    pub async fn clear_cache(&self) -> Result<SellersJsonCacheInfo, AppError> {
        self.cache.clear().await.map_err(AppError::Storage)?;
        Ok(self.cache.info().await)
    }

    /// Removes what a write that was cut short left in the cache.
    pub async fn sweep_cache(&self) {
        self.cache.sweep().await;
    }
}
