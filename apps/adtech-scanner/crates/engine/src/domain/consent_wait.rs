//! What became of the consent dialog on each page view, and what that means
//! for reading the rest of the scan: an ad stack that waits for consent has
//! nothing to show until someone gives it. Whether the scanner gave it, a
//! person did, or nobody, decides whether a page view without auctions says
//! something about the site or only about the scan.

use adscan_model::{
    Capture, ConsentNoteKind, ScanEnvironment, TrackerEventKind as Ev, VisitorBehavior,
};

/// A choice the consent platform reports this long after the scanner's click
/// (or shortly before it was noted) is that click's.
const CLICK_TO_CHOICE_MS: std::ops::RangeInclusive<f64> = -1_000.0..=5_000.0;
/// Consent that comes later than this into a page view held the page up.
pub const LATE_CONSENT_MS: f64 = 3_000.0;
/// A dialog the consent platform reported this long before the scanner
/// accepted it was up while the scanner was not yet looking.
const SHOWN_BEFORE_ACCEPTED_MS: f64 = 1_000.0;

/// The consent dialog of one page view. Times are ms into the page view.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PageConsent {
    pub page: u32,
    /// When the consent platform said its dialog was showing.
    pub prompt_at_ms: Option<f64>,
    /// When it said the visitor had made a choice.
    pub decided_at_ms: Option<f64>,
    /// When the scanner pressed an accept-all control, and which.
    pub scanner_accepted: Option<(f64, String)>,
    /// A dialog the scanner saw and found no accept-all control in.
    pub dialog_without_accept: Option<String>,
}

impl PageConsent {
    pub fn of(capture: &Capture, page: u32) -> Self {
        let status_at = |wanted: &str| {
            capture
                .events
                .iter()
                .filter(|event| event.page == page)
                .find_map(|event| match &event.kind {
                    Ev::TcfState { event_status, .. }
                        if event_status.as_deref() == Some(wanted) =>
                    {
                        Some(event.t)
                    }
                    _ => None,
                })
        };
        let note = |kind: ConsentNoteKind| {
            capture
                .consent_notes
                .iter()
                .find(|note| note.page == page && note.kind == kind)
        };
        Self {
            page,
            prompt_at_ms: status_at("cmpuishown"),
            decided_at_ms: status_at("useractioncomplete"),
            scanner_accepted: note(ConsentNoteKind::Accepted)
                .map(|note| (note.t, note.banner.clone())),
            dialog_without_accept: note(ConsentNoteKind::DialogWithoutAccept)
                .map(|note| note.banner.clone()),
        }
    }

    /// When a choice was made that the scanner did not make.
    pub fn decided_by_someone_else_at(&self) -> Option<f64> {
        let decided_at = self.decided_at_ms?;
        let by_scanner = self
            .scanner_accepted
            .as_ref()
            .is_some_and(|(clicked_at, _)| CLICK_TO_CHOICE_MS.contains(&(decided_at - clicked_at)));
        (!by_scanner).then_some(decided_at)
    }

    /// The dialog was showing and no choice was made, by anyone.
    pub fn unanswered(&self) -> bool {
        (self.prompt_at_ms.is_some() || self.dialog_without_accept.is_some())
            && self.decided_at_ms.is_none()
            && self.scanner_accepted.is_none()
    }

    /// When consent came, where it came late enough to have held the page
    /// view up: an ad stack that waits for consent could not start before.
    pub fn held_up_until_ms(&self) -> Option<f64> {
        let given_at = self
            .decided_at_ms
            .or(self.scanner_accepted.as_ref().map(|(at, _)| *at))?;
        (given_at >= LATE_CONSENT_MS).then_some(given_at)
    }

    /// One sentence on what happened, when it matters for reading the page
    /// view's results; `None` for a dialog that was accepted promptly, and
    /// for a page view without one.
    pub fn account(&self, environment: Option<&ScanEnvironment>) -> Option<String> {
        let page = self.page + 1;
        let manual = environment.is_some_and(|seen| seen.visitor == VisitorBehavior::Manual);
        let seconds = |ms: f64| format!("{:.1} s", ms / 1000.0);
        let why_not = match &self.dialog_without_accept {
            Some(dialog) => format!(
                " (it found a {dialog} dialog with no control that plainly means \"accept all\")"
            ),
            None => String::new(),
        };
        let consequence = "Where the ad stack waits for consent, nothing on that page view could start \
             before, so auctions that are missing or late there are a consequence of how the scan went, \
             not by themselves a fault of the site";
        if manual {
            let given_at = self.held_up_until_ms()?;
            return Some(format!(
                "On page {page} the person browsing gave consent {} into the page view. {consequence}",
                seconds(given_at)
            ));
        }
        if let Some(decided_at) = self.decided_by_someone_else_at() {
            let who = match environment {
                Some(seen) if !seen.headless => "by hand in the browser window",
                _ => "by something other than the scanner",
            };
            return Some(format!(
                "On page {page} a consent dialog was showing{} and the scanner did not accept it{why_not}. \
                 Consent was given {who} {} into the page view. {consequence}",
                self.prompt_at_ms
                    .map_or(String::new(), |at| format!(" from {}", seconds(at))),
                seconds(decided_at),
            ));
        }
        if self.unanswered() {
            return Some(format!(
                "On page {page} a consent dialog was showing{} and nobody answered it: the scanner did not \
                 accept it{why_not}. Where the ad stack waits for consent, nothing on that page view could \
                 start at all, so missing auctions there are a consequence of how the scan went, not by \
                 themselves a fault of the site",
                self.prompt_at_ms
                    .map_or(String::new(), |at| format!(" from {}", seconds(at))),
            ));
        }
        let (accepted_at, banner) = self.scanner_accepted.as_ref()?;
        if *accepted_at < LATE_CONSENT_MS {
            return None;
        }
        let accepted = format!(
            "On page {page} the scanner accepted the consent dialog ({banner}) {} into the page view.",
            seconds(*accepted_at)
        );
        let earlier = match self.prompt_at_ms {
            Some(shown) if accepted_at - shown >= SHOWN_BEFORE_ACCEPTED_MS => format!(
                "The consent platform reported the dialog from {}; the scanner only starts looking for a \
                 dialog once the page has loaded.",
                seconds(shown)
            ),
            _ => "The scanner only starts looking for a dialog once the page has loaded, so it may have \
                  been showing earlier."
                .to_string(),
        };
        Some(format!("{accepted} {earlier} {consequence}"))
    }
}

/// The accounts of every page view that has one, in page order.
pub fn accounts(capture: &Capture, environment: Option<&ScanEnvironment>) -> Vec<String> {
    (0u32..)
        .zip(&capture.pages)
        .filter_map(|(page, _)| PageConsent::of(capture, page).account(environment))
        .collect()
}

#[cfg(test)]
mod tests {
    use adscan_model::{ConsentNote, ProxyKind, ProxyUse, TrackerEvent};

    use super::*;

    fn tcf(t: f64, status: &str) -> TrackerEvent {
        TrackerEvent {
            page: 0,
            t,
            kind: Ev::TcfState {
                event_status: Some(status.into()),
                gdpr_applies: Some(true),
                has_tc_string: status != "cmpuishown",
                purpose_consents: vec![],
                vendor_consents: vec![],
            },
        }
    }

    fn note(t: f64, kind: ConsentNoteKind, banner: &str) -> ConsentNote {
        ConsentNote {
            page: 0,
            t,
            kind,
            banner: banner.into(),
        }
    }

    fn environment(visitor: VisitorBehavior, headless: bool) -> ScanEnvironment {
        ScanEnvironment {
            browser: adscan_model::BrowserUse::Local,
            visitor,
            seed: 1,
            headless,
            persistent_profile: false,
            webdriver: Some(true),
            user_agent_overridden: false,
            viewport_overridden: false,
            emulated_mobile: false,
            detectors: vec![],
            proxy: ProxyUse {
                kind: ProxyKind::System,
                name: None,
                host: None,
            },
        }
    }

    fn consent(events: Vec<TrackerEvent>, notes: Vec<ConsentNote>) -> PageConsent {
        let mut capture = Capture::single_page("https://pub.example/", events);
        capture.consent_notes = notes;
        PageConsent::of(&capture, 0)
    }

    #[test]
    fn a_dialog_the_scanner_accepted_at_once_needs_no_account() {
        let prompt = consent(
            vec![tcf(300.0, "cmpuishown"), tcf(1_450.0, "useractioncomplete")],
            vec![note(1_400.0, ConsentNoteKind::Accepted, "Sourcepoint")],
        );
        assert_eq!(prompt.decided_by_someone_else_at(), None);
        assert_eq!(prompt.held_up_until_ms(), None);
        assert!(!prompt.unanswered());
        let browse = environment(VisitorBehavior::Browse, false);
        assert_eq!(prompt.account(Some(&browse)), None);
        assert_eq!(consent(vec![], vec![]).account(Some(&browse)), None);
    }

    #[test]
    fn consent_given_by_hand_after_the_scanner_failed_is_said_plainly() {
        let prompt = consent(
            vec![
                tcf(300.0, "cmpuishown"),
                tcf(27_000.0, "useractioncomplete"),
            ],
            vec![note(
                600.0,
                ConsentNoteKind::DialogWithoutAccept,
                "Sourcepoint",
            )],
        );
        assert_eq!(prompt.decided_by_someone_else_at(), Some(27_000.0));
        assert_eq!(prompt.held_up_until_ms(), Some(27_000.0));

        let shown = prompt
            .account(Some(&environment(VisitorBehavior::Browse, false)))
            .unwrap();
        assert!(
            shown.starts_with(
                "On page 1 a consent dialog was showing from 0.3 s and the scanner did not accept it \
                 (it found a Sourcepoint dialog with no control that plainly means \"accept all\"). \
                 Consent was given by hand in the browser window 27.0 s into the page view."
            ),
            "{shown}"
        );
        assert!(shown.contains("a consequence of how the scan went"));
        // Nobody can click in a browser without a window.
        let headless = prompt
            .account(Some(&environment(VisitorBehavior::Scroll, true)))
            .unwrap();
        assert!(
            headless.contains("by something other than the scanner"),
            "{headless}"
        );
    }

    #[test]
    fn a_dialog_nobody_answered_and_one_that_came_late_are_accounted_for() {
        let scroll = environment(VisitorBehavior::Scroll, true);
        let unanswered = consent(vec![tcf(300.0, "cmpuishown")], vec![]);
        assert!(unanswered.unanswered());
        assert!(
            unanswered
                .account(Some(&scroll))
                .unwrap()
                .contains("nobody answered it")
        );

        let late = consent(
            vec![
                tcf(12_000.0, "cmpuishown"),
                tcf(12_600.0, "useractioncomplete"),
            ],
            vec![note(12_500.0, ConsentNoteKind::Accepted, "OneTrust")],
        );
        assert_eq!(late.decided_by_someone_else_at(), None);
        assert_eq!(late.held_up_until_ms(), Some(12_600.0));
        let account = late.account(Some(&scroll)).unwrap();
        assert!(
            account.starts_with(
                "On page 1 the scanner accepted the consent dialog (OneTrust) 12.5 s into the page view. \
                 The scanner only starts looking for a dialog once the page has loaded, so it may have been \
                 showing earlier. Where the ad stack waits"
            ),
            "{account}"
        );

        // bild.de through a residential proxy: shown at 14.5 s, pressed at
        // 18.3 s. The dialog was there before; the scanner was not looking.
        let slow_page = consent(
            vec![
                tcf(14_500.0, "cmpuishown"),
                tcf(18_500.0, "useractioncomplete"),
            ],
            vec![note(18_300.0, ConsentNoteKind::Accepted, "Sourcepoint")],
        );
        let account = slow_page.account(Some(&scroll)).unwrap();
        assert!(
            account.starts_with(
                "On page 1 the scanner accepted the consent dialog (Sourcepoint) 18.3 s into the page view. \
                 The consent platform reported the dialog from 14.5 s; the scanner only starts looking for a \
                 dialog once the page has loaded. Where the ad stack waits"
            ),
            "{account}"
        );
        // Nothing claims the dialog was not there to accept.
        assert!(!account.contains("not there"), "{account}");

        // Without a report from the consent platform the same is said as a
        // possibility.
        let unreported = consent(
            vec![],
            vec![note(5_000.0, ConsentNoteKind::Accepted, "Didomi")],
        );
        let account = unreported.account(Some(&scroll)).unwrap();
        assert!(
            account.contains("so it may have been showing earlier."),
            "{account}"
        );
    }

    #[test]
    fn a_person_browsing_is_the_one_who_answers() {
        let manual = environment(VisitorBehavior::Manual, false);
        let slow = consent(
            vec![tcf(300.0, "cmpuishown"), tcf(9_000.0, "useractioncomplete")],
            vec![],
        );
        assert!(
            slow.account(Some(&manual)).unwrap().starts_with(
                "On page 1 the person browsing gave consent 9.0 s into the page view."
            )
        );
        let quick = consent(
            vec![tcf(300.0, "cmpuishown"), tcf(1_500.0, "useractioncomplete")],
            vec![],
        );
        assert_eq!(quick.account(Some(&manual)), None);
    }
}
