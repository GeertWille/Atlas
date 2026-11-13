//! The capture's clock. The timeline of a scan mixes the browser's own
//! stamps (a request's wall time, the tracker's `timeOrigin`) with moments
//! the scanner notes as they reach it (a committed page, a parsed script).
//! With the browser on another machine the two clocks differ, so what the
//! scanner notes is stamped in the browser's time: this machine's clock
//! plus the offset measured when the connection was made. A local browser
//! shares the clock and the offset is zero.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// How many times the browser is asked for its time when the connection
/// is made. The sample with the shortest round trip is the one used.
pub(crate) const CLOCK_SAMPLES: usize = 8;
/// What a page of the browser is asked to evaluate for a sample.
pub(crate) const CLOCK_QUESTION: &str = "Date.now()";

/// Milliseconds since the epoch on this machine's clock.
pub(crate) fn machine_epoch_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs_f64() * 1000.0)
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct CaptureClock {
    /// The browser's clock minus this machine's, in ms.
    offset_ms: f64,
}

impl CaptureClock {
    /// The browser runs on this machine.
    pub(crate) const SHARED: Self = Self { offset_ms: 0.0 };

    pub(crate) fn ahead_by(offset_ms: f64) -> Self {
        Self { offset_ms }
    }

    /// Now, as the browser's clock reads it: epoch ms.
    pub(crate) fn now_ms(self) -> f64 {
        machine_epoch_ms() + self.offset_ms
    }
}

/// One question to the browser for its time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ClockSample {
    /// This machine's clock when the question left and when the answer
    /// arrived, epoch ms.
    pub asked_ms: f64,
    pub answered_ms: f64,
    /// The browser's `Date.now()`.
    pub browser_ms: f64,
}

impl ClockSample {
    /// The sample of a question asked at `asked_ms` on this machine's
    /// clock, whose answer (to `Runtime.evaluate` of [`CLOCK_QUESTION`])
    /// has just arrived. `None` when the answer holds no time.
    pub(crate) fn answered_now(asked_ms: f64, answer: &Value) -> Option<Self> {
        Some(Self {
            asked_ms,
            answered_ms: machine_epoch_ms(),
            browser_ms: answer["result"]["value"].as_f64()?,
        })
    }

    fn round_trip_ms(&self) -> f64 {
        self.answered_ms - self.asked_ms
    }
}

/// How far the browser is away, in time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Distance {
    /// The shortest round trip of the samples.
    pub round_trip_ms: f64,
    /// The browser's clock minus this machine's, taken from that sample:
    /// the browser read its clock somewhere between question and answer,
    /// and the middle is the best guess, good to half the round trip.
    pub clock_offset_ms: f64,
}

impl Distance {
    pub(crate) fn clock(&self) -> CaptureClock {
        CaptureClock::ahead_by(self.clock_offset_ms)
    }
}

/// The estimate from the sample that took the least time: the less a
/// sample waited on the way, the less its two legs can differ. `None`
/// without a usable sample.
pub(crate) fn distance(samples: &[ClockSample]) -> Option<Distance> {
    samples
        .iter()
        .filter(|sample| sample.round_trip_ms() >= 0.0)
        .min_by(|a, b| a.round_trip_ms().total_cmp(&b.round_trip_ms()))
        .map(|best| Distance {
            round_trip_ms: best.round_trip_ms(),
            clock_offset_ms: best.browser_ms - (best.asked_ms + best.answered_ms) / 2.0,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(asked_ms: f64, answered_ms: f64, browser_ms: f64) -> ClockSample {
        ClockSample {
            asked_ms,
            answered_ms,
            browser_ms,
        }
    }

    #[test]
    fn the_offset_comes_from_the_sample_with_the_shortest_round_trip() {
        let samples = [
            // Held up on the way back: the middle is far off.
            sample(1_000.0, 1_400.0, 1_010.0),
            // 120 ms there and back, the browser 51 ms behind.
            sample(2_000.0, 2_120.0, 2_009.0),
            sample(3_000.0, 3_180.0, 3_050.0),
        ];

        let measured = distance(&samples).unwrap();

        assert_eq!(measured.round_trip_ms, 120.0);
        assert_eq!(measured.clock_offset_ms, -51.0);
    }

    #[test]
    fn a_browser_ahead_has_a_positive_offset_and_no_sample_is_no_estimate() {
        let ahead = distance(&[sample(500.0, 502.0, 5_501.0)]).unwrap();
        assert_eq!(ahead.clock_offset_ms, 5_000.0);
        assert_eq!(ahead.round_trip_ms, 2.0);

        assert_eq!(distance(&[]), None);
        // This machine's clock was set back in between: not a measurement.
        assert_eq!(distance(&[sample(900.0, 100.0, 500.0)]), None);
    }

    #[test]
    fn the_capture_clock_reads_this_machines_time_plus_the_offset() {
        let before = machine_epoch_ms();
        let shared = CaptureClock::SHARED.now_ms();
        let behind = CaptureClock::ahead_by(-60_000.0).now_ms();
        let after = machine_epoch_ms();

        assert!(before <= shared && shared <= after);
        assert!(before - 60_000.0 <= behind && behind <= after - 60_000.0);
        assert_eq!(
            distance(&[sample(0.0, 10.0, 47.0)]).unwrap().clock(),
            CaptureClock::ahead_by(42.0)
        );
    }
}
