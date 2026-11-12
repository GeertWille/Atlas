//! What a scripted visitor does on a page, one action after the other. The
//! sequence is a function of the seed and of the page's geometry, never of
//! how long the browser took, so a visit can be repeated.

use std::time::Duration;

use super::motion::{
    Waypoint, WheelDevice, WheelTick, pointer_path, swipe_path, wheel_by, wheel_onward,
};
use super::rng::Rng;
use super::{Point, Rect, Size};

/// A whole scripted visit may take this long, however many pages it has.
const VISIT_BUDGET: Duration = Duration::from_secs(300);
/// A page is watched untouched this long before the visitor does anything:
/// what an ad stack does without any input has to be seen first.
const MIN_SETTLE: Duration = Duration::from_secs(2);
const SETTLE_SHARE: f64 = 0.15;

/// Time spent on each page of a visit to `1 + extra_pages` pages.
pub(crate) fn dwell_per_page(observe_seconds: u32, extra_pages: u32) -> Duration {
    Duration::from_secs(u64::from(observe_seconds.max(1))).min(VISIT_BUDGET / (1 + extra_pages))
}

/// The untouched part at the start of a page's dwell.
pub(crate) fn settle_time(dwell: Duration) -> Duration {
    dwell.mul_f64(SETTLE_SHARE).max(MIN_SETTLE).min(dwell)
}

/// What the visitor can see and where its pointer is.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PageState {
    pub viewport: Size,
    pub scroll_top: f64,
    pub page_height: f64,
    pub pointer: Point,
    /// Paragraphs, headings and images in view; never anything inside an ad.
    pub reading_targets: Vec<Rect>,
}

impl PageState {
    fn left_to_scroll(&self) -> f64 {
        (self.page_height - self.viewport.height - self.scroll_top).max(0.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Action {
    /// Reading: nothing happens.
    Idle { ms: f64 },
    /// To `reading_targets[target]`.
    MovePointer { path: Vec<Waypoint>, target: usize },
    /// The pointer rests where the last movement took it.
    Hover { ms: f64 },
    /// Wheel events at the pointer's position.
    Wheel { ticks: Vec<WheelTick> },
    /// One finger swipe along `path`.
    Swipe { path: Vec<Waypoint> },
}

impl Action {
    /// How long the action is meant to take. The visit's clock runs on this,
    /// not on the time the browser needed.
    pub(crate) fn planned_ms(&self) -> f64 {
        match self {
            Action::Idle { ms } | Action::Hover { ms } => *ms,
            Action::MovePointer { path, .. } | Action::Swipe { path } => {
                path.last().map_or(0.0, |end| end.at_ms)
            }
            Action::Wheel { ticks } => ticks.iter().map(|tick| tick.gap_ms).sum(),
        }
    }
}

const READING_PAUSE_MEDIAN_MS: f64 = 1_200.0;
const READING_PAUSE_SPREAD: f64 = 0.6;
const READING_PAUSE_CAP_MS: f64 = 6_000.0;
const HOVER_MS: (f64, f64) = (150.0, 900.0);
/// Share of scroll gestures that go back up, and how far, in viewports.
const SCROLL_BACK_SHARE: f64 = 0.12;
const SCROLL_BACK_VIEWPORTS: (f64, f64) = (0.2, 0.6);
/// How often a reader with a mouse scrolls, moves the pointer or just reads.
const SCROLL_WEIGHT: f64 = 5.0;
const POINT_WEIGHT: f64 = 3.0;
const READ_WEIGHT: f64 = 1.5;

/// What the visitor scrolls with on every page of the visit.
pub(crate) fn wheel_device(visit: &Rng) -> WheelDevice {
    WheelDevice::draw(&mut visit.clone())
}

/// Plans the actions of one page view.
pub(crate) struct Planner {
    rng: Rng,
    touch: bool,
    wheel: WheelDevice,
    /// What has to follow the last action: a pause after a scroll gesture, a
    /// rest after a pointer movement.
    owed: Option<Action>,
}

impl Planner {
    /// `visit` is the visit's seed stream; `page` keeps pages independent, so
    /// a longer first page does not change what happens on the second.
    pub(crate) fn new(visit: &Rng, page: u32, touch: bool) -> Self {
        Self {
            rng: visit.fork(u64::from(page) + 1),
            touch,
            wheel: wheel_device(visit),
            owed: None,
        }
    }

    pub(crate) fn next(&mut self, page: &PageState) -> Action {
        if let Some(owed) = self.owed.take() {
            return owed;
        }
        if self.touch {
            self.next_touch(page)
        } else {
            self.next_pointer(page)
        }
    }

    fn next_pointer(&mut self, page: &PageState) -> Action {
        let can_scroll_on = page.left_to_scroll() >= 1.0;
        let can_scroll_back = page.scroll_top > SCROLL_BACK_VIEWPORTS.0 * page.viewport.height;
        let weights = [
            if can_scroll_on {
                SCROLL_WEIGHT * (1.0 - SCROLL_BACK_SHARE)
            } else {
                0.0
            },
            if can_scroll_back {
                SCROLL_WEIGHT * SCROLL_BACK_SHARE
            } else {
                0.0
            },
            if page.reading_targets.is_empty() {
                0.0
            } else {
                POINT_WEIGHT
            },
            READ_WEIGHT,
        ];
        match self.rng.weighted(&weights) {
            Some(0) => {
                self.owed = Some(self.reading_pause());
                Action::Wheel {
                    ticks: wheel_onward(&mut self.rng, self.wheel),
                }
            }
            Some(1) => {
                let distance = self.scroll_back_distance(page);
                self.owed = Some(self.reading_pause());
                Action::Wheel {
                    ticks: wheel_by(&mut self.rng, self.wheel, -distance),
                }
            }
            Some(2) => {
                let target = self.rng.whole(0, page.reading_targets.len() as u32 - 1) as usize;
                let rect = page.reading_targets[target];
                let to = rect.point_at(self.rng.between(0.15, 0.85), self.rng.between(0.2, 0.8));
                self.owed = Some(Action::Hover {
                    ms: self.rng.between(HOVER_MS.0, HOVER_MS.1).round(),
                });
                Action::MovePointer {
                    path: pointer_path(&mut self.rng, page.pointer, to, rect.width, page.viewport),
                    target,
                }
            }
            _ => self.reading_pause(),
        }
    }

    fn next_touch(&mut self, page: &PageState) -> Action {
        let can_scroll_on = page.left_to_scroll() >= 1.0;
        let can_scroll_back = page.scroll_top > SCROLL_BACK_VIEWPORTS.0 * page.viewport.height;
        let weights = [
            if can_scroll_on {
                1.0 - SCROLL_BACK_SHARE
            } else {
                0.0
            },
            if can_scroll_back {
                SCROLL_BACK_SHARE
            } else {
                0.0
            },
        ];
        let Some(direction) = self.rng.weighted(&weights) else {
            return self.reading_pause();
        };
        self.owed = Some(self.reading_pause());
        Action::Swipe {
            path: swipe_path(&mut self.rng, page.viewport, direction == 0),
        }
    }

    fn reading_pause(&mut self) -> Action {
        Action::Idle {
            ms: self
                .rng
                .log_normal(
                    READING_PAUSE_MEDIAN_MS,
                    READING_PAUSE_SPREAD,
                    READING_PAUSE_CAP_MS,
                )
                .round(),
        }
    }

    fn scroll_back_distance(&mut self, page: &PageState) -> f64 {
        let viewports = self
            .rng
            .between(SCROLL_BACK_VIEWPORTS.0, SCROLL_BACK_VIEWPORTS.1);
        (viewports * page.viewport.height).min(page.scroll_top)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESKTOP: Size = Size {
        width: 1440.0,
        height: 900.0,
    };
    const PHONE: Size = Size {
        width: 412.0,
        height: 915.0,
    };

    fn article(viewport: Size) -> PageState {
        PageState {
            viewport,
            scroll_top: 0.0,
            page_height: 6_000.0,
            pointer: Point { x: 700.0, y: 400.0 },
            reading_targets: vec![
                Rect {
                    x: 200.0,
                    y: 120.0,
                    width: 640.0,
                    height: 90.0,
                },
                Rect {
                    x: 200.0,
                    y: 300.0,
                    width: 640.0,
                    height: 180.0,
                },
                Rect {
                    x: 900.0,
                    y: 200.0,
                    width: 300.0,
                    height: 250.0,
                },
            ],
        }
    }

    /// Plays a plan against a page that scrolls as asked, the way the driver
    /// does against the real one, until `dwell_ms` of planned time is used.
    fn visit(
        seed: u64,
        page_index: u32,
        mut page: PageState,
        touch: bool,
        dwell_ms: f64,
    ) -> Vec<Action> {
        let mut planner = Planner::new(&Rng::new(seed), page_index, touch);
        let mut actions = Vec::new();
        let mut planned = 0.0;
        while planned < dwell_ms {
            let action = planner.next(&page);
            planned += action.planned_ms();
            let scrolled = match &action {
                Action::Wheel { ticks } => ticks.iter().map(|tick| tick.delta_y).sum(),
                Action::Swipe { path } => path[0].point.y - path.last().unwrap().point.y,
                Action::MovePointer { path, .. } => {
                    page.pointer = path.last().unwrap().point;
                    0.0
                }
                Action::Idle { .. } | Action::Hover { .. } => 0.0,
            };
            let lowest = page.page_height - page.viewport.height;
            page.scroll_top = (page.scroll_top + scrolled).clamp(0.0, lowest);
            actions.push(action);
        }
        actions
    }

    #[test]
    fn the_same_seed_and_page_give_the_same_actions() {
        let first = visit(2026, 0, article(DESKTOP), false, 30_000.0);
        let again = visit(2026, 0, article(DESKTOP), false, 30_000.0);
        assert_eq!(first, again);
        assert!(first.len() > 10);
    }

    #[test]
    fn another_seed_or_another_page_of_the_visit_acts_differently() {
        let first = visit(2026, 0, article(DESKTOP), false, 30_000.0);
        assert_ne!(first, visit(2027, 0, article(DESKTOP), false, 30_000.0));
        assert_ne!(first, visit(2026, 1, article(DESKTOP), false, 30_000.0));
    }

    #[test]
    fn a_longer_dwell_only_adds_actions_at_the_end() {
        let short = visit(9, 0, article(DESKTOP), false, 10_000.0);
        let long = visit(9, 0, article(DESKTOP), false, 25_000.0);
        assert_eq!(long[..short.len()], short[..]);
    }

    #[test]
    fn planned_time_stops_at_the_dwell_plus_at_most_one_action() {
        for seed in 0..50 {
            let actions = visit(seed, 0, article(DESKTOP), false, 20_000.0);
            let (last, before) = actions.split_last().unwrap();
            let planned_before: f64 = before.iter().map(Action::planned_ms).sum();
            assert!(planned_before < 20_000.0);
            assert!(planned_before + last.planned_ms() >= 20_000.0);
            assert!(last.planned_ms() <= READING_PAUSE_CAP_MS);
        }
    }

    #[test]
    fn a_reader_with_a_mouse_scrolls_points_rests_and_reads() {
        let mut wheels = 0;
        let mut back = 0;
        // Long enough that no visit reaches the bottom, where going back up
        // is the only way left to scroll.
        let mut endless = article(DESKTOP);
        endless.page_height = 400_000.0;
        for seed in 0..40 {
            let actions = visit(seed, 0, endless.clone(), false, 60_000.0);
            for pair in actions.windows(2) {
                match &pair[0] {
                    Action::Wheel { ticks } => {
                        wheels += 1;
                        back += usize::from(ticks[0].delta_y < 0.0);
                        assert!(
                            matches!(pair[1], Action::Idle { .. }),
                            "a pause follows a scroll"
                        );
                    }
                    Action::MovePointer { path, target } => {
                        assert!(
                            matches!(pair[1], Action::Hover { ms } if (150.0..=900.0).contains(&ms))
                        );
                        let end = path.last().unwrap().point;
                        let aimed_at = endless.reading_targets[*target];
                        let on_target = (aimed_at.x..=aimed_at.x + aimed_at.width).contains(&end.x)
                            && (aimed_at.y..=aimed_at.y + aimed_at.height).contains(&end.y);
                        assert!(on_target, "{end:?}");
                    }
                    Action::Swipe { .. } => panic!("a mouse does not swipe"),
                    Action::Idle { ms } => assert!(*ms <= READING_PAUSE_CAP_MS),
                    Action::Hover { .. } => {}
                }
            }
        }
        let share = back as f64 / wheels as f64;
        assert!(
            (0.05..0.2).contains(&share),
            "{back} of {wheels} gestures went back"
        );
    }

    #[test]
    fn one_scroll_device_is_used_for_the_whole_visit() {
        let is_mouse = |action: &Action| match action {
            Action::Wheel { ticks } => Some(ticks[0].delta_y.abs() == 100.0 && ticks.len() <= 9),
            _ => None,
        };
        for seed in 0..20 {
            let devices: std::collections::BTreeSet<bool> = (0..3)
                .flat_map(|page| visit(seed, page, article(DESKTOP), false, 40_000.0))
                .filter_map(|action| is_mouse(&action))
                .collect();
            assert_eq!(devices.len(), 1, "seed {seed}");
        }
    }

    #[test]
    fn a_phone_reader_only_swipes_and_reads() {
        let actions = visit(5, 0, article(PHONE), true, 40_000.0);
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, Action::Swipe { .. }))
        );
        assert!(
            actions
                .iter()
                .all(|action| matches!(action, Action::Swipe { .. } | Action::Idle { .. }))
        );
    }

    #[test]
    fn a_page_that_cannot_scroll_is_read_and_pointed_at() {
        let mut page = article(DESKTOP);
        page.page_height = 900.0;
        let actions = visit(3, 0, page.clone(), false, 20_000.0);
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, Action::Wheel { .. }))
        );
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, Action::MovePointer { .. }))
        );

        page.reading_targets.clear();
        let only_reading = visit(3, 0, page, false, 20_000.0);
        assert!(
            only_reading
                .iter()
                .all(|action| matches!(action, Action::Idle { .. }))
        );
    }

    #[test]
    fn dwell_is_the_observation_time_within_the_budget_of_the_visit() {
        assert_eq!(dwell_per_page(20, 2), Duration::from_secs(20));
        assert_eq!(dwell_per_page(120, 5), Duration::from_secs(50));
        assert_eq!(dwell_per_page(120, 0), Duration::from_secs(120));
        assert_eq!(dwell_per_page(0, 0), Duration::from_secs(1));
        assert_eq!(settle_time(Duration::from_secs(60)), Duration::from_secs(9));
        assert_eq!(settle_time(Duration::from_secs(8)), Duration::from_secs(2));
        assert_eq!(settle_time(Duration::from_secs(1)), Duration::from_secs(1));
    }
}
