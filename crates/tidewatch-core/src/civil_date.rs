//! A calendar date in the proleptic Gregorian calendar, with no time zone (pure).
//!
//! The day arithmetic is Howard Hinnant's `days_from_civil` / `civil_from_days`.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    year: i32,
    month: u8,
    day: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Weekday {
    const ALL: [Self; 7] =
        [Self::Mon, Self::Tue, Self::Wed, Self::Thu, Self::Fri, Self::Sat, Self::Sun];

    #[must_use]
    pub fn short(self) -> &'static str {
        match self {
            Self::Mon => "Mon",
            Self::Tue => "Tue",
            Self::Wed => "Wed",
            Self::Thu => "Thu",
            Self::Fri => "Fri",
            Self::Sat => "Sat",
            Self::Sun => "Sun",
        }
    }
}

fn is_leap(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

impl Date {
    #[must_use]
    pub fn new(year: i32, month: u8, day: u8) -> Option<Self> {
        let valid = (1..=12).contains(&month) && (1..=days_in_month(year, month)).contains(&day);
        valid.then_some(Self { year, month, day })
    }

    /// Strictly `YYYY-MM-DD`: four digits, two digits, two digits, a real day.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        let shaped = bytes.len() == 10
            && bytes[4] == b'-'
            && bytes[7] == b'-'
            && bytes.iter().enumerate().all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit());
        if !shaped {
            return None;
        }
        Self::new(text[..4].parse().ok()?, text[5..7].parse().ok()?, text[8..].parse().ok()?)
    }

    /// The UTC date of a unix timestamp.
    #[must_use]
    pub fn from_unix_secs(secs: i64) -> Self {
        Self::from_day_number(secs.div_euclid(86_400))
    }

    #[must_use]
    pub fn year(self) -> i32 {
        self.year
    }

    #[must_use]
    pub fn month(self) -> u8 {
        self.month
    }

    #[must_use]
    pub fn day(self) -> u8 {
        self.day
    }

    /// Days since 1970-01-01, negative before it.
    #[must_use]
    pub fn day_number(self) -> i64 {
        let year = i64::from(self.year) - i64::from(self.month <= 2);
        let era = year.div_euclid(400);
        let year_of_era = year - era * 400;
        let shifted_month = i64::from((self.month + 9) % 12);
        let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(self.day) - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }

    #[must_use]
    pub fn from_day_number(days: i64) -> Self {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let day_of_era = z - era * 146_097;
        let year_of_era =
            (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let shifted_month = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
        let month = if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 };
        let year = year_of_era + era * 400 + i64::from(month <= 2);
        Self { year: year as i32, month: month as u8, day: day as u8 }
    }

    /// Whole days from `self` to `other`: positive when `other` is later.
    #[must_use]
    pub fn days_until(self, other: Self) -> i64 {
        other.day_number() - self.day_number()
    }

    #[must_use]
    pub fn plus_days(self, days: i64) -> Self {
        Self::from_day_number(self.day_number() + days)
    }

    #[must_use]
    pub fn weekday(self) -> Weekday {
        // 1970-01-01 was a Thursday.
        Weekday::ALL[(self.day_number() + 3).rem_euclid(7) as usize]
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

impl Serialize for Date {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Date {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).ok_or_else(|| serde::de::Error::custom(format!("not a date: {text}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> Date {
        Date::parse(text).unwrap_or_else(|| panic!("{text} should parse"))
    }

    #[test]
    fn parse_takes_only_real_dates_in_the_one_shape() {
        let cases = [
            ("2026-10-17", Some((2026, 10, 17))),
            ("2024-02-29", Some((2024, 2, 29))),
            ("2000-02-29", Some((2000, 2, 29))),
            ("2026-02-29", None),
            ("1900-02-29", None),
            ("2026-13-01", None),
            ("2026-00-10", None),
            ("2026-04-31", None),
            ("2026-4-1", None),
            ("2026/10/17", None),
            ("2026-10-17 ", None),
            ("+026-10-17", None),
            ("", None),
        ];
        for (text, want) in cases {
            let got = Date::parse(text).map(|d| (d.year(), d.month(), d.day()));
            assert_eq!(got, want, "{text:?}");
        }
    }

    #[test]
    fn day_numbers_match_known_anchors_both_ways() {
        let cases = [
            ("1970-01-01", 0),
            ("1970-01-02", 1),
            ("1969-12-31", -1),
            ("2000-03-01", 11_017),
            ("2024-02-29", 19_782),
            ("2026-10-17", 20_743),
            ("1900-01-01", -25_567),
        ];
        for (text, days) in cases {
            assert_eq!(date(text).day_number(), days, "{text}");
            assert_eq!(Date::from_day_number(days), date(text), "{days}");
        }
    }

    #[test]
    fn every_day_over_five_centuries_round_trips_and_steps_by_one() {
        let start = date("1800-01-01").day_number();
        let end = date("2300-12-31").day_number();
        let mut previous = Date::from_day_number(start);
        for days in start + 1..=end {
            let day = Date::from_day_number(days);
            assert_eq!(day.day_number(), days);
            assert_eq!(Date::parse(&day.to_string()), Some(day));
            assert!(day > previous, "{previous} then {day}");
            assert_eq!(previous.days_until(day), 1);
            previous = day;
        }
    }

    #[test]
    fn unix_seconds_give_the_utc_date() {
        let cases = [
            (0, "1970-01-01"),
            (86_399, "1970-01-01"),
            (86_400, "1970-01-02"),
            (-1, "1969-12-31"),
            (1_792_195_200, "2026-10-17"),
            (1_792_281_599, "2026-10-17"),
            (1_792_281_600, "2026-10-18"),
        ];
        for (secs, want) in cases {
            assert_eq!(Date::from_unix_secs(secs), date(want), "{secs}");
        }
    }

    #[test]
    fn days_until_and_plus_days_cross_months_and_years() {
        let cases = [
            ("2026-10-17", "2026-11-01", 15),
            ("2026-11-01", "2026-10-17", -15),
            ("2026-12-31", "2027-01-01", 1),
            ("2024-02-28", "2024-03-01", 2),
            ("2026-10-17", "2026-10-17", 0),
        ];
        for (from, to, days) in cases {
            assert_eq!(date(from).days_until(date(to)), days, "{from} to {to}");
            assert_eq!(date(from).plus_days(days), date(to), "{from} plus {days}");
        }
    }

    #[test]
    fn weekdays_match_the_calendar() {
        let cases = [
            ("1970-01-01", Weekday::Thu),
            ("2026-10-17", Weekday::Sat),
            ("2026-10-19", Weekday::Mon),
            ("2024-02-29", Weekday::Thu),
            ("1969-12-28", Weekday::Sun),
        ];
        for (text, want) in cases {
            assert_eq!(date(text).weekday(), want, "{text}");
        }
    }

    #[test]
    fn prints_zero_padded_and_serializes_as_text() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Holder {
            on: Date,
        }
        assert_eq!(Date::new(987, 3, 5).unwrap().to_string(), "0987-03-05");
        let text = toml::to_string(&Holder { on: date("2026-11-01") }).unwrap();
        assert_eq!(text, "on = \"2026-11-01\"\n");
        assert_eq!(toml::from_str::<Holder>(&text).unwrap(), Holder { on: date("2026-11-01") });
        assert!(toml::from_str::<Holder>("on = \"2026-02-30\"").is_err());
    }
}
