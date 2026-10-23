use std::path::PathBuf;
use std::process::Command;

use tidewatch_core::comment_flags::TabFiles;

use super::*;

struct Lab {
    _root: tempfile::TempDir,
    paths: TidewatchPaths,
    repo: PathBuf,
}

impl Lab {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(&root.path().join("home"));
        let repo = root.path().join("web");
        std::fs::create_dir_all(&repo).unwrap();
        let lab = Self { _root: root, paths, repo };
        lab.git(&["init", "-q", "-b", "main"]);
        lab.git(&["commit", "-q", "--allow-empty", "-m", "init"]);
        lab.git(&["switch", "-q", "-c", "agent/work"]);
        lab
    }

    fn git(&self, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(&self.repo)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn tab_files(&self) -> TabFiles {
        TabFiles::new(&self.paths.tab_state_dir(&self.repo, "work"))
    }

    fn facts<'a>(&'a self, agent: &'a str, status: &'a AgentStatus) -> Vec<Facts<'a>> {
        vec![Facts {
            id: 1,
            agent_id: agent,
            status,
            dir: &self.repo,
            repo: &self.repo,
            name: "work",
        }]
    }
}

fn host() -> Host {
    Host { tabs: HashMap::new(), jobs: Jobs::new(), fake: false, popover: false }
}

fn wait_for_pastes(
    host: &mut Host,
    lab: &Lab,
    agent: &str,
    ctx: &egui::Context,
    now: &mut f64,
) -> Vec<Paste> {
    for _ in 0..200 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        *now += 2.0;
        let pastes = host.tick(
            &lab.facts(agent, &AgentStatus::Done),
            &lab.paths,
            &GuardPrefs::default(),
            ctx,
            *now,
        );
        if !pastes.is_empty() {
            return pastes;
        }
    }
    Vec::new()
}

#[test]
fn flags_the_hook_wrote_show_up_on_the_tab_and_the_off_switch_is_the_marker_file() {
    let lab = Lab::new();
    let ctx = egui::Context::default();
    let mut host = host();
    let guards = GuardPrefs::default();
    host.tick(&lab.facts("claude", &AgentStatus::Working), &lab.paths, &guards, &ctx, 0.0);
    assert!(host.view(1).is_none());

    let record = Record {
        at_ms: 1,
        origin: Origin::Edit,
        file: "a.rs".into(),
        line: 2,
        kind: "narration".into(),
        text: "x".into(),
    };
    lab.tab_files().append(std::slice::from_ref(&record)).unwrap();
    host.tick(&lab.facts("claude", &AgentStatus::Working), &lab.paths, &guards, &ctx, 0.5);
    assert!(host.view(1).is_none(), "looks are a second apart");
    host.tick(&lab.facts("claude", &AgentStatus::Working), &lab.paths, &guards, &ctx, 1.5);
    assert_eq!(host.view(1).unwrap().records, [record]);

    host.set_off(1, true);
    assert!(lab.tab_files().is_off());
    assert!(host.view(1).unwrap().off);
    host.set_off(1, false);
    assert!(!lab.tab_files().is_off());
}

#[test]
fn a_tab_that_closed_is_forgotten() {
    let lab = Lab::new();
    let ctx = egui::Context::default();
    let mut host = host();
    host.tick(
        &lab.facts("claude", &AgentStatus::Working),
        &lab.paths,
        &GuardPrefs::default(),
        &ctx,
        0.0,
    );
    lab.tab_files().set_off(true).unwrap();
    host.tick(
        &lab.facts("claude", &AgentStatus::Working),
        &lab.paths,
        &GuardPrefs::default(),
        &ctx,
        2.0,
    );
    assert!(host.view(1).is_some());
    host.tick(&[], &lab.paths, &GuardPrefs::default(), &ctx, 4.0);
    assert!(host.view(1).is_none());
}

fn end_a_turn(host: &mut Host, lab: &Lab, agent: &str, ctx: &egui::Context, guards: &GuardPrefs) {
    host.tick(&lab.facts(agent, &AgentStatus::Working), &lab.paths, guards, ctx, 0.0);
    host.tick(&lab.facts(agent, &AgentStatus::Done), &lab.paths, guards, ctx, 2.0);
}

#[test]
fn another_cli_at_the_end_of_a_turn_is_swept_and_its_idle_agent_gets_one_note() {
    let lab = Lab::new();
    std::fs::write(lab.repo.join("a.rs"), "// loop over the items\nfn a() {}\n").unwrap();
    let ctx = egui::Context::default();
    let mut host = host();
    end_a_turn(&mut host, &lab, "codex", &ctx, &GuardPrefs::default());
    let mut now = 2.0;
    let pastes = wait_for_pastes(&mut host, &lab, "codex", &ctx, &mut now);
    assert_eq!(pastes.len(), 1);
    assert_eq!(pastes[0].tab, 1);
    assert!(pastes[0].text.starts_with("Comment guard: 1 comment(s) you added fail the keep-list.\n- a.rs:1 [narration] loop over the items"), "{}", pastes[0].text);
    assert_eq!(lab.tab_files().load().len(), 1);

    end_a_turn(&mut host, &lab, "codex", &ctx, &GuardPrefs::default());
    let again = wait_for_pastes(&mut host, &lab, "codex", &ctx, &mut now);
    assert_eq!(again, vec![], "the same report is not pasted twice");
}

#[test]
fn claude_the_rule_off_and_the_tab_off_are_never_swept() {
    let ctx = egui::Context::default();
    let mut now = 2.0;
    for case in ["claude", "off", "tab"] {
        let lab = Lab::new();
        std::fs::write(lab.repo.join("a.rs"), "// loop over the items\nfn a() {}\n").unwrap();
        let mut guards = GuardPrefs::default();
        let agent = if case == "claude" { "claude" } else { "codex" };
        if case == "off" {
            guards.rules.set(Rule::Comments, false);
        }
        if case == "tab" {
            lab.tab_files().set_off(true).unwrap();
        }
        let mut host = host();
        end_a_turn(&mut host, &lab, agent, &ctx, &guards);
        std::thread::sleep(std::time::Duration::from_millis(200));
        let pastes =
            host.tick(&lab.facts(agent, &AgentStatus::Done), &lab.paths, &guards, &ctx, now);
        now += 2.0;
        assert_eq!(pastes, vec![], "{case}");
        assert_eq!(lab.tab_files().load(), vec![], "{case}");
    }
}

#[test]
fn a_sweep_that_finishes_while_the_agent_works_again_pastes_nothing() {
    let lab = Lab::new();
    std::fs::write(lab.repo.join("a.rs"), "// loop over the items\nfn a() {}\n").unwrap();
    let ctx = egui::Context::default();
    let mut host = host();
    let guards = GuardPrefs::default();
    end_a_turn(&mut host, &lab, "codex", &ctx, &guards);
    std::thread::sleep(std::time::Duration::from_millis(300));
    let pastes =
        host.tick(&lab.facts("codex", &AgentStatus::Working), &lab.paths, &guards, &ctx, 4.0);
    assert_eq!(pastes, vec![]);
    assert_eq!(lab.tab_files().load(), vec![]);
}

#[test]
fn the_fake_switch_gives_a_tab_three_flags_and_writes_nothing() {
    let lab = Lab::new();
    let ctx = egui::Context::default();
    let mut host = Host { fake: true, ..host() };
    host.tick(
        &lab.facts("claude", &AgentStatus::Working),
        &lab.paths,
        &GuardPrefs::default(),
        &ctx,
        0.0,
    );
    assert_eq!(host.view(1).unwrap().records.len(), 3);
    host.set_off(1, true);
    assert!(host.view(1).unwrap().off);
    assert!(!lab.tab_files().is_off());
}
