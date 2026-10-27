//! The links of one tab as plain data (pure, table-tested): what an agent reports, what Tidewatch
//! finds in the chat or the tab's pull request, merged by URL into one board that only ever grows.

use serde::{Deserialize, Serialize};

pub const MAX_TITLE_CHARS: usize = 120;
pub const MAX_NOTE_CHARS: usize = 120;
pub const MAX_LINKS_PER_CALL: usize = 20;
const MAX_URL_CHARS: usize = 2048;

/// Where a title came from. A title only gives way to one from the same or a higher source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleFrom {
    /// A plain fallback such as `PR #42`.
    #[default]
    Guess,
    /// Words Tidewatch found: a markdown link text in the chat, the title of the tab's pull request.
    Seen,
    /// What the agent reported.
    Agent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Artifact,
    Pr,
    Issue,
    Doc,
    Web,
}

impl Kind {
    pub const ALL: [Self; 5] = [Self::Artifact, Self::Pr, Self::Issue, Self::Doc, Self::Web];

    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Artifact => "Artifact",
            Self::Pr => "PR",
            Self::Issue => "Issue",
            Self::Doc => "Doc",
            Self::Web => "Web",
        }
    }
}

/// One link the tab keeps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub url: String,
    pub title: String,
    /// What the link is for; for a link sent with a question, the question.
    #[serde(rename = "for", default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub first_seen_ms: i64,
    #[serde(default)]
    pub title_from: TitleFrom,
}

/// A link as an agent reports it, checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkReport {
    pub url: String,
    pub title: String,
    pub note: Option<String>,
}

impl LinkReport {
    #[must_use]
    pub fn into_link(self, now_ms: i64) -> Link {
        Link {
            url: self.url,
            title: self.title,
            note: self.note,
            first_seen_ms: now_ms,
            title_from: TitleFrom::Agent,
        }
    }
}

/// The link's address as `http(s)://host/...` with nothing in it that cannot be a link, or `None`.
#[must_use]
pub fn checked_url(text: &str) -> Option<String> {
    let url = text.trim();
    let rest = strip_scheme(url)?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let fine = !host.is_empty()
        && url.chars().count() <= MAX_URL_CHARS
        && !url.chars().any(|c| c.is_whitespace() || c.is_control());
    fine.then(|| url.to_owned())
}

fn strip_scheme(url: &str) -> Option<&str> {
    ["https://", "http://"].iter().find_map(|scheme| {
        url.get(..scheme.len())
            .filter(|head| head.eq_ignore_ascii_case(scheme))
            .map(|_| &url[scheme.len()..])
    })
}

fn split(url: &str) -> Option<(String, &str)> {
    let rest = strip_scheme(url)?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    Some((rest[..end].to_ascii_lowercase(), rest[end..].trim_start_matches('/')))
}

impl Link {
    /// What two spellings of one link share: the scheme and host in lower case, no trailing slash.
    #[must_use]
    pub fn key(&self) -> String {
        key_of(&self.url)
    }

    #[must_use]
    pub fn kind(&self) -> Kind {
        kind_of(&self.url)
    }

    /// The host without a leading `www.`.
    #[must_use]
    pub fn host(&self) -> String {
        split(&self.url).map_or_else(String::new, |(host, _)| {
            host.strip_prefix("www.").map_or_else(|| host.clone(), str::to_owned)
        })
    }
}

fn key_of(url: &str) -> String {
    match split(url) {
        Some((host, path)) => format!("{host}/{}", path.trim_end_matches('/')),
        None => url.trim().trim_end_matches('/').to_owned(),
    }
}

/// The kind of a link, worked out from its address alone.
#[must_use]
pub fn kind_of(url: &str) -> Kind {
    let Some((host, path)) = split(url) else { return Kind::Web };
    match host.as_str() {
        "claude.ai"
            if ["artifact/", "code/artifact/", "public/artifacts/"]
                .iter()
                .any(|head| path.starts_with(head)) =>
        {
            Kind::Artifact
        }
        "github.com" => {
            let mut parts = path.split('/').skip(2);
            match (
                parts.next(),
                parts.next().is_some_and(|n| n.starts_with(|c: char| c.is_ascii_digit())),
            ) {
                (Some("pull"), true) => Kind::Pr,
                (Some("issues"), true) => Kind::Issue,
                _ => Kind::Web,
            }
        }
        "docs.google.com" => Kind::Doc,
        host if host == "notion.so" || host.ends_with(".notion.so") => Kind::Doc,
        _ => Kind::Web,
    }
}

/// `text` as one line cut to `max` characters, or `None` when nothing is left.
#[must_use]
pub fn one_line(text: &str, max: usize) -> Option<String> {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return None;
    }
    if line.chars().count() <= max {
        return Some(line);
    }
    let head: String = line.chars().take(max - 3).collect();
    Some(format!("{}...", head.trim_end()))
}

/// The links of a tab, oldest first as they arrived.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkBoard {
    #[serde(default)]
    pub links: Vec<Link>,
}

impl LinkBoard {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.links.len()
    }

    /// Takes one link in. A known URL keeps its first-seen time; a non-empty title from the same or
    /// a higher source and a non-empty `for` replace the old ones. Returns whether anything changed.
    pub fn absorb(&mut self, incoming: &Link) -> bool {
        let key = incoming.key();
        let Some(known) = self.links.iter_mut().find(|link| link.key() == key) else {
            self.links.push(incoming.clone());
            return true;
        };
        let before = known.clone();
        known.first_seen_ms = known.first_seen_ms.min(incoming.first_seen_ms);
        let has_title = !incoming.title.trim().is_empty();
        if has_title && (known.title.trim().is_empty() || incoming.title_from >= known.title_from) {
            known.title.clone_from(&incoming.title);
            known.title_from = incoming.title_from;
        }
        if let Some(note) = incoming.note.as_ref().filter(|note| !note.trim().is_empty()) {
            known.note = Some(note.clone());
        }
        *known != before
    }

    pub fn absorb_all<'a>(&mut self, incoming: impl IntoIterator<Item = &'a Link>) -> bool {
        incoming.into_iter().fold(false, |changed, link| self.absorb(link) | changed)
    }

    /// The links by first-seen time, the newest first; links seen at the same time keep the order
    /// they arrived in, the later one first.
    #[must_use]
    pub fn newest_first(&self) -> Vec<&Link> {
        let mut links: Vec<&Link> = self.links.iter().rev().collect();
        links.sort_by_key(|link| std::cmp::Reverse(link.first_seen_ms));
        links
    }
}

#[cfg(test)]
mod tests;
