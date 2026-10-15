//! "Build workflow from this session": the prompt that asks an agent to turn its session into a
//! workflow, the `propose_workflow` tool's arguments, the rules a proposal must pass and how a
//! proposal is pulled out of a reply when the agent answers in text.
//!
//! The schema part of the prompt is written from [`Action`], [`Gate`], [`Level`] and
//! [`prompt::PLACEHOLDERS`], so a new variant shows up in the prompt and in the tool's JSON Schema
//! without anyone remembering to add it; a test fails when one has no description here.

use std::fmt::Write;

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::agents::{AgentTable, Level};
use crate::launch::MCP_SERVER_NAME;
use crate::prompt;
use crate::workflows::{Action, DEFAULT_MAX_LOOPS, Gate, Stage, Workflow, Workflows};

/// The MCP tool the agent calls with its proposal.
pub const TOOL_NAME: &str = "propose_workflow";
/// The most times a failed gate may send a stage back (the editor's limit too).
pub const MAX_LOOPS_LIMIT: u8 = 9;

const NAME_KEY: &str = "name";
const DESCRIPTION_KEY: &str = "description";
const STAGES_KEY: &str = "stages";
const STAGE_KEYS: [&str; 8] =
    ["action", "prompt", "gate", "agent", "permissions", "on_fail", "max_loops", "approve"];

fn key_of<T: Serialize>(value: T) -> String {
    serde_json::to_value(value).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

fn action_help(action: Action) -> &'static str {
    match action {
        Action::Work => "do the task: edit files, run commands, commit in small steps",
        Action::Review => {
            "read only: inspect the changes and report each problem with the report_finding tool"
        }
        Action::Refactor => "simplify the changed code without changing its behavior",
        Action::Reproduce => "write one test that fails because of the bug, and do not fix it yet",
        Action::Fix => "make the failing test pass with the smallest change",
        Action::Handoff => "read only: write the summary or pull request text for the user",
    }
}

fn gate_help(gate: Gate) -> &'static str {
    match gate {
        Gate::Tests => "the repo's test command must pass",
        Gate::TestFails => "the test command must fail, which proves the new test catches the bug",
        Gate::TestPasses => "the test command must pass, the new test and the rest",
        Gate::NoBlocking => "the stage must not have reported a blocking finding",
        Gate::SameBehavior => "the test command must pass, so the old behavior is intact",
        Gate::Approve => "the user must approve before the next stage starts",
    }
}

fn placeholder_help(name: &str) -> &'static str {
    match name {
        "task" => "what the user asks for when they start the workflow",
        "base" => "the branch the work is compared against",
        "branch" => "the branch of this run",
        "previous_summary" => "the summary the last finished stage reported",
        "prev" => "short for {previous_summary}",
        "findings" => "the findings other stages reported, one per line",
        "test_output" => "the tail of the last failing test run",
        _ => "",
    }
}

fn level_help(level: Level) -> &'static str {
    match level {
        Level::Plan => "read only",
        Level::Ask => "asks before each change",
        Level::Edits => "edits files freely, asks before commands",
        Level::Auto => "runs commands and edits without asking",
        Level::Yolo => "no limits",
    }
}

fn proposable_levels() -> impl Iterator<Item = Level> {
    Level::ALL.into_iter().filter(|level| *level != Level::Yolo)
}

fn actions_text() -> String {
    let mut out = String::new();
    for action in Action::ALL {
        let _ = writeln!(
            out,
            "  - {}: {}. Default gate: {}. Default permissions: {}.",
            key_of(action),
            action_help(action),
            key_of(action.default_gate()),
            key_of(action.default_level()),
        );
    }
    out.trim_end().to_owned()
}

fn gates_text() -> String {
    Gate::ALL
        .into_iter()
        .map(|gate| format!("  - {}: {}.", key_of(gate), gate_help(gate)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn levels_text() -> String {
    proposable_levels()
        .map(|level| format!("{} ({})", key_of(level), level_help(level)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn placeholders_text() -> String {
    prompt::PLACEHOLDERS
        .iter()
        .map(|name| format!("- {{{name}}}: {}.", placeholder_help(name)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn agent_ids() -> Vec<String> {
    AgentTable::builtin().iter().map(|agent| agent.id.clone()).collect()
}

/// A small valid workflow in the `workflows.toml` format, shown in the prompt as the example.
fn example_toml() -> String {
    let work = Stage {
        prompt: Some(
            "{task}\n\nKeep the change small and match the style of the surrounding code. Run the \
             checks before you commit."
                .to_owned(),
        ),
        ..Stage::new(Action::Work)
    };
    let review = Stage {
        on_fail: Some(0),
        prompt: Some(
            "Review {base}..HEAD for bugs and missing tests. Report each problem as a finding, \
             blocking or not. Do not edit files."
                .to_owned(),
        ),
        ..Stage::new(Action::Review)
    };
    let handoff = Stage {
        prompt: Some(
            "Write a short summary of {base}..HEAD for the user: what changed and how it was \
             checked."
                .to_owned(),
        ),
        ..Stage::new(Action::Handoff)
    };
    let workflow = Workflow {
        id: "example".to_owned(),
        name: "Change with a review".to_owned(),
        description: "Make a change, get it reviewed, then summarize it.".to_owned(),
        stages: vec![work, review, handoff],
    };
    let workflows = Workflows { workflow: [(workflow.id.clone(), workflow)].into() };
    toml::to_string(&workflows).unwrap_or_default().trim_end().to_owned()
}

const PROMPT_TEMPLATE: &str = "\
Review this whole session: the goal you were given, what you did in order, where you got stuck, and which checks proved the result. Then turn the repeatable part of it into a reusable Tidewatch workflow for a NEW task, not for this one.

A workflow is a short list of stages. Tidewatch runs them one after the other in a git worktree, each stage as its own agent run that sees only that stage's prompt. Between stages Tidewatch checks a gate itself and never takes the agent's word for it.

A workflow has:
- name: a few words, required.
- description: one sentence on what it is for.
- stages: two to five is usual.

A stage has:
- action (required): what the stage does. One of:
<<ACTIONS>>
- prompt (required): the instruction for this stage, in your own words, written for a NEW task.
- gate: what Tidewatch checks before the next stage starts. Leave it out for the action's default. One of:
<<GATES>>
- on_fail: when the gate fails, go back to this earlier stage. Stages count from 1, and it must name an earlier stage. Without it Tidewatch stops and asks the user.
- max_loops: how many times a failed gate may send the work back, 1 to <<MAX_LOOPS>>. The default is <<DEFAULT_LOOPS>>.
- approve: true to wait for the user's approval after the gate passes, before the next stage starts.
- agent: which CLI runs the stage, one of <<AGENTS>>. Leave it out to use the agent you are, which is the right choice unless a stage clearly suits another one.
- permissions: <<LEVELS>>. Leave it out for the action's default.

Placeholders you can put in a prompt, each filled in when the stage starts:
<<PLACEHOLDERS>>
Use {task} in the first stage so the user's next request lands there. Any other word in braces is left as written.

Write for any project:
- No names, paths, URLs, ticket numbers, branch names or secrets from this session. Say what to do, not where this session did it.
- Keep each prompt short and imperative. Put the lessons from where you got stuck into the prompts of the stages they apply to.
- Let a gate prove the result where this session used a check, and use a review stage only where a second look paid off.

Example of the format:

```toml
<<EXAMPLE>>
```

Now send the workflow. Call the Tidewatch MCP tool `<<TOOL>>` (your CLI may list it as `mcp__<<SERVER>>__<<TOOL>>`) with name, description and stages, the stages in the same fields as above. It checks the workflow and answers with what to fix, so fix it and call it again. If that tool is not available, reply with exactly one fenced ```toml block in the format of the example, and put nothing else in the reply that looks like a code block. Do not edit files or run commands for this. The workflow opens as a draft for the user to review, and nothing is saved until they press Save.";

/// The text pasted into the agent's terminal to ask for a workflow.
#[must_use]
pub fn prompt() -> String {
    PROMPT_TEMPLATE
        .replace("<<ACTIONS>>", &actions_text())
        .replace("<<GATES>>", &gates_text())
        .replace("<<PLACEHOLDERS>>", &placeholders_text())
        .replace("<<LEVELS>>", &levels_text())
        .replace("<<AGENTS>>", &agent_ids().join(", "))
        .replace("<<MAX_LOOPS>>", &MAX_LOOPS_LIMIT.to_string())
        .replace("<<DEFAULT_LOOPS>>", &DEFAULT_MAX_LOOPS.to_string())
        .replace("<<SERVER>>", MCP_SERVER_NAME)
        .replace("<<TOOL>>", TOOL_NAME)
        .replace("<<EXAMPLE>>", &example_toml())
}

/// The tool's entry for `tools/list`.
#[must_use]
pub fn tool_definition() -> Value {
    let actions: Vec<String> = Action::ALL.into_iter().map(key_of).collect();
    let gates: Vec<String> = Gate::ALL.into_iter().map(key_of).collect();
    let levels: Vec<String> = proposable_levels().map(key_of).collect();
    json!({
        "name": TOOL_NAME,
        "description": "Propose a reusable Tidewatch workflow built from this session. It opens in Tidewatch's Workflows editor as an unsaved draft for the user to review. Returns what to fix when the workflow is not valid, so call it again with the fix.",
        "inputSchema": {
            "type": "object",
            "properties": {
                NAME_KEY: { "type": "string", "description": "A few words naming the workflow." },
                DESCRIPTION_KEY: { "type": "string", "description": "One sentence on what the workflow is for." },
                STAGES_KEY: {
                    "type": "array",
                    "minItems": 1,
                    "description": "The stages, run one after the other.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "action": { "type": "string", "enum": actions, "description": "What the stage does." },
                            "prompt": { "type": "string", "description": "The stage's instruction for a new task. May use {task} {base} {branch} {previous_summary} {findings} {test_output}." },
                            "gate": { "type": "string", "enum": gates, "description": "What Tidewatch checks before the next stage. Omit for the action's default." },
                            "agent": { "type": "string", "enum": agent_ids(), "description": "The CLI that runs the stage. Omit to use the calling agent." },
                            "permissions": { "type": "string", "enum": levels, "description": "Permission level. Omit for the action's default." },
                            "on_fail": { "type": "integer", "minimum": 1, "description": "When the gate fails, go back to this earlier stage, counted from 1." },
                            "max_loops": { "type": "integer", "minimum": 1, "maximum": MAX_LOOPS_LIMIT, "description": "How many times a failed gate may send the work back." },
                            "approve": { "type": "boolean", "description": "Wait for the user's approval after the gate passes." },
                        },
                        "required": ["action", "prompt"],
                        "additionalProperties": false,
                    },
                },
            },
            "required": [NAME_KEY, STAGES_KEY],
            "additionalProperties": false,
        },
    })
}

fn string_field(object: &Map<String, Value>, key: &str, context: &str) -> Result<String, String> {
    match object.get(key) {
        Some(Value::String(text)) if !text.trim().is_empty() => Ok(text.trim().to_owned()),
        _ => Err(format!("{context}{key} must be a non-empty string")),
    }
}

fn choice<T: serde::de::DeserializeOwned>(
    object: &Map<String, Value>,
    key: &str,
    context: &str,
    allowed: &[String],
) -> Result<Option<T>, String> {
    let Some(value) = object.get(key) else { return Ok(None) };
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|_| format!("{context}{key} must be one of: {}", allowed.join(", ")))
}

fn unknown_keys(object: &Map<String, Value>, known: &[&str], context: &str) -> Vec<String> {
    object
        .keys()
        .filter(|key| !known.contains(&key.as_str()))
        .map(|key| format!("{context}unknown field \"{key}\""))
        .collect()
}

fn stage_from(object: &Map<String, Value>, number: usize, problems: &mut Vec<String>) -> Stage {
    let context = format!("stage {number}: ");
    problems.extend(unknown_keys(object, &STAGE_KEYS, &context));
    let actions: Vec<String> = Action::ALL.into_iter().map(key_of).collect();
    let gates: Vec<String> = Gate::ALL.into_iter().map(key_of).collect();
    let levels: Vec<String> = proposable_levels().map(key_of).collect();
    let action = match choice::<Action>(object, "action", &context, &actions) {
        Ok(Some(action)) => action,
        Ok(None) => {
            problems.push(format!("{context}action is required, one of: {}", actions.join(", ")));
            Action::Work
        }
        Err(problem) => {
            problems.push(problem);
            Action::Work
        }
    };
    let mut stage = Stage::new(action);
    match string_field(object, "prompt", &context) {
        Ok(text) => stage.prompt = Some(text),
        Err(problem) => problems.push(problem),
    }
    match choice::<Gate>(object, "gate", &context, &gates) {
        Ok(gate) => stage.gate = gate.unwrap_or(stage.gate),
        Err(problem) => problems.push(problem),
    }
    match choice::<Level>(object, "permissions", &context, &levels) {
        Ok(Some(Level::Yolo)) => {
            problems.push(format!("{context}permissions must be one of: {}", levels.join(", ")));
        }
        Ok(level) => stage.run.level = level,
        Err(problem) => problems.push(problem),
    }
    if let Some(value) = object.get("agent") {
        match value.as_str().map(str::trim).filter(|agent| !agent.is_empty()) {
            Some(agent) => stage.run.agent = Some(agent.to_owned()),
            None => problems.push(format!("{context}agent must be a non-empty string")),
        }
    }
    if let Some(value) = object.get("on_fail") {
        match value.as_u64().and_then(|n| usize::try_from(n).ok()).filter(|n| *n >= 1) {
            Some(number) => stage.on_fail = Some(number - 1),
            None => problems.push(format!("{context}on_fail must be a stage number from 1")),
        }
    }
    if let Some(value) = object.get("max_loops") {
        match value.as_u64().and_then(|n| u8::try_from(n).ok()) {
            Some(loops) => stage.max_loops = loops,
            None => problems.push(format!("{context}max_loops must be a number")),
        }
    }
    if let Some(value) = object.get("approve") {
        match value.as_bool() {
            Some(approve) => stage.approve = approve,
            None => problems.push(format!("{context}approve must be true or false")),
        }
    }
    stage
}

/// The workflow the tool's arguments describe, or every way they do not. The agent of a stage that
/// names none is left empty; [`with_agent`] fills it.
pub fn parse_args(args: &Map<String, Value>) -> Result<Workflow, Vec<String>> {
    let mut problems = unknown_keys(args, &[NAME_KEY, DESCRIPTION_KEY, STAGES_KEY], "");
    let name = string_field(args, NAME_KEY, "").unwrap_or_else(|problem| {
        problems.push(problem);
        String::new()
    });
    let description = match args.get(DESCRIPTION_KEY) {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.trim().to_owned(),
        Some(_) => {
            problems.push("description must be a string".to_owned());
            String::new()
        }
    };
    let mut stages = Vec::new();
    match args.get(STAGES_KEY) {
        Some(Value::Array(items)) => {
            for (index, item) in items.iter().enumerate() {
                match item.as_object() {
                    Some(object) => stages.push(stage_from(object, index + 1, &mut problems)),
                    None => problems.push(format!("stage {}: must be an object", index + 1)),
                }
            }
        }
        _ => problems.push("stages must be a list of stages".to_owned()),
    }
    if problems.is_empty() {
        Ok(Workflow { id: String::new(), name, description, stages })
    } else {
        Err(problems)
    }
}

/// Gives every stage that names no agent and no profile `agent`: the one that proposed the workflow.
pub fn with_agent(workflow: &mut Workflow, agent: &str) {
    for stage in &mut workflow.stages {
        if stage.run.agent.is_none() && stage.run.profile.is_none() {
            stage.run.agent = Some(agent.to_owned());
        }
    }
}

/// What stops the proposal from becoming a draft, in words for the agent and the toast: the
/// editor's rules, plus the ones that keep a proposal safe to review (no YOLO, no profiles, which
/// are the user's own).
#[must_use]
pub fn problems(workflow: &Workflow) -> Vec<String> {
    let mut problems = Vec::new();
    if workflow.name.trim().is_empty() {
        problems.push("the workflow needs a name".to_owned());
    }
    problems.extend(workflow.validate());
    let agents = agent_ids();
    for (index, stage) in workflow.stages.iter().enumerate() {
        let label = format!("stage {} ({})", index + 1, stage.action.label());
        if !(1..=MAX_LOOPS_LIMIT).contains(&stage.max_loops) {
            problems.push(format!("{label}: loops must be a number from 1 to {MAX_LOOPS_LIMIT}"));
        }
        if stage.run.level == Some(Level::Yolo) {
            problems.push(format!("{label}: a proposal cannot use YOLO permissions"));
        }
        if stage.run.profile.is_some() {
            problems.push(format!("{label}: leave profile out, profiles are the user's own"));
        }
        if let Some(agent) = &stage.run.agent
            && !agents.contains(agent)
        {
            problems.push(format!("{label}: unknown agent \"{agent}\", use {}", agents.join(", ")));
        }
    }
    problems
}

/// The content of the last complete fenced `toml` block in `text`.
#[must_use]
pub fn last_toml_block(text: &str) -> Option<String> {
    let mut found = None;
    let mut open: Option<Vec<&str>> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        match &mut open {
            None => {
                if trimmed
                    .strip_prefix("```")
                    .is_some_and(|tag| tag.trim().eq_ignore_ascii_case("toml"))
                {
                    open = Some(Vec::new());
                }
            }
            Some(lines) => {
                if trimmed == "```" {
                    found = Some(lines.join("\n"));
                    open = None;
                } else {
                    lines.push(line);
                }
            }
        }
    }
    found
}

/// The workflow in `text`, written as `workflows.toml` (the first workflow in it) or as one bare
/// workflow with `name`, `description` and `[[stage]]` tables.
pub fn parse_toml(text: &str) -> Result<Workflow, Vec<String>> {
    let not_valid =
        |e: &toml::de::Error| vec![format!("the toml block is not valid: {}", e.message())];
    let table = Workflows::parse(text).map_err(|e| not_valid(&e))?;
    if let Some(workflow) = table.workflow.into_values().next() {
        return Ok(workflow);
    }
    let workflow: Workflow = toml::from_str(text).map_err(|e| not_valid(&e))?;
    if workflow.stages.is_empty() && workflow.name.is_empty() {
        return Err(vec!["the toml block holds no workflow".to_owned()]);
    }
    Ok(workflow)
}

/// `name`, or `name (2)`, `name (3)` and so on when `taken` already has it (ignoring case).
#[must_use]
pub fn unique_name<'a>(name: &str, taken: impl IntoIterator<Item = &'a str>) -> String {
    let taken: Vec<String> = taken.into_iter().map(str::to_lowercase).collect();
    let free = |candidate: &str| !taken.contains(&candidate.to_lowercase());
    if free(name) {
        return name.to_owned();
    }
    (2..=taken.len() + 2)
        .map(|n| format!("{name} ({n})"))
        .find(|candidate| free(candidate))
        .unwrap_or_else(|| name.to_owned())
}

/// A table key for the file made from a workflow's name: lower case words joined by hyphens.
#[must_use]
pub fn id_stem(name: &str) -> String {
    let words: Vec<String> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    if words.is_empty() { "workflow".to_owned() } else { words.join("-") }
}

#[cfg(test)]
mod tests;
