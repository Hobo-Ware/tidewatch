use std::sync::Mutex;
use std::time::{Duration, Instant};

use tidewatch_core::amphetamine_lid::read_marker;
use tidewatch_core::amphetamine_system::{Call, FakeSystem};

use super::*;

const WALL: f64 = 1_700_000_000.0;

#[derive(Default)]
struct Recorder {
    taken: Vec<String>,
    released: usize,
    held: bool,
    lose: bool,
}

impl Hold for Recorder {
    fn take(&mut self, reason: &str) -> Result<(), String> {
        self.taken.push(reason.to_owned());
        self.held = true;
        Ok(())
    }
    fn release(&mut self) {
        self.held = false;
        self.released += 1;
    }
    fn lost(&mut self) -> bool {
        self.lose
    }
}

struct Rig {
    home: tempfile::TempDir,
    ctx: egui::Context,
    host: Host<Recorder>,
    system: Arc<FakeSystem>,
    clock: Mutex<f64>,
}

impl Rig {
    fn with(system: FakeSystem, lid: bool) -> Self {
        let home = tempfile::tempdir().unwrap();
        let files = Files::of(&TidewatchPaths::new(home.path()));
        let system = Arc::new(system);
        let host = Host::new(system.clone(), Recorder::default(), Some(files), lid, true);
        Self { home, ctx: egui::Context::default(), host, system, clock: Mutex::new(WALL) }
    }

    fn new() -> Self {
        Self::with(FakeSystem::new(), true)
    }

    fn files(&self) -> Files {
        Files::of(&TidewatchPaths::new(self.home.path()))
    }

    fn wall(&self) -> f64 {
        *self.clock.lock().unwrap()
    }

    fn advance(&self, seconds: f64) {
        *self.clock.lock().unwrap() += seconds;
    }

    fn tick(&mut self) -> Option<f64> {
        let wall = self.wall();
        self.host.tick(wall, wall - WALL, &self.ctx)
    }

    fn until(&mut self, done: impl Fn(&Host<Recorder>) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done(&self.host) {
            assert!(Instant::now() < deadline, "state stuck at {:?}", self.host.state());
            self.tick();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn pick(&mut self, choice: Choice) {
        let wall = self.wall();
        self.host.pick(choice, wall, wall - WALL, &self.ctx);
    }

    fn turn_off(&mut self) {
        let wall = self.wall();
        self.host.turn_off(wall, wall - WALL, &self.ctx);
    }

    fn token(&self) -> Option<String> {
        std::fs::read_to_string(self.files().token).ok()
    }
}

fn is_on(host: &Host<Recorder>) -> bool {
    host.state().is_on()
}

#[test]
fn starting_writes_the_marker_and_token_first_then_asks_once_and_turns_on() {
    let mut rig = Rig::new();
    rig.pick(Choice::Hour);
    assert!(matches!(rig.host.state(), State::Starting { .. }));
    assert_eq!(rig.token(), Some(format!("{}", (WALL + 3600.0) as u64)));
    assert_eq!(read_marker(&rig.files().marker).unwrap().helper, None);
    rig.until(is_on);
    assert_eq!(rig.host.take_voices(), [Voice::Start]);
    assert_eq!(rig.host.hold.taken, [HOLD_REASON]);
    assert_eq!(read_marker(&rig.files().marker).unwrap().helper, Some(4242));
    let starts: Vec<_> =
        rig.system.calls().into_iter().filter(|c| matches!(c, Call::StartLid { .. })).collect();
    assert_eq!(starts.len(), 1);
    assert_eq!(rig.host.total(), Some(3600.0));
}

#[test]
fn a_refused_prompt_leaves_nothing_behind_and_says_so() {
    let mut rig = Rig::with(FakeSystem::refusing("the password prompt was cancelled"), true);
    rig.pick(Choice::HalfHour);
    rig.until(|host| host.state() == State::Off);
    assert_eq!(
        rig.host.take_voices(),
        [Voice::NotStarted("the password prompt was cancelled".to_owned())]
    );
    assert_eq!((rig.token(), read_marker(&rig.files().marker)), (None, None));
    assert_eq!(rig.host.hold.taken.len(), 0);
}

#[test]
fn turning_off_removes_the_token_waits_for_sleep_and_asks_for_no_second_password() {
    let mut rig = Rig::new();
    rig.pick(Choice::Open);
    rig.until(is_on);
    rig.host.take_voices();
    rig.turn_off();
    assert_eq!(rig.host.state(), State::Stopping);
    assert_eq!(rig.token(), None);
    rig.until(|host| host.state() == State::Off);
    assert_eq!(rig.host.take_voices(), [Voice::End]);
    assert!(!rig.host.hold.held);
    assert_eq!(read_marker(&rig.files().marker), None);
    assert!(!rig.system.calls().contains(&Call::RestoreLid));
    let starts = rig.system.calls().iter().filter(|c| matches!(c, Call::StartLid { .. })).count();
    assert_eq!(starts, 1);
}

#[test]
fn the_deadline_ends_the_session_by_itself_and_the_hour_is_told() {
    let mut rig = Rig::new();
    rig.pick(Choice::FourHours);
    rig.until(is_on);
    rig.host.take_voices();
    rig.advance(3600.0 + 1.0);
    assert!(rig.tick().is_some());
    assert_eq!(rig.host.take_voices(), [Voice::HoursLeft(3)]);
    rig.advance(3.0 * 3600.0);
    rig.tick();
    assert_eq!(rig.host.state(), State::Stopping);
    rig.until(|host| host.state() == State::Off);
    assert_eq!(rig.host.take_voices(), [Voice::End]);
}

#[test]
fn adding_an_hour_rewrites_the_token_and_the_line_total_without_another_prompt() {
    let mut rig = Rig::new();
    rig.pick(Choice::HalfHour);
    rig.until(is_on);
    let before = rig.token().unwrap().parse::<f64>().unwrap();
    let wall = rig.wall();
    rig.host.add_hour(wall, 1.0, &rig.ctx);
    assert_eq!(rig.token().unwrap().parse::<f64>().unwrap(), before + 3600.0);
    assert_eq!(rig.host.total(), Some(1800.0 + 3600.0));
    let starts = rig.system.calls().iter().filter(|c| matches!(c, Call::StartLid { .. })).count();
    assert_eq!(starts, 1);
}

#[test]
fn without_lid_mode_it_takes_the_hold_and_never_reaches_the_system() {
    let mut rig = Rig::with(FakeSystem::new(), false);
    rig.pick(Choice::Hour);
    assert!(is_on(&rig.host));
    assert_eq!(rig.host.hold.taken, [HOLD_REASON]);
    rig.turn_off();
    assert_eq!(rig.host.state(), State::Off);
    assert!(!rig.host.hold.held);
    assert_eq!(rig.system.calls(), []);
    assert_eq!(rig.token(), None);
}

#[test]
fn a_hold_the_os_ended_turns_the_session_off() {
    let mut rig = Rig::with(FakeSystem::new(), false);
    rig.pick(Choice::Open);
    rig.host.hold.lose = true;
    rig.tick();
    assert_eq!(rig.host.state(), State::Off);
    assert_eq!(rig.host.take_voices(), [Voice::Start, Voice::End]);
}

#[test]
fn closing_the_window_removes_the_token_so_the_helper_restores_sleep() {
    let mut rig = Rig::new();
    rig.pick(Choice::Open);
    rig.until(is_on);
    assert!(rig.token().is_some());
    rig.host.shutdown();
    assert_eq!(rig.token(), None);
    assert!(!rig.host.hold.held);
}

#[test]
fn the_palette_row_starts_for_an_hour_and_ends_a_running_session() {
    let mut rig = Rig::with(FakeSystem::new(), false);
    let wall = rig.wall();
    rig.host.toggle(wall, 0.0, &rig.ctx);
    assert_eq!(rig.host.state().left(wall), Some(3600.0));
    rig.host.toggle(wall, 1.0, &rig.ctx);
    assert_eq!(rig.host.state(), State::Off);
}

fn leave_marker(rig: &Rig, helper: Option<u32>) {
    write_marker(&rig.files().marker, &Marker { pid: 1, helper });
}

#[test]
fn a_crash_that_left_sleep_off_and_no_helper_needs_the_password_and_says_so() {
    let mut rig = Rig::with(FakeSystem::left_disabled(false), true);
    leave_marker(&rig, Some(55));
    rig.host.check_last_run(&rig.ctx);
    rig.until(|host| host.notice().is_some());
    rig.until(|host| host.notice().is_some_and(|n| n.kind == NoticeKind::Restored));
    assert!(rig.system.calls().contains(&Call::RestoreLid));
    assert_eq!(read_marker(&rig.files().marker), None);
    assert_eq!(rig.system.sleep_disabled(), Some(false));
}

#[test]
fn the_notice_names_the_prompt_before_it_comes() {
    let notice = notice_for(NoticeKind::AskingPassword, "");
    assert!(notice.body.contains("ask for your password"));
    assert_eq!(notice.title, "Sleep was left off");
    let done = notice_for(NoticeKind::Restored, "");
    assert_eq!(done.title, "Sleep is back to normal");
    assert!(done.body.contains("turned it back on"));
    assert!(notice_for(NoticeKind::Failed, "nope").body.contains("sudo pmset -a disablesleep 0"));
}

#[test]
fn a_helper_still_running_restores_sleep_on_its_own_with_no_prompt() {
    let mut rig = Rig::with(FakeSystem::left_disabled(true), true);
    leave_marker(&rig, Some(55));
    rig.host.check_last_run(&rig.ctx);
    rig.until(|host| host.notice().is_some_and(|n| n.kind == NoticeKind::Restoring));
    assert!(!rig.system.calls().contains(&Call::RestoreLid));
}

#[test]
fn a_marker_with_sleep_already_on_is_cleaned_up_without_a_word() {
    let mut rig = Rig::with(FakeSystem::new(), true);
    leave_marker(&rig, Some(55));
    rig.host.check_last_run(&rig.ctx);
    let marker = rig.files().marker;
    rig.until(|_| read_marker(&marker).is_none());
    assert_eq!(rig.host.notice(), None);
    assert!(!rig.system.calls().contains(&Call::RestoreLid));
}

#[test]
fn sleep_turned_off_by_hand_is_left_alone() {
    let mut rig = Rig::with(FakeSystem::left_disabled(false), true);
    rig.host.check_last_run(&rig.ctx);
    std::thread::sleep(Duration::from_millis(150));
    rig.tick();
    assert_eq!(rig.host.notice(), None);
    assert!(!rig.system.calls().contains(&Call::RestoreLid));
    assert_eq!(rig.system.sleep_disabled(), Some(true));
}

#[test]
fn the_launch_check_is_skipped_where_there_is_no_lid_mode_or_the_run_is_in_the_background() {
    let system = Arc::new(FakeSystem::left_disabled(false));
    let files = Files::of(&TidewatchPaths::new(tempfile::tempdir().unwrap().path()));
    let ctx = egui::Context::default();
    for (lid, allowed) in [(false, true), (true, false)] {
        let host =
            Host::new(system.clone(), Recorder::default(), Some(files.clone()), lid, allowed);
        host.check_last_run(&ctx);
    }
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(system.calls(), []);
}

#[test]
fn a_not_allowed_host_ignores_every_event() {
    let system = Arc::new(FakeSystem::new());
    let mut host = Host::new(system.clone(), Recorder::default(), None, true, false);
    let ctx = egui::Context::default();
    host.pick(Choice::Hour, WALL, 0.0, &ctx);
    assert_eq!(host.state(), State::Off);
    assert_eq!(system.calls(), []);
}

#[test]
fn the_tablet_follows_the_session() {
    let mut rig = Rig::new();
    assert_eq!(rig.host.look(5.0), Look::Off);
    rig.pick(Choice::Open);
    assert_eq!(rig.host.look(5.0), Look::Pending);
    rig.until(is_on);
    let at = rig.host.changed_at;
    assert!(matches!(rig.host.look(at + 0.25), Look::On { age } if (age - 0.25).abs() < 1e-9));
    rig.turn_off();
    let stopped = rig.host.changed_at;
    assert!(
        matches!(rig.host.look(stopped + 0.1), Look::Stopping { age } if (age - 0.1).abs() < 1e-9)
    );
    assert_eq!(rig.host.look(stopped + 5.0), Look::Off);
}

#[test]
fn each_dev_switch_value_shows_its_own_state_and_none_reaches_the_machine() {
    let cases = [
        ("off", Look::Off, false),
        ("on", Look::On { age: PINNED_ON_AGE }, false),
        ("activating", Look::On { age: PINNED_ACTIVATING_AGE }, false),
        ("short", Look::On { age: PINNED_ON_AGE }, true),
        ("forever", Look::On { age: PINNED_ON_AGE }, false),
    ];
    for (value, look, short) in cases {
        let dev = Dev::parse(Some(value)).unwrap();
        let system = Arc::new(FakeSystem::new());
        let host = Host::dev(&dev, system.clone(), Recorder::default(), true, WALL);
        assert_eq!((host.look(0.0), host.short()), (look, short), "{value}");
        assert_eq!(system.calls(), []);
    }
    assert_eq!(Dev::parse(Some("nonsense")), None);
    assert_eq!(Dev::parse(None), None);
    let forever =
        Host::dev(&Dev::Forever, Arc::new(FakeSystem::new()), Recorder::default(), true, WALL);
    assert_eq!(forever.state(), State::On { until: None, told: 0 });
    let on = Host::dev(&Dev::On, Arc::new(FakeSystem::new()), Recorder::default(), true, WALL);
    assert_eq!(on.state().left(WALL), Some(41.5 * 60.0));
}
