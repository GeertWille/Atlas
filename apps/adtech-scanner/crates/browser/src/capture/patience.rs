//! How long the capture waits for things that take a round trip to the
//! browser. With Chrome on this machine a round trip is about a
//! millisecond and the waits are what the page needs. A browser at
//! Browserbase is 40 to 250 ms away (measured from the US: us-east-1 44 ms,
//! us-west-2 83 ms, eu-central-1 119 ms, ap-southeast-1 250 ms), so each
//! wait is the local one plus the round trips it spans, taken from the
//! round trip measured when the connection was made.

use std::time::Duration;

/// Reading the bodies still on their way at the end of a visit: one round
/// trip each, a few at a time. Reading one body took up to seven round
/// trips when it was measured (843 ms at 119 ms).
const BODY_ROUND_TRIPS: u32 = 20;
/// A click is two commands, and the navigation it starts is reported back.
const CLICK_ROUND_TRIPS: u32 = 4;
/// No wait grows beyond this, however far the browser is.
const LONGEST_ALLOWANCE: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Patience {
    /// Bodies still in flight when the visit ends get this long to arrive.
    pub bodies: Duration,
    /// A click that leads somewhere shows it within this time.
    pub navigation_signal: Duration,
    /// A tab that opens this soon after the visitor's click was opened by it.
    pub click_opens_tab: Duration,
    /// A click on a consent control is seen by the control within this time.
    pub consent_click: Duration,
}

impl Patience {
    /// With the browser on this machine.
    pub(crate) const LOCAL: Self = Self {
        bodies: Duration::from_secs(3),
        navigation_signal: Duration::from_secs(4),
        click_opens_tab: Duration::from_secs(3),
        consent_click: Duration::from_secs(1),
    };

    /// With the browser `round_trip` away.
    pub(crate) fn across(round_trip: Duration) -> Self {
        let plus = |round_trips: u32| (round_trip * round_trips).min(LONGEST_ALLOWANCE);
        let local = Self::LOCAL;
        Self {
            bodies: local.bodies + plus(BODY_ROUND_TRIPS),
            navigation_signal: local.navigation_signal + plus(CLICK_ROUND_TRIPS),
            click_opens_tab: local.click_opens_tab + plus(CLICK_ROUND_TRIPS),
            consent_click: local.consent_click + plus(CLICK_ROUND_TRIPS),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_browser_on_this_machine_is_waited_for_as_before() {
        assert_eq!(Patience::across(Duration::ZERO), Patience::LOCAL);
        assert_eq!(Patience::LOCAL.bodies, Duration::from_secs(3));
    }

    #[test]
    fn a_browser_in_frankfurt_gets_the_round_trips_each_wait_spans() {
        let frankfurt = Patience::across(Duration::from_millis(119));

        assert_eq!(frankfurt.bodies, Duration::from_millis(3_000 + 20 * 119));
        assert_eq!(frankfurt.navigation_signal, Duration::from_millis(4_476));
        assert_eq!(frankfurt.click_opens_tab, Duration::from_millis(3_476));
        assert_eq!(frankfurt.consent_click, Duration::from_millis(1_476));
    }

    #[test]
    fn the_further_the_browser_the_longer_the_wait_up_to_a_bound() {
        let near = Patience::across(Duration::from_millis(44));
        let far = Patience::across(Duration::from_millis(250));
        assert!(near.bodies < far.bodies);
        assert_eq!(far.bodies, Duration::from_secs(8));

        let unreachable = Patience::across(Duration::from_secs(60));
        assert_eq!(unreachable.bodies, Duration::from_secs(33));
        assert_eq!(unreachable.consent_click, Duration::from_secs(31));
    }
}
