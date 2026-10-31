use tidewatch_core::panel_view::{ReportMark, Why};

use super::*;
use crate::panel_blocks::{Blocks, Kept, ticked_by_tidewatch};

const NOW: i64 = 1_800_000_000_000;

fn blocks_of(fake: Fake) -> Blocks {
    Blocks::of(&fake.view(NOW), NOW, &jiff::tz::TimeZone::UTC, Kept::default())
}

#[test]
fn the_switch_names_four_views_and_nothing_else() {
    let cases = [
        (Some("fresh"), Some(Fake::Fresh)),
        (Some(" outdated "), Some(Fake::Outdated)),
        (Some("quiet"), Some(Fake::Quiet)),
        (Some("since"), Some(Fake::Since)),
        (Some("answers"), Some(Fake::Answers)),
        (Some("lastturn"), Some(Fake::LastTurn)),
        (Some("stale"), None),
        (Some(""), None),
        (None, None),
    ];
    for (text, want) in cases {
        assert_eq!(Fake::from_text(text), want, "{text:?}");
    }
}

#[test]
fn each_view_shows_what_its_name_says() {
    let fresh = blocks_of(Fake::Fresh);
    assert_eq!(
        (fresh.out_of_date, fresh.report.as_deref(), fresh.since),
        (None, Some("2m ago"), None)
    );

    let outdated = Fake::Outdated.view(NOW);
    assert_eq!(outdated.report, Some(ReportMark::OutOfDate(Why::WorkingButTurnEnded)));
    assert_eq!(outdated.reported, Some(State::Done));
    assert_eq!(blocks_of(Fake::Outdated).report.as_deref(), Some("14m ago"));

    let quiet = blocks_of(Fake::Quiet);
    assert!(quiet.turn.as_deref().is_some_and(|text| text.starts_with("no word since ")));

    let since = blocks_of(Fake::Since);
    assert_eq!(since.since.as_deref(), Some("3 things happened since"));
    assert_eq!(blocks_of(Fake::Answers).since.as_deref(), Some("3 answers since"));
    let last = Fake::LastTurn.view(NOW);
    assert_eq!(last.report, Some(ReportMark::OutOfDate(Why::DoneButTurnRunning)));
    assert_eq!(last.reported, Some(State::Working));
}

#[test]
fn only_the_since_view_has_a_step_ticked_by_tidewatch() {
    for fake in [Fake::Fresh, Fake::Outdated, Fake::Quiet] {
        assert_eq!(fake.marks(NOW), Vec::<MarkSeen>::new());
    }
    assert!(ticked_by_tidewatch(&Fake::Since.marks(NOW), TICKED_STEP));
    assert!(ticked_by_tidewatch(&Fake::Answers.marks(NOW), TICKED_STEP));
}

#[test]
fn the_fake_percent_is_read_as_a_number_or_ignored() {
    let read = |text: &str| text.trim().parse::<f64>().ok().filter(|p| p.is_finite());
    assert_eq!(read(" 99.96 "), Some(99.96));
    assert_eq!(read("lots"), None);
    assert_eq!(read("NaN"), None);
}
