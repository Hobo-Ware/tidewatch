//! What the History and Diff panes keep for one tab: the loaded history, the working-tree status,
//! the selected commit and file, the open diff, the notes and the file watcher.
//!
//! The panes only change this state (select, scope, file). `sync` then turns what is wanted into
//! background jobs, and `apply` takes their answers back, dropping any that a newer request has
//! overtaken. Nothing here blocks: git and disk work all runs through `pane_jobs`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use kelp_core::ObjectId;
use kelp_core::commit::{FileChange, Summary};
use kelp_core::diff::FileDiff;
use kelp_core::watch::{Change, Watcher};
use kelp_ui::diff::{DiffBody, DiffLayout};
use kelp_ui::graph::{GraphView, Selection};
use tidewatch_core::history::{Loaded, Scope, Source};
use tidewatch_core::notes::Notes;

use tidewatch_core::paths::TidewatchPaths;

use crate::file_nav;
use crate::jobs::Jobs;
use crate::pane::{Body, Pane};
use crate::pane_jobs::{self, Done, LineCounts};
use crate::repo::Workspace;
use crate::tabs::{MAIN_TITLE, Tab};

/// What the user picked in the graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    WorkTree,
    Commit(ObjectId),
}

impl Pick {
    fn source(self) -> Source {
        match self {
            Self::WorkTree => Source::WorkTree,
            Self::Commit(id) => Source::Commit(id),
        }
    }
}

/// The pick to show: the user's own while it still exists, else the uncommitted changes when there
/// are any, else the commit HEAD points at.
pub(crate) fn resolve_pick(
    pick: Option<Pick>,
    has_wip: bool,
    head: Option<ObjectId>,
    in_history: impl Fn(ObjectId) -> bool,
) -> Option<Pick> {
    match pick {
        Some(Pick::WorkTree) if has_wip => return Some(Pick::WorkTree),
        Some(Pick::Commit(id)) if in_history(id) => return Some(Pick::Commit(id)),
        _ => {}
    }
    if has_wip { Some(Pick::WorkTree) } else { head.map(Pick::Commit) }
}

/// Which folder a tab's panes read, and where its notes live. Only a git repo's main tab and its
/// worktree tabs have panes, and only while their shell runs.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ViewSpec {
    pub(crate) id: u64,
    pub(crate) dir: PathBuf,
    pub(crate) name: String,
    pub(crate) is_main: bool,
    pub(crate) notes_file: Option<PathBuf>,
}

impl ViewSpec {
    pub(crate) fn of<T>(
        workspace: &Workspace,
        paths: Option<&TidewatchPaths>,
        tab: &Tab<Pane<T>>,
    ) -> Option<Self> {
        let Workspace::Repo(repo) = workspace else { return None };
        let draft = matches!(tab.pane.body, Body::Draft(_));
        if !draft && !matches!(tab.pane.body, Body::Shell(_)) {
            return None;
        }
        let (dir, repo_root, name, is_main) = match &tab.pane.worktree {
            Some(worktree) => {
                (worktree.path.clone(), worktree.repo.clone(), worktree.name().to_owned(), false)
            }
            None if tab.pinned => {
                (repo.root.clone(), repo.root.clone(), MAIN_TITLE.to_owned(), true)
            }
            None if draft => (repo.root.clone(), repo.root.clone(), tab.title.clone(), false),
            None => return None,
        };
        let notes_file = paths.filter(|_| !draft).map(|paths| paths.notes_file(&repo_root, &name));
        Some(Self { id: tab.id, dir, name, is_main, notes_file })
    }

    pub(crate) fn into_view(self) -> TabView {
        TabView::new(self.id, self.dir, self.is_main, self.notes_file)
    }
}

/// What takes the left column of the side layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Left {
    Terminal,
    File,
}

/// Whether the selected row shows its files under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tree {
    Closed,
    Open,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Counting {
    Off,
    On,
}

pub(crate) struct OpenDiff {
    pub(crate) diff: FileDiff,
    pub(crate) body: DiffBody,
}

#[derive(Default)]
struct Tickets {
    history: u64,
    status: u64,
    files: u64,
    diff: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Request {
    Notes,
    Watcher,
    History,
    Status,
    Files,
}

enum Watching {
    Off,
    Starting,
    On { _watcher: Watcher },
}

pub(crate) struct TabView {
    id: u64,
    pub(crate) dir: PathBuf,
    pub(crate) is_main: bool,
    notes_file: Option<PathBuf>,
    active: bool,
    pub(crate) scope: Scope,
    pub(crate) loaded: Option<Loaded>,
    pub(crate) wip: Option<Vec<FileChange>>,
    pick: Option<Pick>,
    pub(crate) graph: GraphView,
    source_requested: Option<Source>,
    pub(crate) shown_source: Option<Source>,
    pub(crate) files: Vec<FileChange>,
    pub(crate) counts: Vec<Option<LineCounts>>,
    pub(crate) file: Option<String>,
    tree: Tree,
    left: Left,
    opened: Option<String>,
    room_for: Option<Selection>,
    counting: Counting,
    pub(crate) open: Option<OpenDiff>,
    pub(crate) layout: DiffLayout,
    pub(crate) summary: Option<(ObjectId, Summary)>,
    pub(crate) notes: Notes,
    pub(crate) notes_ready: bool,
    pub(crate) error: Option<String>,
    tickets: Tickets,
    want: HashSet<Request>,
    want_file: Option<String>,
    watching: Watching,
    changes: (Sender<Change>, Receiver<Change>),
}

impl TabView {
    pub(crate) fn new(id: u64, dir: PathBuf, is_main: bool, notes_file: Option<PathBuf>) -> Self {
        Self {
            id,
            dir,
            is_main,
            notes_file,
            active: false,
            scope: Scope::Current,
            loaded: None,
            wip: None,
            pick: None,
            graph: GraphView::new(),
            source_requested: None,
            shown_source: None,
            files: Vec::new(),
            counts: Vec::new(),
            file: None,
            tree: Tree::Closed,
            left: Left::Terminal,
            opened: None,
            room_for: None,
            counting: Counting::Off,
            open: None,
            layout: DiffLayout::default(),
            summary: None,
            notes: Notes::default(),
            notes_ready: false,
            error: None,
            tickets: Tickets::default(),
            want: HashSet::new(),
            want_file: None,
            watching: Watching::Off,
            changes: channel(),
        }
    }

    pub(crate) fn notes_file(&self) -> Option<&PathBuf> {
        self.notes_file.as_ref()
    }

    /// The listener wrote this tab's notes file behind the app's copy (an agent replied): read it
    /// again, so the next save does not overwrite the reply.
    pub(crate) fn reload_notes(&mut self) {
        self.notes_ready = false;
        self.want.insert(Request::Notes);
    }

    /// A job rewrote the notes file (sent marks, pulled PR comments, a post link) and answers with
    /// what it saved.
    pub(crate) fn set_notes(&mut self, notes: Notes) {
        self.notes = notes;
        self.notes_ready = true;
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active
    }

    /// The tab became the visible one: reload everything it shows and watch its folder.
    pub(crate) fn start(&mut self) {
        self.active = true;
        self.want.insert(Request::History);
        self.want.insert(Request::Status);
        self.want.insert(Request::Files);
        if !self.notes_ready {
            self.want.insert(Request::Notes);
        }
        if matches!(self.watching, Watching::Off) {
            self.want.insert(Request::Watcher);
        }
    }

    /// The tab was hidden: stop watching so it costs nothing.
    pub(crate) fn stop(&mut self) {
        self.active = false;
        self.watching = Watching::Off;
        self.want.remove(&Request::Watcher);
    }

    pub(crate) fn set_scope(&mut self, scope: Scope) {
        if self.scope != scope {
            self.scope = scope;
            self.want.insert(Request::History);
        }
    }

    pub(crate) fn select(&mut self, selection: Selection) {
        self.pick = match selection {
            Selection::Wip => Some(Pick::WorkTree),
            Selection::Commit(row) => self.loaded.as_ref().map(|l| Pick::Commit(l.history.id(row))),
        };
    }

    pub(crate) fn select_file(&mut self, path: &str) {
        self.file = Some(path.to_owned());
        self.want_file = Some(path.to_owned());
    }

    /// A click on a row of the tree: it opens its files, or closes them when they were open. The
    /// uncommitted changes have no room under their row, so a click there opens their first file.
    pub(crate) fn click_row(&mut self, selection: Selection) {
        if self.expanded() && self.selection() == Some(selection) {
            self.tree = Tree::Closed;
            return;
        }
        self.select(selection);
        if selection == Selection::Wip {
            self.tree = Tree::Closed;
            if !self.is_loading_source()
                && let Some(path) = self.files.first().map(|file| file.path.clone())
            {
                self.open_file(&path);
            }
            return;
        }
        self.expand();
    }

    pub(crate) fn expanded(&self) -> bool {
        self.tree == Tree::Open
    }

    pub(crate) fn expand(&mut self) {
        self.tree = Tree::Open;
        self.room_for = self.selection();
    }

    /// Once per opening: the row the graph should scroll into view with its card.
    pub(crate) fn take_room(&mut self) -> Option<Selection> {
        self.room_for.take()
    }

    /// A click on a file of the open commit: its diff takes the terminal's place.
    pub(crate) fn open_file(&mut self, path: &str) {
        self.select_file(path);
        self.left = Left::File;
        self.opened = Some(path.to_owned());
    }

    pub(crate) fn file_open(&self) -> bool {
        self.left == Left::File
    }

    pub(crate) fn back_to_terminal(&mut self) {
        self.left = Left::Terminal;
    }

    /// Flips between the terminal and the last file opened. Returns false when there is no file to
    /// go to yet.
    pub(crate) fn toggle_file_view(&mut self) -> bool {
        if self.file_open() {
            self.back_to_terminal();
            return true;
        }
        let can = self.opened.is_some() && self.file.is_some();
        if can {
            self.left = Left::File;
        }
        can
    }

    pub(crate) fn step_file(&mut self, delta: isize) {
        if let Some(path) =
            file_nav::step_file(&self.files, self.file.as_deref(), delta).map(str::to_owned)
        {
            self.open_file(&path);
        }
    }

    /// The previous or next row of the tree, opened, with the file view staying where it is.
    pub(crate) fn step_commit(&mut self, delta: isize) {
        let Some(loaded) = &self.loaded else { return };
        let next = file_nav::step_row(
            self.selection(),
            !self.changes().is_empty(),
            loaded.history.len(),
            delta,
        );
        if let Some(selection) = next {
            self.select(selection);
            self.expand();
        }
    }

    /// Whether the file jobs also count each file's lines, which only the side layout shows.
    pub(crate) fn set_counted(&mut self, on: bool) {
        let counting = if on { Counting::On } else { Counting::Off };
        if counting == Counting::On && self.counting == Counting::Off {
            self.want.insert(Request::Files);
        }
        self.counting = counting;
    }

    pub(crate) fn reload_after_checkout(&mut self) {
        self.want.insert(Request::History);
        self.want.insert(Request::Status);
        self.want.insert(Request::Files);
    }

    /// The uncommitted files, empty until the status has loaded.
    pub(crate) fn changes(&self) -> &[FileChange] {
        self.wip.as_deref().unwrap_or_default()
    }

    pub(crate) fn status_known(&self) -> bool {
        self.wip.is_some()
    }

    pub(crate) fn wanted_pick(&self) -> Option<Pick> {
        let loaded = self.loaded.as_ref().filter(|_| self.wip.is_some())?;
        let head = loaded.head_row.map(|row| loaded.history.id(row));
        resolve_pick(self.pick, !self.changes().is_empty(), head, |id| {
            loaded.history.row(&id).is_some()
        })
    }

    pub(crate) fn selection(&self) -> Option<Selection> {
        let loaded = self.loaded.as_ref()?;
        match self.wanted_pick()? {
            Pick::WorkTree => Some(Selection::Wip),
            Pick::Commit(id) => loaded.history.row(&id).map(Selection::Commit),
        }
    }

    /// The commit the diff pane describes, `None` while it shows the working tree.
    pub(crate) fn wanted_commit(&self) -> Option<ObjectId> {
        match self.wanted_pick()? {
            Pick::Commit(id) => Some(id),
            Pick::WorkTree => None,
        }
    }

    /// True while the pane still shows a different source than the one picked.
    pub(crate) fn is_loading_source(&self) -> bool {
        self.wanted_pick().map(Pick::source) != self.shown_source
    }

    /// Merges what the watcher reported since the last frame into reload requests.
    pub(crate) fn poll(&mut self) {
        let mut merged = Change::default();
        for change in self.changes.1.try_iter() {
            merged.history |= change.history;
            merged.status |= change.status;
        }
        if merged.history {
            self.want.insert(Request::History);
        }
        if merged.history || merged.status {
            self.want.insert(Request::Status);
            if self.source_requested == Some(Source::WorkTree) {
                self.want.insert(Request::Files);
            }
        }
    }

    /// Starts a job for everything wanted and not yet requested. Safe to call every frame.
    pub(crate) fn sync(&mut self, jobs: &Jobs<Done>, ctx: &egui::Context) {
        if !self.active {
            return;
        }
        let id = self.id;
        if self.want.remove(&Request::Notes) {
            match &self.notes_file {
                Some(file) => pane_jobs::load_notes(jobs, ctx, id, file.clone()),
                None => self.notes_ready = true,
            }
        }
        if self.want.remove(&Request::Watcher) {
            self.watching = Watching::Starting;
            pane_jobs::watch(jobs, ctx, id, self.dir.clone(), self.changes.0.clone());
        }
        if self.want.remove(&Request::History) {
            self.tickets.history += 1;
            pane_jobs::load_history(
                jobs,
                ctx,
                id,
                self.tickets.history,
                self.dir.clone(),
                self.scope,
            );
        }
        if self.want.remove(&Request::Status) {
            self.tickets.status += 1;
            pane_jobs::load_status(jobs, ctx, id, self.tickets.status, self.dir.clone());
        }
        let wanted = self.wanted_pick().map(Pick::source);
        let refresh = self.want.remove(&Request::Files);
        if wanted != self.source_requested || (refresh && wanted.is_some()) {
            self.source_requested = wanted;
            self.tickets.files += 1;
            self.tickets.diff += 1;
            match wanted {
                Some(source) => pane_jobs::load_files(
                    jobs,
                    ctx,
                    id,
                    self.tickets.files,
                    self.dir.clone(),
                    source,
                    self.file.clone(),
                    self.counting == Counting::On,
                ),
                None => self.clear_diff(),
            }
        }
        if let Some(path) = self.want_file.take()
            && let Some(source) = self.source_requested
        {
            self.tickets.diff += 1;
            pane_jobs::load_diff(jobs, ctx, id, self.tickets.diff, self.dir.clone(), source, path);
        }
        self.cache_summary();
    }

    fn clear_diff(&mut self) {
        self.files.clear();
        self.counts.clear();
        self.open = None;
        self.shown_source = None;
    }

    fn cache_summary(&mut self) {
        let Some(id) = self.wanted_commit() else {
            self.summary = None;
            return;
        };
        if self.summary.as_ref().is_some_and(|(cached, _)| *cached == id) {
            return;
        }
        self.summary = self.loaded.as_ref().map(|l| (id, l.summaries.get(id)));
    }

    /// Takes the answer of a job for this tab. Answers to requests a newer one replaced are
    /// dropped.
    pub(crate) fn apply(&mut self, done: Done) {
        match done {
            Done::History { ticket, scope, result, .. } => {
                if ticket == self.tickets.history && scope == self.scope {
                    self.finish_history(result);
                }
            }
            Done::Status { ticket, result, .. } => {
                if ticket == self.tickets.status {
                    self.finish_status(result);
                }
            }
            Done::Files { ticket, source, result, .. } => {
                if ticket == self.tickets.files && Some(source) == self.source_requested {
                    self.finish_files(source, result);
                }
            }
            Done::Diff { ticket, result, .. } => {
                if ticket == self.tickets.diff {
                    self.finish_diff(result);
                }
            }
            Done::Notes { notes, .. } => {
                self.notes = *notes;
                self.notes_ready = true;
            }
            Done::Watching { result, .. } => self.finish_watching(result),
            Done::Checkout { .. } => {}
        }
    }

    fn finish_history(&mut self, result: Result<Box<Loaded>, String>) {
        match result {
            Ok(loaded) => {
                self.loaded = Some(*loaded);
                self.graph.clear_cache();
                self.error = None;
            }
            Err(message) => self.error = Some(message),
        }
    }

    fn finish_status(&mut self, result: Result<kelp_core::status::WorkingStatus, String>) {
        match result {
            Ok(status) => {
                self.wip = Some(status.all());
            }
            Err(message) => self.error = Some(message),
        }
    }

    fn finish_files(&mut self, source: Source, result: Result<Box<pane_jobs::Files>, String>) {
        self.tickets.diff += 1;
        match result {
            Ok(files) => {
                let pane_jobs::Files { list, shown, counts } = *files;
                self.files = list;
                self.counts = counts;
                self.shown_source = Some(source);
                self.error = None;
                match shown {
                    Some((path, Ok(diff))) => {
                        self.file = Some(path);
                        self.show_diff(diff);
                    }
                    Some((path, Err(message))) => {
                        self.file = Some(path);
                        self.open = None;
                        self.error = Some(message);
                    }
                    None => self.open = None,
                }
            }
            Err(message) => {
                self.clear_diff();
                self.error = Some(message);
            }
        }
    }

    fn finish_diff(&mut self, result: Result<Box<FileDiff>, String>) {
        match result {
            Ok(diff) => {
                self.error = None;
                self.show_diff(*diff);
            }
            Err(message) => {
                self.open = None;
                self.error = Some(message);
            }
        }
    }

    fn show_diff(&mut self, diff: FileDiff) {
        match &mut self.open {
            Some(open) if open.diff.path == diff.path => {
                open.body.reload(&diff);
                open.diff = diff;
            }
            _ => {
                let mut body = DiffBody::new(&diff);
                body.set_layout(self.layout);
                self.open = Some(OpenDiff { diff, body });
            }
        }
    }

    fn finish_watching(&mut self, result: Result<Watcher, String>) {
        match result {
            Ok(watcher) if self.active => self.watching = Watching::On { _watcher: watcher },
            Ok(_) => {}
            Err(message) => {
                eprintln!("tidewatch: cannot watch {}: {message}", self.dir.display());
                self.watching = Watching::Off;
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn watching(&self) -> bool {
        matches!(self.watching, Watching::On { .. })
    }

    #[cfg(test)]
    pub(crate) fn change_sender(&self) -> Sender<Change> {
        self.changes.0.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::{Duration, Instant};

    fn id(n: u8) -> ObjectId {
        ObjectId::from_hex(format!("{n:02x}").repeat(20).as_bytes()).unwrap()
    }

    fn tab(pinned: bool, pane: Pane<u8>) -> Tab<Pane<u8>> {
        Tab { id: 3, group: 0, title: "t".into(), pinned, pane }
    }

    fn repo_workspace() -> Workspace {
        Workspace::Repo(crate::repo::Repo { root: "/work/web".into(), label: "web".into() })
    }

    fn worktree() -> tidewatch_core::worktree::Worktree {
        tidewatch_core::worktree::Worktree {
            repo: "/work/web".into(),
            path: "/home/.tidewatch/web-abc123/brave-otter".into(),
            branch: "agent/brave-otter".into(),
        }
    }

    #[test]
    fn the_main_tab_reads_the_repo_root_and_keeps_notes_under_main() {
        let paths = TidewatchPaths::new(std::path::Path::new("/home"));
        let spec =
            ViewSpec::of(&repo_workspace(), Some(&paths), &tab(true, Pane::shell(0))).unwrap();
        assert_eq!(spec.dir, PathBuf::from("/work/web"));
        assert_eq!((spec.name.as_str(), spec.is_main, spec.id), ("main", true, 3));
        assert_eq!(
            spec.notes_file,
            Some(paths.notes_file(std::path::Path::new("/work/web"), "main"))
        );
    }

    #[test]
    fn a_worktree_tab_reads_its_worktree_and_keeps_notes_under_its_name() {
        let paths = TidewatchPaths::new(std::path::Path::new("/home"));
        let pane = Pane { body: Body::Shell(0), worktree: Some(worktree()), agent: None };
        let spec = ViewSpec::of(&repo_workspace(), Some(&paths), &tab(false, pane)).unwrap();
        assert_eq!(spec.dir, worktree().path);
        assert_eq!((spec.name.as_str(), spec.is_main), ("brave-otter", false));
        assert_eq!(
            spec.notes_file,
            Some(paths.notes_file(std::path::Path::new("/work/web"), "brave-otter"))
        );
    }

    #[test]
    fn tabs_without_a_running_shell_or_outside_a_repo_have_no_panes() {
        let other = Workspace::Other { dir: "/notes".into(), label: "notes".into() };
        assert_eq!(ViewSpec::of(&other, None, &tab(true, Pane::shell(0))), None);
        assert_eq!(ViewSpec::of(&repo_workspace(), None, &tab(false, Pane::creating())), None);
        assert_eq!(ViewSpec::of(&repo_workspace(), None, &tab(false, Pane::shell(0))), None);
        let failed =
            Pane { body: Body::Failed("x".into()), worktree: Some(worktree()), agent: None };
        assert_eq!(ViewSpec::of(&repo_workspace(), None, &tab(false, failed)), None);
    }

    #[test]
    fn a_draft_reads_the_repo_root_for_its_history_and_keeps_no_notes() {
        let paths = TidewatchPaths::new(std::path::Path::new("/home"));
        let form = crate::new_tab::NewTab::open(
            crate::new_tab::Group::Repo,
            "web".into(),
            "/work/web".into(),
            Vec::new(),
            None,
            1,
        );
        let spec =
            ViewSpec::of(&repo_workspace(), Some(&paths), &tab(false, Pane::draft(form))).unwrap();
        assert_eq!(spec.dir, std::path::Path::new("/work/web"));
        assert_eq!((spec.is_main, spec.notes_file), (false, None));
    }

    #[test]
    fn without_a_home_folder_notes_have_no_file() {
        let spec = ViewSpec::of(&repo_workspace(), None, &tab(true, Pane::shell(0))).unwrap();
        assert_eq!(spec.notes_file, None);
    }

    #[test]
    fn the_users_pick_wins_while_it_still_exists() {
        let known = |c: ObjectId| c == id(1) || c == id(2);
        let table = [
            (Some(Pick::Commit(id(1))), true, Some(Pick::Commit(id(1)))),
            (Some(Pick::Commit(id(1))), false, Some(Pick::Commit(id(1)))),
            (Some(Pick::WorkTree), true, Some(Pick::WorkTree)),
        ];
        for (pick, has_wip, want) in table {
            assert_eq!(resolve_pick(pick, has_wip, Some(id(2)), known), want, "{pick:?} {has_wip}");
        }
    }

    #[test]
    fn without_a_pick_changes_come_first_then_head() {
        let known = |_: ObjectId| true;
        assert_eq!(resolve_pick(None, true, Some(id(2)), known), Some(Pick::WorkTree));
        assert_eq!(resolve_pick(None, false, Some(id(2)), known), Some(Pick::Commit(id(2))));
        assert_eq!(resolve_pick(None, false, None, known), None);
    }

    #[test]
    fn a_vanished_pick_falls_back() {
        let none_known = |_: ObjectId| false;
        assert_eq!(
            resolve_pick(Some(Pick::Commit(id(9))), false, Some(id(2)), none_known),
            Some(Pick::Commit(id(2)))
        );
        assert_eq!(
            resolve_pick(Some(Pick::WorkTree), false, Some(id(2)), none_known),
            Some(Pick::Commit(id(2)))
        );
        assert_eq!(
            resolve_pick(Some(Pick::Commit(id(9))), true, Some(id(2)), none_known),
            Some(Pick::WorkTree)
        );
    }

    fn git(dir: &std::path::Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.email=t@example.com",
                "-c",
                "user.name=T",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    fn repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().canonicalize().unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "first"]);
        std::fs::write(repo.join("b.txt"), "b\n").unwrap();
        std::fs::write(repo.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "second"]);
        (dir, repo)
    }

    fn view_for(repo: &std::path::Path, notes: PathBuf) -> TabView {
        TabView::new(7, repo.to_path_buf(), true, Some(notes))
    }

    fn pump(
        view: &mut TabView,
        jobs: &Jobs<Done>,
        ctx: &egui::Context,
        until: impl Fn(&TabView) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !until(view) {
            assert!(Instant::now() < deadline, "the view never settled");
            view.poll();
            view.sync(jobs, ctx);
            for done in jobs.drain() {
                view.apply(done);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_started_view_loads_history_status_notes_and_the_head_diff() {
        let (_dir, repo) = repo();
        let notes_dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let jobs = Jobs::new();
        let mut view = view_for(&repo, notes_dir.path().join("notes.json"));

        view.start();
        pump(&mut view, &jobs, &ctx, |v| v.open.is_some() && v.notes_ready && v.watching());

        assert_eq!(view.changes(), []);
        assert_eq!(view.selection(), Some(Selection::Commit(0)));
        let mut listed: Vec<&str> = view.files.iter().map(|f| f.path.as_str()).collect();
        listed.sort_unstable();
        assert_eq!(listed, ["a.txt", "b.txt"]);
        assert_eq!(view.open.as_ref().unwrap().diff.path, "a.txt");
        assert_eq!(view.summary.as_ref().map(|(_, s)| s.title.as_str()), Some("second"));
        assert!(!view.is_loading_source());
    }

    #[test]
    fn uncommitted_changes_are_shown_first_and_follow_the_files() {
        let (_dir, repo) = repo();
        std::fs::write(repo.join("c.txt"), "fresh\n").unwrap();
        let notes_dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let jobs = Jobs::new();
        let mut view = view_for(&repo, notes_dir.path().join("notes.json"));

        view.start();
        pump(&mut view, &jobs, &ctx, |v| v.open.is_some());

        assert_eq!(view.selection(), Some(Selection::Wip));
        assert_eq!(view.open.as_ref().unwrap().diff.path, "c.txt");
        assert_eq!(view.wanted_commit(), None);
        assert_eq!(view.summary.as_ref().map(|(_, s)| s.title.clone()), None);
    }

    #[test]
    fn picking_a_commit_row_and_a_file_loads_that_diff() {
        let (_dir, repo) = repo();
        let notes_dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let jobs = Jobs::new();
        let mut view = view_for(&repo, notes_dir.path().join("notes.json"));
        view.start();
        pump(&mut view, &jobs, &ctx, |v| v.open.is_some());

        view.select_file("b.txt");
        pump(&mut view, &jobs, &ctx, |v| v.open.as_ref().is_some_and(|o| o.diff.path == "b.txt"));

        view.select(Selection::Commit(1));
        pump(&mut view, &jobs, &ctx, |v| {
            v.open.as_ref().is_some_and(|o| o.diff.path == "a.txt") && !v.is_loading_source()
        });
        assert_eq!(view.summary.as_ref().map(|(_, s)| s.title.as_str()), Some("first"));
        assert_eq!(view.files.len(), 1);
    }

    #[test]
    fn a_watcher_event_reloads_the_working_tree_without_a_new_selection() {
        let (_dir, repo) = repo();
        std::fs::write(repo.join("c.txt"), "fresh\n").unwrap();
        let notes_dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let jobs = Jobs::new();
        let mut view = view_for(&repo, notes_dir.path().join("notes.json"));
        view.start();
        pump(&mut view, &jobs, &ctx, |v| v.open.is_some() && v.watching());

        std::fs::write(repo.join("c.txt"), "fresh\nmore\n").unwrap();
        view.change_sender().send(Change { history: false, status: true }).unwrap();
        pump(&mut view, &jobs, &ctx, |v| v.open.as_ref().is_some_and(|o| o.diff.added == 2));
    }

    #[test]
    fn committing_the_changes_moves_the_selection_to_the_new_commit() {
        let (_dir, repo) = repo();
        std::fs::write(repo.join("c.txt"), "fresh\n").unwrap();
        let notes_dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let jobs = Jobs::new();
        let mut view = view_for(&repo, notes_dir.path().join("notes.json"));
        view.start();
        pump(&mut view, &jobs, &ctx, |v| v.open.is_some() && v.watching());

        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "third"]);
        view.change_sender().send(Change { history: true, status: true }).unwrap();
        pump(&mut view, &jobs, &ctx, |v| {
            v.summary.as_ref().is_some_and(|(_, s)| s.title == "third") && !v.is_loading_source()
        });
        assert_eq!(view.changes(), []);
    }

    #[test]
    fn answers_to_replaced_requests_are_dropped() {
        let (_dir, repo) = repo();
        let mut view = view_for(&repo, repo.join("notes.json"));
        view.tickets.status = 5;
        view.apply(Done::Status {
            tab: 7,
            ticket: 4,
            result: Ok(kelp_core::status::WorkingStatus::default()),
        });
        assert!(!view.status_known());
        view.apply(Done::Status {
            tab: 7,
            ticket: 5,
            result: Ok(kelp_core::status::WorkingStatus::default()),
        });
        assert!(view.status_known());
    }

    #[test]
    fn nothing_is_picked_until_both_history_and_status_have_arrived() {
        let (_dir, repo) = repo();
        let mut view = view_for(&repo, repo.join("notes.json"));
        view.tickets.history = 1;
        view.apply(Done::History {
            tab: 7,
            ticket: 1,
            scope: Scope::Current,
            result: Ok(Box::new(tidewatch_core::history::load(&repo, Scope::Current).unwrap())),
        });
        assert_eq!(view.wanted_pick(), None);
        view.tickets.status = 1;
        view.apply(Done::Status {
            tab: 7,
            ticket: 1,
            result: Ok(kelp_core::status::WorkingStatus::default()),
        });
        assert!(matches!(view.wanted_pick(), Some(Pick::Commit(_))));
    }

    #[test]
    fn switching_the_scope_asks_for_a_new_history_and_ignores_the_old_answer() {
        let (_dir, repo) = repo();
        let mut view = view_for(&repo, repo.join("notes.json"));
        view.tickets.history = 1;
        view.set_scope(Scope::Whole);
        assert!(view.want.contains(&Request::History));
        view.apply(Done::History {
            tab: 7,
            ticket: 1,
            scope: Scope::Current,
            result: Ok(Box::new(tidewatch_core::history::load(&repo, Scope::Current).unwrap())),
        });
        assert!(view.loaded.is_none());
    }

    fn started_view(repo: &std::path::Path, notes: PathBuf, jobs: &Jobs<Done>) -> TabView {
        let ctx = egui::Context::default();
        let mut view = view_for(repo, notes);
        view.set_counted(true);
        view.start();
        pump(&mut view, jobs, &ctx, |v| v.open.is_some() && !v.is_loading_source());
        view
    }

    #[test]
    fn a_click_on_a_row_opens_its_files_and_a_second_click_closes_them() {
        let (_dir, repo) = repo();
        let jobs = Jobs::new();
        let mut view = started_view(&repo, repo.join("notes.json"), &jobs);
        assert!(!view.expanded());
        view.click_row(Selection::Commit(0));
        assert!(view.expanded());
        view.click_row(Selection::Commit(0));
        assert!(!view.expanded());
        view.click_row(Selection::Commit(0));
        view.click_row(Selection::Commit(1));
        assert!(view.expanded(), "another row opens in place of the first");
        assert_eq!(view.selection(), Some(Selection::Commit(1)));
    }

    #[test]
    fn opening_a_row_asks_for_its_room_once() {
        let (_dir, repo) = repo();
        let jobs = Jobs::new();
        let mut view = started_view(&repo, repo.join("notes.json"), &jobs);
        assert_eq!(view.take_room(), None);
        view.click_row(Selection::Commit(1));
        assert_eq!(view.take_room(), Some(Selection::Commit(1)));
        assert_eq!(view.take_room(), None);
        view.step_commit(-1);
        assert_eq!(view.take_room(), Some(Selection::Commit(0)));
    }

    #[test]
    fn the_uncommitted_row_has_no_room_for_a_card_so_a_click_opens_its_first_file() {
        let (_dir, repo) = repo();
        std::fs::write(repo.join("c.txt"), "fresh\n").unwrap();
        let jobs = Jobs::new();
        let mut view = started_view(&repo, repo.join("notes.json"), &jobs);
        view.click_row(Selection::Wip);
        assert!(!view.expanded());
        assert_eq!(view.take_room(), None);
        assert_eq!(view.file.as_deref(), Some("c.txt"));
        assert!(view.file_open());
    }

    #[test]
    fn the_file_jobs_count_each_files_lines_once_counting_is_on() {
        let (_dir, repo) = repo();
        let jobs = Jobs::new();
        let view = started_view(&repo, repo.join("notes.json"), &jobs);
        let mut got: Vec<_> = view
            .files
            .iter()
            .zip(&view.counts)
            .map(|(file, counts)| (file.path.as_str(), *counts))
            .collect();
        got.sort_unstable();
        assert_eq!(got, [("a.txt", Some((1, 0))), ("b.txt", Some((1, 0)))]);
    }

    #[test]
    fn the_toggle_needs_a_file_that_was_opened_and_goes_back_and_forth() {
        let (_dir, repo) = repo();
        let jobs = Jobs::new();
        let mut view = started_view(&repo, repo.join("notes.json"), &jobs);
        assert!(!view.toggle_file_view(), "the auto-loaded first diff is not an opened file");
        assert!(!view.file_open());
        view.open_file("b.txt");
        assert!(view.file_open());
        view.back_to_terminal();
        assert!(!view.file_open());
        assert!(view.toggle_file_view());
        assert!(view.file_open());
        assert!(view.toggle_file_view());
        assert!(!view.file_open());
    }

    #[test]
    fn stepping_files_opens_the_neighbour_and_stops_at_the_ends() {
        let (_dir, repo) = repo();
        let jobs = Jobs::new();
        let ctx = egui::Context::default();
        let mut view = started_view(&repo, repo.join("notes.json"), &jobs);
        let first = view.files[0].path.clone();
        let second = view.files[1].path.clone();
        view.select_file(&first);
        view.step_file(1);
        assert_eq!(view.file.as_deref(), Some(second.as_str()));
        assert!(view.file_open());
        view.step_file(1);
        assert_eq!(view.file.as_deref(), Some(second.as_str()));
        view.step_file(-1);
        assert_eq!(view.file.as_deref(), Some(first.as_str()));
        pump(&mut view, &jobs, &ctx, |v| v.open.as_ref().is_some_and(|o| o.diff.path == first));
    }

    #[test]
    fn stepping_commits_moves_the_selection_opens_it_and_leaves_the_file_view_alone() {
        let (_dir, repo) = repo();
        let jobs = Jobs::new();
        let mut view = started_view(&repo, repo.join("notes.json"), &jobs);
        view.open_file("a.txt");
        view.step_commit(1);
        assert_eq!(view.selection(), Some(Selection::Commit(1)));
        assert!(view.expanded() && view.file_open());
        view.step_commit(1);
        assert_eq!(view.selection(), Some(Selection::Commit(1)), "the oldest commit is the end");
        view.step_commit(-1);
        assert_eq!(view.selection(), Some(Selection::Commit(0)));
    }

    #[test]
    fn a_hidden_view_stops_watching_and_syncs_nothing() {
        let (_dir, repo) = repo();
        let ctx = egui::Context::default();
        let jobs = Jobs::new();
        let mut view = view_for(&repo, repo.join("notes.json"));
        view.start();
        view.stop();
        view.sync(&jobs, &ctx);
        assert!(!view.is_active());
        assert!(!view.watching());
        assert!(matches!(jobs.drain().as_slice(), []));
    }
}
