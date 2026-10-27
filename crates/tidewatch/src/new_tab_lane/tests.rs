use std::ffi::OsString;
use std::path::PathBuf;

use tidewatch_core::agents::Level;
use tidewatch_core::profiles::Profiles;

use super::*;
use crate::new_tab::{Group, Kind};

fn catalog() -> Catalog {
    let mut catalog = Catalog::new(OsString::from("/usr/bin"));
    catalog.set_profiles(Profiles::seeded(&[
        "claude".to_owned(),
        "codex".to_owned(),
        "gemini".to_owned(),
    ]));
    catalog.set_availability("claude", Availability::Found(Some("1.0.0".to_owned())));
    catalog.set_availability("codex", Availability::Missing);
    catalog
}

fn form() -> NewTab {
    NewTab::open(Group::Repo, "web".into(), PathBuf::from("/work/web"), Vec::new(), None, 3)
}

fn rows(entries: &[Entry]) -> Vec<&Row> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Row(row) => Some(row),
            Entry::Group(_) => None,
        })
        .collect()
}

#[test]
fn the_subtitle_is_the_model_and_effort_or_says_the_cli_is_missing() {
    let profiles = Profiles::seeded(&["claude".to_owned(), "codex".to_owned()]).profile;
    let codex = profiles.values().find(|profile| profile.agent == "codex").unwrap();
    let found = Availability::Found(None);
    let unset = Profile { effort: None, ..profiles["reviewer"].clone() };
    let cases = [
        (&unset, found.clone(), "sonnet \u{b7} default (high)"),
        (&profiles["builder"], Availability::Found(None), "opus \u{b7} medium"),
        (&profiles["builder"], Availability::Probing, "opus \u{b7} medium"),
        (&profiles["reviewer"], found.clone(), "sonnet \u{b7} high"),
        (codex, found, "default model"),
        (&profiles["builder"], Availability::Missing, "not installed"),
    ];
    for (profile, availability, want) in cases {
        assert_eq!(subtitle(profile, &availability), want, "{availability:?}");
    }
}

#[test]
fn profiles_are_listed_by_cli_with_one_group_label_each_and_keys_that_count_the_rows() {
    let catalog = catalog();
    let state = form();
    let entries = entries(&state, &catalog);
    let groups: Vec<&str> = entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Group(label) => Some(label.as_str()),
            Entry::Row(_) => None,
        })
        .collect();
    assert_eq!(groups, ["Claude Code", "Codex", "Gemini CLI"]);
    let rows = rows(&entries);
    let indexes: Vec<usize> = rows.iter().map(|row| row.index).collect();
    assert_eq!(indexes, (0..rows.len()).collect::<Vec<_>>());
    let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids, state.visible(&catalog));
    assert!(matches!(entries[0], Entry::Group(_)));
}

#[test]
fn a_cli_that_is_not_installed_is_dimmed_and_says_so() {
    let catalog = catalog();
    let entries = entries(&form(), &catalog);
    let rows = rows(&entries);
    let builder = rows.iter().find(|row| row.id == "builder").unwrap();
    assert!(!builder.dim);
    assert_eq!(builder.cli, "claude");
    let codex = rows.iter().find(|row| row.id == "codex-builder").unwrap();
    assert!(codex.dim);
    assert_eq!(codex.subtitle, "not installed");
}

#[test]
fn the_filter_leaves_only_the_groups_that_still_have_a_row() {
    let catalog = catalog();
    let mut state = form();
    state.set_filter("reviewer", &catalog);
    let entries = entries(&state, &catalog);
    assert_eq!(rows(&entries).len(), 3);
    assert_eq!(entries.iter().filter(|entry| matches!(entry, Entry::Group(_))).count(), 3);
    state.set_filter("zzz", &catalog);
    assert_eq!(super::entries(&state, &catalog), Vec::new());
}

#[test]
fn no_profiles_loaded_means_no_rows() {
    let catalog = Catalog::new(OsString::from("/usr/bin"));
    assert_eq!(super::entries(&form(), &catalog), Vec::new());
}

#[test]
fn a_meeseeks_lane_lists_each_cli_once_with_its_builder_or_defaults_and_dims_the_missing_ones() {
    let catalog = catalog();
    let mut state = form();
    state.settle(&catalog);
    assert_eq!(state.choose(Kind::Meeseeks), None);
    let entries = entries(&state, &catalog);
    assert!(entries.iter().all(|entry| matches!(entry, Entry::Row(_))), "no groups");
    let found: Vec<(usize, &str, &str, &str, bool)> = rows(&entries)
        .iter()
        .map(|row| {
            (row.index, row.title.as_str(), row.subtitle.as_str(), row.cli.as_str(), row.dim)
        })
        .collect();
    let want = [
        (0, "Claude Code", "Builder \u{b7} opus \u{b7} medium", "claude", false),
        (1, "Codex", "not installed", "codex", true),
        (2, "Gemini CLI", "Builder \u{b7} default model", "gemini", false),
        (3, "Copilot CLI", "default model", "copilot", false),
    ];
    assert_eq!(found, want);
}

#[test]
fn the_meeseeks_subtitle_names_the_profile_unless_the_cli_runs_on_defaults() {
    let profile =
        Profile { model: Some("opus".into()), ..Profile::defaults_for("claude", Level::Yolo) };
    let found = Availability::Found(None);
    let cases = [
        ("Claude Code", "builder", &found, "Default \u{b7} opus \u{b7} default (medium)"),
        ("Claude Code", Profile::DEFAULTS_ID, &found, "opus \u{b7} default (medium)"),
        ("Claude Code", "builder", &Availability::Missing, "not installed"),
        ("Claude Code", Profile::DEFAULTS_ID, &Availability::Missing, "not installed"),
    ];
    for (cli, key, availability, want) in cases {
        assert_eq!(
            meeseeks_subtitle(cli, key, &profile, availability),
            want,
            "{key} {availability:?}"
        );
    }
    let prefixed = Profile { name: "Claude Code Builder".into(), ..profile };
    let found = Availability::Found(None);
    assert_eq!(
        meeseeks_subtitle("Claude Code", "claude-builder", &prefixed, &found),
        "Builder \u{b7} opus \u{b7} default (medium)"
    );
}
