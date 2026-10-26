//! The app's side of the progress ledgers: each agent tab's file is read when the tab appears, what
//! an agent sends replaces it, and the schedule index follows (in memory at once, on disk through a
//! writer thread). Reads happen on `Jobs` threads.
//!
//! `TIDEWATCH_DEV_PROGRESS=<file>` gives the first agent tab the ledger in that file instead of
//! reading its own, and writes nothing, so a screen can be captured without a real agent.

use std::path::{Path, PathBuf};

use tidewatch_core::civil_date::Date;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::progress::Ledger;
use tidewatch_core::progress_store;
use tidewatch_core::schedule::{Index, Writer};

use crate::coming_up::Row;
use crate::jobs::Jobs;
use crate::progress_state::{Facts, State, TabLedger};

pub(crate) enum Done {
    Ledger { tab: u64, found: Option<Box<TabLedger>> },
    Index(Index),
}

#[derive(Default)]
pub(crate) struct Dev {
    ledger: Option<Ledger>,
}

impl Dev {
    pub(crate) fn from_env() -> Self {
        let ledger = std::env::var_os("TIDEWATCH_DEV_PROGRESS")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .and_then(|path| match std::fs::read_to_string(&path) {
                Ok(text) => Some(Ledger::parse(&text)),
                Err(e) => {
                    eprintln!("tidewatch: cannot read {} ({e})", path.display());
                    None
                }
            });
        Self { ledger }
    }
}

/// Today's date by the UTC clock.
pub(crate) fn today() -> Date {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    Date::from_unix_secs(secs)
}

pub(crate) struct Host {
    state: State,
    jobs: Jobs<Done>,
    /// Where the index lives and the thread that writes it; none when nothing may be written.
    store: Option<(PathBuf, Writer)>,
    index_asked: bool,
    dev: Dev,
}

impl Host {
    pub(crate) fn new(paths: Option<&TidewatchPaths>, dev: Dev) -> Self {
        let store = paths.map(|paths| (paths.schedule_file(), Writer::new(paths.schedule_file())));
        Self { state: State::default(), jobs: Jobs::new(), store, index_asked: false, dev }
    }

    /// A tab's ledger, once it was read or the agent sent one.
    pub(crate) fn ledger(&self, tab: u64) -> Option<&TabLedger> {
        self.state.ledger(tab)
    }

    /// The scheduled items of every tab, closed ones included.
    pub(crate) fn index(&self) -> &Index {
        self.state.index()
    }

    /// The Next entries of open tabs that stopped with nothing scheduled.
    pub(crate) fn next_rows(&self) -> Vec<Row> {
        self.state.next_rows()
    }

    /// The agent sent a ledger (it is saved already).
    pub(crate) fn arrived(&mut self, tab: u64, ledger: Ledger, now_ms: i64) {
        if let Some(entry) = self.state.arrived(tab, ledger, now_ms)
            && let Some((_, writer)) = &self.store
        {
            writer.set(entry);
        }
    }

    /// Looks at the agent tabs: reads the files of new ones, closes the index entries of gone ones,
    /// and takes what the jobs found.
    pub(crate) fn tick(&mut self, tabs: &[Facts<'_>], ctx: &egui::Context) {
        if !self.index_asked {
            self.index_asked = true;
            if let Some((file, _)) = &self.store {
                let file = file.clone();
                self.jobs.spawn(ctx, move || Done::Index(Index::load(&file)));
            }
        }
        let wants = self.state.look(tabs);
        for (tab, file) in wants.load {
            self.jobs.spawn(ctx, move || Done::Ledger { tab, found: read(&file).map(Box::new) });
        }
        if let Some((_, writer)) = &self.store {
            for (repo, tab) in wants.closed {
                writer.close(repo, tab);
            }
            for entry in wants.write {
                writer.set(entry);
            }
        }
        if let Some(first) = tabs.first()
            && let Some(ledger) = self.dev.ledger.take()
        {
            self.state.given(first.id, ledger);
        }
        for done in self.jobs.drain() {
            match done {
                Done::Ledger { tab, found } => {
                    if let Some(entry) = self.state.loaded(tab, found.map(|found| *found))
                        && let Some((_, writer)) = &self.store
                    {
                        writer.set(entry);
                    }
                }
                Done::Index(loaded) => self.state.merge_loaded_index(loaded),
            }
        }
    }
}

fn read(file: &Path) -> Option<TabLedger> {
    let ledger = progress_store::load(file)?;
    let updated_ms = std::fs::metadata(file)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok());
    Some(TabLedger { ledger, updated_ms })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use tidewatch_core::schedule::Presence;

    struct Fixture {
        _home: tempfile::TempDir,
        paths: TidewatchPaths,
        notes: PathBuf,
    }

    fn fixture() -> Fixture {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let notes = paths.notes_file(Path::new("/work/web"), "otter");
        Fixture { _home: home, paths, notes }
    }

    fn facts(fixture: &Fixture) -> Facts<'_> {
        Facts {
            id: 1,
            notes_file: &fixture.notes,
            worktree: Some((Path::new("/work/web"), "otter")),
            profile_id: Some("builder"),
            session_id: Some("sess"),
        }
    }

    fn wait_for_ledger(host: &mut Host, tabs: &[Facts<'_>], ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while host.ledger(1).is_none() && Instant::now() < deadline {
            host.tick(tabs, ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    const LEDGER: &str =
        "# Goal\nShip it\n\n## Scheduled\n- [ ] 2026-11-01 -- Switch off -- the milestone\n";

    #[test]
    fn a_saved_ledger_is_read_when_the_tab_appears_and_its_items_reach_the_index_file() {
        let fx = fixture();
        std::fs::create_dir_all(fx.notes.parent().unwrap()).unwrap();
        std::fs::write(fx.paths.progress_file(Path::new("/work/web"), "otter"), LEDGER).unwrap();
        let ctx = egui::Context::default();
        let mut host = Host::new(Some(&fx.paths), Dev::default());
        let tabs = [facts(&fx)];
        wait_for_ledger(&mut host, &tabs, &ctx);
        let known = host.ledger(1).unwrap();
        assert_eq!(known.ledger.goal, "Ship it");
        assert!(known.updated_ms.is_some_and(|ms| ms > 0));
        assert_eq!(host.index().tabs[0].items[0].what, "Switch off");

        host.tick(&[], &ctx);
        drop(host);
        let index = Index::load(&fx.paths.schedule_file());
        assert_eq!((index.tabs.len(), index.tabs[0].presence), (1, Presence::Closed));
        assert_eq!(index.tabs[0].session_id.as_deref(), Some("sess"));
        assert_eq!(index.tabs[0].items.len(), 1);
    }

    #[test]
    fn a_ledger_the_agent_sends_is_kept_and_written_to_the_index() {
        let fx = fixture();
        let ctx = egui::Context::default();
        let mut host = Host::new(Some(&fx.paths), Dev::default());
        host.tick(&[facts(&fx)], &ctx);
        host.arrived(1, Ledger::parse(LEDGER), 123);
        assert_eq!(host.ledger(1).unwrap().updated_ms, Some(123));
        drop(host);
        let index = Index::load(&fx.paths.schedule_file());
        assert_eq!(index.tabs[0].goal, "Ship it");
        assert_eq!(index.tabs[0].presence, Presence::Open);
    }

    #[test]
    fn nothing_is_written_without_paths_and_a_dev_ledger_is_never_written() {
        let fx = fixture();
        let ctx = egui::Context::default();
        let mut host = Host::new(None, Dev::default());
        host.tick(&[facts(&fx)], &ctx);
        host.arrived(1, Ledger::parse(LEDGER), 1);
        assert_eq!(host.ledger(1).unwrap().ledger.goal, "Ship it");
        assert!(!fx.paths.schedule_file().exists());

        let mut host = Host::new(Some(&fx.paths), Dev { ledger: Some(Ledger::parse(LEDGER)) });
        host.tick(&[facts(&fx)], &ctx);
        assert_eq!(host.ledger(1).unwrap().ledger.goal, "Ship it");
        assert_eq!(host.index().tabs.len(), 1, "a capture sees Coming up");
        drop(host);
        assert!(!fx.paths.schedule_file().exists(), "but the file is left alone");
    }

    #[test]
    fn the_dev_ledger_goes_to_the_first_agent_tab_only() {
        let fx = fixture();
        let ctx = egui::Context::default();
        let mut host = Host::new(None, Dev { ledger: Some(Ledger::parse(LEDGER)) });
        let second = Facts { id: 2, ..facts(&fx) };
        host.tick(&[second, facts(&fx)], &ctx);
        assert!(host.ledger(2).is_some());
        assert!(host.ledger(1).is_none());
    }
}
