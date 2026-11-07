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

## Review notes

Answer a review note with `reply_to_note` (`list_notes` shows them), not only in chat.
";

const INTRO: &str = "\
# Working in Tidewatch

Tidewatch shows your work to the user. This repo's own rules (CLAUDE.md, AGENTS.md) win on any conflict.
";

const ASKING: &str = "\
## Asking the user

Use your question tool, or `ask_user` if you have none. Tidewatch shows it as a card with buttons, so give two or three options and mark your top pick. Don't ask in plain chat. When the user says \"Let's chat about this\", answer in plain text and do not ask with your question tool or needs_you again until they ask for options.
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
- If a choice needs the user (see the last point), ask it with your question tool, the recommended option first.
- You may push this branch. Do not push anything else or remove anything yourself.
- Before `meeseeks_done`, a task that changed files pushes its branch and opens its own pull request with `gh pr create`, the way this repo's workflow says. Write the title and the body yourself (what changed and why, how you verified it, what you did not verify; short). Tidewatch does not open it for you, and the hand-off check fails until it exists. A read-only task opens none.
- `meeseeks_done` ends your task.
- If Tidewatch pastes a failed check, fix it and call `meeseeks_done` again.
- Do not call `set_avatar`: your face is fixed.
- Don't ask to commit, push, rerun checks or fix CI. Decide product and architecture choices yourself: take the option you would recommend and write it under `decisions` in your progress report with the reason. Ask only before something that loses data, spends money or cannot be undone outside this repo, or when this repo's rules tell you to stop, and keep working on whatever does not depend on the answer.
";

pub const AUTONOMOUS_GUIDANCE: &str = "\
## You run without supervision

Don't ask to commit, push, open or update a pull request, or fix CI. Decide product and architecture choices yourself too: take the option you would recommend, write it under `decisions` in your progress report with the reason, and keep going. The user reads the report and changes a decision there if they want to.
Ask with your question tool only before something that loses data, spends money or cannot be undone outside this repo, or when this repo's rules tell you to stop, and keep working on whatever does not depend on the answer.
";

pub const AWAY_GUIDANCE: &str = "\
## I'm away

I'm away. Make your own decisions: don't ask me and don't call the question tool. Write each decision with its reason under decisions in your progress report. Keep going through the open steps.
";

pub const WAKE_YOURSELF: &str = "When you have to wait (CI, a review, a background job, a time), don't end your turn hoping I'll ping you: wake yourself up. Use CronCreate with recurring false for a one-off check-in, Monitor to stream events from a script, or a background Bash or Agent call that notifies you when it ends. Save your report as waiting with waiting_on while you wait.";

pub const WAIT_IN_REPORT: &str = "When you have to wait (CI, a review, a job, a time), save your report as waiting with waiting_on and end your turn; I'll check back.";

#[must_use]
pub fn wake_line(wakes_itself: bool) -> &'static str {
    if wakes_itself { WAKE_YOURSELF } else { WAIT_IN_REPORT }
}

pub const KEPT_AWAKE_NOTE: &str = "\
Tidewatch keeps this Mac awake, also with the lid closed. Never ask the user to keep the lid open.
";

/// What the owner's presence adds to the launch text: Away, and a Mac Tidewatch keeps awake.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Presence {
    pub away: bool,
    pub kept_awake: bool,
}

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
    compose_for_cli(house_style, must_dos, role, TurnEnd::SentBack, None, Presence::default())
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
    presence: Presence,
) -> String {
    let workflow = workflow.map(|workflow| format!("\n{}", workflow_section(role, workflow)));
    let one_task = match role {
        Role::Agent => String::new(),
        Role::Autonomous => format!("\n{AUTONOMOUS_GUIDANCE}"),
        Role::Meeseeks | Role::MeeseeksOnPr => format!("\n{}", meeseeks_block(role)),
    };
    let away = if presence.away {
        format!("\n{AWAY_GUIDANCE}\n{}\n", wake_line(turn_end == TurnEnd::SentBack))
    } else {
        String::new()
    };
    let awake = if presence.kept_awake { format!("\n{KEPT_AWAKE_NOTE}") } else { String::new() };
    format!(
        "{}\n\n{INTRO}\n{}\n{ASKING}\n{JOBS}{}{one_task}{away}{awake}",
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
        for needle in ["markdown", "path:line", "reply_to_note"] {
            assert!(DEFAULT_HOUSE_STYLE.contains(needle), "missing {needle}");
        }
        for needle in ["**Done**", "**Needs you**", "**Next**", "End of every turn"] {
            assert!(!DEFAULT_HOUSE_STYLE.contains(needle), "still asks for {needle}");
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
    fn away_adds_the_decide_yourself_block_and_not_away_leaves_it_out() {
        for role in ROLES {
            let with = |presence| {
                compose_for_cli("be brief", None, role, TurnEnd::SentBack, None, presence)
            };
            let here = with(Presence::default());
            let away = with(Presence { away: true, kept_awake: false });
            assert!(!here.contains("## I'm away"), "{role:?}");
            assert!(away.contains(AWAY_GUIDANCE), "{role:?}");
            assert!(away.contains("don't call the question tool"), "{role:?}");
            assert!(away.contains("under decisions in your progress report"), "{role:?}");
            assert!(!away.contains(KEPT_AWAKE_NOTE), "{role:?}");
        }
    }

    #[test]
    fn only_a_claude_agent_is_told_the_tools_to_wake_itself_with() {
        let away = Presence { away: true, kept_awake: false };
        let with = |turn_end| compose_for_cli("be brief", None, Role::Agent, turn_end, None, away);
        let claude = with(TurnEnd::SentBack);
        for tool in ["CronCreate", "Monitor", "recurring false"] {
            assert_eq!(claude.matches(tool).count(), 1, "{tool}");
        }
        assert!(claude.contains(WAKE_YOURSELF) && !claude.contains(WAIT_IN_REPORT));
        let other = with(TurnEnd::Reminded);
        for tool in ["CronCreate", "Monitor", "wake yourself up"] {
            assert!(!other.contains(tool), "{tool}");
        }
        assert!(other.contains(WAIT_IN_REPORT) && other.contains("waiting_on"));
    }

    #[test]
    fn the_cli_that_wakes_itself_is_the_one_whose_turn_end_is_sent_back() {
        use crate::agents::AgentTable;
        let table = AgentTable::builtin();
        for id in ["claude", "codex", "gemini", "copilot"] {
            let agent = table.get(id).unwrap();
            let sent_back = TurnEnd::of(agent.wiring) == TurnEnd::SentBack;
            assert_eq!(crate::agents::wakes_itself(id), sent_back, "{id}");
        }
    }

    #[test]
    fn a_kept_awake_mac_is_said_to_stay_awake_with_the_lid_closed_only_while_it_is() {
        let with = |presence| {
            compose_for_cli("be brief", None, Role::Agent, TurnEnd::SentBack, None, presence)
        };
        let awake = with(Presence { away: false, kept_awake: true });
        assert!(awake.contains("Never ask the user to keep the lid open."));
        assert!(!awake.contains("## I'm away"));
        assert!(!with(Presence::default()).contains("lid"));
    }

    #[test]
    fn the_away_text_never_names_an_option_and_has_no_dashes() {
        for text in [AWAY_GUIDANCE, WAKE_YOURSELF, WAIT_IN_REPORT, KEPT_AWAKE_NOTE] {
            assert!(!text.contains('\u{2014}') && !text.contains('\u{2013}'), "{text}");
        }
        assert!(!AWAY_GUIDANCE.to_lowercase().contains("recommend"));
    }

    #[test]
    fn the_launch_text_of_a_claude_agent_tab_stays_within_450_words() {
        let text = compose_for_cli(
            DEFAULT_HOUSE_STYLE,
            None,
            Role::Agent,
            TurnEnd::SentBack,
            Some(Workflow::default()),
            Presence::default(),
        );
        assert!(words(&text) <= 450, "{} words:\n{text}", words(&text));
    }

    #[test]
    fn no_built_in_text_names_a_method_unless_the_profile_filled_it() {
        for role in ROLES {
            for turn_end in [TurnEnd::SentBack, TurnEnd::Reminded] {
                let text = compose_for_cli(
                    DEFAULT_HOUSE_STYLE,
                    None,
                    role,
                    turn_end,
                    None,
                    Presence::default(),
                )
                .to_lowercase();
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
                compose_for_cli(
                    DEFAULT_HOUSE_STYLE,
                    None,
                    role,
                    TurnEnd::SentBack,
                    Some(workflow),
                    Presence::default(),
                )
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
                Presence::default(),
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
            "ask it with your question tool, the recommended option first",
        ] {
            assert!(text.contains(needle), "missing {needle}");
        }
        for gone in [
            "commit your work on this branch as you go",
            "Never end with a decision for the user",
            "Conventional Commits",
            "Co-Authored-By",
            "Tidewatch does that",
            "picks your recommended answer after",
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
            "Decide product and architecture choices yourself",
            "under `decisions` in your progress report",
            "loses data, spends money or cannot be undone outside this repo",
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
    fn a_chat_request_tells_the_agent_to_stop_asking_until_options_are_wanted() {
        let text = compose("be brief", None);
        assert_eq!(text.matches("Let's chat about this").count(), 1);
        assert!(text.contains(
            "do not ask with your question tool or needs_you again until they ask for options"
        ));
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
