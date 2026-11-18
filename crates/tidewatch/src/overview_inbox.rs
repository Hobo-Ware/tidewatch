//! The Needs you inbox of the Overview (pure): one entry per thing the owner has to do or decide,
//! built from the agents that ask, the ones that failed, the steps only the owner can do and the
//! rail's items for the owner. The same ask from several agents is one entry.

use std::cmp::Ordering;

use tidewatch_core::panel_view::Tone;

use crate::agenda::{Day, Mark};
use crate::overview::{Board, Row, ask_button, card_view, fmt_elapsed, idle_line, needs_line};
use crate::progress_model::Ask;

/// What kind of ask an entry is: it sets the icon tile and the left edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Failure,
    DoThis,
    Decision,
}

/// An agent behind an entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Who {
    pub(crate) tab_id: Option<u64>,
    pub(crate) name: String,
    /// The agent cannot go on until the owner answers.
    pub(crate) holds_up: bool,
}

/// Where an entry's Open button goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Go {
    Tab(u64),
    /// The row of Coming up the ask was made from, which opens its tab even when it is closed.
    Scheduled(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// The options of the tab's one open question, answered through the question card's function.
    Inline {
        tab_id: u64,
        ask: Ask,
    },
    Open {
        label: &'static str,
        go: Go,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    pub(crate) kind: Kind,
    pub(crate) text: String,
    pub(crate) who: Vec<Who>,
    /// Seconds the ask has waited, when the app knows.
    pub(crate) waited: Option<f64>,
    pub(crate) answer: Answer,
}

fn every_row(board: &Board) -> impl Iterator<Item = &Row> {
    board.needs.iter().chain(&board.working).chain(&board.waiting).chain(&board.idle)
}

fn who_of(row: &Row) -> Who {
    Who {
        tab_id: Some(row.tab_id),
        name: row.title.clone(),
        holds_up: matches!(row.tone, Tone::Needs | Tone::Blocked | Tone::Waiting),
    }
}

fn open(row: &Row, label: &'static str) -> Answer {
    Answer::Open { label, go: Go::Tab(row.tab_id) }
}

fn from_needs(row: &Row) -> Vec<Entry> {
    let (kind, asked) = (
        if row.tone == Tone::Blocked { Kind::Failure } else { Kind::Decision },
        row.look.as_ref().map_or_default(|look| look.asked.as_slice()),
    );
    let entry = |text: String, answer: Answer| Entry {
        kind,
        text,
        who: vec![who_of(row)],
        waited: Some(row.idle_secs),
        answer,
    };
    if asked.len() > 1 {
        return asked.iter().map(|text| entry(text.clone(), open(row, ask_button(row)))).collect();
    }
    match row.look.as_ref().and_then(|look| look.ask.clone()) {
        Some(ask) => {
            let answer = Answer::Inline { tab_id: row.tab_id, ask: ask.clone() };
            vec![entry(ask.text, answer)]
        }
        None => vec![entry(needs_line(row, &card_view(row).line), open(row, ask_button(row)))],
    }
}

fn normal(text: &str) -> String {
    text.trim().to_lowercase()
}

fn same_who(a: &Who, b: &Who) -> bool {
    match (a.tab_id, b.tab_id) {
        (Some(a), Some(b)) => a == b,
        _ => a.name == b.name,
    }
}

fn merge(found: Vec<Entry>) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for entry in found {
        let Some(known) = out
            .iter_mut()
            .find(|known| known.kind == entry.kind && normal(&known.text) == normal(&entry.text))
        else {
            out.push(entry);
            continue;
        };
        for who in entry.who {
            match known.who.iter_mut().find(|have| same_who(have, &who)) {
                Some(have) => have.holds_up |= who.holds_up,
                None => known.who.push(who),
            }
        }
        known.waited = known.waited.into_iter().chain(entry.waited).max_by(f64::total_cmp);
        if known.who.len() > 1
            && let Answer::Inline { tab_id, .. } = known.answer
        {
            known.answer = Answer::Open { label: "Open", go: Go::Tab(tab_id) };
        }
    }
    out
}

fn order(a: &Entry, b: &Entry) -> Ordering {
    let key = |entry: &Entry| entry.waited.unwrap_or(-1.0);
    (a.kind != Kind::Failure)
        .cmp(&(b.kind != Kind::Failure))
        .then_with(|| key(b).total_cmp(&key(a)))
}

/// The inbox of `board` and the rail's `days`: failures first, then the longest waiting. An answer
/// to a lone question goes through the question card's own function, so only an entry of one agent
/// shows its options; the rest open the tab.
pub(crate) fn build(board: &Board, days: &[Day]) -> Vec<Entry> {
    let mut found: Vec<Entry> = board.needs.iter().flat_map(from_needs).collect();
    found.extend(board.idle.iter().filter(|row| row.tone == Tone::Failed).map(|row| Entry {
        kind: Kind::Failure,
        text: idle_line(row),
        who: vec![who_of(row)],
        waited: Some(row.idle_secs),
        answer: open(row, "Open"),
    }));
    for row in every_row(board) {
        let who = who_of(row);
        let waited = who.holds_up.then_some(row.idle_secs);
        for text in row.look.iter().flat_map(|look| &look.you) {
            found.push(Entry {
                kind: Kind::DoThis,
                text: text.clone(),
                who: vec![who.clone()],
                waited,
                answer: open(row, "Open"),
            });
        }
    }
    for item in days.iter().flat_map(|day| &day.items).filter(|item| item.mark == Mark::Decide) {
        let live = item.tab_id.and_then(|id| every_row(board).find(|row| row.tab_id == id));
        found.push(Entry {
            kind: Kind::DoThis,
            text: item.what.trim().to_owned(),
            who: vec![live.map_or_else(
                || Who { tab_id: None, name: item.tab.clone(), holds_up: false },
                who_of,
            )],
            waited: live.filter(|row| who_of(row).holds_up).map(|row| row.idle_secs),
            answer: Answer::Open { label: "Open", go: Go::Scheduled(item.row) },
        });
    }
    let mut merged = merge(found);
    merged.sort_by(order);
    merged
}

/// `oldest waits 2h 10m · holds up 2 agents`; a part with no fact behind it is left out.
pub(crate) fn summary(entries: &[Entry]) -> String {
    let oldest = entries.iter().filter_map(|entry| entry.waited).max_by(f64::total_cmp);
    let mut held: Vec<u64> = entries
        .iter()
        .flat_map(|entry| &entry.who)
        .filter(|who| who.holds_up)
        .filter_map(|who| who.tab_id)
        .collect();
    held.sort_unstable();
    held.dedup();
    let mut parts = Vec::new();
    if let Some(oldest) = oldest {
        parts.push(format!("oldest waits {}", fmt_elapsed(oldest)));
    }
    match held.len() {
        0 => {}
        1 => parts.push("holds up 1 agent".to_owned()),
        n => parts.push(format!("holds up {n} agents")),
    }
    parts.join(" \u{b7} ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agenda::Item;
    use crate::overview::test_row;
    use crate::progress_model::look;
    use tidewatch_core::progress::Ledger;
    use tidewatch_core::progress_report::ProgressReport;
    use tidewatch_core::status::{AgentStatus, Reason};

    const MAC: &str = "Plug the laptop in: on battery";

    fn with_look(mut row: Row, source: &str) -> Row {
        let ledger = match serde_json::from_str::<serde_json::Value>(source) {
            Ok(args) => ProgressReport::check(args.as_object().expect("an object"), None)
                .expect("a valid report")
                .to_ledger(),
            Err(_) => Ledger::parse(source),
        };
        row.look = Some(look(&ledger, false));
        row
    }

    fn board(rows: Vec<Row>) -> Board {
        let mut board = Board {
            needs: Vec::new(),
            working: Vec::new(),
            waiting: Vec::new(),
            idle: Vec::new(),
            counts: crate::overview::Counts::default(),
            chips: Vec::new(),
        };
        for row in rows {
            match row.tone {
                Tone::Needs | Tone::Blocked => board.needs.push(row),
                Tone::Working => board.working.push(row),
                Tone::Waiting => board.waiting.push(row),
                _ => board.idle.push(row),
            }
        }
        board
    }

    fn you_step(text: &str) -> String {
        serde_json::json!({
            "goal": "x", "state": "working", "now": "building",
            "stages": [
                { "name": "Build", "kind": "build", "status": "active" },
                { "name": "Wrap up", "kind": "verify", "status": "todo" }
            ],
            "steps": [
                { "stage": "Build", "text": "Build", "status": "active" },
                { "stage": "Build", "text": text, "status": "todo", "when": { "you": true } }
            ],
        })
        .to_string()
    }

    fn working(id: u64, title: &str, markdown: &str) -> Row {
        let mut row = test_row(id, title, "acme-api", AgentStatus::Working);
        row.idle_secs = 120.0;
        with_look(row, markdown)
    }

    fn asking(id: u64, title: &str, secs: f64, markdown: &str) -> Row {
        let mut row = test_row(id, title, "acme-api", AgentStatus::NeedsYou(Reason::Question));
        row.idle_secs = secs;
        with_look(row, markdown)
    }

    fn failed(id: u64, title: &str, secs: f64) -> Row {
        let mut row = test_row(id, title, "garden", AgentStatus::Failed("exit 1".into()));
        row.idle_secs = secs;
        row
    }

    fn decide_item(row: usize, what: &str, tab: &str, tab_id: Option<u64>) -> Item {
        Item {
            row,
            what: what.into(),
            place: "acme-api".into(),
            tab: tab.into(),
            tab_id,
            note: None,
            mark: Mark::Decide,
            about: false,
            tip: what.into(),
        }
    }

    fn day(items: Vec<Item>) -> Day {
        Day { label: "NO DATE".into(), today: false, month: None, starts_month: false, items }
    }

    fn shape(entries: &[Entry]) -> Vec<(Kind, String, usize)> {
        entries.iter().map(|e| (e.kind, e.text.clone(), e.who.len())).collect()
    }

    #[test]
    fn the_same_step_for_you_from_two_agents_is_one_entry_with_both_agents() {
        let one = working(1, "album-covers", &you_step(MAC));
        let two = working(2, "import-backfill", &you_step(&MAC.to_uppercase()));
        let entries = build(&board(vec![one, two]), &[]);
        assert_eq!(shape(&entries), [(Kind::DoThis, MAC.to_owned(), 2)]);
        let names: Vec<&str> = entries[0].who.iter().map(|who| who.name.as_str()).collect();
        assert_eq!(names, ["album-covers", "import-backfill"]);
    }

    #[test]
    fn a_rail_item_for_the_owner_merges_with_the_same_step_of_its_open_tab() {
        let row = working(1, "album-covers", &you_step(MAC));
        let days = [day(vec![decide_item(4, &format!("  {MAC} "), "album-covers", Some(1))])];
        let entries = build(&board(vec![row]), &days);
        assert_eq!(shape(&entries), [(Kind::DoThis, MAC.to_owned(), 1)]);
        assert_eq!(entries[0].answer, Answer::Open { label: "Open", go: Go::Tab(1) });
    }

    #[test]
    fn a_rail_item_of_a_closed_tab_opens_its_row_and_names_the_tab() {
        let days = [day(vec![decide_item(4, "Approve the invoice", "old-tab", None)])];
        let entries = build(&board(vec![]), &days);
        assert_eq!(entries[0].who[0].name, "old-tab");
        assert_eq!(entries[0].waited, None);
        assert_eq!(entries[0].answer, Answer::Open { label: "Open", go: Go::Scheduled(4) });
    }

    #[test]
    fn each_kind_of_ask_maps_to_its_kind_and_failures_lead_then_the_longest_wait() {
        let rows = vec![
            asking(
                1,
                "a",
                600.0,
                "# Goal\nx\n\n## Status\nstate: waiting\n\n## Needs you\n- Keep the old URLs?\n",
            ),
            asking(
                2,
                "b",
                7200.0,
                "# Goal\nx\n\n## Status\nstate: waiting\n\n## Needs you\n- Which loss?\n",
            ),
            failed(3, "c", 60.0),
            working(4, "d", &you_step("Plug it in")),
        ];
        let entries = build(&board(rows), &[]);
        assert_eq!(
            shape(&entries),
            [
                (Kind::Failure, "Failed \u{b7} exit 1".to_owned(), 1),
                (Kind::Decision, "Which loss?".to_owned(), 1),
                (Kind::Decision, "Keep the old URLs?".to_owned(), 1),
                (Kind::DoThis, "Plug it in".to_owned(), 1),
            ]
        );
    }

    #[test]
    fn a_lone_question_answers_inline_and_two_agents_asking_the_same_open_the_tab() {
        let markdown = "# Goal\nx\n\n## Status\nstate: waiting\n\n## Needs you\n- Retry or skip? -- options: Retry | Skip\n";
        let alone = build(&board(vec![asking(1, "a", 5.0, markdown)]), &[]);
        assert!(
            matches!(&alone[0].answer, Answer::Inline { tab_id: 1, ask } if ask.options.len() == 2)
        );
        let both =
            build(&board(vec![asking(1, "a", 5.0, markdown), asking(2, "b", 9.0, markdown)]), &[]);
        assert_eq!(shape(&both), [(Kind::Decision, "Retry or skip?".to_owned(), 2)]);
        assert_eq!(both[0].answer, Answer::Open { label: "Open", go: Go::Tab(1) });
        assert_eq!(both[0].waited, Some(9.0));
    }

    #[test]
    fn several_open_questions_are_one_entry_each_that_opens_the_tab() {
        let markdown = "# Goal\nx\n\n## Status\nstate: waiting\n\n## Needs you\n- Deploy failed twice: retry or skip?\n- Which client shows the missing item?\n";
        let entries = build(&board(vec![asking(7, "import-backfill", 5.0, markdown)]), &[]);
        let texts: Vec<&str> = entries.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(
            texts,
            ["Deploy failed twice: retry or skip?", "Which client shows the missing item?"]
        );
        assert!(
            entries.iter().all(|e| e.answer == Answer::Open { label: "Answer", go: Go::Tab(7) })
        );
    }

    #[test]
    fn the_summary_names_the_oldest_wait_and_the_agents_held_up_and_drops_what_has_no_fact() {
        let rows = vec![
            asking(
                1,
                "a",
                7800.0,
                "# Goal\nx\n\n## Status\nstate: waiting\n\n## Needs you\n- Which loss?\n",
            ),
            working(2, "b", &you_step("Plug it in")),
        ];
        let entries = build(&board(rows), &[]);
        assert_eq!(summary(&entries), "oldest waits 2h 10m \u{b7} holds up 1 agent");
        let days = [day(vec![decide_item(0, "Approve", "closed", None)])];
        assert_eq!(summary(&build(&board(vec![]), &days)), "");
    }
}
