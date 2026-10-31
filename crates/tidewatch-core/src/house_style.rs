//! The house-style file (D13): one Tidewatch-owned markdown file added to every agent session.
//!
//! `ensure` creates it with the default text and never touches it again, so edits survive. Each agent
//! gets that text with Tidewatch's own short launch text (`compose`) appended. The launch text names a
//! method (rebase, fixup, draft) only through the repo's workflow profile. Copilot only reads
//! `*.instructions.md` files with `applyTo` front matter, so a derived copy is rendered for it.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::agents::Level;
use crate::autonomy::autonomous;
use crate::fsutil::write_atomic;
use crate::meeseeks_target::Target;
use crate::paths::TidewatchPaths;
use crate::progress_prompt::{self, TurnEnd};
use crate::workflow_profile::Workflow;

pub const DEFAULT_HOUSE_STYLE: &str = "\
# Tidewatch house style

Tidewatch shows your replies as markdown.

## Answers

- Start with a one or two line summary, then details under short headings.
- Use lists for several items and fenced code blocks, with a language, for code, commands and output.
- Refer to code as `path:line` (for example `src/main.rs:42`) so Tidewatch can link it.
- Keep it short. No filler openers and no closing recap.

## End of every turn

Finish with exactly this block:

**Done**
- what changed or what you found

**Needs you**
- the decision or input you need, or \"nothing\"

**Next**
- the single next step

## Review notes

Answer a review note with `reply_to_note` (`list_notes` shows them), not only in chat.
";

const INTRO: &str = "\
# Working in Tidewatch

Tidewatch shows your work to the user. This repo's own rules (CLAUDE.md, AGENTS.md) win on any conflict.
";

const ASKING: &str = "\
## Asking the user

Use your question tool, or `ask_user` if you have none. Tidewatch shows it as a card with buttons, so give two or three options and mark your top pick. Don't ask in plain chat.
";

const JOBS: &str = "\
## Long jobs

Background work writes JSON lines to `$TIDEWATCH_PROGRESS`, because you cannot call a tool while you wait: `{\"name\",\"done\",\"total\",\"unit\"}`. Call `report_job` only for work you drive turn by turn.
";

const PUSH_LINE: &str =
    "You may push any branch except the default branch. Tags and the default branch are refused.";

fn workflow_section(role: Role, workflow: Workflow) -> String {
    let mut lines = workflow.launch_lines().to_vec();
    if matches!(role, Role::Agent | Role::Autonomous) {
        lines.push(PUSH_LINE);
    }
    format!("## This repo's workflow\n\n- {}\n", lines.join("\n- "))
}

pub const MEESEEKS_GUIDANCE: &str = "\
## You are a Meeseeks

You are a Meeseeks with exactly one task: the one you were given. Do it, and nothing else.

- First thing, call `set_name` once with a short kebab-case name for your task: 2 to 4 lowercase words, at most 30 characters, such as `fix-https-validation` or `my-super-feature`. It is what Tidewatch shows on your tab.
- Work only in this worktree. Commit the way this repo's workflow says.
- Set `kind` to `read-only` in your progress report for a question or an investigation (your final summary is the answer), and to `edit` as soon as you will change files.
- When you believe you are done or cannot go further, call `meeseeks_done` once with a one-line summary (a read-only task puts its whole answer there). If you are blocked, start it with `Blocked:` and the reason. Tidewatch decides what to check and ends this session itself.
- If a choice is left, ask it with your question tool, the recommended option first. Tidewatch picks your recommended answer after 5 minutes and tells the user it did.
- You may push this branch. Do not push anything else or remove anything yourself.
- Before `meeseeks_done`, a task that changed files pushes its branch and opens its own pull request with `gh pr create`, the way this repo's workflow says. Write the title and the body yourself (what changed and why, how you verified it, what you did not verify; short). Tidewatch does not open it for you, and the hand-off check fails until it exists. A read-only task opens none.
- `meeseeks_done` ends your task. Your end block's Needs you says `nothing`.
- If Tidewatch pastes a failed check, fix it and call `meeseeks_done` again.
- Do not call `set_avatar`: your face is fixed.
- Don't ask to commit, push, rerun checks or fix CI. Ask for product design or architecture choices, or when this repo's rules tell you to stop, and keep working on whatever does not depend on the answer.
";

pub const AUTONOMOUS_GUIDANCE: &str = "\
## You run without supervision

Don't ask to commit, push, open or update a pull request, or fix CI. Ask with your question tool for product or architecture choices, or when this repo's rules tell you to stop, and keep working on whatever does not depend on the answer.
";

/// Which extra block an agent's house style gets after the launch text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Agent,
    Autonomous,
    Meeseeks,
    /// A Meeseeks whose task names a pull request: it updates that one and opens no other.
    MeeseeksOnPr,
}

const OPEN_PR_LINE_START: &str =
    "- Before `meeseeks_done`, a task that changed files pushes its branch";
const ON_PR_LINE: &str = "- Your task names a pull request, so before `meeseeks_done` push your commits to that pull request's branch (`gh pr view <pr> --json headRefName` names it), the one push besides this branch, and open no new pull request: Tidewatch checks that one.";

fn meeseeks_block(role: Role) -> String {
    match role {
        Role::MeeseeksOnPr => {
            MEESEEKS_GUIDANCE
                .lines()
                .map(|line| if line.starts_with(OPEN_PR_LINE_START) { ON_PR_LINE } else { line })
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"
        }
        _ => MEESEEKS_GUIDANCE.to_owned(),
    }
}

impl Role {
    /// The role of a Meeseeks with this task.
    #[must_use]
    pub fn meeseeks_for(task: &str) -> Self {
        match Target::of_task(task) {
            Target::New => Self::Meeseeks,
            Target::Existing(_) => Self::MeeseeksOnPr,
        }
    }

    #[must_use]
    pub fn of(level: Level, is_meeseeks: bool) -> Self {
        match (is_meeseeks, autonomous(level, is_meeseeks)) {
            (true, _) => Self::Meeseeks,
            (false, true) => Self::Autonomous,
            (false, false) => Self::Agent,
        }
    }
}

/// Writes the default text over `house-style.md`, edits included. Returns its path.
pub fn reset(paths: &TidewatchPaths) -> std::io::Result<PathBuf> {
    let file = paths.house_style_file();
    std::fs::create_dir_all(paths.config_dir())?;
    write_atomic(&file, DEFAULT_HOUSE_STYLE)?;
    Ok(file)
}

const COPILOT_FILE_NAME: &str = "tidewatch.instructions.md";

/// Creates `house-style.md` with the default text when it is missing. An existing file, edited or
/// empty, is never overwritten. Returns its path.
pub fn ensure(paths: &TidewatchPaths) -> std::io::Result<PathBuf> {
    let file = paths.house_style_file();
    std::fs::create_dir_all(paths.config_dir())?;
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&file) {
        Ok(mut created) => created.write_all(DEFAULT_HOUSE_STYLE.as_bytes())?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    Ok(file)
}

/// The house style as a Copilot instructions file: `applyTo: "**"` front matter, then the text.
#[must_use]
pub fn copilot_instructions_md(house_style: &str) -> String {
    format!("---\napplyTo: \"**\"\n---\n\n{house_style}")
}

#[must_use]
pub fn compose(house_style: &str, must_dos: Option<&str>) -> String {
    compose_for(house_style, must_dos, Role::Agent)
}

#[must_use]
pub fn compose_for(house_style: &str, must_dos: Option<&str>, role: Role) -> String {
    compose_for_cli(house_style, must_dos, role, TurnEnd::SentBack, None)
}

/// `workflow` is the repo's profile; `None` (no repo: a plain folder) leaves out "This repo's
/// workflow", so the text then names no method at all.
#[must_use]
pub fn compose_for_cli(
    house_style: &str,
    must_dos: Option<&str>,
    role: Role,
    turn_end: TurnEnd,
    workflow: Option<Workflow>,
) -> String {
    let workflow = workflow.map(|workflow| format!("\n{}", workflow_section(role, workflow)));
    let one_task = match role {
        Role::Agent => String::new(),
        Role::Autonomous => format!("\n{AUTONOMOUS_GUIDANCE}"),
        Role::Meeseeks | Role::MeeseeksOnPr => format!("\n{}", meeseeks_block(role)),
    };
    format!(
        "{}\n\n{INTRO}\n{}\n{ASKING}\n{JOBS}{}{one_task}",
        house_style.trim_end(),
        progress_prompt::section(must_dos, turn_end),
        workflow.unwrap_or_default(),
    )
}

/// Rewrites the derived Copilot copy of `house_style` in `dir` and returns `dir`, the value for
/// `COPILOT_CUSTOM_INSTRUCTIONS_DIRS`. Safe to call on every launch.
pub fn sync_copilot_instructions(dir: &Path, house_style: &str) -> std::io::Result<PathBuf> {
    write_atomic(&dir.join(COPILOT_FILE_NAME), &copilot_instructions_md(house_style))?;
    Ok(dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (tempfile::TempDir, TidewatchPaths) {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        (home, paths)
    }

    #[test]
    fn default_text_asks_for_the_d13_contract() {
        for needle in
            ["markdown", "path:line", "**Done**", "**Needs you**", "**Next**", "reply_to_note"]
        {
            assert!(DEFAULT_HOUSE_STYLE.contains(needle), "missing {needle}");
        }
    }

    #[test]
    fn ensure_creates_the_file_with_the_default_text() {
        let (_home, paths) = paths();
        let file = ensure(&paths).unwrap();
        assert_eq!(file, paths.house_style_file());
        assert_eq!(std::fs::read_to_string(file).unwrap(), DEFAULT_HOUSE_STYLE);
    }

    #[test]
    fn ensure_keeps_an_edited_file() {
        let (_home, paths) = paths();
        ensure(&paths).unwrap();
        std::fs::write(paths.house_style_file(), "my own style\n").unwrap();
        ensure(&paths).unwrap();
        assert_eq!(std::fs::read_to_string(paths.house_style_file()).unwrap(), "my own style\n");
    }

    #[test]
    fn reset_puts_the_default_text_back_over_edits_and_creates_a_missing_file() {
        let (_home, paths) = paths();
        assert_eq!(reset(&paths).unwrap(), paths.house_style_file());
        assert_eq!(std::fs::read_to_string(paths.house_style_file()).unwrap(), DEFAULT_HOUSE_STYLE);
        std::fs::write(paths.house_style_file(), "my own style\n").unwrap();
        reset(&paths).unwrap();
        assert_eq!(std::fs::read_to_string(paths.house_style_file()).unwrap(), DEFAULT_HOUSE_STYLE);
    }

    #[test]
    fn ensure_keeps_an_emptied_file() {
        let (_home, paths) = paths();
        ensure(&paths).unwrap();
        std::fs::write(paths.house_style_file(), "").unwrap();
        ensure(&paths).unwrap();
        assert_eq!(std::fs::read_to_string(paths.house_style_file()).unwrap(), "");
    }

    #[test]
    fn ensure_recreates_a_deleted_file() {
        let (_home, paths) = paths();
        ensure(&paths).unwrap();
        std::fs::remove_file(paths.house_style_file()).unwrap();
        ensure(&paths).unwrap();
        assert_eq!(std::fs::read_to_string(paths.house_style_file()).unwrap(), DEFAULT_HOUSE_STYLE);
    }

    #[test]
    fn copilot_copy_has_apply_to_front_matter_and_holds_the_text_it_is_given() {
        let dir = tempfile::tempdir().unwrap();
        let written = sync_copilot_instructions(dir.path(), "be brief\n").unwrap();
        assert_eq!(written, dir.path());
        let text = std::fs::read_to_string(dir.path().join("tidewatch.instructions.md")).unwrap();
        assert_eq!(text, "---\napplyTo: \"**\"\n---\n\nbe brief\n");
    }

    const METHOD_WORDS: [&str; 10] = [
        "rebase",
        "merge",
        "fixup",
        "autosquash",
        "--draft",
        "draft",
        "force-with-lease",
        "force-push",
        "squash",
        "conventional commits",
    ];

    const ROLES: [Role; 4] = [Role::Agent, Role::Autonomous, Role::Meeseeks, Role::MeeseeksOnPr];

    fn words(text: &str) -> usize {
        text.split_whitespace().count()
    }

    #[test]
    fn compose_appends_the_launch_text_after_whatever_the_user_wrote() {
        let checklist = Some("- [ ] only this\n");
        for mine in ["be brief\n", "be brief", "be brief\n\n\n", ""] {
            let text = compose(mine, checklist);
            assert!(text.starts_with(mine.trim_end()), "{mine:?}");
            assert!(text.contains("# Working in Tidewatch\n"), "{mine:?}");
            assert!(text.contains("## Report\n"), "{mine:?}");
            assert!(text.contains("Must-do steps from the user or this repo"), "{mine:?}");
            assert!(text.contains("- [ ] only this\n"), "{mine:?}");
            assert!(text.ends_with('\n'));
        }
        assert!(compose(DEFAULT_HOUSE_STYLE, checklist).contains("`get_progress` first"));
    }

    #[test]
    fn the_launch_text_reaches_agents_whatever_the_users_file_says() {
        let old_style =
            "# Tidewatch house style\n\nOld rules, written before the job lines existed.\n";
        let checklist = Some("- [ ] only this\n");
        for mine in [old_style, "", DEFAULT_HOUSE_STYLE] {
            let text = compose(mine, checklist);
            assert_eq!(text.matches("report_job").count(), 1, "{mine:?}");
            assert!(text.contains("`$TIDEWATCH_PROGRESS`"));
            let launch = text.find("# Working in Tidewatch").unwrap();
            assert!(text.starts_with(mine.trim_end()));
            assert!(launch >= mine.trim_end().len());
            let order = ["## Report", "## Asking the user", "## Long jobs"].map(|h| text.find(h));
            assert!(order.iter().all(Option::is_some), "{order:?}");
            assert!(launch < order[0].unwrap() && order[0] < order[1] && order[1] < order[2]);
        }
    }

    #[test]
    fn the_launch_text_of_a_claude_agent_tab_stays_within_450_words() {
        let text = compose_for_cli(
            DEFAULT_HOUSE_STYLE,
            None,
            Role::Agent,
            TurnEnd::SentBack,
            Some(Workflow::default()),
        );
        assert!(words(&text) <= 450, "{} words:\n{text}", words(&text));
    }

    #[test]
    fn no_built_in_text_names_a_method_unless_the_profile_filled_it() {
        for role in ROLES {
            for turn_end in [TurnEnd::SentBack, TurnEnd::Reminded] {
                let text =
                    compose_for_cli(DEFAULT_HOUSE_STYLE, None, role, turn_end, None).to_lowercase();
                for word in METHOD_WORDS {
                    assert!(!text.contains(word), "{role:?} names {word:?}");
                }
            }
        }
    }

    #[test]
    fn the_profile_fills_this_repos_workflow_and_nothing_else_names_a_method() {
        let rebase = Workflow::default();
        let merge = Workflow {
            catch_up: crate::workflow_profile::CatchUp::Merge,
            fixes: crate::workflow_profile::Fixes::NewCommits,
            pull_requests: crate::workflow_profile::PullRequests::Ready,
            ..Workflow::default()
        };
        for role in ROLES {
            let with = |workflow| {
                compose_for_cli(DEFAULT_HOUSE_STYLE, None, role, TurnEnd::SentBack, Some(workflow))
            };
            let rebased = with(rebase);
            assert_eq!(rebased.matches("## This repo's workflow").count(), 1, "{role:?}");
            for line in rebase.launch_lines() {
                assert!(rebased.contains(&format!("- {line}\n")), "{role:?} {line}");
            }
            let merged = with(merge).to_lowercase();
            assert!(merged.contains("merging the default branch"), "{role:?}");
            for word in ["rebase", "fixup", "autosquash", "draft", "force-with-lease"] {
                assert!(
                    !merged.replace("no `--draft`", "").contains(word),
                    "{role:?} still names {word:?}"
                );
            }
            let before = rebased.split("## This repo's workflow").next().unwrap().to_lowercase();
            for word in METHOD_WORDS {
                assert!(!before.contains(word), "{role:?} names {word:?} before the profile");
            }
        }
    }

    #[test]
    fn only_an_agent_that_may_push_is_told_which_branches_it_may_push() {
        for role in ROLES {
            let text = compose_for_cli(
                "be brief",
                None,
                role,
                TurnEnd::SentBack,
                Some(Workflow::default()),
            );
            let told = text.contains(PUSH_LINE);
            assert_eq!(told, matches!(role, Role::Agent | Role::Autonomous), "{role:?}");
        }
    }

    #[test]
    fn a_meeseeks_gets_its_one_task_block_after_the_launch_text() {
        let checklist = Some("- [ ] only this\n");
        let plain = compose("be brief", checklist);
        assert!(!plain.contains("Meeseeks") && !plain.contains("meeseeks_done"));
        assert!(!plain.contains("set_name"));
        assert_eq!(compose_for("be brief", checklist, Role::Agent), plain);
        let text = compose_for("be brief", checklist, Role::Meeseeks);
        let launch = text.find("# Working in Tidewatch").unwrap();
        let block = text.find("## You are a Meeseeks").unwrap();
        assert!(
            launch < text.find("## Report").unwrap() && text.find("## Long jobs").unwrap() < block
        );
        assert_eq!(text.matches("`meeseeks_done`").count(), 4);
        assert!(text.contains("You may push this branch. Do not push anything else"));
        for needle in [
            "Before `meeseeks_done`, a task that changed files pushes its branch and opens its own pull request with `gh pr create`, the way this repo's workflow says",
            "what changed and why, how you verified it, what you did not verify",
            "Tidewatch does not open it for you",
            "Commit the way this repo's workflow says",
            "If a choice is left, ask it with your question tool",
            "Tidewatch picks your recommended answer after 5 minutes and tells the user it did",
        ] {
            assert!(text.contains(needle), "missing {needle}");
        }
        for gone in [
            "commit your work on this branch as you go",
            "Never end with a decision for the user",
            "Conventional Commits",
            "Co-Authored-By",
            "Tidewatch does that",
        ] {
            assert!(!text.contains(gone), "still says {gone}");
        }
        assert!(text.contains("`kind` to `read-only`") && text.contains("`edit`"));
        assert!(text.contains("start it with `Blocked:`"));
        assert!(text.contains("First thing, call `set_name` once"));
        assert!(text.contains("`fix-https-validation` or `my-super-feature`"));
        assert!(
            text.find("`set_name`").unwrap() < text.find("Work only in this worktree").unwrap()
        );
        assert!(!text.contains("done and committed"));
        assert!(!compose("be brief", checklist).contains("kind: read-only"));
    }

    #[test]
    fn no_role_is_told_to_draw_an_avatar_or_to_write_its_reply_in_a_certain_place() {
        for role in ROLES {
            let text = compose_for("be brief", Some("- [ ] only this\n"), role);
            assert!(!text.contains("call `set_avatar` once"), "{role:?}");
            assert!(!text.contains("one message at the end of the turn"), "{role:?}");
            assert!(!text.contains("Never put the answer before tool calls"), "{role:?}");
        }
        let meeseeks = compose_for("be brief", None, Role::Meeseeks);
        assert!(meeseeks.contains("Do not call `set_avatar`: your face is fixed."));
    }

    #[test]
    fn every_role_can_recover_when_the_reporting_tools_are_missing() {
        for role in [Role::Agent, Role::Autonomous, Role::Meeseeks] {
            let text = compose_for("be brief", None, role);
            assert!(
                text.contains("`mcp__tidewatch__update_progress` with tool search once"),
                "{role:?}"
            );
            assert!(text.contains("Tidewatch reporting is down"), "{role:?}");
        }
    }

    #[test]
    fn only_yolo_and_meeseeks_agents_are_told_to_keep_working_on_what_the_answer_does_not_block() {
        let checklist = Some("- [ ] only this\n");
        let cases = [
            (Level::Plan, false, Role::Agent),
            (Level::Ask, false, Role::Agent),
            (Level::Edits, false, Role::Agent),
            (Level::Auto, false, Role::Agent),
            (Level::Yolo, false, Role::Autonomous),
            (Level::Auto, true, Role::Meeseeks),
            (Level::Yolo, true, Role::Meeseeks),
        ];
        for (level, is_meeseeks, role) in cases {
            assert_eq!(Role::of(level, is_meeseeks), role, "{level:?} {is_meeseeks}");
            let text = compose_for("be brief", checklist, role);
            let told = text.contains("architecture choices");
            assert_eq!(told, role != Role::Agent, "{role:?}");
            assert_eq!(text.contains("## You run without supervision"), role == Role::Autonomous);
        }
    }

    #[test]
    fn the_autonomous_block_is_short_and_settles_the_fight_with_stop_when_confused() {
        assert!(AUTONOMOUS_GUIDANCE.lines().count() <= 5, "{AUTONOMOUS_GUIDANCE}");
        for needle in [
            "Don't ask to commit, push, open or update a pull request, or fix CI.",
            "question tool",
            "product or architecture choices",
            "or when this repo's rules tell you to stop",
            "keep working on whatever does not depend on the answer",
        ] {
            assert!(AUTONOMOUS_GUIDANCE.contains(needle), "missing {needle}");
        }
        assert!(!AUTONOMOUS_GUIDANCE.contains("Decide and proceed on everything"));
        let text = compose_for("be brief", Some("- [ ] only this\n"), Role::Autonomous);
        let launch = text.find("# Working in Tidewatch").unwrap();
        let block = text.find("## You run without supervision").unwrap();
        assert!(launch < block);
    }

    #[test]
    fn the_job_lines_keep_report_job_for_turn_by_turn_work_and_name_the_progress_file() {
        let text = compose("be brief", None);
        assert!(text.contains("Call `report_job` only for work you drive turn by turn."));
        assert!(text.contains("JSON lines to `$TIDEWATCH_PROGRESS`"));
        assert!(text.contains("`{\"name\",\"done\",\"total\",\"unit\"}`"));
        assert!(!text.contains('\u{2014}') && !text.contains('\u{2013}'));
    }

    #[test]
    fn questions_go_through_the_question_tool_and_never_plain_chat() {
        let text = compose("be brief", None);
        assert!(text.contains("Use your question tool, or `ask_user` if you have none."));
        assert!(text.contains("Don't ask in plain chat."));
    }

    #[test]
    fn the_editable_default_does_not_carry_the_launch_text() {
        for needle in [
            "report_job",
            "TIDEWATCH_PROGRESS",
            "set_avatar",
            "update_progress",
            "get_progress",
            "ask_user",
            "AskUserQuestion",
        ] {
            assert!(!DEFAULT_HOUSE_STYLE.contains(needle), "{needle}");
        }
    }

    #[test]
    fn a_task_that_names_a_pull_request_swaps_one_line_of_the_meeseeks_block_and_nothing_else() {
        let new = compose_for("be brief", None, Role::Meeseeks);
        let on_pr = compose_for("be brief", None, Role::MeeseeksOnPr);
        assert_ne!(new, on_pr);
        assert_eq!(on_pr.matches("open no new pull request").count(), 1);
        assert!(!on_pr.contains("opens its own pull request"));
        assert_eq!(new.lines().count(), on_pr.lines().count());
        let differing: Vec<&str> =
            new.lines().zip(on_pr.lines()).filter(|(a, b)| a != b).map(|(_, b)| b).collect();
        assert_eq!(differing, [ON_PR_LINE]);
        assert_eq!(on_pr.matches("`meeseeks_done` ends your task").count(), 1);
    }

    #[test]
    fn the_role_of_a_meeseeks_follows_the_pull_request_its_task_names() {
        let cases = [
            ("Sensei https://github.com/trakt/trakt-workers/pull/2437", Role::MeeseeksOnPr),
            ("Rebase PR #12", Role::MeeseeksOnPr),
            ("Add a retry to the sync job", Role::Meeseeks),
            ("Explain issue #12", Role::Meeseeks),
            ("", Role::Meeseeks),
        ];
        for (task, want) in cases {
            assert_eq!(Role::meeseeks_for(task), want, "{task:?}");
        }
    }
}
