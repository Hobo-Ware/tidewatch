use super::*;

const NOW: f64 = 1_000_000.0;

fn run(state: State, event: &Event, now: f64, lid: bool) -> (State, Vec<Effect>) {
    step(state, event, now, lid)
}

fn on(until: Option<f64>, told: u32) -> State {
    State::On { until, told }
}

#[test]
fn picking_on_a_mac_asks_for_the_password_before_anything_else() {
    let (state, effects) = run(State::Off, &Event::Pick(Choice::Hour), NOW, true);
    assert_eq!(state, State::Starting { choice: Choice::Hour });
    assert_eq!(effects, [Effect::StartLid { until: Some(NOW + 3600.0) }]);
}

#[test]
fn an_answered_prompt_turns_it_on_from_that_moment() {
    let starting = State::Starting { choice: Choice::HalfHour };
    let (state, effects) = run(starting, &Event::Started, NOW + 20.0, true);
    let until = NOW + 20.0 + 1800.0;
    assert_eq!(state, on(Some(until), 1));
    assert_eq!(
        effects,
        [
            Effect::TakeHold,
            Effect::WriteDeadline(Some(until)),
            Effect::Say(Voice::Start),
            Effect::ConfirmOn
        ]
    );
}

#[test]
fn a_start_pmset_never_confirms_flips_back_with_the_reason() {
    let (state, effects) = run(on(Some(NOW + 100.0), 1), &Event::Unconfirmed, NOW, true);
    assert_eq!(state, State::Off);
    assert_eq!(
        effects,
        [
            Effect::ReleaseHold,
            Effect::Abandon,
            Effect::Say(Voice::NotStarted("macOS did not turn sleep off".to_owned()))
        ]
    );
}

#[test]
fn a_refused_prompt_leaves_it_off_and_says_why() {
    let starting = State::Starting { choice: Choice::Open };
    let (state, effects) = run(starting, &Event::StartFailed("cancelled".to_owned()), NOW, true);
    assert_eq!(state, State::Off);
    assert_eq!(effects, [Effect::Say(Voice::NotStarted("cancelled".to_owned()))]);
}

#[test]
fn without_lid_mode_there_is_no_prompt_and_no_waiting() {
    let (state, effects) = run(State::Off, &Event::Pick(Choice::FourHours), NOW, false);
    assert_eq!(state, on(Some(NOW + 4.0 * 3600.0), 4));
    assert_eq!(effects, [Effect::TakeHold, Effect::Say(Voice::Start)]);
    let (state, effects) = run(state, &Event::TurnOff, NOW + 5.0, false);
    assert_eq!(state, State::Off);
    assert_eq!(effects, [Effect::ReleaseHold, Effect::Say(Voice::End)]);
}

#[test]
fn turning_off_is_off_as_soon_as_the_stop_command_returns_and_sleep_is_checked_after() {
    let (state, effects) = run(on(None, 0), &Event::TurnOff, NOW, true);
    assert_eq!(state, State::Stopping);
    assert_eq!(effects, [Effect::ReleaseHold, Effect::StopLid, Effect::Say(Voice::End)]);
    let (state, effects) = run(state, &Event::Stopped, NOW, true);
    assert_eq!((state, effects), (State::Off, vec![Effect::ConfirmOff]));
    assert_eq!(run(state, &Event::Settled(true), NOW, true), (State::Off, vec![]));
    assert_eq!(
        run(state, &Event::Settled(false), NOW, true),
        (State::Off, vec![Effect::Say(Voice::StillOff)])
    );
}

#[test]
fn adding_an_hour_moves_the_deadline_and_the_token_but_asks_nothing() {
    let (state, effects) = run(on(Some(NOW + 600.0), 1), &Event::AddHour, NOW, true);
    assert_eq!(state, on(Some(NOW + 4200.0), 2));
    assert_eq!(effects, [Effect::WriteDeadline(Some(NOW + 4200.0))]);
    assert_eq!(run(on(None, 0), &Event::AddHour, NOW, true), (on(None, 0), vec![]));
}

#[test]
fn the_deadline_ends_the_session_like_turning_it_off() {
    let state = on(Some(NOW + 100.0), 1);
    assert_eq!(run(state, &Event::Tick, NOW + 99.0, true), (state, vec![]));
    let (after, effects) = run(state, &Event::Tick, NOW + 100.0, true);
    assert_eq!(after, State::Stopping);
    assert_eq!(effects, [Effect::ReleaseHold, Effect::StopLid, Effect::Say(Voice::End)]);
}

#[test]
fn each_full_hour_left_is_said_once_and_a_short_session_says_none() {
    let mut state = run(State::Off, &Event::Pick(Choice::FourHours), NOW, false).0;
    let mut said = Vec::new();
    for minute in 0..=240 {
        let (next, effects) = run(state, &Event::Tick, NOW + f64::from(minute) * 60.0, false);
        said.extend(effects.into_iter().filter_map(|effect| match effect {
            Effect::Say(Voice::HoursLeft(hours)) => Some((minute, hours)),
            _ => None,
        }));
        state = next;
    }
    assert_eq!(said, [(60, 3), (120, 2), (180, 1)]);

    let mut short = run(State::Off, &Event::Pick(Choice::Hour), NOW, false).0;
    for minute in 0..60 {
        let (next, effects) = run(short, &Event::Tick, NOW + f64::from(minute) * 60.0, false);
        assert_eq!(effects, []);
        short = next;
    }
}

#[test]
fn an_added_hour_is_announced_again_when_it_comes_round() {
    let state = on(Some(NOW + 2.0 * 3600.0 + 30.0), 3);
    let (state, _) = run(state, &Event::AddHour, NOW, false);
    let (_, effects) = run(state, &Event::Tick, NOW + 30.0, false);
    assert_eq!(effects, [Effect::Say(Voice::HoursLeft(3))]);
}

#[test]
fn events_that_make_no_sense_in_a_state_change_nothing() {
    let cases = [
        (State::Off, Event::TurnOff),
        (State::Off, Event::Started),
        (State::Off, Event::AddHour),
        (State::Off, Event::Settled(true)),
        (State::Off, Event::Unconfirmed),
        (State::Off, Event::Stopped),
        (State::Stopping, Event::Settled(false)),
        (on(None, 0), Event::Stopped),
        (State::Starting { choice: Choice::Hour }, Event::Unconfirmed),
        (State::Starting { choice: Choice::Hour }, Event::TurnOff),
        (State::Starting { choice: Choice::Hour }, Event::Pick(Choice::Open)),
        (State::Stopping, Event::Pick(Choice::Hour)),
        (State::Stopping, Event::Tick),
        (on(None, 0), Event::Pick(Choice::Hour)),
        (on(None, 0), Event::Tick),
    ];
    for (state, event) in cases {
        assert_eq!(run(state, &event, NOW, true), (state, vec![]), "{state:?} {event:?}");
    }
}

#[test]
fn the_choices_are_the_menu_in_order() {
    let labels: Vec<_> = Choice::ALL.iter().map(|choice| choice.label()).collect();
    assert_eq!(labels, ["30 min", "1 h", "4 h", "Until I turn it off"]);
    let seconds: Vec<_> = Choice::ALL.iter().map(|choice| choice.seconds()).collect();
    assert_eq!(seconds, [Some(1800.0), Some(3600.0), Some(14400.0), None]);
}

#[test]
fn the_voice_is_cosmic_horror_omen_by_omen() {
    let tags = |voice: &Voice| -> Vec<&'static str> {
        voice.lines().iter().map(|(omen, _)| omen.tag()).collect()
    };
    assert_eq!(tags(&Voice::Start), ["THE STARS ARE RIGHT", "SHOGGOTH"]);
    assert_eq!(tags(&Voice::End), ["ELDER SIGN", "R'LYEH SLEEPS"]);
    assert_eq!(tags(&Voice::HoursLeft(2)), ["NON-EUCLIDEAN HOURS"]);
    assert_eq!(Voice::HoursLeft(3).lines()[0].1, "Another non-Euclidean hour. 3 h remain.");
    assert_eq!(Voice::HoursLeft(1).lines()[0].1, "The last non-Euclidean hour. 1 h remains.");
    assert_eq!(Voice::Start.lines()[0].1, "The stars are right. Sleep will not claim this night.");
    assert_eq!(
        Voice::End.lines()[1].1,
        "That is not dead which can eternal lie. Session complete."
    );
    for voice in [
        Voice::Start,
        Voice::End,
        Voice::HoursLeft(2),
        Voice::StillOff,
        Voice::TouchIdReady,
        Voice::TouchIdGone,
        Voice::TouchIdFailed("cancelled".to_owned()),
    ] {
        for (_, text) in voice.lines() {
            assert!(!text.contains(['\u{2014}', '\u{2013}']), "{text}");
        }
    }
}

#[test]
fn time_left_counts_down_and_stops_at_zero() {
    assert_eq!(on(Some(NOW + 90.0), 1).left(NOW), Some(90.0));
    assert_eq!(on(Some(NOW + 90.0), 1).left(NOW + 100.0), Some(0.0));
    assert_eq!(on(None, 0).left(NOW), None);
    assert_eq!(State::Off.left(NOW), None);
    assert!(on(None, 0).is_on() && !State::Stopping.is_on());
    assert!(State::Stopping.busy() && !State::Off.busy());
}

#[test]
fn hours_left_rounds_up_so_the_last_minutes_still_say_one_hour() {
    let cases = [(0.0, 0), (1.0, 1), (3600.0, 1), (3601.0, 2), (4.0 * 3600.0, 4)];
    for (seconds, hours) in cases {
        assert_eq!(hours_left(seconds), hours, "{seconds}");
    }
}
