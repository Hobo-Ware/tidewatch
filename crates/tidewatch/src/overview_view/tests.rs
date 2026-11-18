use super::*;
use crate::agenda::{Item, Mark};
use crate::overview::{AgentKind, PlaceInput, Row, test_row};
use crate::test_support::painted_text_rects_nonblank;
use egui::{Event, PointerButton, RawInput};
use tidewatch_core::progress::{Entry, Ledger};
use tidewatch_core::progress_report::ProgressReport;
use tidewatch_core::status::{AgentStatus, Reason};

const MAC: &str = "Plug the laptop in: on battery";

fn row(id: u64, title: &str, place: &str, status: AgentStatus) -> Row {
    let mut row = test_row(id, title, place, status);
    row.detail = Some("Bash: touch hello.txt".into());
    row.elapsed = 242.0;
    row.idle_secs = 600.0;
    row
}

fn overview(rows: Vec<Row>) -> Overview {
    let mut places: Vec<PlaceInput> = Vec::new();
    for row in rows {
        match places.iter_mut().find(|place| place.label == row.place) {
            Some(place) => place.rows.push(row),
            None => places.push(PlaceInput {
                label: row.place.clone(),
                path: format!("/work/{}", row.place),
                rows: vec![row],
            }),
        }
    }
    crate::overview::build(places)
}

fn walk_boxes(shape: &egui::Shape, found: &mut Vec<Rect>) {
    match shape {
        egui::Shape::Rect(rect) => found.push(rect.rect),
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk_boxes(shape, found)),
        _ => {}
    }
}

struct Screen {
    ctx: egui::Context,
    data: Overview,
    state: State,
    agenda: Vec<Day>,
    poofed: Vec<PoofedRow>,
    width: f32,
    height: f32,
    now_secs: i64,
}

impl Screen {
    fn new(data: Overview) -> Self {
        Self {
            ctx: crate::fonts::test_context(),
            data,
            state: State::default(),
            agenda: Vec::new(),
            poofed: Vec::new(),
            width: 1200.0,
            height: 900.0,
            now_secs: 0,
        }
    }

    fn frame(&mut self, events: Vec<Event>) -> (Out, Vec<(String, Rect)>) {
        let (out, found, _) = self.frame_drawn(events);
        (out, found)
    }

    fn frame_drawn(&mut self, events: Vec<Event>) -> (Out, Vec<(String, Rect)>, Vec<Rect>) {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(self.width, self.height))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let mut out = Out::default();
        let sections =
            Sections { agenda: &self.agenda, poofed: &self.poofed, now_secs: self.now_secs };
        let output = self.ctx.run_ui(raw, |ui| {
            let area = ui.max_rect();
            out = show(ui, area, &self.data, &mut self.state, &sections, |_| {});
        });
        let mut found = Vec::new();
        let mut boxes = Vec::new();
        for clipped in &output.shapes {
            painted_text_rects_nonblank(&clipped.shape, &mut found);
            walk_boxes(&clipped.shape, &mut boxes);
        }
        (out, found, boxes)
    }

    fn drawn(&mut self) -> (Vec<(String, Rect)>, Vec<Rect>) {
        self.frame(vec![]);
        let (_, found, boxes) = self.frame_drawn(vec![]);
        (found, boxes)
    }

    fn texts(&mut self) -> Vec<String> {
        self.frame(vec![]);
        self.frame(vec![]).1.into_iter().map(|(text, _)| text).collect()
    }

    fn click(&mut self, label: &str) -> Out {
        self.frame(vec![]);
        let found = self.frame(vec![]).1;
        let at = found.iter().find(|(text, _)| text == label).expect(label).1.center();
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]).0
    }

    fn assert_no_overlap(&mut self) {
        self.frame(vec![]);
        let found = self.frame(vec![]).1;
        for (index, (a, a_rect)) in found.iter().enumerate() {
            for (b, b_rect) in &found[index + 1..] {
                assert!(
                    !a_rect.shrink(0.5).intersects(b_rect.shrink(0.5)),
                    "{a:?} {a_rect:?} overlaps {b:?} {b_rect:?} at {}x{}",
                    self.width,
                    self.height
                );
            }
        }
    }
}

fn has(texts: &[String], want: &str) -> bool {
    texts.iter().any(|text| text == want)
}

fn two_repos() -> Overview {
    overview(vec![
        row(1, "web-working", "acme-web", AgentStatus::Working),
        row(2, "web-asks", "acme-web", AgentStatus::NeedsYou(Reason::Question)),
        row(3, "api-working", "acme-api", AgentStatus::Working),
        row(4, "api-idle", "acme-api", AgentStatus::Done),
    ])
}

#[test]
fn the_button_tip_says_what_the_badge_counts() {
    let repo = |name: &str, count: usize| (name.to_owned(), count);
    let cases = [
        (vec![], "Overview (K)"),
        (vec![repo("acme-web", 0)], "Overview (K)"),
        (vec![repo("acme-web", 1)], "Overview (K)\n1 agent needs you in acme-web"),
        (vec![repo("acme-web", 2)], "Overview (K)\n2 agents need you in acme-web"),
        (
            vec![repo("acme-web", 3), repo("acme-api", 0), repo("acme-docs", 1)],
            "Overview (K)\n4 agents need you: 3 in acme-web, 1 in acme-docs",
        ),
    ];
    for (needs, want) in cases {
        assert_eq!(button_tip("K", &needs), want, "{needs:?}");
    }
}

#[test]
fn a_quiet_agent_is_listed_under_idle_and_nothing_needs_you() {
    let mut screen = Screen::new(overview(vec![
        row(1, "went-quiet", "acme-web", AgentStatus::NeedsYou(Reason::Quiet)),
        row(2, "busy", "acme-web", AgentStatus::Working),
    ]));
    let texts = screen.texts();
    assert!(has(&texts, "Idle"), "{texts:?}");
    assert!(!texts.iter().any(|text| text.starts_with("Needs you")), "{texts:?}");
    assert!(has(&texts, "2 agents \u{b7} 1 working \u{b7} 1 idle"));
    assert!(has(&texts, "went-quiet"));
}

fn reported_row(id: u64, title: &str, state: &str, extra: &serde_json::Value) -> Row {
    let mut args = serde_json::json!({
        "goal": "g", "state": state, "now": "doing it",
        "stages": [
            { "name": "Main", "kind": "ship", "status": "active" },
            { "name": "Verify", "kind": "verify", "status": "todo" }
        ],
        "steps": [{ "stage": "Main", "text": "a step", "status": "active" }]
    });
    args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    let ledger = ProgressReport::check(args.as_object().unwrap(), None).unwrap().to_ledger();
    let mut row = row(id, title, "acme-web", AgentStatus::Done);
    let reported = ledger.shown_state();
    row.tone = Tone::shown(&row.status, Some(reported));
    row.look = Some(crate::progress_model::look_as(&ledger, false, reported));
    row
}

#[test]
fn waiting_agents_have_their_own_group_and_blocked_ones_sit_with_what_needs_you() {
    let mut screen = Screen::new(overview(vec![
        reported_row(
            1,
            "waits-on-pr",
            "waiting",
            &serde_json::json!({ "waiting_on": { "pr": 2340 } }),
        ),
        reported_row(
            2,
            "waits-on-date",
            "waiting",
            &serde_json::json!({ "waiting_on": { "date": "2026-10-28" } }),
        ),
        reported_row(3, "is-stuck", "blocked", &serde_json::json!({})),
        row(4, "busy", "acme-web", AgentStatus::Working),
    ]));
    let texts = screen.texts();
    for want in [
        "Waiting",
        "soonest first",
        "Waiting on PR #2340",
        "Waiting until Wed 28",
        "Needs you",
        "Review",
    ] {
        assert!(has(&texts, want), "{want:?} not drawn in {texts:?}");
    }
    assert!(has(&texts, "4 agents \u{b7} 1 need you \u{b7} 1 working \u{b7} 2 waiting"));
    let order = |name: &str| texts.iter().position(|text| text == name).unwrap();
    assert!(order("waits-on-date") < order("waits-on-pr"), "the dated wait is soonest");
    assert_eq!(screen.click("Review").jump, Some(3));
}

#[test]
fn a_picked_repo_that_loses_its_last_agent_falls_back_to_all_and_the_chips_agree() {
    let mut screen = Screen::new(two_repos());
    screen.click("acme-api");
    assert_eq!(screen.state.repo.as_deref(), Some("acme-api"));
    screen.data = crate::overview::build(vec![
        PlaceInput {
            label: "acme-web".into(),
            path: "/work/acme-web".into(),
            rows: vec![
                row(1, "web-working", "acme-web", AgentStatus::Working),
                row(2, "web-asks", "acme-web", AgentStatus::NeedsYou(Reason::Question)),
            ],
        },
        PlaceInput { label: "acme-api".into(), path: "/work/acme-api".into(), rows: vec![] },
    ]);
    let texts = screen.texts();
    assert_eq!(screen.state.repo, None);
    assert!(has(&texts, "web-working") && has(&texts, "web-asks"), "{texts:?}");
    assert!(!has(&texts, "Nothing running. Start an agent with"), "{texts:?}");
    assert!(has(&texts, "2 agents \u{b7} 1 need you \u{b7} 1 working"));
}

fn poofed(count: usize) -> Vec<PoofedRow> {
    (0..count)
        .map(|n| PoofedRow {
            index: n,
            name: format!("poof-{n}"),
            outcome: "PR #1".into(),
            took: "1m".into(),
            summary: String::new(),
            place: "acme-web".into(),
            secs: 60,
        })
        .collect()
}

fn rail_right(screen: &mut Screen) -> f32 {
    let (_, boxes) = screen.drawn();
    boxes
        .iter()
        .filter(|rect| rect.width() > 100.0 && rect.top() > 56.0)
        .map(egui::Rect::right)
        .filter(|right| *right < screen.width - 1.0)
        .fold(0.0, f32::max)
}

#[test]
fn the_rail_reaches_the_header_edge_until_the_body_scrolls_and_then_leaves_the_bar_room() {
    let mut screen = Screen::new(two_repos());
    brand::tint_visuals(&screen.ctx);
    screen.poofed = poofed(2);
    assert_eq!(rail_right(&mut screen), screen.width - SIDE);
    let rows: Vec<Row> =
        (0..40).map(|n| row(n, &format!("agent-{n}"), "acme-web", AgentStatus::Done)).collect();
    let mut busy = Screen::new(overview(rows));
    brand::tint_visuals(&busy.ctx);
    busy.poofed = poofed(2);
    let right = rail_right(&mut busy);
    assert_eq!(right, busy.width - SIDE - brand::SCROLL_ROOM);
}

#[test]
fn wrapped_chips_show_three_rows_and_scroll_past_them() {
    let cap = 3.0 * CHIP_H + 2.0 * CHIP_GAP;
    let cases = [(3, 500.0, false), (8, 500.0, false), (40, 500.0, true), (200, 900.0, true)];
    for (count, width, scrolls) in cases {
        let wrapped = wrap_chips(&vec![110.0; count], width);
        assert_eq!(wrapped.scrolls, scrolls, "{count} chips in {width}");
        assert!(wrapped.shown <= cap, "{count} chips in {width}");
        assert_eq!(wrapped.at.len(), count);
        if scrolls {
            let right = wrapped.at.iter().map(|(x, _)| x + 110.0).fold(0.0, f32::max);
            assert!(right <= width - brand::SCROLL_ROOM, "the bar keeps its room");
            assert!(wrapped.full > cap);
        }
    }
}

#[test]
fn many_repos_in_a_narrow_window_leave_the_rows_on_screen() {
    let rows: Vec<Row> = (0..80)
        .map(|n| row(n, &format!("agent-{n}"), &format!("repo-{n:02}"), AgentStatus::Working))
        .collect();
    let mut screen = Screen::new(overview(rows));
    screen.width = 600.0;
    let (found, _) = screen.drawn();
    let first = found.iter().find(|(text, _)| text == "agent-0").map(|(_, rect)| *rect);
    let first = first.expect("the first row is drawn");
    assert!(first.top() < 360.0, "the chips pushed the rows down to {}", first.top());
}

#[test]
fn a_repo_chip_narrows_every_row_and_all_brings_them_back() {
    let mut screen = Screen::new(two_repos());
    let texts = screen.texts();
    for name in ["web-working", "web-asks", "api-working", "api-idle"] {
        assert!(has(&texts, name), "{name}");
    }
    assert!(has(&texts, "All") && has(&texts, "acme-api") && has(&texts, "acme-web"));
    screen.click("acme-api");
    assert_eq!(screen.state.repo.as_deref(), Some("acme-api"));
    let texts = screen.texts();
    assert!(has(&texts, "api-working") && has(&texts, "api-idle"));
    assert!(!has(&texts, "web-working") && !has(&texts, "web-asks"));
    assert!(!texts.iter().any(|text| text.starts_with("Needs you")));
    assert!(has(&texts, "2 agents \u{b7} 1 working \u{b7} 1 idle"));
    screen.click("All");
    assert_eq!(screen.state.repo, None);
    assert!(has(&screen.texts(), "web-asks"));
}

fn item(row: usize, what: &str, place: &str, mark: Mark) -> Item {
    Item {
        row,
        what: what.into(),
        place: place.into(),
        tab: "t".into(),
        tab_id: None,
        note: None,
        mark,
        about: false,
        tip: String::new(),
    }
}

fn day(label: &str, today: bool, items: Vec<Item>) -> Day {
    Day {
        label: label.into(),
        today,
        month: (!today && label != "NO DATE").then(|| "October".to_owned()),
        starts_month: false,
        items,
    }
}

#[test]
fn a_repo_chip_narrows_the_rail_too() {
    let mut screen = Screen::new(two_repos());
    screen.agenda = vec![day(
        "TODAY \u{b7} MON 26",
        true,
        vec![
            item(0, "web chore", "acme-web", Mark::Plain),
            item(1, "api chore", "acme-api", Mark::Plain),
        ],
    )];
    let poofed = |index, name: &str, place: &str| PoofedRow {
        index,
        name: name.into(),
        outcome: "answered".into(),
        took: "2m".into(),
        summary: String::new(),
        place: place.into(),
        secs: 120,
    };
    screen.poofed = vec![poofed(0, "web poof", "acme-web"), poofed(1, "api poof", "acme-api")];
    let all = screen.texts();
    assert!(has(&all, "web chore") && has(&all, "api chore") && has(&all, "2 scheduled"));
    assert!(has(&all, "web poof") && has(&all, "api poof") && has(&all, "2 \u{b7} 4m"));
    screen.click("acme-web");
    let web = screen.texts();
    assert!(has(&web, "web chore") && !has(&web, "api chore") && has(&web, "1 scheduled"));
    assert!(has(&web, "web poof") && !has(&web, "api poof") && has(&web, "1 \u{b7} 2m"));
}

fn eight_working() -> Overview {
    overview(
        (0..8).map(|n| row(n, &format!("worker-{n}"), "acme-web", AgentStatus::Working)).collect(),
    )
}

#[test]
fn working_shows_six_then_show_all_expands_in_place() {
    let mut screen = Screen::new(eight_working());
    let texts = screen.texts();
    let shown = |texts: &[String]| (0..8).filter(|n| has(texts, &format!("worker-{n}"))).count();
    assert_eq!(shown(&texts), 6);
    assert!(has(&texts, "Show all 8"));
    screen.click("Show all 8");
    let texts = screen.texts();
    assert_eq!(shown(&texts), 8);
    assert!(has(&texts, "Show fewer") && !has(&texts, "Show all 8"));
    screen.click("Show fewer");
    assert_eq!(shown(&screen.texts()), 6);
}

#[test]
fn six_working_agents_have_no_show_all() {
    let mut screen = Screen::new(overview(
        (0..6).map(|n| row(n, &format!("worker-{n}"), "acme-web", AgentStatus::Working)).collect(),
    ));
    assert!(!screen.texts().iter().any(|text| text.starts_with("Show")));
}

#[test]
fn poofed_today_shows_three_and_show_all_expands_in_place() {
    let mut screen = Screen::new(two_repos());
    screen.poofed = poofed(5);
    let texts = screen.texts();
    assert_eq!((0..5).filter(|n| has(&texts, &format!("poof-{n}"))).count(), 3);
    assert!(has(&texts, "Show all 5") && has(&texts, "5 \u{b7} 5m"));
    screen.click("Show all 5");
    let texts = screen.texts();
    assert!((0..5).all(|n| has(&texts, &format!("poof-{n}"))));
    assert_eq!(screen.click("poof-4").poofed, Some(4));
}

#[test]
fn a_permission_ask_is_an_inbox_row_whose_review_button_opens_the_tab() {
    let mut waiting = row(7, "asks-a-thing", "acme-web", AgentStatus::NeedsYou(Reason::Permission));
    waiting.kind = AgentKind::Meeseeks;
    let mut screen = Screen::new(overview(vec![waiting]));
    let texts = screen.texts();
    for want in ["Needs you \u{b7} 1", "asks-a-thing", "Meeseeks", "acme-web", "Review"] {
        assert!(has(&texts, want), "{want} in {texts:?}");
    }
    assert_eq!(screen.click("Review").jump, Some(7));
}

#[test]
fn an_idle_row_reads_its_idle_line() {
    let mut idle = row(1, "wakes-later", "acme-web", AgentStatus::NeedsYou(Reason::Quiet));
    idle.idle_secs = 3.0 * 3600.0;
    let mut screen = Screen::new(overview(vec![idle]));
    assert!(screen.texts().iter().any(|text| text == "Idle 3h \u{b7} nothing scheduled"));
}

#[test]
fn the_overview_has_no_checklist() {
    let mut screen = Screen::new(two_repos());
    let texts = screen.texts();
    assert!(!texts.iter().any(|text| text.contains("Checklist") || text == "Edit"));
    assert!(!texts.iter().any(|text| text.contains("Four stages")));
}

#[test]
fn an_ask_for_the_owner_from_the_rail_moves_to_the_inbox_and_its_open_button_opens_its_row() {
    let mut screen = Screen::new(two_repos());
    screen.agenda = vec![
        day(
            "NO DATE",
            false,
            vec![
                item(3, "Install the bot", "acme-web", Mark::Decide),
                item(4, "Watch the deploy", "acme-web", Mark::Plain),
            ],
        ),
        day("TODAY \u{b7} MON 26", true, vec![item(5, "Dated chore", "acme-web", Mark::Plain)]),
    ];
    let texts = screen.texts();
    assert_eq!(texts.iter().filter(|text| *text == "Install the bot").count(), 1, "{texts:?}");
    assert!(!has(&texts, "Decide") && !has(&texts, "NO DATE"), "{texts:?}");
    assert!(has(&texts, "Dated chore") && has(&texts, "Waiting on events \u{b7} 1"));
    assert!(!has(&texts, "Watch the deploy"), "folded until opened");
    let out = screen.click("Open");
    assert_eq!((out.agenda, out.jump), (Some(3), None));
}

#[test]
fn the_rail_folds_events_and_parked_items_open_in_place() {
    let mut screen = Screen::new(two_repos());
    screen.agenda = vec![day(
        "NO DATE",
        false,
        vec![
            item(0, "Watch the deploy", "acme-web", Mark::Plain),
            item(1, "Revisit the plan", "acme-web", Mark::Parked),
        ],
    )];
    let texts = screen.texts();
    assert!(has(&texts, "Waiting on events \u{b7} 1") && has(&texts, "Parked \u{b7} 1"));
    assert!(!has(&texts, "Watch the deploy") && !has(&texts, "Revisit the plan"));
    screen.click("Waiting on events \u{b7} 1");
    let texts = screen.texts();
    assert!(has(&texts, "Watch the deploy") && !has(&texts, "Revisit the plan"));
    assert_eq!(screen.click("Watch the deploy").agenda, Some(0));
    screen.click("Parked \u{b7} 1");
    assert!(has(&screen.texts(), "Revisit the plan"));
    screen.click("Parked \u{b7} 1");
    assert!(!has(&screen.texts(), "Revisit the plan"));
}

#[test]
fn clicking_a_row_asks_for_its_tab_and_empty_space_asks_for_none() {
    let mut screen = Screen::new(two_repos());
    assert_eq!(screen.click("web-working").jump, Some(1));
    assert_eq!(screen.click("Overview").jump, None);
}

#[test]
fn an_empty_overview_has_nothing_to_click() {
    let mut screen = Screen::new(overview(vec![]));
    assert_eq!(screen.click("Overview"), Out::default());
}

const RETRY: &str = "# Goal\nx\n\n## Status\nstate: waiting\n\n## Needs you\n- Deploy failed twice: retry or skip? -- the second run broke too -- options: Retry* | Skip | Roll back\n";

fn asking(id: u64, title: &str, ledger: &str) -> Row {
    let mut asking = row(id, title, "acme-web", AgentStatus::NeedsYou(Reason::Question));
    asking.look = Some(crate::progress_model::look(&Ledger::parse(ledger), false));
    asking
}

#[test]
fn a_lone_question_shows_its_options_in_the_inbox_and_a_click_answers_that_tab() {
    let mut screen = Screen::new(overview(vec![asking(7, "deployer", RETRY)]));
    let texts = screen.texts();
    for want in ["Deploy failed twice: retry or skip?", "Retry", "Skip", "Roll back", "Chat"] {
        assert!(has(&texts, want), "{want} in {texts:?}");
    }
    let out = screen.click("Skip");
    assert_eq!((out.answer, out.jump), (Some((7, Click::Option(1))), None));
    assert_eq!(screen.click("Retry").answer, Some((7, Click::Option(0))));
}

#[test]
fn the_chat_button_chats_about_that_tab_and_does_not_open_it() {
    let mut screen = Screen::new(overview(vec![asking(7, "deployer", RETRY)]));
    let out = screen.click("Chat");
    assert_eq!((out.answer, out.jump), (Some((7, Click::Chat)), None));
}

#[test]
fn options_that_do_not_fit_leave_a_more_button_that_opens_the_tab_card() {
    let long = "# Goal\nx\n\n## Status\nstate: waiting\n\n## Needs you\n- Which loss should we keep?\n -- options: Keep the new loss and rerun everything | Keep the old loss for now | Blend both losses together | Rerun the benchmark first\n";
    let long = long.replace("?\n --", "? --");
    let mut screen = Screen::new(overview(vec![asking(7, "tuner", &long)]));
    screen.width = 900.0;
    let texts = screen.texts();
    assert!(has(&texts, "More"), "{texts:?}");
    assert_eq!(screen.click("More").answer, Some((7, Click::Open)));
}

#[test]
fn two_agents_asking_the_same_question_share_one_row_that_opens_the_first() {
    let mut screen =
        Screen::new(overview(vec![asking(7, "deployer", RETRY), asking(8, "releaser", RETRY)]));
    let texts = screen.texts();
    assert_eq!(texts.iter().filter(|text| text.starts_with("Deploy failed twice")).count(), 1);
    assert!(has(&texts, "2 agents \u{b7} waiting 10m") && has(&texts, "Needs you \u{b7} 1"));
    assert!(!has(&texts, "Skip"), "options of two agents are not answered inline");
    let out = screen.click("Open");
    assert_eq!((out.jump, out.answer), (Some(7), None));
}

#[test]
fn a_failed_agent_is_an_inbox_row_that_opens_its_tab() {
    let failed = row(3, "importer", "acme-web", AgentStatus::Failed("exit 1".into()));
    let mut screen = Screen::new(overview(vec![failed]));
    let texts = screen.texts();
    assert!(has(&texts, "Needs you \u{b7} 1") && has(&texts, "Failed \u{b7} exit 1"));
    assert_eq!(screen.click("Open").jump, Some(3));
}

fn steps(done: usize, total: usize, you: bool) -> Vec<serde_json::Value> {
    (0..total)
        .map(|n| {
            let mut step = serde_json::json!({
                "stage": "Build", "text": format!("Step {}", n + 1), "status": "todo"
            });
            if n < done {
                step["status"] = "done".into();
            } else if n == done {
                step["status"] = "active".into();
            } else if you && n == total - 1 {
                step["text"] = MAC.into();
                step["when"] = serde_json::json!({ "you": true });
            }
            step
        })
        .collect()
}

fn shaped_row(
    (id, title, place, status): (u64, &str, &str, AgentStatus),
    (done, total, you): (usize, usize, bool),
    (state, asked): (&str, &[&str]),
) -> Row {
    let mut args = serde_json::json!({
        "goal": "x", "state": state, "now": "doing it",
        "stages": [
            { "name": "Build", "kind": "build", "status": "active" },
            { "name": "Wrap up", "kind": "verify", "status": "todo" }
        ],
        "steps": steps(done, total, you),
    });
    if state == "waiting" {
        args["waiting_on"] = serde_json::json!({ "event": "the deploy" });
    }
    let mut report = ProgressReport::check(args.as_object().unwrap(), None).unwrap();
    report.stamp_done(None, tidewatch_core::clock::now_secs());
    let mut ledger = report.to_ledger();
    ledger.needs_you = asked
        .iter()
        .map(|text| Entry { text: (*text).into(), why: None, options: vec![] })
        .collect();
    let mut row = row(id, title, place, status);
    row.tokens =
        Some(crate::token_budget::TabTokens { short: "567M tokens".into(), tip: String::new() });
    let reported = ledger.shown_state();
    row.tone = Tone::shown(&row.status, Some(reported));
    row.look = Some(crate::progress_model::look_as(&ledger, false, reported));
    row
}

fn owner_like() -> Overview {
    overview(vec![
        shaped_row(
            (1, "fix-login-redirect", "acme-api", AgentStatus::Working),
            (0, 5, false),
            ("working", &[]),
        ),
        shaped_row(
            (2, "album-covers", "garden", AgentStatus::Working),
            (9, 18, true),
            ("working", &[]),
        ),
        shaped_row(
            (3, "import-backfill", "garden", AgentStatus::NeedsYou(Reason::Question)),
            (4, 12, true),
            (
                "waiting",
                &["Deploy failed twice: retry or skip?", "Which client shows the missing item?"],
            ),
        ),
    ])
}

/// The panels of the main column: the boxes at its left edge that are not the scroll area's own.
fn main_panel(boxes: &[Rect]) -> Rect {
    let panels = boxes
        .iter()
        .filter(|rect| (rect.left() - SIDE).abs() < 2.0 && rect.height() > 150.0)
        .min_by(|a, b| a.width().total_cmp(&b.width()));
    *panels.expect("a panel at the left edge")
}

#[test]
fn the_owner_shaped_overview_has_one_inbox_row_for_the_shared_ask_and_a_row_per_agent() {
    let mut screen = Screen::new(owner_like());
    let texts = screen.texts();
    assert_eq!(texts.iter().filter(|text| *text == MAC).count(), 1, "{texts:?}");
    assert!(has(&texts, "Needs you \u{b7} 3"), "the shared ask and two questions: {texts:?}");
    assert!(has(&texts, "2 agents \u{b7} waiting 10m"), "{texts:?}");
    for name in ["fix-login-redirect", "album-covers", "import-backfill"] {
        assert!(has(&texts, name), "{name}");
    }
    assert!(has(&texts, "9 of 18 \u{b7} 567M tokens") && has(&texts, "0 of 5 \u{b7} 567M tokens"));
}

#[test]
fn agent_rows_span_the_main_column_at_every_width() {
    for width in [900.0, 1280.0, 1512.0, 1920.0] {
        let mut screen = Screen::new(owner_like());
        screen.width = width;
        screen.height = 1400.0;
        let (found, boxes) = screen.drawn();
        let panel = main_panel(&boxes);
        let stacked = width < STACK_BELOW;
        let want = if stacked {
            width - 2.0 * SIDE
        } else {
            width - 2.0 * SIDE - rail_width(width) - COLUMN_GAP
        };
        assert!((panel.width() - want).abs() < 1.0, "{width}: panel {} want {want}", panel.width());
        let at = |label: &str| found.iter().find(|(text, _)| text == label).expect(label).1;
        let name = at("album-covers");
        let progress = at("9 of 18 \u{b7} 567M tokens");
        assert!(name.left() < panel.left() + 70.0, "{width}: name at {}", name.left());
        assert!(
            progress.left() > panel.center().x - 150.0,
            "{width}: progress at {}",
            progress.left()
        );
        let times: Vec<&Rect> = found
            .iter()
            .filter(|(text, rect)| {
                text.ends_with('m')
                    && text.len() <= 4
                    && rect.center().y > name.top() - 10.0
                    && rect.center().y < name.bottom() + 20.0
            })
            .map(|(_, rect)| rect)
            .collect();
        let right = times.iter().map(|rect| rect.right()).fold(0.0, f32::max);
        assert!(
            right > panel.right() - 40.0,
            "{width}: time ends at {right}, panel at {}",
            panel.right()
        );
    }
}

#[test]
fn no_text_overlaps_in_the_owner_shaped_overview_at_any_width() {
    for width in [700.0, 900.0, 1280.0, 1512.0, 1920.0] {
        for height in [500.0, 900.0, 1400.0] {
            let mut screen = Screen::new(owner_like());
            screen.width = width;
            screen.height = height;
            screen.poofed = poofed(2);
            screen.agenda = vec![
                day(
                    "TODAY \u{b7} MON 26",
                    true,
                    vec![item(0, "Rerun the check", "garden", Mark::Plain)],
                ),
                day(
                    "NO DATE",
                    false,
                    vec![
                        item(1, MAC, "garden", Mark::Decide),
                        item(2, "Revisit", "garden", Mark::Parked),
                    ],
                ),
            ];
            screen.assert_no_overlap();
        }
    }
}

fn step_lines(texts: &[String]) -> usize {
    texts.iter().filter(|text| text.starts_with("Step ")).count()
}

#[test]
fn rows_expand_to_four_step_lines_in_a_tall_window_and_collapse_to_two_in_a_short_one() {
    let mut tall = Screen::new(owner_like());
    tall.height = 1500.0;
    let texts = tall.texts();
    assert!(step_lines(&texts) >= 8, "expanded rows list their steps: {texts:?}");
    assert!(!texts.iter().any(|text| text.starts_with("next: ")), "{texts:?}");
    let mut short = Screen::new(owner_like());
    short.height = 700.0;
    let texts = short.texts();
    assert!(texts.iter().any(|text| text.starts_with("next: ")), "{texts:?}");
    assert!(step_lines(&texts) < 8, "{texts:?}");
}

#[test]
fn finished_today_lists_steps_ticked_today_newest_first_and_folds_the_rest() {
    let mut screen = Screen::new(owner_like());
    screen.now_secs = i64::try_from(tidewatch_core::clock::now_secs()).unwrap();
    screen.height = 1500.0;
    let texts = screen.texts();
    assert!(has(&texts, "Finished today"), "{texts:?}");
    assert!(has(&texts, "Step 1") && has(&texts, "album-covers"));
    let mut cramped = Screen::new(owner_like());
    cramped.now_secs = screen.now_secs;
    cramped.height = 1000.0;
    let texts = cramped.texts();
    let ticked =
        texts.iter().filter(|text| text.as_str() >= "Step 1" && text.starts_with("Step ")).count();
    assert!(ticked > 0);
    let hidden =
        texts.iter().find(|text| text.ends_with(" more") && !text.starts_with("Show")).cloned();
    if let Some(label) = hidden {
        cramped.click(&label);
        assert!(has(&cramped.texts(), "Show fewer"));
    }
}
