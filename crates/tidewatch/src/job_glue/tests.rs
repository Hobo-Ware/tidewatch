use std::io::Write;
use std::time::{Duration, Instant};

use tidewatch_core::job::{JobState, Report};

use super::*;

const NOW: i64 = 1_000_000_000;

struct Fx {
    dir: tempfile::TempDir,
    ctx: egui::Context,
}

impl Fx {
    fn new() -> Self {
        Self { dir: tempfile::tempdir().unwrap(), ctx: egui::Context::default() }
    }

    fn notes(&self) -> PathBuf {
        self.dir.path().join("notes.json")
    }

    fn feed(&self) -> PathBuf {
        progress_feed_file_beside(&self.notes())
    }

    fn append(&self, text: &str) {
        let mut open =
            std::fs::OpenOptions::new().create(true).append(true).open(self.feed()).unwrap();
        open.write_all(text.as_bytes()).unwrap();
    }

    fn tick(&self, host: &mut Host, ids: &[u64], now: i64) -> Option<i64> {
        let notes = self.notes();
        let facts: Vec<Facts<'_>> =
            ids.iter().map(|id| Facts { id: *id, notes_file: &notes }).collect();
        host.tick(&facts, &self.ctx, now)
    }

    fn until_loaded(&self, host: &mut Host, id: u64, now: i64) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while host.tabs.get(&id).is_none_or(|tab| !tab.loaded) && Instant::now() < deadline {
            self.tick(host, &[id], now);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn reading(name: &str, done: f64, total: f64) -> Report {
    Report { done: Some(done), total: Some(total), ..Report::named(name) }
}

#[test]
fn a_tab_with_nothing_saved_starts_empty_and_is_ready_after_the_load() {
    let fx = Fx::new();
    let mut host = Host::new(true, false);
    assert!(host.board(1).is_none());
    fx.until_loaded(&mut host, 1, NOW);
    assert!(host.board(1).is_none());
    assert!(host.tabs[&1].loaded);
}

#[test]
fn a_report_that_comes_before_the_load_is_kept_and_applied_after_it() {
    let fx = Fx::new();
    let mut host = Host::new(true, false);
    fx.tick(&mut host, &[1], NOW);
    host.report(1, reading("orders", 5.0, 10.0), NOW);
    assert!(host.board(1).is_none());
    fx.until_loaded(&mut host, 1, NOW);
    assert_eq!(host.board(1).unwrap().jobs[0].done, 5.0);
}

#[test]
fn a_report_for_a_tab_that_is_not_open_is_dropped() {
    let mut host = Host::new(true, false);
    host.report(9, reading("orders", 5.0, 10.0), NOW);
    assert!(host.board(9).is_none());
}

#[test]
fn lines_appended_to_the_progress_file_become_jobs_and_survive_a_pipe_in_front_of_the_script() {
    let fx = Fx::new();
    let mut host = Host::new(true, false);
    fx.until_loaded(&mut host, 1, NOW);
    fx.append("{\"name\":\"users\",\"done\":10,\"total\":100}\nnot json\n");
    fx.append("{\"name\":\"users\",\"done\":20,\"total\":100,\"unit\":\"rows\"}\n");
    fx.tick(&mut host, &[1], NOW + 2_000);
    let job = &host.board(1).unwrap().jobs[0];
    assert_eq!((job.done, job.total, job.unit.as_str()), (20.0, Some(100.0), "rows"));
    assert_eq!(job.samples.len(), 2);
}

#[test]
fn the_file_is_looked_at_once_a_second_at_most() {
    let fx = Fx::new();
    let mut host = Host::new(true, false);
    fx.until_loaded(&mut host, 1, NOW);
    fx.tick(&mut host, &[1], NOW + 1_000);
    fx.append("{\"name\":\"a\",\"done\":1}\n");
    fx.tick(&mut host, &[1], NOW + 1_500);
    assert!(host.board(1).is_none());
    fx.tick(&mut host, &[1], NOW + 2_100);
    assert!(host.board(1).is_some());
}

#[test]
fn a_running_job_wakes_the_frame_for_the_next_look_and_the_stall_moment() {
    let fx = Fx::new();
    let mut host = Host::new(true, false);
    fx.until_loaded(&mut host, 1, NOW);
    assert_eq!(fx.tick(&mut host, &[1], NOW), None);
    host.report(1, reading("orders", 5.0, 10.0), NOW);
    assert_eq!(fx.tick(&mut host, &[1], NOW + 10), Some(NOW + 10 + RUNNING_POLL_MS));
}

#[test]
fn five_quiet_minutes_stall_the_job() {
    let fx = Fx::new();
    let mut host = Host::new(true, false);
    fx.until_loaded(&mut host, 1, NOW);
    host.report(1, reading("orders", 5.0, 10.0), NOW);
    fx.tick(&mut host, &[1], NOW + 5 * MIN_MS);
    assert_eq!(host.board(1).unwrap().jobs[0].state, JobState::Stalled);
}

#[test]
fn a_changed_board_is_saved_after_a_pause_and_loads_back_with_the_tab() {
    let fx = Fx::new();
    let mut host = Host::new(true, false);
    fx.until_loaded(&mut host, 1, NOW);
    host.report(1, reading("orders", 5.0, 10.0), NOW);
    fx.tick(&mut host, &[1], NOW + 10);
    assert!(!jobs_file_beside(&fx.notes()).exists(), "not before the pause");
    let wake = fx.tick(&mut host, &[1], NOW + SAVE_EVERY_MS + 10);
    assert!(wake.is_some());
    let file = jobs_file_beside(&fx.notes());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !file.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut again = Host::new(true, false);
    fx.until_loaded(&mut again, 1, NOW + 20_000);
    assert_eq!(again.board(1).unwrap().jobs[0].name, "orders");
}

#[test]
fn a_restored_tab_does_not_replay_the_lines_its_saved_board_already_holds() {
    let fx = Fx::new();
    fx.append("{\"name\":\"orders\",\"done\":5,\"total\":10}\n");
    let mut saved = JobBoard::default();
    saved.report(&reading("orders", 5.0, 10.0), NOW - 60_000);
    job_store::save(&jobs_file_beside(&fx.notes()), &saved).unwrap();
    let mut host = Host::new(true, false);
    fx.until_loaded(&mut host, 1, NOW);
    fx.tick(&mut host, &[1], NOW + 2_000);
    assert_eq!(host.board(1).unwrap().jobs[0].samples.len(), 1);
    fx.append("{\"name\":\"orders\",\"done\":6,\"total\":10}\n");
    fx.tick(&mut host, &[1], NOW + 4_000);
    assert_eq!(host.board(1).unwrap().jobs[0].samples.len(), 2);
}

#[test]
fn nothing_is_written_when_the_host_may_not_write() {
    let fx = Fx::new();
    let mut host = Host::new(false, false);
    fx.until_loaded(&mut host, 1, NOW);
    host.report(1, reading("orders", 5.0, 10.0), NOW);
    fx.tick(&mut host, &[1], NOW + 10 * SAVE_EVERY_MS);
    std::thread::sleep(Duration::from_millis(50));
    assert!(!jobs_file_beside(&fx.notes()).exists());
}

#[test]
fn a_tab_that_closed_takes_its_board_with_it() {
    let fx = Fx::new();
    let mut host = Host::new(true, false);
    fx.until_loaded(&mut host, 1, NOW);
    host.report(1, reading("orders", 5.0, 10.0), NOW);
    fx.tick(&mut host, &[], NOW + 10);
    assert!(host.board(1).is_none());
}

#[test]
fn the_terminal_scan_reads_only_lines_it_has_not_seen_and_waits_between_looks() {
    let fx = Fx::new();
    let mut host = Host::new(false, false);
    fx.until_loaded(&mut host, 1, NOW);
    let known = Report { done: Some(1.0), total: Some(10.0), ..Report::named("orders") };
    host.report(1, known, NOW);
    let screen = |lines: &[&str]| lines.iter().map(|line| (*line).to_owned()).collect::<Vec<_>>();
    let first = screen(&["noise", "orders [###-------] 30% 3/10", "more noise"]);
    assert_eq!(host.scan(1, 1, NOW + 5_000, || first.clone()), None);
    assert_eq!(host.board(1).unwrap().jobs[0].done, 3.0);
    assert_eq!(host.scan(1, 1, NOW + 6_000, || panic!("no new output")), None);
    assert_eq!(host.scan(1, 2, NOW + 5_500, || panic!("too soon")), Some(NOW + 6_000));
    let again = screen(&["orders [###-------] 30% 3/10", "orders [#####-----] 50% 5/10"]);
    assert_eq!(host.scan(1, 2, NOW + 6_000, || again.clone()), None);
    let job = &host.board(1).unwrap().jobs[0];
    assert_eq!((job.done, job.samples.len()), (5.0, 3));
}

#[test]
fn a_line_from_the_terminal_never_starts_a_job() {
    let fx = Fx::new();
    let mut host = Host::new(false, false);
    fx.until_loaded(&mut host, 1, NOW);
    let screen = vec!["orders [###-------] 30% 3/10".to_owned(), "[2/5] building".to_owned()];
    assert_eq!(host.scan(1, 1, NOW + 5_000, || screen.clone()), None);
    assert!(host.board(1).is_none());
}

#[test]
fn the_terminal_scan_ignores_a_tab_it_does_not_know() {
    let mut host = Host::new(false, false);
    assert_eq!(host.scan(4, 1, NOW, || panic!("unknown tab")), None);
}

#[test]
fn the_fake_board_has_the_jobs_of_the_mockup() {
    let board = fake_board(NOW);
    let by_name = |name: &str| board.jobs.iter().find(|job| job.name == name).unwrap();
    let orders = by_name("Backfill orders");
    assert_eq!(orders.state, JobState::Running);
    assert_eq!(orders.percent().map(|p| (p * 10.0).round()), Some(212.0));
    assert!(orders.samples.len() >= 10);
    assert!(orders.eta_left(NOW).is_some());
    let checks = by_name("PR checks");
    assert!(checks.is_segmented());
    assert_eq!((checks.done, checks.total), (3.0, Some(5.0)));
    let users = by_name("Import users");
    assert_eq!(users.state, JobState::Stalled);
    assert_eq!(users.quiet_ms(NOW), 6 * MIN_MS);
    assert_eq!(by_name("cargo test").state, JobState::Done);
    assert!(by_name("Worktree setup").total.is_none());
    assert!(by_name("Deploy staging").is_timed());
}

#[test]
fn the_fake_switch_fills_every_new_tab_and_never_reads_or_writes() {
    let fx = Fx::new();
    let mut host = Host::new(true, true);
    fx.tick(&mut host, &[1, 2], NOW);
    assert_eq!(host.board(1).unwrap().jobs.len(), 6);
    assert_eq!(host.board(2).unwrap().jobs.len(), 6);
    fx.tick(&mut host, &[1, 2], NOW + 10 * SAVE_EVERY_MS);
    std::thread::sleep(Duration::from_millis(50));
    assert!(!jobs_file_beside(&fx.notes()).exists());
}

#[test]
fn screen_rows_cut_trailing_blanks_and_skip_the_spacers_of_wide_glyphs() {
    use stdusk_core::terminal::CellSnap;
    let cell = |c| CellSnap {
        c,
        fg: egui::Color32::WHITE,
        bg: None,
        selected: false,
        wide: false,
        bold: false,
        dim: false,
    };
    let text = "ab \u{4e2d}\0 ";
    let snap = GridSnap {
        cols: 6,
        rows: 2,
        cells: text.chars().chain("cd    ".chars()).map(cell).collect(),
        cursor: None,
        top_line: 0,
    };
    assert_eq!(screen_rows(&snap), ["ab \u{4e2d}", "cd"]);
}

fn launch(host: &mut Host, id: &str, at: i64) {
    let signal = Signal::Started { tool_use_id: id.into(), command: "./import.sh".into() };
    host.background(1, signal, at);
}

#[test]
fn a_background_launch_with_no_line_or_report_is_due_once_after_ninety_seconds() {
    let fx = Fx::new();
    let mut host = Host::new(true, false);
    fx.until_loaded(&mut host, 1, NOW);
    launch(&mut host, "a", NOW);
    let late = NOW + 90_000;
    assert_eq!(host.background_due(1, true, late - 1), None);
    assert_eq!(host.background_silent(1, late - 1).len(), 0);
    let due = host.background_due(1, true, late).unwrap();
    assert_eq!(due.command, "./import.sh");
    assert_eq!(host.background_silent(1, late).len(), 1);
    assert_eq!(host.background_due(1, false, late), None, "not while the agent works");
    host.background_nudged(1, &due.tool_use_id);
    assert_eq!(host.background_due(1, true, late + 3_600_000), None);
}

#[test]
fn a_line_in_the_progress_file_after_the_launch_means_no_nudge() {
    let fx = Fx::new();
    let mut host = Host::new(true, false);
    fx.until_loaded(&mut host, 1, NOW);
    launch(&mut host, "a", NOW);
    fx.append("{\"name\":\"import\",\"done\":1,\"total\":9}\n");
    fx.tick(&mut host, &[1], NOW + 30_000);
    assert_eq!(host.background_due(1, true, NOW + 120_000), None);
    assert_eq!(host.background_silent(1, NOW + 120_000).len(), 0);
}

#[test]
fn a_bad_line_in_the_progress_file_is_not_progress() {
    let fx = Fx::new();
    let mut host = Host::new(true, false);
    fx.until_loaded(&mut host, 1, NOW);
    launch(&mut host, "a", NOW);
    fx.append("not json\n");
    fx.tick(&mut host, &[1], NOW + 30_000);
    assert!(host.background_due(1, true, NOW + 120_000).is_some());
}

#[test]
fn a_report_job_call_after_the_launch_means_no_nudge() {
    let fx = Fx::new();
    let mut host = Host::new(true, false);
    fx.until_loaded(&mut host, 1, NOW);
    launch(&mut host, "a", NOW);
    host.report(1, reading("import", 1.0, 9.0), NOW + 40_000);
    assert_eq!(host.background_due(1, true, NOW + 120_000), None);
}

#[test]
fn a_report_from_before_the_launch_does_not_count() {
    let fx = Fx::new();
    let mut host = Host::new(true, false);
    fx.until_loaded(&mut host, 1, NOW);
    host.report(1, reading("older", 1.0, 9.0), NOW - 1_000);
    launch(&mut host, "a", NOW);
    assert!(host.background_due(1, true, NOW + 120_000).is_some());
}

#[test]
fn a_task_that_ended_is_never_nudged() {
    let fx = Fx::new();
    let mut host = Host::new(true, false);
    fx.until_loaded(&mut host, 1, NOW);
    launch(&mut host, "a", NOW);
    host.background(1, Signal::Ended { tool_use_id: "a".into() }, NOW + 20_000);
    assert_eq!(host.background_due(1, true, NOW + 120_000), None);
    assert_eq!(host.background_silent(1, NOW + 120_000).len(), 0);
}

#[test]
fn a_launch_wakes_the_frame_when_it_turns_quiet() {
    let fx = Fx::new();
    let mut host = Host::new(true, false);
    fx.until_loaded(&mut host, 1, NOW);
    launch(&mut host, "a", NOW);
    assert_eq!(fx.tick(&mut host, &[1], NOW + 1_000), Some(NOW + 90_000));
}
