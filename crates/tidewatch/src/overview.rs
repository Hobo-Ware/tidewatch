//! What the Overview lists: one row per agent tab, grouped by repo (or folder), with the tabs that
//! need you pulled out on top. Built from plain data, one place per repo the window holds.

use std::path::Path;

use egui::Color32;

use tidewatch_core::end_block::EndBlock;
use tidewatch_core::status::AgentStatus;

use crate::pane::Pane;
use crate::progress_model::{CardTag, Look};
use crate::progress_ring::tone_color;
use crate::status_view::{Tone, color, label, tone};
use crate::tabs::Tabs;

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
    /// What the tab's ledger says, when it has one.
    pub(crate) look: Option<Look>,
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
}

impl Counts {
    pub(crate) fn total(self) -> usize {
        self.setup + self.working + self.needs + self.done + self.failed
    }
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
        Tone::Needs | Tone::Shell => 4,
    }
}

/// Groups the rows: tabs that need you first (oldest tab first, as opened), then each place with
/// failed, working, setting up and done tabs in that order.
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
                title: tab.title.clone(),
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
                look: None,
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
    pub(crate) tag: String,
    pub(crate) tag_ink: Color32,
}

fn first_line(text: &str) -> String {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .map_or_else(String::new, |line| line.trim().to_owned())
}

pub(crate) fn card_view(row: &Row) -> CardView {
    if let Some(look) = &row.look {
        let tag_ink = match look.tag {
            CardTag::Question => crate::brand::NEEDS,
            _ => tone_color(look.ring.tone),
        };
        return CardView {
            fraction: look.ring.fraction,
            ring: tone_color(look.ring.tone),
            line: look.line.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_run::AgentRun;
    use crate::pane::{AgentTab, Body};
    use crate::workflow_run::TabRun;
    use std::path::PathBuf;
    use tidewatch_core::status::Reason;
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
            look: None,
        }
    }

    fn place(label: &str, rows: Vec<Row>) -> PlaceInput {
        PlaceInput { label: label.into(), path: format!("/work/{label}"), rows }
    }

    fn titles(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.title.as_str()).collect()
    }

    #[test]
    fn tabs_that_need_you_come_first_and_the_rest_sort_by_urgency() {
        let overview = build(vec![place(
            "web",
            vec![
                row(1, "done-a", AgentStatus::Done),
                row(2, "needs-a", AgentStatus::NeedsYou(Reason::Permission)),
                row(3, "working-a", AgentStatus::Working),
                row(4, "failed-a", AgentStatus::Failed("x".into())),
                row(5, "setup-a", AgentStatus::SettingUp),
                row(6, "needs-b", AgentStatus::NeedsYou(Reason::Quiet)),
            ],
        )]);
        assert_eq!(titles(&overview.needs), ["needs-a", "needs-b"]);
        assert_eq!(
            titles(&overview.places[0].rows),
            ["failed-a", "working-a", "setup-a", "done-a"]
        );
        assert_eq!(overview.counts, Counts { setup: 1, working: 1, needs: 2, done: 1, failed: 1 });
        assert_eq!(overview.counts.total(), 6);
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
        assert_eq!(overview.counts.total(), 0);
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
        let waiting =
            build(vec![place("web", vec![row(1, "a", AgentStatus::NeedsYou(Reason::Quiet))])]);
        assert!(is_ticking(&waiting));
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
        with.look = Some(look(&ledger));
        let view = card_view(&with);
        assert_eq!(view.line, "Build \u{b7} writing the validator");
        assert_eq!((view.tag.as_str(), view.fraction), ("Working", 0.5));
        assert_eq!(view.ring, crate::brand::WORKING);

        let mut question = Ledger::parse("# Goal\nx\n\n## Needs you\n- Pick\n");
        question.status.state = Some(tidewatch_core::progress::State::Done);
        with.look = Some(look(&question));
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
}
