//! The Coming up timeline as plain data: date labels, each over the cards of its items (pure).

use tidewatch_core::civil_date::Date;
use tidewatch_core::progress::Mark;

use crate::coming_up::{Group, Row};

const MONTHS: [&str; 12] =
    ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Urgency {
    Ahead,
    Today,
    Late,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Label {
    /// An item with a step being worked on.
    Now,
    /// `NOV 1` with `15 d` under it; `~NOV 7` for an about-date.
    Date { text: String, under: String, urgency: Urgency },
    /// Whatever waits on an event, a condition or the user.
    Later,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Look {
    Now,
    Plain,
    /// `on: <event>`: dashed and faded.
    On,
    /// `parked: <until>`: dashed and fainter.
    Parked,
    /// Only the user can decide it: amber.
    You,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Button {
    Decide,
    Open,
}

impl Button {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Decide => "Decide",
            Self::Open => "Open",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Card {
    /// Which of the rows it was built from.
    pub(crate) row: usize,
    pub(crate) title: String,
    pub(crate) tab: String,
    pub(crate) steps: Vec<Mark>,
    pub(crate) look: Look,
    pub(crate) button: Option<Button>,
    pub(crate) tip: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Block {
    pub(crate) label: Label,
    pub(crate) cards: Vec<Card>,
}

/// A row and the marks of its steps, when its tab is open and its ledger lists them.
pub(crate) struct Item<'a> {
    pub(crate) row: &'a Row,
    pub(crate) steps: Vec<Mark>,
}

fn date_label(date: Date, days: i64, about: bool) -> Label {
    let month = MONTHS[usize::from(date.month().clamp(1, 12)) - 1];
    let text = format!("{}{month} {}", if about { "~" } else { "" }, date.day());
    let (under, urgency) = match days {
        0 => ("today".to_owned(), Urgency::Today),
        d if d < 0 => (format!("{} d late", -d), Urgency::Late),
        d => (format!("{d} d"), Urgency::Ahead),
    };
    Label::Date { text, under, urgency }
}

fn look_of(group: Group, doing: bool) -> Look {
    match group {
        _ if doing => Look::Now,
        Group::On => Look::On,
        Group::Parked => Look::Parked,
        Group::You => Look::You,
        Group::Due | Group::Dated | Group::About | Group::Other => Look::Plain,
    }
}

fn button_of(group: Group) -> Option<Button> {
    match group {
        Group::You => Some(Button::Decide),
        Group::Due => Some(Button::Open),
        _ => None,
    }
}

fn card_of(index: usize, item: &Item<'_>, doing: bool) -> Card {
    let row = item.row;
    let tip = match &row.why {
        Some(why) => format!("{}\n{why}", row.what),
        None => row.what.clone(),
    };
    Card {
        row: index,
        title: row.what.clone(),
        tab: row.tab.clone(),
        steps: item.steps.clone(),
        look: look_of(row.group, doing),
        button: button_of(row.group),
        tip,
    }
}

/// The blocks of the timeline, top to bottom: what is being worked on now, then each dated item
/// under its own date, then every undated one under Later. Within each, the order of `items`.
pub(crate) fn blocks(items: &[Item<'_>]) -> Vec<Block> {
    let doing = |item: &Item<'_>| item.steps.contains(&Mark::Doing);
    let mut now = Vec::new();
    let mut dated = Vec::new();
    let mut later = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let card = card_of(index, item, doing(item));
        if card.look == Look::Now {
            now.push(card);
            continue;
        }
        match (item.row.when.date(), item.row.days) {
            (Some(date), Some(days)) => {
                let about = item.row.group == Group::About;
                dated.push(Block { label: date_label(date, days, about), cards: vec![card] });
            }
            _ => later.push(card),
        }
    }
    let mut out = Vec::new();
    if !now.is_empty() {
        out.push(Block { label: Label::Now, cards: now });
    }
    out.extend(dated);
    if !later.is_empty() {
        out.push(Block { label: Label::Later, cards: later });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coming_up::coming_up;
    use tidewatch_core::progress::Ledger;
    use tidewatch_core::schedule::{Entry, Index, Owner, Presence};

    fn today() -> Date {
        Date::new(2026, 10, 17).unwrap()
    }

    fn rows(scheduled: &str, presence: Presence) -> Vec<Row> {
        let owner = Owner {
            repo: "/work/web".into(),
            tab: "heron".into(),
            session_id: Some("s".into()),
            profile_id: Some("builder".into()),
        };
        let ledger = Ledger::parse(&format!("# Goal\nG\n\n## Scheduled\n{scheduled}"));
        let mut index = Index::default();
        index.set(Entry::of(owner, &ledger, presence));
        coming_up(today(), &index)
    }

    fn built(scheduled: &str, steps: &[Vec<Mark>]) -> Vec<Block> {
        let rows = rows(scheduled, Presence::Open);
        let items: Vec<Item<'_>> = rows
            .iter()
            .enumerate()
            .map(|(i, row)| Item { row, steps: steps.get(i).cloned().unwrap_or_default() })
            .collect();
        blocks(&items)
    }

    fn label(block: &Block) -> String {
        match &block.label {
            Label::Now => "NOW".into(),
            Label::Later => "LATER".into(),
            Label::Date { text, under, .. } => format!("{text} / {under}"),
        }
    }

    #[test]
    fn dated_items_get_their_date_and_days_and_undated_ones_share_later() {
        let blocks = built(
            "- [ ] 2026-11-01 -- Switch off\n- [ ] ~2026-11-07 -- Count apps\n- [ ] on: sync ships -- Labels\n- [ ] parked: later -- Stop writes\n- [ ] you -- Install the bot\n",
            &[],
        );
        let seen: Vec<_> = blocks.iter().map(label).collect();
        assert_eq!(seen, ["NOV 1 / 15 d", "~NOV 7 / 21 d", "LATER"]);
        let later = &blocks[2].cards;
        let looks: Vec<_> =
            later.iter().map(|card| (card.title.as_str(), card.look, card.button)).collect();
        assert_eq!(
            looks,
            [
                ("Labels", Look::On, None),
                ("Stop writes", Look::Parked, None),
                ("Install the bot", Look::You, Some(Button::Decide)),
            ]
        );
    }

    #[test]
    fn a_step_being_worked_on_pulls_its_item_up_to_now_whatever_its_date() {
        let steps = [vec![Mark::Done, Mark::Doing], vec![], vec![Mark::Todo]];
        let blocks = built(
            "- [ ] 2026-10-20 -- Soon\n- [ ] 2026-11-01 -- Later one\n- [ ] on: x -- Event\n",
            &steps,
        );
        assert_eq!(blocks.iter().map(label).collect::<Vec<_>>(), ["NOW", "NOV 1 / 15 d", "LATER"]);
        assert_eq!(blocks[0].cards[0].title, "Soon");
        assert_eq!(blocks[0].cards[0].look, Look::Now);
        assert_eq!(blocks[0].cards[0].steps, [Mark::Done, Mark::Doing]);
    }

    #[test]
    fn due_and_late_items_say_so_under_their_date_and_offer_open() {
        let blocks = built("- [ ] 2026-10-10 -- Late one\n- [ ] 2026-10-17 -- Today one\n", &[]);
        assert_eq!(
            blocks.iter().map(label).collect::<Vec<_>>(),
            ["OCT 10 / 7 d late", "OCT 17 / today"]
        );
        assert!(blocks.iter().all(|block| block.cards[0].button == Some(Button::Open)));
        let urgencies: Vec<_> = blocks
            .iter()
            .map(|block| match &block.label {
                Label::Date { urgency, .. } => *urgency,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(urgencies, [Urgency::Late, Urgency::Today]);
    }

    #[test]
    fn an_unreadable_when_waits_under_later_and_nothing_scheduled_is_no_block() {
        let blocks = built("- [ ] someday -- Whenever\n", &[]);
        assert_eq!(blocks.iter().map(label).collect::<Vec<_>>(), ["LATER"]);
        assert_eq!(blocks[0].cards[0].look, Look::Plain);
        assert_eq!(built("", &[]), Vec::new());
    }

    #[test]
    fn a_card_remembers_its_row_its_tab_and_its_why() {
        let blocks =
            built("- [ ] 2026-11-01 -- First\n- [ ] 2026-11-02 -- Second -- because\n", &[]);
        let second = &blocks[1].cards[0];
        assert_eq!((second.row, second.tab.as_str()), (1, "heron"));
        assert_eq!(second.tip, "Second\nbecause");
        assert_eq!(blocks[0].cards[0].tip, "First");
    }

    #[test]
    fn every_month_has_its_three_letters() {
        for (month, want) in MONTHS.iter().enumerate() {
            let date = Date::new(2026, month as u8 + 1, 5).unwrap();
            let Label::Date { text, .. } = date_label(date, 3, false) else { panic!() };
            assert_eq!(text, format!("{want} 5"));
        }
    }
}
