//! The built-in workflows beyond Ship a feature, Fix a bug and Review a PR: six habits that keep
//! work honest. Each proves a claim before the next stage builds on it, ends with a review of your
//! own before anything leaves the worktree, and never pushes or tags (the delivery step does that).

use crate::agents::Level;
use crate::workflows::{Action, Gate, Stage, Workflow, default_stage};

pub(crate) const ROOT_CAUSE: &str = "rootcause";
pub(crate) const FINISH: &str = "finish";
pub(crate) const UI_PROOF: &str = "uiproof";
pub(crate) const PORT: &str = "port";
pub(crate) const RELEASE: &str = "release";
pub(crate) const REFACTOR: &str = "refactor";

fn stage(action: Action, agent: &str, model: Option<&str>, level: Level, prompt: &str) -> Stage {
    Stage { prompt: Some(prompt.to_owned()), ..default_stage(action, agent, model, level) }
}

fn review(prompt: &str, on_fail: usize) -> Stage {
    Stage { on_fail: Some(on_fail), ..stage(Action::Review, "gemini", None, Level::Plan, prompt) }
}

fn handoff(prompt: &str) -> Stage {
    stage(Action::Handoff, "claude", Some("haiku"), Level::Plan, prompt)
}

fn workflow(id: &str, name: &str, description: &str, stages: Vec<Stage>) -> Workflow {
    Workflow { id: id.into(), name: name.into(), description: description.into(), stages }
}

fn root_cause() -> Workflow {
    let hunt = Stage {
        approve: true,
        ..stage(
            Action::Reproduce,
            "claude",
            Some("opus"),
            Level::Edits,
            "{task}\n\nFind the root cause before you change anything.\n\
             1. Reproduce the symptom and note exactly how.\n\
             2. List the plausible causes, most likely first. Test each one against the code and \
             the logs or data, and rule them out one by one.\n\
             3. Write one test that fails because of the cause you found, not only because of the \
             symptom. Do not fix the bug yet. Do not push.\n\
             Report the cause, the evidence, and the causes you ruled out. If you could not prove \
             the cause, say it is suspected and name what would confirm it.",
        )
    };
    let fix = stage(
        Action::Fix,
        "claude",
        Some("sonnet"),
        Level::Edits,
        "Fix the cause the previous stage proved.\n\nWhat the previous stage found:\n\
         {previous_summary}\n\nMake the failing test pass with the smallest change at the cause, \
         not a patch over the symptom. Do not edit the test. If other places have the same flaw, \
         report them instead of changing them. Commit when the tests pass. Do not push.\n\n\
         Open findings:\n{findings}\n\nOutput of the last failing test run:\n{test_output}",
    );
    let check = review(
        "Review {base}..HEAD, the fix for: {task}\n\nCheck that the change removes the cause and \
         does not only hide the symptom, that the new test would fail without the fix, and that \
         nothing unrelated changed. Report each finding with file, line, blocking or nit, and a \
         one-line fix. Do not edit files.",
        1,
    );
    let summary = handoff(
        "Write a PR title and description for {base}..HEAD. State the cause as proven only if the \
         new test shows it. Otherwise write that it is suspected and name what would confirm it. \
         List the symptom, the cause, the fix, the test, and what was not verified.",
    );
    workflow(
        ROOT_CAUSE,
        "Hunt a root cause",
        "Find the cause, prove it with a failing test, fix it there, review the fix.",
        vec![hunt, fix, check, summary],
    )
}

fn finish() -> Workflow {
    let checks = stage(
        Action::Work,
        "claude",
        Some("sonnet"),
        Level::Edits,
        "{task}\n\nThis is a finish pass on the work in {base}..HEAD. Run the project's own \
         checks: formatter, linter, type check, tests and build, whichever exist. Fix what fails \
         with the smallest change and do not add features. Keep a list of every command you ran \
         and its result. Commit fixes in small steps. Do not push.\n\n\
         Output of the last failing test run:\n{test_output}",
    );
    let audit = review(
        "Audit whether {base}..HEAD does what was asked.\n\nThe ask:\n{task}\n\nWrite one line per \
         ask, side asks included. Check each against the real state of the code and the repo, \
         not against memory. Mark it done, blocked (name the blocker) or open. A claim that \
         something works needs a command output or a test that shows it. Report every open ask \
         and every unproven claim as a blocking finding with file, line and a one-line fix. \
         Report nits as nit. Do not edit files.",
        0,
    );
    let report = handoff(
        "Write the final report for {base}..HEAD. Start with what is still open, each item with \
         the one command or decision it needs, or write that nothing is open. Then three separate \
         lists: verified (with the check that proves it and how much it covered), not verified \
         (and why), and blocked. Do not claim a check you did not run. Say what each check \
         replaced, for example a mock in place of the real service.",
    );
    workflow(
        FINISH,
        "Finish with proof",
        "Run the checks, audit every ask against the real state, report verified and not verified.",
        vec![checks, audit, report],
    )
}

fn ui_proof() -> Workflow {
    let baseline = Stage {
        gate: Gate::Approve,
        ..stage(
            Action::Work,
            "claude",
            Some("sonnet"),
            Level::Edits,
            "{task}\n\nDo not change any code yet. Capture the baseline of the unchanged build. \
             Hold the inputs still: same data, window size, scale and theme, with clocks and \
             animations frozen. Render each affected screen twice and compare the two renders. The \
             gap between them is the noise floor. Save the images outside the repo and list them \
             with the command that made each one. Report the noise floor per screen. Do not push.",
        )
    };
    let change = stage(
        Action::Work,
        "claude",
        Some("opus"),
        Level::Edits,
        "{task}\n\nMake the change. Keep it to what the task asks and match the style of the code \
         around it. Compress any image you commit and check that it still looks the same. Commit \
         in small steps once the tests pass. Do not push.\n\nBaseline notes from the earlier \
         stage:\n{previous_summary}\n\nOutput of the last failing test run:\n{test_output}",
    );
    let compare = Stage {
        approve: true,
        ..stage(
            Action::Work,
            "claude",
            Some("sonnet"),
            Level::Edits,
            "Render the same screens again at the current commit, with exactly the inputs of the \
             baseline. Compare each one against its baseline and count the changed pixels. A \
             difference inside the noise floor is noise. For every region that moved, name what \
             moved and say whether the task asked for it. Fix the code for anything it did not \
             ask for, then render again. Keep the images outside the repo. Report three groups: \
             screens that match, screens that changed on purpose (with the reason), and known \
             limits (with the size where they still differ). Do not push.",
        )
    };
    let check = review(
        "Review {base}..HEAD, the UI change for: {task}\n\nCheck narrow and wide sizes, empty and \
         very long content, and every state the change touches: hover, focus, disabled, loading \
         and error. Check that no unrelated screen changed. Report each finding with file, line, \
         blocking or nit, and a one-line fix. Do not edit files.",
        1,
    );
    let summary = handoff(
        "Write a PR title and description for {base}..HEAD. Include the three groups from the \
         screenshot check: match, changed on purpose, known limits. Say that each image was \
         rendered at the final commit. Name anything you could not render.",
    );
    workflow(
        UI_PROOF,
        "Change the UI with proof",
        "Baseline screenshots, the change, a fresh render, and every difference explained.",
        vec![baseline, change, compare, check, summary],
    )
}

fn port() -> Workflow {
    let audit = Stage {
        gate: Gate::Approve,
        ..stage(
            Action::Review,
            "claude",
            Some("opus"),
            Level::Plan,
            "{task}\n\nAudit before porting. Read the current code of the original platform, not \
             the docs. List every feature. For each one, name the mechanism that makes it work (a \
             setting, a file it writes, an OS call) and search the target platform's code for that \
             mechanism. Sort each feature into Kept, Partly or Missing, and cite a file for every \
             Kept one. A search shows that a mechanism exists, not that it works, so mark those \
             'search only' unless you ran them. For each Missing item, say where its code lives \
             and whether it must move into shared code first. Report the Missing list first, each \
             item with its effect on a user, then a short plan. Do not edit files.",
        )
    };
    let build = stage(
        Action::Work,
        "claude",
        Some("opus"),
        Level::Edits,
        "{task}\n\nPort in the order of the approved plan:\n{previous_summary}\n\nThe original \
         platform's code paths and behavior do not change. Put platform code behind platform \
         gates and shared code in shared modules, and move code only where the plan says so. \
         Small commits, one concern each, with tests. Do not push.\n\n\
         Open findings:\n{findings}\n\nOutput of the last failing test run:\n{test_output}",
    );
    let check = review(
        "Review {base}..HEAD. The task was: {task}\n\nFind every change that could alter behavior \
         on the original platform: edited shared code, changed defaults, renamed or moved items, \
         changed build or packaging files. Each one is blocking unless it is a pure move covered \
         by tests. Also find code that only builds on one platform without a gate, and items left \
         unused on the other platform. Report each finding with file, line, blocking or nit, and \
         a one-line fix. Do not edit files.",
        1,
    );
    let unchanged = Stage {
        gate: Gate::SameBehavior,
        on_fail: Some(1),
        ..stage(
            Action::Work,
            "claude",
            Some("sonnet"),
            Level::Edits,
            "Prove the original platform is unchanged. Run the full test suite and the build for \
             the original platform. Where there is a way to compare its output before and after, \
             such as a screenshot or the list of produced files, do that too. Do not change code \
             unless a check fails. Report each check you ran and what it covered. Do not push.\n\n\
             Output of the last failing test run:\n{test_output}",
        )
    };
    let summary = handoff(
        "Write a PR title and description for {base}..HEAD. List the features now Kept, Partly \
         and Missing on the new platform. Say plainly what was run on the new platform and what \
         only CI or a real machine of that platform can prove, under 'not verified'. Say that the \
         original platform was not changed and what shows it.",
    );
    workflow(
        PORT,
        "Port to a platform",
        "Audit what exists, port behind platform gates, prove the original did not change.",
        vec![audit, build, check, unchanged, summary],
    )
}

fn release() -> Workflow {
    let prepare = stage(
        Action::Work,
        "claude",
        Some("sonnet"),
        Level::Edits,
        "{task}\n\nPrepare a release. The task names the version or the kind of bump.\n\
         1. Find every place the version lives (manifests, lockfile, packaging files, site or doc \
         badges) and change them all in one commit. Refresh the lockfile only for the project's \
         own packages, not for its dependencies.\n\
         2. Write the changelog or release notes from the commits since the last release, grouped \
         by what a user notices. Leave out chores.\n\
         3. Check that the README and any screenshots match the build you are about to ship, and \
         list the ones that are stale.\n\
         Do not tag, push or publish. A pushed tag starts the release.\n\n\
         Output of the last failing test run:\n{test_output}",
    );
    let check = review(
        "Review {base}..HEAD, which prepares a release. Check that the version is the same \
         everywhere it appears, that the lockfile moved only for the project's own packages, that \
         the notes match the commits since the last release, and that nothing publishes by \
         default (publish steps stay behind an explicit switch). Check the packaging: every \
         artifact has a checksum, the install steps say how it is signed, and downloaded release \
         tools are pinned. Report each finding with file, line, blocking or nit, and a one-line \
         fix. Do not edit files.",
        0,
    );
    let artifacts = Stage {
        approve: true,
        ..stage(
            Action::Work,
            "claude",
            Some("sonnet"),
            Level::Edits,
            "Build the release artifacts locally if the project has a script for it, and check \
             them without running the app: the embedded version equals the new version, the \
             architectures are the ones the notes promise, the signature state is what the notes \
             say, and a checksum exists. Keep the artifacts in an ignored build folder, out of \
             git. If something cannot be built on this machine, say so and put it under 'not \
             verified'. Do not tag or push. If a check fails, report which one instead of \
             changing things quietly.",
        )
    };
    let checklist = handoff(
        "Write the release checklist for the owner. Include the version and what changed in one \
         paragraph, the exact commands to run after merging (the tag command, the workflow to \
         watch, how to check the published files in a new empty folder), what you verified, what \
         you could not verify, and what stays switched off until the owner allows it. Do not run \
         the tag command.",
    );
    workflow(
        RELEASE,
        "Prepare a release",
        "Bump the version, write the notes, check the artifacts. Never tags or pushes.",
        vec![prepare, check, artifacts, checklist],
    )
}

fn refactor() -> Workflow {
    let pin = stage(
        Action::Work,
        "claude",
        Some("sonnet"),
        Level::Edits,
        "{task}\n\nDo not change any non-test code. Before the refactor, pin the current behavior \
         of the code in scope with tests: the inputs, edge cases and error paths its callers rely \
         on. Run them against the unchanged code and make sure they pass. Where a test needs real \
         data to be honest, use real data and not only hand-made fixtures. List what stays \
         untested and why. Commit the tests on their own. Do not push.\n\n\
         Output of the last failing test run:\n{test_output}",
    );
    let change = stage(
        Action::Refactor,
        "claude",
        Some("opus"),
        Level::Edits,
        "{task}\n\nRefactor the code in scope. Behavior pinned by the previous stage:\n\
         {previous_summary}\n\nWork in small steps, one idea per commit, with the tests green \
         after each. Extract shared pieces first, then move call sites over. Do not edit the \
         tests that pin behavior and do not change what the code outputs. Every changed line must \
         trace to the task. If you find a bug, or a change that would alter output, do not make \
         it: add it to your report. Do not push.\n\nOpen findings:\n{findings}\n\n\
         Output of the last failing test run:\n{test_output}",
    );
    let check = review(
        "Review {base}..HEAD, a refactor for: {task}\n\nLook for any change in behavior: edited \
         test expectations, changed defaults, reordered side effects, widened or narrowed types, \
         dropped error handling. Each one is blocking. Also report changed lines that do not \
         trace to the task. Report each finding with file, line, blocking or nit, and a one-line \
         fix. Do not edit files.",
        1,
    );
    let summary = handoff(
        "Write a PR title and description for {base}..HEAD. Say that behavior is unchanged and \
         what shows it: which tests pin it. List what you noticed but did not change.",
    );
    workflow(
        REFACTOR,
        "Refactor safely",
        "Pin the behavior with tests, refactor in small steps, review for drift.",
        vec![pin, change, check, summary],
    )
}

pub(crate) fn all() -> Vec<Workflow> {
    vec![root_cause(), finish(), ui_proof(), port(), release(), refactor()]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::prompt;
    use crate::workflows::BUILT_IN_IDS;

    #[test]
    fn the_built_in_ids_are_the_defaults_and_nothing_else() {
        let defaults = crate::workflows::Workflows::defaults();
        let ids: BTreeSet<&str> = defaults.workflow.keys().map(String::as_str).collect();
        assert_eq!(ids, BUILT_IN_IDS.iter().copied().collect::<BTreeSet<_>>());
        assert_eq!(BUILT_IN_IDS.len(), ids.len());
    }

    #[test]
    fn names_are_unique_and_every_built_in_validates() {
        let all = all();
        let names: BTreeSet<&str> = all.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names.len(), all.len());
        for workflow in &all {
            assert_eq!(workflow.validate(), Vec::<String>::new(), "{}", workflow.id);
            assert_ne!(workflow.description, "", "{}", workflow.id);
            assert!(BUILT_IN_IDS.contains(&workflow.id.as_str()), "{}", workflow.id);
        }
    }

    #[test]
    fn prompts_use_only_known_placeholders_and_plain_hyphens() {
        for workflow in all() {
            for stage in &workflow.stages {
                let prompt = stage.prompt_template();
                assert_eq!(prompt::unknown_placeholders(prompt), Vec::<String>::new());
                assert!(!prompt.contains(['\u{2014}', '\u{2013}']), "{}: {prompt}", workflow.id);
                assert!(
                    !prompt.contains('@') && !prompt.contains("://"),
                    "{}: {prompt}",
                    workflow.id
                );
            }
        }
    }

    #[test]
    fn no_stage_pushes_and_every_workflow_ends_at_your_approval() {
        for workflow in all() {
            for stage in &workflow.stages {
                if stage.run.level != Some(Level::Plan) {
                    assert!(stage.prompt_template().contains("push"), "{}", workflow.id);
                }
            }
            let last = workflow.stages.last().unwrap();
            assert_eq!(
                (last.action, last.gate),
                (Action::Handoff, Gate::Approve),
                "{}",
                workflow.id
            );
        }
    }

    #[test]
    fn loops_only_go_back_to_a_stage_that_changes_things() {
        for workflow in all() {
            for stage in &workflow.stages {
                if let Some(target) = stage.on_fail {
                    let back = &workflow.stages[target];
                    assert_ne!(back.run.level, Some(Level::Plan), "{}", workflow.id);
                }
            }
        }
    }

    #[test]
    fn each_workflow_keeps_its_shape() {
        type Shape = (Action, Gate, Option<usize>, bool);
        let shapes: Vec<(&str, Vec<Shape>)> = vec![
            (
                ROOT_CAUSE,
                vec![
                    (Action::Reproduce, Gate::TestFails, None, true),
                    (Action::Fix, Gate::TestPasses, None, false),
                    (Action::Review, Gate::NoBlocking, Some(1), false),
                    (Action::Handoff, Gate::Approve, None, false),
                ],
            ),
            (
                FINISH,
                vec![
                    (Action::Work, Gate::Tests, None, false),
                    (Action::Review, Gate::NoBlocking, Some(0), false),
                    (Action::Handoff, Gate::Approve, None, false),
                ],
            ),
            (
                UI_PROOF,
                vec![
                    (Action::Work, Gate::Approve, None, false),
                    (Action::Work, Gate::Tests, None, false),
                    (Action::Work, Gate::Tests, None, true),
                    (Action::Review, Gate::NoBlocking, Some(1), false),
                    (Action::Handoff, Gate::Approve, None, false),
                ],
            ),
            (
                PORT,
                vec![
                    (Action::Review, Gate::Approve, None, false),
                    (Action::Work, Gate::Tests, None, false),
                    (Action::Review, Gate::NoBlocking, Some(1), false),
                    (Action::Work, Gate::SameBehavior, Some(1), false),
                    (Action::Handoff, Gate::Approve, None, false),
                ],
            ),
            (
                RELEASE,
                vec![
                    (Action::Work, Gate::Tests, None, false),
                    (Action::Review, Gate::NoBlocking, Some(0), false),
                    (Action::Work, Gate::Tests, None, true),
                    (Action::Handoff, Gate::Approve, None, false),
                ],
            ),
            (
                REFACTOR,
                vec![
                    (Action::Work, Gate::Tests, None, false),
                    (Action::Refactor, Gate::SameBehavior, None, false),
                    (Action::Review, Gate::NoBlocking, Some(1), false),
                    (Action::Handoff, Gate::Approve, None, false),
                ],
            ),
        ];
        let all = all();
        assert_eq!(all.len(), shapes.len());
        for (id, shape) in shapes {
            let workflow = all.iter().find(|w| w.id == id).unwrap();
            let got: Vec<_> =
                workflow.stages.iter().map(|s| (s.action, s.gate, s.on_fail, s.approve)).collect();
            assert_eq!(got, shape, "{id}");
        }
    }
}
