//! A running agent's command line read back into a level, model, effort and carried flags (tables
//! from the `--help` of claude 2.1.301, codex 0.162.1, gemini 0.46.0, copilot 1.0.96). `ps` joins
//! words with a space, so a value holding a space comes back as two words.

use crate::agents::{Agent, Level};
use crate::elsewhere::Cli;

/// What the command line says about how the session runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    /// `None` when no permission flag was found: the CLI's own default.
    pub level: Option<Level>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Everything else, in order, ready to append to a launch.
    pub extra: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub flags: Flags,
    /// The first word that is not a flag or a flag's value.
    pub subcommand: Option<String>,
    /// A one-shot run (`-p`, a subcommand such as `exec` or `mcp`), not a conversation.
    pub non_interactive: bool,
    /// The session id the command line names.
    pub resume_id: Option<String>,
}

struct Names {
    resume: &'static [&'static str],
    cont: &'static [&'static str],
    session_id: &'static [&'static str],
    level_value: &'static [&'static str],
    level_switch: &'static [&'static str],
    one: &'static [&'static str],
    many: &'static [&'static str],
    plain: &'static [&'static str],
    headless: &'static [&'static str],
    subcommands: &'static [&'static str],
    model: &'static [&'static str],
}

const CLAUDE: Names = Names {
    resume: &["--resume", "-r"],
    cont: &["--continue", "-c", "--fork-session"],
    session_id: &["--session-id"],
    level_value: &["--permission-mode"],
    level_switch: &["--dangerously-skip-permissions"],
    one: &[
        "--agent",
        "--agents",
        "--append-system-prompt",
        "--append-system-prompt-file",
        "--autocompact",
        "--debug-file",
        "--environment",
        "--fallback-model",
        "--input-format",
        "--json-schema",
        "--max-budget-usd",
        "--max-turns",
        "--name",
        "-n",
        "--output-format",
        "--permission-prompt-tool",
        "--plugin-dir",
        "--settings",
        "--setting-sources",
        "--system-prompt",
        "--system-prompt-file",
    ],
    many: &[
        "--add-dir",
        "--allowedTools",
        "--allowed-tools",
        "--disallowedTools",
        "--disallowed-tools",
        "--mcp-config",
        "--betas",
        "--file",
    ],
    plain: &[
        "--allow-dangerously-skip-permissions",
        "--ax-screen-reader",
        "--bare",
        "--brief",
        "--chrome",
        "--debug",
        "-d",
        "--desktop",
        "--disable-slash-commands",
        "--exclude-dynamic-system-prompt-sections",
        "--forward-subagent-text",
        "--ide",
        "--include-hook-events",
        "--include-partial-messages",
        "--mcp-debug",
        "--no-chrome",
        "--no-session-persistence",
        "--replay-user-messages",
        "--strict-mcp-config",
        "--verbose",
    ],
    headless: &["-p", "--print", "--bg", "--background", "--cloud"],
    subcommands: &[
        "mcp",
        "plugin",
        "plugins",
        "update",
        "upgrade",
        "doctor",
        "install",
        "setup-token",
        "agents",
        "auth",
        "config",
    ],
    model: &["--model"],
};

const CODEX: Names = Names {
    resume: &[],
    cont: &["--last", "--all"],
    session_id: &[],
    level_value: &["--ask-for-approval", "-a", "--sandbox", "-s"],
    level_switch: &[
        "--approve-for-me",
        "--full-auto",
        "--dangerously-bypass-approvals-and-sandbox",
        "--yolo",
    ],
    one: &[
        "--cd",
        "-C",
        "--profile",
        "-p",
        "--enable",
        "--disable",
        "--remote",
        "--remote-auth-token-env",
        "--local-provider",
        "--add-dir",
        "--image",
        "-i",
        "--config",
        "-c",
    ],
    many: &[],
    plain: &[
        "--dangerously-bypass-hook-trust",
        "--search",
        "--no-alt-screen",
        "--no-daemon",
        "--strict-config",
        "--worktree",
        "--oss",
    ],
    headless: &[],
    subcommands: &[
        "exec",
        "e",
        "review",
        "login",
        "logout",
        "mcp",
        "mcp-server",
        "app-server",
        "proto",
        "completion",
        "cloud",
        "apply",
        "sandbox",
        "debug",
        "features",
        "update",
    ],
    model: &["--model", "-m"],
};

const GEMINI: Names = Names {
    resume: &["--resume", "-r"],
    cont: &[],
    session_id: &["--session-id"],
    level_value: &["--approval-mode"],
    level_switch: &["--yolo", "-y"],
    one: &["--prompt-interactive", "-i", "--output-format", "-o", "--worktree", "-w"],
    many: &[
        "--policy",
        "--admin-policy",
        "--allowed-mcp-server-names",
        "--allowed-tools",
        "--extensions",
        "-e",
        "--include-directories",
    ],
    plain: &["--debug", "-d", "--sandbox", "-s", "--skip-trust", "--screen-reader", "--raw-output"],
    headless: &[
        "--prompt",
        "-p",
        "--acp",
        "--experimental-acp",
        "--list-extensions",
        "-l",
        "--list-sessions",
        "--delete-session",
    ],
    subcommands: &["mcp", "extensions", "extension", "skills", "hooks", "config"],
    model: &["--model", "-m"],
};

const COPILOT: Names = Names {
    resume: &["--resume", "-r"],
    cont: &["--continue"],
    session_id: &["--session-id"],
    level_value: &["--mode"],
    level_switch: &["--allow-all-tools", "--allow-all", "--yolo", "--plan"],
    one: &[
        "--agent",
        "--context",
        "--auto-tier",
        "--name",
        "-n",
        "--log-dir",
        "--log-level",
        "--stream",
        "--output-format",
        "--add-dir",
        "--attachment",
        "--max-autopilot-continues",
        "--plugin-dir",
        "--additional-mcp-config",
        "--interactive",
        "-i",
    ],
    many: &[
        "--allow-tool",
        "--deny-tool",
        "--available-tools",
        "--excluded-tools",
        "--secret-env-vars",
        "--allow-url",
        "--deny-url",
    ],
    plain: &[
        "--allow-all-paths",
        "--allow-all-urls",
        "--autopilot",
        "--experimental",
        "--banner",
        "--screen-reader",
        "--plain-diff",
        "--no-color",
        "--no-auto-update",
        "--no-ask-user",
        "--no-experimental",
        "--no-mouse",
        "--no-remote",
    ],
    headless: &["--prompt", "-p", "--acp"],
    subcommands: &["login", "logout", "update", "plugin", "help"],
    model: &["--model"],
};

fn names(cli: Cli) -> &'static Names {
    match cli {
        Cli::Claude => &CLAUDE,
        Cli::Codex => &CODEX,
        Cli::Gemini => &GEMINI,
        Cli::Copilot => &COPILOT,
    }
}

/// Hand-written spellings of a level, on top of the table in `agents.rs`. The longest pattern that
/// matches wins, so `--approval-mode yolo --sandbox` is still Auto for Gemini.
fn aliases(cli: Cli) -> &'static [(&'static str, Level)] {
    match cli {
        Cli::Claude => &[
            ("--permission-mode bypassPermissions", Level::Yolo),
            ("--permission-mode plan", Level::Plan),
            ("--permission-mode dontAsk", Level::Plan),
            ("--permission-mode default", Level::Ask),
        ],
        Cli::Codex => &[("--full-auto", Level::Edits), ("--yolo", Level::Yolo)],
        Cli::Gemini => &[("--approval-mode yolo", Level::Yolo), ("-y", Level::Yolo)],
        Cli::Copilot => &[("--allow-all", Level::Yolo), ("--plan", Level::Plan)],
    }
}

/// The words of `argv` after the program (`claude`, or `node /path/to/gemini`).
fn after_program(agent: &Agent, argv: &[String]) -> Vec<String> {
    let at = argv.iter().take(3).position(|word| basename(word) == agent.bin).map_or(1, |i| i + 1);
    argv.iter().skip(at).cloned().collect()
}

pub(crate) fn basename(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

type Item = Vec<String>;
type Match = (Level, usize, Vec<(usize, usize)>);

/// A level's flags as items: a flag with the words that follow it up to the next flag.
fn items(tokens: &[String]) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    for token in tokens {
        match items.last_mut() {
            Some(item) if !token.starts_with('-') => item.push(token.clone()),
            _ => items.push(vec![token.clone()]),
        }
    }
    items
}

/// How many words of `args` the item takes at `at`, when it is there.
fn item_len(args: &[String], at: usize, item: &Item) -> Option<usize> {
    let flag = item.first()?;
    if args[at] == *flag && item.len() == 1 {
        return Some(1);
    }
    if item.len() == 2 {
        if args[at] == format!("{flag}={}", item[1]) {
            return Some(1);
        }
        if args[at] == *flag && args.get(at + 1) == Some(&item[1]) {
            return Some(2);
        }
    }
    None
}

fn spans_of(args: &[String], used: &[bool], pattern: &[Item]) -> Option<Vec<(usize, usize)>> {
    let mut taken = used.to_vec();
    let mut spans = Vec::new();
    for item in pattern {
        let found = (0..args.len()).find_map(|at| {
            let free = !taken[at];
            free.then(|| item_len(args, at, item).map(|len| (at, len))).flatten()
        })?;
        for slot in &mut taken[found.0..found.0 + found.1] {
            *slot = true;
        }
        spans.push(found);
    }
    (!spans.is_empty()).then_some(spans)
}

fn match_level(cli: Cli, agent: &Agent, args: &[String], used: &mut [bool]) -> Option<Level> {
    let mut patterns: Vec<(Level, Vec<Item>)> = Vec::new();
    for level in Level::ALL {
        if let Some(flags) = agent.level_args(level).filter(|flags| !flags.is_empty()) {
            patterns.push((level, items(flags)));
        }
    }
    for (text, level) in aliases(cli) {
        let tokens: Vec<String> = text.split_whitespace().map(str::to_owned).collect();
        patterns.push((*level, items(&tokens)));
    }
    let mut best: Option<Match> = None;
    for (level, pattern) in &patterns {
        let Some(spans) = spans_of(args, used, pattern) else { continue };
        let size: usize = spans.iter().map(|(_, len)| len).sum();
        if best.as_ref().is_none_or(|(_, most, _)| size > *most) {
            best = Some((*level, size, spans));
        }
    }
    let (level, _, spans) = best?;
    for (at, len) in spans {
        for slot in &mut used[at..at + len] {
            *slot = true;
        }
    }
    Some(level)
}

fn value_after(args: &[String], used: &mut [bool], at: &mut usize) -> Option<String> {
    let next = args.get(*at).filter(|word| !word.starts_with('-') && !used[*at])?;
    used[*at] = true;
    *at += 1;
    Some(next.clone())
}

fn effort_value(agent: &Agent, value: &str) -> Option<String> {
    let prefix = agent.effort.as_ref()?.value_prefix.as_str();
    let rest = value.strip_prefix(prefix).filter(|_| !prefix.is_empty())?;
    Some(rest.trim_matches('"').to_owned())
}

/// The model, effort, level and carried flags of one running command line.
#[must_use]
pub fn parse_command_line(cli: Cli, agent: &Agent, argv: &[String]) -> Parsed {
    let names = names(cli);
    let args = after_program(agent, argv);
    let mut used = vec![false; args.len()];
    let mut parsed = Parsed::default();
    let before_terminator = args.iter().position(|word| word == "--").unwrap_or(args.len());
    parsed.flags.level =
        match_level(cli, agent, &args[..before_terminator], &mut used[..before_terminator]);
    let mut positionals = 0;
    let mut resume_word = false;
    let mut at = 0;
    while at < args.len() {
        let word = &args[at];
        at += 1;
        if used[at - 1] {
            continue;
        }
        if word == "--" {
            break;
        }
        if !word.starts_with('-') || word == "-" {
            if positionals == 0 {
                parsed.subcommand = Some(word.clone());
                resume_word = cli == Cli::Codex && matches!(word.as_str(), "resume" | "fork");
            } else if positionals == 1 && resume_word {
                parsed.resume_id = Some(word.clone());
            }
            positionals += 1;
            continue;
        }
        let (flag, inline) = match word.split_once('=') {
            Some((flag, value)) if word.starts_with("--") => (flag, Some(value.to_owned())),
            _ => (word.as_str(), None),
        };
        let has = |list: &[&str]| list.contains(&flag);
        let value = |at: &mut usize, used: &mut [bool]| {
            inline.clone().or_else(|| value_after(&args, used, at))
        };
        parsed.non_interactive |= has(names.headless);
        if has(names.model) {
            parsed.flags.model = value(&mut at, &mut used);
        } else if cli == Cli::Codex && (flag == "--config" || flag == "-c") {
            let given = value(&mut at, &mut used);
            if let Some(effort) = given.as_deref().and_then(|v| effort_value(agent, v)) {
                parsed.flags.effort = Some(effort);
            } else {
                parsed.flags.extra.push(flag.to_owned());
                parsed.flags.extra.extend(given);
            }
        } else if agent.effort.as_ref().is_some_and(|effort| flag == effort.flag) {
            parsed.flags.effort = value(&mut at, &mut used);
        } else if has(names.resume) || has(names.session_id) {
            let id = value(&mut at, &mut used);
            parsed.resume_id = parsed.resume_id.take().or(id);
        } else if has(names.level_value) {
            let _ = value(&mut at, &mut used);
        } else if !(has(names.cont) || has(names.level_switch)) {
            parsed.flags.extra.push(word.clone());
            let takes_words = has(names.one) || has(names.many) || !has(names.plain);
            if takes_words && inline.is_none() {
                parsed.flags.extra.extend(value_after(&args, &mut used, &mut at));
            }
            while has(names.many)
                && inline.is_none()
                && let Some(more) = value_after(&args, &mut used, &mut at)
            {
                parsed.flags.extra.push(more);
            }
        }
    }
    parsed.non_interactive |=
        parsed.subcommand.as_deref().is_some_and(|word| names.subcommands.contains(&word));
    parsed
}

#[cfg(test)]
mod tests;
