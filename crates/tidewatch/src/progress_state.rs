//! The ledger of each agent tab and the schedule index, as plain data: what arrived, what was read
//! from disk, which tabs still need reading and which went away (pure).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use tidewatch_core::paths::progress_file_beside;
use tidewatch_core::progress::Ledger;
use tidewatch_core::schedule::{Entry, Index, Owner, Presence};

use crate::coming_up::{self, Row};

/// A tab's ledger and when it last changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TabLedger {
    pub(crate) ledger: Ledger,
    /// Unix milliseconds: when the agent sent it, or when its file was last written.
    pub(crate) updated_ms: Option<i64>,
}

/// What one frame knows about an agent tab.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Facts<'a> {
    pub(crate) id: u64,
    pub(crate) notes_file: &'a Path,
    /// The repo and the worktree's name, for a tab that has a worktree.
    pub(crate) worktree: Option<(&'a Path, &'a str)>,
    pub(crate) profile_id: Option<&'a str>,
    pub(crate) session_id: Option<&'a str>,
    /// The folder the agent runs in and the tab's title: who owns a ledger when there is no
    /// worktree, for the rows of Coming up that live in memory only.
    pub(crate) fallback: Option<(&'a Path, &'a str)>,
}

impl Facts<'_> {
    fn is_owned_by(&self, owner: &Owner) -> bool {
        self.worktree.is_some_and(|(repo, tab)| owner.repo == repo && owner.tab == tab)
            && owner.session_id.as_deref() == self.session_id
            && owner.profile_id.as_deref() == self.profile_id
    }

    fn owner(&self) -> Option<Owner> {
        self.worktree.map(|(repo, tab)| Owner {
            repo: repo.to_path_buf(),
            tab: tab.to_owned(),
            session_id: self.session_id.map(str::to_owned),
            profile_id: self.profile_id.map(str::to_owned),
        })
    }
}

/// What a frame's look at the tabs asks for.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Wants {
    /// Tabs whose `progress.md` has not been read yet.
    pub(crate) load: Vec<(u64, PathBuf)>,
    /// Tabs that were open and are gone: their dated items stay in the index, marked closed.
    pub(crate) closed: Vec<(PathBuf, String)>,
    /// Tabs that were open and are gone with their worktree folder: their entry is removed.
    pub(crate) removed: Vec<(PathBuf, String)>,
    /// Index entries that changed because a tab's session or profile became known.
    pub(crate) write: Vec<Entry>,
}

#[derive(Default)]
pub(crate) struct State {
    ledgers: HashMap<u64, TabLedger>,
    asked: HashSet<u64>,
    owners: HashMap<u64, Owner>,
    fallback_owners: HashMap<u64, Owner>,
    index: Index,
}

impl State {
    pub(crate) fn ledger(&self, tab: u64) -> Option<&TabLedger> {
        self.ledgers.get(&tab)
    }

    pub(crate) fn index(&self) -> &Index {
        &self.index
    }

    /// The Next entries of the open tabs whose ledger stopped with nothing scheduled, in memory
    /// only, ordered by repo and tab. A tab without a worktree is listed under its folder and title.
    pub(crate) fn next_rows(&self) -> Vec<Row> {
        let mut owners: Vec<(&Owner, &Ledger)> = self
            .ledgers
            .iter()
            .filter_map(|(id, known)| {
                let owner = self.owners.get(id).or_else(|| self.fallback_owners.get(id))?;
                Some((owner, &known.ledger))
            })
            .collect();
        owners.sort_by(|(a, _), (b, _)| (&a.repo, &a.tab).cmp(&(&b.repo, &b.tab)));
        owners.into_iter().flat_map(|(owner, ledger)| coming_up::next_rows(owner, ledger)).collect()
    }

    /// The tabs to read and the ones that closed. `tabs` is every agent tab now open.
    /// `folder_exists` says whether a gone tab's worktree folder is still there.
    pub(crate) fn look(
        &mut self,
        tabs: &[Facts<'_>],
        folder_exists: impl Fn(&Path, &str) -> bool,
    ) -> Wants {
        let mut wants = Wants::default();
        for facts in tabs {
            if let Some((repo, tab)) = facts.fallback {
                self.fallback_owners.insert(
                    facts.id,
                    Owner {
                        repo: repo.to_path_buf(),
                        tab: tab.to_owned(),
                        session_id: facts.session_id.map(str::to_owned),
                        profile_id: facts.profile_id.map(str::to_owned),
                    },
                );
            }
            if !self.owners.get(&facts.id).is_some_and(|owner| facts.is_owned_by(owner))
                && let Some(owner) = facts.owner()
            {
                self.owners.insert(facts.id, owner);
                wants.write.extend(self.entry_of(facts.id));
            }
            if self.asked.insert(facts.id) {
                wants.load.push((facts.id, progress_file_beside(facts.notes_file)));
            }
        }
        let gone: Vec<u64> = self
            .asked
            .iter()
            .copied()
            .filter(|id| !tabs.iter().any(|facts| facts.id == *id))
            .collect();
        for id in gone {
            self.asked.remove(&id);
            self.ledgers.remove(&id);
            self.fallback_owners.remove(&id);
            if let Some(owner) = self.owners.remove(&id) {
                if folder_exists(&owner.repo, &owner.tab) {
                    self.index.close(&owner.repo, &owner.tab);
                    wants.closed.push((owner.repo, owner.tab));
                } else {
                    self.index.remove(&owner.repo, &owner.tab);
                    wants.removed.push((owner.repo, owner.tab));
                }
            }
        }
        wants
    }

    /// The agent sent a ledger. Returns the tab's index entry to write.
    pub(crate) fn arrived(&mut self, tab: u64, ledger: Ledger, now_ms: i64) -> Option<Entry> {
        self.ledgers.insert(tab, TabLedger { ledger, updated_ms: Some(now_ms) });
        self.entry_of(tab)
    }

    /// A tab's file was read. A ledger the agent sent in the meantime is newer and stays. Returns
    /// the tab's index entry to write.
    pub(crate) fn loaded(&mut self, tab: u64, found: Option<TabLedger>) -> Option<Entry> {
        if !self.asked.contains(&tab) || self.ledgers.contains_key(&tab) {
            return None;
        }
        self.ledgers.insert(tab, found?);
        self.entry_of(tab)
    }

    /// A ledger the app was given for a tab (`TIDEWATCH_DEV_PROGRESS`); nothing is read for it.
    pub(crate) fn given(&mut self, tab: u64, ledger: Ledger) {
        self.asked.insert(tab);
        self.ledgers.insert(tab, TabLedger { ledger, updated_ms: None });
        if let Some(entry) = self.entry_of(tab) {
            self.index.set(entry);
        }
    }

    /// The index file as it was read at startup; entries this run already has win.
    pub(crate) fn merge_loaded_index(&mut self, loaded: Index) {
        for entry in loaded.tabs {
            if !self
                .index
                .tabs
                .iter()
                .any(|known| known.repo == entry.repo && known.tab == entry.tab)
            {
                self.index.tabs.push(entry);
            }
        }
    }

    fn entry_of(&mut self, tab: u64) -> Option<Entry> {
        let owner = self.owners.get(&tab)?.clone();
        let ledger = &self.ledgers.get(&tab)?.ledger;
        let entry = Entry::of(owner, ledger, Presence::Open);
        self.index.set(entry.clone());
        Some(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTES: &str = "/state/web-abc123/otter/notes.json";
    const PROGRESS: &str = "/state/web-abc123/otter/progress.md";

    fn facts(id: u64, with_worktree: bool, session: Option<&'static str>) -> Facts<'static> {
        Facts {
            id,
            notes_file: Path::new(NOTES),
            worktree: with_worktree.then_some((Path::new("/work/web"), "otter")),
            profile_id: Some("builder"),
            session_id: session,
            fallback: Some((Path::new("/work/web"), "otter-title")),
        }
    }

    fn ledger(scheduled: &str) -> Ledger {
        Ledger::parse(&format!("# Goal\nShip it\n\n## Scheduled\n{scheduled}"))
    }

    fn at(ledger: Ledger) -> TabLedger {
        TabLedger { ledger, updated_ms: Some(7) }
    }

    #[test]
    fn a_new_tab_is_read_once_from_the_file_beside_its_notes() {
        let mut state = State::default();
        let wants = state.look(&[facts(1, true, None)], |_, _| true);
        assert_eq!(wants.load, [(1, PathBuf::from(PROGRESS))]);
        assert_eq!(state.look(&[facts(1, true, None)], |_, _| true), Wants::default());
        assert_eq!(
            state.look(&[facts(1, true, None), facts(2, true, None)], |_, _| true).load.len(),
            1
        );
    }

    #[test]
    fn what_the_agent_sent_becomes_the_tabs_ledger_and_its_open_items_the_index_entry() {
        let mut state = State::default();
        state.look(&[facts(1, true, Some("sess"))], |_, _| true);
        let entry = state.arrived(1, ledger("- [ ] 2026-11-01 -- A\n- [x] you -- B\n"), 9).unwrap();
        assert_eq!(entry.items.len(), 1);
        assert_eq!((entry.tab.as_str(), entry.session_id.as_deref()), ("otter", Some("sess")));
        assert_eq!(state.ledger(1).unwrap().updated_ms, Some(9));
        assert_eq!(state.index().tabs, [entry]);
        state.arrived(1, ledger("- [x] 2026-11-01 -- A\n"), 10);
        assert_eq!(state.index(), &Index::default(), "no open item, no entry");
        assert_eq!(state.ledger(1).unwrap().ledger.scheduled.len(), 1);
    }

    #[test]
    fn a_tab_without_a_worktree_has_a_ledger_but_no_index_entry() {
        let mut state = State::default();
        state.look(&[facts(1, false, None)], |_, _| true);
        assert_eq!(state.arrived(1, ledger("- [ ] you -- A\n"), 1), None);
        assert!(state.ledger(1).is_some());
        assert_eq!(state.index(), &Index::default());
    }

    #[test]
    fn a_file_read_late_never_replaces_what_the_agent_sent_and_a_gone_tab_is_ignored() {
        let mut state = State::default();
        state.look(&[facts(1, true, None)], |_, _| true);
        state.arrived(1, ledger("- [ ] you -- new\n"), 5);
        assert_eq!(state.loaded(1, Some(at(ledger("- [ ] you -- old\n")))), None);
        assert_eq!(state.ledger(1).unwrap().ledger.scheduled[0].what, "new");

        state.look(&[facts(2, true, None)], |_, _| true);
        assert_eq!(state.loaded(3, Some(at(ledger("- [ ] you -- x\n")))), None, "never asked for");
        assert_eq!(state.loaded(2, None), None);
        assert!(state.ledger(2).is_none());
        let entry = state.loaded(2, Some(at(ledger("- [ ] you -- from file\n")))).unwrap();
        assert_eq!(entry.items[0].what, "from file");
        assert_eq!(state.ledger(2).unwrap().updated_ms, Some(7));
    }

    #[test]
    fn a_tab_that_goes_away_closes_its_entry_and_keeps_its_dated_items() {
        let mut state = State::default();
        state.look(&[facts(1, true, None)], |_, _| true);
        state.arrived(1, ledger("- [ ] 2026-11-01 -- A\n- [ ] on: x -- B\n"), 1);
        let wants = state.look(&[], |_, _| true);
        assert_eq!(wants.closed, [(PathBuf::from("/work/web"), "otter".to_owned())]);
        assert!(state.ledger(1).is_none());
        let tabs = &state.index().tabs;
        assert_eq!((tabs.len(), tabs[0].presence, tabs[0].items.len()), (1, Presence::Closed, 1));
        assert_eq!(state.look(&[], |_, _| true), Wants::default(), "closed once");

        let wants = state.look(&[facts(5, true, None)], |_, _| true);
        assert_eq!(wants.load.len(), 1, "the same worktree opened again is read again");
        let entry = state.loaded(5, Some(at(ledger("- [ ] 2026-11-01 -- A\n")))).unwrap();
        assert_eq!(entry.presence, Presence::Open);
        assert_eq!(state.index().tabs[0].presence, Presence::Open);
    }

    #[test]
    fn learning_the_session_of_a_tab_rewrites_its_entry() {
        let mut state = State::default();
        state.look(&[facts(1, true, None)], |_, _| true);
        state.arrived(1, ledger("- [ ] you -- A\n"), 1);
        let wants = state.look(&[facts(1, true, Some("sess"))], |_, _| true);
        assert_eq!(wants.write.len(), 1);
        assert_eq!(wants.write[0].session_id.as_deref(), Some("sess"));
        assert_eq!(state.look(&[facts(1, true, Some("sess"))], |_, _| true), Wants::default());
    }

    #[test]
    fn a_given_ledger_needs_no_read_and_fills_the_index_in_memory() {
        let mut state = State::default();
        state.look(&[facts(1, true, None)], |_, _| true);
        state.given(1, ledger("- [ ] ~2026-11-07 -- Count\n"));
        assert_eq!(state.index().tabs[0].items.len(), 1);
        assert_eq!(state.ledger(1).unwrap().updated_ms, None);
        assert_eq!(state.loaded(1, Some(at(ledger("- [ ] you -- file\n")))), None);
    }

    #[test]
    fn the_index_read_at_startup_adds_the_other_tabs_and_never_overrides_this_run() {
        let mut state = State::default();
        state.look(&[facts(1, true, None)], |_, _| true);
        state.arrived(1, ledger("- [ ] you -- live\n"), 1);
        let mut loaded = Index::default();
        let owner = |tab: &str| Owner {
            repo: "/work/web".into(),
            tab: tab.into(),
            session_id: None,
            profile_id: None,
        };
        loaded.set(Entry::of(owner("otter"), &ledger("- [ ] you -- stale\n"), Presence::Open));
        loaded.set(Entry::of(
            owner("heron"),
            &ledger("- [ ] you -- closed one\n"),
            Presence::Closed,
        ));
        state.merge_loaded_index(loaded);
        let whats: Vec<_> = state
            .index()
            .tabs
            .iter()
            .map(|entry| (entry.tab.as_str(), entry.items[0].what.as_str()))
            .collect();
        assert_eq!(whats, [("otter", "live"), ("heron", "closed one")]);
    }

    #[test]
    fn next_rows_come_from_open_tabs_in_memory_and_never_reach_the_index() {
        let mut state = State::default();
        state.look(&[facts(1, true, None), facts(2, false, None)], |_, _| true);
        let stopped =
            "# Goal\nShip it\n\n## Status\nstate: blocked\n\n## Next\n- Merge after approval\n";
        state.arrived(1, Ledger::parse(stopped), 1);
        state.arrived(2, Ledger::parse(stopped), 1);
        let rows = state.next_rows();
        let tabs: Vec<_> = rows.iter().map(|row| row.tab.as_str()).collect();
        assert_eq!(tabs, ["otter", "otter-title"], "a tab with no worktree is listed by its title");
        assert!(rows.iter().all(|row| row.what == "Merge after approval"));
        assert_eq!(state.index(), &Index::default());
        state.look(&[], |_, _| true);
        assert_eq!(state.next_rows(), Vec::new(), "a closed tab keeps nothing in memory");
    }

    #[test]
    fn a_tab_that_goes_away_with_its_worktree_folder_takes_its_entry_along() {
        let mut state = State::default();
        state.look(&[facts(1, true, None), facts(2, true, None)], |_, _| true);
        state.arrived(1, ledger("- [ ] 2026-11-01 -- A\n"), 1);
        let wants = state.look(&[facts(2, true, None)], |_, _| false);
        assert_eq!(wants.removed, [(PathBuf::from("/work/web"), "otter".to_owned())]);
        assert_eq!(wants.closed, Vec::new());
        assert_eq!(state.index(), &Index::default());
    }

    const STOPPED: &str =
        "# Goal\nShip it\n\n## Status\nstate: blocked\n\n## Next\n- Merge after approval\n";

    #[test]
    fn a_stopped_ledger_makes_rows_whichever_comes_first_the_ledger_or_the_owner() {
        let no_worktree = || facts(1, false, None);
        let with_worktree = || facts(1, true, None);
        let whos = |state: &State| {
            state
                .next_rows()
                .iter()
                .map(|row| (row.repo.clone(), row.tab.clone()))
                .collect::<Vec<_>>()
        };
        let by_title = vec![(PathBuf::from("/work/web"), "otter-title".to_owned())];
        let by_worktree = vec![(PathBuf::from("/work/web"), "otter".to_owned())];

        let mut state = State::default();
        state.given(1, Ledger::parse(STOPPED));
        assert_eq!(whos(&state), Vec::new(), "no tab seen yet");
        state.look(&[no_worktree()], |_, _| true);
        assert_eq!(
            whos(&state),
            by_title,
            "the ledger came before the tab, and there is no worktree"
        );
        state.look(&[with_worktree()], |_, _| true);
        assert_eq!(whos(&state), by_worktree, "the worktree shows up later and wins");

        let mut state = State::default();
        state.look(&[no_worktree()], |_, _| true);
        state.given(1, Ledger::parse(STOPPED));
        assert_eq!(whos(&state), by_title, "the tab came first");

        let mut state = State::default();
        state.look(&[with_worktree()], |_, _| true);
        state.arrived(1, Ledger::parse(STOPPED), 1);
        assert_eq!(whos(&state), by_worktree);
        state.look(&[], |_, _| true);
        assert_eq!(whos(&state), Vec::new(), "a closed tab keeps nothing");
    }
}
