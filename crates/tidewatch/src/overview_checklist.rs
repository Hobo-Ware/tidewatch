//! The Overview's checklist row: the active template's phases, where the template comes from and
//! the file Edit opens. The template is read on a job thread, again while the Overview shows.

use std::path::{Path, PathBuf};

use tidewatch_core::checklist;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::progress::checklist_phases;
use tidewatch_core::test_command::checklist_path;

use crate::jobs::Jobs;

const MAX_TEMPLATE_BYTES: u64 = 16 * 1024;
const REFRESH_SECS: f64 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Default,
    Yours,
    Repo,
}

impl Source {
    pub(crate) fn caption(self) -> &'static str {
        match self {
            Self::Default => "Task to done \u{b7} default",
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

fn usable(file: &Path) -> bool {
    std::fs::metadata(file).is_ok_and(|meta| meta.len() <= MAX_TEMPLATE_BYTES)
        && std::fs::read_to_string(file).is_ok_and(|text| !text.trim().is_empty())
}

fn is_the_default(file: &Path) -> bool {
    std::fs::read_to_string(file)
        .is_ok_and(|text| text.trim() == checklist::DEFAULT_CHECKLIST.trim())
}

pub(crate) fn load(paths: &TidewatchPaths, repo: &Path) -> Checklist {
    let own = checklist_path(repo).filter(|file| usable(file));
    let config = paths.checklist_file();
    let (source, file) = match own {
        Some(file) => (Source::Repo, file),
        None if usable(&config) && !is_the_default(&config) => (Source::Yours, config),
        None => (Source::Default, config),
    };
    let chips = checklist_phases(&checklist::for_repo(paths, repo))
        .into_iter()
        .map(|phase| Chip { number: phase.number, title: phase.title })
        .collect();
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

    /// Asks for a fresh read when the Overview shows for a repo not read yet, or the last read is
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
    fn with_no_file_the_default_template_shows_and_edit_opens_the_settings_file() {
        let (_home, paths) = home();
        let repo = tempfile::tempdir().unwrap();
        let list = load(&paths, repo.path());
        assert_eq!(list.source, Source::Default);
        assert_eq!(list.file, paths.checklist_file());
        assert_eq!(list.chips.len(), 10);
        assert_eq!(
            (list.chips[0].number, list.chips[0].title.as_str()),
            (Some(1), "Understand and decide")
        );
    }

    #[test]
    fn the_installed_default_is_still_the_default() {
        let (_home, paths) = home();
        checklist::ensure(&paths).unwrap();
        let repo = tempfile::tempdir().unwrap();
        let list = load(&paths, repo.path());
        assert_eq!((list.source, list.file), (Source::Default, paths.checklist_file()));
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
            (Source::Yours, vec!["Plan".into(), "Ship".into()])
        );
        assert_eq!(yours.file, paths.checklist_file());

        std::fs::write(repo.path().join(".tidewatch.toml"), "checklist = \"docs/list.md\"\n")
            .unwrap();
        std::fs::create_dir_all(repo.path().join("docs")).unwrap();
        std::fs::write(repo.path().join("docs/list.md"), "### 1. Only\n- [ ] x\n").unwrap();
        let own = load(&paths, repo.path());
        assert_eq!((own.source, titles(&own)), (Source::Repo, vec!["Only".to_owned()]));
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
        assert_eq!(host.of(repo.path()).unwrap().chips.len(), 10);
        assert!(host.of(Path::new("/elsewhere")).is_none());
        assert_eq!(host.asked.as_ref().unwrap().1, 1.0, "the second tick did not ask again");
    }
}
