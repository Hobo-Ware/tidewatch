//! Built-in workflows that arrived after a user's `workflows.toml` was written, appended once.
//! `workflows.offered` beside the file lists the built-in ids already offered, one per line.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;

use crate::agents::AgentTable;
use crate::fsutil::write_atomic;
use crate::paths::TidewatchPaths;
use crate::workflows::{BUILT_IN_IDS, ORIGINAL_IDS, Workflows};

/// What an upgrade does to a file that exists: the built-ins to append and the ids to list as offered.
#[derive(Debug, PartialEq, Eq)]
struct Plan {
    append: Workflows,
    offered: BTreeSet<String>,
}

fn read_offered(marker: &Path) -> BTreeSet<String> {
    let mut offered: BTreeSet<String> = ORIGINAL_IDS.iter().map(|id| (*id).to_owned()).collect();
    if let Ok(text) = std::fs::read_to_string(marker) {
        offered
            .extend(text.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned));
    }
    offered
}

fn plan(existing: &Workflows, offered: &BTreeSet<String>, built_in: &Workflows) -> Plan {
    let mut plan = Plan { append: Workflows::default(), offered: offered.clone() };
    for (id, workflow) in &built_in.workflow {
        if offered.contains(id) {
            continue;
        }
        plan.offered.insert(id.clone());
        if !existing.workflow.contains_key(id) {
            plan.append.workflow.insert(id.clone(), workflow.clone());
        }
    }
    plan
}

/// `text` with the workflows in `append` added after it, or `None` when the result would not parse
/// back to the same workflows plus the new ones.
fn appended(text: &str, existing: &Workflows, append: &Workflows) -> Option<String> {
    let tables = toml::to_string(append).ok()?;
    let separator = if text.is_empty() || text.ends_with('\n') { "\n" } else { "\n\n" };
    let combined = format!("{text}{separator}{tables}");
    let parsed = Workflows::parse(&combined).ok()?;
    let expected = existing.workflow.len() + append.workflow.len();
    let intact = parsed.workflow.len() == expected
        && append.workflow.iter().all(|(id, workflow)| parsed.workflow.get(id) == Some(workflow));
    intact.then_some(combined)
}

fn write_offered(paths: &TidewatchPaths, offered: &BTreeSet<String>) {
    let text = offered.iter().map(String::as_str).collect::<Vec<_>>().join("\n") + "\n";
    let marker = paths.workflows_offered_file();
    if let Err(e) = write_atomic(&marker, &text) {
        eprintln!("tidewatch: cannot write {} ({e})", marker.display());
    }
}

/// A file Tidewatch has just written holds every built-in, so all of them count as offered.
pub(crate) fn record_all_offered(paths: &TidewatchPaths) {
    write_offered(paths, &BUILT_IN_IDS.iter().map(|id| (*id).to_owned()).collect());
}

/// Appends the built-ins this `workflows.toml` has never been offered. A file that cannot be read
/// or parsed is left alone and not marked, so fixing it later still brings them in.
pub(crate) fn add_new_built_ins(paths: &TidewatchPaths, path_var: Option<&OsStr>) {
    let file = paths.workflows_file();
    let offered = read_offered(&paths.workflows_offered_file());
    if BUILT_IN_IDS.iter().all(|id| offered.contains(*id)) {
        return;
    }
    let Ok(text) = std::fs::read_to_string(&file) else { return };
    let Ok(existing) = Workflows::parse(&text) else { return };
    let built_in = Workflows::seeded(&AgentTable::builtin().found_on_path(path_var));
    let plan = plan(&existing, &offered, &built_in);
    if !plan.append.workflow.is_empty() {
        let Some(combined) = appended(&text, &existing, &plan.append) else { return };
        if let Err(e) = write_atomic(&file, &combined) {
            eprintln!("tidewatch: cannot write {} ({e})", file.display());
            return;
        }
    }
    write_offered(paths, &plan.offered);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offered(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    fn ids(workflows: &Workflows) -> Vec<&str> {
        workflows.workflow.keys().map(String::as_str).collect()
    }

    fn only(keep: &[&str]) -> Workflows {
        let mut workflows = Workflows::defaults();
        workflows.workflow.retain(|id, _| keep.contains(&id.as_str()));
        workflows
    }

    const NEW_IDS: [&str; 6] = ["finish", "port", "refactor", "release", "rootcause", "uiproof"];

    #[test]
    fn plan_appends_what_is_missing_and_unoffered() {
        let built_in = Workflows::defaults();
        let original = offered(&ORIGINAL_IDS);
        let cases = [
            (
                "an old file with the original three",
                only(&ORIGINAL_IDS),
                original.clone(),
                NEW_IDS.to_vec(),
            ),
            (
                "an old file that deleted pr",
                only(&["ship", "fix"]),
                original.clone(),
                NEW_IDS.to_vec(),
            ),
            (
                "a file that deleted every new one, all offered",
                only(&["ship"]),
                offered(&BUILT_IN_IDS),
                Vec::new(),
            ),
            ("an empty file", Workflows::default(), original, NEW_IDS.to_vec()),
        ];
        for (label, existing, already, appended) in cases {
            let plan = plan(&existing, &already, &built_in);
            assert_eq!(ids(&plan.append), appended, "{label}");
            assert_eq!(plan.offered, offered(&BUILT_IN_IDS), "{label}");
        }
    }

    #[test]
    fn plan_never_replaces_a_workflow_the_user_already_has() {
        let built_in = Workflows::defaults();
        let mut existing = only(&ORIGINAL_IDS);
        let mut mine = built_in.workflow["finish"].clone();
        mine.name = "My finish".into();
        existing.workflow.insert("finish".into(), mine);
        let plan = plan(&existing, &offered(&ORIGINAL_IDS), &built_in);
        assert_eq!(ids(&plan.append), ["port", "refactor", "release", "rootcause", "uiproof"]);
        assert!(plan.offered.contains("finish"));
    }

    #[test]
    fn appending_keeps_the_users_text_byte_for_byte() {
        let text = "# my notes\n[[workflow.quick.stage]]\naction = \"work\"\nagent = \"claude\"";
        let existing = Workflows::parse(text).unwrap();
        let combined = appended(text, &existing, &only(&["finish"])).unwrap();
        assert!(combined.starts_with(text));
        let parsed = Workflows::parse(&combined).unwrap();
        assert_eq!(ids(&parsed), ["finish", "quick"]);
        assert_eq!(parsed.workflow["quick"], existing.workflow["quick"]);
    }

    #[test]
    fn appending_refuses_a_file_it_cannot_extend_cleanly() {
        let text = "workflow = { quick = { name = \"Quick\" } }\n";
        let existing = Workflows::parse(text).unwrap();
        assert_eq!(appended(text, &existing, &only(&["finish"])), None);
    }

    struct Home {
        dir: tempfile::TempDir,
    }

    impl Home {
        fn new() -> Self {
            Self { dir: tempfile::tempdir().unwrap() }
        }

        fn paths(&self) -> TidewatchPaths {
            TidewatchPaths::new(self.dir.path())
        }

        fn run(&self) -> Workflows {
            let nothing_on_path = std::ffi::OsString::new();
            Workflows::load_or_seed(&self.paths(), Some(&nothing_on_path))
        }

        fn write(&self, text: &str) {
            let file = self.paths().workflows_file();
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }

        fn text(&self) -> String {
            std::fs::read_to_string(self.paths().workflows_file()).unwrap()
        }

        fn offered(&self) -> BTreeSet<String> {
            read_offered(&self.paths().workflows_offered_file())
        }
    }

    #[test]
    fn a_first_launch_writes_every_built_in_and_marks_them_offered() {
        let home = Home::new();
        assert_eq!(ids(&home.run()).len(), BUILT_IN_IDS.len());
        assert_eq!(home.offered(), offered(&BUILT_IN_IDS));
    }

    #[test]
    fn an_older_file_gains_the_new_built_ins_once() {
        let home = Home::new();
        let old = toml::to_string(&only(&ORIGINAL_IDS)).unwrap();
        home.write(&old);

        let first = home.run();
        assert_eq!(ids(&first).len(), BUILT_IN_IDS.len());
        assert!(home.text().starts_with(&old));
        assert_eq!(Workflows::load(&home.paths().workflows_file()), first);
        assert_eq!(home.offered(), offered(&BUILT_IN_IDS));

        let after_first = home.text();
        home.run();
        assert_eq!(home.text(), after_first);
    }

    #[test]
    fn a_deleted_built_in_stays_deleted_and_an_edited_one_stays_edited() {
        let home = Home::new();
        home.write(&toml::to_string(&only(&ORIGINAL_IDS)).unwrap());
        home.run();

        let mut edited = Workflows::load(&home.paths().workflows_file());
        edited.workflow.remove("release");
        edited.workflow.remove("pr");
        edited.workflow.get_mut("finish").unwrap().name = "Wrap up".into();
        edited.save(&home.paths().workflows_file()).unwrap();

        let after = home.run();
        assert_eq!(after, edited);
        assert_eq!(after.workflow["finish"].name, "Wrap up");
        assert!(!after.workflow.contains_key("release"));
        assert!(!after.workflow.contains_key("pr"));
    }

    #[test]
    fn a_workflow_deleted_before_the_upgrade_does_not_come_back() {
        let home = Home::new();
        home.write(&toml::to_string(&only(&["ship"])).unwrap());
        let after = home.run();
        assert!(!after.workflow.contains_key("pr"));
        assert!(!after.workflow.contains_key("fix"));
        assert_eq!(after.workflow.len(), BUILT_IN_IDS.len() - 2);
    }

    #[test]
    fn a_file_that_does_not_parse_is_left_alone_and_not_marked() {
        let home = Home::new();
        let garbage = "[workflow.x\nnot toml {{";
        home.write(garbage);
        home.run();
        assert_eq!(home.text(), garbage);
        assert!(!home.paths().workflows_offered_file().exists());

        home.write(&toml::to_string(&only(&ORIGINAL_IDS)).unwrap());
        assert_eq!(ids(&home.run()).len(), BUILT_IN_IDS.len());
    }

    #[test]
    fn a_users_own_workflow_with_a_built_in_id_is_kept_as_written() {
        let home = Home::new();
        let mine = "[[workflow.release.stage]]\naction = \"work\"\nagent = \"claude\"\n";
        home.write(mine);
        let after = home.run();
        assert_eq!(after.workflow["release"].name, "release");
        assert_eq!(after.workflow["release"].stages.len(), 1);
        assert!(home.text().starts_with(mine));
    }
}
