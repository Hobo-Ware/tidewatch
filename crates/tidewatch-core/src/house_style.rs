//! The house-style file (D13): one Tidewatch-owned markdown file added to every agent session.
//!
//! `ensure` creates it with the default text and never touches it again, so edits survive. Each agent
//! gets that text with the progress report format (`compose`) appended. Copilot only reads
//! `*.instructions.md` files with `applyTo` front matter, so a derived copy is rendered for it.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::agents::Level;
use crate::autonomy::autonomous;
use crate::fsutil::write_atomic;
use crate::paths::TidewatchPaths;
use crate::progress_prompt::{self, TurnEnd};

pub const DEFAULT_HOUSE_STYLE: &str = "\
# Tidewatch house style

You are running inside Tidewatch, which shows your replies as markdown. Follow these rules in every
answer. The repo's own instructions (CLAUDE.md, AGENTS.md) still apply and win on conflicts.

## Answers

- Write markdown. Start with a one or two line summary, then details under short headings.
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

## Tidewatch tools

When the `tidewatch` MCP tools are available:

- If a `stage_done` tool is offered, call it with a short summary when the stage you were given is finished.
- Call `reply_to_note` to answer a review note (`list_notes` shows them) instead of answering it only in chat.

## Reporting progress

Report your progress on the task with the `update_progress` tool of the `tidewatch` MCP server: report before any other work, again at every milestone, and call `get_progress` first after a context reset. A question for the user goes in the report's `needs_you`, never only in your reply.

## Questions

When you need the user to choose between options, ask a multiple-choice question instead of describing the options in prose. Use your own question tool if you have one (for example `AskUserQuestion`), otherwise call the `ask_user` tool of the `tidewatch` MCP server. Write the question for someone who has not followed your work: plain words, no jargon. Give two or three options, put the one you recommend first and end its label with (Recommended). With `ask_user`, also give the question a one-sentence `decision` and each option a one-line `tradeoff`, and set `recommended` on your top pick. An option that deletes anything names the exact items and how many.
";

const TOOL_GUIDANCE_HEADING: &str = "## Tidewatch tool guidance\n\n";

const JOBS_LINE: &str = "- Call `report_job` only for work you drive turn by turn. Anything you run in the background, or that takes more than a few minutes, must write JSON lines (`name`, `done`, `total`, `unit`, `phase`, `state`) to `$TIDEWATCH_PROGRESS` itself, because you cannot call a tool while you wait. For example: `printf '{\"name\":\"import\",\"done\":%s,\"total\":%s,\"unit\":\"rows\",\"phase\":\"load\"}\\n' \"$n\" \"$total\" >> \"$TIDEWATCH_PROGRESS\"`. Before you say the job is running, tail that file and check the first line landed.\n";

const CHOICES_LINE: &str = "- Ask choices in plain words with two or three options, mark your top pick, and name the exact items and the count for anything that deletes. The user can answer every open question with your picks in one click.\n";

const RECOVERY_LINE: &str = "- If `update_progress` is not available, look for `mcp__tidewatch__update_progress` with tool search once. If it is still missing, tell the user that Tidewatch reporting is down.\n";

const AVATAR_LINE: &str = "- Once you understand the task, call `set_avatar` once with a 64x64 flat round icon about the task (2-3 bright colors, simple shapes, nothing near the edge, and no ring, frame or background because Tidewatch draws the dark plate) and a one-line reason.\n";

const PUSH_LINE: &str = "- You may push any branch except the repo's default branch, force included; after folding fixups use `git push --force-with-lease`. The default branch and tags are refused. When Tidewatch asks you to open a pull request, push the branch and run `gh pr create --draft` yourself, writing the title (Conventional Commits style, what changed for the user) and the body (what changed and why, how you verified it, what you did not verify; short) in the house style.\n";

/// Tool guidance Tidewatch always puts after the house style, so an older or edited `house-style.md`
/// reaches agents with it too. A Meeseeks gets only the lines its own block does not contradict:
/// its face is fixed and it may push only its own branch.
#[must_use]
pub fn tool_guidance(role: Role) -> String {
    let lines: &[&str] = match role {
        Role::Meeseeks => &[JOBS_LINE, CHOICES_LINE, RECOVERY_LINE],
        Role::Agent | Role::Autonomous => {
            &[JOBS_LINE, CHOICES_LINE, RECOVERY_LINE, AVATAR_LINE, PUSH_LINE]
        }
    };
    format!("{TOOL_GUIDANCE_HEADING}{}", lines.concat())
}

pub const MEESEEKS_GUIDANCE: &str = "\
## You are a Meeseeks

You are a Meeseeks with exactly one task: the one you were given. Do it, and nothing else.

- First thing, call `set_name` once with a short kebab-case name for your task: 2 to 4 lowercase words, at most 30 characters, such as `fix-https-validation` or `my-super-feature`. It is what Tidewatch shows on your tab.
- Work only in this worktree and commit your work on this branch as you go.
- Set `kind` to `read-only` in your progress report for a question or an investigation (your final summary is the answer), and to `edit` as soon as you will change files.
- When you believe you are done or cannot go further, call `meeseeks_done` once with a one-line summary (a read-only task puts its whole answer there). If you are blocked, start it with `Blocked:` and the reason. Tidewatch decides what to check and ends this session itself.
- Never end with a decision for the user. If your answer leaves a choice (switch or not, which option), ask it with your question tool, the recommended option first, and act on the answer before you finish.
- You may push this branch; after folding fixups use `git push --force-with-lease`. Do not push anything else or remove anything yourself.
- Before `meeseeks_done`, a task that changed files pushes its branch and opens its own draft pull request with `gh pr create --draft`. Write the title (Conventional Commits style, what changed for the user) and the body (what changed and why, how you verified it, what you did not verify; short). No em or en dashes, no Co-Authored-By trailer, no \"generated with\" footer, no private repository names on a public repo. Tidewatch does not open it for you, and the hand-off check fails until it exists. A read-only task opens none.
- Do not call `stage_done`: `meeseeks_done` ends your task. Your end block's Needs you says `nothing`.
- If Tidewatch pastes a failed check, fix it, commit, and call `meeseeks_done` again.
- Do not call `set_avatar`: your face is fixed.
- Decide and proceed on everything except product design and architecture choices. Never ask for confirmation to commit, push, rerun checks or fix CI. For a design or architecture choice, ask with your question tool and keep working on whatever does not depend on the answer.
";

pub const AUTONOMOUS_GUIDANCE: &str = "\
## You run autonomously

This tab runs without hand holding. Decide and proceed on everything except product design and architecture choices.

- Never ask for confirmation to commit, push, open or update a pull request, rerun checks, answer review bots or fix CI. Do it.
- For a design or architecture choice, ask with your question tool and keep working on whatever does not depend on the answer.
";

/// Which extra block an agent's house style gets after the tool guidance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Agent,
    Autonomous,
    Meeseeks,
}

impl Role {
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
    compose_for_cli(house_style, must_dos, role, TurnEnd::SentBack)
}

#[must_use]
pub fn compose_for_cli(
    house_style: &str,
    must_dos: Option<&str>,
    role: Role,
    turn_end: TurnEnd,
) -> String {
    let one_task = match role {
        Role::Agent => String::new(),
        Role::Autonomous => format!("{AUTONOMOUS_GUIDANCE}\n"),
        Role::Meeseeks => format!("{MEESEEKS_GUIDANCE}\n"),
    };
    format!(
        "{}\n\n{}\n{one_task}{}",
        house_style.trim_end(),
        tool_guidance(role),
        progress_prompt::section(must_dos, turn_end)
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
        for needle in [
            "markdown",
            "path:line",
            "**Done**",
            "**Needs you**",
            "**Next**",
            "stage_done",
            "reply_to_note",
            "ask_user",
            "AskUserQuestion",
            "update_progress",
            "get_progress",
        ] {
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

    #[test]
    fn compose_appends_the_report_format_after_whatever_the_user_wrote() {
        let checklist = Some("- [ ] only this\n");
        for mine in ["be brief\n", "be brief", "be brief\n\n\n", ""] {
            let text = compose(mine, checklist);
            assert!(text.starts_with(mine.trim_end()), "{mine:?}");
            assert!(text.contains("## Progress report\n"), "{mine:?}");
            assert!(text.ends_with("not stages:\n\n- [ ] only this\n"));
            assert!(text.ends_with('\n'));
        }
        assert!(compose(DEFAULT_HOUSE_STYLE, checklist).contains("`get_progress` first"));
    }

    #[test]
    fn the_tool_guidance_reaches_agents_whatever_the_users_file_says() {
        let old_style =
            "# Tidewatch house style\n\nOld rules, written before the job lines existed.\n";
        let checklist = Some("- [ ] only this\n");
        for mine in [old_style, "", DEFAULT_HOUSE_STYLE] {
            let text = compose(mine, checklist);
            assert_eq!(text.matches("report_job").count(), 1, "{mine:?}");
            assert!(text.contains("`$TIDEWATCH_PROGRESS`"));
            assert_eq!(text.matches("`set_avatar`").count(), 1, "{mine:?}");
            let guidance = text.find("## Tidewatch tool guidance").unwrap();
            assert!(text.starts_with(mine.trim_end()));
            assert!(guidance >= mine.trim_end().len());
            assert!(guidance < text.find("## Progress report").unwrap());
        }
    }

    #[test]
    fn a_meeseeks_gets_its_one_task_block_between_the_tool_guidance_and_the_report_format() {
        let checklist = Some("- [ ] only this\n");
        let plain = compose("be brief", checklist);
        assert!(!plain.contains("Meeseeks") && !plain.contains("meeseeks_done"));
        assert!(!plain.contains("set_name"));
        assert_eq!(compose_for("be brief", checklist, Role::Agent), plain);
        let text = compose_for("be brief", checklist, Role::Meeseeks);
        let guidance = text.find("## Tidewatch tool guidance").unwrap();
        let block = text.find("## You are a Meeseeks").unwrap();
        assert!(guidance < block && block < text.find("## Progress report").unwrap());
        assert_eq!(text.matches("`meeseeks_done`").count(), 4);
        assert!(text.contains("You may push this branch; after folding fixups use `git push --force-with-lease`. Do not push anything else"));
        for needle in [
            "Before `meeseeks_done`, a task that changed files pushes its branch and opens its own draft pull request with `gh pr create --draft`",
            "Conventional Commits style",
            "how you verified it, what you did not verify",
            "No em or en dashes, no Co-Authored-By trailer, no \"generated with\" footer",
            "Tidewatch does not open it for you",
        ] {
            assert!(text.contains(needle), "missing {needle}");
        }
        assert!(!text.contains("Tidewatch does that"));
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
    fn a_meeseeks_never_gets_a_tool_line_its_own_block_contradicts() {
        let text = compose_for("be brief", Some("- [ ] only this\n"), Role::Meeseeks);
        for line in [AVATAR_LINE, PUSH_LINE] {
            assert!(!text.contains(line.trim()), "{line}");
        }
        assert!(!text.contains("You may push any branch"));
        assert!(!text.contains("call `set_avatar` once"));
        assert_eq!(text.matches("git push --force-with-lease").count(), 1);
        assert!(text.contains("Do not call `stage_done`"));
        for role in [Role::Agent, Role::Autonomous] {
            let other = compose_for("be brief", Some("- [ ] only this\n"), role);
            assert!(
                other.contains(AVATAR_LINE.trim()) && other.contains(PUSH_LINE.trim()),
                "{role:?}"
            );
        }
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
    fn only_yolo_and_meeseeks_agents_are_told_to_decide_and_proceed() {
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
            let told = text.contains("product design and architecture choices");
            assert_eq!(told, role != Role::Agent, "{role:?}");
            assert_eq!(text.contains("## You run autonomously"), role == Role::Autonomous);
        }
    }

    #[test]
    fn the_autonomous_block_is_short_and_keeps_design_questions_for_the_owner() {
        assert!(AUTONOMOUS_GUIDANCE.lines().count() <= 8, "{AUTONOMOUS_GUIDANCE}");
        for needle in [
            "commit, push, open or update a pull request",
            "rerun checks",
            "review bots",
            "fix CI",
            "question tool",
            "keep working",
        ] {
            assert!(AUTONOMOUS_GUIDANCE.contains(needle), "missing {needle}");
        }
        let text = compose_for("be brief", Some("- [ ] only this\n"), Role::Autonomous);
        let guidance = text.find("## Tidewatch tool guidance").unwrap();
        let block = text.find("## You run autonomously").unwrap();
        assert!(guidance < block && block < text.find("## Progress report").unwrap());
    }

    #[test]
    fn the_job_guidance_keeps_report_job_for_turn_by_turn_work_and_gives_the_one_liner() {
        let line = tool_guidance(Role::Agent);
        assert!(line.contains("`report_job` only for work you drive turn by turn"));
        assert!(line.contains("in the background"));
        assert!(line.contains(crate::background::PROGRESS_EXAMPLE));
        assert!(line.contains("tail that file and check the first line landed"));
        assert!(!line.contains('\u{2014}') && !line.contains('\u{2013}'));
    }

    #[test]
    fn the_editable_default_does_not_carry_the_tool_guidance() {
        assert!(!DEFAULT_HOUSE_STYLE.contains("report_job"));
        assert!(!DEFAULT_HOUSE_STYLE.contains("TIDEWATCH_PROGRESS"));
        assert!(!DEFAULT_HOUSE_STYLE.contains("set_avatar"));
    }
}
