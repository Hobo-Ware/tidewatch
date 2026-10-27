use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use stdusk_core::repaint::Repaint;
use stdusk_core::shell::ShellProfile;
use stdusk_core::terminal::SpawnOpts;
use tidewatch_core::status::AgentStatus;
use tidewatch_core::tools::Tools;

use super::*;
use crate::agent_run::AgentRun;
use crate::pane::AgentTab;

const TAB: u64 = 0;

fn echoing_terminal() -> PtyTerm {
    PtyTerm::spawn(
        80,
        24,
        Repaint::none(),
        &SpawnOpts {
            detect_progress: false,
            shell_integration: false,
            autosuggestions: false,
            scrollback_lines: 200,
            word_separators: " ".into(),
            bold_bright: false,
            agent_tracking: None,
            cwd: None,
            profile: Some(ShellProfile {
                shell: Some("/bin/sh".into()),
                args: vec!["-c".into(), "cat".into()],
                cwd: None,
                env: BTreeMap::new(),
            }),
        },
    )
}

fn agent_in(status: AgentStatus) -> Tabs<Pane<PtyTerm>> {
    let mut pane = Pane::shell(echoing_terminal());
    let mut agent = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
    agent.run.hold(Some(status), None);
    agent.run.refresh(0.0);
    pane.agent = Some(agent);
    Tabs::new(pane)
}

fn set_status(tabs: &mut Tabs<Pane<PtyTerm>>, status: AgentStatus) {
    let agent = tabs.get_mut(TAB).unwrap().pane.agent.as_mut().unwrap();
    agent.run.hold(Some(status), None);
    agent.run.refresh(1.0);
}

fn screen(tabs: &Tabs<Pane<PtyTerm>>) -> String {
    match &tabs.items()[0].pane.body {
        crate::pane::Body::Shell(term) => {
            let rows = crate::job_glue::screen_rows(&term.grid_snapshot());
            rows.concat().split_whitespace().collect()
        }
        _ => String::new(),
    }
}

fn asked() -> String {
    ASK.split_whitespace().collect()
}

fn typed_within_a_while(tabs: &Tabs<Pane<PtyTerm>>) -> bool {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(5) {
        if screen(tabs).contains(&asked()) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn an_idle_agent_is_asked_at_once_and_the_owner_hears_its_name() {
    let mut tabs = agent_in(AgentStatus::Done);
    let (mut flow, ctx) = (Flow::new(Tools::default()), Context::default());
    let mut waiting = Waiting::default();
    waiting.add(TAB);
    let toasts = waiting.drive(&mut tabs, &mut flow, &ctx, 1.0);
    assert_eq!(toasts, ["Asked main to redraw its icon"]);
    assert!(typed_within_a_while(&tabs), "{}", screen(&tabs));
    assert!(!waiting.holds(TAB));
}

#[test]
fn a_busy_agent_is_asked_nothing_until_it_is_idle_and_then_once() {
    let mut tabs = agent_in(AgentStatus::Working);
    let (mut flow, ctx) = (Flow::new(Tools::default()), Context::default());
    let mut waiting = Waiting::default();
    waiting.add(TAB);
    for now in [1.0, 2.0, 3.0] {
        assert_eq!(waiting.drive(&mut tabs, &mut flow, &ctx, now), Vec::<String>::new());
        assert!(waiting.holds(TAB), "the ask is held while the agent works");
    }
    std::thread::sleep(Duration::from_millis(300));
    assert!(!screen(&tabs).contains(&asked()), "nothing was typed into a working agent");
    set_status(&mut tabs, AgentStatus::Done);
    let toasts = waiting.drive(&mut tabs, &mut flow, &ctx, 4.0);
    assert!(typed_within_a_while(&tabs), "{}", screen(&tabs));
    assert_eq!(toasts, ["Asked main to redraw its icon"]);
    assert_eq!(waiting.drive(&mut tabs, &mut flow, &ctx, 5.0), Vec::<String>::new());
    assert_eq!(screen(&tabs).matches(&asked()).count(), 1, "asked once");
}

#[test]
fn an_agent_waiting_on_the_owner_holds_the_ask_too() {
    let mut tabs = agent_in(AgentStatus::NeedsYou(tidewatch_core::status::Reason::Permission));
    let (mut flow, ctx) = (Flow::new(Tools::default()), Context::default());
    let mut waiting = Waiting::default();
    waiting.add(TAB);
    assert_eq!(waiting.drive(&mut tabs, &mut flow, &ctx, 1.0), Vec::<String>::new());
    assert!(waiting.holds(TAB));
}

#[test]
fn a_tab_with_no_agent_to_ask_says_why_and_a_closed_tab_is_forgotten_quietly() {
    let mut tabs = agent_in(AgentStatus::Stopped);
    let (mut flow, ctx) = (Flow::new(Tools::default()), Context::default());
    let mut waiting = Waiting::default();
    waiting.add(TAB);
    assert_eq!(
        waiting.drive(&mut tabs, &mut flow, &ctx, 1.0),
        [crate::outbox::NO_AGENT.to_owned()]
    );
    assert!(!waiting.holds(TAB));
    waiting.add(99);
    assert_eq!(waiting.drive(&mut tabs, &mut flow, &ctx, 2.0), Vec::<String>::new());
    assert!(!waiting.holds(99));
}

#[test]
fn asking_twice_while_it_waits_holds_one_ask() {
    let mut waiting = Waiting::default();
    waiting.add(TAB);
    waiting.add(TAB);
    assert_eq!(waiting.tabs, [TAB]);
}

#[test]
fn a_saved_avatar_that_fails_the_check_shows_the_letter_and_asks_its_idle_agent_to_redraw_once() {
    use std::path::PathBuf;
    use tidewatch_core::avatar::clean;
    use tidewatch_core::avatar_store;
    use tidewatch_core::paths::avatar_file_beside;

    use crate::avatar_glue::Host;
    use crate::job_glue::Facts;

    let dir = tempfile::tempdir().unwrap();
    let notes: PathBuf = dir.path().join("notes.json");
    let dark =
        "<svg viewBox=\"0 0 64 64\"><circle cx=\"32\" cy=\"32\" r=\"20\" fill=\"#2a2f3a\"/></svg>";
    avatar_store::save(&avatar_file_beside(&notes), &clean(dark, "old").unwrap()).unwrap();
    let (ctx, mut host) = (Context::default(), Host::new(true, false));
    let mut waiting = Waiting::default();
    let started = Instant::now();
    while !waiting.holds(TAB) && started.elapsed() < Duration::from_secs(5) {
        host.tick(&[Facts { id: TAB, notes_file: &notes }], &ctx);
        waiting.queue_unreadable(&mut host);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(host.get(TAB).is_none(), "the tab shows the letter");
    assert!(waiting.holds(TAB), "one redraw ask is queued");

    let mut tabs = agent_in(AgentStatus::Working);
    let mut flow = Flow::new(Tools::default());
    assert_eq!(waiting.drive(&mut tabs, &mut flow, &ctx, 1.0), Vec::<String>::new());
    set_status(&mut tabs, AgentStatus::Done);
    assert_eq!(waiting.drive(&mut tabs, &mut flow, &ctx, 2.0), ["Asked main to redraw its icon"]);
    assert!(typed_within_a_while(&tabs), "{}", screen(&tabs));
    host.tick(&[Facts { id: TAB, notes_file: &notes }], &ctx);
    waiting.queue_unreadable(&mut host);
    assert_eq!(waiting.drive(&mut tabs, &mut flow, &ctx, 3.0), Vec::<String>::new());
    assert_eq!(screen(&tabs).matches(&asked()).count(), 1, "asked once");
}
