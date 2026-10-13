//! RFC 3339 timestamps to unix milliseconds, for the timestamps GitHub and the agent CLIs write.
//! Pure arithmetic, no time zone database and no calendar crate.

/// `2026-10-12T23:03:55.696Z` or `2024-02-29T12:00:00+02:00` as milliseconds since 1970-01-01 UTC.
/// `None` for anything else.
pub(crate) fn unix_millis(text: &str) -> Option<i64> {
    let text = text.trim();
    let (date, rest) = text.split_once(['T', 't'])?;
    let mut date_parts = date.splitn(3, '-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = number(date_parts.next()?)?;
    let day: i64 = number(date_parts.next()?)?;
    let (clock, offset_secs) = split_offset(rest)?;
    let mut clock_parts = clock.splitn(3, ':');
    let hour: i64 = number(clock_parts.next()?)?;
    let minute: i64 = number(clock_parts.next()?)?;
    let (second, millis) = seconds(clock_parts.next()?)?;
    let valid = (1..=12).contains(&month) && (1..=31).contains(&day) && hour < 24 && minute < 60;
    if !valid || second > 60 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let secs = days * 86_400 + hour * 3600 + minute * 60 + second - offset_secs;
    Some(secs * 1000 + millis)
}

fn number(text: &str) -> Option<i64> {
    (text.len() == 2 && text.bytes().all(|b| b.is_ascii_digit())).then(|| text.parse().ok())?
}

fn seconds(text: &str) -> Option<(i64, i64)> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let millis = fraction.bytes().take(3).chain(std::iter::repeat(b'0')).take(3);
    let digits: Vec<u8> = millis.collect();
    let fraction_ok = fraction.bytes().all(|b| b.is_ascii_digit());
    let millis = std::str::from_utf8(&digits).ok()?.parse().ok()?;
    (fraction_ok && !whole.is_empty()).then_some(())?;
    Some((number(whole)?, millis))
}

fn split_offset(rest: &str) -> Option<(&str, i64)> {
    if let Some(clock) = rest.strip_suffix(['Z', 'z']) {
        return Some((clock, 0));
    }
    let at = rest.rfind(['+', '-'])?;
    let (clock, offset) = rest.split_at(at);
    let sign = if offset.starts_with('-') { -1 } else { 1 };
    let (hours, minutes) = offset[1..].split_once(':')?;
    Some((clock, sign * (number(hours)? * 3600 + number(minutes)? * 60)))
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_from_march = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_instants() {
        let cases = [
            ("2026-10-12T23:03:55.696Z", Some(1_791_846_235_696)),
            ("2000-01-01T00:00:00Z", Some(946_684_800_000)),
            ("2024-02-29T12:00:00+02:00", Some(1_709_200_800_000)),
            ("1969-12-31T23:59:59Z", Some(-1000)),
            ("2026-10-12T23:03:55.6Z", Some(1_791_846_235_600)),
            ("2026-10-12T23:03:55.123456Z", Some(1_791_846_235_123)),
        ];
        for (text, want) in cases {
            assert_eq!(unix_millis(text), want, "{text}");
        }
    }

    #[test]
    fn rubbish_is_none() {
        for text in [
            "",
            "yesterday",
            "2026-13-01T00:00:00Z",
            "2026-10-12",
            "2026-10-12T25:00:00Z",
            "2026-10-12T10:00:00",
        ] {
            assert_eq!(unix_millis(text), None, "{text:?}");
        }
    }
}
