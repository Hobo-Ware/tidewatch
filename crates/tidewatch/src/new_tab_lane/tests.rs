use std::ffi::OsString;
use std::path::PathBuf;

use tidewatch_core::profiles::Profiles;
use tidewatch_core::workflows::Workflows;

use super::*;
use crate::new_tab::Group;

fn catalog() -> Catalog {
    let mut catalog = Catalog::new(OsString::from("/usr/bin"));
    catalog.set_profiles(Profiles::seeded(&[
        "claude".to_owned(),
        "codex".to_owned(),
        "gemini".to_owned(),
    ]));
    catalog.set_workflows(Workflows::defaults());
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
    let cases = [
        (&profiles["builder"], Availability::Found(None), "opus \u{b7} high"),
        (&profiles["builder"], Availability::Probing, "opus \u{b7} high"),
        (&profiles["reviewer"], found.clone(), "sonnet"),
        (codex, found, "default model"),
        (&profiles["builder"], Availability::Missing, "not installed"),
    ];
    for (profile, availability, want) in cases {
        assert_eq!(subtitle(profile, &availability), want, "{availability:?}");
    }
}

#[test]
fn stages_read_as_one_line() {
    assert_eq!(stages_line(&["Plan", "Build", "Review"]), "Plan \u{203a} Build \u{203a} Review");
    assert_eq!(stages_line(&["Plan"]), "Plan");
    assert_eq!(stages_line(&[]), "");
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
    assert_eq!(builder.badge, Badge::Cli("claude".to_owned()));
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
fn workflows_are_a_flat_list_with_their_stages_as_the_subtitle() {
    let catalog = catalog();
    let mut state = form();
    state.choose(Kind::Workflow);
    let entries = entries(&state, &catalog);
    assert!(entries.iter().all(|entry| matches!(entry, Entry::Row(_))));
    let rows = rows(&entries);
    assert_eq!(rows[0].id, "ship");
    assert_eq!(rows[0].title, "Ship a feature");
    assert_eq!(rows[0].subtitle, "Work \u{203a} Review \u{203a} Refactor \u{203a} Hand off");
    assert_eq!(rows[0].badge, Badge::Stages(4));
    let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids, state.visible(&catalog));
}

#[test]
fn no_profiles_loaded_means_no_rows() {
    let catalog = Catalog::new(OsString::from("/usr/bin"));
    assert_eq!(super::entries(&form(), &catalog), Vec::new());
}
