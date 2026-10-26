//! The instruction that puts the progress report in front of an agent: what to send through
//! `update_progress`, which states and stage kinds Tidewatch maps, and the user's must-do steps when
//! there are any (pure). Every CLI gets the same text; only the sentence on a turn that ends with
//! no report differs.

use crate::agents::Wiring;

const REPORT_FORMAT: &str = "\
## Progress report

Tidewatch shows the user a live view of your task and maps what you report onto its screens: one state word and one colour everywhere. You report with the `update_progress` tool of the `tidewatch` MCP server.

- This report is required. A notes file of your own, a plan in the chat or a memory entry never replaces it: only `update_progress` shows it to the user and lists your later work in Coming up. {turn_end}
- Report first, before any other work: the goal, your stages and the state you start in. Report again at every milestone: a decision, a finished step, a check, a blocker, a question, a wait, a change of stage. Each call replaces the last, so always send all of it.
- After a context reset, or when you resume a session, call `get_progress` first and carry on from what it returns. Never start the report over.
- If a rule below is broken, nothing is saved and the tool lists each fix. Fix them all and call again.

The arguments of `update_progress`:

```json
{
  \"goal\": \"Retry failed syncs and backfill the old rows\",
  \"state\": \"needs_you\",
  \"now\": \"Backfill batch 2 is running. Batch 3 waits for your pick.\",
  \"stages\": [
    { \"name\": \"Build\", \"kind\": \"build\", \"status\": \"done\" },
    { \"name\": \"Ship\", \"kind\": \"ship\", \"status\": \"done\" },
    { \"name\": \"Backfill\", \"kind\": \"ship\", \"status\": \"active\" },
    { \"name\": \"Verify\", \"kind\": \"verify\", \"status\": \"todo\" }
  ],
  \"steps\": [
    { \"stage\": \"Ship\", \"text\": \"Deploy both services\", \"status\": \"done\", \"evidence\": \"both deploys succeeded\" },
    { \"stage\": \"Backfill\", \"text\": \"Check links against each season\", \"status\": \"active\" },
    { \"stage\": \"Backfill\", \"text\": \"Guarded backfill writes\", \"status\": \"blocked\", \"reason\": \"waits on the user's answer\" },
    { \"stage\": \"Verify\", \"text\": \"Check one row on the live route\", \"status\": \"todo\" }
  ],
  \"needs_you\": {
    \"question\": \"When should batch 3 run?\",
    \"options\": [\"Pause after batch 2\", \"Let it all run now\", \"Pause right now\"],
    \"recommended\": 0
  },
  \"waiting_on\": { \"job\": \"recap-backfill\" },
  \"scheduled\": [
    { \"when\": { \"date\": \"2026-11-01\" }, \"text\": \"Drop the old column\", \"why\": \"nothing reads it after the backfill\" }
  ],
  \"verified\": [ { \"text\": \"Unit tests\", \"why\": \"cargo test, 412 passed\" } ],
  \"not_verified\": [ { \"text\": \"Live route\", \"why\": \"needs the backfill to finish\" } ],
  \"decisions\": [ { \"text\": \"Batches of 500\", \"why\": \"keeps each write under the lock timeout\" } ]
}
```

How to fill it in:

- `state` is one of `working`, `needs_you`, `waiting`, `blocked` or `done`. Tidewatch shows exactly that word. `working`: you are on it. `needs_you`: only the user can unblock it. `waiting`: a job, a pull request, a date or an event has to happen first. `blocked`: something outside your control stops you. `done`: the whole task is finished.
- `now` is one plain sentence on what is happening right now, at most 120 characters.
- `stages` are yours: name 2 to 5 that fit this task, in order, and give each a `kind` (`understand`, `build`, `ship`, `verify` or `research`) and a `status` (`todo`, `active`, `done` or `skipped`). Work after a merge, such as a deploy, a backfill or a live check, is its own stage. Never hide it inside a finished stage and never report done before it is over.
- `steps` are the concrete steps of this task (`Read the retry code`, `Add a test for a 429`). Each names one of your stages in `stage` and has a `status` (`todo`, `active`, `done`, `skipped` or `blocked`). Give a done step its `evidence` and a blocked or skipped step its `reason`. Add, split, reword and drop steps as the work changes.
- `done` means the whole task is finished: no step is todo, active or blocked, and no stage is open. A report that says done with an open step is refused. If anything is left, the state is working, waiting, blocked or needs_you.
- A question for the user goes in `needs_you`: the `question`, 2 to 4 short `options` the user can click, and `recommended`, the index of the one you recommend. Then `state` is `needs_you`. Never leave a question only in your chat message: Tidewatch shows `needs_you` as a card with buttons, and a question that sits only in the chat is missed. When the user answers, report again.
- When you wait on something, `state` is `waiting` and `waiting_on` names exactly one of `job` (the name of a job you report to Tidewatch), `pr` (a pull request number), `date` (`YYYY-MM-DD`) or `event` (words). A job you run keeps its progress bar next to your `now` line while it runs.
- `scheduled` holds later work that is not for this turn. Its `when` is exactly one of `{\"date\": \"YYYY-MM-DD\"}`, `{\"about\": \"YYYY-MM-DD\"}` for a guess, `{\"event\": \"words\"}` when it waits for something to happen, `{\"parked\": \"until what\"}` when it is set aside, or `{\"you\": true}` when only the user can decide it. Every date or later step you tell the user in chat must also be in `scheduled`. Dates are `YYYY-MM-DD`: take today's from your environment, or run `date +%F`.
- `verified` and `not_verified` are separate lists: name the check and its result, or what you could not check and why. `decisions` hold what you decided and why.
- Send `goal` on the first report. Later reports may leave `goal` and `title` out.
- Plain ASCII in every text: no dash characters, write a hyphen.
- The tool answers with one line such as `saved: phase 2 Ship, 3/9 done, 1 scheduled`. Lines after it that start with `Warning:` name something to correct in your next call. Do not paste the report into the chat.
";

const MUST_DOS_INTRO: &str = "\
Must-do steps from the user or this repo. Add each as a step under the stage where it fits. They are a flat list, not stages:
";

/// Who answers a turn that ends with no report saved: Claude's Stop hook, or a pasted reminder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnEnd {
    SentBack,
    Reminded,
}

impl TurnEnd {
    #[must_use]
    pub fn of(wiring: Wiring) -> Self {
        if wiring == Wiring::Claude { Self::SentBack } else { Self::Reminded }
    }

    fn sentence(self) -> &'static str {
        match self {
            Self::SentBack => {
                "A turn that ends with no report saved in it is sent back to write one."
            }
            Self::Reminded => {
                "A turn that ends with no report saved in it gets a reminder pasted to you, to write one."
            }
        }
    }
}

/// The report section of an agent's house style. `must_dos` is the user's or the repo's list of
/// steps every task includes, when there is one.
#[must_use]
pub fn section(must_dos: Option<&str>, turn_end: TurnEnd) -> String {
    let must_dos = must_dos.map_or_else(String::new, |text| {
        format!("\n{MUST_DOS_INTRO}\n{}\n", flatten_headings(text.trim_end_matches('\n')))
    });
    format!("{}{must_dos}", REPORT_FORMAT.replace("{turn_end}", turn_end.sentence()))
}

/// A heading in a must-do list (`### 1. Understand`) becomes a plain bullet, so an agent that
/// copies the list never grows a stage of its own. Fenced code is left alone.
fn flatten_headings(text: &str) -> String {
    let mut fenced = false;
    let lines: Vec<String> = text
        .lines()
        .filter_map(|line| {
            if line.trim_start().starts_with("```") {
                fenced = !fenced;
            }
            match line.trim_start().strip_prefix('#') {
                Some(heading) if !fenced => {
                    let title = without_number(heading.trim_start_matches('#').trim());
                    (!title.is_empty()).then(|| format!("- {title}"))
                }
                _ => Some(line.to_owned()),
            }
        })
        .collect();
    lines.join("\n")
}

fn without_number(title: &str) -> &str {
    title
        .split_once(['.', ')'])
        .filter(|(number, _)| !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()))
        .map_or(title, |(_, rest)| rest.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::State;
    use crate::progress_report::{ProgressReport, StageKind, StageStatus, StepStatus};

    fn example_block(text: &str) -> &str {
        let start = text.find("```json\n").unwrap() + "```json\n".len();
        let end = start + text[start..].find("```").unwrap();
        &text[start..end]
    }

    fn example_args(text: &str) -> serde_json::Map<String, serde_json::Value> {
        serde_json::from_str(example_block(text)).expect("the example is JSON")
    }

    #[test]
    fn the_example_is_a_report_the_tool_accepts() {
        let text = section(None, TurnEnd::SentBack);
        let report = ProgressReport::check(&example_args(&text), None).unwrap();
        assert_eq!(report.state, State::NeedsYou);
        assert_eq!(report.stages.len(), 4);
        assert_eq!(report.needs_you.as_ref().map(|ask| ask.options.len()), Some(3));
        assert!(report.waiting_on.is_some());
        assert_eq!(report.scheduled.len(), 1);
        assert!(report.now.chars().count() <= 120);
    }

    #[test]
    fn every_state_stage_kind_and_status_word_is_named() {
        let text = section(None, TurnEnd::SentBack);
        let words = State::ALL
            .iter()
            .map(|state| state.word())
            .chain(StageKind::ALL.iter().map(|kind| kind.word()))
            .chain(StageStatus::ALL.iter().map(|status| status.word()))
            .chain(StepStatus::ALL.iter().map(|status| status.word()));
        for word in words {
            assert!(text.contains(&format!("`{word}`")), "missing {word}");
        }
    }

    #[test]
    fn the_stages_are_the_agents_own_and_after_merge_work_is_a_stage_of_its_own() {
        let text = section(None, TurnEnd::SentBack);
        assert!(text.contains("`stages` are yours: name 2 to 5 that fit this task"));
        assert!(text.contains(
            "Work after a merge, such as a deploy, a backfill or a live check, is its own stage"
        ));
        assert!(!text.contains("the same four stages"));
        assert!(!text.contains("Never rename, drop, reorder or add one"));
        assert!(!text.contains("Validate against production"));
    }

    #[test]
    fn done_means_the_whole_task_and_an_open_step_is_refused() {
        let text = section(None, TurnEnd::SentBack);
        assert!(text.contains("`done` means the whole task is finished"));
        assert!(text.contains("A report that says done with an open step is refused"));
    }

    #[test]
    fn a_question_goes_in_needs_you_and_never_only_in_the_chat() {
        let text = section(None, TurnEnd::SentBack);
        assert!(text.contains("A question for the user goes in `needs_you`"));
        assert!(text.contains("2 to 4 short `options`"));
        assert!(text.contains("Never leave a question only in your chat message"));
    }

    #[test]
    fn what_the_agent_waits_on_goes_in_waiting_on() {
        let text = section(None, TurnEnd::SentBack);
        assert!(text.contains("`waiting_on` names exactly one of `job`"));
        for word in ["`pr`", "`date`", "`event`"] {
            assert!(text.contains(word), "{word}");
        }
    }

    #[test]
    fn every_scheduled_when_has_a_typed_shape_in_the_text() {
        let text = section(None, TurnEnd::SentBack);
        for shape in [
            "{\"date\": \"YYYY-MM-DD\"}",
            "{\"about\": \"YYYY-MM-DD\"}",
            "{\"event\": \"words\"}",
            "{\"parked\": \"until what\"}",
            "{\"you\": true}",
        ] {
            assert!(text.contains(shape), "{shape}");
        }
        assert!(text.contains(
            "Every date or later step you tell the user in chat must also be in `scheduled`"
        ));
    }

    #[test]
    fn the_report_is_required_and_a_context_reset_starts_from_get_progress() {
        let text = section(None, TurnEnd::SentBack);
        assert!(text.contains("This report is required."));
        assert!(text.contains("A notes file of your own"));
        assert!(text.contains("call `get_progress` first"));
    }

    #[test]
    fn a_list_of_must_dos_comes_after_the_format_and_never_inside_it() {
        let text = section(Some("- [ ] only this\n"), TurnEnd::SentBack);
        let at = text.find("Must-do steps from the user or this repo").unwrap();
        assert!(at > text.rfind("```").unwrap());
        assert!(text.ends_with("- [ ] only this\n"), "{text}");
        assert!(ProgressReport::check(&example_args(&text), None).is_ok());
    }

    #[test]
    fn must_do_headings_become_plain_bullets_and_never_look_like_stages() {
        let cases = [
            (
                "a numbered heading",
                "### 1. Understand\n- [ ] read it",
                "- Understand\n- [ ] read it",
            ),
            (
                "a bare heading",
                "## Before merge\n- [ ] run tests",
                "- Before merge\n- [ ] run tests",
            ),
            ("a paren number", "# 2) Ship", "- Ship"),
            ("an empty heading", "###\n- [ ] a", "- [ ] a"),
            ("a title with a dot inside", "### Fix v1.2 notes", "- Fix v1.2 notes"),
            ("code is left alone", "```sh\n# a comment\n```", "```sh\n# a comment\n```"),
            ("plain bullets", "- [ ] a\n- [ ] b", "- [ ] a\n- [ ] b"),
        ];
        for (name, input, want) in cases {
            assert_eq!(flatten_headings(input), want, "{name}");
        }
        let ten_phases = "### 1. Phase\n- [ ] x\n".repeat(10);
        let text = section(Some(&ten_phases), TurnEnd::SentBack);
        let after = &text[text.find("Must-do steps").unwrap()..];
        assert!(!after.contains('#'), "{after}");
    }

    #[test]
    fn what_happens_to_a_turn_with_no_report_is_told_per_cli_and_is_true_of_it() {
        let sent_back = section(None, TurnEnd::of(Wiring::Claude));
        assert!(sent_back.contains("is sent back to write one."));
        assert!(!sent_back.contains("reminder"));
        for wiring in [Wiring::Codex, Wiring::Gemini, Wiring::Copilot] {
            let text = section(None, TurnEnd::of(wiring));
            assert!(text.contains("gets a reminder pasted to you, to write one."), "{wiring:?}");
            assert!(!text.contains("sent back"), "{wiring:?}");
        }
        for turn_end in [TurnEnd::SentBack, TurnEnd::Reminded] {
            assert!(!section(None, turn_end).contains("{turn_end}"));
        }
    }

    #[test]
    fn every_cli_gets_the_same_text_but_for_the_one_sentence_on_a_turn_with_no_report() {
        let claude = section(None, TurnEnd::of(Wiring::Claude));
        for wiring in [Wiring::Codex, Wiring::Gemini, Wiring::Copilot] {
            let other = section(None, TurnEnd::of(wiring));
            assert_eq!(
                other.replace(TurnEnd::Reminded.sentence(), TurnEnd::SentBack.sentence()),
                claude,
                "{wiring:?}"
            );
        }
    }

    #[test]
    fn it_is_plain_ascii_and_names_both_tools() {
        for text in
            [section(None, TurnEnd::SentBack), section(Some("- [ ] a\n"), TurnEnd::Reminded)]
        {
            assert!(text.is_ascii());
            assert!(text.contains("update_progress"));
            assert!(text.contains("get_progress"));
        }
    }
}
