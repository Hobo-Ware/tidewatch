//! The wall clock as unix time, the one place the workspace asks for "now".

use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the unix epoch, 0 when the clock is set before 1970.
#[must_use]
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}

/// Seconds since the unix epoch, 0 when the clock is set before 1970.
#[must_use]
pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    const Y2020_MS: i64 = 1_577_836_800_000;

    #[test]
    fn both_readings_are_current_unix_time() {
        let (before, secs, after) = (now_ms(), now_secs(), now_ms());
        assert!(before > Y2020_MS);
        assert!(i64::try_from(secs).unwrap() * 1000 >= before - 1000);
        assert!(i64::try_from(secs).unwrap() * 1000 <= after);
    }

    #[test]
    fn milliseconds_never_go_backwards_between_two_reads() {
        let first = now_ms();
        assert!(now_ms() >= first);
    }
}
