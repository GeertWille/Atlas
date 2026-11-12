//! The TCF consent state over time, and what each bidder was sent of it.
//!
//! A visit can span several page views, and a choice made on one holds on
//! the next. So every time here is ms since the scan's navigation start, not
//! since the start of a page view.

use std::collections::BTreeSet;

use adscan_model::{
    Auction, Capture, ConsentReport, DiagnosticArea, DiagnosticCheck, DiagnosticStatus,
    TrackerEventKind as Ev,
};

use super::{capitalized, check, codes, pages_seen};

/// A wrapper that starts its auction from the CMP's callback runs in the same
/// moment the tracker's own listener hears of the choice, and either may come
/// first. An auction this close before the recorded choice was not early.
const CALLBACK_TOLERANCE_MS: f64 = 250.0;

/// What the CMP reported at one moment.
#[derive(Debug, Clone, Copy)]
struct State<'a> {
    t: f64,
    /// The user's choice is known: loaded from an earlier visit or just made.
    decided: bool,
    gdpr_applies: Option<bool>,
    has_tc_string: bool,
    purpose_consents: &'a [u32],
    vendor_consents: &'a [u32],
}

pub struct ConsentTimeline<'a> {
    capture: &'a Capture,
    cmp_present: bool,
    states: Vec<State<'a>>,
}

/// The consent signals of one bidder request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RequestSignals {
    pub page: u32,
    /// ms since the scan's navigation start.
    pub t: f64,
    pub gdpr_applies: Option<bool>,
    pub has_consent_string: Option<bool>,
}

impl<'a> ConsentTimeline<'a> {
    pub fn from_capture(capture: &'a Capture) -> Self {
        let mut timeline = Self {
            capture,
            cmp_present: false,
            states: Vec::new(),
        };
        for event in &capture.events {
            match &event.kind {
                Ev::CmpDetected { .. } => timeline.cmp_present = true,
                Ev::TcfState {
                    event_status,
                    gdpr_applies,
                    has_tc_string,
                    purpose_consents,
                    vendor_consents,
                } => {
                    timeline.cmp_present = true;
                    timeline.states.push(State {
                        t: capture.absolute_ms(event.page, event.t),
                        decided: matches!(
                            event_status.as_deref(),
                            Some("tcloaded" | "useractioncomplete")
                        ),
                        gdpr_applies: *gdpr_applies,
                        has_tc_string: *has_tc_string,
                        purpose_consents,
                        vendor_consents,
                    });
                }
                _ => {}
            }
        }
        // Events are grouped by page view; states are looked up by time.
        timeline.states.sort_by(|a, b| a.t.total_cmp(&b.t));
        timeline
    }

    fn state_at(&self, t: f64) -> Option<&State<'a>> {
        self.states.iter().rev().find(|state| state.t <= t)
    }

    /// Whether the CMP ever said that GDPR applies to this page view.
    fn gdpr_applies(&self) -> bool {
        self.states
            .iter()
            .any(|state| state.gdpr_applies == Some(true))
    }

    fn decided_at(&self) -> Option<f64> {
        self.states
            .iter()
            .find(|state| state.decided)
            .map(|state| state.t)
    }

    /// Whether the user's choice was known when something happened at `t`.
    fn decided_by(&self, t: f64) -> bool {
        self.decided_at()
            .is_some_and(|at| at <= t + CALLBACK_TOLERANCE_MS)
    }

    /// `auctions_with_consent` are the ids of the auctions in which Prebid
    /// passed a consent string: for those the choice was evidently known.
    /// `started_at` gives an auction's start in ms since the scan's
    /// navigation start.
    pub fn report(
        &self,
        auctions: &[Auction],
        started_at: impl Fn(&Auction) -> f64,
        consent_management_configured: Option<bool>,
        auctions_with_consent: &BTreeSet<&str>,
    ) -> ConsentReport {
        let decided_at = self.decided_at();
        let auctions_before_consent = if self.gdpr_applies() {
            auctions
                .iter()
                .filter(|auction| {
                    !self.decided_by(started_at(auction))
                        && !auctions_with_consent.contains(auction.id.as_str())
                })
                .count() as u32
        } else {
            0
        };
        let last = self.states.last();
        ConsentReport {
            cmp_present: self.cmp_present,
            consent_management_configured,
            gdpr_applies: last.and_then(|state| state.gdpr_applies),
            resolved_at_ms: decided_at,
            auction_count: auctions.len() as u32,
            auctions_before_consent,
            consented_purposes: last
                .map(|state| state.purpose_consents.to_vec())
                .unwrap_or_default(),
        }
    }

    /// `vendor` is the bidder's IAB vendor id with the name to show for it.
    pub fn bidder_checks(
        &self,
        requests: &[RequestSignals],
        vendor: Option<(u32, &str)>,
    ) -> Vec<DiagnosticCheck> {
        // A request that left before the CMP said anything is judged by what
        // the CMP said afterwards.
        let gdpr_applied = |request: &RequestSignals| {
            request.gdpr_applies == Some(true)
                || match self.state_at(request.t) {
                    Some(state) => state.gdpr_applies == Some(true),
                    None => self.gdpr_applies(),
                }
        };
        let under_gdpr: Vec<&RequestSignals> = requests
            .iter()
            .filter(|request| gdpr_applied(request))
            .collect();
        if under_gdpr.is_empty() {
            let reason = if !self.cmp_present {
                "The page exposes no TCF consent API, so there was no consent to check"
            } else if requests.is_empty() {
                "No bid request was made, so no consent was passed"
            } else {
                "GDPR did not apply to this page view, so no consent was needed"
            };
            return vec![check(
                DiagnosticArea::Consent,
                DiagnosticStatus::NotChecked,
                codes::CONSENT_NOT_APPLICABLE,
                reason,
            )];
        }

        let total = under_gdpr.len();
        let mut checks = Vec::new();
        let mut problem = |code: &str, summary: String, offending: &[&&RequestSignals]| {
            let mut check = check(
                DiagnosticArea::Consent,
                DiagnosticStatus::Problem,
                code,
                summary,
            );
            check.pages = pages_seen(self.capture, offending.iter().map(|request| request.page));
            checks.push(check);
        };
        let without_string: Vec<_> = under_gdpr
            .iter()
            .filter(|request| request.has_consent_string != Some(true))
            .collect();
        if !without_string.is_empty() {
            problem(
                codes::CONSENT_NO_STRING,
                format!(
                    "{} went out without a consent string although GDPR applies",
                    capitalized(&share_of_requests(without_string.len(), total))
                ),
                &without_string,
            );
        }
        let undecided: Vec<_> = under_gdpr
            .iter()
            .filter(|request| {
                request.has_consent_string != Some(true) && !self.decided_by(request.t)
            })
            .collect();
        if !undecided.is_empty() {
            problem(
                codes::CONSENT_PENDING,
                format!(
                    "{} went out before the user's consent choice was known",
                    capitalized(&share_of_requests(undecided.len(), total))
                ),
                &undecided,
            );
        }
        if let Some((vendor_id, name)) = vendor {
            let denied: Vec<_> = under_gdpr
                .iter()
                .filter(|request| {
                    self.state_at(request.t).is_some_and(|state| {
                        state.decided
                            && state.has_tc_string
                            && !state.vendor_consents.contains(&vendor_id)
                    })
                })
                .collect();
            if !denied.is_empty() {
                problem(
                    codes::CONSENT_VENDOR_DENIED,
                    format!(
                        "The user's consent does not cover {name} (IAB vendor {vendor_id}) on {}",
                        share_of_requests(denied.len(), total)
                    ),
                    &denied,
                );
            }
        }
        if checks.is_empty() {
            let summary = match vendor {
                Some((vendor_id, name)) => format!(
                    "A consent string was passed and it covers {name} (IAB vendor {vendor_id})"
                ),
                None => "A consent string was passed; this bidder's IAB vendor id is not known, so its vendor consent was not checked".to_string(),
            };
            checks.push(check(
                DiagnosticArea::Consent,
                DiagnosticStatus::Pass,
                codes::CONSENT_OK,
                summary,
            ));
        }
        checks
    }
}

fn share_of_requests(part: usize, whole: usize) -> String {
    match (part, whole) {
        (1, 1) => "its bid request".into(),
        (part, whole) if part == whole => format!("all {whole} bid requests"),
        (part, whole) => format!("{part} of {whole} bid requests"),
    }
}

#[cfg(test)]
mod tests {
    use adscan_model::{AuctionAdUnit, CapturedPage, PageReach, TrackerEvent};

    use super::*;

    fn visit(events: &[TrackerEvent]) -> Capture {
        Capture::single_page("https://pub.example/", events.to_vec())
    }

    fn started(auction: &Auction) -> f64 {
        auction.started_at_ms
    }

    fn tcf(t: f64, status: &str, gdpr: Option<bool>, vendors: &[u32]) -> TrackerEvent {
        TrackerEvent {
            page: 0,
            t,
            kind: Ev::TcfState {
                event_status: Some(status.into()),
                gdpr_applies: gdpr,
                has_tc_string: status != "cmpuishown",
                purpose_consents: if status == "cmpuishown" {
                    vec![]
                } else {
                    vec![1, 2, 7]
                },
                vendor_consents: vendors.to_vec(),
            },
        }
    }

    fn cmp(t: f64) -> TrackerEvent {
        TrackerEvent {
            page: 0,
            t,
            kind: Ev::CmpDetected { api: "tcf".into() },
        }
    }

    fn request(t: f64, gdpr: Option<bool>, string: Option<bool>) -> RequestSignals {
        RequestSignals {
            page: 0,
            t,
            gdpr_applies: gdpr,
            has_consent_string: string,
        }
    }

    fn auction(started_at_ms: f64) -> Auction {
        Auction {
            page: 0,
            id: format!("a-{started_at_ms}"),
            started_at_ms,
            ended_at_ms: None,
            duration_ms: None,
            timeout_ms: None,
            ad_units: Vec::<AuctionAdUnit>::new(),
        }
    }

    fn outcomes(checks: &[DiagnosticCheck]) -> Vec<(DiagnosticStatus, &str, &str)> {
        checks
            .iter()
            .map(|check| (check.status, check.code.as_str(), check.summary.as_str()))
            .collect()
    }

    const XANDR: Option<(u32, &str)> = Some((32, "appnexus"));

    #[test]
    fn auctions_that_start_before_the_choice_is_known_are_counted() {
        let events = [
            cmp(10.0),
            tcf(300.0, "cmpuishown", Some(true), &[]),
            tcf(2_000.0, "useractioncomplete", Some(true), &[32, 52]),
        ];
        let capture = visit(&events);
        let timeline = ConsentTimeline::from_capture(&capture);
        let report = timeline.report(
            &[
                auction(500.0),
                auction(1_700.0),
                auction(1_800.0),
                auction(2_500.0),
            ],
            started,
            Some(true),
            &BTreeSet::new(),
        );
        assert_eq!(
            report,
            ConsentReport {
                cmp_present: true,
                consent_management_configured: Some(true),
                gdpr_applies: Some(true),
                resolved_at_ms: Some(2_000.0),
                auction_count: 4,
                // The third starts within the callback tolerance of the choice.
                auctions_before_consent: 2,
                consented_purposes: vec![1, 2, 7],
            }
        );
    }

    #[test]
    fn a_choice_that_is_never_made_leaves_every_auction_before_consent() {
        let events = [tcf(300.0, "cmpuishown", Some(true), &[])];
        let capture = visit(&events);
        let timeline = ConsentTimeline::from_capture(&capture);
        let report = timeline.report(&[auction(500.0)], started, None, &BTreeSet::new());
        assert_eq!(report.auctions_before_consent, 1);
        // Prebid passed a consent string in this auction, so it knew the choice.
        let with_string =
            timeline.report(&[auction(500.0)], started, None, &BTreeSet::from(["a-500"]));
        assert_eq!(with_string.auctions_before_consent, 0);
        assert_eq!(report.resolved_at_ms, None);
        assert_eq!(report.consented_purposes, Vec::<u32>::new());
    }

    #[test]
    fn nothing_is_counted_where_gdpr_does_not_apply_or_no_cmp_exists() {
        let outside = [tcf(100.0, "tcloaded", Some(false), &[])];
        let outside = visit(&outside);
        let report = ConsentTimeline::from_capture(&outside).report(
            &[auction(50.0)],
            started,
            Some(false),
            &BTreeSet::new(),
        );
        assert_eq!(report.auctions_before_consent, 0);
        assert_eq!(report.gdpr_applies, Some(false));

        let no_cmp = visit(&[]);
        let report = ConsentTimeline::from_capture(&no_cmp).report(
            &[auction(50.0)],
            started,
            None,
            &BTreeSet::new(),
        );
        assert!(!report.cmp_present);
        assert_eq!(report.auctions_before_consent, 0);
        assert_eq!(report.gdpr_applies, None);
    }

    #[test]
    fn a_bidder_is_checked_against_the_state_in_effect_at_each_request() {
        let events = [
            tcf(100.0, "cmpuishown", Some(true), &[]),
            tcf(1_000.0, "useractioncomplete", Some(true), &[52]),
            tcf(5_000.0, "useractioncomplete", Some(true), &[32, 52]),
        ];
        let capture = visit(&events);
        let timeline = ConsentTimeline::from_capture(&capture);
        use DiagnosticStatus::{NotChecked, Pass, Problem};
        /// Bid requests, the bidder's vendor, expected checks.
        type Case<'a> = (
            &'a [RequestSignals],
            Option<(u32, &'static str)>,
            &'a [(DiagnosticStatus, &'static str, &'static str)],
        );
        let cases: [Case; 7] = [
            (
                &[request(6_000.0, Some(true), Some(true))],
                XANDR,
                &[(
                    Pass,
                    "consent-ok",
                    "A consent string was passed and it covers appnexus (IAB vendor 32)",
                )],
            ),
            (
                &[
                    request(2_000.0, Some(true), Some(true)),
                    request(6_000.0, Some(true), Some(true)),
                ],
                XANDR,
                &[(
                    Problem,
                    "consent-vendor-denied",
                    "The user's consent does not cover appnexus (IAB vendor 32) on 1 of 2 bid requests",
                )],
            ),
            (
                &[request(500.0, Some(true), Some(false))],
                XANDR,
                &[
                    (
                        Problem,
                        "consent-no-string",
                        "Its bid request went out without a consent string although GDPR applies",
                    ),
                    (
                        Problem,
                        "consent-pending",
                        "Its bid request went out before the user's consent choice was known",
                    ),
                ],
            ),
            // Prebid attached no consent data at all, but the CMP says GDPR applies.
            (
                &[request(6_000.0, None, None), request(6_100.0, None, None)],
                XANDR,
                &[(
                    Problem,
                    "consent-no-string",
                    "All 2 bid requests went out without a consent string although GDPR applies",
                )],
            ),
            (
                &[request(6_000.0, Some(true), Some(true))],
                None,
                &[(
                    Pass,
                    "consent-ok",
                    "A consent string was passed; this bidder's IAB vendor id is not known, so its vendor consent was not checked",
                )],
            ),
            // Sent before the CMP had said anything.
            (
                &[request(50.0, None, None)],
                XANDR,
                &[
                    (
                        Problem,
                        "consent-no-string",
                        "Its bid request went out without a consent string although GDPR applies",
                    ),
                    (
                        Problem,
                        "consent-pending",
                        "Its bid request went out before the user's consent choice was known",
                    ),
                ],
            ),
            (
                &[],
                XANDR,
                &[(
                    NotChecked,
                    "consent-not-applicable",
                    "No bid request was made, so no consent was passed",
                )],
            ),
        ];
        for (requests, vendor, expected) in cases {
            assert_eq!(
                outcomes(&timeline.bidder_checks(requests, vendor)),
                expected
            );
        }
    }

    #[test]
    fn without_a_cmp_or_outside_gdpr_there_is_no_consent_to_check() {
        let not_applicable = |events: &[TrackerEvent]| {
            let timeline = ConsentTimeline::from_events(events);
            let checks = timeline.bidder_checks(&[request(500.0, None, None)], XANDR);
            assert_eq!(checks.len(), 1);
            assert_eq!(checks[0].status, DiagnosticStatus::NotChecked);
            assert_eq!(checks[0].code, "consent-not-applicable");
            checks[0].summary.clone()
        };
        assert_eq!(
            not_applicable(&[]),
            "The page exposes no TCF consent API, so there was no consent to check"
        );
        assert_eq!(
            not_applicable(&[tcf(100.0, "tcloaded", Some(false), &[])]),
            "GDPR did not apply to this page view, so no consent was needed"
        );
    }
}
