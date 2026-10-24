//! The command palette's data: the command table plus the open tabs and repos, the profiles and the workflows,
//! a fuzzy filter over their labels, and the selection. The matcher follows Kelp's palette.
//! `palette_view` draws it.

use crate::commands::{Chord, Command, Group, Platform, TABLE};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Match {
    pub(crate) score: i32,
    pub(crate) positions: Vec<usize>,
}

pub(crate) fn fuzzy(query: &str, text: &str) -> Option<Match> {
    let query: Vec<char> = query.trim().to_lowercase().chars().collect();
    if query.is_empty() {
        return Some(Match { score: 0, positions: Vec::new() });
    }
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = text.to_lowercase().chars().collect();
    if lower.len() != chars.len() {
        return None;
    }
    let word_start = |i: usize| {
        i == 0
            || matches!(chars[i - 1], ' ' | '/' | '-' | '_' | '.' | ':' | '(')
            || (chars[i].is_uppercase() && chars[i - 1].is_lowercase())
    };
    let mut positions = Vec::with_capacity(query.len());
    let mut score = 0;
    let mut from = 0;
    for &wanted in &query {
        let contiguous = !positions.is_empty() && lower.get(from) == Some(&wanted);
        let found = if contiguous {
            from
        } else {
            (from..lower.len())
                .find(|&i| lower[i] == wanted && word_start(i))
                .or_else(|| (from..lower.len()).find(|&i| lower[i] == wanted))?
        };
        score += if found == 0 {
            15
        } else if word_start(found) {
            10
        } else {
            1
        };
        if positions.last().is_some_and(|&last| found == last + 1) {
            score += 6;
        }
        score -= (found - from).min(10) as i32;
        positions.push(found);
        from = found + 1;
    }
    let joined: String = query.iter().collect();
    let haystack: String = lower.iter().collect();
    if haystack.starts_with(&joined) {
        score += 25;
    } else if haystack.contains(&joined) {
        score += 10;
    }
    score -= (chars.len() as i32 / 8).min(10);
    Some(Match { score, positions })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Command(Command),
    Tab(usize),
    Repo(usize),
    OpenRepo,
    CloseRepo,
    Profile(String),
    Workflow(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Item {
    pub(crate) label: String,
    pub(crate) keys: String,
    pub(crate) kind: &'static str,
    pub(crate) target: Target,
    /// Why the row cannot run now. It is shown in the row and as its tooltip, and Enter skips it.
    pub(crate) disabled: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TabRow {
    pub(crate) title: String,
    pub(crate) parked: bool,
}

impl TabRow {
    pub(crate) fn new(title: impl Into<String>, parked: bool) -> Self {
        Self { title: title.into(), parked }
    }
}

/// What the app has open right now: the tabs of the repo on show (and which one is active), the
/// repos of the window (and which one is on show). Profile and workflow entries are `(id, name)`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Sources {
    pub(crate) tabs: Vec<TabRow>,
    pub(crate) active: usize,
    pub(crate) repos: Vec<String>,
    pub(crate) repo: usize,
    pub(crate) profiles: Vec<(String, String)>,
    pub(crate) workflows: Vec<(String, String)>,
    /// Why Build workflow from this session cannot run on the active tab; `None` when it can.
    pub(crate) build_workflow_blocked: Option<&'static str>,
}

impl Sources {
    fn blocked(&self, command: Command) -> Option<&'static str> {
        (command == Command::BuildWorkflow).then_some(self.build_workflow_blocked).flatten()
    }
}

fn group_kind(group: Group) -> &'static str {
    match group {
        Group::Tabs => "tabs",
        Group::View => "view",
        Group::App => "app",
        Group::Dialogs => "dialogs",
    }
}

pub(crate) fn items(sources: &Sources, platform: Platform) -> Vec<Item> {
    let several = sources.repos.len() > 1;
    let commands = TABLE
        .iter()
        .filter(|entry| entry.command.in_palette())
        .filter(|entry| several || !entry.command.needs_several_repos())
        .map(|entry| Item {
            label: entry.title.to_owned(),
            keys: entry.chord(platform).map_or_else(String::new, Chord::label),
            kind: group_kind(entry.group),
            target: Target::Command(entry.command),
            disabled: sources.blocked(entry.command),
        });
    let mut shown = 0;
    let mut tabs = Vec::new();
    for (i, tab) in sources.tabs.iter().enumerate() {
        shown += usize::from(!tab.parked);
        if i == sources.active {
            continue;
        }
        let (label, keys) = if tab.parked {
            (format!("Open parked tab: {}", tab.title), String::new())
        } else {
            (
                format!("Switch to tab {shown}: {}", tab.title),
                Command::SelectTab(shown - 1).keys_on(platform),
            )
        };
        tabs.push(Item { label, keys, kind: "tab", target: Target::Tab(i), disabled: None });
    }
    let repos =
        sources.repos.iter().enumerate().filter(|(i, _)| *i != sources.repo).map(|(i, label)| {
            Item {
                label: format!("Switch to repo: {label}"),
                keys: String::new(),
                kind: "repo",
                target: Target::Repo(i),
                disabled: None,
            }
        });
    let repo_menu = [
        Some(("Open repo...", Target::OpenRepo, Command::OpenRepo.keys_on(platform))),
        several.then_some(("Close repo", Target::CloseRepo, String::new())),
    ]
    .into_iter()
    .flatten()
    .map(|(label, target, keys)| Item {
        label: label.to_owned(),
        keys,
        kind: "repo",
        target,
        disabled: None,
    });
    let profiles = sources.profiles.iter().map(|(id, name)| Item {
        label: format!("New tab: {name}"),
        keys: String::new(),
        kind: "profile",
        target: Target::Profile(id.clone()),
        disabled: None,
    });
    let workflows = sources.workflows.iter().map(|(id, name)| Item {
        label: format!("Workflow: {name}"),
        keys: String::new(),
        kind: "workflow",
        target: Target::Workflow(id.clone()),
        disabled: None,
    });
    commands.chain(tabs).chain(repos).chain(repo_menu).chain(profiles).chain(workflows).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Hit {
    pub(crate) item: usize,
    /// The matched letters of the label, or of `alias` when that is what matched.
    pub(crate) positions: Vec<usize>,
    pub(crate) alias: Option<&'static str>,
}

/// Everything for an empty query, in list order. Otherwise the matches, best first; ties keep list order.
pub(crate) fn filter(items: &[Item], query: &str) -> Vec<Hit> {
    let mut hits: Vec<(i32, Hit)> = items
        .iter()
        .enumerate()
        .filter_map(|(item, entry)| {
            if let Some(m) = fuzzy(query, &entry.label) {
                return Some((m.score, Hit { item, positions: m.positions, alias: None }));
            }
            let alias = match entry.target {
                Target::Command(command) => command.alias()?,
                _ => return None,
            };
            let m = fuzzy(query, alias)?;
            Some((m.score, Hit { item, positions: m.positions, alias: Some(alias) }))
        })
        .collect();
    if !query.trim().is_empty() {
        hits.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    }
    hits.into_iter().map(|(_, hit)| hit).collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Palette {
    pub(crate) query: String,
    selected: usize,
    pub(crate) focus: bool,
}

impl Palette {
    pub(crate) fn open(query: &str) -> Self {
        Self { query: query.to_owned(), selected: 0, focus: true }
    }

    pub(crate) fn selected(&self, len: usize) -> usize {
        self.selected.min(len.saturating_sub(1))
    }

    pub(crate) fn select(&mut self, index: usize) {
        self.selected = index;
    }

    pub(crate) fn step(&mut self, down: bool, len: usize) {
        if len == 0 {
            return;
        }
        let now = self.selected(len);
        self.selected = if down { (now + 1) % len } else { (now + len - 1) % len };
    }

    pub(crate) fn typed(&mut self) {
        self.selected = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(query: &str, text: &str) -> i32 {
        fuzzy(query, text).map_or(i32::MIN, |m| m.score)
    }

    fn sources() -> Sources {
        Sources {
            tabs: vec![
                TabRow::new("main", false),
                TabRow::new("calm-comet", false),
                TabRow::new("brave-otter", false),
            ],
            active: 0,
            repos: vec!["web".into()],
            repo: 0,
            profiles: vec![("builder".into(), "Builder".into())],
            workflows: vec![("ship".into(), "Ship a feature".into())],
            build_workflow_blocked: None,
        }
    }

    fn labels(items: &[Item], hits: &[Hit]) -> Vec<String> {
        hits.iter().map(|hit| items[hit.item].label.clone()).collect()
    }

    #[test]
    fn a_prefix_beats_the_middle() {
        assert!(score("ne", "New tab") > score("ne", "Close tab and never"));
        assert!(score("set", "Settings") > score("set", "Reset focus"));
    }

    #[test]
    fn word_starts_beat_mid_word() {
        assert!(score("nt", "New tab") > score("nt", "Unmount"));
    }

    #[test]
    fn subsequences_match_case_insensitively_and_report_their_positions() {
        let m = fuzzy("NWT", "New worktree").unwrap();
        assert_eq!(m.positions, [0, 4, 8]);
        assert_eq!(fuzzy("xyz", "Settings"), None);
        assert_eq!(fuzzy("  ", "Settings").unwrap().positions, Vec::<usize>::new());
    }

    #[test]
    fn the_list_is_the_table_then_other_tabs_profiles_and_workflows() {
        let all = items(&sources(), Platform::Mac);
        let names: Vec<_> = all.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(
            names,
            [
                "New tab",
                "Close tab",
                "Move tab left",
                "Move tab right",
                "Park tab",
                "Overview",
                "Focus mode",
                "Interface bigger",
                "Interface smaller",
                "Interface size 100%",
                "Reader or raw terminal",
                "Terminal or last file",
                "Settings",
                "Keyboard shortcuts",
                "Build workflow from this session",
                "Move sessions to Tidewatch",
                "Amphetamine",
                "Switch to tab 2: calm-comet",
                "Switch to tab 3: brave-otter",
                "Open repo...",
                "New tab: Builder",
                "Workflow: Ship a feature",
            ]
        );
    }

    #[test]
    fn typing_cocaine_finds_amphetamine_and_names_the_alias_while_amph_does_not() {
        let all = items(&sources(), Platform::Mac);
        let row = all.iter().position(|item| item.label == "Amphetamine").unwrap();
        let by_alias = filter(&all, "cocaine");
        assert_eq!(by_alias.len(), 1);
        assert_eq!(by_alias[0].item, row);
        assert_eq!(by_alias[0].alias, Some("cocaine"));
        assert_eq!(by_alias[0].positions, [0, 1, 2, 3, 4, 5, 6]);
        let by_name = filter(&all, "amph");
        assert_eq!(by_name[0].item, row);
        assert_eq!(by_name[0].alias, None);
        assert_eq!(filter(&all, "").iter().filter(|hit| hit.alias.is_some()).count(), 0);
        assert!(filter(&all, "coca").iter().any(|hit| hit.item == row));
    }

    #[test]
    fn several_repos_add_the_repo_commands_and_a_row_for_each_other_repo() {
        let two = Sources { repos: vec!["web".into(), "api".into()], ..sources() };
        let all = items(&two, Platform::Mac);
        let names: Vec<_> = all.iter().map(|item| item.label.as_str()).collect();
        assert!(names.contains(&"Next repo") && names.contains(&"Previous repo"));
        assert!(names.contains(&"Switch to repo: api"));
        assert!(names.contains(&"Open repo...") && names.contains(&"Close repo"));
        assert!(!names.contains(&"Switch to repo: web"), "that one is on show");
        let row = all.iter().find(|item| item.label == "Next repo").unwrap();
        assert_eq!(row.keys, "Cmd+Ctrl+]");
        let one = items(&sources(), Platform::Mac);
        let kept: Vec<_> = one.iter().filter(|item| item.label.contains("repo")).collect();
        assert_eq!(kept.len(), 1, "only Open repo is left in a window with one repo");
        assert_eq!(kept[0].target, Target::OpenRepo);
    }

    #[test]
    fn rows_show_the_chord_of_the_platform() {
        let mac = items(&sources(), Platform::Mac);
        let linux = items(&sources(), Platform::Linux);
        assert_eq!(mac[0].keys, "Cmd+T");
        assert_eq!(linux[0].keys, "Ctrl+Shift+T");
        assert_eq!(mac[17].keys, "Cmd+2");
        assert_eq!(linux[17].keys, "Alt+2");
        assert_eq!((mac[19].label.as_str(), mac[19].keys.as_str()), ("Open repo...", "Cmd+O"));
        assert_eq!(linux[19].keys, "Ctrl+Shift+O");
    }

    #[test]
    fn build_workflow_is_a_row_without_a_chord_that_carries_the_reason_it_cannot_run() {
        let row = |sources: &Sources| {
            items(sources, Platform::Mac)
                .into_iter()
                .find(|item| item.target == Target::Command(Command::BuildWorkflow))
                .unwrap()
        };
        let ready = row(&sources());
        assert_eq!(
            (ready.label.as_str(), ready.keys.as_str()),
            ("Build workflow from this session", "")
        );
        assert_eq!(ready.disabled, None);
        let blocked = Sources { build_workflow_blocked: Some("agent is working"), ..sources() };
        assert_eq!(row(&blocked).disabled, Some("agent is working"));
        let others = items(&blocked, Platform::Mac).iter().filter(|i| i.disabled.is_some()).count();
        assert_eq!(others, 1, "only that row is ever disabled");
    }

    #[test]
    fn parked_tabs_are_listed_without_a_number_and_the_chords_count_the_shown_ones() {
        let tabs = ["main", "a", "b", "c", "d"];
        let parked = [false, false, true, false, true];
        let sources = Sources {
            tabs: tabs
                .iter()
                .zip(parked)
                .map(|(title, parked)| TabRow::new(*title, parked))
                .collect(),
            ..sources()
        };
        let rows: Vec<_> = items(&sources, Platform::Mac)
            .into_iter()
            .filter(|item| item.kind == "tab")
            .map(|item| (item.label, item.keys, item.target))
            .collect();
        assert_eq!(
            rows,
            [
                ("Switch to tab 2: a".to_owned(), "Cmd+2".to_owned(), Target::Tab(1)),
                ("Open parked tab: b".to_owned(), String::new(), Target::Tab(2)),
                ("Switch to tab 3: c".to_owned(), "Cmd+3".to_owned(), Target::Tab(3)),
                ("Open parked tab: d".to_owned(), String::new(), Target::Tab(4)),
            ]
        );
    }

    #[test]
    fn a_tab_past_the_ninth_has_no_chord() {
        let many = Sources {
            tabs: (0..12).map(|n| TabRow::new(format!("t{n}"), false)).collect(),
            ..sources()
        };
        let all = items(&many, Platform::Mac);
        let twelfth = all.iter().find(|item| item.target == Target::Tab(11)).unwrap();
        assert_eq!(twelfth.keys, "");
    }

    #[test]
    fn an_empty_query_keeps_list_order() {
        let all = items(&sources(), Platform::Mac);
        let hits = filter(&all, "");
        assert_eq!(hits.len(), all.len());
        assert!(hits.iter().enumerate().all(|(i, hit)| hit.item == i));
    }

    #[test]
    fn typing_narrows_and_ranks() {
        let all = items(&sources(), Platform::Mac);
        assert_eq!(labels(&all, &filter(&all, "set"))[0], "Settings");
        assert_eq!(labels(&all, &filter(&all, "otter")), ["Switch to tab 3: brave-otter"]);
        assert_eq!(labels(&all, &filter(&all, "ship")), ["Workflow: Ship a feature"]);
        assert_eq!(filter(&all, "zzzz"), Vec::<Hit>::new());
    }

    #[test]
    fn the_selection_wraps_and_stays_inside_a_shorter_list() {
        let mut palette = Palette::open("");
        palette.step(false, 3);
        assert_eq!(palette.selected(3), 2);
        palette.step(true, 3);
        assert_eq!(palette.selected(3), 0);
        palette.select(8);
        assert_eq!(palette.selected(2), 1);
        palette.typed();
        assert_eq!(palette.selected(5), 0);
        palette.step(true, 0);
        assert_eq!(palette.selected(0), 0);
    }

    #[test]
    fn opening_prefills_the_query_and_asks_for_focus_once() {
        let palette = Palette::open("tab");
        assert_eq!(palette.query, "tab");
        assert!(palette.focus);
    }
}
