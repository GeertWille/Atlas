//! The shape of a visitor's gestures: where the pointer is at each moment of
//! a movement, how a wheel or trackpad scroll is spread over time, how a
//! finger swipes. Pages time and count these events (viewability, engagement
//! and lazy-load logic), so they are produced the way an input device would.

use super::rng::Rng;
use super::{Point, Size};

/// A position reached `at_ms` after the gesture began.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Waypoint {
    pub point: Point,
    pub at_ms: f64,
}

/// Pointer events are sent about once per display frame.
const FRAME_MS: (f64, f64) = (12.0, 20.0);
/// Sideways bend of a movement, as a share of its length.
const BEND: f64 = 0.2;
const JITTER_PX: f64 = 1.0;
/// Long movements sometimes land past the target and are corrected.
const OVERSHOOT_FROM_PX: f64 = 400.0;
const OVERSHOOT_CHANCE: f64 = 0.2;
const OVERSHOOT_PX: (f64, f64) = (8.0, 20.0);
const CORRECTION_MS: (f64, f64) = (90.0, 160.0);

/// The pointer's way from `from` to `to`, a target `target_width` wide. Ends
/// exactly on `to` and never leaves the viewport.
pub(crate) fn pointer_path(
    rng: &mut Rng,
    from: Point,
    to: Point,
    target_width: f64,
    viewport: Size,
) -> Vec<Waypoint> {
    let to = viewport.clamp(to);
    let distance = from.distance_to(to);
    if distance < 1.0 {
        return vec![Waypoint {
            point: to,
            at_ms: 0.0,
        }];
    }
    let duration = movement_ms(distance, target_width) * rng.between(0.8, 1.2);
    let mut path = Vec::new();
    if distance > OVERSHOOT_FROM_PX && rng.chance(OVERSHOOT_CHANCE) {
        let past = 1.0 + rng.between(OVERSHOOT_PX.0, OVERSHOOT_PX.1) / distance;
        let overshot = viewport.clamp(Point {
            x: from.x + (to.x - from.x) * past,
            y: from.y + (to.y - from.y) * past,
        });
        stroke(rng, from, overshot, 0.0, duration, viewport, &mut path);
        let correction = rng.between(CORRECTION_MS.0, CORRECTION_MS.1);
        stroke(rng, overshot, to, duration, correction, viewport, &mut path);
    } else {
        stroke(rng, from, to, 0.0, duration, viewport, &mut path);
    }
    path
}

/// Fitts's law: farther and smaller targets take longer to reach.
fn movement_ms(distance: f64, target_width: f64) -> f64 {
    250.0 + 120.0 * (1.0 + distance / target_width.max(1.0)).log2()
}

/// One continuous movement along a bent curve, slow at both ends.
fn stroke(
    rng: &mut Rng,
    from: Point,
    to: Point,
    starts_at_ms: f64,
    duration: f64,
    viewport: Size,
    path: &mut Vec<Waypoint>,
) {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let mut control = |along: f64| {
        let sideways = rng.between(-BEND, BEND);
        Point {
            x: from.x + dx * along - dy * sideways,
            y: from.y + dy * along + dx * sideways,
        }
    };
    let (first, second) = (control(0.3), control(0.7));
    let mut elapsed = rng.between(FRAME_MS.0, FRAME_MS.1);
    while elapsed < duration {
        let progress = minimum_jerk(elapsed / duration);
        let on_curve = cubic_bezier(from, first, second, to, progress);
        path.push(Waypoint {
            point: viewport.clamp(Point {
                x: on_curve.x + rng.between(-JITTER_PX, JITTER_PX),
                y: on_curve.y + rng.between(-JITTER_PX, JITTER_PX),
            }),
            at_ms: starts_at_ms + elapsed,
        });
        elapsed += rng.between(FRAME_MS.0, FRAME_MS.1);
    }
    path.push(Waypoint {
        point: to,
        at_ms: starts_at_ms + duration,
    });
}

/// Share of the way covered after a share `time` of the duration, for a
/// movement that starts and ends at rest.
fn minimum_jerk(time: f64) -> f64 {
    let t = time.clamp(0.0, 1.0);
    t * t * t * (10.0 - 15.0 * t + 6.0 * t * t)
}

fn cubic_bezier(start: Point, first: Point, second: Point, end: Point, t: f64) -> Point {
    let u = 1.0 - t;
    let blend = |a: f64, b: f64, c: f64, d: f64| {
        u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
    };
    Point {
        x: blend(start.x, first.x, second.x, end.x),
        y: blend(start.y, first.y, second.y, end.y),
    }
}

/// What the visitor scrolls with. One per visit, drawn from the seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WheelDevice {
    /// Few large notches.
    Mouse,
    /// Many small deltas that die away.
    Trackpad,
}

impl WheelDevice {
    pub(crate) fn draw(rng: &mut Rng) -> Self {
        if rng.chance(0.5) {
            WheelDevice::Mouse
        } else {
            WheelDevice::Trackpad
        }
    }
}

/// One wheel event: `delta_y` pixels, `gap_ms` after the previous one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct WheelTick {
    pub delta_y: f64,
    pub gap_ms: f64,
}

const MOUSE_NOTCH_PX: f64 = 100.0;
const MOUSE_NOTCHES: (u32, u32) = (3, 9);
const MOUSE_GAP_MS: (f64, f64) = (30.0, 90.0);
const TRACKPAD_EVENTS: (u32, u32) = (12, 40);
const TRACKPAD_FIRST_PX: (f64, f64) = (30.0, 60.0);
const TRACKPAD_LAST_PX: f64 = 4.0;
const TRACKPAD_GAP_MS: f64 = 16.0;

/// One scroll gesture down the page.
pub(crate) fn wheel_onward(rng: &mut Rng, device: WheelDevice) -> Vec<WheelTick> {
    match device {
        WheelDevice::Mouse => {
            let notches = rng.whole(MOUSE_NOTCHES.0, MOUSE_NOTCHES.1);
            mouse_notches(rng, notches, MOUSE_NOTCH_PX)
        }
        WheelDevice::Trackpad => trackpad_flick(rng),
    }
}

/// One scroll gesture of about `distance` pixels: down the page when
/// positive, back up when negative.
pub(crate) fn wheel_by(rng: &mut Rng, device: WheelDevice, distance: f64) -> Vec<WheelTick> {
    let direction = if distance < 0.0 { -1.0 } else { 1.0 };
    match device {
        WheelDevice::Mouse => {
            let notches = (distance.abs() / MOUSE_NOTCH_PX).round().max(1.0) as u32;
            mouse_notches(rng, notches, direction * MOUSE_NOTCH_PX)
        }
        WheelDevice::Trackpad => {
            let mut flick = trackpad_flick(rng);
            let length: f64 = flick.iter().map(|tick| tick.delta_y).sum();
            for tick in &mut flick {
                let share = (tick.delta_y * distance.abs() / length).round().max(1.0);
                tick.delta_y = direction * share;
            }
            flick
        }
    }
}

fn mouse_notches(rng: &mut Rng, notches: u32, delta_y: f64) -> Vec<WheelTick> {
    (0..notches)
        .map(|_| WheelTick {
            delta_y,
            gap_ms: rng.between(MOUSE_GAP_MS.0, MOUSE_GAP_MS.1).round(),
        })
        .collect()
}

/// Deltas shrinking geometrically from the first to `TRACKPAD_LAST_PX`.
fn trackpad_flick(rng: &mut Rng) -> Vec<WheelTick> {
    let events = rng.whole(TRACKPAD_EVENTS.0, TRACKPAD_EVENTS.1);
    let first = rng.between(TRACKPAD_FIRST_PX.0, TRACKPAD_FIRST_PX.1);
    let shrink = (TRACKPAD_LAST_PX / first).powf(1.0 / f64::from(events - 1));
    (0..events)
        .map(|event| WheelTick {
            delta_y: (first * shrink.powi(event.cast_signed())).round(),
            gap_ms: TRACKPAD_GAP_MS,
        })
        .collect()
}

/// Share of the viewport height a thumb covers in one swipe.
const SWIPE_VIEWPORTS: (f64, f64) = (0.25, 0.6);
const SWIPE_MS: (f64, f64) = (180.0, 450.0);
const SWIPE_DRIFT_PX: f64 = 12.0;
const SWIPE_FRAME_MS: f64 = 16.0;
/// Where a thumb comes down, as shares of the viewport.
const THUMB_ACROSS: (f64, f64) = (0.4, 0.7);
const THUMB_DOWN_ONWARD: (f64, f64) = (0.68, 0.82);
const THUMB_DOWN_BACK: (f64, f64) = (0.2, 0.35);

/// The finger's way across the screen for one swipe of a length a thumb
/// would choose. `onward` moves the page down (the finger goes up).
pub(crate) fn swipe_path(rng: &mut Rng, viewport: Size, onward: bool) -> Vec<Waypoint> {
    let distance = viewport.height * rng.between(SWIPE_VIEWPORTS.0, SWIPE_VIEWPORTS.1);
    swipe_by(rng, viewport, if onward { distance } else { -distance })
}

/// One swipe that scrolls the page by `distance` pixels, down when positive,
/// or by as much of it as one swipe covers.
pub(crate) fn swipe_by(rng: &mut Rng, viewport: Size, distance: f64) -> Vec<Waypoint> {
    let onward = distance >= 0.0;
    let length = distance.abs().min(viewport.height * SWIPE_VIEWPORTS.1);
    let duration = rng.between(SWIPE_MS.0, SWIPE_MS.1);
    let drift = rng.between(-SWIPE_DRIFT_PX, SWIPE_DRIFT_PX);
    let down = if onward {
        THUMB_DOWN_ONWARD
    } else {
        THUMB_DOWN_BACK
    };
    let start = Point {
        x: viewport.width * rng.between(THUMB_ACROSS.0, THUMB_ACROSS.1),
        y: viewport.height * rng.between(down.0, down.1),
    };
    let travel = if onward { -length } else { length };
    let frames = (duration / SWIPE_FRAME_MS).ceil().max(2.0) as u32;
    (0..=frames)
        .map(|frame| {
            let time = f64::from(frame) / f64::from(frames);
            // A swipe is fastest as the finger lands and slows towards its end.
            let progress = 1.0 - (1.0 - time).powi(3);
            Waypoint {
                point: viewport.clamp(Point {
                    x: start.x + drift * progress,
                    y: start.y + travel * progress,
                }),
                at_ms: duration * time,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: Size = Size {
        width: 1440.0,
        height: 900.0,
    };

    fn point(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    fn assert_steady_clock(path: &[Waypoint]) {
        assert!(path[0].at_ms >= 0.0);
        for pair in path.windows(2) {
            let gap = pair[1].at_ms - pair[0].at_ms;
            assert!(gap > 0.0 && gap <= FRAME_MS.1 + 1e-9, "gap of {gap} ms");
        }
    }

    #[test]
    fn a_pointer_path_ends_on_its_target_and_stays_in_the_viewport() {
        for seed in 0..200 {
            let mut rng = Rng::new(seed);
            let from = point(rng.between(0.0, 1440.0), rng.between(0.0, 900.0));
            let to = point(rng.between(-50.0, 1500.0), rng.between(-50.0, 950.0));
            let path = pointer_path(&mut rng, from, to, 120.0, VIEWPORT);
            assert_eq!(
                path.last().unwrap().point,
                VIEWPORT.clamp(to),
                "seed {seed}"
            );
            for waypoint in &path {
                let inside = (1.0..=1439.0).contains(&waypoint.point.x)
                    && (1.0..=899.0).contains(&waypoint.point.y);
                assert!(inside, "seed {seed}: {:?}", waypoint.point);
            }
            assert_steady_clock(&path);
        }
    }

    #[test]
    fn a_movement_is_slow_at_both_ends_and_fast_in_the_middle() {
        let mut rng = Rng::new(4);
        let path = pointer_path(
            &mut rng,
            point(100.0, 450.0),
            point(380.0, 450.0),
            80.0,
            VIEWPORT,
        );
        let covered = |index: usize| path[index].point.x - 100.0;
        let middle = path.len() / 2;
        let first_steps = covered(2);
        let middle_steps = covered(middle + 1) - covered(middle - 1);
        assert!(
            middle_steps > 3.0 * first_steps,
            "{middle_steps} px in the middle, {first_steps} px at the start"
        );
    }

    #[test]
    fn farther_and_smaller_targets_take_longer() {
        assert!(movement_ms(800.0, 100.0) > movement_ms(200.0, 100.0));
        assert!(movement_ms(400.0, 20.0) > movement_ms(400.0, 200.0));
        assert_eq!(movement_ms(300.0, 100.0), 250.0 + 120.0 * 2.0);
        let duration_of = |seed| {
            let mut rng = Rng::new(seed);
            let path = pointer_path(
                &mut rng,
                point(10.0, 10.0),
                point(310.0, 10.0),
                100.0,
                VIEWPORT,
            );
            path.last().unwrap().at_ms
        };
        for seed in 0..50 {
            let duration = duration_of(seed);
            assert!(
                (490.0 * 0.8..=490.0 * 1.2).contains(&duration),
                "{duration}"
            );
        }
    }

    #[test]
    fn some_long_movements_overshoot_and_come_back() {
        let from = point(100.0, 450.0);
        let to = point(1_000.0, 450.0);
        let farthest = |seed| {
            let path = pointer_path(&mut Rng::new(seed), from, to, 100.0, VIEWPORT);
            path.iter()
                .map(|waypoint| waypoint.point.x)
                .fold(0.0, f64::max)
        };
        let overshooting = (0..300).filter(|seed| farthest(*seed) > to.x + 6.0).count();
        assert!((30..=95).contains(&overshooting), "{overshooting} of 300");
        let short = |seed| {
            let to = point(300.0, 450.0);
            let path = pointer_path(&mut Rng::new(seed), from, to, 100.0, VIEWPORT);
            path.iter()
                .map(|waypoint| waypoint.point.x)
                .fold(0.0, f64::max)
        };
        assert!((0..300).all(|seed| short(seed) < 300.0 + 2.0 * JITTER_PX));
    }

    #[test]
    fn a_pointer_that_is_already_there_does_not_move() {
        let at = point(200.0, 300.0);
        let path = pointer_path(&mut Rng::new(1), at, at, 50.0, VIEWPORT);
        assert_eq!(
            path,
            [Waypoint {
                point: at,
                at_ms: 0.0
            }]
        );
    }

    #[test]
    fn a_mouse_wheel_scrolls_in_a_few_whole_notches() {
        for seed in 0..100 {
            let ticks = wheel_onward(&mut Rng::new(seed), WheelDevice::Mouse);
            assert!((3..=9).contains(&ticks.len()));
            assert!(ticks.iter().all(|tick| tick.delta_y == 100.0));
            assert!(
                ticks
                    .iter()
                    .all(|tick| (30.0..=90.0).contains(&tick.gap_ms))
            );
        }
        let back = wheel_by(&mut Rng::new(1), WheelDevice::Mouse, -340.0);
        assert_eq!(back.len(), 3);
        assert!(back.iter().all(|tick| tick.delta_y == -100.0));
        let on = wheel_by(&mut Rng::new(1), WheelDevice::Mouse, 10.0);
        let deltas: Vec<f64> = on.iter().map(|tick| tick.delta_y).collect();
        assert_eq!(deltas, [100.0]);
    }

    #[test]
    fn a_trackpad_scrolls_in_many_small_deltas_that_die_away() {
        for seed in 0..100 {
            let ticks = wheel_onward(&mut Rng::new(seed), WheelDevice::Trackpad);
            assert!((12..=40).contains(&ticks.len()));
            assert!(
                ticks
                    .iter()
                    .all(|tick| (4.0..=60.0).contains(&tick.delta_y))
            );
            assert!(
                ticks
                    .windows(2)
                    .all(|pair| pair[1].delta_y <= pair[0].delta_y)
            );
            assert_eq!(ticks.last().unwrap().delta_y, 4.0);
            assert!(ticks.iter().all(|tick| tick.gap_ms == 16.0));
        }
        let back = wheel_by(&mut Rng::new(2), WheelDevice::Trackpad, -400.0);
        let length: f64 = back.iter().map(|tick| tick.delta_y).sum();
        assert!(back.iter().all(|tick| tick.delta_y < 0.0));
        assert!((-440.0..=-360.0).contains(&length), "{length}");
        let on = wheel_by(&mut Rng::new(2), WheelDevice::Trackpad, 250.0);
        let length: f64 = on.iter().map(|tick| tick.delta_y).sum();
        assert!((225.0..=275.0).contains(&length), "{length}");
    }

    #[test]
    fn both_scroll_devices_are_drawn() {
        let devices: Vec<WheelDevice> = (0..40)
            .map(|seed| WheelDevice::draw(&mut Rng::new(seed)))
            .collect();
        assert!(devices.contains(&WheelDevice::Mouse));
        assert!(devices.contains(&WheelDevice::Trackpad));
    }

    #[test]
    fn a_swipe_moves_the_finger_up_the_screen_with_a_little_drift() {
        let phone = Size {
            width: 412.0,
            height: 915.0,
        };
        for seed in 0..100 {
            let path = swipe_path(&mut Rng::new(seed), phone, true);
            let (first, last) = (path[0], *path.last().unwrap());
            let distance = first.point.y - last.point.y;
            assert!(
                (0.25 * 915.0..=0.6 * 915.0).contains(&distance),
                "{distance}"
            );
            assert!((first.point.x - last.point.x).abs() <= SWIPE_DRIFT_PX);
            assert!((180.0..=450.0).contains(&last.at_ms));
            assert_eq!(first.at_ms, 0.0);
            assert!(path.windows(2).all(|pair| pair[1].at_ms > pair[0].at_ms));
            assert!(
                path.windows(2)
                    .all(|pair| pair[1].point.y <= pair[0].point.y)
            );
            let early = first.point.y - path[1].point.y;
            let late = path[path.len() - 2].point.y - last.point.y;
            assert!(early > late, "a swipe slows down towards its end");
        }
        let back = swipe_path(&mut Rng::new(3), phone, false);
        assert!(back.last().unwrap().point.y > back[0].point.y + 0.25 * 915.0 - 1e-6);
    }

    #[test]
    fn a_swipe_of_a_given_length_covers_it_up_to_a_thumbs_reach() {
        let phone = Size {
            width: 412.0,
            height: 915.0,
        };
        let travelled = |distance: f64| {
            let path = swipe_by(&mut Rng::new(8), phone, distance);
            path[0].point.y - path.last().unwrap().point.y
        };
        assert!((travelled(120.0) - 120.0).abs() < 1e-6);
        assert!((travelled(-200.0) + 200.0).abs() < 1e-6);
        assert!((travelled(5_000.0) - 0.6 * 915.0).abs() < 1e-6);
    }
}
