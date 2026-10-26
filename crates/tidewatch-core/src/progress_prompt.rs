//! The instruction that puts the progress ledger in front of an agent: how to keep it, the format
//! with the four stages, the user's must-do steps when there are any, and one example for every
//! kind of `<when>` (pure).

const LEDGER_FORMAT: &str = "\
## Progress ledger format

Tidewatch shows the user a live progress view of your task. It is built from a ledger you keep with the `update_progress` tool of the `tidewatch` MCP server.

- Write the ledger first, before any other work: call `update_progress` with the whole ledger: the goal in one line, your plan as steps under the four stages below, and the stage you start in.
- Call `update_progress` again at every milestone: a decision made, an item finished, a check run, a blocker, a change of stage. Each call replaces the ledger, so always send all of it.
- After a context reset, or when you resume a session, call `get_progress` first and carry on from what it returns. Never start the ledger over.
- If the tool answers with an error, fix what it says and call again.

The `ledger` argument is markdown in exactly this format:

```markdown
# Goal
<the goal in one line>

## Status
phase: <number of the stage you are in, 1 to 4>
state: working | waiting | blocked | done
note: <one short line on what is happening now>

## Checklist
### 1. Understand
- [ ] <a step>
### 2. Build
- [ ] <a step>
### 3. PR
- [ ] <a step>
### 4. Done
- [ ] <a step>

## Decisions
- <decision> -- <why>

## Verified
- <what was checked> -- <evidence: the command and its result>

## Not verified
- <what could not be checked> -- <why>

## Next
- <next step>

## Needs you
- <a question or approval only the user can give, or nothing>

## Scheduled
- [ ] <when> -- <what> -- <why>
  - [ ] <a sub-step, indented two spaces>
```

How to fill it in:

- Every session has the same four stages: keep them as the `###` headings above, in that order. Never rename, drop, reorder or add one.
- The steps are yours. Under each stage write the concrete steps of this task (`Read the retry code`, `Add a test for a 429`), not generic ones. Add, split, reword and drop steps as the work changes, and mark them as you go. A stage that does not apply keeps its heading with one step marked skipped and a reason.
- Checklist marks: `[ ]` to do, `[~]` in progress, `[x]` done, `[-]` skipped, `[!]` blocked. After ` -- ` give the reason for a skipped or blocked item.
- Separate a line from its reason or evidence with ` -- ` (two hyphens between spaces). Never use a dash character.
- Needs you holds only what the user must answer or approve. Write `nothing` when there is none.
- Next holds only what you do in this same turn, right after the call. Work that waits for something goes in Scheduled, never in Next: a merge, an approval, a deploy, a date, or the user. Give it the `<when>` that says what it waits for, for example `on: #1999 merges` or `on: the owner approves #1999`. Work meant for later today gets today's date.
- Before you set state to waiting, blocked or done, move every Next item that is not happening right now into Scheduled. Tidewatch lists only Scheduled in Coming up, so work left in Next is easy to forget.
- Scheduled holds open work that is not for now. Leave the section out when there is none. `<when>` is one of these five, with an example of each:
  - an exact date: `- [ ] 2026-11-01 -- Switch off the old webhook -- the main remaining milestone`
  - an about date, for \"in a few weeks\" and the like (work out a concrete date from today): `- [ ] ~2026-11-07 -- Count which apps still send the old field -- needed before it is dropped`
  - `on: <event>` when it waits for something to happen: `- [ ] on: the partner confirms the new feed -- Delete the legacy endpoint -- the partner still reads it`
  - `parked: <until what>` when it is set aside: `- [ ] parked: until the billing rewrite lands -- Drop the invoice columns -- the old billing code reads them`
  - `you` when only the user can decide it. Never write `on: <event>` for the user's call: `- [ ] you -- Choose whether audit rows are kept for 30 or 90 days -- a product call`
- Dates are `YYYY-MM-DD`. Take today's date from your environment, or run `date +%F`.
- The tool answers with one line such as `saved: phase 2 Build, 3/9 done, 1 scheduled`. Do not paste the ledger into the chat.
";

const MUST_DOS_INTRO: &str = "\
Must-do steps from the user or this repo. Add each as a step under the stage where it fits; any headings in them are only hints, never stages:
";

/// The ledger section of an agent's house style. `must_dos` is the user's or the repo's list of
/// steps every task includes, when there is one.
#[must_use]
pub fn section(must_dos: Option<&str>) -> String {
    let must_dos = must_dos.map_or_else(String::new, |text| {
        format!("\n{MUST_DOS_INTRO}\n{}\n", text.trim_end_matches('\n'))
    });
    format!("{LEDGER_FORMAT}{must_dos}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::{Ledger, When};
    use crate::stages::Stage;

    fn format_block(text: &str) -> &str {
        let start = text.find("```markdown\n").unwrap() + "```markdown\n".len();
        let end = start + text[start..].find("```").unwrap();
        &text[start..end]
    }

    #[test]
    fn the_format_block_is_a_ledger_with_the_four_stages_under_its_checklist() {
        let text = section(None);
        let ledger = Ledger::parse(format_block(&text));
        assert_eq!(ledger.goal, "<the goal in one line>");
        let titles: Vec<_> =
            ledger.checklist.iter().map(|p| (p.number, p.title.as_str())).collect();
        assert_eq!(
            titles,
            [(Some(1), "Understand"), (Some(2), "Build"), (Some(3), "PR"), (Some(4), "Done")]
        );
        assert!(ledger.checklist.iter().all(|phase| phase.items.len() == 1));
        assert_eq!(ledger.scheduled.len(), 1);
        assert_eq!(ledger.scheduled[0].steps.len(), 1);
        assert_eq!(ledger.needs_you.len(), 1);
    }

    #[test]
    fn the_prompt_names_the_four_stages_and_tells_the_agent_the_steps_are_its_own() {
        let text = section(None);
        for stage in Stage::ALL {
            assert!(text.contains(&format!(". {}\n", stage.label())), "{stage:?}");
        }
        assert!(text.contains("the same four stages"));
        assert!(text.contains("The steps are yours"));
        assert!(text.contains("Add, split, reword and drop steps as the work changes"));
        assert!(!text.contains("Must-do"), "no list, no must-do paragraph");
        assert!(!text.contains("Validate against production"));
    }

    #[test]
    fn a_list_of_must_dos_comes_after_the_format_and_never_inside_it() {
        let text = section(Some("### 1. Mine\n- [ ] only this\n"));
        let at = text.find("Must-do steps from the user or this repo").unwrap();
        assert!(at > text.rfind("```").unwrap());
        assert!(text.ends_with("### 1. Mine\n- [ ] only this\n"), "{text}");
        let ledger = Ledger::parse(format_block(&text));
        assert_eq!(ledger.checklist.len(), 4);
    }

    #[test]
    fn every_when_has_an_example_that_reads_back_as_that_kind() {
        let text = section(None);
        let examples: Vec<&str> = text
            .lines()
            .filter_map(|line| line.trim_start().strip_prefix("- ")?.split_once(": `- [ ] "))
            .filter_map(|(_, rest)| rest.strip_suffix('`'))
            .collect();
        let kinds: Vec<When> = examples
            .iter()
            .map(|example| {
                let ledger =
                    Ledger::parse(&format!("# Goal\ng\n\n## Scheduled\n- [ ] {example}\n"));
                assert_eq!(ledger.scheduled.len(), 1, "{example}");
                assert!(ledger.scheduled[0].why.is_some(), "{example}");
                ledger.scheduled[0].when.clone()
            })
            .collect();
        assert!(matches!(kinds[0], When::Date(_)), "{kinds:?}");
        assert!(matches!(kinds[1], When::About(_)), "{kinds:?}");
        assert!(matches!(kinds[2], When::On(_)), "{kinds:?}");
        assert!(matches!(kinds[3], When::Parked(_)), "{kinds:?}");
        assert_eq!(kinds[4], When::You);
        assert_eq!(kinds.len(), 5);
    }

    #[test]
    fn next_is_this_turn_only_and_waiting_work_moves_to_scheduled_before_a_stop() {
        let text = section(None);
        assert!(text.contains("Next holds only what you do in this same turn"));
        assert!(text.contains("Work meant for later today gets today's date"));
        assert!(text.contains(
            "Before you set state to waiting, blocked or done, move every Next item that is not happening right now into Scheduled"
        ));
    }

    #[test]
    fn it_is_plain_ascii_and_names_both_tools() {
        for text in [section(None), section(Some("- [ ] a\n"))] {
            assert!(text.is_ascii());
            assert!(text.contains("update_progress"));
            assert!(text.contains("get_progress"));
        }
    }
}
