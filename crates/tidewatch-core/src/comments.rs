//! The comment classifier of the comment guard: which comments in freshly written code fail the
//! keep-list. Default is no comment; a comment survives only as a license header, a note on
//! behavior forced by a dependency or platform, a lint suppression with a reason, a doc comment on a
//! public contract, an issue or RFC link, or a toolchain directive.
//!
//! Pure: text in, flags out. `comment_sweep` finds the added lines in a branch diff and
//! `guard_comments` answers the agent's hooks.

use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syntax {
    /// `//`, `///`, `//!` and `/* */`: Rust, TypeScript, JavaScript, Go, Swift, Kotlin, Java, C and kin.
    CLike,
    /// `#`: Python, Ruby, shell, Elixir.
    Hash,
    /// `--` and `/* */`.
    Sql,
}

const C_LIKE: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "svelte", "vue", "astro", "go", "rs", "swift", "kt",
    "kts", "java", "c", "h", "cc", "cpp", "hpp", "m", "mm", "cs", "php", "scala", "dart", "css",
    "scss", "less",
];
const HASH: &[&str] = &["rb", "rake", "py", "sh", "bash", "zsh", "ex", "exs"];
const SKIPPED: &[&str] = &[
    "/node_modules/",
    "/vendor/",
    "/dist/",
    "/build/",
    "/target/",
    ".min.",
    "__generated__",
    ".snap",
    "/migrate/",
    "db/schema.rb",
    ".d.ts",
    "test/fixtures/",
    "tests/fixtures/",
    "/.claude/",
];

impl Syntax {
    /// The comment syntax of a source file, `None` for anything the guard leaves alone (not code,
    /// generated, vendored or a fixture).
    #[must_use]
    pub fn of_path(path: &str) -> Option<Self> {
        let rooted = format!("/{}", path.trim_start_matches("./"));
        if SKIPPED.iter().any(|part| rooted.contains(part)) {
            return None;
        }
        let ext = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
        if C_LIKE.contains(&ext.as_str()) {
            Some(Self::CLike)
        } else if HASH.contains(&ext.as_str()) {
            Some(Self::Hash)
        } else if ext == "sql" {
            Some(Self::Sql)
        } else {
            None
        }
    }

    fn line_marker(self) -> &'static str {
        match self {
            Self::CLike => "//",
            Self::Hash => "#",
            Self::Sql => "--",
        }
    }

    fn has_blocks(self) -> bool {
        self != Self::Hash
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Suppression,
    Banner,
    DeadCode,
    Alibi,
    BareTodo,
    Narration,
    Restates,
    DocBloat,
    Density,
}

impl Kind {
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::Suppression => "suppression",
            Self::Banner => "banner",
            Self::DeadCode => "dead-code",
            Self::Alibi => "alibi",
            Self::BareTodo => "bare-todo",
            Self::Narration => "narration",
            Self::Restates => "restates-signature",
            Self::DocBloat => "doc-bloat",
            Self::Density => "density",
        }
    }

    #[must_use]
    pub fn why(self) -> &'static str {
        match self {
            Self::Suppression => "lint suppression with no reason",
            Self::Banner => "decorative banner or section divider",
            Self::DeadCode => "commented-out code",
            Self::Alibi => "alibi comment: reshape the code instead",
            Self::BareTodo => "TODO with no issue link",
            Self::Narration => "narrates what the next line already says",
            Self::Restates => "doc comment restates the signature",
            Self::DocBloat => "long comment block: one line of why at most",
            Self::Density => "too many comments for the code written",
        }
    }
}

/// A comment that failed the keep-list. `line` counts from the first line scanned, 0 for the whole text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flag {
    pub line: usize,
    pub kind: Kind,
    pub text: String,
}

pub const ADVICE: &str = "Delete them unless the keep-list applies: a license header, behavior forced by a dependency or platform, a lint suppression with its reason, a doc comment on a public contract, an issue or RFC link, a toolchain directive. When a comment explains your own surprise, reshape the code (rename, extract, type) instead. Do not shorten a flagged comment into a smaller alibi. If a flag is wrong, say which line and why in one sentence and move on.";

/// `L12  [narration] narrates what the next line already says: loop over items`, one per flag.
#[must_use]
pub fn render(flags: &[Flag]) -> Vec<String> {
    flags
        .iter()
        .map(|flag| {
            let place = if flag.line == 0 { "file".to_owned() } else { format!("L{}", flag.line) };
            let text =
                if flag.text.is_empty() { String::new() } else { format!(": {}", flag.text) };
            format!("{place}  [{}] {}{text}", flag.kind.tag(), flag.kind.why())
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Shape {
    Blank,
    Code,
    Comment { body: String, doc: bool },
}

struct Parsed {
    shape: Shape,
    trailing: Option<String>,
    raw: String,
}

fn strip_run(text: &str, marks: &[char]) -> String {
    text.trim_start_matches(|c: char| marks.contains(&c) || c.is_whitespace()).trim().to_owned()
}

fn parse_lines(text: &str, syntax: Syntax) -> Vec<Parsed> {
    let mut block: Option<bool> = None;
    text.lines()
        .enumerate()
        .map(|(at, line)| {
            let raw = line.to_owned();
            let trimmed = line.trim();
            let comment = |body: String, doc: bool| Parsed {
                shape: Shape::Comment { body, doc },
                trailing: None,
                raw: raw.clone(),
            };
            if let Some(doc) = block {
                if let Some((before, _)) = trimmed.split_once("*/") {
                    block = None;
                    return comment(strip_run(before, &['*']), doc);
                }
                return comment(strip_run(trimmed, &['*']), doc);
            }
            if trimmed.is_empty() {
                return Parsed { shape: Shape::Blank, trailing: None, raw };
            }
            if syntax.has_blocks()
                && let Some(rest) = trimmed.strip_prefix("/*")
            {
                let doc =
                    (rest.starts_with('*') && !rest.starts_with("*/")) || rest.starts_with('!');
                if let Some((inside, _)) = rest.split_once("*/") {
                    return comment(strip_run(inside, &['*', '!']), doc);
                }
                block = Some(doc);
                return comment(strip_run(rest, &['*', '!']), doc);
            }
            let marker = syntax.line_marker();
            if let Some(rest) = trimmed.strip_prefix(marker) {
                let first = syntax == Syntax::Hash && at == 0 && rest.starts_with('!');
                if first {
                    return Parsed { shape: Shape::Blank, trailing: None, raw };
                }
                let doc = syntax == Syntax::CLike
                    && (rest.starts_with('/') && !rest.starts_with("//") || rest.starts_with('!'));
                return comment(strip_run(rest, &['/', '!', '#', '-']), doc);
            }
            Parsed { shape: Shape::Code, trailing: trailing_body(line, marker), raw }
        })
        .collect()
}

fn trailing_body(line: &str, marker: &str) -> Option<String> {
    let spaced = format!(" {marker}");
    let at = line.find(&spaced)?;
    let before = &line[..at];
    let in_string = ['"', '\'', '`'].iter().any(|quote| before.matches(*quote).count() % 2 == 1);
    if before.trim().is_empty() || in_string || before.ends_with(':') {
        return None;
    }
    Some(line[at + spaced.len()..].trim().to_owned())
}

fn has_issue_ref(body: &str) -> bool {
    body.match_indices('#')
        .any(|(at, _)| body[at + 1..].chars().take_while(char::is_ascii_digit).count() >= 2)
}

fn is_kept(body: &str, lowered: &str) -> bool {
    const KEEP: &[&str] = &[
        "http://",
        "https://",
        "copyright",
        "spdx-license",
        "licensed under",
        "all rights reserved",
        "@license",
        "prettier-ignore",
        "frozen_string_literal",
        "@ts-check",
        "schema information",
        "eslint-enable",
        "eslint-env",
        "#pragma",
        "@vite-ignore",
        "webpackchunkname",
        "coding:",
        "coding=",
        "encoding:",
        "typed:",
        "safety:",
        "noinspection",
        "rfc ",
        "rfc-",
        "-*- ",
    ];
    KEEP.iter().any(|keep| lowered.contains(keep))
        || has_issue_ref(body)
        || lowered.split(|c: char| !c.is_alphanumeric()).any(|word| {
            word.strip_prefix("rfc")
                .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        })
}

const SUPPRESSIONS: &[&str] = &[
    "eslint-disable",
    "@ts-ignore",
    "@ts-nocheck",
    "@ts-expect-error",
    "rubocop:disable",
    "swiftlint:disable",
    "deno-lint-ignore",
    "biome-ignore",
    "noqa",
    "type: ignore",
    "istanbul ignore",
    "c8 ignore",
    "@suppresswarnings",
    "nolint",
];

fn suppression_has_reason(lowered: &str) -> bool {
    let Some(at) = SUPPRESSIONS.iter().filter_map(|d| lowered.find(d).map(|i| i + d.len())).min()
    else {
        return false;
    };
    let rest = &lowered[at..];
    let after_separator = [" -- ", " - ", ": ", " because "]
        .iter()
        .filter_map(|sep| rest.find(sep).map(|i| &rest[i + sep.len()..]))
        .any(|reason| reason.split_whitespace().count() >= 2);
    after_separator || (lowered.contains("@ts-") && rest.split_whitespace().count() >= 3)
}

fn is_banner(raw: &str) -> bool {
    let ruled = |text: &str| {
        let text = text.trim();
        text.chars().count() >= 6 && text.chars().all(|c| "-=*_#~/+ ".contains(c))
    };
    let inner = raw.trim().trim_start_matches(|c| "/#!*".contains(c)).trim();
    let edge = inner.chars().filter(|c| "-=*_~".contains(*c)).count();
    let fenced = ["---", "===", "***", "###", "___"]
        .iter()
        .any(|f| inner.starts_with(f) && inner.ends_with(f));
    ruled(raw) || (fenced && edge >= 6)
}

const STARTS: &[&str] = &[
    "import ",
    "export ",
    "def ",
    "class ",
    "fn ",
    "pub fn ",
    "let ",
    "const ",
    "var ",
    "function ",
    "if (",
    "if(",
    "for (",
    "for(",
    "while (",
    "return ",
    "use ",
    "async ",
];

fn is_codeish(body: &str) -> bool {
    let lowered = body.to_lowercase();
    let call_shaped = body.find('(').is_some_and(|at| {
        at > 0 && body[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_')
    });
    let ends_like_code = body.ends_with(';')
        || body.ends_with('{')
        || body.ends_with('}')
        || (call_shaped && (body.ends_with(')') || body.ends_with(");")));
    let tag = body.starts_with('<') && body.ends_with('>') && body.len() > 2;
    body.contains("=>")
        || lowered.contains("console.log(")
        || lowered.contains("println!(")
        || (STARTS.iter().any(|start| lowered.starts_with(start)) && ends_like_code)
        || (ends_like_code && !body.contains(' ') && body.len() > 2)
        || (ends_like_code && (body.contains(" = ") || call_shaped || body.starts_with('}')))
        || tag
        || (!body.contains(' ')
            && body.contains('.')
            && (body.ends_with('!') || body.ends_with('?')))
        || body.starts_with(['}', ']', ')'])
}

fn alibi(lowered: &str) -> bool {
    const PHRASES: &[&str] = &[
        "do not remove",
        "do not touch",
        "do not change",
        "do not delete",
        "don't remove",
        "don't touch",
        "dont remove",
        "too risky",
        "fine for now",
        "workaround",
        "work-around",
        "work around",
        "temporar",
        "temp fix",
        "just in case",
        "safety net",
        "be careful",
        "keep this",
        "leave this as",
        "leave it as",
        "leave it alone",
        "leave this alone",
        "otherwise it breaks",
        "fixme",
    ];
    const WORDS: &[&str] = &["important", "hack", "xxx", "tbd"];
    PHRASES.iter().any(|phrase| lowered.contains(phrase))
        || lowered.split(|c: char| !c.is_alphanumeric()).any(|word| WORDS.contains(&word))
        || lowered.ends_with("for now")
        || lowered.contains("for now ")
}

fn bare_todo(body: &str, lowered: &str) -> bool {
    let first = lowered.split(|c: char| !c.is_alphanumeric()).next().unwrap_or_default();
    (first == "todo" || first == "tofix") && !lowered.contains("http") && !has_issue_ref(body)
}

fn narration(body: &str, lowered: &str) -> bool {
    const VERBS: &[&str] = &[
        "get",
        "gets",
        "set",
        "sets",
        "create",
        "creates",
        "return",
        "returns",
        "loop",
        "loops",
        "initialize",
        "initialise",
        "init",
        "handle",
        "handles",
        "update",
        "updates",
        "add",
        "adds",
        "remove",
        "removes",
        "delete",
        "deletes",
        "call",
        "calls",
        "build",
        "builds",
        "fetch",
        "fetches",
        "parse",
        "parses",
        "convert",
        "converts",
        "extract",
        "extracts",
        "define",
        "defines",
        "declare",
        "iterate",
        "iterates",
        "map",
        "maps",
        "filter",
        "filters",
        "sort",
        "sorts",
        "wrap",
        "wraps",
        "ensure",
        "ensures",
        "increment",
        "reset",
        "clear",
        "start",
        "stop",
        "setup",
        "configure",
        "apply",
        "applies",
        "assign",
        "copy",
        "move",
        "send",
        "emit",
        "log",
        "print",
        "check",
        "checks",
        "validate",
        "validates",
        "now",
        "then",
        "first",
        "second",
        "third",
        "finally",
        "next",
        "load",
        "render",
    ];
    const STEMS: &[&str] = &["comput", "calculat", "stor", "sav"];
    const LABELS: &[&str] = &[
        "helper",
        "helpers",
        "constant",
        "constants",
        "type",
        "types",
        "import",
        "imports",
        "main",
    ];
    let first = lowered.split(|c: char| !c.is_alphanumeric()).next().unwrap_or_default();
    let short = body.split_whitespace().count() <= 8;
    let label = body.split_whitespace().count() == 1 && LABELS.contains(&first);
    label || (short && (VERBS.contains(&first) || STEMS.iter().any(|stem| first.starts_with(stem))))
}

fn words_of(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut previous_lower = false;
    for c in text.chars() {
        if c.is_alphanumeric() {
            if c.is_uppercase() && previous_lower {
                words.push(std::mem::take(&mut current));
            }
            previous_lower = c.is_lowercase();
            current.extend(c.to_lowercase());
        } else {
            previous_lower = false;
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn stem(word: &str) -> &str {
    for suffix in ["ing", "es", "ed", "s"] {
        if let Some(root) = word.strip_suffix(suffix).filter(|root| root.len() >= 3) {
            return root;
        }
    }
    word
}

fn restates(doc: &str, item: &str) -> bool {
    const FILLER: &[&str] = &[
        "the", "a", "an", "of", "to", "for", "is", "this", "that", "and", "or", "in", "on",
        "returns", "return", "gets", "get", "sets", "set", "creates", "create", "new", "function",
        "method", "value", "given", "with", "from", "by", "as", "be", "it", "its",
    ];
    let signature: Vec<String> = words_of(item).iter().map(|w| stem(w).to_owned()).collect();
    let significant: Vec<String> = words_of(doc)
        .into_iter()
        .filter(|word| !FILLER.contains(&word.as_str()))
        .map(|word| stem(&word).to_owned())
        .collect();
    !significant.is_empty() && significant.iter().all(|word| signature.contains(word))
}

fn is_item_line(line: &str) -> bool {
    let lowered = line.trim().to_lowercase();
    [
        "fn ",
        "pub ",
        "function ",
        "export ",
        "def ",
        "func ",
        "class ",
        "struct ",
        "enum ",
        "const ",
        "type ",
        "interface ",
        "trait ",
        "impl ",
        "async ",
        "static ",
        "public ",
        "private ",
        "protected ",
    ]
    .iter()
    .any(|start| lowered.starts_with(start))
}

fn clip(text: &str) -> String {
    const MAX: usize = 90;
    let text = text.trim();
    if text.chars().count() <= MAX {
        return text.to_owned();
    }
    let cut: String = text.chars().take(MAX).collect();
    format!("{cut}...")
}

fn judge(body: &str, raw: &str, doc: bool) -> Option<Kind> {
    let lowered = body.to_lowercase();
    if is_kept(body, &raw.to_lowercase()) {
        return None;
    }
    let raw_lowered = raw.to_lowercase();
    if SUPPRESSIONS.iter().any(|d| raw_lowered.contains(d)) {
        return (!suppression_has_reason(&raw_lowered)).then_some(Kind::Suppression);
    }
    if is_banner(raw) {
        return Some(Kind::Banner);
    }
    if body.is_empty() {
        return None;
    }
    if !doc && is_codeish(body) {
        return Some(Kind::DeadCode);
    }
    if alibi(&lowered) {
        return Some(Kind::Alibi);
    }
    if bare_todo(body, &lowered) {
        return Some(Kind::BareTodo);
    }
    (!doc && narration(body, &lowered)).then_some(Kind::Narration)
}

/// The comments in `text` that fail the keep-list. Line numbers count from `first_line` (1 for a
/// whole file).
#[must_use]
pub fn scan(text: &str, syntax: Syntax, first_line: usize) -> Vec<Flag> {
    let parsed = parse_lines(text, syntax);
    let place = |index: usize| first_line + index;
    let mut flags = Vec::new();
    let (mut run, mut run_start) = (0usize, 0usize);
    let mut own = 0usize;
    for (index, line) in parsed.iter().enumerate() {
        let normal_comment = matches!(&line.shape, Shape::Comment { doc: false, .. });
        if normal_comment {
            own += 1;
            if run == 0 {
                run_start = index;
            }
            run += 1;
        } else {
            if run >= 5 {
                flags.push(Flag {
                    line: place(run_start),
                    kind: Kind::DocBloat,
                    text: String::new(),
                });
            }
            run = 0;
        }
        let judged = match &line.shape {
            Shape::Comment { body, doc } => {
                judge(body, &line.raw, *doc).map(|kind| (kind, body.clone()))
            }
            Shape::Code => line
                .trailing
                .as_deref()
                .and_then(|body| judge(body, &line.raw, false).map(|kind| (kind, body.to_owned()))),
            Shape::Blank => None,
        };
        if let Some((kind, body)) = judged {
            flags.push(Flag { line: place(index), kind, text: clip(&body) });
        }
    }
    if run >= 5 {
        flags.push(Flag { line: place(run_start), kind: Kind::DocBloat, text: String::new() });
    }
    flags.extend(restating_docs(&parsed, first_line));
    let code = parsed.iter().filter(|l| l.shape != Shape::Blank).count();
    if own > 4 && code > 0 && own * 5 > code {
        flags.push(Flag {
            line: 0,
            kind: Kind::Density,
            text: format!("{own} comment lines in {code} lines"),
        });
    }
    flags.sort_by_key(|flag| flag.line);
    flags.dedup_by(|a, b| a.line == b.line && a.kind == b.kind);
    flags
}

fn restating_docs(parsed: &[Parsed], first_line: usize) -> Vec<Flag> {
    let mut flags = Vec::new();
    let mut index = 0;
    while index < parsed.len() {
        let start = index;
        while index < parsed.len()
            && matches!(parsed[index].shape, Shape::Comment { doc: true, .. })
        {
            index += 1;
        }
        if index == start {
            index += 1;
            continue;
        }
        let item = parsed[index..].iter().find(|line| {
            let t = line.raw.trim();
            !(t.starts_with("#[") || t.starts_with('@'))
        });
        let docs: Vec<&str> = parsed[start..index]
            .iter()
            .filter_map(|line| match &line.shape {
                Shape::Comment { body, .. } if !body.is_empty() => Some(body.as_str()),
                _ => None,
            })
            .collect();
        if (1..=2).contains(&docs.len())
            && let Some(item) =
                item.filter(|line| line.shape == Shape::Code && is_item_line(&line.raw))
        {
            let text = docs.join(" ");
            if !is_kept(&text, &text.to_lowercase()) && restates(&text, &item.raw) {
                flags.push(Flag {
                    line: first_line + start,
                    kind: Kind::Restates,
                    text: clip(&text),
                });
            }
        }
    }
    flags
}

#[cfg(test)]
mod tests;
