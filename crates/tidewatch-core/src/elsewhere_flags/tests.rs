use super::*;
use crate::agents::AgentTable;

const ID: &str = "7d198e16-6157-439f-a389-34a4951f5dd8";

fn parse(cli: Cli, line: &str) -> Parsed {
    let table = AgentTable::builtin();
    let argv: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
    parse_command_line(cli, table.get(cli.id()).unwrap(), &argv)
}

fn shape(cli: Cli, line: &str) -> (Option<Level>, Option<String>, Option<String>, Vec<String>) {
    let Flags { level, model, effort, extra } = parse(cli, line).flags;
    (level, model, effort, extra)
}

type Case = (
    &'static str,
    Option<Level>,
    Option<&'static str>,
    Option<&'static str>,
    &'static [&'static str],
);

fn strings(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

#[test]
fn every_level_of_the_table_reads_back_as_itself() {
    let table = AgentTable::builtin();
    for cli in Cli::ALL {
        let agent = table.get(cli.id()).unwrap();
        for level in Level::ALL {
            let flags = agent.level_args(level).unwrap();
            if flags.is_empty() {
                continue;
            }
            let mut argv = vec![agent.bin.clone()];
            argv.extend(flags.iter().cloned());
            let got = parse_command_line(cli, agent, &argv).flags;
            assert_eq!(got.level, Some(level), "{cli:?} {level:?}: {flags:?}");
            assert_eq!(got.extra, Vec::<String>::new(), "{cli:?} {level:?}");
        }
    }
}

#[test]
fn claude_command_lines_read_back_as_flags() {
    let cases: &[Case] = &[
        ("claude", None, None, None, &[]),
        (
            "claude --permission-mode acceptEdits --model opus --effort high",
            Some(Level::Edits),
            Some("opus"),
            Some("high"),
            &[],
        ),
        ("claude --dangerously-skip-permissions", Some(Level::Yolo), None, None, &[]),
        (
            "claude --permission-mode bypassPermissions --resume 7d198e16-6157-439f-a389-34a4951f5dd8",
            Some(Level::Yolo),
            None,
            None,
            &[],
        ),
        (
            "claude --resume 7d198e16 --permission-mode acceptEdits --add-dir /a /b --verbose",
            Some(Level::Edits),
            None,
            None,
            &["--add-dir", "/a", "/b", "--verbose"],
        ),
        ("claude --verbose fix the bug", None, None, None, &["--verbose"]),
        (
            "claude --permission-mode dontAsk --allowedTools=Read,Grep,Glob",
            Some(Level::Plan),
            None,
            None,
            &[],
        ),
        ("claude --permission-mode manual", Some(Level::Ask), None, None, &[]),
        ("claude --permission-mode something-new", None, None, None, &[]),
        (
            "claude --mcp-config a.json --fork-session --resume 7d198e16",
            None,
            None,
            None,
            &["--mcp-config", "a.json"],
        ),
        ("claude -c --model sonnet", None, Some("sonnet"), None, &[]),
        (
            "claude --name my-session --settings {\"a\":1}",
            None,
            None,
            None,
            &["--name", "my-session", "--settings", "{\"a\":1}"],
        ),
        ("claude --weird value --other", None, None, None, &["--weird", "value", "--other"]),
        ("claude --model=opus -- prompt words", None, Some("opus"), None, &[]),
    ];
    for &(line, level, model, effort, extra) in cases {
        let want = (level, model.map(str::to_owned), effort.map(str::to_owned), strings(extra));
        assert_eq!(shape(Cli::Claude, line), want, "{line}");
    }
}

#[test]
fn codex_command_lines_read_back_as_flags() {
    let cases: &[Case] = &[
        (
            "codex --sandbox workspace-write --ask-for-approval on-request -m gpt-5 -c model_reasoning_effort=high -c foo=bar",
            Some(Level::Edits),
            Some("gpt-5"),
            Some("high"),
            &["-c", "foo=bar"],
        ),
        ("codex --full-auto", Some(Level::Edits), None, None, &[]),
        ("codex --yolo", Some(Level::Yolo), None, None, &[]),
        ("codex --dangerously-bypass-approvals-and-sandbox", Some(Level::Yolo), None, None, &[]),
        ("codex --approve-for-me", Some(Level::Auto), None, None, &[]),
        (
            "codex --ask-for-approval on-request --sandbox read-only",
            Some(Level::Ask),
            None,
            None,
            &[],
        ),
        ("node /opt/homebrew/bin/codex --search", None, None, None, &["--search"]),
        ("codex -C /work --model=gpt-5", None, Some("gpt-5"), None, &["-C", "/work"]),
    ];
    for &(line, level, model, effort, extra) in cases {
        let want = (level, model.map(str::to_owned), effort.map(str::to_owned), strings(extra));
        assert_eq!(shape(Cli::Codex, line), want, "{line}");
    }
}

#[test]
fn gemini_command_lines_read_back_as_flags() {
    let cases: &[Case] = &[
        ("gemini --approval-mode yolo --sandbox", Some(Level::Auto), None, None, &[]),
        ("gemini --approval-mode yolo", Some(Level::Yolo), None, None, &[]),
        ("gemini --yolo", Some(Level::Yolo), None, None, &[]),
        ("gemini -y", Some(Level::Yolo), None, None, &[]),
        (
            "gemini --approval-mode plan --model gemini-3-pro",
            Some(Level::Plan),
            Some("gemini-3-pro"),
            None,
            &[],
        ),
        (
            "node /opt/homebrew/bin/gemini --approval-mode auto_edit",
            Some(Level::Edits),
            None,
            None,
            &[],
        ),
        (
            "gemini --include-directories /a /b --debug",
            None,
            None,
            None,
            &["--include-directories", "/a", "/b", "--debug"],
        ),
    ];
    for &(line, level, model, effort, extra) in cases {
        let got = shape(Cli::Gemini, line);
        let want = (level, model.map(str::to_owned), effort.map(str::to_owned), strings(extra));
        assert_eq!(got, want, "{line}");
    }
}

#[test]
fn copilot_command_lines_read_back_as_flags() {
    let cases: &[Case] = &[
        (
            "copilot --allow-all-tools --model gpt-5 --reasoning-effort high",
            Some(Level::Auto),
            Some("gpt-5"),
            Some("high"),
            &[],
        ),
        ("copilot --resume=3f2a9c1e-1111 --yolo", Some(Level::Yolo), None, None, &[]),
        ("copilot --allow-tool=write", Some(Level::Edits), None, None, &[]),
        ("copilot --mode plan", Some(Level::Plan), None, None, &[]),
        (
            "copilot --banner --log-level debug",
            None,
            None,
            None,
            &["--banner", "--log-level", "debug"],
        ),
    ];
    for &(line, level, model, effort, extra) in cases {
        let want = (level, model.map(str::to_owned), effort.map(str::to_owned), strings(extra));
        assert_eq!(shape(Cli::Copilot, line), want, "{line}");
    }
}

#[test]
fn the_session_id_comes_from_the_resume_flags_or_codexs_resume_word() {
    let cases = [
        (Cli::Claude, format!("claude --resume {ID}"), Some(ID)),
        (Cli::Claude, format!("claude --resume={ID} --verbose"), Some(ID)),
        (Cli::Claude, format!("claude --session-id {ID}"), Some(ID)),
        (Cli::Claude, "claude".to_owned(), None),
        (Cli::Copilot, format!("copilot --resume={ID}"), Some(ID)),
        (Cli::Codex, format!("codex resume {ID} --last"), Some(ID)),
        (Cli::Codex, format!("codex fork {ID}"), Some(ID)),
        (Cli::Codex, "codex resume".to_owned(), None),
        (Cli::Gemini, "gemini --resume latest".to_owned(), Some("latest")),
    ];
    for (cli, line, want) in cases {
        assert_eq!(parse(cli, &line).resume_id.as_deref(), want, "{line}");
    }
}

#[test]
fn a_one_shot_run_is_not_a_conversation() {
    let cases = [
        (Cli::Claude, "claude -p hello", true),
        (Cli::Claude, "claude --print --output-format json", true),
        (Cli::Claude, "claude mcp list", true),
        (Cli::Claude, "claude --verbose", false),
        (Cli::Codex, "codex exec fix", true),
        (Cli::Codex, "codex app-server", true),
        (Cli::Codex, "codex login", true),
        (Cli::Codex, "codex --sandbox read-only", false),
        (Cli::Codex, "codex resume", false),
        (Cli::Gemini, "gemini -p hi", true),
        (Cli::Gemini, "gemini --prompt hi", true),
        (Cli::Gemini, "gemini mcp list", true),
        (Cli::Gemini, "gemini --yolo", false),
        (Cli::Copilot, "copilot --prompt hi", true),
        (Cli::Copilot, "copilot --acp", true),
        (Cli::Copilot, "copilot --allow-all-tools", false),
    ];
    for (cli, line, want) in cases {
        assert_eq!(parse(cli, line).non_interactive, want, "{line}");
    }
}

#[test]
fn the_first_word_that_is_not_a_flag_is_the_subcommand() {
    assert_eq!(
        parse(Cli::Codex, "codex --model gpt-5 resume").subcommand.as_deref(),
        Some("resume")
    );
    assert_eq!(parse(Cli::Codex, "codex --sandbox read-only").subcommand, None);
}

#[test]
fn a_flag_with_a_value_after_a_double_dash_is_never_read() {
    let got = shape(Cli::Claude, "claude --verbose -- --dangerously-skip-permissions");
    assert_eq!(got, (None, None, None, strings(&["--verbose"])));
}

#[test]
fn the_program_is_found_by_name_in_the_first_three_words() {
    let table = AgentTable::builtin();
    let agent = table.get("gemini").unwrap();
    let argv = strings(&["/usr/bin/env", "node", "/opt/bin/gemini", "--yolo"]);
    assert_eq!(parse_command_line(Cli::Gemini, agent, &argv).flags.level, Some(Level::Yolo));
}
