//! Comments on a pull request that have nothing to act on, so they never become a card: a review bot's
//! receipt ("I looked, nothing to say"), the status post Codex keeps up to date while it reviews, and
//! the status posts of deploy, coverage and CI bots. A bot comment that carries a suggestion is not
//! noise, and neither is a Codex review whose note says its suggestions are inline. Pure.

use crate::text::squash;

/// The hidden markers of Codex's comments that only report progress: the receipt of a review, and
/// the summary comment it edits as a review starts, runs and ends.
const COMMENT_MARKERS: [&str; 3] =
    ["codex-review-receipt", "codex-pull-request-review-summary", "codex-review-state"];
/// The same without the receipt: Codex marks a review that carries inline suggestions with it too.
const REVIEW_MARKERS: [&str; 2] = ["codex-pull-request-review-summary", "codex-review-state"];
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

/// What a Codex post with nothing to act on says once its footer is gone: a review that found
/// nothing, or the notice to a reviewer without a Codex account.
const NOTHING_FOUND: [&str; 4] = [
    "find any major issues",
    "no major issues",
    "no suggestions",
    "to use codex here create a codex account",
];

/// True when the comment by `author` with text `body` leaves nothing for an agent or the owner to do.
#[must_use]
pub fn is_noise(author: &str, body: &str) -> bool {
    noise(author, body, &COMMENT_MARKERS)
}

/// True when a review's own note leaves nothing to do; its inline comments are not part of `body`.
#[must_use]
pub fn is_review_noise(author: &str, body: &str) -> bool {
    noise(author, body, &REVIEW_MARKERS)
}

fn noise(author: &str, body: &str, markers: &[&str]) -> bool {
    if body.contains(SUGGESTION_FENCE) {
        return false;
    }
    if markers.iter().any(|marker| body.contains(marker)) {
        return true;
    }
    if author.to_ascii_lowercase().contains("codex")
        || body.contains(CODEX_FOOTER)
        || body.contains(RECEIPT_MARKER)
    {
        return only_codex_boilerplate(body);
    }
    is_status_bot(author)
}

/// The body as a reader needs it: no hidden HTML comments and no "About Codex in GitHub" footer.
#[must_use]
pub fn readable(body: &str) -> String {
    without_footer(body).trim().to_owned()
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
    squash(&spaced)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOOTER: &str = "<details> <summary>ℹ️ About Codex in GitHub</summary>\n<br/>\n\nYour team has set up Codex to review pull requests in this repo. Reviews are triggered when you open a pull request for review, mark a draft as ready, or comment \"@codex review\". If Codex has suggestions, it will comment; otherwise it will react with 👍.\n\n</details>";

    const SUMMARY: &str = "<!-- codex-pull-request-review-summary -->\n<!-- codex-review-state:v1 {\"code_review\":{\"commit_sha\":\"0123456789abcdef0123456789abcdef01234567\",\"has_any_findings\":null,\"has_unresolved_findings\":null,\"status\":\"running\"},\"security_review\":{\"commit_sha\":null,\"has_any_findings\":null,\"has_unresolved_findings\":null,\"status\":null}} -->\n\n## Codex Review Summary\n\nThis comment shows the latest Codex review activity on this pull request.\n\n| Review | Status | Commit | Review trigger |\n| --- | --- | --- | --- |\n| \u{1f4dd} **Code Review** | \u{1f504} **Running** since <relative-time datetime=\"2026-10-26T16:09:23.961331Z\">2026-10-26T16:09:23.961331Z</relative-time> | `0123456` | Manual request |\n\n\n\n<details> <summary>\u{2139}\u{fe0f} About Codex in GitHub</summary>\n<br/>\n\n[Your team has set up Codex to review pull requests in this repo](https://chatgpt.com/codex/cloud/settings/general). Reviews are triggered when you\n- Open a pull request for review\n- Mark a draft as ready\n- Comment \"@codex review\" or \"@codex security review\".\n\nCodex reacts with \u{1f440} while any review is running, comments if it has suggestions, and reacts with \u{1f44d} once all reviews finish with no findings.\n\n</details>";

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
    fn codex_status_post_is_noise_in_every_state_it_is_edited_through() {
        let finished = SUMMARY
            .replace("\"status\":\"running\"", "\"status\":\"completed\"")
            .replace("\"has_any_findings\":null", "\"has_any_findings\":true");
        let state_only =
            "<!-- codex-review-state:v1 {\"code_review\":{\"status\":\"running\"}} -->";
        let summary_only = "<!-- codex-pull-request-review-summary -->\n## Codex Review Summary";
        let cases = [
            ("chatgpt-codex-connector", SUMMARY.to_owned()),
            ("chatgpt-codex-connector[bot]", finished),
            ("someone-else", SUMMARY.to_owned()),
            ("chatgpt-codex-connector", state_only.to_owned()),
            ("chatgpt-codex-connector", summary_only.to_owned()),
        ];
        for (author, body) in cases {
            assert!(is_noise(author, &body), "{author}: {body}");
        }
    }

    #[test]
    fn a_status_post_that_carries_a_suggestion_stays() {
        let fenced = format!("{SUMMARY}\n```suggestion\nlet total = 0;\n```");
        assert!(!is_noise("chatgpt-codex-connector", &fenced));
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

    const LIVE_FOOTER: &str = "<details> <summary>\u{2139}\u{fe0f} About Codex in GitHub</summary>\n<br/>\n\nCodex has been enabled to automatically review pull requests in this repo. Reviews are triggered when you\n- Open a pull request for review\n- Mark a draft as ready\n- Comment \"@codex review\".\n\nIf Codex has suggestions, it will comment; otherwise it will react with \u{1f44d}.\n\n\n\n\nWhen you [sign up for Codex through ChatGPT](https://openai.com/codex), Codex can also answer questions or update the PR, like \"@codex address that feedback\".\n            \n</details>";

    #[test]
    fn codex_posts_as_github_sends_them_are_noise_only_when_nothing_is_to_be_done() {
        let review_with_suggestions = format!(
            "<!-- codex-review-receipt:v1 -->\n\n\n### \u{1f4a1} Codex Review\n\nHere are some automated review suggestions for this pull request.\n\n**Reviewed commit:** `0123456789`\n    \n\n{LIVE_FOOTER}"
        );
        let nothing_found = format!(
            "<!-- codex-review-receipt:v1 -->\n\nCodex Review: Didn't find any major issues. Nice work.\n\n**Reviewed commit:** `0123456789`\n\n{LIVE_FOOTER}"
        );
        let no_account = "To use Codex here, [create a Codex account and connect to github](https://chatgpt.com/codex/cloud/settings/connectors).";
        let inline = "**<sub><sub>![P1 Badge](https://img.shields.io/badge/P1-orange?style=flat)</sub></sub>  Reset the counter between stages**\n\nThe second stage starts at the limit.";
        let cases = [
            ("chatgpt-codex-connector", review_with_suggestions.as_str(), false, true),
            ("chatgpt-codex-connector[bot]", review_with_suggestions.as_str(), false, true),
            ("chatgpt-codex-connector", inline, false, false),
            ("chatgpt-codex-connector", nothing_found.as_str(), true, true),
            ("chatgpt-codex-connector[bot]", nothing_found.as_str(), true, true),
            ("chatgpt-codex-connector", no_account, true, true),
            ("someone-else", "<!-- codex-review-receipt:v1 -->", true, true),
            ("chatgpt-codex-connector", SUMMARY, true, true),
        ];
        for (author, body, as_review, as_comment) in cases {
            assert_eq!(is_review_noise(author, body), as_review, "review {author}: {body}");
            assert_eq!(is_noise(author, body), as_comment, "comment {author}: {body}");
        }
    }

    #[test]
    fn a_readable_body_has_no_hidden_markers_and_no_footer() {
        let body = format!(
            "<!-- codex-review-receipt:v1 -->\n\n### Codex Review\n\nHere are some automated review suggestions for this pull request.\n\n{LIVE_FOOTER}"
        );
        assert_eq!(
            readable(&body),
            "### Codex Review\n\nHere are some automated review suggestions for this pull request."
        );
        assert_eq!(readable("  plain words  "), "plain words");
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
