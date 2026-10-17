use tidewatch_core::profiles::Profiles;

use super::*;

#[test]
fn cards_sit_in_two_columns_with_the_same_gap_both_ways() {
    let rects = grid(pos2(10.0, 20.0), 408.0, 2, 5, 50.0);
    assert_eq!(rects.len(), 5);
    assert_eq!(rects[0], Rect::from_min_size(pos2(10.0, 20.0), vec2(200.0, 50.0)));
    assert_eq!(rects[1].left(), 10.0 + 200.0 + GAP);
    assert_eq!(rects[1].right(), 10.0 + 408.0);
    assert_eq!(rects[2].top(), 20.0 + 50.0 + GAP);
    assert_eq!(rects[4].left(), 10.0);
    assert_eq!(grid(pos2(0.0, 0.0), 100.0, 2, 0, 50.0), Vec::<Rect>::new());
}

#[test]
fn the_height_of_a_grid_is_its_rows_and_the_gaps_between_them() {
    let cases = [(0, 0.0), (1, 52.0), (2, 52.0), (3, 112.0), (8, 52.0 * 4.0 + 3.0 * GAP)];
    for (count, want) in cases {
        assert_eq!(agents_height(count), want, "{count} cards");
    }
    assert_eq!(workflows_height(9), 5.0 * WORKFLOW_H + 4.0 * GAP);
    assert_eq!(stages_height(4), STAGE_H);
    assert_eq!(stages_height(5), 2.0 * STAGE_H + GAP);
}

#[test]
fn the_subline_names_the_cli_the_model_and_the_effort() {
    let profiles = Profiles::seeded(&["claude".to_owned(), "codex".to_owned()]).profile;
    assert_eq!(subline(&profiles["builder"], "Claude Code"), "Claude Code \u{b7} opus \u{b7} high");
    assert_eq!(subline(&profiles["reviewer"], "Claude Code"), "Claude Code \u{b7} sonnet");
    let codex = profiles.values().find(|p| p.agent == "codex").unwrap();
    assert_eq!(subline(codex, "Codex"), "Codex \u{b7} default model");
}

#[test]
fn a_cli_that_is_not_installed_says_so_instead_of_its_model() {
    let profiles = Profiles::seeded(&["claude".to_owned()]);
    let agent = profiles.agent_table().get("claude").unwrap().clone();
    let profile = &profiles.profile["builder"];
    let cases = [
        (Availability::Missing, "Claude Code \u{b7} not installed"),
        (Availability::Probing, "Claude Code \u{b7} opus \u{b7} high"),
        (Availability::Found(None), "Claude Code \u{b7} opus \u{b7} high"),
    ];
    for (availability, want) in cases {
        assert_eq!(card_subline(profile, &agent, &availability), want, "{availability:?}");
    }
}

#[test]
fn chips_that_do_not_fit_one_row_give_way_to_a_count() {
    let widths = [40.0, 40.0, 40.0, 40.0];
    let (arrow, more) = (8.0, 20.0);
    let between = arrow + 2.0 * CHIP_GAP;
    let all = 160.0 + 3.0 * between;
    let cases = [
        (all + 1.0, 4),
        (all, 4),
        (all - 1.0, 3),
        (2.0 * 40.0 + between + between + more, 2),
        (40.0, 0),
        (0.0, 0),
    ];
    for (room, want) in cases {
        assert_eq!(chips_that_fit(&widths, arrow, more, room), want, "room {room}");
    }
    assert_eq!(chips_that_fit(&[], arrow, more, 10.0), 0);
}

#[test]
fn an_empty_list_says_why() {
    assert_eq!(empty_text("zz", true, "profile"), "No profile matches \"zz\".");
    assert!(
        empty_text("", true, "workflow").starts_with("No workflows yet. Add one in Settings (")
    );
    assert_eq!(empty_text("", false, "profile"), "The profiles are still loading...");
}
