use super::*;

#[test]
fn a_line_that_names_a_removed_tool_is_found_with_its_number() {
    let text = "# House style\n\nCall `stage_done` when the stage is finished.\nKeep it short.\n";
    let found = scan(text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].number, 3);
    assert_eq!(found[0].text, "Call `stage_done` when the stage is finished.");
    assert_eq!(found[0].tool.name, "stage_done");
}

#[test]
fn the_client_side_name_counts_and_a_longer_word_does_not() {
    let cases = [
        ("call mcp__tidewatch__stage_done now", true),
        ("call the tidewatch stage_done tool", true),
        ("(stage_done)", true),
        ("use propose_workflow first", true),
        ("my_stage_done helper", false),
        ("stage_done_twice", false),
        ("stage_donee", false),
        ("a stage is done", false),
        ("update_progress and get_progress", false),
    ];
    for (line, dead) in cases {
        assert_eq!(scan(line).len(), usize::from(dead), "{line:?}");
    }
}

#[test]
fn a_clean_file_has_nothing_to_remove() {
    assert_eq!(scan("Call update_progress at milestones.\n"), Vec::new());
    assert_eq!(scan(""), Vec::new());
}

#[test]
fn removing_a_line_keeps_every_other_byte() {
    let text = "one\r\ntwo `stage_done`\r\nthree\nfour";
    let found = scan(text);
    assert_eq!(found[0].number, 2);
    assert_eq!(remove_line(text, 2, &found[0].text).as_deref(), Some("one\r\nthree\nfour"));
    let last = "keep\nstage_done";
    assert_eq!(remove_line(last, 2, "stage_done").as_deref(), Some("keep\n"));
}

#[test]
fn a_line_that_changed_since_the_scan_is_not_removed() {
    let text = "one\nCall stage_done\nthree\n";
    assert_eq!(remove_line(text, 2, "Call something else"), None);
    assert_eq!(remove_line(text, 9, "Call stage_done"), None);
    assert_eq!(remove_line(text, 0, "one"), None);
}

#[test]
fn the_file_is_rewritten_only_when_the_line_still_matches() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("house-style.md");
    std::fs::write(&file, "a\nCall stage_done\nb\n").unwrap();
    assert!(!remove_from_file(&file, 2, "Call something else").unwrap());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "a\nCall stage_done\nb\n");
    assert!(remove_from_file(&file, 2, "Call stage_done").unwrap());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "a\nb\n");
}

#[test]
fn no_removed_tool_is_offered_to_any_role() {
    use crate::mcp::{TabRole, tool_definitions_for};
    for role in [TabRole::Agent, TabRole::Meeseeks] {
        let offered = tool_definitions_for(role).to_string();
        for tool in REMOVED {
            assert!(!offered.contains(tool.name), "{} is offered to {role:?}", tool.name);
        }
    }
}
