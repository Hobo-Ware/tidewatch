//! Whose pull request comments an agent that runs on its own may act on without the owner reading
//! them first: people with write access to the repository and the review bots the owner relies on.
//! Anyone else can write anything into a public pull request, so their text waits for the owner's
//! Ask. Pure.

const TRUSTED_ASSOCIATIONS: [&str; 3] = ["OWNER", "MEMBER", "COLLABORATOR"];

const REVIEW_BOTS: [&str; 4] =
    ["chatgpt-codex-connector", "copilot-pull-request-reviewer", "gemini-code-assist", "trakt-bot"];

/// True when the comment's author (GitHub's `authorAssociation` is `association`) may steer an agent
/// on its own.
#[must_use]
pub fn is_trusted(author: &str, association: &str) -> bool {
    TRUSTED_ASSOCIATIONS.contains(&association.trim().to_ascii_uppercase().as_str())
        || is_review_bot(author)
}

fn is_review_bot(author: &str) -> bool {
    let login = author.trim().to_ascii_lowercase();
    let login = login.strip_prefix("app/").unwrap_or(&login);
    let login = login.strip_suffix("[bot]").unwrap_or(login);
    REVIEW_BOTS.contains(&login)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn people_with_write_access_and_listed_review_bots_are_trusted() {
        let cases = [
            ("octo", "OWNER", true),
            ("octo", "MEMBER", true),
            ("octo", "COLLABORATOR", true),
            ("octo", "CONTRIBUTOR", false),
            ("octo", "FIRST_TIME_CONTRIBUTOR", false),
            ("octo", "NONE", false),
            ("octo", "", false),
            ("gemini-code-assist[bot]", "NONE", true),
            ("gemini-code-assist", "NONE", true),
            ("app/chatgpt-codex-connector", "NONE", true),
            ("trakt-bot[bot]", "NONE", true),
            ("evil-gemini-code-assist", "NONE", false),
            ("vercel[bot]", "NONE", false),
        ];
        for (author, association, want) in cases {
            assert_eq!(is_trusted(author, association), want, "{author} {association}");
        }
    }
}
