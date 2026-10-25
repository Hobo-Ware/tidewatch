//! What the Overview lists: one row per agent tab, grouped by repo (or folder), with the tabs that
//! need you pulled out on top. Built from plain data, one place per repo the window holds.

use std::path::Path;

use egui::Color32;

use tidewatch_core::agents::Level;
use tidewatch_core::civil_date::Date;
use tidewatch_core::end_block::EndBlock;
use tidewatch_core::meeseeks::elapsed_text;
use tidewatch_core::meeseeks_log::Poofed;
use tidewatch_core::status::{AgentStatus, Reason};

use crate::coming_up::{Group, Row as Scheduled};
use crate::pane::{AgentTab, Pane};
use crate::progress_model::{CardTag, Look};
use crate::progress_ring::tone_color;
use crate::status_view::{Tone, color, label, tone};
use crate::tabs::Tabs;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AgentKind {
    Meeseeks,
    Yolo,
    Cli(String),
}

impl AgentKind {
    pub(crate) fn of(agent: &AgentTab) -> Self {
        if agent.meeseeks.is_some() {
            Self::Meeseeks
        } else if agent.launched.as_ref().is_some_and(|launched| launched.level == Level::Yolo) {
            Self::Yolo
        } else {
            Self::Cli(agent.run.agent_id.clone())
        }
    }

    pub(crate) fn label(&self) -> String {
        match self {
            Self::Meeseeks => "Meeseeks".to_owned(),
            Self::Yolo => "YOLO".to_owned(),
            Self::Cli(id) => crate::screen_kit::cli_short_name(id),
        }
    }

    pub(crate) fn ink(&self) -> Color32 {
        match self {
            Self::Meeseeks => crate::brand::VIOLET_TEXT,
            Self::Yolo => crate::brand::AMBER,
            Self::Cli(id) if id == "claude" => crate::brand::TEAL_TEXT,
            Self::Cli(id) if id == "codex" => crate::brand::BLUE_TEXT,
            Self::Cli(_) => crate::brand::SOFT,
        }
    }
}

/// The next scheduled item of an agent's tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Next {
    pub(crate) date: Option<Date>,
    pub(crate) days: Option<i64>,
    pub(crate) what: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    pub(crate) tab_id: u64,
    pub(crate) title: String,
    pub(crate) place: String,
    pub(crate) profile: String,
    /// Where a workflow tab is: `Ship a feature: Review 2/4`.
    pub(crate) workflow: Option<String>,
    pub(crate) agent_id: String,
    pub(crate) branch: String,
    pub(crate) status: AgentStatus,
    pub(crate) detail: Option<String>,
    pub(crate) end_block: Option<EndBlock>,
    pub(crate) elapsed: f64,
    /// The tab's pull request: `#42 checks failed`, with its tone.
    pub(crate) pr: Option<(tidewatch_core::pr_status::Tone, String)>,
    /// What the done check found, when it ran on the tab.
    pub(crate) done_check: Option<(tidewatch_core::pr_status::Tone, &'static str)>,
    /// What the tab's ledger says, when it has one.
    pub(crate) look: Option<Look>,
    /// The tab's leading job as `21% \u{b7} 1h 52m`, when it has one.
    pub(crate) job_line: Option<String>,
    /// The picture the agent drew for the tab, shown at the card's corner.
    pub(crate) avatar: Option<std::sync::Arc<tidewatch_core::avatar::Avatar>>,
    pub(crate) kind: AgentKind,
    /// `Picks the recommended answer in 3:40`, while a question of an agent that runs on its own waits.
    pub(crate) auto_pick: Option<String>,
    /// Seconds since the status last changed.
    pub(crate) idle_secs: f64,
    pub(crate) next: Option<Next>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlaceInput {
    pub(crate) label: String,
    pub(crate) path: String,
    pub(crate) rows: Vec<Row>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Place {
    pub(crate) label: String,
    pub(crate) path: String,
    pub(crate) rows: Vec<Row>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub(crate) setup: usize,
    pub(crate) working: usize,
    pub(crate) needs: usize,
    pub(crate) done: usize,
    pub(crate) failed: usize,
    pub(crate) stopped: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Overview {
    pub(crate) needs: Vec<Row>,
    pub(crate) places: Vec<Place>,
    pub(crate) counts: Counts,
}

fn rank(tone: Tone) -> u8 {
    match tone {
        Tone::Failed => 0,
        Tone::Working => 1,
        Tone::Setup => 2,
        Tone::Done => 3,
        Tone::Stopped => 4,
        Tone::Needs | Tone::Shell => 5,
    }
}

/// Groups the rows: tabs that need you first (oldest tab first, as opened), then each place with
/// failed, working, setting up, done and stopped tabs in that order.
pub(crate) fn build(places: Vec<PlaceInput>) -> Overview {
    let mut counts = Counts::default();
    let mut needs = Vec::new();
    let mut grouped = Vec::new();
    for place in places {
        let mut rows = Vec::new();
        for row in place.rows {
            match tone(&row.status) {
                Tone::Setup => counts.setup += 1,
                Tone::Working => counts.working += 1,
                Tone::Needs => counts.needs += 1,
                Tone::Done => counts.done += 1,
                Tone::Failed => counts.failed += 1,
                Tone::Stopped => counts.stopped += 1,
                Tone::Shell => continue,
            }
            if tone(&row.status) == Tone::Needs {
                needs.push(row);
            } else {
                rows.push(row);
            }
        }
        rows.sort_by_key(|row| rank(tone(&row.status)));
        grouped.push(Place { label: place.label, path: place.path, rows });
    }
    Overview { needs, places: grouped, counts }
}

/// The agent tabs of one repo of the window (a tab group) as a place. Plain shells and the pinned
/// main tab are not agents.
pub(crate) fn collect<T>(
    label: &str,
    dir: &Path,
    group: usize,
    tabs: &Tabs<Pane<T>>,
    now: f64,
) -> PlaceInput {
    let folder = dir.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let rows = tabs
        .in_group(group)
        .filter_map(|(_, tab)| {
            let agent = tab.pane.agent.as_ref()?;
            let branch = tab
                .pane
                .worktree
                .as_ref()
                .map_or_else(|| folder.clone(), |worktree| worktree.branch.clone());
            Some(Row {
                tab_id: tab.id,
                title: crate::tab_title::label(Some(agent), &tab.title),
                place: label.to_owned(),
                profile: agent.profile_name.clone(),
                workflow: agent.workflow.as_ref().map(|run| run.position()),
                agent_id: agent.run.agent_id.clone(),
                branch,
                status: agent.run.status().clone(),
                detail: agent.run.detail().map(str::to_owned),
                end_block: agent.run.end_block().cloned(),
                elapsed: agent.run.elapsed(now),
                pr: None,
                done_check: None,
                look: None,
                job_line: None,
                avatar: None,
                kind: AgentKind::of(agent),
                auto_pick: auto_pick_line(agent, now),
                idle_secs: agent.run.since_change(now),
                next: None,
            })
        })
        .collect();
    PlaceInput { label: label.to_owned(), path: dir.display().to_string(), rows }
}

/// What one Overview card draws, from the ledger when the tab has one and from its status when not.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CardView {
    pub(crate) fraction: f32,
    pub(crate) ring: Color32,
    pub(crate) line: String,
    pub(crate) line_ink: Color32,
    pub(crate) tag: String,
    pub(crate) tag_ink: Color32,
}

fn first_line(text: &str) -> String {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .map_or_else(String::new, |line| line.trim().to_owned())
}

pub(crate) fn card_view(row: &Row) -> CardView {
    let mut view = card_view_of(row);
    if let Some(line) = &row.job_line {
        view.line.clone_from(line);
        view.line_ink = crate::brand::TEAL;
    }
    view
}

fn card_view_of(row: &Row) -> CardView {
    if let Some(look) = &row.look {
        let tag_ink = match look.tag {
            CardTag::Question => crate::brand::NEEDS,
            _ => tone_color(look.ring.tone),
        };
        return CardView {
            fraction: look.ring.fraction,
            ring: tone_color(look.ring.tone),
            line: look.line.clone(),
            line_ink: crate::brand::SOFT,
            tag: look.tag.label(look.questions),
            tag_ink,
        };
    }
    let status_tone = tone(&row.status);
    let ink = color(status_tone).unwrap_or(crate::brand::MUTED);
    CardView {
        fraction: if status_tone == Tone::Done { 1.0 } else { 0.0 },
        ring: ink,
        line: row
            .end_block
            .as_ref()
            .and_then(|block| block.done.first())
            .map(|done| first_line(done))
            .or_else(|| row.detail.as_deref().map(first_line))
            .unwrap_or_default(),
        line_ink: crate::brand::SOFT,
        tag: label(status_tone).to_owned(),
        tag_ink: ink,
    }
}

/// `42s`, `4m 02s`, `1h 03m`.
pub(crate) fn fmt_elapsed(secs: f64) -> String {
    let total = secs.max(0.0) as u64;
    match total {
        0..60 => format!("{total}s"),
        60..3600 => format!("{}m {:02}s", total / 60, total % 60),
        _ => format!("{}h {:02}m", total / 3600, (total % 3600) / 60),
    }
}

/// True while any row's clock still runs, so the view needs a frame every second.
pub(crate) fn is_ticking(overview: &Overview) -> bool {
    overview
        .needs
        .iter()
        .chain(overview.places.iter().flat_map(|place| &place.rows))
        .any(|row| !matches!(tone(&row.status), Tone::Done | Tone::Failed))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PoofedRow {
    /// Where the entry stood in the list it was made from.
    pub(crate) index: usize,
    pub(crate) name: String,
    /// `PR #213`, or `answered` when the Meeseeks opened no pull request.
    pub(crate) outcome: String,
    pub(crate) took: String,
    pub(crate) summary: String,
    pub(crate) place: String,
    pub(crate) secs: u64,
}

pub(crate) fn poofed_rows(
    entries: &[Poofed],
    place_of: impl Fn(&Path) -> String,
) -> Vec<PoofedRow> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| PoofedRow {
            index,
            name: entry.name.clone(),
            outcome: entry.pr_label().unwrap_or_else(|| "answered".to_owned()),
            took: elapsed_text(i64::try_from(entry.duration_secs).unwrap_or(0) * 1000),
            summary: entry.summary.clone(),
            place: place_of(&entry.repo),
            secs: entry.duration_secs,
        })
        .collect()
}

/// `8 \u{b7} 1h 37m`: how many poofed today and the time they ran together.
pub(crate) fn poofed_header(rows: &[PoofedRow]) -> String {
    let total: u64 = rows.iter().map(|row| row.secs).sum();
    format!("{} \u{b7} {}", rows.len(), elapsed_text(i64::try_from(total).unwrap_or(0) * 1000))
}

pub(crate) const WORKING_CAP: usize = 6;
pub(crate) const POOFED_CAP: usize = 3;

/// The first `cap` of `items` unless `all`, and how many that leaves out.
pub(crate) fn capped<T>(items: &[T], cap: usize, all: bool) -> (&[T], usize) {
    if all || items.len() <= cap { (items, 0) } else { (&items[..cap], items.len() - cap) }
}

/// A stable color for a repo: the same name always gets the same one of the palette.
pub(crate) fn repo_ink(name: &str) -> Color32 {
    let hash = name.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
    });
    crate::brand::REPO_INKS[(hash % crate::brand::REPO_INKS.len() as u64) as usize]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RepoChip {
    pub(crate) label: String,
    pub(crate) count: usize,
}

/// What the screen draws: three lists and the repo chips, narrowed to one repo when asked.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Board {
    pub(crate) needs: Vec<Row>,
    pub(crate) working: Vec<Row>,
    pub(crate) idle: Vec<Row>,
    pub(crate) counts: Counts,
    pub(crate) chips: Vec<RepoChip>,
}

impl Board {
    pub(crate) fn total(&self) -> usize {
        self.needs.len() + self.working.len() + self.idle.len()
    }

    /// `12 agents \u{b7} 3 need you \u{b7} 5 working \u{b7} 4 idle`.
    pub(crate) fn summary(&self) -> String {
        let total = self.total();
        let agents = if total == 1 { "1 agent".to_owned() } else { format!("{total} agents") };
        format!(
            "{agents} \u{b7} {} need you \u{b7} {} working \u{b7} {} idle",
            self.needs.len(),
            self.working.len(),
            self.idle.len()
        )
    }
}

impl Overview {
    /// The lists of the screen for `repo` (a place label), or for every repo when `None`. A repo
    /// the window no longer holds shows everything.
    pub(crate) fn board(&self, repo: Option<&str>) -> Board {
        let chips: Vec<RepoChip> = self
            .places
            .iter()
            .map(|place| RepoChip {
                label: place.label.clone(),
                count: place.rows.len()
                    + self.needs.iter().filter(|row| row.place == place.label).count(),
            })
            .filter(|chip| chip.count > 0)
            .collect();
        let repo = repo.filter(|repo| self.places.iter().any(|place| place.label == *repo));
        let keep = |row: &&Row| repo.is_none_or(|repo| row.place == repo);
        let needs: Vec<Row> = self.needs.iter().filter(keep).cloned().collect();
        let rest = self.places.iter().flat_map(|place| &place.rows).filter(keep);
        let (mut working, mut idle) = (Vec::new(), Vec::new());
        let mut counts = Counts { needs: needs.len(), ..Counts::default() };
        for row in rest {
            match tone(&row.status) {
                Tone::Setup => {
                    counts.setup += 1;
                    working.push(row.clone());
                }
                Tone::Working => {
                    counts.working += 1;
                    working.push(row.clone());
                }
                Tone::Failed | Tone::Done | Tone::Stopped | Tone::Needs | Tone::Shell => {
                    match tone(&row.status) {
                        Tone::Failed => counts.failed += 1,
                        Tone::Stopped => counts.stopped += 1,
                        _ => counts.done += 1,
                    }
                    idle.push(row.clone());
                }
            }
        }
        idle.sort_by_key(|row| rank(tone(&row.status)));
        Board { needs, working, idle, counts, chips }
    }
}

fn idle_text(secs: f64) -> String {
    let secs = secs.max(0.0) as u64;
    match secs {
        0..3600 => format!("{}m", (secs / 60).max(1)),
        3600..172_800 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

fn day_text(date: Date) -> String {
    format!("{} {}", date.weekday().short(), date.day())
}

/// The second line of an idle card: how long it has been idle and what it does next, or when it
/// wakes when that is days away.
pub(crate) fn idle_line(row: &Row) -> String {
    let lead = match &row.status {
        AgentStatus::Failed(why) => return format!("Failed \u{b7} {}", first_line(why)),
        AgentStatus::Stopped => "Stopped".to_owned(),
        _ => format!("Idle {}", idle_text(row.idle_secs)),
    };
    let Some(next) = &row.next else { return format!("{lead} \u{b7} nothing scheduled") };
    match (next.days, next.date) {
        (Some(days), Some(date)) if days >= 2 => format!("Waiting until {}", day_text(date)),
        (Some(1), _) => format!("{lead} \u{b7} Next: tomorrow, {}", next.what),
        (Some(_), _) => format!("{lead} \u{b7} Next: today, {}", next.what),
        (None, _) => format!("{lead} \u{b7} Next: {}", next.what),
    }
}

/// The next item of one tab: the first open one that is not parked and is not the owner's to decide.
pub(crate) fn next_of<'a>(mut rows: impl Iterator<Item = &'a Scheduled>) -> Option<Next> {
    rows.find(|row| !matches!(row.group, Group::Parked | Group::You)).map(|row| Next {
        date: row.when.date(),
        days: row.days,
        what: row.what.clone(),
    })
}

fn auto_pick_line(agent: &AgentTab, now: f64) -> Option<String> {
    use crate::reader_rows::Kind;
    if !agent.autonomous() {
        return None;
    }
    let row = agent.reader.rows.pending_question()?;
    let left = agent.reader.auto.left(row.seq, now, crate::dev_reader::autopick_wait())?;
    let Kind::Question(question) = &row.kind else { return None };
    Some(crate::question_auto::line(left, &question.questions))
}

/// What the card's bottom line says about a question or a permission.
pub(crate) fn ask_tag(row: &Row, tag: &str) -> String {
    match &row.status {
        AgentStatus::NeedsYou(Reason::Permission) if row.look.is_none() => "permission".to_owned(),
        AgentStatus::NeedsYou(Reason::Question) if row.look.is_none() => "question".to_owned(),
        AgentStatus::NeedsYou(Reason::Input) if row.look.is_none() => "your message".to_owned(),
        AgentStatus::NeedsYou(Reason::Attention) if row.look.is_none() => {
            "rang the bell".to_owned()
        }
        _ => tag.to_owned(),
    }
}

/// The button of a card that needs you.
pub(crate) fn ask_button(row: &Row) -> &'static str {
    match &row.status {
        AgentStatus::NeedsYou(Reason::Permission) => "Review",
        _ => "Answer",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_run::AgentRun;
    use crate::pane::{AgentTab, Body};
    use crate::workflow_run::TabRun;
    use std::path::PathBuf;
    use tidewatch_core::worktree::Worktree;

    fn row(id: u64, title: &str, status: AgentStatus) -> Row {
        Row {
            tab_id: id,
            title: title.into(),
            place: "web".into(),
            profile: "Builder".into(),
            workflow: None,
            agent_id: "claude".into(),
            branch: format!("agent/{title}"),
            status,
            detail: None,
            end_block: None,
            elapsed: 0.0,
            pr: None,
            done_check: None,
            look: None,
            job_line: None,
            avatar: None,
            kind: AgentKind::Cli("claude".into()),
            auto_pick: None,
            idle_secs: 0.0,
            next: None,
        }
    }

    fn place(label: &str, rows: Vec<Row>) -> PlaceInput {
        PlaceInput { label: label.into(), path: format!("/work/{label}"), rows }
    }

    fn titles(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.title.as_str()).collect()
    }

    #[test]
    fn tabs_that_need_you_come_first_and_a_quiet_one_is_idle_not_needing_you() {
        let overview = build(vec![place(
            "web",
            vec![
                row(1, "done-a", AgentStatus::Done),
                row(2, "needs-a", AgentStatus::NeedsYou(Reason::Permission)),
                row(3, "working-a", AgentStatus::Working),
                row(4, "failed-a", AgentStatus::Failed("x".into())),
                row(5, "setup-a", AgentStatus::SettingUp),
                row(6, "needs-b", AgentStatus::NeedsYou(Reason::Quiet)),
                row(7, "stopped-a", AgentStatus::Stopped),
            ],
        )]);
        assert_eq!(titles(&overview.needs), ["needs-a"]);
        assert_eq!(
            titles(&overview.places[0].rows),
            ["failed-a", "working-a", "setup-a", "done-a", "needs-b", "stopped-a"]
        );
        assert_eq!(
            overview.counts,
            Counts { setup: 1, working: 1, needs: 1, done: 2, failed: 1, stopped: 1 }
        );
    }

    #[test]
    fn several_places_keep_their_own_groups_and_share_the_needs_list() {
        let overview = build(vec![
            place("web", vec![row(1, "a", AgentStatus::Working)]),
            place("api", vec![row(2, "b", AgentStatus::NeedsYou(Reason::Input))]),
        ]);
        assert_eq!(overview.places.len(), 2);
        assert_eq!(titles(&overview.places[0].rows), ["a"]);
        assert_eq!(overview.places[1].rows, []);
        assert_eq!(titles(&overview.needs), ["b"]);
    }

    #[test]
    fn a_plain_shell_is_not_listed() {
        let overview = build(vec![place("web", vec![row(1, "main", AgentStatus::Shell)])]);
        assert_eq!(overview.counts, Counts::default());
        assert_eq!(overview.places[0].rows, []);
    }

    #[test]
    fn elapsed_time_reads_in_seconds_minutes_and_hours() {
        let cases = [
            (0.0, "0s"),
            (42.9, "42s"),
            (60.0, "1m 00s"),
            (242.0, "4m 02s"),
            (3599.0, "59m 59s"),
            (3780.0, "1h 03m"),
            (-5.0, "0s"),
        ];
        for (secs, want) in cases {
            assert_eq!(fmt_elapsed(secs), want, "{secs}");
        }
    }

    #[test]
    fn only_a_list_with_a_live_tab_ticks() {
        let live = build(vec![place("web", vec![row(1, "a", AgentStatus::Working)])]);
        assert!(is_ticking(&live));
        let asking =
            build(vec![place("web", vec![row(1, "a", AgentStatus::NeedsYou(Reason::Input))])]);
        assert!(is_ticking(&asking));
        let finished = build(vec![place(
            "web",
            vec![row(1, "a", AgentStatus::Done), row(2, "b", AgentStatus::Failed("x".into()))],
        )]);
        assert!(!is_ticking(&finished));
    }

    #[test]
    fn collecting_lists_agent_tabs_with_their_branch_or_folder() {
        let worktree = Worktree {
            repo: PathBuf::from("/work/web"),
            path: PathBuf::from("/home/.tidewatch/web-abc/brave-otter"),
            branch: "agent/brave-otter".into(),
        };
        let agent = |name: &str| {
            Some(AgentTab::new(name.into(), AgentRun::setting_up("claude".into(), 0.0)))
        };
        let mut tabs: Tabs<Pane<u32>> = Tabs::new(Pane::shell(0));
        tabs.push(
            "brave-otter".into(),
            Pane { body: Body::Shell(1), worktree: Some(worktree), agent: agent("Builder") },
        );
        tabs.push("shell 1".into(), Pane::shell(2));
        tabs.push(
            "Reviewer".into(),
            Pane { body: Body::Shell(3), worktree: None, agent: agent("Reviewer") },
        );
        let input = collect("web", Path::new("/work/web"), 0, &tabs, 5.0);
        assert_eq!(input.label, "web");
        assert_eq!(input.path, "/work/web");
        let branches: Vec<&str> = input.rows.iter().map(|r| r.branch.as_str()).collect();
        assert_eq!(branches, ["agent/brave-otter", "web"]);
        assert_eq!(titles(&input.rows), ["brave-otter", "Reviewer"]);
        assert_eq!(input.rows[0].elapsed, 5.0);
        assert_eq!(input.rows[0].status, AgentStatus::SettingUp);
    }

    #[test]
    fn a_row_carries_the_work_of_its_tab_and_not_the_worktree_name() {
        let mut agent = AgentTab::new("Builder".into(), AgentRun::setting_up("claude".into(), 0.0));
        agent.run.set_goal(Some("Fix the login redirect"));
        let mut tabs: Tabs<Pane<u32>> = Tabs::new(Pane::shell(0));
        tabs.push(
            "brave-otter".into(),
            Pane { body: Body::Shell(1), worktree: None, agent: Some(agent) },
        );
        let input = collect("web", Path::new("/work/web"), 0, &tabs, 0.0);
        assert_eq!(titles(&input.rows), ["Fix the login redirect"]);
    }

    #[test]
    fn each_repo_of_the_window_is_a_place_with_its_own_agent_tabs() {
        let agent = |name: &str| {
            Some(AgentTab::new(name.into(), AgentRun::setting_up("claude".into(), 0.0)))
        };
        let mut tabs: Tabs<Pane<u32>> = Tabs::new(Pane::shell(0));
        tabs.push(
            "web-agent".into(),
            Pane { body: Body::Shell(1), worktree: None, agent: agent("A") },
        );
        let api = tabs.add_group(Pane::shell(10));
        tabs.push_in(
            api,
            "api-agent".into(),
            Pane { body: Body::Shell(2), worktree: None, agent: agent("B") },
        );
        let places = vec![
            collect("web", Path::new("/work/web"), 0, &tabs, 0.0),
            collect("api", Path::new("/work/api"), api, &tabs, 0.0),
        ];
        let overview = build(places);
        assert_eq!(overview.counts.setup, 2);
        let per_place: Vec<(&str, Vec<&str>)> = overview
            .places
            .iter()
            .map(|place| (place.label.as_str(), titles(&place.rows)))
            .collect();
        assert_eq!(per_place, [("web", vec!["web-agent"]), ("api", vec!["api-agent"])]);
    }

    #[test]
    fn a_workflow_tab_shows_which_stage_the_run_is_at() {
        use std::collections::BTreeMap;
        use std::time::Duration;
        use tidewatch_core::runner::{RunContext, Runner};
        use tidewatch_core::workflows::Workflows;
        let ship = Workflows::defaults().workflow.remove("ship").unwrap();
        let ctx = RunContext {
            task: "t".into(),
            base: "main".into(),
            branch: "b".into(),
            test_command: None,
        };
        let mut run = TabRun::new(
            Runner::new(&ship, &BTreeMap::new(), ctx).unwrap(),
            None,
            Duration::from_secs(60),
        );
        run.start();
        let mut tab =
            AgentTab::new("Ship a feature".into(), AgentRun::setting_up("claude".into(), 0.0));
        tab.workflow = Some(Box::new(run));
        let mut tabs: Tabs<Pane<u32>> = Tabs::new(Pane::shell(0));
        tabs.push(
            "brave-otter".into(),
            Pane { body: Body::Shell(1), worktree: None, agent: Some(tab) },
        );
        tabs.push(
            "plain".into(),
            Pane {
                body: Body::Shell(2),
                worktree: None,
                agent: Some(AgentTab::new(
                    "Builder".into(),
                    AgentRun::setting_up("claude".into(), 0.0),
                )),
            },
        );
        let rows = collect("web", Path::new("/work/web"), 0, &tabs, 5.0).rows;
        assert_eq!(rows[0].workflow.as_deref(), Some("Ship a feature: Work 1/4"));
        assert_eq!(rows[1].workflow, None);
    }

    #[test]
    fn a_card_reads_its_ledger_and_falls_back_to_the_status() {
        use crate::progress_model::look;
        use tidewatch_core::progress::Ledger;
        let ledger = Ledger::parse(
            "# Goal\nx\n\n## Status\nphase: 2 Build\nstate: working\nnote: writing the validator\n\n## Checklist\n### 1. A\n- [x] a\n### 2. Build\n- [~] b\n",
        );
        let mut with = row(1, "a", AgentStatus::Working);
        with.look = Some(look(&ledger, false));
        let view = card_view(&with);
        assert_eq!(view.line, "Build \u{b7} writing the validator");
        assert_eq!((view.tag.as_str(), view.fraction), ("Working", 0.5));
        assert_eq!(view.ring, crate::brand::WORKING);

        let mut question = Ledger::parse("# Goal\nx\n\n## Needs you\n- Pick\n");
        question.status.state = Some(tidewatch_core::progress::State::Done);
        with.look = Some(look(&question, false));
        let view = card_view(&with);
        assert_eq!((view.tag.as_str(), view.tag_ink), ("1 question", crate::brand::NEEDS));

        let mut bare = row(2, "b", AgentStatus::NeedsYou(Reason::Permission));
        bare.detail = Some("\nBash: touch hello.txt\nmore".into());
        let view = card_view(&bare);
        assert_eq!(view.line, "Bash: touch hello.txt");
        assert_eq!((view.tag.as_str(), view.fraction), ("Needs you", 0.0));
        assert_eq!(card_view(&row(3, "c", AgentStatus::Done)).fraction, 1.0);
        let mut done = row(5, "d", AgentStatus::Done);
        done.detail = Some("ignored".into());
        done.end_block =
            Some(EndBlock { done: vec!["Committed ec84557".into()], ..EndBlock::default() });
        assert_eq!(card_view(&done).line, "Committed ec84557");
    }

    #[test]
    fn a_job_line_takes_the_status_line_in_teal_and_leaves_the_rest() {
        let mut working = row(1, "a", AgentStatus::Working);
        working.detail = Some("thinking".into());
        let plain = card_view(&working);
        assert_eq!((plain.line.as_str(), plain.line_ink), ("thinking", crate::brand::SOFT));
        working.job_line = Some("21% \u{b7} 1h 52m".into());
        let view = card_view(&working);
        assert_eq!((view.line.as_str(), view.line_ink), ("21% \u{b7} 1h 52m", crate::brand::TEAL));
        assert_eq!((view.tag, view.fraction), (plain.tag, plain.fraction));
    }

    fn two_repos() -> Overview {
        let mut web = vec![
            row(1, "w1", AgentStatus::Working),
            row(2, "w2", AgentStatus::NeedsYou(Reason::Quiet)),
            row(3, "w3", AgentStatus::Done),
        ];
        let mut api = vec![
            row(4, "a1", AgentStatus::Working),
            row(5, "a2", AgentStatus::SettingUp),
            row(6, "a3", AgentStatus::Failed("boom".into())),
            row(7, "a4", AgentStatus::NeedsYou(Reason::Permission)),
        ];
        for r in &mut web {
            r.place = "web".into();
        }
        for r in &mut api {
            r.place = "api".into();
        }
        build(vec![place("web", web), place("api", api)])
    }

    #[test]
    fn the_board_has_chips_with_counts_and_a_quiet_agent_in_idle() {
        let board = two_repos().board(None);
        assert_eq!(titles(&board.needs), ["a4"]);
        assert_eq!(titles(&board.working), ["w1", "a1", "a2"]);
        assert_eq!(titles(&board.idle), ["a3", "w2", "w3"]);
        let chips: Vec<_> = board.chips.iter().map(|c| (c.label.as_str(), c.count)).collect();
        assert_eq!(chips, [("web", 3), ("api", 4)]);
        assert_eq!(board.total(), 7);
        assert_eq!(board.summary(), "7 agents \u{b7} 1 need you \u{b7} 3 working \u{b7} 3 idle");
    }

    #[test]
    fn a_repo_filter_narrows_every_list_and_the_summary_but_not_the_chips() {
        let board = two_repos().board(Some("api"));
        assert_eq!(titles(&board.needs), ["a4"]);
        assert_eq!(titles(&board.working), ["a1", "a2"]);
        assert_eq!(titles(&board.idle), ["a3"]);
        assert_eq!(board.chips.len(), 2);
        assert_eq!(board.summary(), "4 agents \u{b7} 1 need you \u{b7} 2 working \u{b7} 1 idle");
        let web = two_repos().board(Some("web"));
        assert_eq!(
            (web.needs.len(), titles(&web.working), titles(&web.idle)),
            (0, vec!["w1"], vec!["w2", "w3"])
        );
        assert_eq!(two_repos().board(Some("gone")), two_repos().board(None));
    }

    #[test]
    fn working_shows_six_until_show_all() {
        let rows: Vec<u32> = (0..8).collect();
        assert_eq!(capped(&rows, WORKING_CAP, false), (&rows[..6], 2));
        assert_eq!(capped(&rows, WORKING_CAP, true), (&rows[..], 0));
        assert_eq!(capped(&rows[..6], WORKING_CAP, false), (&rows[..6], 0));
        assert_eq!(capped(&rows, POOFED_CAP, false), (&rows[..3], 5));
    }

    #[test]
    fn a_repo_gets_the_same_color_every_time_and_it_comes_from_the_palette() {
        assert_eq!(repo_ink("acme-api"), repo_ink("acme-api"));
        for name in ["acme-api", "acme-web", "acme-workers", "x", ""] {
            assert!(crate::brand::REPO_INKS.contains(&repo_ink(name)), "{name}");
        }
        let used: std::collections::HashSet<_> =
            (0..40).map(|n| repo_ink(&format!("repo-{n}"))).collect();
        assert!(used.len() >= 4);
    }

    #[test]
    fn an_idle_line_says_how_long_and_what_comes_next() {
        let date = |d| Date::new(2026, 10, d).unwrap();
        let next = |days: Option<i64>, day: Option<u8>, what: &str| {
            Some(Next { date: day.map(date), days, what: what.into() })
        };
        let mut idle = row(1, "a", AgentStatus::NeedsYou(Reason::Quiet));
        idle.idle_secs = 3.0 * 3600.0 + 120.0;
        assert_eq!(idle_line(&idle), "Idle 3h \u{b7} nothing scheduled");
        idle.next = next(Some(0), Some(26), "rerun the check");
        assert_eq!(idle_line(&idle), "Idle 3h \u{b7} Next: today, rerun the check");
        idle.next = next(Some(1), Some(27), "rerun the check");
        assert_eq!(idle_line(&idle), "Idle 3h \u{b7} Next: tomorrow, rerun the check");
        idle.next = next(Some(5), Some(31), "read back");
        assert_eq!(idle_line(&idle), "Waiting until Sat 31");
        idle.next = next(None, None, "after the deploy");
        assert_eq!(idle_line(&idle), "Idle 3h \u{b7} Next: after the deploy");
        idle.status = AgentStatus::Done;
        idle.idle_secs = 12.0 * 60.0;
        idle.next = None;
        assert_eq!(idle_line(&idle), "Idle 12m \u{b7} nothing scheduled");
        idle.status = AgentStatus::Failed("exited with code 2\nmore".into());
        assert_eq!(idle_line(&idle), "Failed \u{b7} exited with code 2");
        idle.status = AgentStatus::Stopped;
        assert_eq!(idle_line(&idle), "Stopped \u{b7} nothing scheduled");
    }

    #[test]
    fn idle_time_reads_in_minutes_hours_and_days() {
        let cases = [
            (5.0, "1m"),
            (59.0 * 60.0, "59m"),
            (3600.0, "1h"),
            (47.0 * 3600.0, "47h"),
            (3.0 * 86_400.0, "3d"),
        ];
        for (secs, want) in cases {
            assert_eq!(idle_text(secs), want, "{secs}");
        }
    }

    #[test]
    fn the_next_item_skips_parked_and_owner_items() {
        use crate::coming_up::coming_up;
        use tidewatch_core::progress::Ledger;
        use tidewatch_core::schedule::{Entry, Index, Owner, Presence};
        let mut index = Index::default();
        let owner = Owner {
            repo: "/work/web".into(),
            tab: "otter".into(),
            session_id: None,
            profile_id: None,
        };
        let ledger = Ledger::parse(
            "# Goal\nG\n\n## Scheduled\n- [ ] you -- decide\n- [ ] parked: later -- parked one\n- [ ] 2026-10-30 -- dated\n",
        );
        index.set(Entry::of(owner, &ledger, Presence::Open));
        let today = Date::new(2026, 10, 26).unwrap();
        let rows = coming_up(today, &index);
        let next = next_of(rows.iter()).unwrap();
        assert_eq!((next.what.as_str(), next.days), ("dated", Some(4)));
        assert_eq!(next_of(rows.iter().filter(|r| r.what != "dated")), None);
    }

    #[test]
    fn the_poofed_header_counts_and_adds_up_the_time() {
        let poofed = |secs: u64| PoofedRow {
            index: 0,
            name: "n".into(),
            outcome: "answered".into(),
            took: String::new(),
            summary: String::new(),
            place: "web".into(),
            secs,
        };
        assert_eq!(poofed_header(&[poofed(3000), poofed(2820)]), "2 \u{b7} 1h 37m");
        assert_eq!(poofed_header(&[]), "0 \u{b7} 0m");
    }

    #[test]
    fn a_poofed_row_names_the_pull_request_or_says_answered() {
        let mut with_pr = Poofed::bare(
            "lint-fix",
            "Did it.",
            Some("https://x/pull/213"),
            360,
            Date::new(2026, 10, 26).unwrap(),
        );
        with_pr.repo = "/work/web".into();
        let without = Poofed::bare("ask", "Answered.", None, 60, Date::new(2026, 10, 26).unwrap());
        let rows = poofed_rows(&[with_pr, without], |repo| {
            repo.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        });
        assert_eq!(
            rows.iter()
                .map(|r| (r.outcome.as_str(), r.took.as_str(), r.place.as_str()))
                .collect::<Vec<_>>(),
            [("PR #213", "6m", "web"), ("answered", "1m", "")]
        );
    }

    #[test]
    fn the_card_button_reviews_a_permission_and_answers_the_rest() {
        assert_eq!(ask_button(&row(1, "a", AgentStatus::NeedsYou(Reason::Permission))), "Review");
        assert_eq!(ask_button(&row(1, "a", AgentStatus::NeedsYou(Reason::Question))), "Answer");
        assert_eq!(ask_button(&row(1, "a", AgentStatus::NeedsYou(Reason::Input))), "Answer");
    }
}
