//! The four stages every session moves through, and how the phases of any ledger fall under them
//! (pure). An agent writes its own steps under the four stage headings; a ledger from before that
//! (the old ten phases) is read under the same four.

use crate::kind::Kind;
use crate::progress::Phase;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Understand,
    Answer,
    Build,
    Pr,
    Done,
}

impl Stage {
    pub const ALL: [Self; 4] = [Self::Understand, Self::Build, Self::Pr, Self::Done];
    pub const READ_ONLY: [Self; 3] = [Self::Understand, Self::Answer, Self::Done];

    #[must_use]
    pub fn of_kind(kind: Kind) -> &'static [Self] {
        match kind {
            Kind::ReadOnly => &Self::READ_ONLY,
            Kind::Edit => &Self::ALL,
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Understand => "Understand",
            Self::Answer => "Answer",
            Self::Build => "Build",
            Self::Pr => "PR",
            Self::Done => "Done",
        }
    }
}

/// The stage a phase title belongs to: the four stage names, and the ten phases the old template
/// had. `None` for any other title.
#[must_use]
pub fn stage_of(title: &str) -> Option<Stage> {
    match title.trim().to_lowercase().as_str() {
        "understand" | "understand and decide" => Some(Stage::Understand),
        "answer" => Some(Stage::Answer),
        "build" | "gate" | "validate against production" | "preview" => Some(Stage::Build),
        "pr" | "review" => Some(Stage::Pr),
        "done" | "ship" | "verify live" | "close out" => Some(Stage::Done),
        _ => None,
    }
}

/// The stage a phase counts under for a task of `kind`: a read-only task has no Build or PR (that
/// work is its answer) and an edit has no Answer (that work is its build).
#[must_use]
pub fn stage_in(title: &str, kind: Kind) -> Option<Stage> {
    match (stage_of(title)?, kind) {
        (Stage::Build | Stage::Pr, Kind::ReadOnly) => Some(Stage::Answer),
        (Stage::Answer, Kind::Edit) => Some(Stage::Build),
        (stage, _) => Some(stage),
    }
}

/// What one bar of the progress panel stands for: a stage with every phase that maps onto it, or
/// a single phase whose title is none of the known ones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub stage: Option<Stage>,
    /// Indices into the ledger's checklist, in checklist order.
    pub phases: Vec<usize>,
}

/// The bars of a checklist, in the order their first phase appears.
#[must_use]
pub fn groups(checklist: &[Phase], kind: Kind) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for (index, phase) in checklist.iter().enumerate() {
        let stage = stage_in(&phase.title, kind);
        match groups.iter_mut().find(|group| stage.is_some() && group.stage == stage) {
            Some(group) => group.phases.push(index),
            None => groups.push(Group { stage, phases: vec![index] }),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::checklist_phases;

    #[test]
    fn every_known_title_maps_to_its_stage() {
        let cases = [
            ("Understand", Some(Stage::Understand)),
            ("Understand and decide", Some(Stage::Understand)),
            ("Build", Some(Stage::Build)),
            ("Gate", Some(Stage::Build)),
            ("Validate against production", Some(Stage::Build)),
            ("Preview", Some(Stage::Build)),
            ("PR", Some(Stage::Pr)),
            ("Review", Some(Stage::Pr)),
            ("Done", Some(Stage::Done)),
            ("Ship", Some(Stage::Done)),
            ("Verify live", Some(Stage::Done)),
            ("Close out", Some(Stage::Done)),
            ("  build ", Some(Stage::Build)),
            ("UNDERSTAND AND DECIDE", Some(Stage::Understand)),
            ("Deploy", None),
            ("Mine", None),
            ("", None),
        ];
        for (title, want) in cases {
            assert_eq!(stage_of(title), want, "{title:?}");
        }
    }

    #[test]
    fn the_four_stages_are_named_in_order() {
        let labels: Vec<_> = Stage::ALL.iter().map(|stage| stage.label()).collect();
        assert_eq!(labels, ["Understand", "Build", "PR", "Done"]);
        assert!(Stage::ALL.iter().all(|stage| stage_of(stage.label()) == Some(*stage)));
    }

    fn shape(text: &str) -> Vec<(Option<Stage>, Vec<usize>)> {
        groups(&checklist_phases(text), Kind::Edit)
            .into_iter()
            .map(|g| (g.stage, g.phases))
            .collect()
    }

    #[test]
    fn the_four_stages_are_four_groups() {
        let text = "### 1. Understand\n- [ ] a\n### 2. Build\n- [ ] b\n### 3. PR\n- [ ] c\n### 4. Done\n- [ ] d\n";
        assert_eq!(
            shape(text),
            [
                (Some(Stage::Understand), vec![0]),
                (Some(Stage::Build), vec![1]),
                (Some(Stage::Pr), vec![2]),
                (Some(Stage::Done), vec![3]),
            ]
        );
    }

    #[test]
    fn the_old_ten_phases_fall_under_the_four_stages() {
        let titles = [
            "Understand and decide",
            "Build",
            "Gate",
            "Validate against production",
            "Preview",
            "PR",
            "Review",
            "Ship",
            "Verify live",
            "Close out",
        ];
        let text = titles
            .iter()
            .enumerate()
            .map(|(i, title)| format!("### {}. {title}\n- [ ] x\n", i + 1))
            .collect::<Vec<_>>()
            .concat();
        assert_eq!(
            shape(&text),
            [
                (Some(Stage::Understand), vec![0]),
                (Some(Stage::Build), vec![1, 2, 3, 4]),
                (Some(Stage::Pr), vec![5, 6]),
                (Some(Stage::Done), vec![7, 8, 9]),
            ]
        );
    }

    #[test]
    fn a_read_only_task_has_understand_answer_and_done_and_an_edit_the_four() {
        let labels =
            |kind| Stage::of_kind(kind).iter().map(|stage| stage.label()).collect::<Vec<_>>();
        assert_eq!(labels(Kind::ReadOnly), ["Understand", "Answer", "Done"]);
        assert_eq!(labels(Kind::Edit), ["Understand", "Build", "PR", "Done"]);
        assert_eq!(stage_of("answer"), Some(Stage::Answer));
    }

    #[test]
    fn phases_fall_under_the_stages_of_the_kind_they_are_read_as() {
        let text = "### 1. Understand\n- [x] a\n### 2. Answer\n- [ ] b\n### 3. Build\n- [ ] c\n### 4. PR\n- [ ] d\n### 5. Done\n- [ ] e\n";
        let phases = checklist_phases(text);
        let shape = |kind| {
            groups(&phases, kind).into_iter().map(|g| (g.stage, g.phases)).collect::<Vec<_>>()
        };
        assert_eq!(
            shape(Kind::ReadOnly),
            [
                (Some(Stage::Understand), vec![0]),
                (Some(Stage::Answer), vec![1, 2, 3]),
                (Some(Stage::Done), vec![4]),
            ]
        );
        assert_eq!(
            shape(Kind::Edit),
            [
                (Some(Stage::Understand), vec![0]),
                (Some(Stage::Build), vec![1, 2]),
                (Some(Stage::Pr), vec![3]),
                (Some(Stage::Done), vec![4]),
            ]
        );
    }

    #[test]
    fn unknown_phases_stay_apart_and_known_ones_gather_around_the_first() {
        let text = "### 1. Plan\n- [ ] a\n### 2. Build\n- [ ] b\n### 3. Plan\n- [ ] c\n### 4. Gate\n- [ ] d\n";
        assert_eq!(
            shape(text),
            [(None, vec![0]), (Some(Stage::Build), vec![1, 3]), (None, vec![2])]
        );
        assert_eq!(shape(""), []);
    }
}
