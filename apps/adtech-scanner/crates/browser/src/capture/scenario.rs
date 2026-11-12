//! What the scanner does while it watches a page: when it scrolls, how far, and
//! when it tries the consent banner. Pure planning, no I/O.

use std::time::Duration;

/// Share of the observation window spent idle before the first scroll. A page
/// that only bids after a scroll must be seen *not* bidding first.
const IDLE_SHARE: f64 = 0.35;
/// Scrolling down ends here; the page is then returned to the top so sticky
/// and above-the-fold slots can be observed again.
const RETURN_SHARE: f64 = 0.85;

/// Pauses between scroll steps and the relative length of each step. Uneven on
/// purpose: readers do not scroll like a metronome.
const PAUSES_MS: [u64; 5] = [650, 900, 550, 1_100, 750];
const STRIDES: [f64; 5] = [1.0, 0.8, 1.15, 0.9, 1.05];

/// Time the last scroll needs to finish animating before the page goes back up.
const SETTLE: Duration = Duration::from_millis(700);

const MIN_STEP_VIEWPORTS: f64 = 0.35;
const MAX_STEP_VIEWPORTS: f64 = 1.25;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ScrollAction {
    /// Scroll down one step. `steps_left` counts this step.
    Down {
        stride: f64,
        steps_left: usize,
    },
    BackToTop,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ScrollStep {
    /// Offset from the start of the observation window.
    pub at: Duration,
    pub action: ScrollAction,
}

pub(crate) fn observation_window(observe_seconds: u32) -> Duration {
    Duration::from_secs(u64::from(observe_seconds.max(1)))
}

/// The scroll steps of one observation window, in order.
pub(crate) fn scroll_plan(window: Duration) -> Vec<ScrollStep> {
    let return_at = window.mul_f64(RETURN_SHARE);
    let mut times = Vec::new();
    let mut at = window.mul_f64(IDLE_SHARE);
    while times.is_empty() || at + SETTLE <= return_at {
        times.push(at);
        at += Duration::from_millis(PAUSES_MS[times.len() % PAUSES_MS.len()]);
    }
    let total = times.len();
    times
        .into_iter()
        .enumerate()
        .map(|(index, at)| ScrollStep {
            at,
            action: ScrollAction::Down {
                stride: STRIDES[index % STRIDES.len()],
                steps_left: total - index,
            },
        })
        .chain([ScrollStep {
            at: return_at,
            action: ScrollAction::BackToTop,
        }])
        .collect()
}

/// The step to perform `elapsed` into the window: the latest one that is due.
/// Steps that fell behind (a slow page, long swipes) are skipped rather than
/// replayed late, so the return to the top still happens on time.
pub(crate) fn due_step(plan: &[ScrollStep], next: usize, elapsed: Duration) -> Option<usize> {
    let due = plan
        .iter()
        .skip(next)
        .take_while(|step| step.at <= elapsed)
        .count();
    (due > 0).then(|| next + due - 1)
}

/// Pixels to scroll for one step: paced to reach the bottom by the last step,
/// but never faster or slower than a person would plausibly scroll.
pub(crate) fn step_distance(
    viewport_height: f64,
    remaining: f64,
    steps_left: usize,
    stride: f64,
) -> f64 {
    let even_share = remaining / steps_left.max(1) as f64;
    let paced = even_share.clamp(
        MIN_STEP_VIEWPORTS * viewport_height,
        MAX_STEP_VIEWPORTS * viewport_height,
    );
    (paced * stride).round()
}

/// A thumb covers about this share of the screen in one swipe.
const MAX_SWIPE_VIEWPORTS: f64 = 0.6;

/// Splits a scroll distance into the finger swipes that cover it on a touch screen.
pub(crate) fn swipe_lengths(distance: f64, viewport_height: f64) -> Vec<f64> {
    let longest = (MAX_SWIPE_VIEWPORTS * viewport_height).max(1.0);
    let swipes = (distance / longest).ceil().max(1.0);
    vec![distance / swipes; swipes as usize]
}

const CONSENT_ATTEMPT_INTERVAL: Duration = Duration::from_millis(500);
/// Most banners are there within ten seconds of the page; that long, they
/// are looked for twice a second.
const CONSENT_EAGER_ATTEMPTS: u32 = 20;
/// A consent wall can come seconds later still (a slow tag manager, a
/// platform that waits for the page to settle): after the eager attempts it
/// is looked for at this pace for as long as the page is watched.
const CONSENT_PATIENT_INTERVAL: Duration = Duration::from_secs(2);

/// Whether to look for a consent banner again, `since_load` after the page loaded.
pub(crate) fn consent_attempt_due(since_load: Duration, attempts_made: u32) -> bool {
    let eager = attempts_made.min(CONSENT_EAGER_ATTEMPTS);
    let patient = attempts_made - eager;
    since_load >= CONSENT_ATTEMPT_INTERVAL * eager + CONSENT_PATIENT_INTERVAL * patient
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_idles_first_scrolls_down_then_returns_to_top() {
        let window = observation_window(20);
        let plan = scroll_plan(window);

        assert_eq!(plan.first().unwrap().at, Duration::from_secs(7));
        let (last, downs) = plan.split_last().unwrap();
        assert_eq!(
            *last,
            ScrollStep {
                at: Duration::from_secs(17),
                action: ScrollAction::BackToTop
            }
        );
        assert!(downs.len() >= 10);
        assert!(
            downs
                .iter()
                .all(|step| matches!(step.action, ScrollAction::Down { .. }))
        );
        assert!(plan.windows(2).all(|pair| pair[0].at < pair[1].at));
        assert!(plan.iter().all(|step| step.at < window));
    }

    #[test]
    fn steps_left_counts_down_to_one() {
        let plan = scroll_plan(observation_window(10));
        let lefts: Vec<usize> = plan
            .iter()
            .filter_map(|step| match step.action {
                ScrollAction::Down { steps_left, .. } => Some(steps_left),
                ScrollAction::BackToTop => None,
            })
            .collect();
        let expected: Vec<usize> = (1..=lefts.len()).rev().collect();
        assert_eq!(lefts, expected);
    }

    #[test]
    fn overdue_steps_are_skipped_in_favour_of_the_latest() {
        let plan = scroll_plan(observation_window(20));
        assert_eq!(due_step(&plan, 0, Duration::from_secs(1)), None);
        assert_eq!(due_step(&plan, 0, plan[0].at), Some(0));
        assert_eq!(due_step(&plan, 1, plan[0].at), None);
        assert_eq!(due_step(&plan, 1, plan[3].at), Some(3));
        assert_eq!(
            due_step(&plan, 2, Duration::from_secs(60)),
            Some(plan.len() - 1)
        );
        assert_eq!(due_step(&plan, plan.len(), Duration::from_secs(60)), None);
    }

    #[test]
    fn shortest_window_still_scrolls_once() {
        let plan = scroll_plan(observation_window(0));
        assert_eq!(plan.len(), 2);
        assert!(matches!(
            plan[0].action,
            ScrollAction::Down { steps_left: 1, .. }
        ));
        assert_eq!(plan[1].action, ScrollAction::BackToTop);
    }

    #[test]
    fn step_distance_is_paced_to_the_page_but_stays_human() {
        assert_eq!(step_distance(900.0, 4_500.0, 5, 1.0), 900.0);
        assert_eq!(step_distance(900.0, 4_500.0, 5, 0.8), 720.0);
        assert_eq!(step_distance(900.0, 90_000.0, 5, 1.0), 1_125.0);
        assert_eq!(step_distance(900.0, 100.0, 5, 1.0), 315.0);
        assert_eq!(step_distance(900.0, 0.0, 0, 1.0), 315.0);
    }

    #[test]
    fn long_scrolls_become_several_equal_swipes() {
        assert_eq!(swipe_lengths(300.0, 1_000.0), vec![300.0]);
        assert_eq!(swipe_lengths(600.0, 1_000.0), vec![600.0]);
        assert_eq!(swipe_lengths(900.0, 1_000.0), vec![450.0, 450.0]);
        assert_eq!(swipe_lengths(1_500.0, 1_000.0), vec![500.0; 3]);
        assert_eq!(swipe_lengths(0.0, 1_000.0), vec![0.0]);
    }

    #[test]
    fn consent_is_looked_for_twice_a_second_for_ten_seconds_and_every_two_seconds_after() {
        assert!(consent_attempt_due(Duration::ZERO, 0));
        assert!(!consent_attempt_due(Duration::from_millis(400), 1));
        assert!(consent_attempt_due(Duration::from_millis(500), 1));
        assert!(consent_attempt_due(Duration::from_secs(9), 3));
        assert!(!consent_attempt_due(
            Duration::from_millis(9_900),
            CONSENT_EAGER_ATTEMPTS
        ));
        assert!(consent_attempt_due(
            Duration::from_secs(10),
            CONSENT_EAGER_ATTEMPTS
        ));
        // A wall that only comes after half a minute is still found, without
        // asking the page twice a second for all that time.
        assert!(!consent_attempt_due(
            Duration::from_secs(11),
            CONSENT_EAGER_ATTEMPTS + 1
        ));
        assert!(consent_attempt_due(
            Duration::from_secs(12),
            CONSENT_EAGER_ATTEMPTS + 1
        ));
        assert!(consent_attempt_due(
            Duration::from_secs(60),
            CONSENT_EAGER_ATTEMPTS + 25
        ));
    }
}
