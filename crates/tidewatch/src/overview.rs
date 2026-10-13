//! What the Overview lists: one row per agent tab, grouped by repo (or folder), with the tabs that
//! need you pulled out on top. Built from plain data so it takes any number of places; today a
//! window has one.

use std::path::Path;

use tidewatch_core::end_block::EndBlock;
use tidewatch_core::status::AgentStatus;

use crate::pane::Pane;
use crate::status_view::{Tone, tone};
use crate::tabs::Tabs;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    pub(crate) tab_id: u64,
    pub(crate) title: String,
    pub(crate) place: String,
    pub(crate) profile: String,
    pub(crate) agent_id: String,
    pub(crate) branch: String,
    pub(crate) status: AgentStatus,
    pub(crate) detail: Option<String>,
    pub(crate) end_block: Option<EndBlock>,
    pub(crate) elapsed: f64,
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

/// The agent tabs of one window as a place. Plain shells and the pinned main tab are not agents.
pub(crate) fn collect<T>(label: &str, dir: &Path, tabs: &Tabs<Pane<T>>, now: f64) -> PlaceInput {
    let folder = dir.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let rows = tabs
        .items()
        .iter()
        .filter_map(|tab| {
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
                agent_id: agent.run.agent_id.clone(),
                branch,
                status: agent.run.status().clone(),
                detail: agent.run.detail().map(str::to_owned),
                end_block: agent.run.end_block().cloned(),
                elapsed: agent.run.elapsed(now),
            })
        })
        .collect();
    PlaceInput { label: label.to_owned(), path: dir.display().to_string(), rows }
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
    use std::path::PathBuf;
    use tidewatch_core::status::Reason;
    use tidewatch_core::worktree::Worktree;

    fn row(id: u64, title: &str, status: AgentStatus) -> Row {
        Row {
            tab_id: id,
            title: title.into(),
            place: "web".into(),
            profile: "Builder".into(),
            agent_id: "claude".into(),
            branch: format!("agent/{title}"),
            status,
            detail: None,
            end_block: None,
            elapsed: 0.0,
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
        let input = collect("web", Path::new("/work/web"), &tabs, 5.0);
        assert_eq!(input.label, "web");
        assert_eq!(input.path, "/work/web");
        let branches: Vec<&str> = input.rows.iter().map(|r| r.branch.as_str()).collect();
        assert_eq!(branches, ["agent/brave-otter", "web"]);
        assert_eq!(titles(&input.rows), ["brave-otter", "Reviewer"]);
        assert_eq!(input.rows[0].elapsed, 5.0);
        assert_eq!(input.rows[0].status, AgentStatus::SettingUp);
    }
}
