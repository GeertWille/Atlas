//! Which request on the network carried which of Prebid's bids, and whether
//! its answer came in time.
//!
//! Prebid announces every bidder request (`bidRequested`) with ids of its
//! own. An adapter puts some of them into what it sends, so a request that
//! contains one is that bidder request for certain ("exact"). Where none is
//! found, the bidder's request that started right after the announcement is
//! taken, and labelled as matched by timing.

use std::collections::{BTreeMap, HashSet};

use adscan_model::{
    AdLink, AdSlot, Auction, BidExchange, BidStatus, ExchangeVerdict, TrackedBidRequest,
    TrackerEvent, TrackerEventKind, request_id,
};

/// The tracker's clock and the network's clock can disagree by a few
/// milliseconds; an answer this close to the auction's close is not called
/// early or late. A starting value, not calibrated against real pages.
pub const VERDICT_TOLERANCE_MS: f64 = 10.0;
/// How long before and after its announcement a bidder's request may start
/// to be taken for it without an id. Starting values, not calibrated.
const TIMING_MATCH_BEFORE_MS: f64 = 30.0;
const TIMING_MATCH_AFTER_MS: f64 = 300.0;
/// Shorter ids ("1", "top") turn up in unrelated requests.
const MIN_ID_CHARS: usize = 8;
/// How long before and after the tag reported a slot request the ad-server
/// call that carries it may start. Starting values, not calibrated.
const SLOT_MATCH_BEFORE_MS: f64 = 50.0;
const SLOT_MATCH_AFTER_MS: f64 = 1_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Answered,
    Failed,
    /// Nothing came back before the visit ended.
    Pending,
}

/// A bid request as recorded on the network.
#[derive(Debug, Clone, PartialEq)]
pub struct ExchangeRequest {
    pub seq: u32,
    pub page: u32,
    /// Bidder code of the endpoint, when it is a known adapter's.
    pub bidder: Option<String>,
    /// A Prebid Server request, which carries several bidders' bids.
    pub via_server: bool,
    /// ms since the start of page view `page`.
    pub started_ms: f64,
    pub ended_ms: Option<f64>,
    pub answer: Answer,
    /// URL and request body: where the ids are looked for.
    pub haystack: String,
    /// `haystack` is all of the request. False when the body could not be
    /// read or was longer than what is searched: an id that is not found
    /// may then have been sent all the same.
    pub complete: bool,
    /// From the decoded answer: its highest price and the currency it
    /// states for its prices.
    pub best_cpm: Option<f64>,
    pub currency: Option<String>,
    pub nbr: Option<u32>,
}

/// An ad server's ad call as recorded on the network.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdServerCall {
    pub seq: u32,
    pub page: u32,
    /// ms since the start of page view `page`.
    pub started_ms: f64,
}

/// Ties each ad request of a slot to the ad-server call that started
/// nearest to it on the same page view. One call can carry several slots
/// (single-request mode), so calls are shared.
pub fn link_slots(slots: &mut [AdSlot], calls: &[AdServerCall]) {
    for slot in slots {
        let page = slot.page;
        for request in &mut slot.requests {
            request.request_id = calls
                .iter()
                .filter(|call| call.page == page)
                .map(|call| (call, call.started_ms - request.at_ms))
                .filter(|(_, after)| (-SLOT_MATCH_BEFORE_MS..=SLOT_MATCH_AFTER_MS).contains(after))
                .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                .map(|(call, _)| request_id(call.seq));
        }
    }
}

/// For a scan that stored no network data: the verdicts stand, but there is
/// no request row to point at.
pub fn forget_requests(auctions: &mut [Auction]) {
    let exchanges = auctions
        .iter_mut()
        .flat_map(|auction| &mut auction.ad_units)
        .flat_map(|unit| &mut unit.bids)
        .filter_map(|bid| bid.exchange.as_mut());
    for exchange in exchanges {
        exchange.request_id = None;
    }
}

/// One `bidRequested` of the tracker.
#[derive(Debug, Clone, PartialEq)]
pub struct Announcement {
    pub auction_id: String,
    pub page: u32,
    pub bidder: String,
    /// ms since the start of page view `page`.
    pub at_ms: f64,
    pub bidder_request_id: Option<String>,
    pub bids: Vec<TrackedBidRequest>,
    pub ad_unit_codes: Vec<String>,
}

pub fn announcements(events: &[TrackerEvent]) -> Vec<Announcement> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            TrackerEventKind::BidRequested {
                auction_id,
                bidder,
                ad_unit_codes,
                bidder_request_id,
                bids,
                ..
            } => Some(Announcement {
                auction_id: auction_id.clone(),
                page: event.page,
                bidder: bidder.clone(),
                at_ms: event.t,
                bidder_request_id: bidder_request_id.clone(),
                bids: bids.clone(),
                ad_unit_codes: ad_unit_codes.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// What linking found out about one request.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestAdLink {
    pub seq: u32,
    pub link: AdLink,
    /// (bid id, ad unit code) for Prebid's bid ids found in the request.
    pub bid_units: Vec<(String, String)>,
}

struct Match<'a> {
    request: &'a ExchangeRequest,
    exact: bool,
}

/// Sets `Bid.exchange` on the bids of `auctions` and returns the link of
/// every request that was matched to an auction. `page_starts[page]` is the
/// start of that page view in ms since the scan's navigation start.
pub fn link(
    auctions: &mut [Auction],
    announcements: &[Announcement],
    requests: &[ExchangeRequest],
    page_starts: &[f64],
) -> Vec<RequestAdLink> {
    let absolute =
        |page: u32, at_ms: f64| page_starts.get(page as usize).copied().unwrap_or(0.0) + at_ms;
    let visible_bidders: HashSet<&str> = requests
        .iter()
        .filter_map(|request| request.bidder.as_deref())
        .collect();

    // Exact matches first, so that a request with an id in it is never
    // given away to another announcement by timing.
    let mut matched: Vec<Vec<Match>> = announcements
        .iter()
        .map(|announcement| exact_matches(announcement, requests))
        .collect();
    let named: HashSet<u32> = matched
        .iter()
        .flatten()
        .map(|found| found.request.seq)
        .collect();
    // Without an id, a request is taken for an announcement only when the
    // two can belong to nobody else: it is the one request of that bidder
    // that started around the announcement, and no other announcement
    // without ids could claim it. A bidder that sends one request per ad
    // unit, or two auctions at the same moment, stay unmatched: a wrong
    // guess here would put one request's timing on another's bid.
    let started_around = |announcement: &Announcement, request: &ExchangeRequest| {
        let after = absolute(request.page, request.started_ms)
            - absolute(announcement.page, announcement.at_ms);
        (-TIMING_MATCH_BEFORE_MS..=TIMING_MATCH_AFTER_MS).contains(&after)
    };
    let candidates: Vec<Vec<&ExchangeRequest>> = announcements
        .iter()
        .zip(&matched)
        .map(|(announcement, matches)| {
            if !matches.is_empty() {
                return Vec::new();
            }
            requests
                .iter()
                .filter(|request| {
                    request.bidder.as_deref() == Some(announcement.bidder.as_str())
                        && !named.contains(&request.seq)
                        && started_around(announcement, request)
                })
                .collect()
        })
        .collect();
    for (index, matches) in matched.iter_mut().enumerate() {
        let [only] = candidates[index].as_slice() else {
            continue;
        };
        let contested = candidates
            .iter()
            .enumerate()
            .any(|(other, theirs)| other != index && theirs.iter().any(|r| r.seq == only.seq));
        if !contested {
            matches.push(Match {
                request: only,
                exact: false,
            });
        }
    }

    let mut links: BTreeMap<u32, RequestAdLink> = BTreeMap::new();
    for (announcement, matches) in announcements.iter().zip(&matched) {
        let Some(auction) = auctions
            .iter_mut()
            .rev()
            .find(|auction| auction.id == announcement.auction_id)
        else {
            continue;
        };
        // An auction closes at its timeout; when all bidders answered sooner
        // it ended early and nobody was late.
        let deadline = auction
            .timeout_ms
            .map(|timeout| auction.started_at_ms + timeout)
            .or(auction.ended_at_ms)
            .map(|at| absolute(auction.page, at));
        let ids_are_visible = matches.iter().any(|found| {
            announcement
                .bids
                .iter()
                .any(|bid| contains_id(&found.request.haystack, &bid.bid_id))
        });
        for code in &announcement.ad_unit_codes {
            let bid_id = announcement
                .bids
                .iter()
                .find(|bid| &bid.ad_unit_code == code)
                .map(|bid| bid.bid_id.as_str());
            let carrier = match bid_id {
                Some(id) if ids_are_visible => matches
                    .iter()
                    .find(|found| contains_id(&found.request.haystack, id)),
                // One request and no bid ids in it: it carried the whole
                // bidder request. Several, and nothing says which carried
                // which ad unit.
                _ if matches.len() == 1 => matches.first(),
                _ if matches.is_empty() => None,
                _ => continue,
            };
            let Some(bid) = auction
                .ad_units
                .iter_mut()
                .find(|unit| &unit.code == code)
                .and_then(|unit| {
                    unit.bids
                        .iter_mut()
                        .find(|bid| bid.bidder == announcement.bidder)
                })
            else {
                continue;
            };
            let Some(found) = carrier else {
                // "Not sent" is an accusation, and is only made where the
                // bidder's requests can be seen and nothing is left that
                // could have been this one: either the bidder's request is
                // there, whole, with the other ad units' bid ids in it, or
                // the capture knows this bidder's endpoint and no request
                // that might be the missing one started since the
                // announcement.
                let left_out =
                    ids_are_visible && matches.iter().all(|found| found.request.complete);
                let announced_at = absolute(announcement.page, announcement.at_ms);
                let could_be_it = |request: &&ExchangeRequest| {
                    let carries_this_bidder = request.via_server
                        || request.bidder.is_none()
                        || request.bidder.as_deref() == Some(announcement.bidder.as_str());
                    carries_this_bidder
                        && !named.contains(&request.seq)
                        && request.page == announcement.page
                        && absolute(request.page, request.started_ms)
                            >= announced_at - TIMING_MATCH_BEFORE_MS
                };
                let never_made = matches.is_empty()
                    && visible_bidders.contains(announcement.bidder.as_str())
                    && !requests.iter().any(|request| could_be_it(&request));
                if left_out || never_made {
                    bid.exchange = Some(BidExchange {
                        request_id: None,
                        verdict: ExchangeVerdict::NotSent,
                        late_by_ms: None,
                        network_ms: None,
                        exact: false,
                    });
                }
                continue;
            };
            let request = found.request;
            let ended = request.ended_ms.map(|at| absolute(request.page, at));
            let (verdict, late_by_ms) = verdict(
                request.answer,
                ended,
                deadline,
                bid.status == BidStatus::Timeout,
            );
            if !found.exact && contradicts(verdict, bid.status) {
                // Prebid has this bidder's answer, and the request guessed
                // by timing has none in time: the guess is wrong.
                continue;
            }
            bid.exchange = Some(BidExchange {
                request_id: Some(request_id(request.seq)),
                verdict,
                late_by_ms,
                network_ms: request.ended_ms.map(|ended| ended - request.started_ms),
                exact: found.exact,
            });
            let entry = links.entry(request.seq).or_insert_with(|| RequestAdLink {
                seq: request.seq,
                link: AdLink {
                    auction_id: auction.id.clone(),
                    ad_units: Vec::new(),
                    via_server: request.via_server,
                    exact: found.exact,
                    verdict,
                    late_by_ms,
                    cpm: request.best_cpm,
                    currency: request.best_cpm.and(request.currency.clone()),
                    nbr: request.nbr,
                },
                bid_units: Vec::new(),
            });
            if let Some(id) = bid_id.filter(|id| contains_id(&request.haystack, id)) {
                entry.bid_units.push((id.to_string(), code.clone()));
            }
            let link = &mut entry.link;
            if !link.ad_units.contains(code) {
                link.ad_units.push(code.clone());
            }
            // One request, several bids: it is "in time but timed out" as
            // soon as Prebid timed out any of them.
            if verdict == ExchangeVerdict::InTimeButTimedOut {
                link.verdict = verdict;
            }
            // Without a decoded answer the best of Prebid's own prices
            // stands in, as long as they are in one currency: a price in
            // another one is not compared with it.
            if request.best_cpm.is_none()
                && let Some(cpm) = bid.cpm
            {
                match link.cpm {
                    None => {
                        link.cpm = Some(cpm);
                        link.currency = bid.currency.clone();
                    }
                    Some(best) if link.currency == bid.currency => link.cpm = Some(best.max(cpm)),
                    Some(_) => {}
                }
            }
        }
    }
    links.into_values().collect()
}

fn exact_matches<'a>(
    announcement: &Announcement,
    requests: &'a [ExchangeRequest],
) -> Vec<Match<'a>> {
    let own_ids: Vec<&str> = announcement
        .bidder_request_id
        .iter()
        .map(String::as_str)
        .chain(announcement.bids.iter().map(|bid| bid.bid_id.as_str()))
        .collect();
    requests
        .iter()
        .filter(|request| {
            let same_bidder = request.bidder.as_deref() == Some(announcement.bidder.as_str());
            let holds_own_id = own_ids.iter().any(|id| contains_id(&request.haystack, id));
            // The auction id is shared by every bidder of the auction, so it
            // only identifies this bidder's request on its own endpoint, or a
            // Prebid Server request that carries everyone's bids.
            let holds_auction_id = contains_id(&request.haystack, &announcement.auction_id);
            let may_carry = same_bidder || request.via_server || request.bidder.is_none();
            (holds_own_id && may_carry) || (holds_auction_id && (same_bidder || request.via_server))
        })
        .map(|request| Match {
            request,
            exact: true,
        })
        .collect()
}

/// Whether what the network says of a request cannot be true of a bid that
/// Prebid recorded as answered.
fn contradicts(verdict: ExchangeVerdict, status: BidStatus) -> bool {
    let answered_in_prebid = matches!(
        status,
        BidStatus::Bid | BidStatus::Won | BidStatus::Rendered | BidStatus::NoBid
    );
    answered_in_prebid
        && matches!(
            verdict,
            ExchangeVerdict::LateOnNetwork
                | ExchangeVerdict::NeverAnswered
                | ExchangeVerdict::Failed
        )
}

fn contains_id(haystack: &str, id: &str) -> bool {
    id.len() >= MIN_ID_CHARS && haystack.contains(id)
}

/// The verdict and, where it means something, how many ms after the
/// auction's close the answer was complete (negative when before).
pub fn verdict(
    answer: Answer,
    ended_ms: Option<f64>,
    deadline_ms: Option<f64>,
    timed_out_in_prebid: bool,
) -> (ExchangeVerdict, Option<f64>) {
    let ended_ms = match (answer, ended_ms) {
        (Answer::Failed, _) => return (ExchangeVerdict::Failed, None),
        (Answer::Pending, _) | (Answer::Answered, None) => {
            return (ExchangeVerdict::NeverAnswered, None);
        }
        (Answer::Answered, Some(ended_ms)) => ended_ms,
    };
    let Some(deadline_ms) = deadline_ms else {
        return (ExchangeVerdict::InTime, None);
    };
    let late_by = ended_ms - deadline_ms;
    if late_by.abs() <= VERDICT_TOLERANCE_MS {
        (ExchangeVerdict::AtDeadline, None)
    } else if late_by > 0.0 {
        (ExchangeVerdict::LateOnNetwork, Some(late_by))
    } else if timed_out_in_prebid {
        (ExchangeVerdict::InTimeButTimedOut, Some(late_by))
    } else {
        (ExchangeVerdict::InTime, Some(late_by))
    }
}

#[cfg(test)]
mod tests {
    use adscan_model::{AuctionAdUnit, Bid};

    use super::*;

    fn bid(bidder: &str, status: BidStatus, cpm: Option<f64>) -> Bid {
        Bid {
            bidder: bidder.into(),
            status,
            cpm,
            currency: None,
            size: None,
            requested_at_ms: Some(1_000.0),
            response_time_ms: None,
            deal_id: None,
            exchange: None,
        }
    }

    fn auction(id: &str, units: Vec<(&str, Vec<Bid>)>) -> Auction {
        Auction {
            id: id.into(),
            page: 0,
            started_at_ms: 1_000.0,
            ended_at_ms: Some(1_250.0),
            duration_ms: Some(250.0),
            timeout_ms: Some(250.0),
            ad_units: units
                .into_iter()
                .map(|(code, bids)| AuctionAdUnit {
                    code: code.into(),
                    sizes: Vec::new(),
                    bids,
                    winning_bidder: None,
                    winning_cpm: None,
                })
                .collect(),
        }
    }

    fn announced(auction: &str, bidder: &str, units: &[(&str, &str)]) -> Announcement {
        Announcement {
            auction_id: auction.into(),
            page: 0,
            bidder: bidder.into(),
            at_ms: 1_001.0,
            bidder_request_id: Some(format!("breq-{bidder}-{auction}")),
            bids: units
                .iter()
                .map(|(code, bid_id)| TrackedBidRequest {
                    bid_id: (*bid_id).into(),
                    ad_unit_code: (*code).into(),
                })
                .collect(),
            ad_unit_codes: units.iter().map(|(code, _)| (*code).to_string()).collect(),
        }
    }

    fn request(
        seq: u32,
        bidder: Option<&str>,
        started: f64,
        ended: Option<f64>,
        body: &str,
    ) -> ExchangeRequest {
        ExchangeRequest {
            seq,
            page: 0,
            bidder: bidder.map(str::to_string),
            via_server: false,
            started_ms: started,
            ended_ms: ended,
            answer: if ended.is_some() {
                Answer::Answered
            } else {
                Answer::Pending
            },
            haystack: body.into(),
            complete: true,
            best_cpm: None,
            currency: None,
            nbr: None,
        }
    }

    fn exchange_of<'a>(
        auctions: &'a [Auction],
        unit: &str,
        bidder: &str,
    ) -> Option<&'a BidExchange> {
        auctions[0]
            .ad_units
            .iter()
            .find(|candidate| candidate.code == unit)?
            .bids
            .iter()
            .find(|bid| bid.bidder == bidder)?
            .exchange
            .as_ref()
    }

    #[test]
    fn a_request_that_holds_a_bid_id_is_that_bid_for_certain() {
        let mut auctions = vec![auction(
            "auction-0001",
            vec![("top", vec![bid("openx", BidStatus::NoBid, None)])],
        )];
        let announcements = [announced(
            "auction-0001",
            "openx",
            &[("top", "bid-openx-top")],
        )];
        let requests = [
            // Started closer in time, but it is somebody else's.
            request(
                1,
                Some("openx"),
                1_002.0,
                Some(1_100.0),
                r#"{"imp":[{"id":"bid-other-x"}]}"#,
            ),
            ExchangeRequest {
                nbr: Some(7),
                ..request(
                    2,
                    Some("openx"),
                    1_040.0,
                    Some(1_212.0),
                    r#"{"imp":[{"id":"bid-openx-top"}]}"#,
                )
            },
        ];
        let links = link(&mut auctions, &announcements, &requests, &[0.0]);
        let exchange = exchange_of(&auctions, "top", "openx").unwrap();
        assert_eq!(exchange.request_id.as_deref(), Some("r2"));
        assert!(exchange.exact);
        assert_eq!(exchange.verdict, ExchangeVerdict::InTime);
        assert_eq!(exchange.late_by_ms, Some(-38.0));
        assert_eq!(exchange.network_ms, Some(172.0));
        assert_eq!(
            links,
            vec![RequestAdLink {
                seq: 2,
                link: AdLink {
                    auction_id: "auction-0001".into(),
                    ad_units: vec!["top".into()],
                    via_server: false,
                    exact: true,
                    verdict: ExchangeVerdict::InTime,
                    late_by_ms: Some(-38.0),
                    cpm: None,
                    currency: None,
                    nbr: Some(7),
                },
                bid_units: vec![("bid-openx-top".into(), "top".into())],
            }]
        );
    }

    #[test]
    fn without_an_id_the_bidders_request_right_after_the_announcement_is_taken_and_labelled() {
        let mut auctions = vec![auction(
            "auction-0001",
            vec![("top", vec![bid("rubicon", BidStatus::Timeout, None)])],
        )];
        let announcements = [announced(
            "auction-0001",
            "rubicon",
            &[("top", "bid-rubicon-top")],
        )];
        let requests = [
            request(4, Some("rubicon"), 400.0, Some(500.0), "account_id=1"),
            request(5, Some("rubicon"), 1_010.0, Some(1_334.0), "account_id=1"),
            request(6, Some("pubmatic"), 1_005.0, Some(1_100.0), "x"),
        ];
        let links = link(&mut auctions, &announcements, &requests, &[0.0]);
        let exchange = exchange_of(&auctions, "top", "rubicon").unwrap();
        assert_eq!(exchange.request_id.as_deref(), Some("r5"));
        assert!(!exchange.exact, "matched by timing");
        assert_eq!(exchange.verdict, ExchangeVerdict::LateOnNetwork);
        assert_eq!(exchange.late_by_ms, Some(84.0));
        assert_eq!(links.len(), 1);
        assert!(!links[0].link.exact);
        assert!(links[0].bid_units.is_empty());
    }

    #[test]
    fn a_bid_whose_id_is_not_in_the_bidders_request_was_not_sent() {
        let mut auctions = vec![auction(
            "auction-0001",
            vec![
                ("top", vec![bid("triplelift", BidStatus::Bid, Some(0.4))]),
                ("sidebar", vec![bid("triplelift", BidStatus::NoBid, None)]),
                ("anchor", vec![bid("triplelift", BidStatus::NoBid, None)]),
            ],
        )];
        let announcements = [announced(
            "auction-0001",
            "triplelift",
            &[
                ("top", "bid-tl-top"),
                ("sidebar", "bid-tl-side"),
                ("anchor", "bid-tl-anchor"),
            ],
        )];
        let requests = [request(
            9,
            Some("triplelift"),
            1_003.0,
            Some(1_120.0),
            r#"{"imp":[{"id":"bid-tl-top"}]}"#,
        )];
        let links = link(&mut auctions, &announcements, &requests, &[0.0]);
        assert_eq!(
            exchange_of(&auctions, "top", "triplelift")
                .unwrap()
                .request_id
                .as_deref(),
            Some("r9")
        );
        for unit in ["sidebar", "anchor"] {
            let exchange = exchange_of(&auctions, unit, "triplelift").unwrap();
            assert_eq!(exchange.verdict, ExchangeVerdict::NotSent, "{unit}");
            assert_eq!(exchange.request_id, None);
        }
        assert_eq!(links[0].link.ad_units, ["top"]);
        assert_eq!(
            links[0].link.cpm,
            Some(0.4),
            "from Prebid's bid when the answer was not decoded"
        );
    }

    #[test]
    fn a_bidder_whose_endpoint_is_known_but_made_no_request_was_not_sent() {
        let mut auctions = vec![auction(
            "auction-0002",
            vec![("top", vec![bid("ix", BidStatus::NoBid, None)])],
        )];
        let announcements = [announced("auction-0002", "ix", &[("top", "bid-ix-top")])];
        // The same bidder was seen on the network, in an earlier auction.
        let requests = [request(1, Some("ix"), 100.0, Some(150.0), "s=1")];
        link(&mut auctions, &announcements, &requests, &[0.0]);
        assert_eq!(
            exchange_of(&auctions, "top", "ix").unwrap().verdict,
            ExchangeVerdict::NotSent
        );
    }

    #[test]
    fn nothing_is_claimed_about_a_bidder_whose_requests_cannot_be_recognised() {
        let mut auctions = vec![auction(
            "auction-0001",
            vec![("top", vec![bid("obscure", BidStatus::Bid, Some(1.0))])],
        )];
        let announcements = [announced(
            "auction-0001",
            "obscure",
            &[("top", "bid-obscure")],
        )];
        let links = link(&mut auctions, &announcements, &[], &[0.0]);
        assert_eq!(exchange_of(&auctions, "top", "obscure"), None);
        assert!(links.is_empty());
    }

    #[test]
    fn a_prebid_server_request_carries_the_bids_of_every_bidder_it_was_sent_for() {
        let mut auctions = vec![auction(
            "auction-0001",
            vec![(
                "top",
                vec![
                    bid("appnexus", BidStatus::Bid, Some(0.88)),
                    bid("sovrn", BidStatus::Timeout, None),
                ],
            )],
        )];
        let announcements = [
            announced("auction-0001", "appnexus", &[("top", "bid-apn-top")]),
            announced("auction-0001", "sovrn", &[("top", "bid-sovrn-top")]),
        ];
        let requests = [ExchangeRequest {
            via_server: true,
            best_cpm: Some(0.88),
            ..request(
                3,
                None,
                1_004.0,
                Some(1_200.0),
                r#"{"id":"auction-0001","imp":[{"id":"top"}]}"#,
            )
        }];
        let links = link(&mut auctions, &announcements, &requests, &[0.0]);
        for bidder in ["appnexus", "sovrn"] {
            let exchange = exchange_of(&auctions, "top", bidder).unwrap();
            assert_eq!(exchange.request_id.as_deref(), Some("r3"), "{bidder}");
            assert!(exchange.exact);
        }
        assert_eq!(
            exchange_of(&auctions, "top", "sovrn").unwrap().verdict,
            ExchangeVerdict::InTimeButTimedOut
        );
        assert_eq!(links.len(), 1);
        let link = &links[0].link;
        assert!(link.via_server);
        assert_eq!(link.verdict, ExchangeVerdict::InTimeButTimedOut);
        assert_eq!(link.cpm, Some(0.88));
    }

    #[test]
    fn the_price_of_a_request_names_its_currency_and_is_never_the_best_of_two_currencies() {
        let priced = |cpm: f64, currency: &str| Bid {
            currency: Some(currency.into()),
            ..bid("gumgum", BidStatus::Bid, Some(cpm))
        };
        let announcements = [announced(
            "auction-0001",
            "gumgum",
            &[("top", "bid-gg-top"), ("side", "bid-gg-side")],
        )];
        let sent = r#"{"imp":[{"id":"bid-gg-top"},{"id":"bid-gg-side"}]}"#;

        // The answer was decoded: its price and its currency stand.
        let mut auctions = vec![auction(
            "auction-0001",
            vec![("top", vec![priced(10.01, "EUR")])],
        )];
        let decoded = [ExchangeRequest {
            best_cpm: Some(10.01),
            currency: Some("EUR".into()),
            ..request(1, Some("gumgum"), 1_004.0, Some(1_200.0), sent)
        }];
        let links = link(&mut auctions, &announcements[..1], &decoded, &[0.0]);
        assert_eq!(
            (links[0].link.cpm, links[0].link.currency.as_deref()),
            (Some(10.01), Some("EUR"))
        );

        // Not decoded: Prebid's own prices stand in. 2.05 USD is not "less
        // than" 10.01 EUR; the first price keeps its place and its currency.
        let undecoded = [request(1, Some("gumgum"), 1_004.0, Some(1_200.0), sent)];
        let mut auctions = vec![auction(
            "auction-0001",
            vec![
                ("top", vec![priced(2.05, "USD")]),
                ("side", vec![priced(10.01, "EUR")]),
            ],
        )];
        let links = link(&mut auctions, &announcements, &undecoded, &[0.0]);
        assert_eq!(
            (links[0].link.cpm, links[0].link.currency.as_deref()),
            (Some(2.05), Some("USD"))
        );
        // In one currency the higher price is the request's.
        let mut auctions = vec![auction(
            "auction-0001",
            vec![
                ("top", vec![priced(2.05, "EUR")]),
                ("side", vec![priced(10.01, "EUR")]),
            ],
        )];
        let links = link(&mut auctions, &announcements, &undecoded, &[0.0]);
        assert_eq!(
            (links[0].link.cpm, links[0].link.currency.as_deref()),
            (Some(10.01), Some("EUR"))
        );
    }

    #[test]
    fn a_request_with_an_id_in_it_is_not_given_to_another_auction_by_timing() {
        let mut auctions = vec![
            auction(
                "auction-0001",
                vec![("top", vec![bid("openx", BidStatus::NoBid, None)])],
            ),
            auction(
                "auction-0002",
                vec![("top", vec![bid("openx", BidStatus::NoBid, None)])],
            ),
        ];
        let announcements = [
            Announcement {
                bids: Vec::new(),
                bidder_request_id: None,
                ..announced("auction-0001", "openx", &[("top", "unused-id-1")])
            },
            announced("auction-0002", "openx", &[("top", "bid-second-top")]),
        ];
        let requests = [request(
            7,
            Some("openx"),
            1_002.0,
            Some(1_100.0),
            "bid-second-top",
        )];
        link(&mut auctions, &announcements, &requests, &[0.0]);
        assert_eq!(
            auctions[1].ad_units[0].bids[0]
                .exchange
                .as_ref()
                .unwrap()
                .request_id
                .as_deref(),
            Some("r7")
        );
        assert_eq!(
            auctions[0].ad_units[0].bids[0]
                .exchange
                .as_ref()
                .unwrap()
                .verdict,
            ExchangeVerdict::NotSent
        );
    }

    #[test]
    fn times_on_different_page_views_are_compared_on_one_clock() {
        let mut auctions = vec![Auction {
            page: 1,
            ..auction(
                "auction-0001",
                vec![("top", vec![bid("openx", BidStatus::Timeout, None)])],
            )
        }];
        let announcements = [Announcement {
            page: 1,
            ..announced("auction-0001", "openx", &[("top", "bid-openx-top")])
        }];
        // Page view 1 started 5 s into the visit; the request is recorded
        // on page view 1 as well.
        let requests = [ExchangeRequest {
            page: 1,
            ..request(2, Some("openx"), 1_010.0, Some(1_400.0), "no ids")
        }];
        link(&mut auctions, &announcements, &requests, &[0.0, 5_000.0]);
        let exchange = auctions[0].ad_units[0].bids[0].exchange.as_ref().unwrap();
        assert_eq!(exchange.verdict, ExchangeVerdict::LateOnNetwork);
        assert_eq!(exchange.late_by_ms, Some(150.0));
    }

    #[test]
    fn verdicts_follow_the_answer_and_the_auctions_close() {
        use ExchangeVerdict::*;
        let deadline = Some(1_250.0);
        #[rustfmt::skip]
        let table = [
            (Answer::Answered, Some(1_212.0), deadline, false, InTime, Some(-38.0)),
            (Answer::Answered, Some(1_334.0), deadline, false, LateOnNetwork, Some(84.0)),
            (Answer::Answered, Some(1_334.0), deadline, true, LateOnNetwork, Some(84.0)),
            (Answer::Answered, Some(1_210.0), deadline, true, InTimeButTimedOut, Some(-40.0)),
            (Answer::Pending, None, deadline, true, NeverAnswered, None),
            (Answer::Answered, None, deadline, false, NeverAnswered, None),
            (Answer::Failed, Some(1_041.0), deadline, false, Failed, None),
            // Without a known close there is nothing to be late for.
            (Answer::Answered, Some(9_999.0), None, false, InTime, None),
        ];
        for (answer, ended, deadline, timed_out, expected, late_by) in table {
            assert_eq!(
                verdict(answer, ended, deadline, timed_out),
                (expected, late_by),
                "{answer:?} {ended:?} {timed_out}"
            );
        }
    }

    #[test]
    fn inside_the_clock_tolerance_an_answer_is_neither_early_nor_late() {
        let deadline = Some(1_250.0);
        for ended in [1_240.0, 1_245.5, 1_250.0, 1_259.9, 1_260.0] {
            assert_eq!(
                verdict(Answer::Answered, Some(ended), deadline, true),
                (ExchangeVerdict::AtDeadline, None),
                "{ended}"
            );
        }
        assert_eq!(
            verdict(Answer::Answered, Some(1_260.5), deadline, false).0,
            ExchangeVerdict::LateOnNetwork
        );
        assert_eq!(
            verdict(Answer::Answered, Some(1_239.5), deadline, false).0,
            ExchangeVerdict::InTime
        );
    }

    #[test]
    fn a_slot_request_is_tied_to_the_ad_call_that_started_nearest_on_its_page() {
        use adscan_model::{SlotOutcome, SlotRequest};
        let slot = |page: u32, element: &str, at: &[f64]| AdSlot {
            page,
            element_id: element.into(),
            ad_unit_path: None,
            sizes: Vec::new(),
            outcome: SlotOutcome::Filled,
            rendered_size: None,
            defined_at_ms: None,
            requested_at_ms: at.first().copied(),
            response_at_ms: None,
            rendered_at_ms: None,
            viewable_at_ms: None,
            in_initial_viewport: None,
            had_prebid_targeting: None,
            requested_before_auction_end: None,
            advertiser_id: None,
            line_item_id: None,
            requests: at
                .iter()
                .map(|at_ms| SlotRequest {
                    at_ms: *at_ms,
                    had_prebid_targeting: false,
                    prebid_ad_id: None,
                    request_id: None,
                })
                .collect(),
        };
        let mut slots = vec![
            slot(0, "top", &[1_500.0, 9_000.0]),
            slot(0, "sidebar", &[1_501.0]),
            slot(1, "top", &[1_500.0]),
            slot(0, "late", &[20_000.0]),
        ];
        let calls = [
            AdServerCall {
                seq: 10,
                page: 0,
                started_ms: 1_504.0,
            },
            AdServerCall {
                seq: 30,
                page: 0,
                started_ms: 9_020.0,
            },
            AdServerCall {
                seq: 40,
                page: 1,
                started_ms: 1_480.0,
            },
        ];
        link_slots(&mut slots, &calls);
        let ids = |slot: &AdSlot| -> Vec<Option<String>> {
            slot.requests
                .iter()
                .map(|request| request.request_id.clone())
                .collect()
        };
        assert_eq!(
            ids(&slots[0]),
            [Some("r10".to_string()), Some("r30".to_string())]
        );
        assert_eq!(
            ids(&slots[1]),
            [Some("r10".to_string())],
            "one call carries both slots"
        );
        assert_eq!(ids(&slots[2]), [Some("r40".to_string())]);
        assert_eq!(ids(&slots[3]), [None], "no call near it");
    }

    #[test]
    fn without_stored_requests_the_verdicts_stay_and_the_request_ids_go() {
        let mut auctions = vec![auction(
            "auction-0001",
            vec![("top", vec![bid("openx", BidStatus::NoBid, None)])],
        )];
        let announcements = [announced(
            "auction-0001",
            "openx",
            &[("top", "bid-openx-top")],
        )];
        let requests = [request(
            2,
            Some("openx"),
            1_040.0,
            Some(1_334.0),
            "bid-openx-top",
        )];
        link(&mut auctions, &announcements, &requests, &[0.0]);
        forget_requests(&mut auctions);
        let exchange = exchange_of(&auctions, "top", "openx").unwrap();
        assert_eq!(exchange.request_id, None);
        assert_eq!(exchange.verdict, ExchangeVerdict::LateOnNetwork);
        assert_eq!(exchange.late_by_ms, Some(84.0));
    }

    #[test]
    fn short_ids_are_not_searched_for() {
        assert!(!contains_id("imp id top and 1", "top"));
        assert!(contains_id("x bid-openx-top y", "bid-openx-top"));
    }

    #[test]
    fn announcements_are_read_from_the_tracker_events() {
        let events = vec![
            TrackerEvent {
                page: 2,
                t: 77.0,
                kind: TrackerEventKind::BidRequested {
                    auction_id: "a".into(),
                    bidder: "ix".into(),
                    ad_unit_codes: vec!["top".into()],
                    gdpr_applies: None,
                    has_consent_string: None,
                    schain: Vec::new(),
                    bidder_request_id: Some("br".into()),
                    bids: vec![TrackedBidRequest {
                        bid_id: "b1".into(),
                        ad_unit_code: "top".into(),
                    }],
                },
            },
            TrackerEvent {
                page: 2,
                t: 80.0,
                kind: TrackerEventKind::AuctionEnd {
                    auction_id: "a".into(),
                },
            },
        ];
        let found = announcements(&events);
        assert_eq!(found.len(), 1);
        assert_eq!(
            (found[0].page, found[0].at_ms, found[0].bidder.as_str()),
            (2, 77.0, "ix")
        );
        assert_eq!(found[0].bidder_request_id.as_deref(), Some("br"));
        assert_eq!(
            (
                found[0].bids[0].bid_id.as_str(),
                found[0].bids[0].ad_unit_code.as_str()
            ),
            ("b1", "top")
        );
    }

    #[test]
    fn a_bidder_that_sends_one_request_per_ad_unit_without_ids_is_not_guessed_at() {
        // Three requests of one bidder right after its announcement, none
        // with an id: which carried which ad unit cannot be told, and the
        // late one must not be pinned on a bid that was in time.
        let mut auctions = vec![auction(
            "auction-0001",
            vec![
                ("top", vec![bid("rubicon", BidStatus::Bid, Some(1.0))]),
                ("side", vec![bid("rubicon", BidStatus::Timeout, None)]),
                ("foot", vec![bid("rubicon", BidStatus::NoBid, None)]),
            ],
        )];
        let announcements = [Announcement {
            bidder_request_id: None,
            bids: Vec::new(),
            ..announced(
                "auction-0001",
                "rubicon",
                &[("top", "x"), ("side", "y"), ("foot", "z")],
            )
        }];
        let requests = [
            request(1, Some("rubicon"), 1_003.0, Some(1_100.0), "slot=top"),
            request(2, Some("rubicon"), 1_004.0, Some(1_600.0), "slot=side"),
            request(3, Some("rubicon"), 1_005.0, Some(1_120.0), "slot=foot"),
        ];
        let links = link(&mut auctions, &announcements, &requests, &[0.0]);
        for unit in ["top", "side", "foot"] {
            assert_eq!(exchange_of(&auctions, unit, "rubicon"), None, "{unit}");
        }
        assert!(links.is_empty());
    }

    #[test]
    fn two_announcements_that_could_claim_the_same_request_get_neither() {
        let mut auctions = vec![
            auction(
                "auction-0001",
                vec![("top", vec![bid("ix", BidStatus::Timeout, None)])],
            ),
            auction(
                "auction-0002",
                vec![("side", vec![bid("ix", BidStatus::Timeout, None)])],
            ),
        ];
        let without_ids = |auction: &str, unit: &str| Announcement {
            bidder_request_id: None,
            bids: Vec::new(),
            ..announced(auction, "ix", &[(unit, "unused")])
        };
        let announcements = [
            without_ids("auction-0001", "top"),
            without_ids("auction-0002", "side"),
        ];
        // One request for two announcements made in the same millisecond.
        let requests = [request(1, Some("ix"), 1_010.0, Some(1_900.0), "s=1")];
        let links = link(&mut auctions, &announcements, &requests, &[0.0]);
        assert_eq!(auctions[0].ad_units[0].bids[0].exchange, None);
        assert_eq!(auctions[1].ad_units[0].bids[0].exchange, None);
        assert!(links.is_empty());
    }

    #[test]
    fn a_guess_by_timing_that_prebid_contradicts_is_dropped() {
        // Prebid has the bidder's bid; the only request that could be
        // guessed never answered. It is not this bid's request.
        let mut auctions = vec![auction(
            "auction-0001",
            vec![("top", vec![bid("ix", BidStatus::Bid, Some(0.7))])],
        )];
        let announcements = [Announcement {
            bidder_request_id: None,
            bids: Vec::new(),
            ..announced("auction-0001", "ix", &[("top", "unused")])
        }];
        for answer in [None, Some(1_900.0)] {
            let requests = [request(1, Some("ix"), 1_010.0, answer, "s=1")];
            let links = link(&mut auctions, &announcements, &requests, &[0.0]);
            assert_eq!(exchange_of(&auctions, "top", "ix"), None, "{answer:?}");
            assert!(links.is_empty());
        }
        // With the bid's id in the request the network's word stands.
        let announcements = [announced("auction-0001", "ix", &[("top", "bid-ix-top")])];
        let requests = [request(1, Some("ix"), 1_010.0, Some(1_900.0), "bid-ix-top")];
        link(&mut auctions, &announcements, &requests, &[0.0]);
        let exchange = exchange_of(&auctions, "top", "ix").unwrap();
        assert_eq!(exchange.verdict, ExchangeVerdict::LateOnNetwork);
        assert!(exchange.exact);
    }

    #[test]
    fn several_requests_that_name_the_auction_but_no_bid_are_not_split_by_guessing() {
        let mut auctions = vec![auction(
            "auction-0001",
            vec![
                ("top", vec![bid("ix", BidStatus::Bid, Some(0.7))]),
                ("side", vec![bid("ix", BidStatus::Timeout, None)]),
            ],
        )];
        let announcements = [Announcement {
            bids: Vec::new(),
            ..announced("auction-0001", "ix", &[("top", "x"), ("side", "y")])
        }];
        let requests = [
            request(
                1,
                Some("ix"),
                1_003.0,
                Some(1_100.0),
                "tid=auction-0001&slot=a",
            ),
            request(
                2,
                Some("ix"),
                1_004.0,
                Some(1_700.0),
                "tid=auction-0001&slot=b",
            ),
        ];
        link(&mut auctions, &announcements, &requests, &[0.0]);
        assert_eq!(exchange_of(&auctions, "top", "ix"), None);
        assert_eq!(exchange_of(&auctions, "side", "ix"), None);
    }

    #[test]
    fn not_sent_is_not_said_of_a_request_whose_body_was_not_read_whole() {
        let mut auctions = vec![auction(
            "auction-0001",
            vec![
                ("top", vec![bid("triplelift", BidStatus::Bid, Some(0.4))]),
                ("side", vec![bid("triplelift", BidStatus::NoBid, None)]),
            ],
        )];
        let announcements = [announced(
            "auction-0001",
            "triplelift",
            &[("top", "bid-tl-top"), ("side", "bid-tl-side")],
        )];
        // The second impression may be in the part that was cut off.
        let requests = [ExchangeRequest {
            complete: false,
            ..request(
                9,
                Some("triplelift"),
                1_010.0,
                Some(1_120.0),
                r#"{"imp":[{"id":"bid-tl-top"},"#,
            )
        }];
        link(&mut auctions, &announcements, &requests, &[0.0]);
        assert_eq!(
            exchange_of(&auctions, "top", "triplelift").unwrap().verdict,
            ExchangeVerdict::InTime
        );
        assert_eq!(exchange_of(&auctions, "side", "triplelift"), None);
    }

    #[test]
    fn not_sent_is_not_said_while_a_request_that_could_be_the_missing_one_is_unexplained() {
        let announcements = [Announcement {
            bidder_request_id: None,
            bids: Vec::new(),
            ..announced("auction-0001", "ix", &[("top", "unused")])
        }];
        for unexplained in [
            // The bidder's own request, sent later than a guess would allow
            // (the adapter waited for an id, or Prebid queued it).
            request(1, Some("ix"), 1_900.0, Some(2_000.0), "s=1"),
            // An auction endpoint the capture has no adapter for.
            request(1, None, 1_020.0, Some(1_100.0), "{}"),
            // A Prebid Server call, which can carry any bidder.
            ExchangeRequest {
                via_server: true,
                ..request(1, None, 1_020.0, Some(1_100.0), "{}")
            },
        ] {
            let mut auctions = vec![auction(
                "auction-0001",
                vec![("top", vec![bid("ix", BidStatus::Timeout, None)])],
            )];
            // The bidder's endpoint is known from an earlier auction.
            let requests = [
                request(0, Some("ix"), 100.0, Some(150.0), "s=1"),
                unexplained,
            ];
            link(&mut auctions, &announcements, &requests, &[0.0]);
            assert_eq!(exchange_of(&auctions, "top", "ix"), None);
        }
    }
}
