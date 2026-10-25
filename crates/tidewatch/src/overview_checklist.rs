//! The checklist of Settings, General: the four stages, or the must-do items of the list in use when
//! there is one, where the list comes from and the file Edit opens. Read on a job thread, again while
//! Settings shows.

use std::path::{Path, PathBuf};

use tidewatch_core::checklist;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::progress::checklist_phases;
use tidewatch_core::stages::Stage;
use tidewatch_core::test_command::checklist_path;

use crate::jobs::Jobs;

const REFRESH_SECS: f64 = 3.0;
/// Items shown as chips before the rest fold into one `+N more`.
const MAX_ITEM_CHIPS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Default,
    Yours,
    Repo,
}

impl Source {
    pub(crate) fn caption(self) -> &'static str {
        match self {
            Self::Default => "Four stages \u{b7} default",
            Self::Yours => "Your own list",
            Self::Repo => "This repo's list",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Chip {
    pub(crate) number: Option<u32>,
    pub(crate) title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Checklist {
    pub(crate) chips: Vec<Chip>,
    pub(crate) source: Source,
    /// The file Edit opens: the repo's own, else `checklist.md` in the settings folder.
    pub(crate) file: PathBuf,
}

fn stage_chips() -> Vec<Chip> {
    (1..)
        .zip(Stage::ALL)
        .map(|(number, stage)| Chip { number: Some(number), title: stage.label().to_owned() })
        .collect()
}

fn item_chips(text: &str) -> Vec<Chip> {
    let items: Vec<String> = checklist_phases(text)
        .into_iter()
        .flat_map(|phase| phase.items)
        .map(|item| item.text)
        .collect();
    let hidden = items.len().saturating_sub(MAX_ITEM_CHIPS);
    let more = (hidden > 0).then(|| format!("+{hidden} more"));
    items
        .into_iter()
        .take(MAX_ITEM_CHIPS)
        .chain(more)
        .map(|title| Chip { number: None, title })
        .collect()
}

pub(crate) fn load(paths: &TidewatchPaths, repo: &Path) -> Checklist {
    let own = checklist_path(repo).filter(|file| checklist::must_dos(file).is_some());
    let config = paths.checklist_file();
    let (source, file) = match own {
        Some(file) => (Source::Repo, file),
        None if checklist::must_dos(&config).is_some() => (Source::Yours, config),
        None => (Source::Default, config),
    };
    let chips = checklist::for_repo(paths, repo)
        .map(|text| item_chips(&text))
        .filter(|chips| !chips.is_empty())
        .unwrap_or_else(stage_chips);
    Checklist { chips, source, file }
}

pub(crate) struct Host {
    jobs: Jobs<(PathBuf, Checklist)>,
    current: Option<(PathBuf, Checklist)>,
    asked: Option<(PathBuf, f64)>,
}

impl Host {
    pub(crate) fn new() -> Self {
        Self { jobs: Jobs::new(), current: None, asked: None }
    }

    /// Asks for a fresh read when Settings shows for a repo not read yet, or the last read is
    /// old, and takes what finished.
    pub(crate) fn tick(
        &mut self,
        paths: Option<&TidewatchPaths>,
        repo: &Path,
        shown: bool,
        now: f64,
        ctx: &egui::Context,
    ) {
        if let Some(done) = self.jobs.drain().pop() {
            self.current = Some(done);
        }
        let Some(paths) = paths.filter(|_| shown) else { return };
        let stale =
            self.asked.as_ref().is_none_or(|(asked, at)| asked != repo || now - at >= REFRESH_SECS);
        if stale {
            self.asked = Some((repo.to_path_buf(), now));
            let (paths, repo) = (paths.clone(), repo.to_path_buf());
            self.jobs.spawn(ctx, move || {
                let checklist = load(&paths, &repo);
                (repo, checklist)
            });
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(REFRESH_SECS + 0.1));
        }
    }

    pub(crate) fn of(&self, repo: &Path) -> Option<&Checklist> {
        self.current.as_ref().filter(|(known, _)| known == repo).map(|(_, checklist)| checklist)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn home() -> (tempfile::TempDir, TidewatchPaths) {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        (home, paths)
    }

    fn titles(list: &Checklist) -> Vec<String> {
        list.chips.iter().map(|chip| chip.title.clone()).collect()
    }

    #[test]
    fn with_no_file_the_four_stages_show_and_edit_opens_the_settings_file() {
        let (_home, paths) = home();
        let repo = tempfile::tempdir().unwrap();
        let list = load(&paths, repo.path());
        assert_eq!(list.source, Source::Default);
        assert_eq!(list.file, paths.checklist_file());
        assert_eq!(titles(&list), ["Understand", "Build", "PR", "Done"]);
        assert_eq!(
            list.chips.iter().map(|chip| chip.number).collect::<Vec<_>>(),
            [Some(1), Some(2), Some(3), Some(4)]
        );
        assert_eq!(Source::Default.caption(), "Four stages \u{b7} default");
    }

    #[test]
    fn the_installed_default_is_still_the_default() {
        let (_home, paths) = home();
        checklist::ensure(&paths).unwrap();
        let repo = tempfile::tempdir().unwrap();
        let list = load(&paths, repo.path());
        assert_eq!((list.source, list.file), (Source::Default, paths.checklist_file()));
        assert_eq!(list.chips.len(), 4);
    }

    #[test]
    fn a_list_with_many_items_shows_eight_chips_and_a_count_of_the_rest() {
        let (_home, paths) = home();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        let items = (1..=11).map(|n| format!("- [ ] step {n}\n")).collect::<Vec<_>>().concat();
        std::fs::write(paths.checklist_file(), items).unwrap();
        let repo = tempfile::tempdir().unwrap();
        let list = load(&paths, repo.path());
        let shown = titles(&list);
        assert_eq!(shown.len(), 9);
        assert_eq!(
            (shown[0].as_str(), shown[7].as_str(), shown[8].as_str()),
            ("step 1", "step 8", "+3 more")
        );
        assert!(list.chips.iter().all(|chip| chip.number.is_none()));
    }

    #[test]
    fn a_list_without_items_still_shows_the_stages() {
        let (_home, paths) = home();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.checklist_file(), "Always ask before deleting anything.\n").unwrap();
        let repo = tempfile::tempdir().unwrap();
        let list = load(&paths, repo.path());
        assert_eq!((list.source, list.chips.len()), (Source::Yours, 4));
    }

    #[test]
    fn an_edited_settings_file_is_yours_and_a_repo_file_wins_and_is_what_edit_opens() {
        let (_home, paths) = home();
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(paths.checklist_file(), "### 1. Plan\n- [ ] a\n### 2. Ship\n- [ ] b\n")
            .unwrap();
        let repo = tempfile::tempdir().unwrap();
        let yours = load(&paths, repo.path());
        assert_eq!(
            (yours.source, titles(&yours)),
            (Source::Yours, vec!["a".to_owned(), "b".to_owned()])
        );
        assert_eq!(yours.file, paths.checklist_file());

        std::fs::write(repo.path().join(".tidewatch.toml"), "checklist = \"docs/list.md\"\n")
            .unwrap();
        std::fs::create_dir_all(repo.path().join("docs")).unwrap();
        std::fs::write(repo.path().join("docs/list.md"), "### 1. Only\n- [ ] x\n").unwrap();
        let own = load(&paths, repo.path());
        assert_eq!((own.source, titles(&own)), (Source::Repo, vec!["x".to_owned()]));
        assert_eq!(own.file, repo.path().join("docs/list.md"));
    }

    #[test]
    fn an_empty_or_missing_repo_file_falls_through_to_the_next() {
        let (_home, paths) = home();
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join(".tidewatch.toml"), "checklist = \"list.md\"\n").unwrap();
        assert_eq!(load(&paths, repo.path()).source, Source::Default);
        std::fs::write(repo.path().join("list.md"), "  \n").unwrap();
        assert_eq!(load(&paths, repo.path()).source, Source::Default);
        assert_eq!(Source::Repo.caption(), "This repo's list");
    }

    #[test]
    fn the_host_reads_once_for_a_repo_then_again_after_a_few_seconds_and_only_while_shown() {
        let (_home, paths) = home();
        let repo = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut host = Host::new();
        host.tick(Some(&paths), repo.path(), false, 0.0, &ctx);
        assert!(host.of(repo.path()).is_none(), "nothing is read while the Overview is closed");
        host.tick(Some(&paths), repo.path(), true, 1.0, &ctx);
        let deadline = Instant::now() + Duration::from_secs(5);
        while host.of(repo.path()).is_none() && Instant::now() < deadline {
            host.tick(Some(&paths), repo.path(), true, 1.5, &ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(host.of(repo.path()).unwrap().chips.len(), 4);
        assert!(host.of(Path::new("/elsewhere")).is_none());
        assert_eq!(host.asked.as_ref().unwrap().1, 1.0, "the second tick did not ask again");
    }
}
