//! How long a kept copy of an exchange's sellers.json may be used. No I/O.

const HOUR_MS: f64 = 3_600_000.0;

/// A downloaded file is used without asking the exchange again for a day:
/// exchanges publish a new sellers.json at most a few times a day, and a
/// scan's verdict on a seller id does not hinge on the last hours.
pub const FRESH_FOR_MS: f64 = 24.0 * HOUR_MS;
/// "Could not be read" is asked again sooner: most such answers are passing
/// trouble at the exchange.
pub const UNAVAILABLE_FRESH_FOR_MS: f64 = HOUR_MS;
/// When the exchange cannot be reached, a copy this old is still better
/// than no check. Nothing older is ever used.
pub const USABLE_WHEN_UNREACHABLE_FOR_MS: f64 = 7.0 * 24.0 * HOUR_MS;
/// The sellers.json of a large exchange is a few megabytes. A larger file
/// is neither downloaded whole nor read back from a kept copy.
pub const MAX_FILE_BYTES: usize = 24 * 1024 * 1024;
/// All kept copies together; the least recently used go first.
pub const MAX_CACHE_BYTES: u64 = 500 * 1024 * 1024;

/// What to do with a kept answer that was fetched `fetched_at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Use it; the exchange is not contacted.
    Fresh,
    /// Ask the exchange. Should that fail, the kept file is used all the same.
    AskAgainOrUse,
    /// Ask the exchange, and use only what it answers.
    AskAgain,
}

/// `has_file`: the kept answer is a file, not a note that there was none.
/// A copy dated in the future (the clock was set back) is not trusted.
pub fn standing(has_file: bool, fetched_at: f64, now: f64) -> Standing {
    let age = now - fetched_at;
    let fresh_for = if has_file {
        FRESH_FOR_MS
    } else {
        UNAVAILABLE_FRESH_FOR_MS
    };
    if (0.0..fresh_for).contains(&age) {
        Standing::Fresh
    } else if has_file && (0.0..=USABLE_WHEN_UNREACHABLE_FOR_MS).contains(&age) {
        Standing::AskAgainOrUse
    } else {
        Standing::AskAgain
    }
}

/// Whether an answer says nothing about the file itself, only that the
/// exchange has trouble right now: a kept copy outlives it.
pub fn is_passing_trouble(status: u16) -> bool {
    status == 429 || status >= 500
}

/// The UTC calendar day of an epoch time in ms, as `2026-11-12`.
pub fn utc_day(epoch_ms: f64) -> String {
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let days = (epoch_ms / 86_400_000.0).floor() as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_794_500_000_000.0;

    #[test]
    fn a_file_is_used_for_a_day_and_kept_as_a_fallback_for_a_week() {
        let aged = |hours: f64| standing(true, NOW - hours * HOUR_MS, NOW);
        assert_eq!(aged(0.0), Standing::Fresh);
        assert_eq!(aged(23.9), Standing::Fresh);
        assert_eq!(aged(24.0), Standing::AskAgainOrUse);
        assert_eq!(aged(7.0 * 24.0), Standing::AskAgainOrUse);
        assert_eq!(aged(7.0 * 24.0 + 0.1), Standing::AskAgain);
        assert_eq!(aged(-0.1), Standing::AskAgain, "dated in the future");
    }

    #[test]
    fn a_failed_lookup_is_remembered_for_an_hour_and_never_used_after_it() {
        let aged = |hours: f64| standing(false, NOW - hours * HOUR_MS, NOW);
        assert_eq!(aged(0.5), Standing::Fresh);
        assert_eq!(aged(1.0), Standing::AskAgain);
        assert_eq!(aged(30.0), Standing::AskAgain);
        assert_eq!(aged(-0.1), Standing::AskAgain);
    }

    #[test]
    fn only_server_side_trouble_and_rate_limits_keep_a_copy_in_use() {
        for status in [429, 500, 502, 503, 504] {
            assert!(is_passing_trouble(status), "{status}");
        }
        for status in [301, 400, 403, 404, 410] {
            assert!(!is_passing_trouble(status), "{status}");
        }
    }

    #[test]
    fn days_are_utc_calendar_days() {
        assert_eq!(utc_day(0.0), "1970-01-01");
        assert_eq!(utc_day(951_782_400_000.0), "2000-02-29");
        assert_eq!(utc_day(1_794_441_600_000.0 - 1.0), "2026-11-11");
        assert_eq!(utc_day(1_794_441_600_000.0), "2026-11-12");
        assert_eq!(utc_day(1_798_761_599_000.0), "2026-12-31");
    }
}
