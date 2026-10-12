//! Stage prompt templates: `{name}` placeholders filled from the run.
//!
//! | Placeholder | Filled with |
//! | --- | --- |
//! | `{task}` | what the user asked for when opening the tab |
//! | `{base}` | the base branch the work is compared against |
//! | `{branch}` | the tab's branch |
//! | `{previous_summary}` (`{prev}`) | the summary of the last stage that reported one |
//! | `{findings}` | findings other stages reported, one `- [blocking] file:line text` per line |
//! | `{test_output}` | the tail of the last failing test run |
//!
//! Anything else in braces stays as written, so code samples in a prompt survive. Values are
//! inserted as is and never scanned again, so a task that mentions `{findings}` stays literal.

/// Everything a template can ask for. An empty value renders as `(none)`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Vars<'a> {
    pub task: &'a str,
    pub base: &'a str,
    pub branch: &'a str,
    pub previous_summary: &'a str,
    pub findings: &'a str,
    pub test_output: &'a str,
}

pub const PLACEHOLDERS: [&str; 7] =
    ["task", "base", "branch", "previous_summary", "prev", "findings", "test_output"];

const EMPTY_VALUE: &str = "(none)";

impl Vars<'_> {
    fn value(&self, name: &str) -> Option<&str> {
        let value = match name {
            "task" => self.task,
            "base" => self.base,
            "branch" => self.branch,
            "previous_summary" | "prev" => self.previous_summary,
            "findings" => self.findings,
            "test_output" => self.test_output,
            _ => return None,
        };
        Some(if value.trim().is_empty() { EMPTY_VALUE } else { value })
    }
}

fn next_placeholder(text: &str) -> Option<(usize, usize, &str)> {
    let mut from = 0;
    while let Some(offset) = text[from..].find('{') {
        let open = from + offset;
        let rest = &text[open + 1..];
        let len = rest.bytes().take_while(|b| b.is_ascii_lowercase() || *b == b'_').count();
        if len > 0 && rest.as_bytes().get(len) == Some(&b'}') {
            return Some((open, open + len + 2, &rest[..len]));
        }
        from = open + 1;
    }
    None
}

#[must_use]
pub fn render(template: &str, vars: &Vars) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some((start, end, name)) = next_placeholder(rest) {
        out.push_str(&rest[..start]);
        out.push_str(vars.value(name).unwrap_or(&rest[start..end]));
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Placeholder-shaped names in `template` that [`render`] would leave alone: almost always a typo.
#[must_use]
pub fn unknown_placeholders(template: &str) -> Vec<String> {
    let mut unknown = Vec::new();
    let mut rest = template;
    while let Some((_, end, name)) = next_placeholder(rest) {
        if !PLACEHOLDERS.contains(&name) && !unknown.iter().any(|seen| seen == name) {
            unknown.push(name.to_owned());
        }
        rest = &rest[end..];
    }
    unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> Vars<'static> {
        Vars {
            task: "add a retry",
            base: "main",
            branch: "agent/otter",
            previous_summary: "wrote it",
            findings: "- [blocking] a.rs:3 leak",
            test_output: "1 failed",
        }
    }

    #[test]
    fn every_placeholder_is_filled() {
        let cases = [
            ("{task}", "add a retry"),
            ("{base}..HEAD", "main..HEAD"),
            ("on {branch}", "on agent/otter"),
            ("{previous_summary}", "wrote it"),
            ("{prev}", "wrote it"),
            ("{findings}", "- [blocking] a.rs:3 leak"),
            ("{test_output}", "1 failed"),
            ("{task} then {task}", "add a retry then add a retry"),
        ];
        for (template, expected) in cases {
            assert_eq!(render(template, &vars()), expected, "{template}");
        }
    }

    #[test]
    fn empty_values_render_as_none() {
        let empty = Vars::default();
        assert_eq!(
            render("{task}|{findings}|{test_output}|{prev}", &empty),
            "(none)|(none)|(none)|(none)"
        );
        let blank = Vars { findings: "  \n", ..Vars::default() };
        assert_eq!(render("{findings}", &blank), "(none)");
    }

    #[test]
    fn unknown_and_malformed_braces_stay_literal() {
        let cases = [
            ("{nope}", "{nope}"),
            ("fn main() {}", "fn main() {}"),
            ("{ task }", "{ task }"),
            ("{Task}", "{Task}"),
            ("{{task}}", "{add a retry}"),
            ("open { and {task", "open { and {task"),
            ("{task", "{task"),
            ("", ""),
        ];
        for (template, expected) in cases {
            assert_eq!(render(template, &vars()), expected, "{template}");
        }
    }

    #[test]
    fn inserted_values_are_not_rescanned() {
        let sneaky = Vars { task: "see {findings}", findings: "F", ..Vars::default() };
        assert_eq!(render("{task} / {findings}", &sneaky), "see {findings} / F");
    }

    #[test]
    fn multibyte_text_around_placeholders_survives() {
        assert_eq!(render("héllo {base} ✓ {x}", &vars()), "héllo main ✓ {x}");
    }

    #[test]
    fn unknown_placeholders_lists_each_typo_once() {
        assert_eq!(
            unknown_placeholders("{task} {typo} {typo} {other_one} {}"),
            ["typo", "other_one"]
        );
        assert_eq!(
            unknown_placeholders(
                "{task}{base}{branch}{prev}{previous_summary}{findings}{test_output}"
            ),
            Vec::<String>::new()
        );
    }
}
