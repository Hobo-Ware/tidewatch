//! The instruction that puts the progress ledger in front of an agent: how to keep it, the format
//! with the repo's checklist filled in, and one example for every kind of `<when>` (pure).

const BEFORE_CHECKLIST: &str = "\
## Progress ledger format

Tidewatch shows the user a live progress view of your task. It is built from a ledger you keep with the `update_progress` tool of the `tidewatch` MCP server.

- Write the ledger first, before any other work: call `update_progress` with the whole ledger, the goal in one line, the checklist below and the phase you start in marked in progress.
- Call `update_progress` again at every milestone: a decision made, an item finished, a check run, a blocker, a change of phase. Each call replaces the ledger, so always send all of it.
- After a context reset, or when you resume a session, call `get_progress` first and carry on from what it returns. Never start the ledger over.
- If the tool answers with an error, fix what it says and call again.

The `ledger` argument is markdown in exactly this format:

```markdown
# Goal
<the goal in one line>

## Status
phase: <number of the phase you are in>
state: working | waiting | blocked | done
note: <one short line on what is happening now>

## Checklist
";

const AFTER_CHECKLIST: &str = "\
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

- Checklist marks: `[ ]` to do, `[~]` in progress, `[x]` done, `[-]` skipped, `[!]` blocked. After ` -- ` give the reason for a skipped or blocked item. Keep the phase headings and the item wording; add items under a phase when the task needs them. A phase that does not apply is marked skipped with a reason, never dropped.
- Separate a line from its reason or evidence with ` -- ` (two hyphens between spaces). Never use a dash character.
- Needs you holds only what the user must answer or approve. Write `nothing` when there is none.
- Scheduled holds open work that is not for now. Leave the section out when there is none. `<when>` is one of these five, with an example of each:
  - an exact date: `- [ ] 2026-11-01 -- Switch off the old webhook -- the main remaining milestone`
  - an about date, for \"in a few weeks\" and the like (work out a concrete date from today): `- [ ] ~2026-11-07 -- Count which apps still send the old field -- needed before it is dropped`
  - `on: <event>` when it waits for something to happen: `- [ ] on: the partner confirms the new feed -- Delete the legacy endpoint -- the partner still reads it`
  - `parked: <until what>` when it is set aside: `- [ ] parked: until the billing rewrite lands -- Drop the invoice columns -- the old billing code reads them`
  - `you` when only the user can decide it. Never write `on: <event>` for the user's call: `- [ ] you -- Choose whether audit rows are kept for 30 or 90 days -- a product call`
- Dates are `YYYY-MM-DD`. Take today's date from your environment, or run `date +%F`.
- The tool answers with one line such as `saved: phase 2 Build, 3/9 done, 1 scheduled`. Do not paste the ledger into the chat.
";

/// The ledger section of an agent's house style, with `template` (the checklist's phases and
/// items) under `## Checklist`.
#[must_use]
pub fn section(template: &str) -> String {
    let template = template.trim_end_matches('\n');
    format!("{BEFORE_CHECKLIST}{template}\n\n{AFTER_CHECKLIST}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checklist::DEFAULT_CHECKLIST;
    use crate::progress::{Ledger, When};

    fn format_block(text: &str) -> &str {
        let start = text.find("```markdown\n").unwrap() + "```markdown\n".len();
        let end = start + text[start..].find("```").unwrap();
        &text[start..end]
    }

    #[test]
    fn the_format_block_is_a_ledger_with_the_template_under_its_checklist() {
        let text = section(DEFAULT_CHECKLIST);
        let ledger = Ledger::parse(format_block(&text));
        assert_eq!(ledger.goal, "<the goal in one line>");
        assert_eq!(ledger.checklist.len(), 10);
        assert_eq!(ledger.checklist, crate::progress::checklist_phases(DEFAULT_CHECKLIST));
        assert_eq!(ledger.scheduled.len(), 1);
        assert_eq!(ledger.scheduled[0].steps.len(), 1);
        assert_eq!(ledger.needs_you.len(), 1);
    }

    #[test]
    fn a_repos_own_template_replaces_the_default() {
        let text = section("### 1. Mine\n- [ ] only this\n");
        let ledger = Ledger::parse(format_block(&text));
        assert_eq!(ledger.checklist.len(), 1);
        assert_eq!(ledger.checklist[0].title, "Mine");
        assert!(!text.contains("Understand and decide"));
    }

    #[test]
    fn every_when_has_an_example_that_reads_back_as_that_kind() {
        let text = section(DEFAULT_CHECKLIST);
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
    fn it_is_plain_ascii_and_names_both_tools() {
        let text = section(DEFAULT_CHECKLIST);
        assert!(text.is_ascii());
        assert!(text.contains("update_progress"));
        assert!(text.contains("get_progress"));
    }
}
