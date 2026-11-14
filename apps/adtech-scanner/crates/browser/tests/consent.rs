//! Consent click-through against fake CMP banners of the common shapes.

mod support;

use adscan_model::{
    Capture, CaptureRequest, ConsentNoteKind, DeviceProfile, TrackerEventKind as Kind,
};
use support::{Fixtures, Outcome};

struct Clicked {
    accepted: bool,
    console: Vec<String>,
    progress: Vec<String>,
}

async fn scan(fixture: &str, accept_consent: bool) -> Clicked {
    let fixtures = support::Fixtures::start().await;
    let mut request = support::request(&fixtures.url(fixture), DeviceProfile::Desktop);
    request.accept_consent = accept_consent;
    request.observe_seconds = 2;
    let outcome = support::run(request).await;
    let capture = outcome.result.expect("capture succeeds");
    Clicked {
        accepted: capture.consent_accepted,
        console: capture.console.into_iter().map(|line| line.text).collect(),
        progress: outcome.progress,
    }
}

fn assert_accepted(clicked: &Clicked, banner: &str, label: &str) {
    assert!(
        clicked.accepted,
        "consent_accepted should be set; progress: {:#?}",
        clicked.progress
    );
    assert_eq!(
        clicked.console,
        ["CONSENT accepted"],
        "exactly the accept button is clicked, once"
    );
    let line = format!("Accepted the consent banner ({banner}): pressed \"{label}\"");
    assert!(
        clicked.progress.contains(&line),
        "missing {line:?} in {:#?}",
        clicked.progress
    );
}

#[tokio::test]
async fn accepts_a_known_cmp_banner_that_appears_late() {
    assert_accepted(
        &scan("consent-onetrust.html", true).await,
        "OneTrust",
        "allow",
    );
}

#[tokio::test]
async fn accepts_a_banner_inside_shadow_dom() {
    assert_accepted(
        &scan("consent-shadow.html", true).await,
        "Usercentrics",
        "yes",
    );
}

#[tokio::test]
async fn accepts_a_banner_inside_a_cross_site_iframe() {
    assert_accepted(
        &scan("consent-iframe.html", true).await,
        "Sourcepoint",
        "continue",
    );
}

#[tokio::test]
async fn accepts_an_unknown_banner_by_its_dutch_accept_all_label() {
    assert_accepted(
        &scan("consent-generic.html", true).await,
        "button \"alles accepteren\"",
        "alles accepteren",
    );
}

#[tokio::test]
async fn short_labels_only_count_inside_a_consent_dialog() {
    assert_accepted(
        &scan("consent-context.html", true).await,
        "button \"ok\"",
        "ok",
    );
}

#[tokio::test]
async fn accepts_a_google_funding_choices_dialog() {
    assert_accepted(
        &scan("consent-funding-choices.html", true).await,
        "Google Funding Choices",
        "consent",
    );
}

#[tokio::test]
async fn presses_nothing_when_no_button_plainly_means_accept_all() {
    let clicked = scan("consent-paywall.html", true).await;
    assert!(!clicked.accepted, "{:#?}", clicked.progress);
    assert_eq!(clicked.console, Vec::<String>::new());
}

#[tokio::test]
async fn leaves_the_banner_alone_when_not_asked_to_accept() {
    let clicked = scan("consent-onetrust.html", false).await;
    assert!(!clicked.accepted);
    assert_eq!(clicked.console, Vec::<String>::new());
}

// -- consent platforms, one by one ------------------------------------------------------

/// The platforms `injected/consent.js` knows by their own markup. A new row
/// there gets a row here and one in `fixtures/consent-platforms.html`.
const PLATFORMS: [&str; 14] = [
    "Sourcepoint",
    "OneTrust",
    "Didomi",
    "Cookiebot",
    "Quantcast / InMobi Choice",
    "Usercentrics",
    "TrustArc",
    "Google Funding Choices",
    "consentmanager.net",
    "Iubenda",
    "Borlabs Cookie",
    "Osano",
    "Complianz",
    "CookieYes",
];

fn accepting(url: &str) -> CaptureRequest {
    let mut request = support::request(url, DeviceProfile::Desktop);
    request.accept_consent = true;
    request.observe_seconds = 2;
    request
}

fn console_of(capture: &Capture) -> Vec<&str> {
    capture
        .console
        .iter()
        .map(|line| line.text.as_str())
        .collect()
}

fn captured(outcome: &Outcome) -> &Capture {
    outcome.result.as_ref().unwrap_or_else(|error| {
        panic!("capture failed: {error}\nprogress: {:#?}", outcome.progress)
    })
}

#[tokio::test]
async fn every_known_consent_platform_is_accepted_by_its_own_control() {
    let fixtures = Fixtures::start().await;
    for platforms in PLATFORMS.chunks(4) {
        let mut scans = tokio::task::JoinSet::new();
        for platform in platforms {
            let url = format!(
                "{}?cmp={}",
                fixtures.url("consent-platforms.html"),
                platform.replace(' ', "%20")
            );
            scans.spawn(async move { (*platform, support::run(accepting(&url)).await) });
        }
        while let Some(scanned) = scans.join_next().await {
            let (platform, outcome) = scanned.unwrap();
            let capture = captured(&outcome);
            assert!(
                capture.consent_accepted,
                "{platform}: {:#?}",
                outcome.progress
            );
            assert_eq!(
                console_of(capture),
                ["CONSENT accepted trusted=true"],
                "{platform}: the accept control and nothing else, pressed like a reader would"
            );
            let accepted: Vec<&str> = capture
                .consent_notes
                .iter()
                .filter(|note| note.kind == ConsentNoteKind::Accepted)
                .map(|note| note.banner.as_str())
                .collect();
            assert_eq!(accepted, [platform]);
        }
    }
}

// -- a consent wall in a frame of another origin --------------------------------------------

/// The page of a publisher whose consent message is served from the consent
/// platform's own origin: another port of this machine, which the browser
/// keeps in the page's process, as it does for a subdomain of the publisher.
fn choice_wall(fixtures: &Fixtures, cmp: &Fixtures, shape: &str, delay_ms: u32) -> String {
    format!(
        "{}?cmp={}&shape={shape}&delay={delay_ms}",
        fixtures.url("consent-sourcepoint.html"),
        cmp.url("")
    )
}

fn consent_statuses(capture: &Capture) -> Vec<&str> {
    capture
        .events
        .iter()
        .filter_map(|event| match &event.kind {
            Kind::TcfState { event_status, .. } => event_status.as_deref(),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_choice_wall_in_a_frame_of_another_origin_is_accepted_and_the_paid_option_never_pressed()
{
    let fixtures = Fixtures::start().await;
    let cmp = Fixtures::start().await;
    for (shape, banner, label) in [
        ("class", "Sourcepoint", "weiter"),
        // Sourcepoint's own message button, known by its title; the label
        // recorded is the title, not the icon.
        ("title", "Sourcepoint", "alle akzeptieren"),
        (
            "text",
            "button \"akzeptieren und weiter\"",
            "akzeptieren und weiter",
        ),
        // bild.de's wall as it was served, with its PUR subscription.
        ("bild", "Sourcepoint", "alle akzeptieren"),
    ] {
        let outcome = support::run(accepting(&choice_wall(&fixtures, &cmp, shape, 300))).await;
        let capture = captured(&outcome);

        assert!(capture.consent_accepted, "{shape}: {:#?}", outcome.progress);
        assert_eq!(
            console_of(capture),
            ["CHOICE accept-all trusted=true"],
            "{shape}: one real click on the accept option"
        );
        let line = format!("Accepted the consent banner ({banner}): pressed \"{label}\"");
        assert!(
            outcome.progress.contains(&line),
            "{shape}: missing {line:?} in {:#?}",
            outcome.progress
        );
        // The consent platform itself says the choice was made.
        assert_eq!(
            consent_statuses(capture),
            ["cmpuishown", "useractioncomplete"],
            "{shape}"
        );
        let note = &capture.consent_notes[0];
        assert_eq!(
            (note.kind, note.banner.as_str(), note.page),
            (ConsentNoteKind::Accepted, banner, 0),
            "{shape}"
        );
    }
}

#[tokio::test]
async fn a_wall_that_appears_long_after_the_page_loaded_is_still_accepted() {
    let fixtures = Fixtures::start().await;
    let cmp = Fixtures::start().await;
    let mut request = accepting(&choice_wall(&fixtures, &cmp, "class", 12_000));
    request.observe_seconds = 16;
    let outcome = support::run(request).await;
    let capture = captured(&outcome);

    assert!(capture.consent_accepted, "{:#?}", outcome.progress);
    assert_eq!(console_of(capture), ["CHOICE accept-all trusted=true"]);
    let accepted_at = capture.consent_notes[0].t;
    assert!(
        (12_000.0..16_000.0).contains(&accepted_at),
        "accepted at {accepted_at}"
    );
}

#[tokio::test]
async fn consent_or_pay_in_the_page_itself_gets_the_consent_and_never_the_purchase() {
    let clicked = scan("consent-choice-wall.html", true).await;
    assert_accepted(
        &clicked,
        "button \"akzeptieren und weiter\"",
        "akzeptieren und weiter",
    );
}

#[tokio::test]
async fn a_wall_is_left_alone_when_the_scan_was_not_asked_to_accept() {
    let fixtures = Fixtures::start().await;
    let cmp = Fixtures::start().await;
    let mut request = accepting(&choice_wall(&fixtures, &cmp, "class", 300));
    request.accept_consent = false;
    let outcome = support::run(request).await;
    let capture = captured(&outcome);

    assert!(!capture.consent_accepted);
    assert_eq!(console_of(capture), Vec::<&str>::new());
    assert!(capture.consent_notes.is_empty());
    assert_eq!(consent_statuses(capture), ["cmpuishown"]);
}

#[tokio::test]
async fn a_wall_without_an_accept_option_is_noted_once_and_nothing_is_pressed() {
    let fixtures = Fixtures::start().await;
    let cmp = Fixtures::start().await;
    let mut request = accepting(&choice_wall(&fixtures, &cmp, "none", 300));
    request.observe_seconds = 5;
    let outcome = support::run(request).await;
    let capture = captured(&outcome);

    assert!(!capture.consent_accepted);
    assert_eq!(console_of(capture), Vec::<&str>::new());
    let notes: Vec<(ConsentNoteKind, &str)> = capture
        .consent_notes
        .iter()
        .map(|note| (note.kind, note.banner.as_str()))
        .collect();
    assert_eq!(
        notes,
        [(ConsentNoteKind::DialogWithoutAccept, "Sourcepoint")]
    );
    // Once in the audit trail, however often the scanner looked.
    let said: Vec<&String> = outcome
        .progress
        .iter()
        .filter(|line| line.contains("Consent banner detected"))
        .collect();
    assert_eq!(
        said,
        [
            "Consent banner detected (Sourcepoint), automatic accept failed: nothing in it plainly means \"accept all\", so it was left alone"
        ]
    );
}

// -- markup seen on real sites -----------------------------------------------------------

#[tokio::test]
async fn a_cookiebot_dialog_in_the_publishers_own_design_is_accepted_by_its_danish_label() {
    assert_accepted(
        &scan("consent-cookiebot-custom.html", true).await,
        "button \"tillad alle\"",
        "tillad alle",
    );
}

#[tokio::test]
async fn both_generations_of_the_usercentrics_dialog_are_accepted() {
    let fixtures = Fixtures::start().await;
    for (version, label) in [("3", "alle akzeptieren"), ("2", "akzeptieren")] {
        let url = format!(
            "{}?version={version}",
            fixtures.url("consent-usercentrics.html")
        );
        let outcome = support::run(accepting(&url)).await;
        let capture = captured(&outcome);
        assert!(
            capture.consent_accepted,
            "{version}: {:#?}",
            outcome.progress
        );
        assert_eq!(
            console_of(capture),
            ["CONSENT accepted trusted=true"],
            "{version}: the accept control and nothing else"
        );
        let line = format!("Accepted the consent banner (Usercentrics): pressed \"{label}\"");
        assert!(
            outcome.progress.contains(&line),
            "{version}: missing {line:?} in {:#?}",
            outcome.progress
        );
    }
}

#[tokio::test]
async fn an_empty_consent_platform_container_is_not_reported_as_a_dialog() {
    let fixtures = Fixtures::start().await;
    // Usercentrics' empty host, which no dialog selector names, and an empty
    // Quantcast container, which the platform's dialog selector does name:
    // only its having no box keeps it from being reported.
    for fixture in ["consent-usercentrics.html", "consent-wrapper.html"] {
        let mut request = accepting(&format!("{}?version=empty", fixtures.url(fixture)));
        request.observe_seconds = 4;
        let outcome = support::run(request).await;
        let capture = captured(&outcome);

        assert!(!capture.consent_accepted, "{fixture}");
        assert_eq!(console_of(capture), Vec::<&str>::new(), "{fixture}");
        assert!(
            capture.consent_notes.is_empty(),
            "{fixture}: {:?}",
            capture.consent_notes
        );
        assert!(
            !outcome
                .progress
                .iter()
                .any(|line| line.contains("Consent banner detected")),
            "{fixture}: {:#?}",
            outcome.progress
        );
    }
}

/// A platform's container without height of its own, its dialog in a fixed
/// child: the dialog is showing, and one without an accept option is noted.
#[tokio::test]
async fn a_dialog_in_a_container_without_height_is_showing() {
    let fixtures = Fixtures::start().await;
    let mut request = accepting(&fixtures.url("consent-wrapper.html"));
    request.observe_seconds = 5;
    let outcome = support::run(request).await;
    let capture = captured(&outcome);

    assert!(!capture.consent_accepted);
    assert_eq!(console_of(capture), Vec::<&str>::new());
    let notes: Vec<(ConsentNoteKind, &str)> = capture
        .consent_notes
        .iter()
        .map(|note| (note.kind, note.banner.as_str()))
        .collect();
    assert_eq!(
        notes,
        [(
            ConsentNoteKind::DialogWithoutAccept,
            "Quantcast / InMobi Choice"
        )]
    );
}

#[tokio::test]
async fn consent_or_pay_in_danish_gets_the_consent_and_never_the_purchase() {
    let clicked = scan("consent-choice-wall-da.html", true).await;
    assert_accepted(&clicked, "button \"accepter og luk\"", "accepter og luk");
}

#[tokio::test]
async fn the_wall_is_found_behind_two_dozen_ad_frames_and_only_its_accept_pressed() {
    let fixtures = Fixtures::start().await;
    let cmp = Fixtures::start().await;
    let url = format!(
        "{}?cmp={}",
        fixtures.url("consent-many-frames.html"),
        cmp.url("")
    );
    let outcome = support::run(accepting(&url)).await;
    let capture = captured(&outcome);

    assert!(capture.consent_accepted, "{:#?}", outcome.progress);
    assert_eq!(console_of(capture), ["CHOICE accept-all trusted=true"]);
    assert!(
        outcome.progress.contains(
            &"Accepted the consent banner (Sourcepoint): pressed \"alle akzeptieren\"".to_string()
        ),
        "{:#?}",
        outcome.progress
    );
}
