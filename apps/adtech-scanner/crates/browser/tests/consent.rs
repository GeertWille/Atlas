//! Consent click-through against fake CMP banners of the common shapes.

mod support;

use adscan_model::DeviceProfile;

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

fn assert_accepted(clicked: &Clicked, banner: &str) {
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
    let line = format!("Accepted the consent banner ({banner})");
    assert!(
        clicked.progress.contains(&line),
        "missing {line:?} in {:#?}",
        clicked.progress
    );
}

#[tokio::test]
async fn accepts_a_known_cmp_banner_that_appears_late() {
    assert_accepted(&scan("consent-onetrust.html", true).await, "OneTrust");
}

#[tokio::test]
async fn accepts_a_banner_inside_shadow_dom() {
    assert_accepted(&scan("consent-shadow.html", true).await, "Usercentrics");
}

#[tokio::test]
async fn accepts_a_banner_inside_a_cross_site_iframe() {
    assert_accepted(&scan("consent-iframe.html", true).await, "Sourcepoint");
}

#[tokio::test]
async fn accepts_an_unknown_banner_by_its_dutch_accept_all_label() {
    assert_accepted(
        &scan("consent-generic.html", true).await,
        "button \"alles accepteren\"",
    );
}

#[tokio::test]
async fn short_labels_only_count_inside_a_consent_dialog() {
    assert_accepted(&scan("consent-context.html", true).await, "button \"ok\"");
}

#[tokio::test]
async fn accepts_a_google_funding_choices_dialog() {
    assert_accepted(
        &scan("consent-funding-choices.html", true).await,
        "Google Funding Choices",
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
