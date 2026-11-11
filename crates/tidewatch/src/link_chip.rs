//! Link chips in the message box: which addresses are one (a GitHub pull request or issue, a
//! claude.ai artifact), the words a chip shows, and how the box asks `link_glue` for a title and a
//! state and reads the answer back, through two entries of the egui context (pure apart from those).

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Context, Id};
use tidewatch_core::link_board::{Kind, Link, LinkState, Status, TitleFrom, checked_url, kind_of};
use tidewatch_core::text::clip;

const MAX_LABEL_CHARS: usize = 44;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Chip {
    Pull { repo: String, number: u64 },
    Issue { repo: String, number: u64 },
    Artifact,
}

/// What the app knows of a chip's link: its title (never the `PR #N` fallback) and its state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Info {
    pub(crate) title: Option<String>,
    pub(crate) state: LinkState,
}

pub(crate) type Known = HashMap<String, Info>;

fn as_link(url: &str) -> Link {
    Link {
        url: url.to_owned(),
        title: String::new(),
        note: None,
        first_seen_ms: 0,
        title_from: TitleFrom::Guess,
        state: LinkState::Unknown,
        status: Status::default(),
    }
}

/// What two spellings of one link share, as `link_glue` and the Links card key it.
pub(crate) fn key_of(url: &str) -> String {
    as_link(url).key()
}

/// The chip a whole address stands for, or `None` for any other text or link.
pub(crate) fn parse(url: &str) -> Option<Chip> {
    if checked_url(url).as_deref() != Some(url) {
        return None;
    }
    let path = url.split_once("://")?.1.split_once('/')?.1;
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let mut parts = path.split('/');
    match kind_of(url) {
        Kind::Artifact => parts.next_back().filter(|id| !id.is_empty()).map(|_| Chip::Artifact),
        kind @ (Kind::Pr | Kind::Issue) => {
            let (owner, name, _, number) =
                (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
            let repo = format!("{owner}/{name}");
            let number = number.parse().ok()?;
            Some(if kind == Kind::Pr {
                Chip::Pull { repo, number }
            } else {
                Chip::Issue { repo, number }
            })
        }
        _ => None,
    }
}

fn without_number(title: &str, number: u64) -> &str {
    let rest = title
        .trim()
        .strip_prefix(&format!("#{number}"))
        .map(|rest| rest.trim_start_matches([':', ' ']));
    rest.filter(|rest| !rest.is_empty()).unwrap_or(title.trim())
}

/// The words on a chip: `#2903 Title` once the title is known, else `repo #2903`; an artifact is
/// `Artifact` and, with a title, `Artifact · Title`.
pub(crate) fn label(chip: &Chip, info: Option<&Info>) -> String {
    let title =
        info.and_then(|info| info.title.as_deref()).filter(|title| !title.trim().is_empty());
    let words = match chip {
        Chip::Pull { repo, number } | Chip::Issue { repo, number } => match title {
            Some(title) => format!("#{number} {}", without_number(title, *number)),
            None => format!("{} #{number}", repo.rsplit('/').next().unwrap_or(repo)),
        },
        Chip::Artifact => match title {
            Some(title) => format!("Artifact \u{b7} {}", title.trim()),
            None => "Artifact".to_owned(),
        },
    };
    clip(&words, MAX_LABEL_CHARS)
}

fn wants_id() -> Id {
    Id::new("link-chip-wants")
}

fn known_id() -> Id {
    Id::new("link-chip-known")
}

/// A chip of `url` is on screen: `link_glue` looks it up once.
pub(crate) fn want(ctx: &Context, url: &str) {
    ctx.data_mut(|data| {
        let wanted = data.get_temp_mut_or_default::<Vec<String>>(wants_id());
        if !wanted.iter().any(|known| known == url) {
            wanted.push(url.to_owned());
        }
    });
}

pub(crate) fn take_wants(ctx: &Context) -> Vec<String> {
    ctx.data_mut(|data| data.remove_temp::<Vec<String>>(wants_id())).unwrap_or_default()
}

pub(crate) fn publish(ctx: &Context, known: Arc<Known>) {
    ctx.data_mut(|data| data.insert_temp(known_id(), known));
}

pub(crate) fn info(ctx: &Context, url: &str) -> Option<Info> {
    let known = ctx.data(|data| data.get_temp::<Arc<Known>>(known_id()))?;
    known.get(&key_of(url)).cloned()
}

#[cfg(test)]
mod tests;
