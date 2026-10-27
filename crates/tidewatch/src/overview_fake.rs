//! `TIDEWATCH_FAKE_OVERVIEW=<n>` stand-ins for a capture of the Overview: twelve agents of three made-up
//! repos (n of them working, 5 by default), a Coming up rail, eight poofed Meeseeks and two sessions
//! running elsewhere. `TIDEWATCH_FAKE_OVERVIEW_REPO=<name>` starts with that repo chip picked.

use tidewatch_core::civil_date::Date;
use tidewatch_core::elsewhere::{Cli, Key};
use tidewatch_core::progress::{Ledger, State as Reported};
use tidewatch_core::progress_report::ProgressReport;
use tidewatch_core::status::{AgentStatus, Reason};

use crate::agenda::{Day, Item, Mark};
use crate::overview::{self, AgentKind, Next, Overview, PlaceInput, PoofedRow, Row};
use crate::overview_view::State;
use crate::progress_model::{look, look_as};
use crate::status_view::shown;
use crate::takeover::{Offer, Status};
use crate::takeover_view::Card;

pub(crate) struct Fake {
    pub(crate) overview: Overview,
    pub(crate) agenda: Vec<Day>,
    pub(crate) poofed: Vec<PoofedRow>,
    pub(crate) takeover: Card,
}

pub(crate) fn requested() -> Option<usize> {
    let value = std::env::var("TIDEWATCH_FAKE_OVERVIEW").ok()?;
    Some(value.parse().unwrap_or(5))
}

pub(crate) fn initial_state() -> State {
    State { repo: std::env::var("TIDEWATCH_FAKE_OVERVIEW_REPO").ok(), ..State::default() }
}

fn kind_of(phase: &str) -> &'static str {
    match phase {
        "Understand" => "understand",
        "PR" => "ship",
        _ => "build",
    }
}

fn report(state: &str, now: &str, stage: (&str, &str), extra: &serde_json::Value) -> Ledger {
    let mut args = serde_json::json!({
        "goal": "x",
        "state": state,
        "now": now,
        "stages": [
            { "name": stage.0, "kind": stage.1, "status": "active" },
            { "name": "Wrap up", "kind": "verify", "status": "todo" }
        ],
        "steps": [{ "stage": stage.0, "text": "step", "status": "active" }],
    });
    if let (Some(args), Some(extra)) = (args.as_object_mut(), extra.as_object()) {
        args.extend(extra.clone());
    }
    if state == "done" {
        args["stages"][0]["status"] = serde_json::json!("done");
        args["steps"][0]["status"] = serde_json::json!("done");
    }
    ProgressReport::check(args.as_object().expect("the stand-in report is an object"), None)
        .map(|report| report.to_ledger())
        .expect("the stand-in report follows the contract")
}

fn ledger_look(phase: &str, note: &str, done: usize, total: usize) -> crate::progress_model::Look {
    let steps: Vec<serde_json::Value> = (0..total)
        .map(|n| {
            let status = if n < done { "done" } else { "todo" };
            serde_json::json!({ "stage": phase, "text": format!("step {n}"), "status": status })
        })
        .collect();
    let mut ledger =
        report("working", note, (phase, kind_of(phase)), &serde_json::json!({ "steps": steps }));
    note.clone_into(&mut ledger.status.note);
    look(&ledger, false)
}

fn ask_look(question: &str, why: &str, options: &str) -> crate::progress_model::Look {
    let why = if why.is_empty() { String::new() } else { format!(" -- {why}") };
    let options =
        if options.is_empty() { String::new() } else { format!(" -- options: {options}") };
    look(
        &Ledger::parse(&format!(
            "# Goal\nx\n\n## Status\nstate: waiting\n\n## Needs you\n- {question}{why}{options}\n"
        )),
        false,
    )
}

/// A row whose agent reported `state`: its tone follows the report, as on every other screen.
fn reported_row(id: u64, spec: &Spec, state: Reported, ledger: &Ledger) -> Row {
    let mut r = row(id, spec);
    r.look = Some(look_as(ledger, false, state));
    r.tone = shown(&spec.status, Some(state));
    r
}

struct Spec {
    title: &'static str,
    place: &'static str,
    kind: AgentKind,
    status: AgentStatus,
    line: &'static str,
    mins: f64,
}

fn row(id: u64, spec: &Spec) -> Row {
    Row {
        tab_id: id,
        title: spec.title.to_owned(),
        place: spec.place.to_owned(),
        profile: spec.title.to_owned(),
        workflow: None,
        agent_id: match &spec.kind {
            AgentKind::Cli(id) => id.clone(),
            _ => "claude".to_owned(),
        },
        branch: String::new(),
        status: spec.status.clone(),
        tone: crate::status_view::tone(&spec.status),
        detail: Some(spec.line.to_owned()),
        end_block: None,
        elapsed: spec.mins * 60.0,
        pr: None,
        done_check: None,
        look: None,
        job_line: None,
        avatar: crate::avatar_glue::fake_requested()
            .then(|| crate::avatar_glue::fake_face(id as usize))
            .flatten(),
        kind: spec.kind.clone(),
        auto_pick: None,
        idle_secs: spec.mins * 60.0,
        next: None,
    }
}

fn cli(id: &str) -> AgentKind {
    AgentKind::Cli(id.to_owned())
}

pub(crate) fn build(working: usize) -> Fake {
    use AgentStatus::{Done, NeedsYou, Working};
    let spec =
        |title, place, kind, status, line, mins| Spec { title, place, kind, status, line, mins };
    let needs = [
        spec(
            "recommendation-nn",
            "acme-workers",
            cli("claude"),
            NeedsYou(Reason::Question),
            "Asks which loss to keep",
            30.0,
        ),
        spec(
            "investigate-fix-season",
            "acme-api",
            cli("codex"),
            NeedsYou(Reason::Permission),
            "Wants to run a migration",
            55.0,
        ),
        spec(
            "dedupe-show-slugs",
            "acme-web",
            AgentKind::Meeseeks,
            NeedsYou(Reason::Question),
            "Asks: keep old URLs as redirects?",
            9.0,
        ),
    ];
    let names = [
        (
            "migrate-typescript-tsrs",
            "acme-web",
            cli("claude"),
            "Build",
            "type check on CI",
            124.0,
            7,
            10,
        ),
        (
            "soundtrack-retry",
            "acme-workers",
            AgentKind::Meeseeks,
            "Understand",
            "reading job logs",
            12.0,
            1,
            4,
        ),
        ("media-sync", "acme-workers", AgentKind::Yolo, "Build", "4 files changed", 38.0, 4, 8),
        ("watchlist-sort-fix", "acme-web", cli("codex"), "Build", "running tests", 21.0, 6, 10),
        (
            "figure-out-why-the-lookup-keeps-timing-out-on-replica",
            "acme-workers",
            AgentKind::Meeseeks,
            "Understand",
            "querying replica",
            6.0,
            1,
            7,
        ),
        (
            "search-index-rebuild",
            "acme-api",
            cli("claude"),
            "Build",
            "reindexing shows",
            47.0,
            5,
            9,
        ),
        ("push-notification-copy", "acme-api", cli("codex"), "PR", "waiting on checks", 83.0, 8, 9),
        (
            "cache-warmup-job",
            "acme-workers",
            AgentKind::Yolo,
            "Build",
            "profiling the loader",
            15.0,
            2,
            8,
        ),
        (
            "season-art-fallback",
            "acme-web",
            cli("claude"),
            "Build",
            "writing the fallback",
            29.0,
            3,
            8,
        ),
    ];
    let mut working_rows: Vec<Row> = names
        .iter()
        .cycle()
        .take(working)
        .enumerate()
        .map(|(n, (title, place, kind, phase, note, mins, done, total))| {
            let mut r =
                row(100 + n as u64, &spec(title, place, kind.clone(), Working, note, *mins));
            r.look = Some(ledger_look(phase, note, *done, *total));
            r
        })
        .collect();
    let mut idle = vec![
        spec("pkce-auth-security", "acme-api", cli("claude"), NeedsYou(Reason::Quiet), "", 180.0),
        spec(
            "figure-out-precisely-triggered",
            "acme-workers",
            cli("claude"),
            NeedsYou(Reason::Quiet),
            "",
            300.0,
        ),
        spec("calendar-ics-export", "acme-api", AgentKind::Yolo, Done, "", 12.0),
        spec("search-rank-tune", "acme-web", cli("codex"), Done, "", 60.0),
    ];
    let mut idle_rows: Vec<Row> =
        idle.drain(..).enumerate().map(|(n, s)| row(200 + n as u64, &s)).collect();
    let date = |day| Date::new(2026, 10, day);
    idle_rows[0].next =
        Some(Next { date: date(26), days: Some(0), what: "rerun no-secret check".into() });
    idle_rows[1].next =
        Some(Next { date: date(31), days: Some(5), what: "read back the 33 people".into() });
    let mut needs_rows: Vec<Row> =
        needs.iter().enumerate().map(|(n, s)| row(10 + n as u64, s)).collect();
    needs_rows[0].look = Some(ask_look(
        "Which loss should the recommendation model keep?",
        "Run 14 beat the others on held-out data",
        "Keep the new loss and rerun the whole benchmark on the replica before the rollout* | Keep the old loss | Blend both losses | Rerun the benchmark",
    ));
    needs_rows[2].look = Some(ask_look("Keep the old URLs as redirects?", "", ""));
    needs_rows[2].auto_pick = Some("Picks the recommended answer in 3:40".to_owned());
    let waits = [
        (
            "recap-backfill",
            "acme-web",
            cli("claude"),
            "Backfill",
            "Backfill batch 2 is running",
            serde_json::json!({ "waiting_on": { "date": "2026-10-29" } }),
            42.0,
        ),
        (
            "deploy-season-links",
            "acme-api",
            cli("codex"),
            "Deploy",
            "Deploy is queued behind the freeze",
            serde_json::json!({ "waiting_on": { "date": "2026-10-27" } }),
            130.0,
        ),
        (
            "partner-feed-switch",
            "acme-workers",
            AgentKind::Yolo,
            "Cutover",
            "Waiting for the partner to confirm the new feed",
            serde_json::json!({ "waiting_on": { "event": "the partner confirms" } }),
            75.0,
        ),
    ];
    for (n, (title, place, kind, stage, note, extra, mins)) in waits.into_iter().enumerate() {
        let spec = spec(title, place, kind, Done, "", mins);
        let ledger = report("waiting", note, (stage, "ship"), &extra);
        idle_rows.push(reported_row(300 + n as u64, &spec, Reported::Waiting, &ledger));
    }
    let blocked = spec("rebuild-people-index", "acme-workers", cli("claude"), Done, "", 95.0);
    let stuck = report(
        "blocked",
        "The staging database refuses writes",
        ("Build", "build"),
        &serde_json::json!({}),
    );
    needs_rows.push(reported_row(20, &blocked, Reported::Blocked, &stuck));
    let finished = spec("season-recap-links", "acme-web", cli("claude"), Done, "", 8.0);
    let summary =
        report("done", "Shipped and checked live", ("Verify", "verify"), &serde_json::json!({}));
    idle_rows.push(reported_row(310, &finished, Reported::Done, &summary));
    let mut by_place: Vec<(&str, Vec<Row>)> =
        vec![("acme-api", vec![]), ("acme-web", vec![]), ("acme-workers", vec![])];
    for r in needs_rows.drain(..).chain(working_rows.drain(..)).chain(idle_rows.drain(..)) {
        if let Some((_, rows)) = by_place.iter_mut().find(|(place, _)| *place == r.place) {
            rows.push(r);
        }
    }
    let places = by_place
        .into_iter()
        .map(|(label, rows)| PlaceInput {
            label: label.to_owned(),
            path: format!("/work/{label}"),
            rows,
        })
        .collect();
    Fake {
        overview: overview::build(places),
        agenda: agenda(),
        poofed: poofed(),
        takeover: takeover(),
    }
}

fn item(row: usize, what: &str, place: &str, tab: &str, note: Option<&str>, mark: Mark) -> Item {
    Item {
        row,
        what: what.to_owned(),
        place: place.to_owned(),
        tab: tab.to_owned(),
        note: note.map(str::to_owned),
        mark,
        tip: what.to_owned(),
    }
}

fn agenda() -> Vec<Day> {
    vec![
        Day {
            label: "TODAY \u{b7} MON 26".to_owned(),
            today: true,
            items: vec![
                item(
                    0,
                    "Rerun the no-secret check, mark #1999 ready",
                    "acme-api",
                    "pkce-auth-security",
                    None,
                    Mark::Plain,
                ),
                item(
                    1,
                    "Watch the auth worker deploy",
                    "acme-api",
                    "pkce-auth-security",
                    None,
                    Mark::Plain,
                ),
            ],
        },
        Day {
            label: "TUE 27".to_owned(),
            today: false,
            items: vec![item(
                2,
                "Compare the new ranking with last week",
                "acme-web",
                "search-rank-tune",
                None,
                Mark::Plain,
            )],
        },
        Day {
            label: "SAT 31 \u{b7} ABOUT".to_owned(),
            today: false,
            items: vec![item(
                3,
                "Read back the 33 people",
                "acme-workers",
                "figure-out-precisely-triggered",
                None,
                Mark::Plain,
            )],
        },
        Day {
            label: "NO DATE".to_owned(),
            today: false,
            items: vec![
                item(4, "Install the bot on the new repo", "acme-web", "heron", None, Mark::Decide),
                item(
                    5,
                    "Move the backfill to nightly",
                    "acme-workers",
                    "backfill-orders",
                    Some("parked"),
                    Mark::Parked,
                ),
            ],
        },
    ]
}

fn poofed() -> Vec<PoofedRow> {
    let rows = [
        ("lint-fix-tests", Some(2340), 360, "acme-api"),
        ("promo-test-copy", Some(2338), 780, "acme-web"),
        ("which-queue-retries", None, 120, "acme-workers"),
        ("rename-flags", Some(2331), 900, "acme-web"),
        ("trim-fixtures", Some(2329), 600, "acme-api"),
        ("answer-cache-ttl", None, 240, "acme-workers"),
        ("bump-serde", Some(2325), 1200, "acme-api"),
        ("dead-route-sweep", Some(2322), 1620, "acme-web"),
    ];
    rows.iter()
        .enumerate()
        .map(|(index, (name, pr, secs, place))| PoofedRow {
            index,
            name: (*name).to_owned(),
            outcome: pr.map_or_else(|| "answered".to_owned(), |n| format!("PR #{n}")),
            took: tidewatch_core::meeseeks::elapsed_text(*secs * 1000),
            summary: format!("Did {name}."),
            place: (*place).to_owned(),
            secs: *secs as u64,
        })
        .collect()
}

fn takeover() -> Card {
    let session = |id: &str, name: &str| crate::takeover::Row {
        key: Key::Stored { cli: Cli::Claude, id: id.to_owned() },
        cli_id: "claude".to_owned(),
        name: name.to_owned(),
        folder: "~/code/acme-web".to_owned(),
        host: "Terminal".to_owned(),
        level: "Auto edit".to_owned(),
        yolo: false,
        status: Status::Idle,
        offer: Offer::MoveHere,
    };
    Card {
        summary: "2 sessions \u{b7} 2 idle".to_owned(),
        can_move_all: true,
        rows: vec![session("a", "fix-flaky-spec"), session("b", "api-docs-pass")],
        ended: Vec::new(),
        ended_open: false,
    }
}
