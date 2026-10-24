//! Self-update: Kelp's shared updater pointed at Tidewatch's repo and cask.

use std::path::{Path, PathBuf};

use kelp_core::update::Release;
use kelp_ui::update::{Status, UpdateConfig, Updater};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prefs::{Prefs, UpdatePrefs};
use tidewatch_core::update::{self, APP_NAME, Mode};

use crate::fix_dialog;
use crate::update_chip;

pub(crate) const NOTIFY_ONLY_HINT: &str =
    "This copy has no installer: Tidewatch tells you when a release is out and opens its page.";
pub(crate) const INSTALL_HINT: &str = "With Homebrew. The new version starts when you restart.";

const FAKE_APP: &str = "/Applications/Tidewatch.app";
const FAKE_STUCK: &str = "/opt/homebrew/Caskroom/tidewatch/0.1.10.upgrading";
const FAKE_FAILURE: &str = "brew upgrade failed: Error: It seems there is already an App at '/opt/homebrew/Caskroom/tidewatch/0.1.10.upgrading/Tidewatch.app'";

/// `TIDEWATCH_FAKE_UPDATE_STATE`: a failed or a stuck update to look at; `confirm` also opens the Fix confirm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FakeState {
    Failed,
    Stuck,
    Confirm,
}

pub(crate) fn fake_state(value: Option<&str>) -> Option<FakeState> {
    match value?.trim() {
        "failed" => Some(FakeState::Failed),
        "stuck" => Some(FakeState::Stuck),
        "confirm" => Some(FakeState::Confirm),
        _ => None,
    }
}

/// What the host has to do after a click on the chip or the Fix confirm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Wish {
    OpenSettings,
    Quit,
    Toast(String),
}

pub(crate) struct Updates {
    updater: Updater,
    prefs: UpdatePrefs,
    quiet: bool,
    restart: bool,
    fake_state: Option<FakeState>,
    fix_dialog: bool,
    wish: Option<Wish>,
}

impl Updates {
    /// `fake` is `TIDEWATCH_FAKE_UPDATE`; `screenshot` is a screenshot run. Either one, and every test
    /// build, keeps the updater off the network and off the installer, Check now included.
    pub(crate) fn new(
        ctx: &egui::Context,
        prefs: UpdatePrefs,
        fake: Option<Release>,
        screenshot: bool,
        state_dir: PathBuf,
    ) -> Self {
        let config = UpdateConfig {
            source: update::source(),
            app_name: APP_NAME.to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            state_dir,
        };
        let mut updater = Updater::new(ctx.clone(), config);
        let quiet = offline(screenshot, fake.is_some(), cfg!(test));
        if let Some(release) = fake {
            updater.status = Status::Available(release);
        }
        Self {
            updater,
            prefs,
            quiet,
            restart: false,
            fake_state: None,
            fix_dialog: false,
            wish: None,
        }
    }

    /// Shows a failed or a stuck update without one; like a fake release it keeps the run offline.
    pub(crate) fn with_fake_state(mut self, state: Option<FakeState>) -> Self {
        let Some(state) = state else { return self };
        self.quiet = true;
        self.fake_state = Some(state);
        self.updater.status = match state {
            FakeState::Failed => Status::Failed(FAKE_FAILURE.to_owned()),
            FakeState::Stuck | FakeState::Confirm => Status::Stuck(PathBuf::from(FAKE_STUCK)),
        };
        self.fix_dialog = state == FakeState::Confirm;
        self
    }

    /// Once per frame: collects a finished check and starts the next one when it is due.
    pub(crate) fn tick(&mut self) {
        let (check, install) = background_work(&self.prefs, self.quiet);
        self.updater.tick(check, install);
    }

    /// The update chip. Once the update is installed a click on it only sets a flag, because the
    /// updater's own Restart cannot hand the terminals over; it is taken with [`Self::take_restart`].
    pub(crate) fn pill(&mut self, ui: &mut egui::Ui) {
        let Some(spec) = self.chip() else { return };
        if !update_chip::show(ui, &spec).clicked() {
            return;
        }
        match (spec.click, &self.updater.status) {
            (update_chip::Click::Restart, _) if self.updater.can_restart() => self.restart = true,
            (update_chip::Click::Restart, _) => {
                self.wish = Some(Wish::Toast(update_chip::RESTART_WAITS.to_owned()));
            }
            (update_chip::Click::Fix, _) => self.fix_dialog = true,
            (update_chip::Click::OpenSettings, _) => self.wish = Some(Wish::OpenSettings),
            (update_chip::Click::Install, Status::Available(release)) => {
                self.updater.install(release.clone());
            }
            (update_chip::Click::Open, Status::Available(release)) => {
                ui.ctx().open_url(egui::OpenUrl::new_tab(&release.url));
            }
            _ => {}
        }
    }

    /// How wide the chip is, 0 when there is none.
    pub(crate) fn pill_width(&self, ui: &egui::Ui) -> f32 {
        self.chip().map_or(0.0, |spec| update_chip::width(ui, &spec))
    }

    fn chip(&self) -> Option<update_chip::Spec> {
        if !self.updater.is_active() {
            return None;
        }
        update_chip::spec(&self.updater.status, self.updater.brew_install())
    }

    /// Draws the Fix confirm while it is open and acts on the answer.
    pub(crate) fn show_fix_dialog(&mut self, ctx: &egui::Context) {
        if !self.fix_dialog {
            return;
        }
        match fix_dialog::show(ctx) {
            Some(fix_dialog::Answer::Cancel) => self.fix_dialog = false,
            Some(fix_dialog::Answer::QuitAndFix) => {
                self.fix_dialog = false;
                self.start_fix();
            }
            None => {}
        }
    }

    fn start_fix(&mut self) {
        if self.fake_state.is_some() {
            self.wish = Some(Wish::Toast(format!(
                "Fake update state, nothing was started. Fix would run:\n{}",
                Self::repair_script()
            )));
            return;
        }
        self.updater.fix();
        if self.updater.take_quit_request() {
            self.wish = Some(Wish::Quit);
        }
    }

    /// The script Fix runs once the app is gone.
    pub(crate) fn repair_script() -> String {
        let bundle = kelp_core::update::app_bundle().unwrap_or_else(|| Path::new(FAKE_APP).into());
        kelp_core::update::repair_command(&update::source(), &bundle, std::process::id())
    }

    pub(crate) fn dialog_open(&self) -> bool {
        self.fix_dialog
    }

    pub(crate) fn is_stuck(&self) -> bool {
        matches!(self.updater.status, Status::Stuck(_))
    }

    pub(crate) fn can_restart(&self) -> bool {
        self.updater.can_restart()
    }

    pub(crate) fn take_wish(&mut self) -> Option<Wish> {
        self.wish.take()
    }

    /// Whether Restart was clicked since the last call.
    pub(crate) fn take_restart(&mut self) -> bool {
        std::mem::take(&mut self.restart)
    }

    pub(crate) fn check_now(&mut self) {
        if !self.quiet {
            self.updater.check_now();
        }
    }

    pub(crate) fn mode(&self) -> Mode {
        Mode::pick(cfg!(target_os = "macos"), self.updater.brew_install())
    }

    pub(crate) fn prefs(&self) -> &UpdatePrefs {
        &self.prefs
    }

    pub(crate) fn status_line(&self) -> String {
        status_line(&self.updater.status, self.updater.current())
    }

    pub(crate) fn short_status(&self) -> String {
        short_status(&self.updater.status)
    }

    /// The new settings, which are the whole table with only this switch changed.
    /// The settings file changed under the app (a sync): take its switches without writing.
    pub(crate) fn reload(&mut self, prefs: UpdatePrefs) {
        self.prefs = prefs;
    }

    pub(crate) fn set_check(&mut self, on: bool) -> UpdatePrefs {
        self.prefs.check = on;
        self.prefs.clone()
    }

    pub(crate) fn set_install(&mut self, on: bool) -> UpdatePrefs {
        self.prefs.install = on;
        self.prefs.clone()
    }
}

fn offline(screenshot: bool, fake: bool, test_build: bool) -> bool {
    screenshot || fake || test_build
}

/// Whether the background check runs, and whether it may install what it finds.
fn background_work(prefs: &UpdatePrefs, quiet: bool) -> (bool, bool) {
    (prefs.check && !quiet, prefs.install && !quiet)
}

fn status_line(status: &Status, current: &str) -> String {
    match status {
        Status::Idle => format!("{APP_NAME} {current}"),
        Status::Checking => "Checking for updates...".to_owned(),
        Status::UpToDate => format!("{APP_NAME} {current} is the latest version"),
        Status::Available(release) => {
            format!("{APP_NAME} {} is out (you have {current})", release.version)
        }
        Status::Installing(release) => format!("Installing {APP_NAME} {}...", release.version),
        Status::Ready(release) => {
            format!("{APP_NAME} {} is installed. Restart to use it.", release.version)
        }
        Status::Failed(reason) => format!("Update failed: {reason}"),
        Status::Stuck(path) => {
            format!("Update stuck: an earlier Homebrew upgrade left {}", path.display())
        }
    }
}

fn short_status(status: &Status) -> String {
    match status {
        Status::Idle => "Not checked yet".to_owned(),
        Status::Checking => "Checking...".to_owned(),
        Status::UpToDate => "Up to date".to_owned(),
        Status::Available(release) => format!("{} available", release.version),
        Status::Installing(_) => "Installing...".to_owned(),
        Status::Ready(_) => "Restart to update".to_owned(),
        Status::Failed(_) => "Update failed".to_owned(),
        Status::Stuck(_) => "Update stuck".to_owned(),
    }
}

/// Writes the update settings off the UI thread, leaving the rest of the file as it is on disk; a
/// failure goes to stderr.
pub(crate) fn save(paths: TidewatchPaths, prefs: UpdatePrefs) {
    std::thread::spawn(move || {
        if let Err(e) = Prefs::update(&paths, |p| p.updates = prefs) {
            eprintln!("tidewatch: cannot save settings.toml: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(version: &str) -> Release {
        Release { version: version.to_owned(), url: format!("https://example.test/{version}") }
    }

    fn updates(fake: Option<Release>, screenshot: bool) -> Updates {
        Updates::new(
            &egui::Context::default(),
            UpdatePrefs::default(),
            fake,
            screenshot,
            std::env::temp_dir(),
        )
    }

    #[test]
    fn a_fake_update_is_available_at_once_and_counts_as_news() {
        let updates = updates(Some(release("9.9.9")), false);
        assert_eq!(updates.updater.status, Status::Available(release("9.9.9")));
        assert!(updates.updater.has_news());
    }

    #[test]
    fn without_a_fake_nothing_is_known_until_a_check_ends() {
        let updates = updates(None, false);
        assert_eq!(updates.updater.status, Status::Idle);
        assert!(!updates.updater.has_news());
        assert_eq!(updates.updater.current(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn the_updater_is_never_switched_off_so_linux_still_hears_about_a_release() {
        assert!(updates(None, false).updater.is_active());
    }

    #[test]
    fn the_settings_decide_the_background_work() {
        let both = UpdatePrefs { check: true, install: true };
        let check_only = UpdatePrefs { check: true, install: false };
        let off = UpdatePrefs { check: false, install: true };
        assert_eq!(background_work(&both, false), (true, true));
        assert_eq!(background_work(&check_only, false), (true, false));
        assert_eq!(background_work(&off, false), (false, true));
    }

    #[test]
    fn a_screenshot_or_fake_run_does_no_background_work() {
        assert_eq!(background_work(&UpdatePrefs::default(), true), (false, false));
        let mut fake = updates(Some(release("9.9.9")), false);
        assert!(fake.quiet);
        fake.tick();
        assert_eq!(fake.updater.status, Status::Available(release("9.9.9")));
        assert!(updates(None, true).quiet);
        assert!(!offline(false, false, false));
        assert!(offline(false, false, true));
    }

    #[test]
    fn a_reload_takes_the_new_switches_without_a_write_back() {
        let mut updates = updates(None, false);
        updates.reload(UpdatePrefs { check: false, install: false });
        assert_eq!(updates.prefs(), &UpdatePrefs { check: false, install: false });
    }

    #[test]
    fn each_switch_returns_the_whole_table_with_only_itself_changed() {
        let mut updates = updates(None, false);
        assert_eq!(updates.set_check(false), UpdatePrefs { check: false, install: true });
        assert_eq!(updates.set_install(false), UpdatePrefs { check: false, install: false });
        assert_eq!(updates.set_check(true), UpdatePrefs { check: true, install: false });
        assert_eq!(updates.prefs(), &UpdatePrefs { check: true, install: false });
    }

    #[test]
    fn the_status_line_says_what_the_updater_is_doing() {
        let cases = [
            (Status::Idle, "Tidewatch 0.1.0"),
            (Status::Checking, "Checking for updates..."),
            (Status::UpToDate, "Tidewatch 0.1.0 is the latest version"),
            (Status::Available(release("0.2.0")), "Tidewatch 0.2.0 is out (you have 0.1.0)"),
            (Status::Installing(release("0.2.0")), "Installing Tidewatch 0.2.0..."),
            (Status::Ready(release("0.2.0")), "Tidewatch 0.2.0 is installed. Restart to use it."),
            (
                Status::Failed("GitHub answered 404 without a release link".to_owned()),
                "Update failed: GitHub answered 404 without a release link",
            ),
            (
                Status::Stuck(PathBuf::from("/tmp/x/tidewatch/0.1.10.upgrading")),
                "Update stuck: an earlier Homebrew upgrade left /tmp/x/tidewatch/0.1.10.upgrading",
            ),
        ];
        for (status, want) in cases {
            assert_eq!(status_line(&status, "0.1.0"), want);
        }
    }

    #[test]
    fn the_sidebar_status_is_short_for_every_state() {
        let cases = [
            (Status::Idle, "Not checked yet"),
            (Status::Checking, "Checking..."),
            (Status::UpToDate, "Up to date"),
            (Status::Available(release("0.2.0")), "0.2.0 available"),
            (Status::Installing(release("0.2.0")), "Installing..."),
            (Status::Ready(release("0.2.0")), "Restart to update"),
            (Status::Failed("offline".to_owned()), "Update failed"),
            (Status::Stuck(PathBuf::from("/tmp/x")), "Update stuck"),
        ];
        for (status, want) in cases {
            assert_eq!(short_status(&status), want);
        }
    }

    #[test]
    fn saving_changes_only_the_updates_table_on_disk() {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        Prefs::update(&paths, |p| p.restore_session = false).unwrap();
        save(paths.clone(), UpdatePrefs { check: false, install: false });
        let wanted = UpdatePrefs { check: false, install: false };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while Prefs::load(&paths).updates != wanted && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let prefs = Prefs::load(&paths);
        assert_eq!(prefs.updates, wanted);
        assert!(!prefs.restore_session);
    }

    #[test]
    fn check_now_stays_off_the_network_in_a_quiet_run() {
        let mut updates = updates(None, true);
        updates.check_now();
        assert_eq!(updates.updater.status, Status::Idle);
    }

    fn pill_frame(
        ctx: &egui::Context,
        updates: &mut Updates,
        events: Vec<egui::Event>,
    ) -> egui::Rect {
        let mut used = egui::Rect::NOTHING;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(700.0, 200.0),
            )),
            focused: true,
            events,
            ..egui::RawInput::default()
        };
        let _ = ctx.run_ui(input, |ui| {
            used = ui.scope(|ui| updates.pill(ui)).response.rect;
        });
        used
    }

    fn click_pill(ctx: &egui::Context, updates: &mut Updates) {
        let at = pill_frame(ctx, updates, vec![]).center();
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        pill_frame(ctx, updates, vec![egui::Event::PointerMoved(at)]);
        pill_frame(ctx, updates, vec![button(true)]);
        pill_frame(ctx, updates, vec![button(false)]);
    }

    #[test]
    fn clicking_restart_on_an_installed_update_asks_this_app_to_restart_once() {
        let ctx = crate::fonts::test_context();
        let mut updates = updates(Some(release("9.9.9")), false);
        updates.updater.status = Status::Ready(release("9.9.9"));
        assert!(!updates.take_restart());
        click_pill(&ctx, &mut updates);
        assert!(updates.take_restart());
        assert!(!updates.take_restart(), "one click, one restart");
    }

    #[test]
    fn the_pill_of_an_update_that_is_not_installed_yet_does_not_ask_for_a_restart() {
        let ctx = crate::fonts::test_context();
        let mut updates = updates(Some(release("9.9.9")), false);
        click_pill(&ctx, &mut updates);
        assert!(!updates.take_restart());
    }

    #[test]
    fn the_chip_shows_once_there_is_an_update_to_tell_about() {
        let ctx = crate::fonts::test_context();
        let mut updates = updates(None, false);
        assert_eq!(pill_frame(&ctx, &mut updates, vec![]).width(), 0.0, "nothing to show yet");
        updates.updater.status = Status::Ready(release("0.2.0"));
        assert!(pill_frame(&ctx, &mut updates, vec![]).width() > 0.0);
    }

    #[test]
    fn the_fake_state_switch_reads_three_words() {
        assert_eq!(fake_state(Some("failed")), Some(FakeState::Failed));
        assert_eq!(fake_state(Some(" stuck ")), Some(FakeState::Stuck));
        assert_eq!(fake_state(Some("confirm")), Some(FakeState::Confirm));
        assert_eq!(fake_state(Some("nope")), None);
        assert_eq!(fake_state(None), None);
    }

    #[test]
    fn a_fake_state_shows_the_chip_and_keeps_the_run_offline() {
        let failed = updates(None, false).with_fake_state(Some(FakeState::Failed));
        assert_eq!(failed.updater.status, Status::Failed(FAKE_FAILURE.to_owned()));
        assert!(failed.quiet && !failed.fix_dialog && !failed.is_stuck());
        let stuck = updates(None, false).with_fake_state(Some(FakeState::Stuck));
        assert!(stuck.is_stuck() && !stuck.fix_dialog);
        let confirm = updates(None, false).with_fake_state(Some(FakeState::Confirm));
        assert!(confirm.is_stuck() && confirm.fix_dialog);
        assert_eq!(updates(None, false).with_fake_state(None).updater.status, Status::Idle);
    }

    #[test]
    fn clicking_the_failed_chip_asks_the_host_for_settings() {
        let ctx = crate::fonts::test_context();
        let mut updates = updates(None, false).with_fake_state(Some(FakeState::Failed));
        assert_eq!(updates.take_wish(), None);
        click_pill(&ctx, &mut updates);
        assert_eq!(updates.take_wish(), Some(Wish::OpenSettings));
        assert_eq!(updates.take_wish(), None);
        assert!(!updates.fix_dialog);
    }

    #[test]
    fn clicking_the_stuck_chip_opens_the_confirm_without_starting_anything() {
        let ctx = crate::fonts::test_context();
        let mut updates = updates(None, false).with_fake_state(Some(FakeState::Stuck));
        click_pill(&ctx, &mut updates);
        assert!(updates.fix_dialog);
        assert_eq!(updates.take_wish(), None);
    }

    #[test]
    fn quit_and_fix_under_a_fake_state_only_toasts_the_script() {
        let mut updates = updates(None, false).with_fake_state(Some(FakeState::Stuck));
        updates.start_fix();
        let Some(Wish::Toast(text)) = updates.take_wish() else { panic!("a toast") };
        assert!(text.starts_with("Fake update state, nothing was started."), "{text}");
        assert!(text.contains("brew reinstall --cask --force 'hobo-ware/tap/tidewatch'"), "{text}");
        assert!(!updates.updater.take_quit_request());
    }

    #[test]
    fn a_restart_is_refused_out_loud_while_installing() {
        let mut updates = updates(Some(release("9.9.9")), false);
        updates.updater.status = Status::Installing(release("9.9.9"));
        assert!(!updates.can_restart());
        updates.updater.status = Status::Ready(release("9.9.9"));
        assert!(updates.can_restart());
    }

    #[test]
    fn the_repair_script_names_the_cask_and_this_process() {
        let script = Updates::repair_script();
        assert!(script.contains(&format!("kill -0 {}", std::process::id())));
        assert!(script.contains("HOMEBREW_NO_AUTO_UPDATE=1 brew reinstall --cask --force"));
        assert!(script.contains("open -a '"));
    }
}
