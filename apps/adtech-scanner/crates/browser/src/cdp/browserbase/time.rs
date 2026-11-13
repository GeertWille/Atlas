//! Instants as Browserbase writes them: `2026-11-13T08:26:07.123Z`.

/// Epoch milliseconds of an RFC 3339 instant in UTC or with a numeric
/// offset. `None` for anything else.
pub(crate) fn epoch_ms(instant: &str) -> Option<f64> {
    let (date, time) = instant.split_once(['T', 't', ' '])?;
    let mut date = date.splitn(3, '-').map(str::parse::<i64>);
    let (year, month, day) = (date.next()?.ok()?, date.next()?.ok()?, date.next()?.ok()?);
    let offset_starts = time.find(['Z', 'z', '+', '-'])?;
    let (clock, offset) = time.split_at(offset_starts);
    let mut clock = clock.splitn(3, ':');
    let hours: i64 = clock.next()?.parse().ok()?;
    let minutes: i64 = clock.next()?.parse().ok()?;
    let seconds: f64 = clock.next()?.parse().ok()?;
    let offset_minutes = match offset {
        "Z" | "z" => 0,
        offset => {
            let (hours, minutes) = offset[1..].split_once(':')?;
            let ahead = hours.parse::<i64>().ok()? * 60 + minutes.parse::<i64>().ok()?;
            if offset.starts_with('-') {
                -ahead
            } else {
                ahead
            }
        }
    };
    let in_range = (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && (0..24).contains(&hours)
        && (0..60).contains(&minutes)
        && (0.0..61.0).contains(&seconds);
    if !in_range {
        return None;
    }
    let minutes_of_epoch =
        days_from_civil(year, month, day) * 24 * 60 + hours * 60 + minutes - offset_minutes;
    Some(minutes_of_epoch as f64 * 60_000.0 + seconds * 1_000.0)
}

/// Days from 1970-01-01 to a date of the proleptic Gregorian calendar
/// (Howard Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_services_instants_are_read_to_the_millisecond() {
        assert_eq!(epoch_ms("1970-01-01T00:00:00.000Z"), Some(0.0));
        assert_eq!(
            epoch_ms("2026-11-13T08:26:07.123Z"),
            Some(1_794_558_367_123.0)
        );
        assert_eq!(epoch_ms("2024-02-29T23:59:59Z"), Some(1_709_251_199_000.0));
        assert_eq!(
            epoch_ms("2026-11-13T10:26:07.123+02:00"),
            epoch_ms("2026-11-13T08:26:07.123Z")
        );
        assert_eq!(
            epoch_ms("2026-11-13T03:26:07.123-05:00"),
            epoch_ms("2026-11-13T08:26:07.123Z")
        );
    }

    #[test]
    fn what_is_not_an_instant_is_not_guessed_at() {
        for text in [
            "",
            "yesterday",
            "2026-11-13",
            "2026-11-13T08:26:07",
            "2026-13-01T00:00:00Z",
            "2026-11-13T25:00:00Z",
            "2026-11-13T08:26Z",
            "1794558367123",
        ] {
            assert_eq!(epoch_ms(text), None, "{text}");
        }
    }
}
