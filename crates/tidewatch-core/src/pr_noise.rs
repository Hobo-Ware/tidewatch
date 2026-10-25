//! Comments on a pull request that have nothing to act on, so they never become a card: a review bot's
//! receipt ("I looked, nothing to say") and the status posts of deploy, coverage and CI bots. A bot
//! comment that carries a suggestion is not noise. Pure.

const RECEIPT_MARKER: &str = "codex-review-receipt";
const CODEX_FOOTER: &str = "About Codex in GitHub";
const SUGGESTION_FENCE: &str = "```suggestion";

/// Bots whose comments only report status (a preview URL, a coverage number, a bot's greeting).
const STATUS_BOTS: [&str; 13] = [
    "changeset-bot",
    "cloudflare-workers-and-pages",
    "codecov",
    "codecov-commenter",
    "coveralls",
    "dependabot",
    "github-actions",
    "linear",
    "netlify",
    "renovate",
    "sonarcloud",
    "sonarqubecloud",
    "vercel",
];

/// What a Codex review that found nothing says once its footer is gone.
const NOTHING_FOUND: [&str; 3] = ["find any major issues", "no major issues", "no suggestions"];

/// True when the comment by `author` with text `body` leaves nothing for an agent or the owner to do.
#[must_use]
pub fn is_noise(author: &str, body: &str) -> bool {
    if body.contains(SUGGESTION_FENCE) {
        return false;
    }
    if body.contains(RECEIPT_MARKER) {
        return true;
    }
    if author.to_ascii_lowercase().contains("codex") || body.contains(CODEX_FOOTER) {
        return only_codex_boilerplate(body);
    }
    is_status_bot(author)
}

fn is_status_bot(author: &str) -> bool {
    let login = author.trim().to_ascii_lowercase();
    let login = login.strip_prefix("app/").unwrap_or(&login);
    let login = login.strip_suffix("[bot]").unwrap_or(login);
    STATUS_BOTS.contains(&login)
}

fn only_codex_boilerplate(body: &str) -> bool {
    let words = words_of(&without_footer(body));
    words.is_empty()
        || words == "codex review"
        || NOTHING_FOUND.iter().any(|phrase| words.contains(phrase))
}

/// The body without its HTML comments and without the `<details>` blocks that hold the footer.
fn without_footer(body: &str) -> String {
    let mut text = body.to_owned();
    while let Some(start) = text.find("<!--") {
        let Some(len) = text[start..].find("-->") else { break };
        text.replace_range(start..start + len + 3, " ");
    }
    let mut from = 0;
    while let Some(found) = text[from..].find("<details") {
        let start = from + found;
        let end = text[start..].find("</details>").map_or(text.len(), |len| start + len + 10);
        if text[start..end].contains(CODEX_FOOTER) {
            text.replace_range(start..end, " ");
            from = start;
        } else {
            from = end;
        }
    }
    text
}

/// Lowercase letters and digits only, one space between words, so emoji and markup do not matter.
fn words_of(text: &str) -> String {
    let spaced: String = text
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { ' ' })
        .collect();
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOOTER: &str = "<details> <summary>ℹ️ About Codex in GitHub</summary>\n<br/>\n\nYour team has set up Codex to review pull requests in this repo. Reviews are triggered when you open a pull request for review, mark a draft as ready, or comment \"@codex review\". If Codex has suggestions, it will comment; otherwise it will react with 👍.\n\n</details>";

    fn receipt() -> String {
        format!("<!-- codex-review-receipt:v1 -->\n### 💡 Codex Review\n\n{FOOTER}")
    }

    #[test]
    fn a_review_bot_that_found_nothing_is_noise_whoever_posts_it() {
        let nothing = format!("Codex Review: Didn't find any major issues. 🚀\n\n{FOOTER}");
        let footer_only = format!("### 💡 Codex Review\n\n{FOOTER}");
        let cases = [
            ("chatgpt-codex-connector", receipt()),
            ("chatgpt-codex-connector[bot]", footer_only.clone()),
            ("some-reviewer", receipt()),
            ("chatgpt-codex-connector", nothing),
            ("chatgpt-codex-connector", FOOTER.to_owned()),
            ("someone-else", footer_only),
            ("chatgpt-codex-connector", "<!-- codex-review-receipt:v1 -->".to_owned()),
        ];
        for (author, body) in cases {
            assert!(is_noise(author, &body), "{author}: {body}");
        }
    }

    #[test]
    fn a_codex_comment_with_something_to_act_on_stays() {
        let with_findings = format!(
            "### 💡 Codex Review\n\nHere are some automated review suggestions for this pull request.\n\n**Reviewed commit:** `abc1234`\n\n{FOOTER}"
        );
        let prose = "The retry loop in `runner.rs` never resets its counter, so the second stage starts at the limit.";
        let fenced = format!("{}\n```suggestion\nlet total = 0;\n```", receipt());
        for (author, body) in [
            ("chatgpt-codex-connector", with_findings.as_str()),
            ("chatgpt-codex-connector", prose),
            ("chatgpt-codex-connector", fenced.as_str()),
            ("reviewer", prose),
        ] {
            assert!(!is_noise(author, body), "{author}: {body}");
        }
    }

    #[test]
    fn a_status_bot_is_noise_unless_it_suggests_something() {
        let fenced = "Lint found one thing:\n```suggestion\nuse std::fmt;\n```";
        let cases = [
            ("vercel[bot]", "Preview deployed to https://example.test", true),
            ("codecov", "Coverage is 91.2% (+0.3%)", true),
            ("github-actions[bot]", "Docs preview is ready", true),
            ("app/netlify", "Deploy preview ready", true),
            ("Vercel", "Preview deployed", true),
            ("vercel[bot]", fenced, false),
            ("gemini-code-assist[bot]", "Consider renaming this helper.", false),
            ("copilot-pull-request-reviewer", "Copilot reviewed 3 files.", false),
            ("rudf0rd", "Preview deployed to https://example.test", false),
            ("", "", false),
        ];
        for (author, body, want) in cases {
            assert_eq!(is_noise(author, body), want, "{author}: {body}");
        }
    }

    #[test]
    fn markup_and_emoji_around_the_words_do_not_matter() {
        assert_eq!(words_of("### 💡 **Codex**  Review!"), "codex review");
        assert_eq!(without_footer("a <!-- x --> b").split_whitespace().count(), 2);
        let unclosed = "keep <details> this";
        assert_eq!(without_footer(unclosed), unclosed);
    }
}
