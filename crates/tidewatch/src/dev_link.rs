//! Dev-only switches for the Open a repo dialog, read once at startup.
//!
//! - `TIDEWATCH_DEV_LINK=<link>`: open the dialog with that link typed in.
//! - `TIDEWATCH_DEV_LINK_PARENT=<folder>`: the folder a clone goes in.
//! - `TIDEWATCH_DEV_LINK_REMOTE=<address>`: clone from this address (a local bare repo) instead of GitHub.
//! - `TIDEWATCH_DEV_LINK_GO=1`: press Open once the link has been looked up.
//! - `TIDEWATCH_DEV_LINK_FAKE=<line>`: show a clone under way with that progress line and clone nothing.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DevLink {
    pub(crate) link: String,
    pub(crate) parent: Option<String>,
    pub(crate) remote: Option<String>,
    pub(crate) go: bool,
    pub(crate) fake: Option<String>,
    pub(crate) opened: bool,
}

impl DevLink {
    pub(crate) fn from_env() -> Option<Self> {
        let var = |name: &str| std::env::var(name).ok();
        Self::parse(
            var("TIDEWATCH_DEV_LINK").as_deref(),
            var("TIDEWATCH_DEV_LINK_PARENT").as_deref(),
            var("TIDEWATCH_DEV_LINK_REMOTE").as_deref(),
            var("TIDEWATCH_DEV_LINK_GO").as_deref(),
            var("TIDEWATCH_DEV_LINK_FAKE").as_deref(),
        )
    }

    fn parse(
        link: Option<&str>,
        parent: Option<&str>,
        remote: Option<&str>,
        go: Option<&str>,
        fake: Option<&str>,
    ) -> Option<Self> {
        let set = |value: Option<&str>| value.filter(|v| !v.is_empty()).map(str::to_owned);
        Some(Self {
            link: set(link)?,
            parent: set(parent),
            remote: set(remote),
            go: go.is_some_and(|v| !v.is_empty() && v != "0"),
            fake: set(fake),
            opened: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_happens_without_a_link_and_the_rest_are_optional() {
        assert_eq!(DevLink::parse(None, Some("/x"), None, Some("1"), None), None);
        assert_eq!(DevLink::parse(Some(""), None, None, None, None), None);
        let dev =
            DevLink::parse(Some("github.com/a/b"), Some("/tmp/x"), None, Some("0"), Some("l"));
        assert_eq!(
            dev,
            Some(DevLink {
                link: "github.com/a/b".into(),
                parent: Some("/tmp/x".into()),
                remote: None,
                go: false,
                fake: Some("l".into()),
                opened: false,
            })
        );
        assert!(DevLink::parse(Some("x"), None, None, Some("1"), None).unwrap().go);
    }
}
