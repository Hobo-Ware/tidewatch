---
trigger: always_on
globs: 'crates/**'
description: 'One channel per job between Tidewatch, the agent and the owner; Tidewatch reports facts, the agent decides; how to add a sender or a surface without making a second path.'
applyTo: 'crates/**'
---

# Agent channels

Tidewatch is plumbing. It sees events, shows them, and passes facts on. The agent is the smart
part: it decides what to do next, watches what it started and schedules its own wake-ups. Until
0.2.0 every new agent CLI or channel got its own path and none was retired, which left 9 ways
to show a choice, about 14 status sources and 18 senders. This file keeps it from coming back.

## One channel per job

| Job | The one channel | Where |
| --- | --- | --- |
| Show the owner a choice | The question card: one model with an answer route (Claude menu, held `ask_user` call, typed chat), one widget, one answer function, a "Let's chat about this" button | `question_view.rs`, `tidewatch-core/src/question.rs` |
| Live state (working, needs you, done, failed) | Hooks and terminal signals, folded by the status tracker | `tidewatch-core/src/status.rs` |
| Task state (stages, the checklist with dates, done, waiting, blocked) | The agent's `update_progress` report, nothing parsed from chat text. Its steps are the one checklist of the whole goal: later work is a step with a `when`, never a second list; Coming up, due today and the Overview cards read those steps | `tidewatch-core/src/progress_report.rs` |
| PR and issue news to the agent | The tab's `$TIDEWATCH_EVENTS` file: a JSON line when a link's state, checks, review, conflicts or comment count changes, whichever source saw it (webhook, timed lookup, PR watch); the agent watches it itself | `tidewatch-core/src/github_feed.rs`, `link_glue.rs` |
| Job progress | `report_job`, or JSON lines in `$TIDEWATCH_PROGRESS`; never scraped from the terminal | `job_mcp.rs`, `job_glue.rs` |
| Text into an agent | `flow_glue::send_or_queue` with a `Reason`, drawn as the owner's bubble with the Tidewatch badge | `outbox.rs`, `flow_glue.rs`, `tidewatch-core/src/sent.rs` |
| What the agent should do on its own | The launch text, written once per tab | `tidewatch-core/src/house_style.rs`, `prepare.rs` |

## Rules

- **Extend, never add a second.** Before you add a card, chip, panel block, status source, parse
  of agent text, or sender, find its row above and extend that channel. If a job truly needs a
  new channel, remove the old one in the same PR and change the row.
- **Facts, not instructions.** Rust code never writes what the agent should do next ("fix the
  checks, then push"). A message states what happened ("PR #42: checks failed: build"). The
  agent was told at launch how to react.
- **Delegate at launch, nudge only as a fallback.** Anything the agent can do itself (watch its
  PR, CI and reviews, wait for a job, wake itself up while the owner is away) goes into the launch
  text once. Tidewatch sends a fact only when an event lands and the agent has not reacted after
  a grace period, and at most once per event.
- **One send budget.** Every automatic send goes through the outbox and counts against the tab's
  one budget. No sender gets its own counter or its own queue.
- **Speak as the owner.** A message to the agent reads in the owner's voice and is drawn as the
  owner's bubble with the Tidewatch badge, with its reason as a quiet caption. Every automatic
  send carries a `Reason`; a send without a record would show as the owner's own words.
- **Hidden context is for resume only.** Hook context lines (`additionalContext`, SessionStart)
  carry a refresh after a resume or compaction, not reminders on every prompt.
- **Each tool asked for once.** The launch text names each tool in one place. A tool that is
  removed comes out of the launch text, the house style default and `dead_tools.rs` in the same
  commit.
- A test checks every `Reason` tag appears in this file: a new sender adds its row or line here.

## Senders (each `Reason`)

Kept current by the test in `tidewatch-core/src/sent.rs`.

- `auto`: Auto mode on a pull request, sent only when the agent has not picked up a failed check,
  review or conflict itself.
- `pull request`: the owner asked for a push or a PR from a card, or the PR was merged or closed.
- `Meeseeks check`: a Meeseeks tab's check failed; the fact of what failed.
- `due today`: a report item is due today (the owner clicked Ask, or the tab runs on its own).
- `report check`: the turn ended with the progress report not saved.
- `comment guard`: new code comments on the branch broke the repo's comment rule.
- `done check`: what the done check found (commits not pushed, checks failing).
- `avatar`: the tab has a goal and no icon yet, or the owner asked for a new avatar.
- `workflow`: the owner ran Build workflow from the session.
- `resume`: the tab's agent was resumed after a restart or crash.
- `stuck job`: the owner pressed Ask agent on a stuck job.
- `background job`: a background command has not reported progress.
- `away`: the owner turned away mode on or off.
- `keep going`: the fallback nudge while the owner is away and the agent stopped with open
  steps and no wake-up set.
