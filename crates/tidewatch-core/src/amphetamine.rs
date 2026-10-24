//! Amphetamine, the keep-awake button, as a pure state machine: OS calls come back as [`Effect`]s,
//! and time (wall seconds) is passed in.

pub const HOUR_S: f64 = 3600.0;
pub const ADD_S: f64 = HOUR_S;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    HalfHour,
    Hour,
    FourHours,
    Open,
}

impl Choice {
    pub const ALL: [Self; 4] = [Self::HalfHour, Self::Hour, Self::FourHours, Self::Open];

    #[must_use]
    pub fn seconds(self) -> Option<f64> {
        match self {
            Self::HalfHour => Some(30.0 * 60.0),
            Self::Hour => Some(HOUR_S),
            Self::FourHours => Some(4.0 * HOUR_S),
            Self::Open => None,
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::HalfHour => "30 min",
            Self::Hour => "1 h",
            Self::FourHours => "4 h",
            Self::Open => "Until I turn it off",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    Off,
    Starting {
        choice: Choice,
    },
    /// `until` is the wall-clock deadline, `None` for no timer. `told` is the hour count last
    /// announced, so each full hour left is said once.
    On {
        until: Option<f64>,
        told: u32,
    },
    /// The stop command is running; sleep is confirmed back in the background after it.
    Stopping,
}

impl State {
    #[must_use]
    pub fn is_on(self) -> bool {
        matches!(self, Self::On { .. })
    }

    #[must_use]
    pub fn busy(self) -> bool {
        !matches!(self, Self::Off)
    }

    /// Seconds left on the timer, `None` when off or without one.
    #[must_use]
    pub fn left(self, now: f64) -> Option<f64> {
        match self {
            Self::On { until: Some(until), .. } => Some((until - now).max(0.0)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Pick(Choice),
    /// The start command answered and the helper runs.
    Started,
    /// `pmset` never showed sleep off after a start that reported success.
    Unconfirmed,
    /// The stop command returned.
    Stopped,
    StartFailed(String),
    AddHour,
    TurnOff,
    Tick,
    /// Sleep is back on (`true`), or could not be confirmed in time.
    Settled(bool),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Ask for the password, set the lid mode and start the helper, with this deadline in the token.
    StartLid {
        until: Option<f64>,
    },
    TakeHold,
    ReleaseHold,
    WriteDeadline(Option<f64>),
    /// Check in the background that `pmset` shows lid mode on.
    ConfirmOn,
    /// Drop the token so the helper lets go, with no check of what comes back.
    Abandon,
    StopLid,
    /// Check in the background that `pmset` shows sleep back on.
    ConfirmOff,
    Say(Voice),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Voice {
    Start,
    HoursLeft(u32),
    End,
    NotStarted(String),
    StillOff,
    TouchIdReady,
    TouchIdGone,
    TouchIdFailed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Omen {
    StarsRight,
    Shoggoth,
    Hours,
    ElderSign,
    Rlyeh,
    Plain,
}

impl Omen {
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::StarsRight => "THE STARS ARE RIGHT",
            Self::Shoggoth => "SHOGGOTH",
            Self::Hours => "NON-EUCLIDEAN HOURS",
            Self::ElderSign => "ELDER SIGN",
            Self::Rlyeh => "R'LYEH SLEEPS",
            Self::Plain => "",
        }
    }
}

impl Voice {
    /// The toast lines, each an omen tag and one short sentence.
    #[must_use]
    pub fn lines(&self) -> Vec<(Omen, String)> {
        let line = |omen, text: &str| (omen, text.to_owned());
        match self {
            Self::Start => vec![
                line(Omen::StarsRight, "The stars are right. Sleep will not claim this night."),
                line(Omen::Shoggoth, "Amphetamine engaged. The mask stays on."),
            ],
            Self::HoursLeft(1) => {
                vec![line(Omen::Hours, "The last non-Euclidean hour. 1 h remains.")]
            }
            Self::HoursLeft(hours) => {
                vec![(Omen::Hours, format!("Another non-Euclidean hour. {hours} h remain."))]
            }
            Self::End => vec![
                line(Omen::ElderSign, "The Old Ones stir. Restoring the sleep of mortals..."),
                line(Omen::Rlyeh, "That is not dead which can eternal lie. Session complete."),
            ],
            Self::NotStarted(why) => {
                vec![(Omen::Plain, format!("Amphetamine did not start: {why}"))]
            }
            Self::StillOff => {
                vec![line(Omen::Plain, "Sleep may still be off. Run: sudo pmset -a disablesleep 0")]
            }
            Self::TouchIdReady => {
                vec![line(
                    Omen::Plain,
                    "Touch ID is set up. Amphetamine asks for your fingerprint now.",
                )]
            }
            Self::TouchIdGone => vec![line(
                Omen::Plain,
                "Touch ID for Amphetamine is removed. It asks for your password again.",
            )],
            Self::TouchIdFailed(why) => {
                vec![(Omen::Plain, format!("Touch ID for Amphetamine was not changed: {why}"))]
            }
        }
    }
}

/// How many whole hours are left, rounded up: the count the hourly toast names.
#[must_use]
pub fn hours_left(seconds: f64) -> u32 {
    (seconds / HOUR_S).ceil().max(0.0) as u32
}

/// The next state and what to do about it. `lid` is whether this computer has lid mode (a Mac).
#[must_use]
pub fn step(state: State, event: &Event, now: f64, lid: bool) -> (State, Vec<Effect>) {
    match (state, event) {
        (State::Off, Event::Pick(choice)) => {
            let until = choice.seconds().map(|seconds| now + seconds);
            if lid {
                (State::Starting { choice: *choice }, vec![Effect::StartLid { until }])
            } else {
                (on(until, now), vec![Effect::TakeHold, Effect::Say(Voice::Start)])
            }
        }
        (State::Starting { choice }, Event::Started) => {
            let until = choice.seconds().map(|seconds| now + seconds);
            (
                on(until, now),
                vec![
                    Effect::TakeHold,
                    Effect::WriteDeadline(until),
                    Effect::Say(Voice::Start),
                    Effect::ConfirmOn,
                ],
            )
        }
        (State::On { .. }, Event::Unconfirmed) => (
            State::Off,
            vec![
                Effect::ReleaseHold,
                Effect::Abandon,
                Effect::Say(Voice::NotStarted("macOS did not turn sleep off".to_owned())),
            ],
        ),
        (State::Starting { .. }, Event::StartFailed(why)) => {
            (State::Off, vec![Effect::Say(Voice::NotStarted(why.clone()))])
        }
        (State::On { until: Some(until), .. }, Event::AddHour) => {
            let later = until + ADD_S;
            (
                State::On { until: Some(later), told: hours_left(later - now) },
                vec![Effect::WriteDeadline(Some(later))],
            )
        }
        (State::On { .. }, Event::TurnOff) => stop(lid),
        (State::On { until: Some(until), told }, Event::Tick) => {
            if now >= until {
                return stop(lid);
            }
            let mark = hours_left(until - now);
            if mark < told && mark >= 1 {
                (
                    State::On { until: Some(until), told: mark },
                    vec![Effect::Say(Voice::HoursLeft(mark))],
                )
            } else {
                (state, Vec::new())
            }
        }
        (State::Stopping, Event::Stopped) => (State::Off, vec![Effect::ConfirmOff]),
        (State::Off, Event::Settled(false)) => (State::Off, vec![Effect::Say(Voice::StillOff)]),
        _ => (state, Vec::new()),
    }
}

fn on(until: Option<f64>, now: f64) -> State {
    State::On { until, told: until.map_or(0, |until| hours_left(until - now)) }
}

fn stop(lid: bool) -> (State, Vec<Effect>) {
    if lid {
        (State::Stopping, vec![Effect::ReleaseHold, Effect::StopLid, Effect::Say(Voice::End)])
    } else {
        (State::Off, vec![Effect::ReleaseHold, Effect::Say(Voice::End)])
    }
}

#[cfg(test)]
mod tests;
