//! A GitHub repo named by whatever a person pastes: a web link, a clone address, a pull request.
//! Pure. `owner` and `name` keep their case; `same_repo` ignores it, as GitHub does.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GithubRepo {
    pub owner: String,
    pub name: String,
}

const HOSTS: [&str; 2] = ["github.com", "www.github.com"];
const NOT_OWNERS: [&str; 22] = [
    "about",
    "apps",
    "codespaces",
    "collections",
    "enterprise",
    "events",
    "explore",
    "features",
    "issues",
    "login",
    "marketplace",
    "new",
    "notifications",
    "orgs",
    "pricing",
    "pulls",
    "search",
    "security",
    "settings",
    "sponsors",
    "topics",
    "trending",
];
const OWNER_MAX: usize = 39;
const NAME_MAX: usize = 100;

impl GithubRepo {
    /// The repo a pasted link, clone address or `github.com/owner/repo/...` text names. Extra path
    /// (`/tree/main`, `/pull/7`, `/issues`), a query, a fragment and a `.git` suffix are ignored.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_matches(|c| matches!(c, '<' | '>' | '"' | '\''));
        let path = path_of(text)?;
        let mut parts = path.split('/').filter(|part| !part.is_empty());
        let owner = parts.next()?;
        let name = parts.next()?;
        let name = name.strip_suffix(".git").unwrap_or(name);
        let owner_ok = owner.len() <= OWNER_MAX
            && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !owner.starts_with('-')
            && !NOT_OWNERS.contains(&owner.to_ascii_lowercase().as_str());
        let name_ok = !name.is_empty()
            && name.len() <= NAME_MAX
            && !matches!(name, "." | "..")
            && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        (owner_ok && name_ok).then(|| Self { owner: owner.to_owned(), name: name.to_owned() })
    }

    #[must_use]
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    #[must_use]
    pub fn clone_url(&self) -> String {
        format!("https://github.com/{}/{}.git", self.owner, self.name)
    }

    #[must_use]
    pub fn same_repo(&self, other: &Self) -> bool {
        self.owner.eq_ignore_ascii_case(&other.owner) && self.name.eq_ignore_ascii_case(&other.name)
    }
}

fn path_of(text: &str) -> Option<&str> {
    let text = text.split(['?', '#']).next().unwrap_or_default();
    if let Some(rest) = text.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        return is_github(host).then_some(path);
    }
    let after_scheme = match text.split_once("://") {
        Some((scheme, rest)) => {
            matches!(scheme.to_ascii_lowercase().as_str(), "https" | "http" | "ssh" | "git")
                .then_some(rest)?
        }
        None => text,
    };
    let (authority, path) = after_scheme.split_once('/')?;
    let host = authority.rsplit('@').next().unwrap_or_default();
    let host = host.split(':').next().unwrap_or_default();
    is_github(host).then_some(path)
}

fn is_github(host: &str) -> bool {
    HOSTS.iter().any(|known| host.eq_ignore_ascii_case(known))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(owner: &str, name: &str) -> GithubRepo {
        GithubRepo { owner: owner.into(), name: name.into() }
    }

    #[test]
    fn every_way_to_name_a_repo_gives_owner_and_name() {
        let cases = [
            ("https://github.com/acme/web", Some(repo("acme", "web"))),
            ("https://github.com/acme/web/", Some(repo("acme", "web"))),
            ("  https://github.com/acme/web\n", Some(repo("acme", "web"))),
            ("<https://github.com/acme/web>", Some(repo("acme", "web"))),
            ("http://github.com/acme/web", Some(repo("acme", "web"))),
            ("https://www.github.com/acme/web", Some(repo("acme", "web"))),
            ("HTTPS://GitHub.com/Acme/Web", Some(repo("Acme", "Web"))),
            ("https://github.com/acme/web.git", Some(repo("acme", "web"))),
            ("https://github.com/acme/web.js", Some(repo("acme", "web.js"))),
            ("https://github.com/acme/web.js.git", Some(repo("acme", "web.js"))),
            ("https://github.com/acme/.github", Some(repo("acme", ".github"))),
            ("https://tok:x@github.com/acme/web.git", Some(repo("acme", "web"))),
            ("git@github.com:acme/web.git", Some(repo("acme", "web"))),
            ("git@github.com:acme/web", Some(repo("acme", "web"))),
            ("ssh://git@github.com/acme/web.git", Some(repo("acme", "web"))),
            ("ssh://git@github.com:22/acme/web.git", Some(repo("acme", "web"))),
            ("git://github.com/acme/web.git", Some(repo("acme", "web"))),
            ("github.com/acme/web", Some(repo("acme", "web"))),
            ("www.github.com/acme/web", Some(repo("acme", "web"))),
            ("github.com/acme/web/tree/feature/login", Some(repo("acme", "web"))),
            ("https://github.com/acme/web/pull/42", Some(repo("acme", "web"))),
            ("https://github.com/acme/web/pull/42/files", Some(repo("acme", "web"))),
            ("https://github.com/acme/web/issues/9#issuecomment-1", Some(repo("acme", "web"))),
            ("https://github.com/acme/web?tab=readme-ov-file", Some(repo("acme", "web"))),
            ("https://github.com/acme/web/blob/main/src/lib.rs#L10", Some(repo("acme", "web"))),
            ("https://github.com/a-b/c_d", Some(repo("a-b", "c_d"))),
        ];
        for (text, want) in cases {
            assert_eq!(GithubRepo::parse(text), want, "{text:?}");
        }
    }

    #[test]
    fn anything_that_is_not_a_repo_on_github_is_refused() {
        let cases = [
            "",
            "   ",
            "github",
            "github.com",
            "github.com/",
            "github.com/acme",
            "https://github.com/acme",
            "https://github.com/acme/",
            "https://gitlab.com/acme/web",
            "https://github.com.evil.example/acme/web",
            "https://evil.example/github.com/acme/web",
            "git@gitlab.com:acme/web.git",
            "ftp://github.com/acme/web",
            "/Users/me/Git/web",
            "~/Git/web",
            "acme/web",
            "https://github.com/settings/profile",
            "https://github.com/orgs/acme/people",
            "https://github.com/-acme/web",
            "https://github.com/acme/..",
            "https://github.com/acme/we b",
            "https://github.com/acme/.git",
            "https://github.com/ac%20me/web",
        ];
        for text in cases {
            assert_eq!(GithubRepo::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn two_spellings_of_one_repo_match_whatever_the_case() {
        let a = GithubRepo::parse("git@github.com:Acme/Web.git").unwrap();
        let b = GithubRepo::parse("https://github.com/acme/web").unwrap();
        assert!(a.same_repo(&b));
        assert!(!a.same_repo(&GithubRepo::parse("github.com/acme/web2").unwrap()));
        assert!(!a.same_repo(&GithubRepo::parse("github.com/other/web").unwrap()));
    }

    #[test]
    fn the_names_a_clone_needs() {
        let repo = GithubRepo::parse("github.com/Acme/Web/pull/3").unwrap();
        assert_eq!(repo.slug(), "Acme/Web");
        assert_eq!(repo.clone_url(), "https://github.com/Acme/Web.git");
    }
}
