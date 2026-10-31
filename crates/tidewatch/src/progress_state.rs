//! The ledger of each agent tab and the schedule index, as plain data: what arrived, what was read
//! from disk, which tabs still need reading and which went away (pure).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use tidewatch_core::paths::progress_file_beside;
use tidewatch_core::progress::{Entry as Line, Ledger};
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

/// The Needs you lines of the questions a tab's agent asked with a question tool, and when.
#[derive(Debug)]
struct Held {
    lines: Vec<Line>,
    at_ms: i64,
}

#[derive(Default)]
pub(crate) struct State {
    ledgers: HashMap<u64, TabLedger>,
    held: HashMap<u64, Held>,
    shown: HashMap<u64, TabLedger>,
    asked: HashSet<u64>,
    owners: HashMap<u64, Owner>,
    fallback_owners: HashMap<u64, Owner>,
    index: Index,
}

impl State {
    /// The ledger the screens show: the agent's, with its open questions laid over `needs_you`.
    pub(crate) fn ledger(&self, tab: u64) -> Option<&TabLedger> {
        self.shown.get(&tab).or_else(|| self.ledgers.get(&tab))
    }

    /// The ledger as the agent sent it, without the questions Tidewatch laid over it.
    pub(crate) fn sent(&self, tab: u64) -> Option<&TabLedger> {
        self.ledgers.get(&tab)
    }

    /// The questions the tab's agent has open now, as Needs you lines, empty once none is. They
    /// stand in `needs_you` ahead of what the agent wrote itself, which keeps whatever else it
    /// says. Returns whether the ledger the screens show changed.
    pub(crate) fn open_questions(&mut self, tab: u64, lines: Vec<Line>, now_ms: i64) -> bool {
        if lines.is_empty() {
            let had = self.held.remove(&tab).is_some();
            if had {
                self.lay(tab);
            }
            return had;
        }
        if self.held.get(&tab).is_some_and(|held| held.lines == lines) {
            return false;
        }
        self.held.insert(tab, Held { lines, at_ms: now_ms });
        self.lay(tab);
        true
    }

    fn lay(&mut self, tab: u64) {
        let Some(held) = self.held.get(&tab) else {
            self.shown.remove(&tab);
            return;
        };
        let under = self.ledgers.get(&tab);
        let mut ledger = under.map(|known| known.ledger.clone()).unwrap_or_default();
        let own = std::mem::take(&mut ledger.needs_you);
        let same = |a: &Line, b: &Line| a.text.trim().eq_ignore_ascii_case(b.text.trim());
        ledger.needs_you = held
            .lines
            .iter()
            .cloned()
            .chain(own.into_iter().filter(|line| !held.lines.iter().any(|ours| same(ours, line))))
            .collect();
        let updated_ms = under.and_then(|known| known.updated_ms).or(Some(held.at_ms));
        self.shown.insert(tab, TabLedger { ledger, updated_ms });
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
            self.held.remove(&id);
            self.shown.remove(&id);
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
        self.lay(tab);
        self.entry_of(tab)
    }

    /// A tab's file was read. A ledger the agent sent in the meantime is newer and stays. Returns
    /// the tab's index entry to write.
    pub(crate) fn loaded(&mut self, tab: u64, found: Option<TabLedger>) -> Option<Entry> {
        if !self.asked.contains(&tab) || self.ledgers.contains_key(&tab) {
            return None;
        }
        self.ledgers.insert(tab, found?);
        self.lay(tab);
        self.entry_of(tab)
    }

    /// A ledger the app was given for a tab (`TIDEWATCH_DEV_PROGRESS`); nothing is read for it.
    pub(crate) fn given(&mut self, tab: u64, ledger: Ledger) {
        self.asked.insert(tab);
        self.ledgers.insert(tab, TabLedger { ledger, updated_ms: None });
        self.lay(tab);
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

    fn question(text: &str) -> Line {
        Line { text: text.into(), why: None, options: Vec::new() }
    }

    fn ledger_asking(text: &str) -> Ledger {
        Ledger::parse(&format!("# Goal\nShip it\n\n## Needs you\n- {text}\n"))
    }

    #[test]
    fn a_question_asked_with_a_tool_fills_needs_you_and_answering_it_takes_it_out_again() {
        let mut state = State::default();
        state.look(&[facts(1, true, None)], |_, _| true);
        state.arrived(1, ledger("- [ ] 2026-11-01 -- A\n"), 5);
        assert!(state.open_questions(1, vec![question("Where?")], 9));
        let shown = state.ledger(1).unwrap();
        assert_eq!(shown.ledger.needs_you, [question("Where?")]);
        assert_eq!(shown.ledger.scheduled.len(), 1, "the rest of the report stays");
        assert_eq!(shown.updated_ms, Some(5), "the agent's save time stands");
        assert_eq!(state.sent(1).unwrap().ledger.needs_you, Vec::<Line>::new());

        assert!(!state.open_questions(1, vec![question("Where?")], 12), "same question, no change");
        assert!(state.open_questions(1, Vec::new(), 13));
        assert_eq!(state.ledger(1).unwrap().ledger.needs_you, Vec::<Line>::new());
        assert!(!state.open_questions(1, Vec::new(), 14), "nothing to take out twice");
    }

    #[test]
    fn a_tab_with_no_report_gets_a_ledger_of_only_the_question_and_loses_it_when_answered() {
        let mut state = State::default();
        state.look(&[facts(1, true, None)], |_, _| true);
        state.open_questions(1, vec![question("Where?")], 9);
        let shown = state.ledger(1).unwrap();
        assert_eq!((shown.ledger.needs_you.len(), shown.updated_ms), (1, Some(9)));
        assert!(state.sent(1).is_none(), "the agent sent nothing");
        state.open_questions(1, Vec::new(), 10);
        assert!(state.ledger(1).is_none());
    }

    #[test]
    fn what_the_agent_wrote_in_needs_you_stays_after_the_question_and_a_repeat_is_shown_once() {
        let mut state = State::default();
        state.look(&[facts(1, true, None)], |_, _| true);
        state.arrived(1, ledger_asking("where?"), 5);
        state.arrived(1, Ledger::parse("# Goal\ng\n\n## Needs you\n- where?\n- when?\n"), 6);
        state.open_questions(1, vec![question("Where?")], 9);
        let texts: Vec<&str> = state
            .ledger(1)
            .unwrap()
            .ledger
            .needs_you
            .iter()
            .map(|line| line.text.as_str())
            .collect();
        assert_eq!(texts, ["Where?", "when?"]);
    }

    #[test]
    fn a_report_that_arrives_while_a_question_is_open_keeps_it_on_top() {
        let mut state = State::default();
        state.look(&[facts(1, true, None)], |_, _| true);
        state.open_questions(1, vec![question("Where?")], 9);
        state.arrived(1, ledger("- [ ] 2026-11-01 -- A\n"), 10);
        let shown = state.ledger(1).unwrap();
        assert_eq!((shown.ledger.needs_you.len(), shown.updated_ms), (1, Some(10)));
        assert_eq!(shown.ledger.scheduled.len(), 1);
        let late = state.loaded(1, Some(at(ledger("- [ ] you -- file\n"))));
        assert_eq!(late, None, "a file read late still never replaces what the agent sent");
    }

    #[test]
    fn a_file_read_while_a_question_is_open_shows_the_question_over_the_file() {
        let mut state = State::default();
        state.look(&[facts(1, true, None)], |_, _| true);
        state.open_questions(1, vec![question("Where?")], 9);
        state.loaded(1, Some(at(ledger("- [ ] you -- from file\n"))));
        let shown = state.ledger(1).unwrap();
        assert_eq!((shown.ledger.needs_you.len(), shown.ledger.scheduled.len()), (1, 1));
        assert_eq!(shown.updated_ms, Some(7));
    }

    #[test]
    fn a_tab_that_goes_away_forgets_its_open_question() {
        let mut state = State::default();
        state.look(&[facts(1, true, None)], |_, _| true);
        state.open_questions(1, vec![question("Where?")], 9);
        state.look(&[], |_, _| true);
        assert!(state.ledger(1).is_none());
        state.look(&[facts(1, true, None)], |_, _| true);
        assert!(state.ledger(1).is_none(), "nothing is laid over the next tab");
    }

    #[test]
    fn a_claude_menu_that_opens_fills_the_tabs_needs_you_and_answering_it_clears_it() {
        use crate::ask_model::open_lines;
        use crate::reader_rows::{Asked, Rows};
        use tidewatch_core::question::{Question, QuestionOption, Select};
        use tidewatch_core::transcript::{AskPart, Entry as Call, Role, ToolCall, ToolPhase};
        let asked = vec![Question {
            text: "Where?".into(),
            header: None,
            decision: None,
            select: Select::Single,
            options: vec![QuestionOption::new("Stage"), QuestionOption::new("Run (Recommended)")],
        }];
        let mut rows = Rows::default();
        rows.push(Call {
            role: Role::Tool,
            text: String::new(),
            tool: Some(ToolCall {
                name: "AskUserQuestion".into(),
                summary: String::new(),
                id: Some("q1".into()),
                phase: ToolPhase::Call,
                ask: Some(AskPart::Asked(asked)),
            }),
            ts: None,
            images: Vec::new(),
        });
        let mut state = State::default();
        state.look(&[facts(1, true, None)], |_, _| true);

        state.open_questions(1, open_lines(&rows), 20);
        let needs = &state.ledger(1).unwrap().ledger.needs_you;
        assert_eq!(needs.len(), 1);
        assert_eq!(needs[0].text, "Where?");
        let options: Vec<(&str, bool)> =
            needs[0].options.iter().map(|c| (c.text.as_str(), c.recommended)).collect();
        assert_eq!(options, [("Stage", false), ("Run", true)]);
        assert!(state.sent(1).is_none(), "the agent sent no report");

        assert!(rows.set_asked(0, Asked::Sent(vec![("Where?".into(), "Run".into())])));
        state.open_questions(1, open_lines(&rows), 21);
        assert!(state.ledger(1).is_none());
    }
}
