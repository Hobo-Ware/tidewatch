use std::fs;
use std::process::Command;
use std::time::{Duration, Instant};

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, pos2, vec2};
use kelp_ui::avatars::AvatarStore;
use tidewatch_core::folder_repos::{self, Target};

use super::*;
use crate::history_pane::{self, Out, Panel};
use crate::jobs::Jobs;
use crate::pane_jobs::Done;
use crate::progress_view::TabState;
use crate::tab_view::{TabView, ViewSpec};
use crate::test_support::painted_text_rects;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@example.com", "-c", "user.name=T", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn make_repo(parent: &Path, name: &str, branch: &str, message: &str) {
    let root = parent.join(name);
    fs::create_dir(&root).unwrap();
    git(&root, &["init", "-q", "-b", branch]);
    fs::write(root.join("a.txt"), "one\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", message]);
}

fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(),
    )
    .unwrap()
}

const SIZE: egui::Vec2 = vec2(460.0, 600.0);

struct Screen {
    ctx: egui::Context,
    avatars: AvatarStore,
    state: TabState,
    texts: Vec<(String, egui::Rect)>,
    picked: Option<PathBuf>,
}

impl Screen {
    fn new() -> Self {
        let ctx = crate::fonts::test_context();
        Self {
            avatars: AvatarStore::offline(ctx.clone()),
            ctx,
            state: TabState::default(),
            texts: Vec::new(),
            picked: None,
        }
    }

    fn run(
        &mut self,
        events: Vec<Event>,
        mut draw: impl FnMut(&mut Ui, Rect, &mut AvatarStore, &mut TabState) -> Out,
    ) {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), SIZE)),
            events,
            focused: true,
            ..RawInput::default()
        };
        let (avatars, state) = (&mut self.avatars, &mut self.state);
        let mut out = Out::default();
        let output = self.ctx.run_ui(raw, |ui| {
            out = draw(ui, Rect::from_min_size(pos2(0.0, 0.0), SIZE), avatars, state);
        });
        if out.repo.is_some() {
            self.picked = out.repo;
        }
        self.texts.clear();
        for clipped in &output.shapes {
            painted_text_rects(&clipped.shape, &mut self.texts);
        }
    }

    fn has(&self, text: &str) -> bool {
        self.texts.iter().any(|(drawn, _)| drawn == text)
    }

    fn at(&self, text: &str) -> Pos2 {
        self.texts
            .iter()
            .find(|(drawn, _)| drawn == text)
            .unwrap_or_else(|| panic!("{text:?} is not drawn: {:?}", self.texts))
            .1
            .center()
    }
}

fn click(screen: &mut Screen, at: Pos2, draw: &mut dyn FnMut(&mut Screen, Vec<Event>)) {
    let button = |pressed| Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    draw(screen, vec![Event::PointerMoved(at)]);
    draw(screen, vec![button(true)]);
    draw(screen, vec![button(false)]);
    draw(screen, Vec::new());
}

fn loaded_view(root: &Path) -> TabView {
    let ctx = egui::Context::default();
    let jobs: Jobs<Done> = Jobs::new();
    let mut view = ViewSpec::of_repo(1, root).into_view();
    view.start();
    let deadline = Instant::now() + Duration::from_secs(20);
    while view.loaded.is_none() || view.groups.is_none() {
        assert!(Instant::now() < deadline, "the view never loaded");
        view.poll();
        view.sync(&jobs, &ctx);
        for done in jobs.drain() {
            view.apply(done);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    view
}

#[test]
fn a_folder_tab_draws_the_picker_and_the_history_rows_of_the_picked_repo_and_the_list_picks_another()
 {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().canonicalize().unwrap();
    make_repo(&folder, "alpha", "main", "alpha first commit");
    make_repo(&folder, "beta", "dev", "beta first commit");
    let scan = folder_repos::scan(&folder, now_ms());
    assert_eq!(scan.repos.len(), 2);
    let edits = [folder.join("alpha/a.txt")];
    let Target::Repo(picked) = folder_repos::target(&scan, None, &edits) else {
        panic!("no repo picked")
    };
    assert_eq!(picked.name, "alpha");
    let mut view = loaded_view(&picked.root);
    let mut screen = Screen::new();
    let head = Head { repos: &scan.repos, picked: &picked.root, open: false };
    let mut draw = |screen: &mut Screen, events: Vec<Event>| {
        screen.run(events, |ui, rect, avatars, state| {
            let panel = Panel {
                agent: "",
                progress: None,
                state,
                face: None,
                folder: Some(Head { repos: head.repos, picked: head.picked, open: false }),
            };
            history_pane::show(ui, rect, &mut view, avatars, true, panel)
        });
    };
    draw(&mut screen, Vec::new());
    draw(&mut screen, Vec::new());
    assert!(screen.has("alpha"), "the picker chip names the repo: {:?}", screen.texts);
    assert!(screen.has("main"), "the header keeps the branch");
    assert!(screen.has("This branch"), "History's own switch is there");
    assert!(
        screen.texts.iter().any(|(text, _)| text.contains("alpha first commit")),
        "the History rows of the picked repo: {:?}",
        screen.texts
    );
    assert!(!screen.texts.iter().any(|(text, _)| text.contains("beta first commit")));
    assert!(!screen.has("dev"), "the list is closed");
    let chip = screen.at("alpha");
    click(&mut screen, chip, &mut draw);
    assert!(screen.has("beta"), "the open list names the other repo: {:?}", screen.texts);
    assert!(screen.has("dev"), "and its branch");
    let beta = screen.at("beta");
    click(&mut screen, beta, &mut draw);
    assert_eq!(screen.picked.as_deref(), Some(folder.join("beta").as_path()));
}

#[test]
fn a_folder_with_no_repo_draws_its_path_the_line_and_the_recent_files() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().canonicalize().unwrap();
    fs::create_dir(folder.join("sub")).unwrap();
    fs::write(folder.join("sub/report.md"), "x").unwrap();
    fs::write(folder.join("todo.txt"), "x").unwrap();
    let scan = folder_repos::scan(&folder, now_ms());
    assert_eq!(target_name(&scan), "folder");
    let mut screen = Screen::new();
    let draw = |screen: &mut Screen, events: Vec<Event>| {
        screen.run(events, |ui, rect, _, state| {
            let panel = Panel { agent: "", progress: None, state, face: None, folder: None };
            history_pane::show_folder(ui, rect, &scan, now_ms(), panel)
        });
    };
    draw(&mut screen, Vec::new());
    draw(&mut screen, Vec::new());
    let title = folder_title(&folder);
    assert!(screen.has(&title), "{title}");
    assert!(screen.has(&folder_name(&folder)), "the header names the folder");
    assert!(screen.has(NO_REPO));
    assert!(screen.has("Changed in the last hour"));
    assert!(screen.has("todo.txt") && screen.has("report.md"), "{:?}", screen.texts);
    assert!(screen.has("Changes") && !screen.has("History"), "no History without a repo");
    assert!(!screen.has(NOTHING_RECENT));
}

#[test]
fn an_empty_folder_still_says_so_and_never_draws_an_empty_panel() {
    let dir = tempfile::tempdir().unwrap();
    let scan = folder_repos::scan(dir.path(), now_ms());
    let mut screen = Screen::new();
    let draw = |screen: &mut Screen, events: Vec<Event>| {
        screen.run(events, |ui, rect, _, state| {
            let panel = Panel { agent: "", progress: None, state, face: None, folder: None };
            history_pane::show_folder(ui, rect, &scan, now_ms(), panel)
        });
    };
    draw(&mut screen, Vec::new());
    draw(&mut screen, Vec::new());
    assert!(screen.has(NO_REPO) && screen.has(NOTHING_RECENT));
    assert!(screen.has(&folder_title(&scan.folder)));
}

fn target_name(scan: &folder_repos::Scan) -> &'static str {
    match folder_repos::target(scan, None, &[]) {
        Target::Folder => "folder",
        Target::Repo(_) => "repo",
        Target::Inside(_) => "inside",
    }
}

#[test]
fn names_and_ages_are_cut_and_worded_for_the_picker_and_the_rows() {
    assert_eq!(short_name("short", 8), "short");
    assert_eq!(short_name("a-very-long-repo-name", 8), "a-ver...");
    let ages = [
        (0, "just now"),
        (59, "just now"),
        (60, "1m ago"),
        (59 * 60 + 59, "59m ago"),
        (3600, "1h ago"),
    ];
    for (secs, want) in ages {
        assert_eq!(age_text(secs), want, "{secs}");
    }
}
