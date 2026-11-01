//! The filters of the Links card, as plain data (pure): the type chips, the state chips and the
//! search, which links they keep and in what order, and the counts the chips show.

use tidewatch_core::link_board::{Kind, Link, LinkBoard};

/// The type chips: everything, or one kind of link.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Type {
    #[default]
    All,
    Prs,
    Issues,
    Artifacts,
    Docs,
}

impl Type {
    pub(crate) const ALL: [Self; 5] =
        [Self::All, Self::Prs, Self::Issues, Self::Artifacts, Self::Docs];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Prs => "PRs",
            Self::Issues => "Issues",
            Self::Artifacts => "Artifacts",
            Self::Docs => "Docs and web",
        }
    }

    fn plural(self) -> &'static str {
        match self {
            Self::All => "links",
            Self::Prs => "PRs",
            Self::Issues => "issues",
            Self::Artifacts => "artifacts",
            Self::Docs => "docs and web links",
        }
    }

    fn keeps(self, kind: Kind) -> bool {
        match self {
            Self::All => true,
            Self::Prs => kind == Kind::Pr,
            Self::Issues => kind == Kind::Issue,
            Self::Artifacts => kind == Kind::Artifact,
            Self::Docs => matches!(kind, Kind::Doc | Kind::Web),
        }
    }
}

/// The state chips: what is still going, what wants the user, and what is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Bucket {
    Open,
    NeedsAction,
    Done,
}

impl Bucket {
    pub(crate) const ALL: [Self; 3] = [Self::Open, Self::NeedsAction, Self::Done];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::NeedsAction => "Needs action",
            Self::Done => "Done",
        }
    }

    fn keeps(self, link: &Link) -> bool {
        match self {
            Self::Open => !link.is_done(),
            Self::NeedsAction => link.needs_action(),
            Self::Done => link.is_done(),
        }
    }
}

/// What the chips and the search ask for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Filter {
    pub(crate) kind: Type,
    pub(crate) bucket: Option<Bucket>,
    pub(crate) search: String,
}

impl Filter {
    fn query(&self) -> String {
        self.search.trim().to_lowercase()
    }

    fn keeps(&self, query: &str, link: &Link) -> bool {
        self.kind.keeps(link.kind())
            && self.bucket.is_none_or(|b| b.keeps(link))
            && searched(query, link)
    }
}

fn searched(query: &str, link: &Link) -> bool {
    query.is_empty()
        || [link.title.as_str(), link.url.as_str(), link.note.as_deref().unwrap_or_default()]
            .iter()
            .any(|text| text.to_lowercase().contains(query))
}

/// The number on each chip: a type chip counts the links that pass the state chip and the search,
/// a state chip the links that pass the type chip and the search.
pub(crate) struct Counts {
    kinds: [usize; 5],
    buckets: [usize; 3],
}

impl Counts {
    pub(crate) fn of_kind(&self, kind: Type) -> usize {
        self.kinds[Type::ALL.iter().position(|t| *t == kind).unwrap_or(0)]
    }

    pub(crate) fn of_bucket(&self, bucket: Bucket) -> usize {
        self.buckets[Bucket::ALL.iter().position(|b| *b == bucket).unwrap_or(0)]
    }
}

pub(crate) fn counts(board: &LinkBoard, filter: &Filter) -> Counts {
    let query = filter.query();
    let mut counts = Counts { kinds: [0; 5], buckets: [0; 3] };
    for link in board.links.iter().filter(|link| searched(&query, link)) {
        let kind = link.kind();
        let in_bucket = filter.bucket.is_none_or(|b| b.keeps(link));
        let in_kind = filter.kind.keeps(kind);
        for (count, chip) in counts.kinds.iter_mut().zip(Type::ALL) {
            *count += usize::from(chip.keeps(kind) && in_bucket);
        }
        for (count, chip) in counts.buckets.iter_mut().zip(Bucket::ALL) {
            *count += usize::from(chip.keeps(link) && in_kind);
        }
    }
    counts
}

/// The links the filter keeps: the ones that want an action first, then the open ones, then the
/// done ones, each group by the time of its last change, the newest first.
pub(crate) fn shown<'a>(board: &'a LinkBoard, filter: &Filter) -> Vec<&'a Link> {
    let query = filter.query();
    let mut links: Vec<&Link> =
        board.newest_first().into_iter().filter(|link| filter.keeps(&query, link)).collect();
    links.sort_by_key(|link| (rank(link), std::cmp::Reverse(link.changed_ms())));
    links
}

fn rank(link: &Link) -> u8 {
    if link.needs_action() {
        0
    } else if link.is_done() {
        2
    } else {
        1
    }
}

/// The line that stands in for the rows when the filter keeps none.
pub(crate) fn empty_line(filter: &Filter) -> String {
    let what = filter.kind.plural();
    let query = filter.search.trim();
    let base = match filter.bucket {
        None => format!("No {what}"),
        Some(Bucket::Open) => format!("No open {what}"),
        Some(Bucket::NeedsAction) if filter.kind == Type::All => "Nothing needs action".to_owned(),
        Some(Bucket::NeedsAction) => format!("Nothing needs action in {what}"),
        Some(Bucket::Done) if filter.kind == Type::All => "Nothing is done".to_owned(),
        Some(Bucket::Done) => format!("Nothing is done in {what}"),
    };
    if query.is_empty() { base } else { format!("Nothing matches \"{query}\"") }
}

#[cfg(test)]
mod tests;
