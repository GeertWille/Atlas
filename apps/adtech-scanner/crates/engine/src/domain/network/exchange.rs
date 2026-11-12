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
    AdLink, Auction, BidExchange, BidStatus, ExchangeVerdict, TrackedBidRequest, TrackerEvent,
    TrackerEventKind, request_id,
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
    /// From the decoded answer.
    pub best_cpm: Option<f64>,
    pub nbr: Option<u32>,
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

/// The ad unit a bid id of the tracker stands for.
pub fn ad_unit_of_bid(announcements: &[Announcement], bid_id: &str) -> Option<String> {
    announcements
        .iter()
        .flat_map(|announcement| &announcement.bids)
        .find(|bid| bid.bid_id == bid_id)
        .map(|bid| bid.ad_unit_code.clone())
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
    let absolute = |page: u32, at_ms: f64| page_starts.get(page as usize).copied().unwrap_or(0.0) + at_ms;
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
    let mut claimed: HashSet<u32> = matched
        .iter()
        .flatten()
        .map(|found| found.request.seq)
        .collect();
    for (announcement, matches) in announcements.iter().zip(&mut matched) {
        if !matches.is_empty() {
            continue;
        }
        let announced_at = absolute(announcement.page, announcement.at_ms);
        let nearest = requests
            .iter()
            .filter(|request| {
                request.bidder.as_deref() == Some(announcement.bidder.as_str())
                    && !claimed.contains(&request.seq)
            })
            .map(|request| (request, absolute(request.page, request.started_ms) - announced_at))
            .filter(|(_, after)| (-TIMING_MATCH_BEFORE_MS..=TIMING_MATCH_AFTER_MS).contains(after))
            .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()));
        if let Some((request, _)) = nearest {
            claimed.insert(request.seq);
            matches.push(Match {
                request,
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
                _ => matches.first(),
            };
            let Some(bid) = auction
                .ad_units
                .iter_mut()
                .find(|unit| &unit.code == code)
                .and_then(|unit| unit.bids.iter_mut().find(|bid| bid.bidder == announcement.bidder))
            else {
                continue;
            };
            let Some(found) = carrier else {
                // "Not sent" is only said where the bidder's requests can be
                // seen at all: another ad unit's bid is in the request, or
                // the capture knows this bidder's endpoint.
                if ids_are_visible || visible_bidders.contains(announcement.bidder.as_str()) {
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
            if request.best_cpm.is_none() {
                link.cpm = match (link.cpm, bid.cpm) {
                    (Some(best), Some(cpm)) => Some(best.max(cpm)),
                    (best, cpm) => best.or(cpm),
                };
            }
        }
    }
    links.into_values().collect()
}

fn exact_matches<'a>(announcement: &Announcement, requests: &'a [ExchangeRequest]) -> Vec<Match<'a>> {
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

    fn request(seq: u32, bidder: Option<&str>, started: f64, ended: Option<f64>, body: &str) -> ExchangeRequest {
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
            best_cpm: None,
            nbr: None,
        }
    }

    fn exchange_of<'a>(auctions: &'a [Auction], unit: &str, bidder: &str) -> Option<&'a BidExchange> {
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
        let announcements = [announced("auction-0001", "openx", &[("top", "bid-openx-top")])];
        let requests = [
            // Started closer in time, but it is somebody else's.
            request(1, Some("openx"), 1_002.0, Some(1_100.0), r#"{"imp":[{"id":"bid-other-x"}]}"#),
            ExchangeRequest {
                nbr: Some(7),
                ..request(2, Some("openx"), 1_040.0, Some(1_212.0), r#"{"imp":[{"id":"bid-openx-top"}]}"#)
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
        let announcements = [announced("auction-0001", "rubicon", &[("top", "bid-rubicon-top")])];
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
            &[("top", "bid-tl-top"), ("sidebar", "bid-tl-side"), ("anchor", "bid-tl-anchor")],
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
            exchange_of(&auctions, "top", "triplelift").unwrap().request_id.as_deref(),
            Some("r9")
        );
        for unit in ["sidebar", "anchor"] {
            let exchange = exchange_of(&auctions, unit, "triplelift").unwrap();
            assert_eq!(exchange.verdict, ExchangeVerdict::NotSent, "{unit}");
            assert_eq!(exchange.request_id, None);
        }
        assert_eq!(links[0].link.ad_units, ["top"]);
        assert_eq!(links[0].link.cpm, Some(0.4), "from Prebid's bid when the answer was not decoded");
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
        let announcements = [announced("auction-0001", "obscure", &[("top", "bid-obscure")])];
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
            ..request(3, None, 1_004.0, Some(1_200.0), r#"{"id":"auction-0001","imp":[{"id":"top"}]}"#)
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
    fn a_request_with_an_id_in_it_is_not_given_to_another_auction_by_timing() {
        let mut auctions = vec![
            auction("auction-0001", vec![("top", vec![bid("openx", BidStatus::NoBid, None)])]),
            auction("auction-0002", vec![("top", vec![bid("openx", BidStatus::NoBid, None)])]),
        ];
        let announcements = [
            Announcement {
                bids: Vec::new(),
                bidder_request_id: None,
                ..announced("auction-0001", "openx", &[("top", "unused-id-1")])
            },
            announced("auction-0002", "openx", &[("top", "bid-second-top")]),
        ];
        let requests = [request(7, Some("openx"), 1_002.0, Some(1_100.0), "bid-second-top")];
        link(&mut auctions, &announcements, &requests, &[0.0]);
        assert_eq!(
            auctions[1].ad_units[0].bids[0].exchange.as_ref().unwrap().request_id.as_deref(),
            Some("r7")
        );
        assert_eq!(
            auctions[0].ad_units[0].bids[0].exchange.as_ref().unwrap().verdict,
            ExchangeVerdict::NotSent
        );
    }

    #[test]
    fn times_on_different_page_views_are_compared_on_one_clock() {
        let mut auctions = vec![Auction {
            page: 1,
            ..auction("auction-0001", vec![("top", vec![bid("openx", BidStatus::NoBid, None)])])
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
                kind: TrackerEventKind::AuctionEnd { auction_id: "a".into() },
            },
        ];
        let found = announcements(&events);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].page, found[0].at_ms, found[0].bidder.as_str()), (2, 77.0, "ix"));
        assert_eq!(ad_unit_of_bid(&found, "b1").as_deref(), Some("top"));
        assert_eq!(ad_unit_of_bid(&found, "nope"), None);
    }
}
