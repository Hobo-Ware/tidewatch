use std::sync::Mutex;
use std::time::{Duration, Instant};

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use tidewatch_core::amphetamine_helper::{Auth, Serve};
use tidewatch_core::amphetamine_lid::read_marker;
use tidewatch_core::amphetamine_system::{Call, FakeSystem, Route};

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
fn a_prompt_is_open_from_the_click_until_the_start_answers_either_way() {
    let mut rig =
        Rig::with(FakeSystem::with_biometrics().holding(Duration::from_millis(150)), true);
    assert!(!rig.host.prompt_open());
    rig.pick(Choice::Hour);
    assert!(rig.host.prompt_open());
    rig.until(is_on);
    assert!(!rig.host.prompt_open());

    let mut refused = Rig::with(FakeSystem::refusing("the password prompt was cancelled"), true);
    refused.pick(Choice::Hour);
    assert!(refused.host.prompt_open());
    refused.until(|host| host.state() == State::Off);
    assert!(!refused.host.prompt_open());
}

#[test]
fn the_touch_id_setup_and_removal_hold_a_prompt_open_while_they_run() {
    let (mut rig, _, _) = gated_rig();
    assert!(!rig.host.prompt_open());
    rig.host.change_touch_id(Click::Remove, &rig.ctx);
    assert!(rig.host.prompt_open());
    rig.until(|host| host.touch_id() == Some(TouchId::Missing));
    assert!(!rig.host.prompt_open());
}

#[test]
fn a_background_host_never_has_a_prompt_open() {
    let system = Arc::new(FakeSystem::new());
    let mut host = Host::new(system, Recorder::default(), None, true, false);
    let ctx = egui::Context::default();
    host.pick(Choice::Hour, WALL, 0.0, &ctx);
    host.change_touch_id(Click::SetUp, &ctx);
    assert!(!host.prompt_open());
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
    let marker = rig.files().marker;
    rig.until(|_| read_marker(&marker).is_none());
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

fn repaints(ctx: &egui::Context) -> Arc<std::sync::atomic::AtomicUsize> {
    let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = Arc::clone(&asked);
    ctx.set_request_repaint_callback(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    asked
}

#[test]
fn a_click_shows_the_pending_look_and_asks_for_a_frame_before_anything_else_finishes() {
    let mut rig = Rig::with(FakeSystem::with_biometrics().holding(Duration::from_secs(2)), true);
    let asked = repaints(&rig.ctx);
    rig.pick(Choice::Hour);
    assert!(matches!(rig.host.state(), State::Starting { .. }));
    assert_eq!(rig.host.look(0.0), Look::Pending);
    assert!(asked.load(Ordering::SeqCst) >= 1, "nothing asked for the frame that shows it");
}

#[test]
fn the_start_prompt_does_not_wait_behind_other_slow_jobs() {
    let (mut rig, gate, inner) = gated_rig_over(FakeSystem::new());
    for _ in 0..16 {
        rig.host.check_last_run(&rig.ctx);
    }
    std::thread::sleep(Duration::from_millis(50));
    let asked_at = Instant::now();
    rig.pick(Choice::Hour);
    let deadline = asked_at + Duration::from_secs(5);
    while password_starts(&inner) == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    let waited = asked_at.elapsed();
    assert_eq!(password_starts(&inner), 1, "the prompt never ran");
    assert!(!gate.load(Ordering::SeqCst), "the other jobs were meant to still be blocked");
    assert!(waited < Duration::from_secs(2), "the prompt waited {waited:?} behind other jobs");
    gate.store(true, Ordering::SeqCst);
}

fn setups(system: &FakeSystem) -> usize {
    system.calls().iter().filter(|c| matches!(c, Call::InstallAndStart { .. })).count()
}

#[test]
fn the_first_start_on_a_mac_with_a_reader_installs_and_starts_in_one_prompt() {
    let mut rig = Rig::with(FakeSystem::with_biometrics(), true);
    rig.host.check_last_run(&rig.ctx);
    rig.until(|host| host.route() == Route::SetUp);
    rig.pick(Choice::Hour);
    rig.until(is_on);
    assert_eq!(rig.host.take_voices(), [Voice::Start]);
    assert_eq!(
        (setups(&rig.system), password_starts(&rig.system), sudo_starts(&rig.system)),
        (1, 0, 0)
    );
    assert_eq!(read_marker(&rig.files().marker).unwrap().helper, Some(4242));
    assert_eq!(rig.host.touch_id(), Some(TouchId::Installed));
    rig.until(|host| host.route() == Route::Fingerprint);
    rig.turn_off();
    rig.until(|host| host.state() == State::Off);
    rig.pick(Choice::Hour);
    rig.until(is_on);
    assert_eq!(
        (setups(&rig.system), password_starts(&rig.system), sudo_starts(&rig.system)),
        (1, 0, 0)
    );
    assert_eq!(authentications(&rig.system), 0);
}

#[test]
fn a_cancelled_first_prompt_installs_nothing_and_asks_nothing_else() {
    let fake = FakeSystem::refusing("the password prompt was cancelled").with_reader();
    let mut rig = Rig::with(fake, true);
    rig.pick(Choice::Hour);
    rig.until(|host| host.state() == State::Off);
    assert_eq!(
        rig.host.take_voices(),
        [Voice::NotStarted("the password prompt was cancelled".to_owned())]
    );
    assert_eq!((rig.token(), read_marker(&rig.files().marker)), (None, None));
    assert_eq!(rig.host.touch_id(), Some(TouchId::Missing));
    assert_eq!(
        rig.system.calls(),
        [
            Call::MakeFifo(rig.files().serve.clone()),
            Call::InstallAndStart {
                token: rig.files().token,
                owner: std::process::id(),
                fifo: rig.files().serve,
            }
        ]
    );
}

#[test]
fn a_mac_without_a_reader_keeps_asking_for_the_password_and_never_installs() {
    let mut rig = Rig::with(FakeSystem::new(), true);
    rig.host.check_last_run(&rig.ctx);
    rig.pick(Choice::Hour);
    rig.until(is_on);
    assert_eq!(rig.host.route(), Route::Password);
    assert_eq!((setups(&rig.system), password_starts(&rig.system)), (0, 1));
    assert_eq!(rig.host.touch_id(), Some(TouchId::Missing));
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
    let setup = Dev::parse(Some("setup")).unwrap();
    let system = Arc::new(FakeSystem::with_biometrics());
    let host = Host::dev(&setup, system.clone(), Recorder::default(), true, WALL);
    assert_eq!((host.look(0.0), host.route(), system.calls()), (Look::Off, Route::SetUp, vec![]));
    assert_eq!(Dev::parse(Some("pending")), Some(Dev::Pending));
    assert_eq!(Dev::parse(Some("nonsense")), None);
    assert_eq!(Dev::parse(None), None);
    let forever =
        Host::dev(&Dev::Forever, Arc::new(FakeSystem::new()), Recorder::default(), true, WALL);
    assert_eq!(forever.state(), State::On { until: None, told: 0 });
    let on = Host::dev(&Dev::On, Arc::new(FakeSystem::new()), Recorder::default(), true, WALL);
    assert_eq!(on.state().left(WALL), Some(41.5 * 60.0));
}

fn sudo_starts(system: &FakeSystem) -> usize {
    system.calls().iter().filter(|c| matches!(c, Call::SudoServe { .. })).count()
}

fn password_starts(system: &FakeSystem) -> usize {
    system.calls().iter().filter(|c| matches!(c, Call::StartLid { .. })).count()
}

#[test]
fn with_the_helper_installed_a_fingerprint_starts_it_and_the_password_is_never_asked() {
    let mut rig = Rig::with(FakeSystem::with_touch_id(), true);
    rig.pick(Choice::Hour);
    rig.until(is_on);
    assert_eq!(rig.host.take_voices(), [Voice::Start]);
    assert_eq!(read_marker(&rig.files().marker).unwrap().helper, Some(4242));
    assert_eq!((sudo_starts(&rig.system), password_starts(&rig.system)), (1, 0));
    let asked = rig.system.calls().iter().filter(|c| **c == Call::Authenticate).count();
    assert_eq!(asked, 1);
}

#[test]
fn a_cancelled_fingerprint_leaves_it_off_and_asks_nothing_else() {
    let fake = FakeSystem::with_touch_id().answering(Auth::Denied("Touch ID was cancelled".into()));
    let mut rig = Rig::with(fake, true);
    rig.pick(Choice::Hour);
    rig.until(|host| host.state() == State::Off);
    assert_eq!(rig.host.take_voices(), [Voice::NotStarted("Touch ID was cancelled".to_owned())]);
    assert_eq!((rig.token(), read_marker(&rig.files().marker)), (None, None));
    assert_eq!((sudo_starts(&rig.system), password_starts(&rig.system)), (0, 0));
}

#[test]
fn a_broken_sudo_rule_falls_back_to_the_password_prompt_and_still_turns_on() {
    let fake = FakeSystem::with_touch_id().sudo_refusing("a password is required");
    let mut rig = Rig::with(fake, true);
    rig.pick(Choice::Hour);
    rig.until(is_on);
    assert_eq!((sudo_starts(&rig.system), password_starts(&rig.system)), (1, 1));
}

#[test]
fn turning_off_with_the_helper_installed_asks_it_to_restore_sleep_at_once() {
    let mut rig = Rig::with(FakeSystem::with_touch_id(), true);
    rig.pick(Choice::Open);
    rig.until(is_on);
    rig.turn_off();
    rig.until(|host| host.state() == State::Off);
    assert!(
        rig.system.calls().iter().any(|c| matches!(c, Call::SendServe { command: Serve::Off, .. }))
    );
    assert!(!rig.system.calls().contains(&Call::SudoStop));
    assert!(!rig.system.calls().contains(&Call::RestoreLid));
}

fn authentications(system: &FakeSystem) -> usize {
    system.calls().iter().filter(|c| **c == Call::Authenticate).count()
}

#[test]
fn a_second_activation_in_the_same_run_asks_nothing_and_sends_the_helper_a_command() {
    let mut rig = Rig::with(FakeSystem::with_touch_id(), true);
    rig.pick(Choice::Hour);
    rig.until(is_on);
    rig.turn_off();
    rig.until(|host| host.state() == State::Off);
    rig.pick(Choice::Hour);
    rig.until(is_on);
    assert_eq!((authentications(&rig.system), sudo_starts(&rig.system)), (1, 1));
    assert!(
        rig.system
            .calls()
            .iter()
            .any(|c| matches!(c, Call::SendServe { command: Serve::On(_), .. }))
    );
}

#[test]
fn a_helper_gone_before_the_next_activation_asks_for_the_fingerprint_again() {
    let mut rig = Rig::with(FakeSystem::with_touch_id(), true);
    rig.pick(Choice::Hour);
    rig.until(is_on);
    rig.system.owner_exits();
    rig.turn_off();
    rig.until(|host| host.state() == State::Off);
    rig.pick(Choice::Hour);
    rig.until(is_on);
    assert_eq!((authentications(&rig.system), sudo_starts(&rig.system)), (2, 2));
}

#[test]
fn an_outdated_helper_is_replaced_by_the_first_start_with_no_notice() {
    let mut rig = Rig::with(FakeSystem::with_outdated_helper(), true);
    rig.host.check_last_run(&rig.ctx);
    rig.until(|host| host.route() == Route::Outdated);
    assert_eq!(rig.host.touch_id(), Some(TouchId::Outdated));
    rig.pick(Choice::Hour);
    rig.until(is_on);
    rig.until(|host| host.route() == Route::Fingerprint);
    assert_eq!(rig.host.notice(), None);
    assert_eq!((setups(&rig.system), password_starts(&rig.system)), (1, 0));
}

struct Gated {
    inner: Arc<FakeSystem>,
    open: Arc<AtomicBool>,
}

impl System for Gated {
    fn start_lid(&self, token: &Path, owner: u32) -> Result<u32, String> {
        self.inner.start_lid(token, owner)
    }
    fn restore_lid(&self) -> Result<(), String> {
        self.inner.restore_lid()
    }
    fn sleep_disabled(&self) -> Option<bool> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.open.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        self.inner.sleep_disabled()
    }
    fn alive(&self, pid: u32) -> bool {
        self.inner.alive(pid)
    }
    fn touch_id_installed(&self) -> bool {
        self.inner.touch_id_installed()
    }
    fn install_and_start_lid(&self, token: &Path, owner: u32, fifo: &Path) -> Result<u32, String> {
        self.inner.install_and_start_lid(token, owner, fifo)
    }
    fn touch_id_current(&self) -> bool {
        self.inner.touch_id_current()
    }
    fn make_fifo(&self, fifo: &Path) -> Result<(), String> {
        self.inner.make_fifo(fifo)
    }
    fn send_serve(&self, token: &Path, fifo: &Path, command: Serve) -> Result<(), String> {
        self.inner.send_serve(token, fifo, command)
    }
    fn biometrics_available(&self) -> bool {
        self.inner.biometrics_available()
    }
    fn install_touch_id(&self) -> Result<(), String> {
        self.inner.install_touch_id()
    }
    fn remove_touch_id(&self) -> Result<(), String> {
        self.inner.remove_touch_id()
    }
    fn authenticate(&self, reason: &str) -> Auth {
        self.inner.authenticate(reason)
    }
    fn sudo_serve(&self, token: &Path, owner: u32, fifo: &Path) -> Result<u32, String> {
        self.inner.sudo_serve(token, owner, fifo)
    }
    fn sudo_stop(&self) -> Result<(), String> {
        self.inner.sudo_stop()
    }
}

fn gated_rig() -> (Rig, Arc<AtomicBool>, Arc<FakeSystem>) {
    gated_rig_over(FakeSystem::with_touch_id())
}

fn gated_rig_over(inner: FakeSystem) -> (Rig, Arc<AtomicBool>, Arc<FakeSystem>) {
    let inner = Arc::new(inner);
    let open = Arc::new(AtomicBool::new(false));
    let home = tempfile::tempdir().unwrap();
    let files = Files::of(&TidewatchPaths::new(home.path()));
    let system: Arc<dyn System> = Arc::new(Gated { inner: inner.clone(), open: open.clone() });
    let host = Host::new(system, Recorder::default(), Some(files), true, true);
    let rig = Rig {
        home,
        ctx: egui::Context::default(),
        host,
        system: Arc::new(FakeSystem::new()),
        clock: Mutex::new(WALL),
    };
    (rig, open, inner)
}

#[test]
fn the_button_is_on_as_soon_as_the_start_returns_without_waiting_for_pmset() {
    let (mut rig, open, _) = gated_rig();
    rig.pick(Choice::Hour);
    rig.until(is_on);
    assert_eq!(rig.host.take_voices(), [Voice::Start]);
    assert!(rig.host.hold.held);
    assert!(matches!(rig.host.look(1.0), Look::On { .. }));
    open.store(true, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(100));
    rig.tick();
    assert!(is_on(&rig.host), "a confirmed start changes nothing");
    assert_eq!(rig.host.take_voices(), []);
}

#[test]
fn the_button_is_off_as_soon_as_the_stop_returns_and_the_check_runs_after() {
    let (mut rig, open, inner) = gated_rig();
    open.store(true, Ordering::SeqCst);
    rig.pick(Choice::Open);
    rig.until(is_on);
    rig.host.take_voices();
    open.store(false, Ordering::SeqCst);
    rig.turn_off();
    rig.until(|host| host.state() == State::Off);
    assert_eq!(rig.host.take_voices(), [Voice::End]);
    assert!(inner.calls().iter().any(|c| matches!(c, Call::SendServe { command: Serve::Off, .. })));
    assert!(
        read_marker(&rig.files().marker).is_some(),
        "the marker stays until sleep is confirmed"
    );
    open.store(true, Ordering::SeqCst);
    let marker = rig.files().marker;
    rig.until(|_| read_marker(&marker).is_none());
    assert_eq!(rig.host.take_voices(), []);
}

#[test]
fn the_tablet_plays_its_whole_close_even_though_the_state_is_already_off() {
    let (mut rig, open, _) = gated_rig();
    open.store(true, Ordering::SeqCst);
    rig.pick(Choice::Open);
    rig.until(is_on);
    rig.turn_off();
    rig.until(|host| host.state() == State::Off);
    let at = rig.host.fading_since.unwrap();
    assert!(matches!(rig.host.look(at + 0.1), Look::Stopping { age } if (age - 0.1).abs() < 1e-9));
    assert_eq!(rig.host.look(at + 5.0), Look::Off);
}

#[test]
fn a_start_pmset_never_shows_flips_back_with_a_toast_and_lets_the_helper_go() {
    let mut rig = Rig::with(FakeSystem::new().sleep_stays_on(), true);
    rig.pick(Choice::Hour);
    rig.until(is_on);
    assert_eq!(rig.host.take_voices(), [Voice::Start]);
    rig.until(|host| host.state() == State::Off);
    assert_eq!(
        rig.host.take_voices(),
        [Voice::NotStarted("macOS did not turn sleep off".to_owned())]
    );
    assert!(!rig.host.hold.held);
    assert_eq!((rig.token(), read_marker(&rig.files().marker)), (None, None));
}

#[test]
fn a_stop_pmset_never_confirms_says_sleep_may_still_be_off() {
    let mut rig = Rig::with(FakeSystem::new().helper_ignoring_the_token(), true);
    rig.pick(Choice::Open);
    rig.until(is_on);
    rig.host.take_voices();
    rig.turn_off();
    rig.until(|host| host.state() == State::Off);
    let mut said = rig.host.take_voices();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !said.contains(&Voice::StillOff) && Instant::now() < deadline {
        rig.tick();
        std::thread::sleep(Duration::from_millis(5));
        said.extend(rig.host.take_voices());
    }
    assert_eq!(said, [Voice::End, Voice::StillOff]);
    assert!(read_marker(&rig.files().marker).is_some());
}

#[test]
fn a_check_that_belongs_to_an_earlier_session_is_ignored() {
    let mut rig = Rig::new();
    rig.pick(Choice::Open);
    rig.until(is_on);
    let stale = rig.host.epoch - 1;
    let wall = rig.wall();
    for done in [
        Done::Confirmed { epoch: stale, on: false, ok: true },
        Done::Confirmed { epoch: stale, on: true, ok: false },
        Done::Stopped(stale),
    ] {
        rig.host.apply_done(done, wall, 1.0, &rig.ctx);
    }
    assert!(is_on(&rig.host));
    assert!(read_marker(&rig.files().marker).is_some());
    assert_eq!(rig.host.take_voices(), [Voice::Start]);
}

#[test]
fn setting_up_touch_id_shows_working_then_installed_and_remove_goes_back() {
    let mut rig = Rig::new();
    assert_eq!(rig.host.touch_id(), Some(TouchId::Missing));
    rig.host.change_touch_id(Click::SetUp, &rig.ctx);
    assert_eq!(rig.host.touch_id(), Some(TouchId::Working));
    rig.until(|host| host.touch_id() == Some(TouchId::Installed));
    assert_eq!(rig.host.take_voices(), [Voice::TouchIdReady]);
    rig.host.change_touch_id(Click::Remove, &rig.ctx);
    rig.until(|host| host.touch_id() == Some(TouchId::Missing));
    assert_eq!(rig.host.take_voices(), [Voice::TouchIdGone]);
    assert_eq!(rig.system.calls(), [Call::InstallTouchId, Call::RemoveTouchId]);
}

#[test]
fn a_cancelled_setup_prompt_changes_nothing_and_says_why() {
    let mut rig = Rig::with(FakeSystem::refusing("the password prompt was cancelled"), true);
    rig.host.change_touch_id(Click::SetUp, &rig.ctx);
    rig.until(|host| host.touch_id() == Some(TouchId::Missing) && host.touch_id_job.is_none());
    assert_eq!(
        rig.host.take_voices(),
        [Voice::TouchIdFailed("the password prompt was cancelled".to_owned())]
    );
}

#[test]
fn a_second_click_while_one_runs_is_ignored() {
    let (mut rig, _, inner) = gated_rig();
    rig.host.change_touch_id(Click::Remove, &rig.ctx);
    rig.host.change_touch_id(Click::Remove, &rig.ctx);
    rig.until(|host| host.touch_id() == Some(TouchId::Missing));
    let removed = inner.calls().iter().filter(|c| **c == Call::RemoveTouchId).count();
    assert_eq!(removed, 1);
}

#[test]
fn there_is_no_touch_id_row_without_lid_mode_and_a_background_run_cannot_change_it() {
    let rig = Rig::with(FakeSystem::new(), false);
    assert_eq!(rig.host.touch_id(), None);
    let system = Arc::new(FakeSystem::new());
    let mut host = Host::new(system.clone(), Recorder::default(), None, true, false);
    host.change_touch_id(Click::SetUp, &egui::Context::default());
    assert_eq!((host.touch_id(), system.calls()), (Some(TouchId::Missing), vec![]));
}

#[test]
fn the_touchid_dev_switch_shows_the_row_installed_and_the_start_switch_runs_a_whole_session() {
    let ctx = egui::Context::default();
    let dev = Dev::parse(Some("touchid")).unwrap();
    let system = Arc::new(FakeSystem::with_touch_id());
    let host = Host::dev(&dev, system.clone(), Recorder::default(), true, WALL);
    assert_eq!((host.touch_id(), host.state()), (Some(TouchId::Installed), State::Off));
    assert_eq!(system.calls(), []);

    let dev = Dev::parse(Some("start")).unwrap();
    let system = Arc::new(FakeSystem::with_touch_id());
    let mut host = Host::dev(&dev, system.clone(), Recorder::default(), true, WALL);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !host.state().is_on() && Instant::now() < deadline {
        host.tick(WALL, 0.0, &ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(host.state().is_on());
    assert_eq!(sudo_starts(&system), 1);
}
