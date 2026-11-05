//! What flipping the Away toggle does, as plain data the app host and the tests share.

/// What a click on the Away pill changes: the new Away value, and whether keep-awake has to be
/// started with it. Turning Away off never touches keep-awake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Flip {
    pub(crate) away: bool,
    pub(crate) start_keep_awake: bool,
}

/// Whether the owner is at the keyboard. An enum keeps the app struct free of one more loose bool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Presence {
    #[default]
    Here,
    Away,
}

impl Presence {
    pub(crate) const fn of(away: bool) -> Self {
        if away { Self::Away } else { Self::Here }
    }

    pub(crate) const fn is_away(self) -> bool {
        matches!(self, Self::Away)
    }
}

pub(crate) const fn flip(away: bool, keep_awake_on: bool) -> Flip {
    let away = !away;
    Flip { away, start_keep_awake: away && !keep_awake_on }
}

pub(crate) const TIP: &str =
    "Away: agents decide on their own and keep working. Also keeps the Mac awake.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_away_on_starts_keep_awake_only_when_it_is_off() {
        let cases = [
            (false, false, Flip { away: true, start_keep_awake: true }),
            (false, true, Flip { away: true, start_keep_awake: false }),
            (true, false, Flip { away: false, start_keep_awake: false }),
            (true, true, Flip { away: false, start_keep_awake: false }),
        ];
        for (away, keep_awake_on, want) in cases {
            assert_eq!(flip(away, keep_awake_on), want, "away {away}, awake {keep_awake_on}");
        }
    }
}
