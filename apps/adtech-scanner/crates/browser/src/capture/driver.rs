//! The scripted visitor at work: opens the page, watches it, and for the
//! `browse` behaviour acts on it and follows links to further pages.
//!
//! ```text
//! open -> settle -> [consent] -> engage -> pick a link -> approach -> verify -> click
//!           ^                                   |                                 |
//!           +------- await the navigation <-----+---------------------------------+
//! ```
//!
//! What to do is planned in `visitor` from the seed and the page's geometry;
//! this module carries it out. Every wait records page events, so the scan is
//! cancellable at each of them.

use std::collections::HashSet;
use std::time::Duration;

use adscan_model::{
    ActivityKind, CaptureActivity, CaptureRequest, FollowedLink, TrackerEventKind, VisitorBehavior,
};
use serde_json::{Value, json};
use tokio::time::Instant;
use url::Url;

use super::error::CaptureError;
use super::injected::{hit_test_expression, page_facts_expression};
use super::navigation::{NavigationFailure, host_of, http_failure, network_failure};
use super::page::Page;
use super::recorder::epoch_ms_now;
use super::scenario::{
    ScrollAction, ScrollStep, consent_attempt_due, due_step, observation_window, scroll_plan,
    step_distance,
};
use super::visitor::links::{
    Followable, LinkFacts, Surroundings, Survey, classify_address, loggable, page_key, survey,
};
use super::visitor::motion::{WheelDevice, pointer_path, swipe_by, wheel_by};
use super::visitor::plan::{Action, Planner, dwell_per_page, settle_time, wheel_device};
use super::visitor::rng::Rng;
use super::visitor::{PageFacts, Point, Rect};

const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(30);
/// Ad-heavy pages often never fire `load`; past this point the scan goes on anyway.
const LOAD_TIMEOUT: Duration = Duration::from_secs(15);
const OBSERVATION_TICK: Duration = Duration::from_millis(100);

/// A scripted visit ends here, wherever it is.
const VISIT_DEADLINE: Duration = Duration::from_secs(8 * 60);
const LINK_ATTEMPTS_PER_PAGE: usize = 3;
/// A click that leads somewhere shows it within this time.
const NAVIGATION_SIGNAL_PATIENCE: Duration = Duration::from_secs(4);
/// A single-page app needs a moment to draw the route it changed to.
const ROUTE_SETTLE: Duration = Duration::from_millis(500);
/// The audit trail gets one entry per gesture, up to this many per page view.
const MAX_ACTIONS_REPORTED_PER_PAGE: usize = 40;
/// Scrolling towards a link gives up after this many gestures.
const MAX_APPROACH_GESTURES: usize = 12;
/// How often, and how many times, the page is asked whether it still scrolls.
const STILLNESS_INTERVAL: Duration = Duration::from_millis(120);
const STILLNESS_CHECKS: usize = 15;
/// A link that moved away under the pointer is aimed at once more.
const AIM_ATTEMPTS: usize = 2;
/// Where in the viewport a link should be before it is clicked, as shares of
/// the height: clear of sticky headers and footers.
const CLICKABLE_BAND: (f64, f64) = (0.15, 0.85);
/// Seed streams of the link choices, apart from those of the page plans.
const LINK_STREAM: u64 = 1_000;
const CLICK_HELD_MS: (f64, f64) = (50.0, 130.0);
const TAP_HELD_MS: (f64, f64) = (60.0, 120.0);
const REST_ON_LINK_MS: (f64, f64) = (80.0, 300.0);

/// Navigates and waits for the load event. A page that never finishes loading
/// is still scanned; a page that cannot be reached is an error. `proxy` names
/// the proxy the browser was told to use, for the error messages.
pub(crate) async fn open(
    page: &mut Page,
    address: &Url,
    proxy: Option<&str>,
) -> Result<(), CaptureError> {
    let host = host_of(address);
    let started = Instant::now();
    let navigation = tokio::time::timeout(
        NAVIGATION_TIMEOUT,
        page.call("Page.navigate", json!({ "url": address.as_str() })),
    )
    .await
    .map_err(|_| NavigationFailure::Timeout {
        host: host.clone(),
        seconds: NAVIGATION_TIMEOUT.as_secs(),
    })??;
    if let Some(proxy) = page.interception.failed_proxy_login() {
        return Err(NavigationFailure::ProxyLogin {
            proxy: proxy.to_string(),
        }
        .into());
    }
    if let Some(error_text) = navigation["errorText"]
        .as_str()
        .filter(|text| !text.is_empty())
    {
        // Behind a proxy that takes a login, a login the site itself asks
        // for ends the navigation with an error and no page. It is the
        // site's doing, not the proxy's.
        let site_login = page
            .interception
            .site_wanted_a_login()
            .then(|| http_failure(401, &host))
            .flatten();
        return Err(site_login
            .unwrap_or_else(|| network_failure(error_text, &host, proxy))
            .into());
    }

    let loader_id = navigation["loaderId"].as_str().unwrap_or_default();
    let loaded = wait_for_load(page, started + LOAD_TIMEOUT).await?;
    let status = page.recorder.document_status(loader_id);
    if let Some(failure) = status.and_then(|status| http_failure(status, &host)) {
        return Err(failure.into());
    }
    progress(
        page,
        if loaded {
            format!("Page loaded in {:.1}s", started.elapsed().as_secs_f64())
        } else {
            format!(
                "The page is still loading after {} seconds; scanning what has loaded",
                LOAD_TIMEOUT.as_secs()
            )
        },
    );
    Ok(())
}

/// Waits for the current document's load event and takes the page view for
/// opened either way. Returns whether the event came.
async fn wait_for_load(page: &mut Page, deadline: Instant) -> Result<bool, CaptureError> {
    let loaded = page
        .record_until_or(deadline, |page| page.recorder.load_fired)
        .await?;
    page.recorder.pages.settled();
    page.note_title().await?;
    Ok(loaded)
}

fn progress(page: &Page, message: String) {
    page.recorder.report(CaptureActivity::progress(message));
}

fn report_action(page: &Page, message: String, detail: Value) {
    page.recorder.report(CaptureActivity {
        kind: ActivityKind::Action,
        message,
        page: page.recorder.pages.current(),
        data: Some(detail),
    });
}

/// The consent banner of one page view: when it was looked for, and whether
/// it was accepted.
struct Consent {
    wanted: bool,
    page_opened: Instant,
    attempts: u32,
}

impl Consent {
    fn on_new_page(wanted: bool) -> Self {
        Self {
            wanted,
            page_opened: Instant::now(),
            attempts: 0,
        }
    }

    /// Looks for a banner if it is time to. Returns whether one was accepted.
    async fn attempt_if_due(&mut self, page: &mut Page) -> Result<bool, CaptureError> {
        if !self.wanted || !consent_attempt_due(self.page_opened.elapsed(), self.attempts) {
            return Ok(false);
        }
        self.attempts += 1;
        let Some(banner) = page.accept_consent().await? else {
            return Ok(false);
        };
        self.wanted = false;
        page.recorder.pages.consent_clicked(epoch_ms_now());
        report_action(
            page,
            format!("Accepted the consent banner ({banner})"),
            json!({ "action": "consent", "target": banner }),
        );
        Ok(true)
    }
}

/// The `none` and `scroll` behaviours: watches the loaded page for the
/// requested time, accepting the consent banner and, for `scroll`, scrolling
/// as planned. Returns whether consent was accepted.
pub(crate) async fn observe(
    page: &mut Page,
    request: &CaptureRequest,
) -> Result<bool, CaptureError> {
    let window = observation_window(request.observe_seconds);
    let plan = if request.visitor == VisitorBehavior::Scroll {
        scroll_plan(window)
    } else {
        Vec::new()
    };
    progress(
        page,
        format!("Observing the page for {} seconds", window.as_secs()),
    );

    let mut consent = Consent::on_new_page(request.accept_consent);
    let mut window_started = Instant::now();
    let mut next_step = 0;
    let mut consent_accepted = false;
    while window_started.elapsed() < window {
        if consent.attempt_if_due(page).await? {
            consent_accepted = true;
            // Bidding only starts once consent is given: observe from here.
            window_started = Instant::now();
            next_step = 0;
            continue;
        }
        match due_step(&plan, next_step, window_started.elapsed()) {
            Some(due) => {
                next_step = due + 1;
                scroll_as_planned(page, &plan[due]).await?;
            }
            None => {
                let window_ends = window_started + window;
                page.record_until(window_ends.min(Instant::now() + OBSERVATION_TICK))
                    .await?;
            }
        }
    }
    page.note_title().await?;
    Ok(consent_accepted)
}

async fn scroll_as_planned(page: &mut Page, step: &ScrollStep) -> Result<(), CaptureError> {
    match step.action {
        ScrollAction::Down { stride, steps_left } => {
            let Some(position) = page.scroll_position().await? else {
                return Ok(());
            };
            if position.remaining() < 1.0 {
                return Ok(());
            }
            let distance = step_distance(
                position.viewport_height,
                position.remaining(),
                steps_left,
                stride,
            )
            .min(position.remaining());
            page.scroll_down(distance).await?;
            let depth = position.depth_after(distance);
            report_action(
                page,
                format!("Scrolling ({:.0}%)", depth * 100.0),
                json!({ "action": "scroll", "scrollDepth": depth }),
            );
        }
        ScrollAction::BackToTop => {
            page.scroll_to_top().await?;
            report_action(
                page,
                "Scrolling back to the top".into(),
                json!({ "action": "scrollBack", "scrollDepth": 0.0 }),
            );
        }
    }
    Ok(())
}

/// The `browse` behaviour: on every page, read it the way a visitor would,
/// then follow a link to the next, up to `extra_pages` times. Returns whether
/// a consent banner was accepted.
pub(crate) async fn browse(
    page: &mut Page,
    request: &CaptureRequest,
) -> Result<bool, CaptureError> {
    let visit = Rng::new(u64::from(request.seed));
    let dwell = dwell_per_page(request.observe_seconds, request.extra_pages);
    let deadline = Instant::now() + VISIT_DEADLINE;
    progress(
        page,
        format!(
            "Browsing like a visitor: {} seconds on each of up to {} pages (seed {})",
            dwell.as_secs(),
            request.extra_pages + 1,
            request.seed
        ),
    );
    let mut visited: HashSet<String> = Url::parse(page.recorder.document_url())
        .iter()
        .map(page_key)
        .collect();
    let mut consent_accepted = false;
    for page_number in 0..=request.extra_pages {
        let reader = Reader {
            planner: Planner::new(&visit, page_number, page.emulation().touch),
            consent: Consent::on_new_page(request.accept_consent),
            dwell,
            deadline,
        };
        consent_accepted |= reader.read(page).await?;
        if page_number == request.extra_pages {
            break;
        }
        if Instant::now() >= deadline {
            page.report_blocked(
                "deadline",
                "The visit reached its time limit before all pages were visited",
                None,
            );
            break;
        }
        let mut rng = visit.fork(LINK_STREAM + u64::from(page_number));
        if !follow_a_link(page, &mut rng, wheel_device(&visit), &mut visited).await? {
            break;
        }
    }
    Ok(consent_accepted)
}

/// One page view of a `browse` visit.
struct Reader {
    planner: Planner,
    consent: Consent,
    dwell: Duration,
    deadline: Instant,
}

impl Reader {
    /// Leaves the page alone first, then acts on it as planned until the
    /// planned time fills the dwell. Returns whether consent was accepted.
    async fn read(mut self, page: &mut Page) -> Result<bool, CaptureError> {
        let dwell_ms = self.dwell.as_secs_f64() * 1_000.0;
        let mut consent_accepted = false;
        let mut settle_ends = Instant::now() + settle_time(self.dwell);
        while Instant::now() < settle_ends {
            if self.consent.attempt_if_due(page).await? {
                consent_accepted = true;
                // Bidding only starts once consent is given: settle from here.
                settle_ends = Instant::now() + settle_time(self.dwell);
            }
            page.record_until(settle_ends.min(Instant::now() + OBSERVATION_TICK))
                .await?;
        }

        let mut planned_ms = settle_time(self.dwell).as_secs_f64() * 1_000.0;
        let mut reported = 0;
        // A scroll is reported with the depth it reached, which is known
        // when the page is next measured.
        let mut scrolled: Option<&'static str> = None;
        while planned_ms < dwell_ms && Instant::now() < self.deadline {
            consent_accepted |= self.consent.attempt_if_due(page).await?;
            if scrolled.is_some() {
                // The plan depends on where things are, so the page is
                // measured at rest: the same visit then plans the same way.
                rest_until_still(page).await?;
            }
            let Some(facts) = page_facts(page, false).await? else {
                // Between two documents: nothing to act on.
                page.record_for(OBSERVATION_TICK).await?;
                planned_ms += OBSERVATION_TICK.as_secs_f64() * 1_000.0;
                continue;
            };
            let mut report = |page: &Page, message: String, detail: Value| {
                if reported < MAX_ACTIONS_REPORTED_PER_PAGE {
                    reported += 1;
                    report_action(page, message, detail);
                }
            };
            if let Some(gesture) = scrolled.take() {
                let (message, detail) = scroll_report(gesture, facts.scroll_depth());
                report(page, message, detail);
            }
            let action = self.planner.next(&facts.state(page.pointer));
            planned_ms += action.planned_ms();
            match &action {
                Action::Idle { ms } | Action::Hover { ms } => {
                    page.record_for(Duration::from_secs_f64(ms / 1_000.0))
                        .await?;
                }
                Action::MovePointer { path, target } => {
                    page.move_pointer(path).await?;
                    let over = facts.targets[*target].kind.as_str();
                    report(
                        page,
                        format!("Moved the pointer over {}", with_article(over)),
                        json!({ "action": "pointerMove", "target": over }),
                    );
                }
                Action::Wheel { ticks } => {
                    page.wheel(ticks).await?;
                    let back = ticks.first().is_some_and(|tick| tick.delta_y < 0.0);
                    scrolled = Some(if back { "scrollBack" } else { "scroll" });
                }
                Action::Swipe { path } => {
                    page.swipe(path).await?;
                    scrolled = Some("swipe");
                }
            }
        }
        if let (Some(gesture), Some(facts)) = (scrolled, page_facts(page, false).await?) {
            let (message, detail) = scroll_report(gesture, facts.scroll_depth());
            if reported < MAX_ACTIONS_REPORTED_PER_PAGE {
                report_action(page, message, detail);
            }
        }
        page.note_title().await?;
        Ok(consent_accepted)
    }
}

fn scroll_report(gesture: &'static str, depth: f64) -> (String, Value) {
    let percent = (depth * 100.0).round();
    let message = match gesture {
        "scrollBack" => format!("Scrolled back up to {percent}%"),
        "swipe" => format!("Swiped to {percent}%"),
        _ => format!("Scrolled to {percent}%"),
    };
    (message, json!({ "action": gesture, "scrollDepth": depth }))
}

fn with_article(noun: &str) -> String {
    let article = if noun.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    };
    format!("{article} {noun}")
}

/// What the page says about itself right now. `None` while it has no
/// document to ask.
async fn page_facts(page: &mut Page, links: bool) -> Result<Option<PageFacts>, CaptureError> {
    // On a page of another site nothing of the scanner's is done.
    if !page.recorder.is_recording() {
        return Ok(None);
    }
    let slot_ids = ad_container_ids(page);
    let facts = page
        .evaluate(&page_facts_expression(&slot_ids, links))
        .await?;
    Ok(facts.and_then(|facts| serde_json::from_value(facts).ok()))
}

/// The elements the tracker saw the page's ad stack use as ad containers.
fn ad_container_ids(page: &Page) -> Vec<String> {
    let mut ids: Vec<String> = page
        .recorder
        .tracker
        .events()
        .iter()
        .flat_map(|event| match &event.kind {
            TrackerEventKind::SlotDefined { element_id, .. } => vec![element_id.clone()],
            TrackerEventKind::AuctionInit { ad_units, .. } => {
                ad_units.iter().map(|unit| unit.code.clone()).collect()
            }
            _ => Vec::new(),
        })
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Picks a link of the page and clicks it, trying the next best when one
/// leads nowhere. Returns whether the visit is on a new page view.
async fn follow_a_link(
    page: &mut Page,
    rng: &mut Rng,
    wheel: WheelDevice,
    visited: &mut HashSet<String>,
) -> Result<bool, CaptureError> {
    for _ in 0..LINK_ATTEMPTS_PER_PAGE {
        let (Some(facts), Ok(current), Some(site)) = (
            page_facts(page, true).await?,
            Url::parse(page.recorder.document_url()),
            page.recorder.site().cloned(),
        ) else {
            return Ok(false);
        };
        let surroundings = Surroundings {
            current: &current,
            site: &site,
            visited,
            viewport: facts.viewport,
        };
        let found = survey(rng, &facts.links, &surroundings);
        report_link_choice(page, &found, &facts.links);
        let Some(chosen) = found.chosen.map(|chosen| &found.shortlist[chosen]) else {
            return Ok(false);
        };
        let link = &facts.links[chosen.index];
        let followed = FollowedLink {
            text: link.text.clone(),
            href: loggable(&chosen.url),
            reason: reason_for(chosen, found.shortlist.len()),
        };
        // Tried is tried: a link that leads nowhere is not picked again.
        let mut tried = visited.clone();
        tried.insert(page_key(&chosen.url));
        let outcome = click_link(page, rng, wheel, chosen, link, followed, &tried).await?;
        *visited = tried;
        match outcome {
            Clicked::Arrived => {
                visited.extend(
                    Url::parse(page.recorder.document_url())
                        .iter()
                        .map(page_key),
                );
                return Ok(true);
            }
            Clicked::Refused => return Ok(false),
            Clicked::LedNowhere => {}
        }
    }
    Ok(false)
}

fn reason_for(chosen: &Followable, shortlisted: usize) -> String {
    let because = if chosen.reasons.is_empty() {
        "a link of the page".to_string()
    } else {
        chosen.reasons.join(", ")
    };
    format!("{because}; chosen among the best {shortlisted}")
}

fn report_link_choice(page: &Page, found: &Survey, links: &[LinkFacts]) {
    let describe = |link: &Followable| {
        json!({
            "text": links[link.index].text,
            "href": loggable(&link.url),
            "score": link.score,
        })
    };
    let chosen = found.chosen.map(|chosen| &found.shortlist[chosen]);
    let alternatives: Vec<Value> = found
        .shortlist
        .iter()
        .enumerate()
        .filter(|(index, _)| Some(*index) != found.chosen)
        .map(|(_, link)| describe(link))
        .collect();
    let skipped: serde_json::Map<String, Value> = found
        .skipped
        .iter()
        .map(|(reason, count)| (reason.label().to_string(), json!(count)))
        .collect();
    let message = match chosen {
        Some(link) => format!(
            "Following link: {} (best of {})",
            link.url.path(),
            found.shortlist.len()
        ),
        None => "No link to follow".to_string(),
    };
    page.recorder.report(CaptureActivity {
        kind: ActivityKind::LinkChoice,
        message,
        page: page.recorder.pages.current(),
        data: Some(json!({
            "chosen": chosen.map(|link| {
                let mut chosen = describe(link);
                chosen["reasons"] = json!(link.reasons);
                chosen
            }),
            "alternatives": alternatives,
            "skipped": skipped,
        })),
    });
}

enum Clicked {
    /// The visit is on a new page view.
    Arrived,
    /// The navigation was refused (it left the site, submitted a form or
    /// went to an address the link policy rules out); the visit follows
    /// nothing more.
    Refused,
    LedNowhere,
}

/// What the scan knew of the page's navigations before a click.
struct Before {
    page_views: usize,
    document_requests: u32,
    refusals: u32,
}

impl Before {
    fn of(page: &Page) -> Self {
        Self {
            page_views: page.recorder.pages.len(),
            document_requests: page.recorder.document_requests,
            refusals: page.interception.refusals(),
        }
    }

    fn was_refused(&self, page: &Page) -> bool {
        page.interception.refusals() > self.refusals
    }

    fn opened_a_page_view(&self, page: &Page) -> bool {
        page.recorder.pages.len() > self.page_views
    }

    fn requested_a_document(&self, page: &Page) -> bool {
        page.recorder.document_requests > self.document_requests
    }
}

async fn click_link(
    page: &mut Page,
    rng: &mut Rng,
    wheel: WheelDevice,
    chosen: &Followable,
    link: &LinkFacts,
    followed: FollowedLink,
    tried: &HashSet<String>,
) -> Result<Clicked, CaptureError> {
    let touch = page.emulation().touch;
    let mut aim = None;
    for _ in 0..AIM_ATTEMPTS {
        let Some((rect, facts)) = approach(page, rng, wheel, chosen, link).await? else {
            page.report_blocked(
                "dead-link",
                "The chosen link could not be brought into view",
                Some(chosen.url.as_str()),
            );
            return Ok(Clicked::LedNowhere);
        };
        let at = facts
            .viewport
            .clamp(rect.point_at(rng.between(0.2, 0.8), rng.between(0.3, 0.7)));
        if !touch {
            let path = pointer_path(rng, page.pointer, at, rect.width, facts.viewport);
            page.move_pointer(&path).await?;
            let rest = rng.between(REST_ON_LINK_MS.0, REST_ON_LINK_MS.1);
            page.record_for(Duration::from_secs_f64(rest / 1_000.0))
                .await?;
        }
        if is_still_the_link(page, at, chosen, tried, &facts).await? {
            aim = Some(at);
            break;
        }
    }
    let Some(at) = aim else {
        page.report_blocked(
            "hit-test",
            "Something else is where the chosen link was; it was not clicked",
            Some(chosen.url.as_str()),
        );
        return Ok(Clicked::LedNowhere);
    };

    let before = Before::of(page);
    page.recorder.pages.announce(followed);
    page.expect_tab();
    page.download_began = false;
    if touch {
        let held = rng.between(TAP_HELD_MS.0, TAP_HELD_MS.1);
        page.tap(at, Duration::from_secs_f64(held / 1_000.0))
            .await?;
    } else {
        let held = rng.between(CLICK_HELD_MS.0, CLICK_HELD_MS.1);
        page.click(at, Duration::from_secs_f64(held / 1_000.0))
            .await?;
    }
    report_action(
        page,
        format!(
            "{} the link \"{}\"",
            if touch { "Tapped" } else { "Clicked" },
            link.text
        ),
        json!({ "action": if touch { "tap" } else { "click" }, "target": "link" }),
    );
    let outcome = await_navigation(page, &before, chosen).await;
    page.recorder.pages.withdraw_announcement();
    outcome
}

/// Pages move, and some rewrite a link as the pointer arrives: what would be
/// clicked is what is under the point now, judged again.
async fn is_still_the_link(
    page: &mut Page,
    at: Point,
    chosen: &Followable,
    tried: &HashSet<String>,
    facts: &PageFacts,
) -> Result<bool, CaptureError> {
    let under_the_point = page.evaluate(&hit_test_expression(at)).await?;
    let Some(now) = under_the_point
        .as_ref()
        .and_then(|hit| Url::parse(hit["href"].as_str()?).ok())
    else {
        return Ok(false);
    };
    let (Ok(current), Some(site)) = (
        Url::parse(page.recorder.document_url()),
        page.recorder.site(),
    ) else {
        return Ok(false);
    };
    let surroundings = Surroundings {
        current: &current,
        site,
        visited: tried,
        viewport: facts.viewport,
    };
    Ok(page_key(&now) == page_key(&chosen.url) && classify_address(&now, &surroundings).is_ok())
}

/// Waits for the page to stop scrolling: gestures are animated, and a link
/// measured in motion is somewhere else by the time the pointer gets there.
async fn rest_until_still(page: &mut Page) -> Result<(), CaptureError> {
    let mut last = page.scroll_position().await?.map(|position| position.top);
    for _ in 0..STILLNESS_CHECKS {
        page.record_for(STILLNESS_INTERVAL).await?;
        let top = page.scroll_position().await?.map(|position| position.top);
        if top == last {
            break;
        }
        last = top;
    }
    Ok(())
}

/// Scrolls until the link is where it can be clicked. Returns where it is
/// then, or `None` when it cannot be brought there.
async fn approach(
    page: &mut Page,
    rng: &mut Rng,
    wheel: WheelDevice,
    chosen: &Followable,
    link: &LinkFacts,
) -> Result<Option<(Rect, PageFacts)>, CaptureError> {
    let touch = page.emulation().touch;
    for _ in 0..MAX_APPROACH_GESTURES {
        rest_until_still(page).await?;
        let Some(facts) = page_facts(page, true).await? else {
            return Ok(None);
        };
        // The link as it is now: one that was hidden or covered by an ad
        // since it was chosen is not one to click.
        let still_followable = || {
            facts
                .links
                .iter()
                .filter(|candidate| candidate.visible && !candidate.in_ad)
        };
        let Some(rect) = still_followable()
            .find(|candidate| candidate.href == link.href && candidate.text == link.text)
            .or_else(|| {
                let same_page = |candidate: &&LinkFacts| {
                    Url::parse(&candidate.href)
                        .is_ok_and(|url| page_key(&url) == page_key(&chosen.url))
                };
                still_followable().find(same_page)
            })
            .map(|candidate| candidate.rect)
        else {
            return Ok(None);
        };
        let height = facts.viewport.height;
        let band = CLICKABLE_BAND.0 * height..=CLICKABLE_BAND.1 * height;
        let middle = rect.center().y;
        let cannot_scroll_further = (middle < *band.start() && facts.scroll_top < 1.0)
            || (middle > *band.end() && facts.page_height - height - facts.scroll_top < 1.0);
        if band.contains(&middle) || (cannot_scroll_further && rect.is_inside(facts.viewport)) {
            return Ok(Some((rect, facts)));
        }
        if cannot_scroll_further {
            return Ok(None);
        }
        let distance = middle - height / 2.0;
        if touch {
            page.swipe(&swipe_by(rng, facts.viewport, distance)).await?;
        } else {
            page.wheel(&wheel_by(rng, wheel, distance)).await?;
        }
    }
    Ok(None)
}

/// Waits for what the click did and, when it leads to a new document, for
/// that document to load.
async fn await_navigation(
    page: &mut Page,
    before: &Before,
    chosen: &Followable,
) -> Result<Clicked, CaptureError> {
    let shows_an_outcome = |page: &Page| {
        before.opened_a_page_view(page) || before.was_refused(page) || page.download_began
    };
    let signalled = page
        .record_until_or(Instant::now() + NAVIGATION_SIGNAL_PATIENCE, |page| {
            shows_an_outcome(page) || before.requested_a_document(page)
        })
        .await?;
    if signalled && !shows_an_outcome(page) {
        // A document is on its way. It may still be redirected off the site.
        page.record_until_or(Instant::now() + NAVIGATION_TIMEOUT, shows_an_outcome)
            .await?;
    }
    if before.was_refused(page) {
        return Ok(Clicked::Refused);
    }
    if page.download_began {
        page.report_blocked(
            "download",
            "The link started a download instead of opening a page",
            Some(chosen.url.as_str()),
        );
        return Ok(Clicked::LedNowhere);
    }
    if !before.opened_a_page_view(page) {
        page.report_blocked(
            "dead-link",
            "The click did not lead to another page",
            Some(chosen.url.as_str()),
        );
        return Ok(Clicked::LedNowhere);
    }
    if before.requested_a_document(page) {
        wait_for_load(page, Instant::now() + LOAD_TIMEOUT).await?;
    } else {
        page.record_for(ROUTE_SETTLE).await?;
        page.note_title().await?;
    }
    Ok(Clicked::Arrived)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_gestures_are_reported_with_the_depth_they_reached() {
        assert_eq!(
            scroll_report("scroll", 0.604),
            (
                "Scrolled to 60%".to_string(),
                json!({ "action": "scroll", "scrollDepth": 0.604 })
            )
        );
        assert_eq!(
            scroll_report("scrollBack", 0.25).0,
            "Scrolled back up to 25%"
        );
        assert_eq!(scroll_report("swipe", 1.0).0, "Swiped to 100%");
    }

    #[test]
    fn a_link_choice_is_explained_by_what_spoke_for_the_link() {
        let link = Followable {
            index: 0,
            url: Url::parse("https://news.example/climate/kelp").unwrap(),
            score: 6,
            reasons: vec!["in the main content", "article-like path"],
        };
        assert_eq!(
            reason_for(&link, 5),
            "in the main content, article-like path; chosen among the best 5"
        );
        let plain = Followable {
            reasons: Vec::new(),
            ..link
        };
        assert_eq!(
            reason_for(&plain, 1),
            "a link of the page; chosen among the best 1"
        );
        assert_eq!(with_article("image"), "an image");
        assert_eq!(with_article("paragraph"), "a paragraph");
    }
}
